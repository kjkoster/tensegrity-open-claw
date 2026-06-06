extern crate alloc;

use alloc::string::String;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use rtt_target::rprintln;
use static_cell::ConstStaticCell;

pub struct WifiConfig {
    pub ssid: String,
    pub password: String,
}

// req_buf doubles as response buffer; 2 KiB covers both forms plus headers.
static HTTP_RX: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static HTTP_TX: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static HTTP_REQ: ConstStaticCell<[u8; 2048]> = ConstStaticCell::new([0; 2048]);
// Worst-case WiFi form body: ssid (32 chars × 3 encoded) + password (64 × 3) + overhead ≈ 320 bytes.
static HTTP_BODY: ConstStaticCell<[u8; 512]> = ConstStaticCell::new([0; 512]);

pub fn spawn(
    spawner: Spawner,
    stack: embassy_net::Stack<'static>,
    dmx_address: u16,
    ssid: String,
    dmx_signal: &'static Signal<CriticalSectionRawMutex, u16>,
    wifi_signal: &'static Signal<CriticalSectionRawMutex, WifiConfig>,
) {
    spawner
        .spawn(
            task(
                stack,
                HTTP_RX.take(),
                HTTP_TX.take(),
                HTTP_REQ.take(),
                HTTP_BODY.take(),
                dmx_address,
                ssid,
                dmx_signal,
                wifi_signal,
            )
            .unwrap(),
        );
}

/// Serves a minimal HTTP config page on port 80. Handles one connection at a
/// time; browsers retry if the page is loading.
#[embassy_executor::task]
async fn task(
    stack: embassy_net::Stack<'static>,
    rx_buf: &'static mut [u8; 1024],
    tx_buf: &'static mut [u8; 1024],
    req_buf: &'static mut [u8; 2048],
    body_buf: &'static mut [u8; 512],
    addr: u16,
    ssid: String,
    dmx_signal: &'static Signal<CriticalSectionRawMutex, u16>,
    wifi_signal: &'static Signal<CriticalSectionRawMutex, WifiConfig>,
) -> ! {
    let mut dmx_address = addr;

    loop {
        let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if socket.accept(80u16).await.is_err() {
            Timer::after(Duration::from_millis(10)).await;
            continue;
        }

        handle_request(
            &mut socket,
            req_buf,
            body_buf,
            &mut dmx_address,
            &ssid,
            dmx_signal,
            wifi_signal,
        )
        .await;
        socket.flush().await.ok();
        socket.close();

        // Allow the stack to transmit the FIN before the socket is dropped.
        Timer::after(Duration::from_millis(100)).await;
    }
}

enum FormResult {
    None,
    DmxSaved,
    WifiSaved,
    WifiError,
}

async fn handle_request(
    socket: &mut TcpSocket<'_>,
    req_buf: &mut [u8; 2048],
    body_buf: &mut [u8; 512],
    dmx_address: &mut u16,
    ssid: &str,
    dmx_signal: &Signal<CriticalSectionRawMutex, u16>,
    wifi_signal: &Signal<CriticalSectionRawMutex, WifiConfig>,
) {
    // Read headers byte by byte, storing up to req_buf.len() bytes but always
    // draining to \r\n\r\n so the socket is positioned at the start of the body.
    let mut pos = 0usize;
    let mut state = 0u8; // \r\n\r\n detector: 1=\r 2=\r\n 3=\r\n\r 4=done
    loop {
        let mut b = [0u8; 1];
        match socket.read(&mut b).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if pos < req_buf.len() {
            req_buf[pos] = b[0];
        }
        pos += 1;
        state = match (state, b[0]) {
            (2, b'\r') => 3,
            (3, b'\n') => 4,
            (1, b'\n') => 2,
            (_, b'\r') => 1,
            _ => 0,
        };
        if state == 4 {
            break;
        }
    }

    let stored = pos.min(req_buf.len());
    let is_post = req_buf.starts_with(b"POST");
    let content_length = if is_post {
        parse_content_length(&req_buf[..stored])
    } else {
        0
    };

    let result = if is_post && content_length > 0 {
        let to_read = content_length.min(body_buf.len());
        let mut bpos = 0usize;
        while bpos < to_read {
            match socket.read(&mut body_buf[bpos..to_read]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => bpos += n,
            }
        }
        let body = &body_buf[..bpos];

        if let Some(new_addr) = parse_dmx_address(body) {
            *dmx_address = new_addr;
            dmx_signal.signal(*dmx_address);
            rprintln!("DMX base address set to {}", dmx_address);
            FormResult::DmxSaved
        } else {
            match (parse_field(body, b"ssid"), parse_field(body, b"password")) {
                (Some(s), Some(p)) if valid_ssid(&s) && valid_password(&p) => {
                    rprintln!("wifi credentials updated, ssid={}", s);
                    wifi_signal.signal(WifiConfig { ssid: s, password: p });
                    FormResult::WifiSaved
                }
                (Some(_), Some(_)) => FormResult::WifiError,
                _ => FormResult::None,
            }
        }
    } else {
        FormResult::None
    };

    // req_buf is no longer needed for the request; reuse it for the response.
    send_response(socket, req_buf, *dmx_address, ssid, result).await;
}

