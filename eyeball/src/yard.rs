//! The yard: how far into it the furthest person has come.
//!
//! A distance is in paths: 1 at the entry column, 0 at the trigger column by the rose, negative
//! past it. Linear in the picture, which is linear in the yard as far as the camera is square
//! to the path. Nobody between the columns is the tombstone, `+∞`, so every comparison the show
//! makes falls the safe way.

use crate::config as cfg;
use crate::detect::Person;

/// Where a person's leading foot falls, in paths: the bottom corner of their box on the
/// trigger's side, which on a box merged over a bunch is still the one in front.
fn foot(person: &Person, width: u32) -> f32 {
    let corner = if cfg::TRIGGER_COLUMN < cfg::ENTRY_COLUMN {
        person.left
    } else {
        person.right
    };
    (corner / width as f32 - cfg::TRIGGER_COLUMN) / (cfg::ENTRY_COLUMN - cfg::TRIGGER_COLUMN)
}

/// The furthest person in, or `None` when nobody is: those past the entry column are in the
/// street and are left out.
pub fn furthest(people: &[Person], width: u32) -> Option<&Person> {
    people
        .iter()
        .filter(|person| foot(person, width) <= 1.0)
        .min_by(|a, b| foot(a, width).total_cmp(&foot(b, width)))
}

/// The furthest person's distance, or the tombstone.
pub fn distance(people: &[Person], width: u32) -> f64 {
    furthest(people, width).map_or(f64::INFINITY, |person| f64::from(foot(person, width)))
}

/// A distance as it is published and drawn: to a hundredth, or `tombstone`.
pub fn word(distance: f64) -> String {
    if distance.is_infinite() {
        "tombstone".to_string()
    } else {
        format!("{distance:.2}")
    }
}
