//! The annotated feed: what the eyeball sees, drawn over, as MJPEG for a browser. The
//! operator's view of the yard, for aiming the camera and the two columns.
//!
//! Bound to the WireGuard address alone, never the wildcard, which would show the yard and
//! whoever is in it to the 4G uplink's public address. A frame is drawn and encoded only on a
//! connected viewer's own thread, so with nobody watching the feed costs the detector nothing.

use crate::config as cfg;
use crate::detect::Person;
use crate::draw;
use cortex::latest::LatestRx;
use image::RgbImage;
use image::codecs::jpeg::JpegEncoder;
use std::io::{self, Cursor, Read};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Request, Response, Server, StatusCode};

/// One frame as the eyeball saw it, handed to the feed.
pub struct Look {
    pub picture: RgbImage,
    pub people: Vec<Person>,
    /// The eyeball's latest frame rate, once it has one.
    pub fps: Option<f64>,
}

/// The latest look, or none before the first frame.
pub type Looks = LatestRx<Option<Arc<Look>>>;

const BOUNDARY: &str = "eyeball";

/// How often a viewer's thread looks for a newer frame than the one it last sent: well inside
/// a frame of the eyeball's, and nothing like busy.
const POLL: Duration = Duration::from_millis(20);

/// How hard the feed's frames are compressed: for a person looking, not for the detector.
const QUALITY: u8 = 70;

/// The cap on the backoff between attempts to bind the feed's address.
const RETRY_MAX_S: u64 = 30;

/// Starts the feed's thread, serving `looks`.
pub fn spawn(looks: Looks) {
    thread::Builder::new()
        .name("feed".into())
        .spawn(move || {
            // Before the server exists, so its threads, and every viewer's, inherit it.
            cortex::sched::background();
            let server = bind();
            eprintln!("feed: serving http://{}/", cfg::FEED_ADDRESS);
            for request in server.incoming_requests() {
                serve(request, &looks);
            }
        })
        .expect("failed to spawn the feed's thread");
}

/// The server, once the WireGuard address it binds is up. The tunnel not being up yet is
/// waited out; any other failure, such as something else holding the port, will not go away.
fn bind() -> Server {
    let mut backoff_s = 1u64;
    loop {
        match Server::http(cfg::FEED_ADDRESS) {
            Ok(server) => return server,
            Err(e)
                if e.downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::AddrNotAvailable) =>
            {
                eprintln!(
                    "feed: {}: {e}; binding again in {backoff_s}s",
                    cfg::FEED_ADDRESS
                );
                thread::sleep(Duration::from_secs(backoff_s));
                backoff_s = (backoff_s * 2).min(RETRY_MAX_S);
            }
            Err(e) => panic!("feed: bind {}: {e}", cfg::FEED_ADDRESS),
        }
    }
}

fn serve(request: Request, looks: &Looks) {
    let url = request.url().to_string();
    let answered = match url.as_str() {
        "/" => {
            let page = "<!doctype html><title>eyeball</title>\
                        <body style=\"margin:0;background:#000\">\
                        <img src=\"/stream.mjpg\" style=\"width:100%\">";
            let html = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
            request.respond(Response::from_string(page).with_header(html))
        }
        "/stream.mjpg" => {
            let looks = looks.clone();
            let viewer = request
                .remote_addr()
                .map(ToString::to_string)
                .unwrap_or_default();
            thread::Builder::new()
                .name("feed-viewer".into())
                .spawn(move || {
                    eprintln!("feed: {viewer} watching");
                    let kind = format!("multipart/x-mixed-replace; boundary={BOUNDARY}");
                    let headers = vec![
                        Header::from_bytes("Content-Type", kind).unwrap(),
                        Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                    ];
                    let frames = Frames {
                        looks,
                        last: None,
                        pending: Cursor::new(Vec::new()),
                    };
                    let response = Response::new(StatusCode(200), headers, frames, None, None);
                    // A stream only ends by the viewer going away, which is what the error says.
                    match request.respond(response) {
                        Ok(()) => eprintln!("feed: {viewer} left"),
                        Err(e) => eprintln!("feed: {viewer} left: {e}"),
                    }
                })
                .expect("failed to spawn a feed viewer's thread");
            Ok(())
        }
        _ => request.respond(Response::from_string("not here").with_status_code(404)),
    };
    if let Err(e) = answered {
        eprintln!("feed: answering {url}: {e}");
    }
}

/// The stream one viewer reads: each newer look, annotated and encoded, as one MJPEG part.
struct Frames {
    looks: Looks,
    last: Option<Arc<Look>>,
    pending: Cursor<Vec<u8>>,
}

impl Frames {
    /// Waits for a look newer than the last one sent, and makes it the pending part.
    fn next_part(&mut self) {
        let look = loop {
            if let Some(look) = self.looks.snapshot()
                && !self
                    .last
                    .as_ref()
                    .is_some_and(|last| Arc::ptr_eq(last, &look))
            {
                break look;
            }
            thread::sleep(POLL);
        };
        let mut picture = look.picture.clone();
        let more = look
            .fps
            .map_or(String::new(), |fps| format!("{fps:.1} fps"));
        draw::annotate(&mut picture, &look.people, &more);

        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
            .encode_image(&picture)
            .expect("feed: a picture the JPEG encoder takes");
        let mut part = format!(
            "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
            jpeg.len()
        )
        .into_bytes();
        part.extend(jpeg);
        part.extend(b"\r\n");
        self.pending = Cursor::new(part);
        self.last = Some(look);
    }
}

impl Read for Frames {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.pending.position() == self.pending.get_ref().len() as u64 {
            self.next_part();
        }
        self.pending.read(buffer)
    }
}
