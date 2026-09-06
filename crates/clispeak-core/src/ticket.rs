//! Invite tickets.
//!
//! A ticket carries the inviter's public key and a one-time token. It carries
//! no address: pkarr resolves the key to wherever the device currently is,
//! which is why a ticket stays valid as devices move between networks.
//!
//! The token is not optional. Without it, a ticket photographed over your
//! shoulder — or left in terminal scrollback, or in a screen recording — is
//! permanent access to your speakers.

use anyhow::{Context, Result, bail};
use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};

/// How long an invite stays valid.
pub const TTL_SECS: u64 = 300;

/// The payload encoded into a ticket string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    /// The inviter's public key.
    pub endpoint_id: String,
    /// Single-use secret proving the joiner was actually invited.
    pub token: String,
    /// Unix seconds when this ticket stops being accepted.
    pub expires_at: u64,
    /// Which space the joiner is being invited into.
    ///
    /// Defaulted, and absent means the inviter's default space — both because
    /// tickets minted before this existed carry nothing, and because that is
    /// the honest reading of an invite that never named one.
    ///
    /// The ticket has to carry it. Without it the inviting device decides at
    /// the moment the joiner arrives, which is a different question from the
    /// one the person pressing the button was answering.
    #[serde(default)]
    pub space: Option<String>,
    /// What the inviter calls that space, for the joiner to read.
    ///
    /// `space` above is an id — `<founder>:<joined_at>` — which is exactly
    /// right for deciding *which* roster and exactly wrong for showing to a
    /// person. The label rides along so a joining device can say "this joins
    /// work" before it has spoken to anyone, which is the only moment the
    /// answer is still useful: after the round trip the join has happened.
    ///
    /// Advisory, and not to be trusted for anything but display — it is
    /// whatever the inviting device wrote. The id is what selects the roster.
    #[serde(default)]
    pub label: Option<String>,
}

impl Ticket {
    /// Mint a ticket for `endpoint_id`, valid for [`TTL_SECS`].
    pub fn mint(endpoint_id: String, space: Option<String>, label: Option<String>) -> Self {
        Self {
            endpoint_id,
            token: random_token(),
            expires_at: now() + TTL_SECS,
            space,
            label,
        }
    }

    /// Where an outstanding invite is kept between restarts.
    fn path() -> Option<std::path::PathBuf> {
        crate::identity::config_dir()
            .ok()
            .map(|d| d.join("invite.json"))
    }

