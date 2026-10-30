//! The view a consumer asks of the retina: the picture it needs, within what the camera offers.
//!
//! The consumer says what it needs and the retina sets the camera up for it, so a different
//! consumer with different needs takes no change here. The bounds are the camera's own and are
//! checked where they are written: a resolution it does not list is not a variant, and a frame
//! rate out of range does not compile in a constant.

/// The pictures the camera offers, as it lists them in `Properties.Image.Resolution`. The
/// sensor is 16:9; the 4:3 and 16:10 ones crop the sides off its view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    // 16:9, the whole view.
    R1920x1080,
    R1280x720,
    R800x450,
    R640x360,
    R480x270,
    R320x180,
    R240x135,
    R160x90,
    // 4:3.
    R1280x960,
    R1024x768,
    R800x600,
    R640x480,
    R480x360,
    R320x240,
    R240x180,
    R160x120,
    // 16:10.
    R1280x800,
    R1024x640,
    R800x500,
    R640x400,
    R480x300,
    R320x200,
    R160x100,
}

impl Resolution {
    /// Width by height, in pixels.
    pub const fn size(self) -> (u32, u32) {
        match self {
            Resolution::R1920x1080 => (1920, 1080),
            Resolution::R1280x720 => (1280, 720),
            Resolution::R800x450 => (800, 450),
            Resolution::R640x360 => (640, 360),
            Resolution::R480x270 => (480, 270),
            Resolution::R320x180 => (320, 180),
            Resolution::R240x135 => (240, 135),
            Resolution::R160x90 => (160, 90),
            Resolution::R1280x960 => (1280, 960),
            Resolution::R1024x768 => (1024, 768),
            Resolution::R800x600 => (800, 600),
            Resolution::R640x480 => (640, 480),
            Resolution::R480x360 => (480, 360),
            Resolution::R320x240 => (320, 240),
            Resolution::R240x180 => (240, 180),
            Resolution::R160x120 => (160, 120),
            Resolution::R1280x800 => (1280, 800),
            Resolution::R1024x640 => (1024, 640),
            Resolution::R800x500 => (800, 500),
            Resolution::R640x400 => (640, 400),
            Resolution::R480x300 => (480, 300),
            Resolution::R320x200 => (320, 200),
            Resolution::R160x100 => (160, 100),
        }
    }
}

/// A frame rate the camera can deliver: 1 to 25 a second, 25 being the most it captures on
/// 50 Hz mains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramesPerSecond(u32);

impl FramesPerSecond {
    pub const MAX: u32 = 25;

    /// In a constant, a rate out of range does not compile:
    ///
    /// ```compile_fail
    /// use retina::view::FramesPerSecond;
    /// const TOO_FAST: FramesPerSecond = FramesPerSecond::new(30);
    /// let _ = TOO_FAST;
    /// ```
    pub const fn new(rate: u32) -> FramesPerSecond {
        assert!(
            rate >= 1 && rate <= Self::MAX,
            "the camera delivers 1 to 25 frames a second"
        );
        FramesPerSecond(rate)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// What the picture is lit by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    /// The camera's own 850 nm illuminator, with the IR-cut filter out: a grey picture that
    /// visible light, the show's included, barely moves.
    Infrared,
    /// Visible light, with the IR-cut filter in: colour, as the eye sees it.
    Colour,
}

/// Which way the camera hangs, seen from behind it, and so how its picture has to be turned to
/// come out the right way up: the four the camera can turn it by, `Properties.Image.Rotation`.
///
/// On its side, the camera sends a portrait picture, the resolution asked for with its width
/// and height swapped: Axis's corridor format, for a view that is taller than it is wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Standing on its base, as it ships: the picture as the sensor sees it.
    Upright,
    /// Turned a quarter anticlockwise onto its left side; the picture turned a quarter
    /// clockwise.
    OnLeftSide,
    /// Hung from its base, under an overhang; the picture turned half a turn.
    UpsideDown,
    /// Turned a quarter clockwise onto its right side; the picture turned a quarter
    /// anticlockwise.
    OnRightSide,
}

impl Orientation {
    /// The camera's `Image.I0.Appearance.Rotation`: degrees clockwise.
    pub const fn rotation(self) -> &'static str {
        match self {
            Orientation::Upright => "0",
            Orientation::OnLeftSide => "90",
            Orientation::UpsideDown => "180",
            Orientation::OnRightSide => "270",
        }
    }

    /// Whether the picture comes out portrait.
    pub const fn sideways(self) -> bool {
        matches!(self, Orientation::OnLeftSide | Orientation::OnRightSide)
    }
}

/// The picture a consumer needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub resolution: Resolution,
    pub rate: FramesPerSecond,
    pub light: Light,
    pub orientation: Orientation,
}
