extern crate alloc;

use alloc::string::String;
use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::Duration;
use picoserve::{
    Router, Server,
    extract::{Form, State},
    response::Content,
    routing::get,
};
use rtt_target::rprintln;
use static_cell::{ConstStaticCell, StaticCell};

use crate::DmxConfig;

pub struct WifiConfig {
    pub ssid: String,
    pub password: String,
}

#[derive(Clone)]
struct AppState {
    dmx_address: &'static Mutex<CriticalSectionRawMutex, u16>,
    universe: &'static Mutex<CriticalSectionRawMutex, u16>,
    sacn_port: &'static Mutex<CriticalSectionRawMutex, u16>,
    ssid: String,
    dmx_signal: &'static Signal<CriticalSectionRawMutex, DmxConfig>,
    wifi_signal: &'static Signal<CriticalSectionRawMutex, WifiConfig>,
}

#[derive(serde::Deserialize)]
struct ConfigForm {
    dmx_address: Option<u16>,
    universe: Option<u16>,
    sacn_port: Option<u16>,
    ssid: Option<String>,
    password: Option<String>,
}

struct Html(String);

impl Content for Html {
    fn content_type(&self) -> &'static str {
        "text/html; charset=utf-8"
    }

    fn content_length(&self) -> usize {
        self.0.len()
    }

    async fn write_content<W: picoserve::io::Write>(self, writer: W) -> Result<(), W::Error> {
        self.0.as_bytes().write_content(writer).await
    }
}

enum FormResult {
    None,
    DmxSaved,
    DmxError,
    WifiSaved,
    WifiError,
}

async fn handle_get(State(state): State<AppState>) -> Html {
    let addr = *state.dmx_address.lock().await;
    let uni = *state.universe.lock().await;
    let port = *state.sacn_port.lock().await;
    Html(build_html(addr, uni, port, &state.ssid, FormResult::None))
}

async fn handle_post(State(state): State<AppState>, Form(form): Form<ConfigForm>) -> Html {
    let result = process_form(&state, form).await;
    let addr = *state.dmx_address.lock().await;
    let uni = *state.universe.lock().await;
    let port = *state.sacn_port.lock().await;
    Html(build_html(addr, uni, port, &state.ssid, result))
}

async fn process_form(state: &AppState, form: ConfigForm) -> FormResult {
    if form.dmx_address.is_some() || form.universe.is_some() || form.sacn_port.is_some() {
        let valid_addr = form.dmx_address.filter(|&a| (1..=512).contains(&a));
        let valid_uni = form.universe.filter(|&u| (1..=63999).contains(&u));
        let valid_port = form.sacn_port.filter(|&p| p != 0);
        match (valid_addr, valid_uni, valid_port) {
            (Some(addr), Some(uni), Some(port)) => {
                *state.dmx_address.lock().await = addr;
                *state.universe.lock().await = uni;
                *state.sacn_port.lock().await = port;
                state.dmx_signal.signal(DmxConfig {
                    address: addr,
                    universe: uni,
                    sacn_port: port,
                });
                rprintln!("DMX address={} universe={} sacn_port={}", addr, uni, port);
                FormResult::DmxSaved
            }
            _ => FormResult::DmxError,
        }
    } else {
        match (form.ssid, form.password) {
            (Some(s), Some(p)) if valid_ssid(&s) && valid_password(&p) => {
                rprintln!("wifi credentials updated, ssid={}", s);
                state.wifi_signal.signal(WifiConfig {
                    ssid: s,
                    password: p,
                });
                FormResult::WifiSaved
            }
            (Some(_), Some(_)) => FormResult::WifiError,
            _ => FormResult::None,
        }
    }
}