    /// Remember this invite, so it survives the app being restarted.
    ///
    /// An invite lives in memory for the five minutes it is valid, and on a
    /// phone the system can kill the app in that window — between showing a
    /// QR code and the other device scanning it. Losing it there refuses a
    /// join for a reason the person cannot see or act on.
    ///
    /// Persisting widens nothing meaningfully: the ticket is already on
    /// screen as a QR code, it still expires, and it is still single use. The
    /// file lives in app-private storage.
    pub fn remember(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = crate::store::create_dir_private(dir);
        }
        if let Ok(text) = serde_json::to_string(self) {
            // Holds the live token: private, and atomic so a
            // crash cannot leave half a ticket to be re-read.
            let _ = crate::store::write_private(&path, text.as_bytes());
        }
    }

    /// Drop the remembered invite, once used or abandoned.
    pub fn forget() {
        if let Some(path) = Self::path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The remembered invite, if there is one and it has not expired.
    pub fn recall() -> Option<Self> {
        let path = Self::path()?;
        let ticket: Self = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
        if ticket.is_valid() {
            Some(ticket)
        } else {
            // Tidy up rather than leave a dead ticket to be re-read on every
            // start.
            let _ = std::fs::remove_file(&path);
            None
        }
    }

    /// Whether this ticket is still within its lifetime.
    pub fn is_valid(&self) -> bool {
        now() <= self.expires_at
    }

    /// Seconds until expiry, saturating at zero.
    pub fn remaining(&self) -> u64 {
        self.expires_at.saturating_sub(now())
    }

    /// What to say about a ticket this device thinks has expired.
    ///
    /// **"Expired" is not a property of the ticket.** It is a statement about
    /// two clocks, and this one is only ever reporting its own. A freshly
    /// minted invite read on a machine whose clock is ahead looks exactly like
    /// one from last week, and the old message — "that invite has expired, ask
    /// for a new one" — sent the reader to mint another that would fail
    /// identically, for as long as they were willing to keep trying. Patrick
    /// met that on a fresh Windows VM, whose clock was wrong, on 6 September
    /// 2026 (#200).
    ///
    /// So it hands over the evidence instead of a verdict: how long ago, and
    /// what this device believes the time is. Somebody whose clock is hours
    /// out reads the second half and diagnoses it immediately.
    fn expiry_objection(&self, now: u64) -> String {
        let ago = now.saturating_sub(self.expires_at);
        format!(
            "that invite expired {} ago, according to this device — which \
             believes it is now {}. If that is not close to the real time, this \
             device's clock is wrong and the invite is fine; fix the clock here \
             rather than asking for another invite.",
            plain_duration(ago),
            utc(now),
        )
    }

    /// A message when this device's clock is provably behind the minting one.
    ///
    /// **This direction can be proved, and the other cannot.** A ticket is
    /// minted with exactly [`TTL_SECS`] of life, so no honest ticket can ever
    /// have more than that remaining. More than that means the clock reading it
    /// is behind the clock that wrote it, by at least the difference — a fact
    /// rather than a suspicion, and worth saying as one.
    ///
    /// The reverse is unprovable from the ticket alone: a clock running ahead
    /// and a genuinely old invite produce identical bytes. That asymmetry is
    /// kept rather than papered over, because guessing on the unprovable half
    /// would be the same mistake in a new coat.
    fn clock_is_behind(&self, now: u64) -> Option<String> {
        let remaining = self.expires_at.saturating_sub(now);
        let excess = remaining.checked_sub(TTL_SECS)?;
        if excess == 0 {
            return None;
        }
        Some(format!(
            "this device's clock is at least {} behind the device that made \
             this invite, so the invite cannot be checked. An invite is only \
             ever valid for {}, and this one claims {}. Fix the clock here; the \
             invite is fine.",
            plain_duration(excess),
            plain_duration(TTL_SECS),
            plain_duration(remaining),
        ))
    }

    /// Render as a `clispeak://join/...` link.
    pub fn to_url(&self) -> Result<String> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).context("encoding ticket")?;
        Ok(format!("clispeak://join/{}", BASE32_NOPAD.encode(&buf)))
    }

    /// Parse a ticket, with or without the URL prefix.
    ///
    /// Errors are written for whoever pasted the thing, not for a log file: a
    /// person who mis-copies an invite should be told that, rather than shown
    /// a CBOR decoder's opinion of a truncated buffer.
    pub fn parse(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            bail!("paste an invite first, then try again");
        }
        let body = trimmed.strip_prefix("clispeak://join/").unwrap_or(trimmed);
        let bytes = BASE32_NOPAD
            .decode(body.to_uppercase().as_bytes())
            .map_err(|_| anyhow::anyhow!("that does not look like a clispeak invite"))?;
        let ticket: Self = ciborium::from_reader(&bytes[..])
            .map_err(|_| anyhow::anyhow!("that invite looks truncated, copy the whole code"))?;
        if !ticket.is_valid() {
            bail!("{}", ticket.expiry_objection(now()));
        }
        // A ticket with more life left than one can be minted with did not come
        // from a device that agrees with this one about the time.
        if let Some(message) = ticket.clock_is_behind(now()) {
            bail!("{message}");
        }
        Ok(ticket)
    }
}

