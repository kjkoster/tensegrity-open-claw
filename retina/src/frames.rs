//! The retina's one-slot mailbox: the newest frame, taken by whoever runs on it.
//!
//! A take and not a latest-value read: a reader that ran slower than the camera would
//! otherwise see the same frame twice, and one that ran faster would wait on nothing. Putting a
//! frame replaces whatever was there, so a slow reader always gets the newest one and never a
//! queue of old ones.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// One frame as the camera sent it.
pub struct Frame {
    pub jpeg: Vec<u8>,
    /// When it arrived, which is when the yard looked like this, give or take the camera's own
    /// delay.
    pub seen: Instant,
}

struct Slot {
    frame: Mutex<Option<Frame>>,
    arrived: Condvar,
}

/// The retina's end.
pub struct FrameTx(Arc<Slot>);

/// The reader's end.
pub struct FrameRx(Arc<Slot>);

pub fn mailbox() -> (FrameTx, FrameRx) {
    let slot = Arc::new(Slot {
        frame: Mutex::new(None),
        arrived: Condvar::new(),
    });
    (FrameTx(slot.clone()), FrameRx(slot))
}

impl FrameTx {
    /// Replaces whatever frame is waiting with `frame`.
    pub fn put(&self, frame: Frame) {
        *self.0.frame.lock().expect("retina: mailbox") = Some(frame);
        self.0.arrived.notify_one();
    }
}

impl FrameRx {
    /// The newest frame, waiting for one if none is there.
    pub fn take(&self) -> Frame {
        let mut slot = self.0.frame.lock().expect("retina: mailbox");
        loop {
            if let Some(frame) = slot.take() {
                return frame;
            }
            slot = self.0.arrived.wait(slot).expect("retina: mailbox");
        }
    }
}
