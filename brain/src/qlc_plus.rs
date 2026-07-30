//! A small model of QLC+'s fixture vocabulary — enough of it to describe the fixtures we
//! patch, and no more.
//!
//! QLC+ splits a fixture across two files. The workspace (`open-claw.qxw`) says *what is
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

/// One DMX channel of a patched fixture, as an absolute 0-based slot in the universe.
///
/// The fixture's start address is already folded in at generation time, so nothing at run
/// time does address arithmetic — which is where off-by-one patch bugs live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Channel(u16);

impl Channel {
    /// Only `build.rs`'s output calls this; the slot is derived from the QLC+ patch.
    pub const fn at(slot: u16) -> Self {
        Self(slot)
    }

    pub fn slot(self) -> usize {
        self.0 as usize
    }

    pub fn set(self, slots: &mut [u8], value: u8) {
        slots[self.0 as usize] = value;
    }

    /// Writes a 0..1 value as a DMX byte. Rounds rather than truncates, to match the
    /// scaling fixtures apply on the way back out.
    pub fn set_unit(self, slots: &mut [u8], value: f64) {
        self.set(slots, (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    }
}

/// The QLC+ channel presets we recognise, which is the Intensity family — the roles a
/// colour-mixing fixture is built from.
///
/// Everything else QLC+ can express (colour wheels, gobos, strobes, maintenance channels)
/// arrives as [`Preset::Custom`]. That is deliberate: those channels are indexed bands
/// rather than continuous levels, so a mixing engine cannot drive them by writing a level,
/// and pretending otherwise in the type system would be worse than leaving them unnamed.
/// They still get a field, named after the channel, so they can be set explicitly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    IntensityMasterDimmer,
    IntensityDimmer,
    IntensityRed,
    IntensityGreen,
    IntensityBlue,
    IntensityWhite,
    IntensityAmber,
    IntensityUV,
    IntensityCyan,
    IntensityMagenta,
    IntensityYellow,
    IntensityHue,
    IntensitySaturation,
    IntensityValue,
    IntensityLightness,
    Custom,
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
    /// Every channel of the patched mode, in mode order, with the role QLC+ gives it.
    pub channels: &'static [(Preset, Channel)],
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
