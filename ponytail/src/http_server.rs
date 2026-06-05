use core::sync::atomic::{AtomicU16, Ordering};

use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Timer};
use rtt_target::rprintln;
use static_cell::ConstStaticCell;

pub(crate) static DMX_BASE_ADDRESS: AtomicU16 = AtomicU16::new(333);

// ConstStaticCell holds the value in BSS/data at link time; take() is a pointer
// op with no stack copy, keeping spawn()'s frame under the 1024-byte limit.
static HTTP_RX: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static HTTP_TX: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static HTTP_REQ: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);

pub fn spawn(spawner: Spawner, stack: embassy_net::Stack<'static>) {
    spawner.spawn(task(stack, HTTP_RX.take(), HTTP_TX.take(), HTTP_REQ.take()).unwrap());
}

/// Serves a minimal HTTP config page for the DMX base address on port 80.
/// Handles one connection at a time; browsers retry if the page is loading.
#[embassy_executor::task]
async fn task(
    stack: embassy_net::Stack<'static>,
    rx_buf: &'static mut [u8; 1024],
    tx_buf: &'static mut [u8; 1024],
    req_buf: &'static mut [u8; 1024],
) -> ! {
    loop {
        let mut socket = TcpSocket::new(stack, rx_buf, tx_buf);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if socket.accept(80u16).await.is_err() {
            Timer::after(Duration::from_millis(10)).await;
            continue;
        }

        handle_request(&mut socket, req_buf).await;
        socket.flush().await.ok();
        socket.close();
        // Allow the stack to transmit the FIN before the socket is dropped.
        Timer::after(Duration::from_millis(100)).await;
    }
}

async fn handle_request(socket: &mut TcpSocket<'_>, req_buf: &mut [u8; 1024]) {
    // Read headers byte by byte, storing up to req_buf.len() bytes but always
    // draining to \r\n\r\n so the socket is positioned at the start of the body.
    // Browser POST headers routinely exceed 512 bytes; stopping early at the
    // buffer limit would leave header bytes in the socket that would then be
    // misread as the request body.
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

    let updated = if is_post && content_length > 0 {
        let mut body = [0u8; 32];
        let to_read = content_length.min(body.len());
        let mut bpos = 0usize;
        while bpos < to_read {
            match socket.read(&mut body[bpos..to_read]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => bpos += n,
            }
        }
        if let Some(addr) = parse_dmx_address(&body[..bpos]) {
            DMX_BASE_ADDRESS.store(addr, Ordering::Relaxed);
            rprintln!("DMX base address set to {}", addr);
            true
        } else {
            false
        }
    } else {
        false
    };

    let addr = DMX_BASE_ADDRESS.load(Ordering::Relaxed);
    // req_buf is no longer needed for the request; reuse it for the response.
    send_response(socket, req_buf, addr, updated).await;
}

async fn send_response(socket: &mut TcpSocket<'_>, buf: &mut [u8; 1024], addr: u16, saved: bool) {
    let len = build_response(buf, addr, saved);
    tcp_write(socket, &buf[..len]).await;
}

fn build_response(buf: &mut [u8; 1024], addr: u16, saved: bool) -> usize {
    fn put(buf: &mut [u8], pos: &mut usize, data: &[u8]) {
        let n = data.len().min(buf.len() - *pos);
        buf[*pos..*pos + n].copy_from_slice(&data[..n]);
        *pos += n;
    }

    let mut pos = 0usize;
    put(buf, &mut pos, b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n");
    put(buf, &mut pos, b"<!DOCTYPE html><html><head><title>DMX Config</title><style>");
    put(buf, &mut pos, b"body{font-family:system-ui,sans-serif;max-width:22em;margin:2em auto;padding:0 1em}");
    put(buf, &mut pos, b".saved{background:#d4edda;color:#155724;padding:.6em;border-radius:4px;margin:0 0 .5em}");
    put(buf, &mut pos, b"input{display:block;width:100%;box-sizing:border-box;padding:.4em;font-size:1em;margin-top:.4em}");
    put(buf, &mut pos, b"input[type=submit]{background:#0066cc;color:#fff;border:none;border-radius:3px;cursor:pointer}");
    put(buf, &mut pos, b"</style></head><body><h1>DMX Base Address</h1>");
    if saved {
        put(buf, &mut pos, b"<p class=\"saved\">Saved.</p>");
    }
    put(buf, &mut pos, b"<form method=\"POST\" action=\"/\"><label>Channel (1-512)");
    put(buf, &mut pos, b"<input type=\"number\" name=\"dmx_address\" min=\"1\" max=\"512\" value=\"");
    let mut num_buf = [0u8; 5];
    let s = fmt_u16(addr, &mut num_buf);
    put(buf, &mut pos, &num_buf[s..]);
    put(buf, &mut pos, b"\"></label><input type=\"submit\" value=\"Save\"></form></body></html>");
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
