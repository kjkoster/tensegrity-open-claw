//! The retina: the rig's camera, set up and read, and its newest frame handed on.
//!
//! A thread of its own in the brain, on the background cores. Its consumer says what picture it
//! needs in a `View`, within what the camera offers; the retina sets the camera up for it over
//! VAPIX on every connect, reads its MJPEG stream, and puts each frame into a one-slot mailbox for
//! the consumer to take. A consumer with other needs takes no change here. A camera that drops
//! off or reboots is reconnected with backoff while the consumer's reading goes stale; a
//! recorded stream stands in for the camera at the bench.
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
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use view::View;

/// Where the picture comes from.
pub enum Source {
    Camera,
    /// An MJPEG stream recorded into a file, played at the rate asked for, and from the start
    /// again at its end. Its frames are as they were recorded; the rest of the view asked for
    /// was the recording's to get right.
    Footage(PathBuf),
}

/// Starts the retina's thread, set up for `view`, and returns the mailbox it fills.
pub fn spawn(source: Source, view: View, telemetry: Publisher) -> FrameRx {
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
            match source {
                Source::Camera => retina.watch_camera(view),
                Source::Footage(path) => retina.play(&path, view),
            }
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

    fn play(&mut self, path: &Path, view: View) -> ! {
        let pace = Duration::from_secs(1) / view.rate.get();
        self.report("footage");
        loop {
            let file =
                File::open(path).unwrap_or_else(|e| panic!("retina: {}: {e}", path.display()));
            let mut footage = BufReader::new(file);
            let mut played = 0;
            loop {
                match mjpeg::next_jpeg(&mut footage) {
                    Ok(jpeg) => self.hand_on(jpeg),
                    // The end, a frame cut short by stopping the recording included.
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                    Err(e) => panic!("retina: {}: after {played} frames: {e}", path.display()),
                }
                played += 1;
                thread::sleep(pace);
            }
            if played == 0 {
                panic!("retina: {}: no MJPEG frames in it", path.display());
            }
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