/// Render an invite as an SVG QR code.
///
/// Generated here rather than in the interface: a QR encoder is Reed-Solomon
/// and bit-masking, and a subtly wrong one produces a code that *looks* right
/// and will not scan. Doing it once in Rust also lets the CLI print the same
/// invite as terminal blocks.
///
/// The SVG carries no fill colours, so it inherits the page's — which keeps
/// it legible in both light and dark without two code paths.
pub fn qr_svg(text: &str) -> Result<String> {
    use qrcode::{EcLevel, QrCode, render::svg};
    // Low correction: the payload is already long, and a screen is a clean
    // scanning surface — no need to spend capacity on damage tolerance.
    let code = QrCode::with_error_correction_level(text, EcLevel::L)
        .context("that invite is too long to encode as a QR code")?;
    Ok(code
        .render()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// A token with enough entropy that guessing it is not worth trying.
fn random_token() -> String {
    use rand::RngExt;
    let bytes: [u8; 16] = rand::rng().random();
    BASE32_NOPAD.encode(&bytes)
}

/// Unix seconds now, or zero if the clock is before the epoch.
/// A duration a person would say out loud, from seconds.
///
/// Deliberately coarse. The reader is deciding whether a number is plausible,
/// not measuring anything: "3 hours" answers that and "10,847 seconds" does
/// not.
fn plain_duration(secs: u64) -> String {
    // Rounded, not ceilinged. `div_ceil` turns 172,801 seconds into "3 days",
    // which is wrong by most of a day in the direction that makes a correct
    // clock look broken.
    let round = |u: u64| (secs + u / 2) / u;
    let (n, unit) = match secs {
        0..=90 => (secs, "second"),
        91..3_600 => (round(60), "minute"),
        3_600..172_800 => (round(3_600), "hour"),
        _ => (round(86_400), "day"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// A unix time as something a person can compare against a clock.
///
/// UTC and named as such, because the point is to be checked against reality
/// and a local rendering of a wrong clock is just the wrong time again in
/// friendlier words.
fn utc(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| format!("unix time {secs}"))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What a ticket says when the clocks disagree.
///
/// Pure functions over a timestamp rather than over `now()`, because the case
/// worth testing is precisely the one where this machine's clock is not the
/// one to trust.
#[cfg(test)]
mod clock_tests {
    use super::*;

    fn at(expires_at: u64) -> Ticket {
        Ticket {
            endpoint_id: "e".into(),
            token: "t".into(),
            expires_at,
            space: None,
            label: None,
        }
    }

    /// A ticket read on a clock that agrees is simply expired — and the message
    /// hands over what this device believes rather than a verdict.
    #[test]
    fn an_expired_invite_says_what_this_device_thinks_the_time_is() {
        let msg = at(1_000_000).expiry_objection(1_000_000 + 7200);
        assert!(msg.contains("2 hours ago"), "{msg}");
        assert!(msg.contains("believes it is now"), "{msg}");
        assert!(msg.contains("UTC"), "{msg}");
        // The old advice sent people to mint another that would fail
        // identically, for as long as they were willing to keep trying.
        assert!(!msg.contains("ask for a new one"), "{msg}");
    }

    /// **The provable direction.** No honest ticket can carry more than
    /// `TTL_SECS` of life, so more than that is a reader's clock behind the
    /// minter's — a fact, and said as one.
    #[test]
    fn more_life_than_can_be_minted_proves_the_reader_is_behind() {
        let msg = at(1_000_000 + TTL_SECS + 3600)
            .clock_is_behind(1_000_000)
            .expect("provably behind");
        assert!(msg.contains("1 hour"), "{msg}");
        assert!(msg.contains("clock"), "{msg}");
    }

    /// A ticket minted a moment ago carries exactly the full lifetime, and that
    /// is not evidence of anything. The boundary has to be inclusive or every
    /// fresh invite is refused as suspicious.
    #[test]
    fn a_ticket_with_exactly_its_full_life_is_not_an_accusation() {
        assert_eq!(at(1_000_000 + TTL_SECS).clock_is_behind(1_000_000), None);
    }

    /// And an ordinary unexpired ticket provokes nothing.
    #[test]
    fn a_healthy_invite_provokes_no_message() {
        assert_eq!(at(1_000_000 + 120).clock_is_behind(1_000_000), None);
    }

    /// The reader is judging plausibility, not measuring. Coarse units, and
    /// singular where it matters, because "1 hours" reads as a bug in the tool
    /// rather than a fact about the clock.
    #[test]
    fn durations_are_said_the_way_a_person_would() {
        assert_eq!(plain_duration(1), "1 second");
        assert_eq!(plain_duration(45), "45 seconds");
        assert_eq!(plain_duration(300), "5 minutes");
        assert_eq!(plain_duration(3600), "1 hour");
        assert_eq!(plain_duration(7200), "2 hours");
        assert_eq!(plain_duration(172_801), "2 days");
    }

    /// A timestamp a person can hold against a wall clock.
    #[test]
    fn a_time_is_rendered_as_utc() {
        assert_eq!(utc(1_000_000_000), "2001-09-09 01:46 UTC");
    }
}
