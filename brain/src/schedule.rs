//! When the incidents happen: a schedule per dark stretch, drawn afresh from its key every
//! frame, so nothing about it is stored and a restart mid-show lands back in the same night.
//!
//! The first incident comes a gap after dusk, and each next one a gap after the last ends. An
//! incident starts only if it would end a margin before the dark stretch does — which covers
//! both dawn and the show's stop, since a dark stretch never runs past the stop — and the
//! first one that would not ends the stretch's schedule, since every later one would end later
//! still.
//!
//! The clock this is measured against is the wall clock, so an NTP step mid-stretch moves the
//! schedule with it and can cut an incident short or open one partway through. That is the
//! calendar's own stance: behind is safe, and the next frame is right.

use crate::show_state::Incident;
use cortex::calendar::Span;
use cortex::draw::Draw;

/// The incidents the schedule picks from: every one there is.
const INCIDENTS: [Incident; 2] = [Incident::Searchlights, Incident::BallLighting];

/// From dusk, or from the end of the last incident, to the start of the next.
const GAP_S: (f64, f64) = (30.0 * 60.0, 45.0 * 60.0);
const DURATION_S: (f64, f64) = (2.0 * 60.0, 4.0 * 60.0);
/// How long before the dark stretch ends an incident must be over.
const END_MARGIN_S: f64 = 20.0 * 60.0;

const SEED: u64 = 0x1dc1_de47_0c0f_fee5;

/// One incident in a dark stretch's schedule.
#[derive(Clone, Copy)]
pub struct Scheduled {
    /// Its place in the stretch's schedule, from 0: with the stretch's start, its key.
    pub number: i64,
    pub incident: Incident,
    /// Unix seconds.
    pub start: f64,
    pub end: f64,
}

/// A dark stretch's schedule, in order.
pub fn plan(stretch: Span) -> Plan {
    Plan {
        stretch,
        number: 0,
        from: Some(stretch.start.unix as f64),
    }
}

/// The incident running at `now` in `stretch`, if one is.
pub fn running(stretch: Span, now: f64) -> Option<Scheduled> {
    plan(stretch)
        .take_while(|scheduled| scheduled.start <= now)
        .find(|scheduled| now < scheduled.end)
}

pub struct Plan {
    stretch: Span,
    number: i64,
    /// Where the next gap is measured from; `None` once the schedule has ended.
    from: Option<f64>,
}

impl Iterator for Plan {
    type Item = Scheduled;

    fn next(&mut self) -> Option<Scheduled> {
        let from = self.from?;
        // The times are drawn before the pick, so adding or removing an incident changes which
        // ones run but not when.
        let mut draw = Draw::new(SEED, &[self.stretch.start.unix, self.number]);
        let start = from + draw.between(GAP_S.0, GAP_S.1);
        let end = start + draw.between(DURATION_S.0, DURATION_S.1);
        let incident = INCIDENTS[draw.index(INCIDENTS.len())];
        if end > self.stretch.stop.unix as f64 - END_MARGIN_S {
            self.from = None;
            return None;
        }
        let scheduled = Scheduled {
            number: self.number,
            incident,
            start,
            end,
        };
        self.number += 1;
        self.from = Some(end);
        Some(scheduled)
    }
}
