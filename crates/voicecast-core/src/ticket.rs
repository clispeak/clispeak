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
}

impl Ticket {
    /// Mint a ticket for `endpoint_id`, valid for [`TTL_SECS`].
    pub fn mint(endpoint_id: String) -> Self {
        Self {
            endpoint_id,
            token: random_token(),
            expires_at: now() + TTL_SECS,
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
    pub fn parse(s: &str) -> Result<Self> {
        let body = s
            .trim()
            .strip_prefix("voicecast://join/")
            .unwrap_or(s.trim());
        let bytes = BASE32_NOPAD
            .decode(body.to_uppercase().as_bytes())
            .context("that does not look like a voicecast invite")?;
        let ticket: Self = ciborium::from_reader(&bytes[..]).context("decoding ticket")?;
        if !ticket.is_valid() {
            bail!("this invite expired; ask for a new one");
        }
        Ok(ticket)
    }
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
