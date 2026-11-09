//! A small model of QLC+'s fixture vocabulary — enough of it to describe the fixtures we
//! patch, and no more.
//!
//! QLC+ splits a fixture across two files. The workspace (`clockwork.qxw`) says *what is
//! patched where*: manufacturer, model, mode, address. The fixture definition (`.qxf`) says
//! *what the channels of that mode mean*: which one is red, which is a strobe, which is a
//! colour wheel. Together they answer "slot 2 of this fixture is its green emitter", which
//! neither file answers alone.
//!
//! `build.rs` reads both and generates one struct per patched fixture into `patch.rs`, with
//! a named field per channel of its mode. The types below are what those structs are built
//! from. The point is that channel roles become part of the type: a fixture whose mode has
//! no red channel has no `red` field and does not implement [`Rgb`], so driving its colour
//! is a compile error rather than a fixture that lights the wrong channel.

use core::net::Ipv4Addr;

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

/// One DMX channel of a patched fixture, as an absolute 0-based slot in the universe.
///
/// The fixture's start address is already folded in at generation time, so nothing at run
/// time does address arithmetic — which is where off-by-one patch bugs live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    slot: u16,
}

impl Channel {
    /// Only `build.rs`'s output calls this, with the slot derived from the QLC+ patch.
    pub const fn at(slot: u16) -> Self {
        Self { slot }
    }

    pub fn slot(self) -> usize {
        self.slot as usize
    }

    pub fn set(self, slots: &mut [u8], value: u8) {
        slots[self.slot as usize] = value;
    }

    /// What this channel is carrying.
    ///
    /// The one thing that reads rather than writes: a look driven by hand from a console and
    /// saved as a scene is a measurement, and getting it back out means asking the same
    /// channel that would have written it. Kept beside [`Self::set`] so the two stay each
    /// other's inverse where a reader can see both at once.
    pub fn get(self, slots: &[u8]) -> u8 {
        slots[self.slot as usize]
    }

    /// Writes a 0..1 value as a DMX byte. Rounds rather than truncates, to match the
    /// scaling fixtures apply on the way back out.
    pub fn set_unit(self, slots: &mut [u8], value: f64) {
        self.set(slots, (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    }
}

/// A coarse channel and the fine channel that extends it, written as one value.
///
/// Two bytes are one number to everything above the wire, and splitting them at the call
/// site is how a 16-bit fixture ends up driven at 8 bits: the coarse write alone is valid
/// code that compiles, runs, and steps visibly. There is no coarse-only form of this type: a
/// mode whose definition omits its fine channels yields a plain [`Channel`].
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

/// The universe a rig lives on, the port it is carried over and where a console reaches it,
/// as the E1.31 output in its workspace states them.
///
/// That output is the console's: it says where QLC+ sends when it takes the rig over. A rig
/// has one universe and a takeover is relayed onto it, so the universe and the port are what
/// the daemon listens for and what it sends.
#[derive(Clone, Copy)]
pub struct Sacn {
    pub universe: u16,
    pub port: u16,
    /// The address the console unicasts to, and the one address the receiver binds. The rig's
    /// `clockwork.dot` says where that is; the build fails a workspace that unicasts elsewhere.
    ///
    /// Deliberately one address and not the wildcard. Binding a single address is what keeps
    /// the daemon's own sACN multicast, which it transmits at frame rate, from being
    /// delivered straight back to its own receiver. There are no multicast joins for the
    /// same reason: a console on a tunnel with no multicast has to unicast here regardless.
    ///
    /// Two consequences worth knowing. A console arriving on any other interface cannot take
    /// over. And if the interface holding this address is down at startup the bind fails,
    /// which is wanted: the receiver retries with backoff and comes up whenever it does.
    pub address: Ipv4Addr,
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
