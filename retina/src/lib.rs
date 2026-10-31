//! The retina: the rig's camera, set up and read, and its newest frame handed on.
//!
//! A thread of its own in the brain, on the background cores. Its consumer says what picture it
//! needs in a `View`, within what the camera offers; the retina sets the camera up for it over
//! VAPIX on every connect, reads its MJPEG stream, and puts each frame into a one-slot mailbox for
//! the consumer to take. A consumer with other needs takes no change here. A camera that drops
//! off or reboots is reconnected with backoff while the consumer's reading goes stale.
//!
//! It is a part on the broker, publishing through the brain's connection under `retina/`:
//! `camera`, `ok` or why not, and `fps`, the frames a second it receives.

pub mod config;
pub mod frames;
pub mod view;

mod camera;
mod mjpeg;

use camera::Camera;
use config as cfg;
use cortex::rate::Rate;
use cortex::telemetry::Publisher;
use frames::{Frame, FrameRx, FrameTx};
use std::io::{self, BufRead};
use std::thread;
use std::time::{Duration, Instant};
use view::View;

/// Starts the retina's thread, set up for `view`, and returns the mailbox it fills.
pub fn spawn(view: View, telemetry: Publisher) -> FrameRx {
    let (frames, reader) = frames::mailbox();
    thread::Builder::new()
        .name("retina".into())
        .spawn(move || {
            cortex::sched::background();
            let mut retina = Retina {
                frames,
                telemetry,
                rate: Rate::new(),
                state: String::new(),
            };
            retina.watch_camera(view)
        })
        .expect("failed to spawn the retina's thread");
    reader
}

struct Retina {
    frames: FrameTx,
    telemetry: Publisher,
    rate: Rate,
    /// What `retina/camera` last said, so it is published only when it changes.
    state: String,
}

impl Retina {
    fn watch_camera(&mut self, view: View) -> ! {
        let camera = Camera::new(view);
        let mut backoff_s = 1u64;
        loop {
            let connected = Instant::now();
            let why = match camera.configure().and_then(|()| camera.stream()) {
                Ok(mut stream) => {
                    self.report("ok");
                    self.pump(&mut stream).to_string()
                }
                Err(why) => why,
            };
            self.report(&why);
            if connected.elapsed().as_secs() > 60 {
                backoff_s = 1;
            }
            eprintln!("retina: {why}; reconnecting in {backoff_s}s");
            thread::sleep(Duration::from_secs(backoff_s));
            backoff_s = (backoff_s * 2).min(cfg::RETRY_MAX_S);
        }
    }

    /// Hands on frames until the stream fails, and returns why.
    fn pump(&mut self, stream: &mut impl BufRead) -> io::Error {
        loop {
            match mjpeg::next_jpeg(stream) {
                Ok(jpeg) => self.hand_on(jpeg),
                Err(e) => return e,
            }
        }
    }

    fn hand_on(&mut self, jpeg: Vec<u8>) {
        self.frames.put(Frame {
            jpeg,
            seen: Instant::now(),
        });
        if let Some(rate) = self.rate.tick() {
            self.telemetry.publish("fps", format!("{rate:.1}"), true);
        }
    }

    fn report(&mut self, state: &str) {
        if self.state != state {
            eprintln!("retina: camera {state}");
            self.telemetry.publish("camera", state, true);
            self.state = state.to_string();
        }
    }
}