fn build_html(dmx_address: u16, universe: u16, sacn_port: u16, ssid: &str, result: FormResult) -> String {
    let redirect = matches!(result, FormResult::DmxSaved | FormResult::WifiSaved);
    let dmx_msg = match result {
        FormResult::DmxSaved => r#"<p class="ok">DMX address, universe and sACN port saved.</p>"#,
        FormResult::DmxError => {
            r#"<p class="err">Channel 1-512; universe 1-63999; port 1-65535.</p>"#
        }
        _ => "",
    };
    let wifi_msg = match result {
        FormResult::WifiSaved => r#"<p class="ok">network config saved, rebooting...</p>"#,
        FormResult::WifiError => {
            r#"<p class="err">SSID must be 1-32 chars; password 8-64 chars.</p>"#
        }
        _ => "",
    };

    let mut html = String::new();
    html.push_str(concat!(
        "<!DOCTYPE html><html><head><title>DMX Config</title>",
        r#"<meta charset="utf-8">"#,
    ));
    if redirect {
        html.push_str(r#"<meta http-equiv="refresh" content="5;url=/">"#);
    }
    html.push_str(concat!(
        "<style>body{font-family:system-ui,sans-serif;max-width:22em;margin:2em auto;padding:0 1em}",
        ".ok{background:#d4edda;color:#155724;padding:.6em;border-radius:4px;margin:0 0 .5em}",
        ".err{background:#f8d7da;color:#721c24;padding:.6em;border-radius:4px;margin:0 0 .5em}",
        "h2{margin-top:1.5em}",
        "input{display:block;width:100%;box-sizing:border-box;padding:.4em;font-size:1em;margin-top:.4em}",
        "input[type=submit]{background:#0066cc;color:#fff;border:none;border-radius:3px;cursor:pointer}",
        "</style></head>",
    ));
    html.push_str("<body>");
    html.push_str("<h1>DMX Base Address</h1>");
    html.push_str(dmx_msg);
    html.push_str(r#"<form method="POST" action="/">"#);
    html.push_str(r#"<label>Channel (1-512)"#);
    html.push_str(r#"<input type="number" name="dmx_address" min="1" max="512" value=""#);
    push_u16(&mut html, dmx_address);
    html.push_str(r#""></label>"#);
    html.push_str(r#"<label>sACN Universe (1-63999)"#);
    html.push_str(r#"<input type="number" name="universe" min="1" max="63999" value=""#);
    push_u16(&mut html, universe);
    html.push_str(r#""></label>"#);
    html.push_str(r#"<label>sACN port (1-65535)"#);
    html.push_str(r#"<input type="number" name="sacn_port" min="1" max="65535" value=""#);
    push_u16(&mut html, sacn_port);
    html.push_str(r#""></label>"#);
    html.push_str(r#"<input type="submit" value="save"></form>"#);
    html.push_str("<h2>Wireless Network</h2>");
    html.push_str(wifi_msg);
    html.push_str(r#"<form method="POST" action="/"><label>Network"#);
    html.push_str(r#"<input type="text" name="ssid" value=""#);
    html_attr_escape(&mut html, ssid);
    html.push_str(r#""></label>"#);
    html.push_str(r#"<label>Password<input type="password" name="password"></label>"#);
    html.push_str(r#"<input type="submit" value="save &amp; reboot"></form>"#);
    html.push_str("</body></html>");
    html
}

fn html_attr_escape(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn push_u16(s: &mut String, val: u16) {
    let mut buf = [0u8; 5];
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
    s.push_str(core::str::from_utf8(&buf[i..]).unwrap());
}

fn valid_ssid(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && !s.bytes().any(|b| b == 0)
}

fn valid_password(s: &str) -> bool {
    s.len() >= 8 && s.len() <= 64 && !s.bytes().any(|b| b == 0)
}

static HTTP_RX: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static HTTP_TX: ConstStaticCell<[u8; 1024]> = ConstStaticCell::new([0; 1024]);
static HTTP_BUF: ConstStaticCell<[u8; 2048]> = ConstStaticCell::new([0; 2048]);
static DMX_ADDR: StaticCell<Mutex<CriticalSectionRawMutex, u16>> = StaticCell::new();
static UNIVERSE: StaticCell<Mutex<CriticalSectionRawMutex, u16>> = StaticCell::new();
static SACN_PORT: StaticCell<Mutex<CriticalSectionRawMutex, u16>> = StaticCell::new();

pub fn spawn(
    spawner: Spawner,
    stack: Stack<'static>,
    dmx_address: u16,
    universe: u16,
    sacn_port: u16,
    ssid: String,
    dmx_signal: &'static Signal<CriticalSectionRawMutex, DmxConfig>,
    wifi_signal: &'static Signal<CriticalSectionRawMutex, WifiConfig>,
) {
    let dmx_mutex = DMX_ADDR.init(Mutex::new(dmx_address));
    let uni_mutex = UNIVERSE.init(Mutex::new(universe));
    let port_mutex = SACN_PORT.init(Mutex::new(sacn_port));
    spawner.spawn(
        task(
            stack,
            HTTP_RX.take(),
            HTTP_TX.take(),
            HTTP_BUF.take(),
            dmx_mutex,
            uni_mutex,
            port_mutex,
            ssid,
            dmx_signal,
            wifi_signal,
        )
        .unwrap(),
    );
}

#[embassy_executor::task]
async fn task(
    stack: Stack<'static>,
    rx_buf: &'static mut [u8; 1024],
    tx_buf: &'static mut [u8; 1024],
    http_buf: &'static mut [u8; 2048],
    dmx_address: &'static Mutex<CriticalSectionRawMutex, u16>,
    universe: &'static Mutex<CriticalSectionRawMutex, u16>,
    sacn_port: &'static Mutex<CriticalSectionRawMutex, u16>,
    ssid: String,
    dmx_signal: &'static Signal<CriticalSectionRawMutex, DmxConfig>,
    wifi_signal: &'static Signal<CriticalSectionRawMutex, WifiConfig>,
) -> ! {
    let state = AppState {
        dmx_address,
        universe,
        sacn_port,
        ssid,
        dmx_signal,
        wifi_signal,
    };
    let app = Router::new()
        .route("/", get(handle_get).post(handle_post))
        .with_state(state);
    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Duration::from_secs(5),
        persistent_start_read_request: Duration::from_secs(1),
        read_request: Duration::from_secs(3),
        write: Duration::from_secs(5),
    });
    Server::new(&app, &config, http_buf)
        .listen_and_serve(0u8, stack, 80, rx_buf, tx_buf)
        .await
        .into_never()
}