async fn send_response(
    socket: &mut TcpSocket<'_>,
    buf: &mut [u8; 2048],
    dmx_address: u16,
    ssid: &str,
    result: FormResult,
) {
    let len = build_response(buf, dmx_address, ssid, result);
    tcp_write(socket, &buf[..len]).await;
}

fn build_response(
    buf: &mut [u8; 2048],
    dmx_address: u16,
    ssid: &str,
    result: FormResult,
) -> usize {
    fn put(buf: &mut [u8], pos: &mut usize, data: &[u8]) {
        let n = data.len().min(buf.len() - *pos);
        buf[*pos..*pos + n].copy_from_slice(&data[..n]);
        *pos += n;
    }

    let mut pos = 0usize;
    put(buf, &mut pos, b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n");
    put(buf, &mut pos, b"<!DOCTYPE html><html><head><title>DMX Config</title><style>");
    put(buf, &mut pos, b"body{font-family:system-ui,sans-serif;max-width:22em;margin:2em auto;padding:0 1em}");
    put(buf, &mut pos, b".ok{background:#d4edda;color:#155724;padding:.6em;border-radius:4px;margin:0 0 .5em}");
    put(buf, &mut pos, b".err{background:#f8d7da;color:#721c24;padding:.6em;border-radius:4px;margin:0 0 .5em}");
    put(buf, &mut pos, b"h2{margin-top:1.5em}");
    put(buf, &mut pos, b"input{display:block;width:100%;box-sizing:border-box;padding:.4em;font-size:1em;margin-top:.4em}");
    put(buf, &mut pos, b"input[type=submit]{background:#0066cc;color:#fff;border:none;border-radius:3px;cursor:pointer}");
    put(buf, &mut pos, b"</style></head><body>");

    // --- DMX section ---
    put(buf, &mut pos, b"<h1>DMX Base Address</h1>");
    if matches!(result, FormResult::DmxSaved) {
        put(buf, &mut pos, b"<p class=\"ok\">Saved.</p>");
    }
    put(buf, &mut pos, b"<form method=\"POST\" action=\"/\"><label>Channel (1-512)");
    put(buf, &mut pos, b"<input type=\"number\" name=\"dmx_address\" min=\"1\" max=\"512\" value=\"");
    let mut num_buf = [0u8; 5];
    let s = fmt_u16(dmx_address, &mut num_buf);
    put(buf, &mut pos, &num_buf[s..]);
    put(buf, &mut pos, b"\"></label><input type=\"submit\" value=\"Save\"></form>");

    // --- Wi-Fi section ---
    put(buf, &mut pos, b"<h2>Wi-Fi</h2>");
    match result {
        FormResult::WifiSaved => put(buf, &mut pos, b"<p class=\"ok\">Saved. Rebooting...</p>"),
        FormResult::WifiError => put(
            buf,
            &mut pos,
            b"<p class=\"err\">SSID must be 1-32 chars; password 8-64 chars.</p>",
        ),
        _ => {}
    }
    put(buf, &mut pos, b"<form method=\"POST\" action=\"/\"><label>Network");
    put(buf, &mut pos, b"<input type=\"text\" name=\"ssid\" value=\"");
    for &b in ssid.as_bytes() {
        match b {
            b'"' => put(buf, &mut pos, b"&quot;"),
            b'&' => put(buf, &mut pos, b"&amp;"),
            b'<' => put(buf, &mut pos, b"&lt;"),
            b'>' => put(buf, &mut pos, b"&gt;"),
            _ => put(buf, &mut pos, &[b]),
        }
    }
    put(buf, &mut pos, b"\"></label>");
    put(buf, &mut pos, b"<label>Password<input type=\"password\" name=\"password\"></label>");
    put(buf, &mut pos, b"<input type=\"submit\" value=\"Save &amp; Reboot\"></form>");

    put(buf, &mut pos, b"</body></html>");
    pos
}

async fn tcp_write(socket: &mut TcpSocket<'_>, data: &[u8]) {
    let mut pos = 0;
    while pos < data.len() {
        match socket.write(&data[pos..]).await {
            Ok(n) => pos += n,
            Err(_) => break,
        }
    }
}

fn parse_content_length(headers: &[u8]) -> usize {
    (|| {
        const NEEDLE: &[u8] = b"Content-Length: ";
        let pos = headers.windows(NEEDLE.len()).position(|w| w == NEEDLE)?;
        let after = &headers[pos + NEEDLE.len()..];
        let end = after
            .iter()
            .position(|&b| b == b'\r')
            .unwrap_or(after.len());
        core::str::from_utf8(&after[..end]).ok()?.parse().ok()
    })()
    .unwrap_or(0)
}

fn parse_dmx_address(body: &[u8]) -> Option<u16> {
    const NEEDLE: &[u8] = b"dmx_address=";
    let pos = body.windows(NEEDLE.len()).position(|w| w == NEEDLE)?;
    let after = &body[pos + NEEDLE.len()..];
    let end = after.iter().position(|&b| b == b'&').unwrap_or(after.len());
    let val: u16 = core::str::from_utf8(&after[..end]).ok()?.parse().ok()?;
    if (1..=512).contains(&val) {
        Some(val)
    } else {
        None
    }
}

/// Finds `name=value` in URL-encoded form body, returning the URL-decoded value.
/// Matches only at field boundaries (start of body or after `&`) so that
/// `password=x` does not accidentally match inside `new_password=x`.
fn parse_field(body: &[u8], name: &[u8]) -> Option<String> {
    let mut i = 0;
    while i <= body.len().saturating_sub(name.len() + 1) {
        if (i == 0 || body[i - 1] == b'&')
            && body[i..].starts_with(name)
            && body.get(i + name.len()) == Some(&b'=')
        {
            let value_start = i + name.len() + 1;
            let value_end = body[value_start..]
                .iter()
                .position(|&b| b == b'&')
                .map(|p| value_start + p)
                .unwrap_or(body.len());
            return Some(url_decode(&body[value_start..value_end]));
        }
        i += 1;
    }
    None
}

fn url_decode(input: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < input.len() => {
                match (hex_nibble(input[i + 1]), hex_nibble(input[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi << 4 | lo) as char);
                        i += 3;
                    }
                    _ => {
                        out.push('%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + b - b'a'),
        b'A'..=b'F' => Some(10 + b - b'A'),
        _ => None,
    }
}

fn valid_ssid(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && !s.bytes().any(|b| b == 0)
}

fn valid_password(s: &str) -> bool {
    s.len() >= 8 && s.len() <= 64 && !s.bytes().any(|b| b == 0)
}

/// Formats a u16 right-aligned into `buf`; returns the start index.
fn fmt_u16(val: u16, buf: &mut [u8; 5]) -> usize {
    let mut i = 5usize;
    let mut v = val;
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    i
}
