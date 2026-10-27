//! The mouth's part of the show: for now, a bench test of the sound against the light.
//!
//! It plays every recording in turn with a second's silence after each, round and round, and
//! lights the face's left lamp with the envelope of the one playing, so that with headphones on
//! the interface the sound and the lamp can be checked against each other. The lamp plays the
//! envelope by frame index from the frame that cued the sound, as any look following a
//! recording does.
//!
//! The e-stop mutes the mouth, as it darkens the light.

use crate::patch::FACE;
use crate::sounds::{COUNT, SOUNDS};
use cortex::config::FRAME_RATE_HZ;
use mouth::Mouth;

/// The silence after each recording: one second.
const PAUSE_FRAMES: usize = FRAME_RATE_HZ as usize;

enum Step {
    /// A recording playing, and the frame of it this one is.
    Sound { index: usize, frame: usize },
    /// The silence before the next recording, and the frames of it still to go.
    Pause { next: usize, frames_left: usize },
}

pub struct MouthShow {
    mouth: Mouth<COUNT>,
    step: Step,
}

impl MouthShow {
    pub fn new(mouth: Mouth<COUNT>) -> Self {
        Self {
            mouth,
            step: Step::Pause {
                next: 0,
                frames_left: PAUSE_FRAMES,
            },
        }
    }

    /// Steps the sequence one frame, and fills the left lamp.
    pub fn fill(&mut self, slots: &mut [u8], estop: bool) {
        self.mouth.mute(estop);
        let level = loop {
            match self.step {
                Step::Sound { index, frame } => match SOUNDS.all[index].envelope.get(frame) {
                    Some(&level) => {
                        self.step = Step::Sound {
                            index,
                            frame: frame + 1,
                        };
                        break level;
                    }
                    None => {
                        self.step = Step::Pause {
                            next: (index + 1) % COUNT,
                            frames_left: PAUSE_FRAMES,
                        }
                    }
                },
                Step::Pause {
                    next,
                    frames_left: 0,
                } => {
                    self.mouth.play(SOUNDS.all[next]);
                    self.step = Step::Sound {
                        index: next,
                        frame: 0,
                    };
                }
                Step::Pause { next, frames_left } => {
                    self.step = Step::Pause {
                        next,
                        frames_left: frames_left - 1,
                    };
                    break 0;
                }
            }
        };
        FACE.left_lamp.set(slots, level);
    }
}
