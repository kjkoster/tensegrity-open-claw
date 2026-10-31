//! The eyeball's tunables, and what it asks of the retina.

use retina::view::{FramesPerSecond, Light, Orientation, Resolution, View};

/// The view the eyeball asks the retina for:
///
/// - 640×360, the camera's 16:9, which keeps the full width of its view, and twice the model's
///   320×192 input, so the letterbox halves it;
/// - 8 frames a second, a little above the 6.5 the detector manages on `claw-pi`, so it is
///   never kept waiting, and not so far above that the retina works for frames thrown away;
/// - infrared, so the show's own light barely moves the picture;
/// - upside down, as the camera hangs.
pub const VIEW: View = View {
    resolution: Resolution::R640x360,
    rate: FramesPerSecond::new(8),
    light: Light::Infrared,
    orientation: Orientation::UpsideDown,
};

/// How sure the detector must be that a box is a person: objectness times the person class.
/// Low on purpose. A false box can only land between the two columns, and the show debounces
/// everything, so a missed child costs more than a phantom one. And lower still in the
/// camera's infrared picture than in a photograph: grey and soft, it scores people far below
/// what the model gives them in daylight. At the bench a half-hidden person scored 0.12 and
/// the empty yard nothing above 0.02.
pub const CONFIDENCE: f32 = 0.09;

/// The overlap above which two boxes are taken for one person. Loose on purpose: children seen
/// from the side overlap, and the usual 0.45 would merge two of them into one box, losing the
/// one in front.
pub const OVERLAP: f32 = 0.6;

/// Where the trigger line, by the rose, falls across the picture, as a fraction of its width.
/// Set by aiming the camera, watching the feed, until the rose lands on it. For now just past
/// the front wheel of the left bike parked at the bench, at 31–38% of the width.
pub const TRIGGER_COLUMN: f32 = 0.40;

/// Where the yard's entry from the street falls across the picture, likewise. Either side of
/// `TRIGGER_COLUMN`: the leading foot is whichever bottom corner of a box faces the trigger.
pub const ENTRY_COLUMN: f32 = 0.85;

/// The detector's threads for its matrix products, one per background core.
pub const MODEL_THREADS: usize = 3;
