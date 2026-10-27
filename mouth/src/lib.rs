//! The mouth: the rig's sound, played through a USB audio interface into two powered speakers.
//!
//! A thread in the brain rather than a process, because a show that cuts light and sound
//! together needs them together. Its recordings are compiled in by `mouth-build`, which
//! generates the rig's `sounds` module out of the types here.
//!
//! The show drives it through a [`Mouth`]: a trigger counter per sound, and mute as a level,
//! carried to the playback thread over a latest-value seam. The thread reads the latest cue
//! once per period, several times a frame, and a counter that has moved is the only kind of
//! trigger that reads exactly once however often it is read.
//!
//! The mouth is a part of its own on the broker, publishing through the brain's connection:
//! `mouth/state`, `mouth/error`, `mouth/xruns`, and what the interface is and where, under
//! `mouth/usb/` and `mouth/alsa/`. A dead or missing interface costs the sound and never a
//! frame: the thread reopens it with backoff, and `mouth/error` says why it is silent.

mod config;
mod playback;

use cortex::latest::{LatestTx, latest};
use cortex::telemetry::Publisher;

/// One recording, compiled in.
pub struct Sound {
    /// Its file's stem, which names it in the log.
    pub name: &'static str,
    /// Where it sits in [`Sounds::all`], so a cue can name it.
    pub index: usize,
    /// Stereo frames at [`Sounds::rate_hz`]: left then right, each a little-endian `i16`. A
    /// whole number of DMX frames long, the end padded with silence. At the level the rig's
    /// sound table gives it, so they play as they are.
    pub frames: &'static [[u8; 4]],
    /// Its loudness, one byte per DMX frame from its first sample, the loudest at 255 whatever
    /// its gain. Its length is the recording's length in DMX frames.
    pub envelope: &'static [u8],
}

impl Sound {
    fn sample(&self, frame: usize) -> [i16; 2] {
        let [l0, l1, r0, r1] = self.frames[frame];
        [i16::from_le_bytes([l0, l1]), i16::from_le_bytes([r0, r1])]
    }
}

/// Every recording a rig has, and the rate they were all compiled at.
pub struct Sounds<const N: usize> {
    pub rate_hz: u32,
    pub all: [&'static Sound; N],
}

/// What the show asks of the mouth, as of the latest frame.
#[derive(Clone, Copy)]
struct Cue<const N: usize> {
    /// Times each sound has been asked for. Only a change means anything.
    triggers: [u32; N],
    /// Silence, now: every voice stopped, and no sound started while it holds.
    mute: bool,
}

/// The show's end of the mouth.
pub struct Mouth<const N: usize> {
    cue: Cue<N>,
    cues: LatestTx<Cue<N>>,
}

impl<const N: usize> Mouth<N> {
    /// Starts `sound` at the next period the mouth fills, unless it is muted.
    pub fn play(&mut self, sound: &Sound) {
        self.cue.triggers[sound.index] = self.cue.triggers[sound.index].wrapping_add(1);
        self.cues.publish(self.cue);
    }

    /// Silences the mouth while `mute` holds. Stopped sounds do not resume.
    pub fn mute(&mut self, mute: bool) {
        if self.cue.mute != mute {
            self.cue.mute = mute;
            self.cues.publish(self.cue);
        }
    }
}

/// Starts the mouth on its own thread, playing `sounds` and reporting to `telemetry`, and
/// returns the show's end of it.
pub fn spawn<const N: usize>(sounds: &'static Sounds<N>, telemetry: Publisher) -> Mouth<N> {
    for sound in sounds.all {
        eprintln!(
            "mouth: {:<12}{:.2} s, {} DMX frames",
            sound.name,
            sound.frames.len() as f64 / f64::from(sounds.rate_hz),
            sound.envelope.len(),
        );
    }
    let cue = Cue {
        triggers: [0; N],
        mute: false,
    };
    let (cues, reader) = latest(cue);
    playback::spawn(sounds, reader, telemetry);
    Mouth { cue, cues }
}
