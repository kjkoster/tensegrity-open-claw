//! The recordings: every value, a line of InfluxDB line protocol, written into GreptimeDB's
//! `rig` database in batches.
//!
//! One table per first level of the address, the rest of it as the `topic` tag, the schema the
//! recorder wrote from MQTT before Memory took over, so old and new read as one. Numbers go
//! into `value`, flags as 1 and 0 so they draw as a line, words and the floats no number can
//! hold, NaN among them, into `text`. Timed when Memory received them.

use crate::osc::Value;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// GreptimeDB's HTTP listener, on loopback, as `clockwork.dot` and its config have it.
const GREPTIME: &str = "127.0.0.1:4000";
const DATABASE: &str = "rig";
/// Short enough that a power cut costs only seconds, long enough that each write carries many
/// values rather than one.
const FLUSH: Duration = Duration::from_secs(5);
/// What is held while GreptimeDB cannot be reached: hours of the rig, in some tens of MB.
/// Beyond it the oldest go, rather than the Pi's memory.
const BACKLOG_MAX: usize = 200_000;
const TIMEOUT: Duration = Duration::from_secs(10);

/// The lines waiting to be written, shared by whoever records and the writer.
#[derive(Clone, Default)]
pub struct Backlog(Arc<Mutex<VecDeque<String>>>);

impl Backlog {
    /// Holds one value, at `address`, received at `at_ms`.
    pub fn push(&self, address: &str, value: &Value, at_ms: u64) {
        let Some(line) = line(address, value, at_ms) else {
            return;
        };
        let mut lines = self.0.lock().unwrap();
        lines.push_back(line);
        if lines.len() > BACKLOG_MAX {
            lines.pop_front();
        }
    }

    fn take(&self) -> VecDeque<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }

    /// Puts back what could not be written, ahead of what arrived meanwhile.
    fn restore(&self, mut taken: VecDeque<String>) {
        let mut lines = self.0.lock().unwrap();
        taken.append(&mut lines);
        let excess = taken.len().saturating_sub(BACKLOG_MAX);
        if excess > 0 {
            taken.drain(..excess);
            eprintln!("memory: GreptimeDB away too long: dropped the oldest {excess} values");
        }
        *lines = taken;
    }
}

/// Writes the backlog every few seconds, for ever, on a thread of its own.
pub fn spawn(backlog: Backlog) {
    thread::Builder::new()
        .name("greptime".into())
        .spawn(move || {
            let mut created = false;
            let mut status = String::new();
            loop {
                thread::sleep(FLUSH);
                let lines = backlog.take();
                let now = match write(&lines, &mut created) {
                    Ok(()) => "ok".to_owned(),
                    Err(Failure::Refused(why)) => {
                        // Refused however often it is sent, and holding it would block what
                        // comes after.
                        eprintln!("memory: GreptimeDB refused {} values, dropped", lines.len());
                        format!("refused: {why}")
                    }
                    Err(Failure::Unreachable(why)) => {
                        backlog.restore(lines);
                        format!("unreachable: {why}")
                    }
                };
                if now != status {
                    eprintln!("memory: GreptimeDB {now}");
                    status = now;
                }
            }
        })
        .expect("failed to spawn the greptime thread");
}

enum Failure {
    /// It answered and said no.
    Refused(String),
    /// It did not answer.
    Unreachable(String),
}

fn write(lines: &VecDeque<String>, created: &mut bool) -> Result<(), Failure> {
    if !*created {
        // Without a time-to-live: the recordings are kept for good.
        post(
            "/v1/sql",
            "application/x-www-form-urlencoded",
            &format!("sql=CREATE%20DATABASE%20IF%20NOT%20EXISTS%20{DATABASE}"),
        )?;
        *created = true;
    }
    if lines.is_empty() {
        return Ok(());
    }
    let body = lines
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    post(
        &format!("/v1/influxdb/write?db={DATABASE}&precision=ms"),
        "text/plain",
        &body,
    )
}

/// One HTTP/1.1 POST, its connection closed after: a request every few seconds, to loopback.
fn post(path: &str, content_type: &str, body: &str) -> Result<(), Failure> {
    let unreachable = |e: std::io::Error| Failure::Unreachable(e.to_string());
    let address: SocketAddr = GREPTIME.parse().unwrap();
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(unreachable)?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(unreachable)?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {GREPTIME}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(unreachable)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(unreachable)?;
    let response = String::from_utf8_lossy(&response);
    let status: u16 = response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| Failure::Unreachable("an answer that is not HTTP".into()))?;
    let detail = || {
        let body = response
            .split_once("\r\n\r\n")
            .map_or("", |(_, body)| body.trim());
        format!("HTTP {status}: {body}")
    };
    match status {
        200..=299 => Ok(()),
        400..=499 => Err(Failure::Refused(detail())),
        _ => Err(Failure::Unreachable(detail())),
    }
}

/// One value as a line, or None for an address with nothing after its first level's slash.
fn line(address: &str, value: &Value, at_ms: u64) -> Option<String> {
    let (table, topic) = match address.trim_start_matches('/').split_once('/') {
        Some((table, topic)) => (table, Some(topic)),
        None => (address.trim_start_matches('/'), None),
    };
    if table.is_empty() {
        return None;
    }
    let tag = topic.map_or(String::new(), |topic| format!(",topic={}", key(topic)));
    let field = match value {
        Value::Int(i) => format!("value={i}"),
        Value::Float(x) if x.is_finite() => format!("value={x}"),
        Value::Float(x) => format!("text=\"{x}\""),
        Value::Bool(b) => format!("value={}", u8::from(*b)),
        Value::Text(s) => format!("text=\"{}\"", string(s)),
    };
    Some(format!("{}{tag} {field} {at_ms}", key(table)))
}

/// For a table or tag name, and a tag value: commas, spaces and equals signs are syntax.
fn key(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace(' ', "\\ ")
        .replace('=', "\\=")
}

/// For a quoted string field. A newline would end the line, so it is written as `\n`.
fn string(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
