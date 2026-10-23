// The pointing presets the show is written against, by their names in QLC+. Read by `build.rs`,
// which refuses a workspace whose scenes are not exactly this set, and turns each name into a
// constant in `presets`: "Look at ball" becomes `presets::LOOK_AT_BALL`.
//
// Each is a scene holding only Pan, Pan Fine, Tilt and Tilt Fine for every head.

const PRESETS: &[&str] = &[
    // The heads on the ball: where they rest, dark, whenever no incident has them.
    "Look at ball",
    // Straight up, where Searchlights starts.
    "Look at heaven",
    // Opposite corners of each head's patch of sky, both in the same stretch of its pan range.
    // Searchlights snaps to random points in the box between them.
    "Sky corner A",
    "Sky corner B",
];
