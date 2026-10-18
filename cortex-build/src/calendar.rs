//! Resolves a rig's show table into the instants the running brain compares its clock to.
//!
//! The table is written the way a show is announced: a city, and local start and stop times.
//! Everything that turns that into instants — where the city is, which zone it keeps, which
//! offset each time falls in, when civil twilight ends and begins again — is settled here, so
//! the brain is handed a complete set of timestamps and carries no time-zone database and no
//! solar arithmetic. It also turns the cases that have no single answer into build errors
//! rather than show-night surprises: a local time the clocks skip, a local time they pass
//! twice, a city the gazetteer holds more than one of.
//!
//! Dark is civil dusk to civil dawn, the sun 6° below the horizon, rather than sunset to
//! sunrise: the light only reads once it is actually dark, which is half an hour after the
//! sun has gone.

use chrono::NaiveDate;
use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use sunrise::{Coordinates, DawnType, SolarDay, SolarEvent};

/// One show, resolved.
struct Show {
    start: Timestamp,
    stop: Timestamp,
    /// The stretches of the show between civil dusk and civil dawn, clipped to the show.
    dark: Vec<(Timestamp, Timestamp)>,
}

/// Generates `shows.rs` into `OUT_DIR`: a `shows` module holding the site, and every show in
/// `shows` with its dark stretches, as one `CALENDAR`.
///
/// `city` is a name from the `cities` gazetteer, or `"City, Country"` where the name alone
/// is not unique. `shows` are local `(start, stop)` pairs such as
/// `("2026-10-21 13:00", "2026-10-21 22:00")`, in order and not overlapping.
pub fn calendar(city: &str, shows: &[(&str, &str)]) {
    let place = find_city(city);
    let zone = tzf_rs::DefaultFinder::new()
        .get_tz_name(place.longitude, place.latitude)
        .to_string();
    if zone.is_empty() {
        panic!(
            "{}, {} lies in no time zone the zone finder knows",
            place.city, place.country
        );
    }
    let tz = TimeZone::get(&zone).unwrap_or_else(|e| {
        panic!("the time-zone database on this machine has no {zone}: {e}")
    });
    let coordinates = Coordinates::new(place.latitude, place.longitude).unwrap_or_else(|| {
        panic!(
            "the gazetteer places {} at {}, {}, which are not coordinates",
            place.city, place.latitude, place.longitude
        )
    });

    let mut resolved: Vec<Show> = Vec::new();
    for &(start_text, stop_text) in shows {
        let start = resolve(&tz, start_text);
        let stop = resolve(&tz, stop_text);
        if stop <= start {
            panic!("the show from {start_text} to {stop_text} does not end after it starts");
        }
        // In order as well as apart, so the table reads as the run of the event does and one
        // comparison with the show above is the whole overlap check.
        if let Some(previous) = resolved.last()
            && start < previous.stop
        {
            panic!(
                "the show starting {start_text} starts before the show above it ends: shows \
                 are listed in order and must not overlap"
            );
        }
        let dark = dark_stretches(&tz, coordinates, start, stop);
        resolved.push(Show { start, stop, dark });
    }

    let site = format!("{}, {}", place.city, place.country);
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(
        out_dir.join("shows.rs"),
        render(&site, place, &zone, &tz, &resolved),
    )
    .unwrap();
}

/// The one city `name` means, as `"City"` or `"City, Country"`.
fn find_city(name: &str) -> &'static cities::City {
    let (city, country) = match name.split_once(',') {
        Some((city, country)) => (city.trim(), Some(country.trim())),
        None => (name.trim(), None),
    };
    let matches: Vec<&cities::City> = cities::all()
        .iter()
        .filter(|c| c.city == city && country.is_none_or(|country| c.country == country))
        .collect();
    match matches.as_slice() {
        [one] => one,
        [] => panic!("the gazetteer has no city {name:?}"),
        several => {
            let options: Vec<String> = several
                .iter()
                .map(|c| format!("{:?}", format!("{}, {}", c.city, c.country)))
                .collect();
            panic!(
                "the gazetteer has more than one {name:?}; name it as one of {}",
                options.join(", ")
            )
        }
    }
}

