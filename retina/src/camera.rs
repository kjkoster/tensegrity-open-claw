//! The camera over VAPIX, Axis's HTTP interface: its settings set and read back, and its MJPEG
//! stream opened.
//!
//! What can go wrong splits in two. The camera being unplugged, booting or out of reach comes
//! back by itself, so it is returned as a reason and retried. A password the camera refuses, a
//! setting it will not take, or a page it does not have will not, so they panic, with the
//! camera's whole answer.

use crate::config as cfg;
use crate::view::View;
use digest_auth::AuthContext;
use std::io::{BufRead, BufReader};
use ureq::Agent;
use ureq::http::Response;

/// Why the camera could not be reached this time.
pub type Unreachable = String;

pub struct Camera {
    agent: Agent,
    view: View,
}

impl Camera {
    /// The camera, to be set up for `view`.
    pub fn new(view: View) -> Camera {
        let agent = Agent::config_builder()
            .timeout_connect(Some(cfg::CONNECT_TIMEOUT))
            .timeout_per_read(Some(cfg::READ_TIMEOUT))
            // A 401 is the Digest challenge to answer, and a 5xx a camera still booting:
            // both are statuses to look at, not errors.
            .http_status_as_error(false)
            .build()
            .new_agent();
        Camera { agent, view }
    }

    /// Sets the camera's own settings and those of the view asked for, its light and which way
    /// up it hangs, and reads them all back, so the camera is as the retina wants it whatever
    /// was done to it since.
    pub fn configure(&self) -> Result<(), Unreachable> {
        let rotation = (
            "Image.I0.Appearance.Rotation",
            self.view.orientation.rotation(),
        );
        let settings: Vec<(&str, &str)> = cfg::SETTINGS
            .iter()
            .chain(cfg::light(self.view.light))
            .copied()
            .chain([rotation])
            .collect();
        let update: Vec<String> = settings
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        let path = format!("/axis-cgi/param.cgi?action=update&{}", update.join("&"));
        let answer = self.text(&path)?;
        if answer.trim() != "OK" {
            panic!(
                "retina: the camera refused a setting: GET http://{}{path} answered {:?}",
                cfg::HOST,
                answer.trim()
            );
        }

        // Listed by the groups the settings live in, `ImageSource.I0` and `Image.I0`, which the
        // camera answers whole; asked for by their own names, all at once, it does not.
        let mut groups: Vec<String> = settings
            .iter()
            .map(|(name, _)| name.splitn(3, '.').take(2).collect::<Vec<_>>().join("."))
            .collect();
        groups.sort();
        groups.dedup();
        let mut listing = String::new();
        for group in &groups {
            listing += &self.text(&format!("/axis-cgi/param.cgi?action=list&group={group}"))?;
        }
        for (name, value) in settings {
            let read = listing.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("root.")?
                    .strip_prefix(name)?
                    .strip_prefix('=')
            });
            if read != Some(value) {
                panic!(
                    "retina: {name} reads back as {read:?} after setting it to {value}: \
                     GET http://{}/axis-cgi/param.cgi?action=list for the groups {} answered {:?}",
                    cfg::HOST,
                    groups.join(", "),
                    listing.trim()
                );
            }
        }
        Ok(())
    }

    /// The MJPEG stream, at the resolution and rate asked for.
    pub fn stream(&self) -> Result<impl BufRead, Unreachable> {
        let (mut width, mut height) = self.view.resolution.size();
        if self.view.orientation.sideways() {
            (width, height) = (height, width);
        }
        let path = format!(
            "/axis-cgi/mjpg/video.cgi?resolution={width}x{height}&fps={}",
            self.view.rate.get()
        );
        let mut response = self.get(&path)?;
        // A camera that answers with a page or a message instead of the stream would otherwise
        // be read as frames that never come, and show only as a stream ending early.
        let multipart = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("multipart/x-mixed-replace"));
        if !multipart {
            panic!(
                "retina: not an MJPEG stream: {}",
                exchange(&path, &mut response)
            );
        }
        Ok(BufReader::new(response.into_body().into_reader()))
    }

    fn text(&self, path: &str) -> Result<String, Unreachable> {
        self.get(path)?
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("{path}: {e}"))
    }

    /// GETs `path`, answering the camera's Digest challenge.
    fn get(&self, path: &str) -> Result<Response<ureq::Body>, Unreachable> {
        let url = format!("http://{}{path}", cfg::HOST);
        let unreachable = |e: ureq::Error| format!("GET {url}: {e}");

        let mut response = self.agent.get(&url).call().map_err(unreachable)?;
        if response.status() == 401 {
            let challenge = response
                .headers()
                .get_all("www-authenticate")
                .iter()
                .filter_map(|value| value.to_str().ok())
                .find(|value| value.starts_with("Digest"))
                .map(str::to_string)
                .unwrap_or_else(|| {
                    panic!(
                        "retina: the camera offers no Digest login: {}",
                        exchange(path, &mut response)
                    )
                });
            let challenge = challenge.as_str();
            let answer = digest_auth::parse(challenge)
                .and_then(|mut prompt| {
                    prompt.respond(&AuthContext::new(cfg::USER, cfg::PASSWORD, path))
                })
                .unwrap_or_else(|e| {
                    panic!("retina: GET {url}: the camera's Digest challenge {challenge:?}: {e}")
                })
                .to_string();
            response = self
                .agent
                .get(&url)
                .header("Authorization", answer)
                .call()
                .map_err(unreachable)?;
        }

        let status = response.status().as_u16();
        if status == 200 {
            return Ok(response);
        }
        let exchange = exchange(path, &mut response);
        match status {
            401 => panic!(
                "retina: the camera refuses {} with the retina's password: {exchange}",
                cfg::USER
            ),
            500.. => Err(exchange),
            _ => panic!("retina: {exchange}"),
        }
    }
}

/// The exchange as it went: the request, and the camera's whole answer — its status, its
/// headers and its body — so a failure shows what the camera said, not only what was made of
/// it. Reads the body, so the response is spent.
fn exchange(path: &str, response: &mut Response<ureq::Body>) -> String {
    let status = response.status();
    let headers: Vec<String> = response
        .headers()
        .iter()
        .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap_or("(not text)")))
        .collect();
    let body = response
        .body_mut()
        .read_to_string()
        .unwrap_or_else(|e| format!("(unreadable: {e})"));
    format!(
        "GET http://{}{path} answered {status} [{}] {:?}",
        cfg::HOST,
        headers.join(" | "),
        body.trim()
    )
}
