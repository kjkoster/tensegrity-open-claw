//! The heads' channels for a cue.
//!
//! Every channel of every head is written every frame, in every state, so nothing a head does
//! is left to whatever its slots held before. In blackout and daylight the heads are parked dark
//! at heaven. At dusk they light the ball in their legs' colours, against the music: dim when
//! the washes flash, brightest when the rig goes dark — and each one's dot dances over the ball,
//! livelier with the music and surging on its beats. A state lights them at its start, rising
//! over a second, and glides them, lit, to its preset — the move is part of the look — and an
//! incident's light falls over its last seconds. Looks elsewhere switch in one frame; only the
//! heads fade.

use crate::ball_lighting;
use crate::legs::{Colour, LEGS, paint_head};
use crate::patch::PoseidonLaserBeamXs;
use crate::presets::{self, Preset};
use crate::show_state::{Base, Cue, Incident, ShowState};
use cortex::draw::Draw;
use cortex::ear::AudioFeatures;
use cortex::moving_head::{Pose, Slew, SlewRate, aim};
use cortex::qlc_plus::Position;
use std::f64::consts::{FRAC_1_SQRT_2, TAU};

/// How fast a head glides: the slowest it moves smoothly.
const GLIDE: SlewRate = SlewRate::SLOWEST;
/// How long the heads take to light at an incident's start, and to go dark at its end.
const LIGHT_RISE_S: f64 = 1.0;
const LIGHT_FALL_S: f64 = 5.0;
/// The heads' light on the ball at dusk, at its brightest: when the washes are at their
/// darkest. Shallow, so the ball glows rather than competes with the washes.
const DUSK_BALL_LEVEL: f64 = 0.35;

/// How far from the ball's centre a dot may wander at dusk, in degrees as the head sees it. The
/// ball is about ±8–10° from a head, so this reaches near its edge, and now and then past it.
const DANCE_RADIUS_DEG: f64 = 6.0;
/// The share of that radius a dot keeps to in silence; the music widens it to all of it.
const DANCE_SILENT_SIZE: f64 = 0.5;
/// How fast a dot travels its path, in cycles a second, in silence and with the music in.
const DANCE_PACE_SILENT_HZ: f64 = 0.03;
const DANCE_PACE_MUSIC_HZ: f64 = 0.2;
/// The extra pace a full-strength beat adds, and how quickly it dies away: a surge along the
/// path rather than a jump off it, because a head needs a tenth of a second or more to get
/// anywhere, and a jump would land visibly after the beat.
const DANCE_BEAT_PUSH_HZ: f64 = 0.6;
const DANCE_PUSH_DECAY_S: f64 = 0.2;
/// How long a dot takes to widen from the centre once its head has arrived on the ball, so the
/// dance does not start with a jolt at the end of the glide.
const DANCE_RAMP_S: f64 = 2.0;
/// The least the ball is taken to sit off a head's pan axis. Near that axis pan barely moves
/// the dot, so the pan asked for grows as the angle shrinks; this caps it.
const DANCE_MIN_OFF_AXIS_DEG: f64 = 10.0;
/// Each head's path: its sideways and up-down waves' speeds against the pace, and the up-down
/// wave's head start in cycles. Unrelated ratios, so no two dots ever move in step.
const DANCE_SHAPES: [(f64, f64, f64); 3] = [(1.0, 1.37, 0.0), (1.19, 1.61, 0.3), (0.87, 1.23, 0.6)];

/// Searchlights' wait between one snap and the next.
const SNAP_PAUSE_S: (f64, f64) = (2.0, 6.0);
const SNAP_SEED: u64 = 0x5ea4_c411_9b7e_0d13;

struct Head {
    slew: Slew,
    /// Whether the head has landed its entry glide.
    arrived: bool,
    dance: Dance,
}

/// One head's dot wandering over the ball at dusk: a point on a smooth path around the ball's
/// centre, moved along it by the music.
struct Dance {
    /// How far along its path the dot is, in cycles of the pace.
    phase: f64,
    shape: (f64, f64, f64),
    /// The pace the beats have added and that is still dying away, in cycles a second.
    push_hz: f64,
    /// The ear's onset count as last seen, so each beat pushes once; `None` until the first
    /// frame of a dance, which only learns where the count stands.
    seen_onsets: Option<u64>,
    /// 0..1, how far the dot has widened from the centre since its head arrived.
    size: f64,
}

impl Dance {
    fn new(shape: (f64, f64, f64)) -> Self {
        Self {
            phase: 0.0,
            shape,
            push_hz: 0.0,
            seen_onsets: None,
            size: 0.0,
        }
    }

