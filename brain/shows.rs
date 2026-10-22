// When the claw plays, as local wall-clock times in the city it stands in. Read by `build.rs`,
// which resolves it into instants and dark stretches; the brain never sees these strings.
//
// Shows are listed in order and must not overlap. A time inside the hour the clocks skip or
// pass twice is refused, because it names no single moment.

const CITY: &str = "Amsterdam";

const SHOWS: &[(&str, &str)] = &[
    // Build day, for testing and tuning.
    ("2026-10-22 16:00", "2026-10-22 22:00"),
    // ADE, Friday and Saturday nights. Sunday is the tear-down.
    ("2026-10-23 16:00", "2026-10-24 06:00"),
    ("2026-10-24 16:00", "2026-10-25 06:00"),
];
