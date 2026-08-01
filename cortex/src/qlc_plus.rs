//! A small model of QLC+'s fixture vocabulary — enough of it to describe the fixtures we
//! patch, and no more.
//!
//! QLC+ splits a fixture across two files. The workspace (`claw.qxw`) says *what is
//! patched where*: manufacturer, model, mode, address. The fixture definition (`.qxf`) says
//! *what the channels of that mode mean*: which one is red, which is a strobe, which is a
//! colour wheel. Together they answer "slot 2 of this fixture is its green emitter", which
//! neither file answers alone.
//!
//! `build.rs` reads both and generates one struct per patched fixture into `patch.rs`, with
//! a named field per channel of its mode. The types below are what those structs are built
//! from. The point is that channel roles become part of the type: a fixture whose mode has
//! no red channel has no `red` field and does not implement [`Rgb`], so wiring the sparkle
//! engine to it is a compile error rather than a fixture that lights the wrong colour.

/// One band of a channel's range, as the definition describes it.
///
/// A `.qxf` states what a fixture does over a span of DMX values, and that statement is the
/// only place the fact exists — a value typed into the show instead is a copy of it, right
/// until the fixture is swapped for one whose bands sit elsewhere. Carrying the bands means
/// the show can ask which values mean something and pick one for a reason it can state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub min: u8,
    pub max: u8,
    /// QLC+'s preset for the band, where it has one. The presets are a small shared
    /// vocabulary across every manufacturer, which is what lets a lookup work on a fixture
    /// nobody has read the manual for.
    pub preset: Option<&'static str>,
    /// The band's label in the definition.
    pub name: &'static str,
}

impl Capability {
    /// The middle of the band — what to send when the band *is* the command, as a wheel
    /// slot is. Fixtures round their own boundaries, and definitions are transcribed by
    /// hand, so the edges are where the two disagree.
    pub const fn center(&self) -> u8 {
        self.min + (self.max - self.min) / 2
    }
}

/// The preset marking a band over which a channel does nothing at all. Not "a safe value" —
/// a dead band, which is a stronger claim and the only one a park can rely on.
const NO_FUNCTION: &str = "NoFunction";

/// One DMX channel of a patched fixture, as an absolute 0-based slot in the universe.
///
/// The fixture's start address is already folded in at generation time, so nothing at run
/// time does address arithmetic — which is where off-by-one patch bugs live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    slot: u16,
    capabilities: &'static [Capability],
}

impl Channel {
    /// Only `build.rs`'s output calls this; both arguments are derived from the QLC+ patch
    /// and the fixture definition it names.
    pub const fn at(slot: u16, capabilities: &'static [Capability]) -> Self {
        Self { slot, capabilities }
    }

    pub fn slot(self) -> usize {
        self.slot as usize
    }

    pub fn set(self, slots: &mut [u8], value: u8) {
        slots[self.slot as usize] = value;
    }

    /// Writes a 0..1 value as a DMX byte. Rounds rather than truncates, to match the
    /// scaling fixtures apply on the way back out.
    pub fn set_unit(self, slots: &mut [u8], value: f64) {
        self.set(slots, (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    }

    /// Sends the channel to a band where it does nothing.
    ///
    /// For the channels a show must never let drift — auto programs, resets, anything that
    /// makes a head self-propelled — asking the definition where "nothing" lives beats
    /// writing a zero, because the zero is only right until a fixture arrives whose dead
    /// band is elsewhere. The low end of the band rather than its middle, so what we send
    /// matches what every unpatched slot on the wire already carries: a frame that loses
    /// this fixture's block lands the channel in the same place we were holding it.
    ///
    /// A channel whose definition declares no dead band falls back to zero, which is that
    /// same wire default and is what the fixture sees before the daemon starts.
    pub fn park(self, slots: &mut [u8]) {
        let value = self
            .capabilities
            .iter()
            .find(|capability| capability.preset == Some(NO_FUNCTION))
            .map_or(0, |capability| capability.min);
        self.set(slots, value);
    }
}

/// A coarse channel and the fine channel that extends it, written as one value.
///
/// Two bytes are one number to everything above the wire, and splitting them at the call
/// site is how a 16-bit fixture ends up driven at 8 bits: the coarse write alone is valid
/// code that compiles, runs, and steps visibly. There is no coarse-only form of this type —
/// a mode whose definition omits its fine channels yields a plain [`Channel`] and does not
/// implement [`Position`], so the fixture that cannot be driven smoothly fails to build
/// rather than moving in stairs nobody notices until the rig is lit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Channel16 {
    pub coarse: Channel,
    pub fine: Channel,
}

impl Channel16 {
    pub const fn pair(coarse: Channel, fine: Channel) -> Self {
        Self { coarse, fine }
    }

    pub fn set(self, slots: &mut [u8], value: u16) {
        self.coarse.set(slots, (value >> 8) as u8);
        self.fine.set(slots, value as u8);
    }

    /// Writes a 0..1 value across the pair.
    pub fn set_unit(self, slots: &mut [u8], value: f64) {
        self.set(slots, (value.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
    }
}

/// One row of the generated patch table: a fixture as the workspace describes it.
///
/// The per-fixture structs are the typed interface; this is the same information as plain
/// data, so the daemon can log the patch it was built against without naming every fixture
/// by hand — which would put a second, hand-maintained copy of the patch in the source.
pub struct PatchEntry {
    /// The fixture's name in the QLC+ workspace.
    pub name: &'static str,
    /// Manufacturer, model and mode, as the workspace names them.
    pub profile: &'static str,
    /// 1-based DMX start address.
    pub address: u16,
    /// Every channel of the patched mode, in mode order.
    ///
    /// Channels only, not roles: what each one *is* lives in the fixture struct's field
    /// names and its capability traits, where the compiler checks it. Carrying a runtime
    /// copy of the same fact would be a second encoding able to drift from the first.
    pub channels: &'static [Channel],
}

/// A fixture with additive red, green and blue emitters.
///
/// Implemented only when the patched mode carries all three, so a fixture patched into a
/// mode without them cannot be handed to code that mixes colour.
pub trait Rgb {
    fn red(&self) -> Channel;
    fn green(&self) -> Channel;
    fn blue(&self) -> Channel;
}

/// A fixture with a discrete white emitter, separate from its RGB mix.
pub trait White {
    fn white(&self) -> Channel;
}

/// A fixture with a master dimmer, so intensity need not be folded into the colours.
pub trait Dimmer {
    fn dimmer(&self) -> Channel;
}

/// A fixture that can be aimed: 16-bit pan and tilt, over a range its definition states.
///
/// The ranges are associated constants rather than numbers in the show because they are the
/// difference between a rental and this head. Everything above this trait speaks degrees, so
/// a wider head runs the same code at the same apparent speed instead of compiling happily
/// and moving half as fast.
///
/// Degrees are unreferenced: zero is wherever the fixture's own zero falls, which depends on
/// how the head is clamped and how the stand sits that night. Nothing here needs an absolute
/// origin; the recorded points are the reference, and they are recorded per setup.
pub trait Position {
    const PAN_RANGE_DEG: f64;
    const TILT_RANGE_DEG: f64;

    fn pan(&self) -> Channel16;
    fn tilt(&self) -> Channel16;
}
