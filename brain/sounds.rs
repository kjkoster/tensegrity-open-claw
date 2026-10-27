// The recordings the show plays, each a file in `sounds/` as it was found, and the gain it
// plays at in dB. Read by `build.rs`, which compiles each into a constant named after its
// file: `heartbeat.m4a` becomes `sounds::HEARTBEAT`.
//
// Each recording is brought up to full scale when it is compiled and then down by its gain, so
// the gain is how loud it plays and nothing else is. 0 dB is the loudest a recording can be,
// and a gain above it fails the build; the speakers' own knobs set the level of the whole.

const SOUNDS: &[(&str, f64)] = &[
    ("heartbeat.m4a", 0.0),
    ("owl.mp3", 0.0),
    ("thunderclap.m4a", 0.0),
];
