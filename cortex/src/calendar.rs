//! When the rig plays, and when it is dark while it does.
//!
//! Every instant here was resolved at build time from the rig's show table, so all the running
//! brain does is compare them against its own clock. Nothing waits for that clock to be right.
//! The Pi has no RTC, and after a reboot it restores the time it last saved, which is behind —
//! by the reboot itself on a clean restart, by the outage on a power cut — and the 4G link can
//! take a minute to let NTP fix it. Behind is the safe direction: the table printed at startup
//! keeps a show that has already ended rather than dropping one to come, and since the decision
//! is taken afresh every frame, the rig is right from the frame after NTP steps the clock.

use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use std::time::{SystemTime, UNIX_EPOCH};

/// How often the clock's synchronisation is checked. A change only reprints the table, so
/// this needs to be prompt for a person reading the log and nothing more.
const SYNC_POLL: Duration = Duration::from_secs(1);

pub struct Calendar {
    pub site: Site,
    pub shows: &'static [Show],
}

/// Where the rig stands, as the build resolved it from the city the table names.
pub struct Site {
    pub city: &'static str,
    pub latitude: f64,
    pub longitude: f64,
    pub zone: &'static str,
}

pub struct Show {
    pub span: Span,
    /// The stretches of the show between civil dusk and civil dawn.
    pub dark: &'static [Span],
}

/// From `start`, up to but not including `stop`.
#[derive(Clone, Copy)]
pub struct Span {
    pub start: Stamp,
    pub stop: Stamp,
}

#[derive(Clone, Copy)]
pub struct Stamp {
    pub unix: i64,
    /// The same instant as local time in the site's zone, with its offset and season.
    pub local: &'static str,
    /// The local time of day alone, for an instant whose date and offset its show already
    /// gives.
    pub time: &'static str,
}

impl Calendar {
    /// The show playing at `now`, in Unix seconds.
    pub fn current(&self, now: f64) -> Option<&Show> {
        self.shows.iter().find(|show| show.span.contains(now))
    }

    /// Prints the site and every show that has not yet ended by the clock as it stands.
    fn announce(&self, synchronised: bool) {
        let site = &self.site;
        eprintln!(
            "show: {} ({:.4}°, {:.4}°), {}",
            site.city, site.latitude, site.longitude, site.zone
        );
        if synchronised {
            eprintln!("show: the clock is NTP-synchronised");
        } else {
            eprintln!(
                "show: the clock is not NTP-synchronised, and may be behind until it is"
            );
        }
        let now = now();
        let remaining: Vec<&Show> = self
            .shows
            .iter()
            .filter(|show| now < show.span.stop.unix as f64)
            .collect();
        eprintln!("show: {} of {} shows to come", remaining.len(), self.shows.len());
        for show in remaining {
            eprintln!(
                "show:   {} → {}",
                show.span.start.local, show.span.stop.local
            );
            for dark in show.dark {
                eprintln!("show:     dark {} → {}", dark.start.time, dark.stop.time);
            }
        }
    }
}

impl Span {
    pub fn contains(&self, now: f64) -> bool {
        self.start.unix as f64 <= now && now < self.stop.unix as f64
    }
}

impl Show {
    /// The stretch of the show between civil dusk and civil dawn that `now` is in, if any.
    pub fn dark_stretch(&self, now: f64) -> Option<Span> {
        self.dark.iter().copied().find(|span| span.contains(now))
    }
}

/// The wall clock, in Unix seconds. Fractional, because a show times things in fractions of a
/// second, and a whole-second clock would put a one-second event anywhere from none to two
/// frames' worth.
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is before 1970")
        .as_secs_f64()
}

/// Prints the calendar, and prints it again whenever the clock becomes NTP-synchronised, so
/// the log says which clock every decision before and after was taken against.
pub fn spawn(spawner: Spawner, calendar: &'static Calendar) {
    spawner.spawn(sync_task(calendar).unwrap());
}

#[embassy_executor::task]
async fn sync_task(calendar: &'static Calendar) -> ! {
    let mut synchronised = clock_synchronised();
    calendar.announce(synchronised);
    let mut ticker = Ticker::every(SYNC_POLL);
    loop {
        ticker.next().await;
        let current = clock_synchronised();
        if current != synchronised {
            synchronised = current;
            calendar.announce(synchronised);
        }
    }
}

/// Whether the kernel holds the clock as synchronised: what timesyncd sets once it has
/// reached a server, and lets lapse if it loses it for long enough.
fn clock_synchronised() -> bool {
    // SAFETY: `timex` is plain data, so all zeroes is a valid value, and with `modes` zero
    // `ntp_adjtime` only reads the kernel's state into it and changes nothing.
    let mut timex: libc::timex = unsafe { std::mem::zeroed() };
    unsafe { libc::ntp_adjtime(&mut timex) != libc::TIME_ERROR }
}
