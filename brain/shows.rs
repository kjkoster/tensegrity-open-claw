// When the claw plays, as local wall-clock times in the city it stands in. Read by `build.rs`,
// which resolves it into instants and dark stretches; the brain never sees these strings.
//
// Shows are listed in order and must not overlap. A time inside the hour the clocks skip or
// pass twice is refused, because it names no single moment.
//
// A `DuskShowDay` plays only between civil dusk and civil dawn inside its range; a `TestDay`
// plays for the whole range, light or dark. Outside a show the rig is dark, and up and
// reporting all the same.

const CITY: &str = "Delft";

const SHOWS: &[Day] = &[
    // Headless Hug PoC.
    TestDay("2026-11-14 08:00", "2026-11-14 23:00"),
    TestDay("2026-11-15 09:00", "2026-11-15 23:00"),
    TestDay("2026-11-16 09:00", "2026-11-16 23:00"),
];
