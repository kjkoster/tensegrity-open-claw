//! Turns the calibration scenes into each head's mount, and prints the file that carries them.
//!
//! The workflow it closes: drive all four heads onto a marker with the QLC+ sliders, save the
//! look as a scene named for that marker, repeat for every marker, deploy, run this. Out comes
//! the text of `geometry.rs` with the mounts solved, and a residual per marker saying how well
//! each head's answer explains what it was driven to.
//!
//! A binary beside the show rather than a tool of its own, so it reads the same generated
//! patch the show aims through: a recorded look becomes degrees by way of the very travel
//! constants that will turn those degrees back into DMX. A wrong constant is then wrong in
//! both directions and shows up as a fit that will not close, instead of cancelling itself out
//! here and reappearing on the field.
//!
//! Nothing is written. The output is pasted, which keeps a solve a thing somebody looked at
//! before the rig started believing it.

// The rig table lives beside the show, because the pairing of a head with its mount has to be
// made once and read twice. A binary is its own crate root, so it is reached by path rather
// than by `mod`.
#[path = "../geometry.rs"]
mod geometry;

use cortex::geometry::{Observation, Sense, Solution, residual_deg, solve};
use cortex::moving_head::pose_of;
use cortex::scenes::Scene;

include!(concat!(env!("OUT_DIR"), "/rig.rs"));

/// A full universe, so a scene can be laid into it wherever the workspace patched things.
const SLOTS: usize = 512;

fn main() {
    println!("// Solved from {} markers.", geometry::MARKERS.len());

    for head in geometry::HEADS {
        // Kept paired with the marker they came from, because a marker whose scene was never
        // saved drops out here — and a residual reported against the wrong marker name would
        // send somebody to re-measure a spot that was never the problem.
        let recorded: Vec<(&geometry::Marker, Observation)> = geometry::MARKERS
            .iter()
            .filter_map(|marker| {
                let scene = scene_named(marker.scene)?;
                let mut slots = [0u8; SLOTS];
                scene.apply(&mut slots);
                Some((
                    marker,
                    Observation {
                        pose: pose_of(head.fixture, &slots),
                        at: marker.at,
                    },
                ))
            })
            .collect();

        // A head with nothing recorded is the ordinary state of this tool before a setup day,
        // and saying so beats printing a mount fitted to no evidence at all.
        if recorded.is_empty() {
            println!("// {}: no calibration scenes found.", head.name);
            continue;
        }

        let observations: Vec<Observation> = recorded.iter().map(|(_, seen)| *seen).collect();
        let solution = solve(head.mount, &observations);
        report(head, &solution, &recorded);
    }
}

fn scene_named(name: &str) -> Option<&'static Scene> {
    scenes::SCENES.iter().find(|scene| scene.name == name)
}

/// Prints one head's answer: the mount as Rust, then what it still misses by.
///
/// The residuals go to standard error and the mount to standard output, so the file can be
/// captured while the evidence for it stays on the terminal being read.
fn report(
    head: &geometry::Head,
    solution: &Solution,
    recorded: &[(&geometry::Marker, Observation)],
) {
    let mount = &solution.mount;
    // The whole item rather than the mount alone, so the answer is pasted over a head instead
    // of threaded into the middle of one. The identifier is the head's own name shouted, which
    // is how the table spells it and how the generated patch spells the fixture beside it.
    let ident = head.name.to_uppercase();
    println!("pub static {ident}: Head = Head {{");
    println!("    name: \"{}\",", head.name);
    println!("    fixture: &patch::{ident},");
    println!("    mount: HeadMount {{");
    println!(
        "        position: Point::new({:.3}, {:.3}, {:.3}),",
        mount.position.x, mount.position.y, mount.position.z
    );
    println!(
        "        zero: Direction::new({:.3}, {:.3}),",
        mount.zero.bearing_deg, mount.zero.elevation_deg
    );
    println!("        pan: Sense::{},", sense(mount.pan));
    println!("        tilt: Sense::{},", sense(mount.tilt));
    println!("    }},");
    println!("}};");

    eprintln!(
        "{}: {:.3}° rms over {} markers",
        head.name,
        solution.rms_deg,
        recorded.len()
    );
    // Marker by marker rather than the total alone: one bad number is a marker mis-measured or
    // a head driven onto the wrong spot, while every number bad together is the mount itself —
    // a stand off level, or an axis reversed — and the two want different repairs.
    for (marker, observation) in recorded {
        eprintln!(
            "{}:   {:.3}° at {}",
            head.name,
            residual_deg(mount, observation),
            marker.scene
        );
    }
    // What setup asked for against what the field gave, in one line. A head that moved is a
    // head to go and look at, not a number to accept.
    eprintln!(
        "{}:   {:.2} m from where the plan put it",
        head.name,
        head.mount.position.distance_to(mount.position)
    );
}

fn sense(sense: Sense) -> &'static str {
    match sense {
        Sense::Forward => "Forward",
        Sense::Reversed => "Reversed",
    }
}
