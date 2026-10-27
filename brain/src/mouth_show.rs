//! The show's sounds: what it asks of the mouth, and the envelopes its looks follow.
//!
//! The one place the show touches the mouth, so a show without one changes here and in the
//! few lines that call it.

use crate::sounds::{COUNT, HEARTBEAT, OWL, THUNDERCLAP};
use mouth::Mouth;

pub struct MouthShow {
    mouth: Mouth<COUNT>,
}

impl MouthShow {
    pub fn new(mouth: Mouth<COUNT>) -> Self {
        Self { mouth }
    }

    /// The heartbeat's loudness per frame, which the rose pulses with.
    pub fn heartbeat_envelope() -> &'static [u8] {
        HEARTBEAT.envelope
    }

    /// The thunderclap's loudness per frame, which the lightning follows; its length is the
    /// length of the thunderclap.
    pub fn thunderclap_envelope() -> &'static [u8] {
        THUNDERCLAP.envelope
    }

    pub fn heartbeat(&mut self) {
        self.mouth.play(&HEARTBEAT);
    }

    pub fn owl(&mut self) {
        self.mouth.play(&OWL);
    }

    pub fn thunderclap(&mut self) {
        self.mouth.play(&THUNDERCLAP);
    }

    pub fn mute(&mut self, mute: bool) {
        self.mouth.mute(mute);
    }
}
