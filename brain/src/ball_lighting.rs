//! Ball Lighting's one fade, which heads, washes and ball all follow.

use crate::show_state::Cue;

/// From white to colour, and as long again back; no hold at either end.
const HALF_PERIOD_S: f64 = 5.0;

/// Where the fade is: 0 at white, 1 at colour. Counted from the state's start, so a restart
/// resumes it where it was.
pub fn phase(cue: &Cue) -> f64 {
    let t = (cue.state_elapsed / HALF_PERIOD_S).rem_euclid(2.0);
    if t <= 1.0 { t } else { 2.0 - t }
}
