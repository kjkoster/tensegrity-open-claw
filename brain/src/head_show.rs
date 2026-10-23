//! The heads' channels for a cue.
//!
//! Every channel of every head is written every frame, in every state, so nothing a head does
//! is left to whatever its slots held before. In the base states the heads rest dark on the
//! ball. An incident lights them at its start, rising over a second, and glides them, lit, to
//! its preset — the move is part of the look — and every head's light falls over the
//! incident's last seconds. Looks elsewhere switch in one frame; only the heads fade.

use crate::ball_lighting;
use crate::legs::{Colour, LEGS, paint_head};
use crate::patch::PoseidonLaserBeamXs;
use crate::presets::{self, Preset};
use crate::show_state::{Cue, Incident, ShowState};
use cortex::draw::Draw;
use cortex::moving_head::{Pose, Slew, SlewRate, aim};

/// How fast a head glides: the slowest it moves smoothly.
const GLIDE: SlewRate = SlewRate::SLOWEST;
/// How long the heads take to light at an incident's start, and to go dark at its end.
const LIGHT_RISE_S: f64 = 1.0;
const LIGHT_FALL_S: f64 = 5.0;
/// Searchlights' wait between one snap and the next.
const SNAP_PAUSE_S: (f64, f64) = (2.0, 6.0);
const SNAP_SEED: u64 = 0x5ea4_c411_9b7e_0d13;

struct Head {
    slew: Slew,
    /// Whether the head has landed its entry glide.
    arrived: bool,
}

pub struct HeadShow {
    heads: [Head; 3],
    /// Searchlights' next snap, by its number and when it is due; not due at all until every
    /// head has arrived at heaven, so the glide up is seen whole.
    snap_number: i64,
    snap_at_s: Option<f64>,
}

impl HeadShow {
    pub fn new() -> Self {
        let head = |index: usize| {
            let rest = (LEGS[index].pose)(&presets::LOOK_AT_BALL);
            Head {
                slew: Slew::new(rest, GLIDE),
                arrived: false,
            }
        };
        Self {
            heads: [head(0), head(1), head(2)],
            snap_number: 0,
            snap_at_s: None,
        }
    }

    /// Fills the heads' channels for this frame, `dt` seconds after the last.
    pub fn fill(&mut self, slots: &mut [u8], cue: Cue, dt: f64) {
        if cue.entering {
            self.enter(cue);
        }
        if cue.state == ShowState::Incident(Incident::Searchlights) {
            self.snap(cue);
        }

        let rise = (cue.state_elapsed / LIGHT_RISE_S).clamp(0.0, 1.0);
        let fall = cue.state_remaining.map_or(1.0, |remaining| (remaining / LIGHT_FALL_S).clamp(0.0, 1.0));
        for (head, leg) in self.heads.iter_mut().zip(&LEGS) {
            head.slew.step(dt);
            head.arrived |= head.slew.landed();

            let (colour, level) = match cue.state {
                ShowState::Base(_) => (Colour::White, 0.0),
                ShowState::Incident(Incident::Searchlights) => (Colour::White, 1.0),
                ShowState::Incident(Incident::BallLighting) => {
                    // The wheel steps where the washes and the ball are halfway.
                    let colour = if ball_lighting::phase(&cue) < 0.5 { Colour::White } else { leg.colour };
                    (colour, 1.0)
                }
            };
            write(&leg.head, slots, head.slew.pose(), colour, level * rise * fall);
        }
    }

    /// Sends every head toward the new state's resting place.
    fn enter(&mut self, cue: Cue) {
        let preset: &Preset = match cue.state {
            ShowState::Base(_) | ShowState::Incident(Incident::BallLighting) => &presets::LOOK_AT_BALL,
            ShowState::Incident(Incident::Searchlights) => &presets::LOOK_AT_HEAVEN,
        };
        for (head, leg) in self.heads.iter_mut().zip(&LEGS) {
            head.slew.glide_to((leg.pose)(preset));
            head.arrived = false;
        }
        self.snap_number = 0;
        self.snap_at_s = None;
    }

    /// Snaps one head to a random point in its patch of sky, when the next snap is due. The
    /// first is due a pause after every head has arrived at heaven.
    fn snap(&mut self, cue: Cue) {
        let at_s = match self.snap_at_s {
            Some(at_s) => at_s,
            None if self.heads.iter().all(|head| head.arrived) => {
                let at_s = cue.state_elapsed + snap_pause(cue.state_key, 0);
                self.snap_at_s = Some(at_s);
                at_s
            }
            None => return,
        };
        if cue.state_elapsed < at_s {
            return;
        }

        let mut draw = Draw::new(SNAP_SEED, &[cue.state_key, self.snap_number, 1]);
        let index = draw.index(self.heads.len());
        let a = (LEGS[index].pose)(&presets::SKY_CORNER_A);
        let b = (LEGS[index].pose)(&presets::SKY_CORNER_B);
        let point = Pose::new(
            mix(a.pan_deg, b.pan_deg, draw.unit()),
            mix(a.tilt_deg, b.tilt_deg, draw.unit()),
        );
        self.heads[index].slew.jump(point);

        self.snap_number += 1;
        self.snap_at_s = Some(cue.state_elapsed + snap_pause(cue.state_key, self.snap_number));
    }
}

/// The wait before snap `number` of the Searchlights keyed `key`.
fn snap_pause(key: i64, number: i64) -> f64 {
    Draw::new(SNAP_SEED, &[key, number, 0]).between(SNAP_PAUSE_S.0, SNAP_PAUSE_S.1)
}

/// One head's every channel: where it points, how bright, and what colour, with everything it
/// could do by itself held off.
fn write(head: &PoseidonLaserBeamXs, slots: &mut [u8], pose: Pose, colour: Colour, dimmer: f64) {
    aim(head, slots, pose);
    head.dimmer.set_unit(slots, dimmer);
    paint_head(head, slots, colour);
    head.shutter.set(slots, PoseidonLaserBeamXs::SHUTTER_SHUTTER_OPEN.center());
    head.gobo_wheel.set(slots, PoseidonLaserBeamXs::GOBO_WHEEL_OPEN.center());
    head.prism_1.set(slots, PoseidonLaserBeamXs::PRISM_1_PRISM_OUT.center());
    head.prism_2.set(slots, PoseidonLaserBeamXs::PRISM_2_PRISM_OUT.center());
    head.frost.set(slots, PoseidonLaserBeamXs::FROST_FROST_OUT.center());
    head.device_set.park(slots);
}

fn mix(from: f64, to: f64, amount: f64) -> f64 {
    from + (to - from) * amount
}
