//! Reads the camera's address from the wired segment's DHCP reservations, so it is written in
//! one place: the reservation the Pi hands the camera is the address the retina looks for it at.

use std::env;
use std::fs;
use std::net::Ipv4Addr;
use std::path::PathBuf;

fn main() {
    let path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("etc")
        .join("claw-eth0.dnsmasq.conf");
    println!("cargo::rerun-if-changed={}", path.display());
    let reservations =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

    // `dhcp-host=<mac>,camera,<address>`: the line named `camera`.
    let address = reservations
        .lines()
        .filter_map(|line| line.trim().strip_prefix("dhcp-host="))
        .map(|host| host.split(',').map(str::trim).collect::<Vec<_>>())
        .find(|fields| fields.get(1) == Some(&"camera"))
        .and_then(|fields| fields.get(2).map(|address| address.to_string()))
        .unwrap_or_else(|| panic!("{}: no dhcp-host line named `camera`", path.display()));
    address
        .parse::<Ipv4Addr>()
        .unwrap_or_else(|e| panic!("{}: the camera's address {address}: {e}", path.display()));

    println!("cargo::rustc-env=CAMERA_ADDRESS={address}");
}
