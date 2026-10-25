// When the claw plays, as local wall-clock times in the city it stands in. Read by `build.rs`,
// which resolves it into instants and dark stretches; the brain never sees these strings.
//
// Shows are listed in order and must not overlap. A time inside the hour the clocks skip or
// pass twice is refused, because it names no single moment.
//
// Each show runs through the day so the rig is up and reporting long before it is needed; it
// stays dark by daylight, and the show proper starts at civil dusk.

const CITY: &str = "Delft";

const SHOWS: &[(&str, &str)] = &[
    // Test day.
    ("2026-10-26 09:00", "2026-10-26 22:00"),
    // Halloween.
    ("2026-10-31 09:00", "2026-10-31 22:00"),
];
