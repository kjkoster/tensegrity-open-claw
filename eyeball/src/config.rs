//! The eyeball's tunables.

/// How sure the detector must be that a box is a person: objectness times the person class.
/// Low on purpose. A false box can only land between the two columns, and the show debounces
/// everything, so a missed child costs more than a phantom one.
pub const CONFIDENCE: f32 = 0.3;

/// The overlap above which two boxes are taken for one person. Loose on purpose: children seen
/// from the side overlap, and the usual 0.45 would merge two of them into one box, losing the
/// one in front.
pub const OVERLAP: f32 = 0.6;

/// Where the trigger line, by the rose, falls across the picture, as a fraction of its width.
/// Set by aiming the camera, watching the feed, until the rose lands on it; these are
/// placeholders until then.
pub const TRIGGER_COLUMN: f32 = 0.25;

/// Where the yard's entry from the street falls across the picture, likewise. Either side of
/// `TRIGGER_COLUMN`: the leading foot is whichever bottom corner of a box faces the trigger.
pub const ENTRY_COLUMN: f32 = 0.85;

/// The detector's threads for its matrix products, one per background core.
pub const MODEL_THREADS: usize = 3;