    /// Moves the dot on by `dt` seconds of music, and returns where it now is from the ball's
    /// centre: sideways and up, in degrees.
    fn step(&mut self, features: &AudioFeatures, dt: f64) -> (f64, f64) {
        let music = if features.is_stale() { 0.0 } else { f64::from(features.music_amount) };
        if self.seen_onsets.is_some_and(|seen| seen != features.onset_count) {
            let strength = f64::from(features.last_onset_strength).min(1.0);
            self.push_hz += DANCE_BEAT_PUSH_HZ * strength * music;
        }
        self.seen_onsets = Some(features.onset_count);
        self.push_hz *= (-dt / DANCE_PUSH_DECAY_S).exp();

        let pace = mix(DANCE_PACE_SILENT_HZ, DANCE_PACE_MUSIC_HZ, music);
        self.phase += (pace + self.push_hz) * dt;
        self.size = (self.size + dt / DANCE_RAMP_S).min(1.0);

        // Two waves of at most one each make a point within a square; the root of a half
        // brings that square's corners onto the circle of the radius.
        let (side, up, head_start) = self.shape;
        let radius = DANCE_RADIUS_DEG * mix(DANCE_SILENT_SIZE, 1.0, music) * self.size * FRAC_1_SQRT_2;
        (
            radius * (TAU * side * self.phase).sin(),
            radius * (TAU * (up * self.phase + head_start)).sin(),
        )
    }

    /// Stops the dance where it is, to start small again the next time the head is on the ball.
    fn rest(&mut self) {
        self.push_hz = 0.0;
        self.seen_onsets = None;
        self.size = 0.0;
    }
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
            let rest = (LEGS[index].pose)(&presets::LOOK_AT_HEAVEN);
            Head {
                slew: Slew::new(rest, GLIDE),
                arrived: false,
                dance: Dance::new(DANCE_SHAPES[index]),
            }
        };
        Self {
            heads: [head(0), head(1), head(2)],
            snap_number: 0,
            snap_at_s: None,
        }
    }

    /// Fills the heads' channels for this frame, `dt` seconds after the last, with `sparkle` the
    /// modulation the washes follow and `features` what the ear hears.
    pub fn fill(&mut self, slots: &mut [u8], cue: Cue, sparkle: f64, features: &AudioFeatures, dt: f64) {
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

            // Once on the ball, the dance moves the head itself: its path is smooth already, so
            // it goes out as it is rather than through the glide's speed limit. Jumping the slew
            // keeps it the one record of where the head was sent, so the glide out of dusk
            // starts from wherever the dot is.
            if cue.state == ShowState::Base(Base::Dusk) && head.arrived {
                let offset = head.dance.step(features, dt);
                head.slew.jump(on_ball((leg.pose)(&presets::LOOK_AT_BALL), offset));
            } else {
                head.dance.rest();
            }

            let (colour, level) = match cue.state {
                ShowState::Base(Base::Blackout | Base::Daylight) => (Colour::White, 0.0),
                ShowState::Base(Base::Dusk) => (leg.colour, DUSK_BALL_LEVEL * (1.0 - sparkle)),
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
            ShowState::Base(Base::Dusk) | ShowState::Incident(Incident::BallLighting) => {
                &presets::LOOK_AT_BALL
            }
            ShowState::Base(Base::Blackout | Base::Daylight)
            | ShowState::Incident(Incident::Searchlights) => &presets::LOOK_AT_HEAVEN,
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

/// The pose that puts a head's dot `side` and `up` degrees from the ball's centre, `centre`
/// being the pose that aims at it.
///
/// The heads are mounted on their sides, facing the ball along their pan axis. Tilt swings the
/// beam away from that axis and moves the dot one for one. Pan turns the beam around the axis,
/// so with the ball only a little way off it, a degree of pan moves the dot sideways by just the
/// sine of that angle; the pan is scaled up by the same to make up for it. How far the ball sits
/// off the axis is read from the preset, taking the middle of the tilt range as the axis.
fn on_ball(centre: Pose, (side, up): (f64, f64)) -> Pose {
    let off_axis_deg = (centre.tilt_deg - PoseidonLaserBeamXs::TILT_RANGE_DEG / 2.0)
        .abs()
        .max(DANCE_MIN_OFF_AXIS_DEG);
    Pose::new(centre.pan_deg + side / off_axis_deg.to_radians().sin(), centre.tilt_deg + up)
}

fn mix(from: f64, to: f64, amount: f64) -> f64 {
    from + (to - from) * amount
}
