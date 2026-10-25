// The pointing presets the show is written against, by their names in QLC+. Read by `build.rs`,
// which refuses a workspace whose scenes are not exactly this set, and turns each name into a
// constant in `presets`: "Rose" becomes `presets::ROSE`.
//
// Each is a scene holding only Pan, Pan Fine, Tilt and Tilt Fine for every head.
//
// Empty while the placeholder show stands in: the workspace holds no scenes yet.

const PRESETS: &[&str] = &[];
