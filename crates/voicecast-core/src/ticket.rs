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
}

impl Ticket {
    /// Mint a ticket for `endpoint_id`, valid for [`TTL_SECS`].
    pub fn mint(endpoint_id: String, space: Option<String>) -> Self {
        Self {
            endpoint_id,
            token: random_token(),
            expires_at: now() + TTL_SECS,
            space,
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
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(path, text);
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

    /// Render as a `voicecast://join/...` link.
    pub fn to_url(&self) -> Result<String> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).context("encoding ticket")?;
        Ok(format!("voicecast://join/{}", BASE32_NOPAD.encode(&buf)))
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
        let body = trimmed.strip_prefix("voicecast://join/").unwrap_or(trimmed);
        let bytes = BASE32_NOPAD
            .decode(body.to_uppercase().as_bytes())
            .map_err(|_| anyhow::anyhow!("that does not look like a voicecast invite"))?;
        let ticket: Self = ciborium::from_reader(&bytes[..])
            .map_err(|_| anyhow::anyhow!("that invite looks truncated, copy the whole code"))?;
        if !ticket.is_valid() {
            bail!("that invite has expired, ask for a new one");
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
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
