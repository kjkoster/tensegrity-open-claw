// When the claw plays, as local wall-clock times in the city it stands in. Read by `build.rs`,
// which resolves it into instants and dark stretches; the brain never sees these strings.
//
// Shows are listed in order and must not overlap. A time inside the hour the clocks skip or
// pass twice is refused, because it names no single moment.
//
// The show plays from its start time to its end, dusk or not; outside a show the rig is dark,
// and up and reporting all the same.

const CITY: &str = "Delft";

const SHOWS: &[(&str, &str)] = &[
    // Test day.
    ("2026-10-27 09:00", "2026-10-27 22:00"),
    // Halloween.
    ("2026-10-31 18:15", "2026-10-31 22:00"),
];