/// The instant a local time in the table names, or a build error saying why it names none or
/// more than one.
fn resolve(tz: &TimeZone, text: &str) -> Timestamp {
    let local: DateTime = text.parse().unwrap_or_else(|e| {
        panic!("{text:?} is not a local date and time like \"2026-10-21 13:00\": {e}")
    });
    tz.to_ambiguous_zoned(local)
        .unambiguous()
        .unwrap_or_else(|e| {
            panic!(
                "{text:?} falls in an hour the clocks skip or pass twice, so it means no one \
                 moment; move it out of that hour: {e}"
            )
        })
        .timestamp()
}

/// Every stretch between civil dusk and the next civil dawn that falls inside the show.
///
/// Starts from the evening before the show's first local date, because a show that opens
/// after midnight opens inside the previous evening's dark.
fn dark_stretches(
    tz: &TimeZone,
    coordinates: Coordinates,
    start: Timestamp,
    stop: Timestamp,
) -> Vec<(Timestamp, Timestamp)> {
    let last = stop.to_zoned(tz.clone()).date();
    let mut evening = start.to_zoned(tz.clone()).date().yesterday().unwrap();
    let mut stretches = Vec::new();
    while evening <= last {
        let morning = evening.tomorrow().unwrap();
        let dusk = solar_event(coordinates, evening, SolarEvent::Dusk(DawnType::Civil));
        let dawn = solar_event(coordinates, morning, SolarEvent::Dawn(DawnType::Civil));
        let (from, to) = (dusk.max(start), dawn.min(stop));
        if from < to {
            stretches.push((from, to));
        }
        evening = morning;
    }
    stretches
}

fn solar_event(coordinates: Coordinates, date: Date, event: SolarEvent) -> Timestamp {
    let day = NaiveDate::from_ymd_opt(
        i32::from(date.year()),
        date.month() as u32,
        date.day() as u32,
    )
    .unwrap();
    let time = SolarDay::new(coordinates, day)
        .event_time(event)
        .unwrap_or_else(|| {
            panic!(
                "the sun does not reach civil twilight at {}, {} on {date}, so dark has no \
                 edge there",
                coordinates.lat(),
                coordinates.lon()
            )
        });
    Timestamp::from_second(time.timestamp()).unwrap()
}

fn render(site: &str, place: &cities::City, zone: &str, tz: &TimeZone, shows: &[Show]) -> String {
    let mut out = String::new();
    writeln!(out, "// Generated from the rig's show table. Do not edit.").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "pub mod shows {{").unwrap();
    writeln!(out, "    use cortex::calendar::{{Calendar, Show, Site, Span, Stamp}};").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    pub static CALENDAR: Calendar = Calendar {{").unwrap();
    writeln!(out, "        site: Site {{").unwrap();
    writeln!(out, "            city: {site:?},").unwrap();
    writeln!(out, "            latitude: {:?},", place.latitude).unwrap();
    writeln!(out, "            longitude: {:?},", place.longitude).unwrap();
    writeln!(out, "            zone: {zone:?},").unwrap();
    writeln!(out, "        }},").unwrap();
    writeln!(out, "        shows: &[").unwrap();
    for show in shows {
        writeln!(out, "            Show {{").unwrap();
        writeln!(out, "                span: {},", span(tz, show.start, show.stop)).unwrap();
        writeln!(out, "                dark: &[").unwrap();
        for &(from, to) in &show.dark {
            writeln!(out, "                    {},", span(tz, from, to)).unwrap();
        }
        writeln!(out, "                ],").unwrap();
        writeln!(out, "            }},").unwrap();
    }
    writeln!(out, "        ],").unwrap();
    writeln!(out, "    }};").unwrap();
    writeln!(out, "}}").unwrap();
    out
}

fn span(tz: &TimeZone, start: Timestamp, stop: Timestamp) -> String {
    format!(
        "Span {{ start: {}, stop: {} }}",
        stamp(tz, start),
        stamp(tz, stop)
    )
}

/// An instant as Unix seconds, and as the local time it is in the site's zone — with the
/// offset and whether it is summer time, because a run of shows can straddle the clock change
/// and the log is where that has to be legible.
fn stamp(tz: &TimeZone, at: Timestamp) -> String {
    let local = at.to_zoned(tz.clone());
    let season = if tz.to_offset_info(at).dst().is_dst() {
        "summer time"
    } else {
        "winter time"
    };
    let text = format!(
        "{} ({}, {season})",
        local.strftime("%Y-%m-%d %H:%M %Z"),
        local.strftime("%:z")
    );
    let time = local.strftime("%H:%M").to_string();
    format!(
        "Stamp {{ unix: {}, local: {text:?}, time: {time:?} }}",
        at.as_second()
    )
}
