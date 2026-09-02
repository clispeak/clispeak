//! Space membership.
//!
//! A space is a signed roster of member devices. Authorisation is "is this
//! endpoint id in my roster?" — there is **no shared group secret**, so
//! compromising one device leaks nothing that decrypts another's traffic.
//!
//! Each entry is signed by the member that invited it. That is what lets a
//! device vouch for itself on arrival: a node can admit a peer it has never
//! seen, provided the peer presents a join record signed by someone the node
//! already trusts. See `docs/architecture.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use iroh_base::{EndpointId, SecretKey, Signature};
use serde::{Deserialize, Serialize};
use voicecast_proto::Member;

/// Why a roster operation failed.
#[derive(Debug, thiserror::Error)]
pub enum RosterError {
    /// The roster file could not be read or written.
    #[error("roster storage: {0}")]
    Storage(String),
    /// A join record's signature did not check out.
    #[error("join record for {0} is not correctly signed")]
    BadSignature(String),
    /// The inviter is not someone we trust.
    #[error("join record for {0} was signed by a non-member")]
    UnknownInviter(String),
}

/// Check that a join record really was signed by its stated inviter.
///
/// Free function rather than a method because `Member` is a wire type in
/// `voicecast-proto`, which stays free of crypto dependencies.
pub fn verify(member: &Member) -> Result<(), RosterError> {
    let inviter: EndpointId = member
        .invited_by
        .parse()
        .map_err(|_| RosterError::BadSignature(member.endpoint_id.clone()))?;
    let bytes: [u8; 64] = member
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| RosterError::BadSignature(member.endpoint_id.clone()))?;
    let sig = Signature::from_bytes(&bytes);
    let payload = Member::signed_payload(&member.endpoint_id, &member.invited_by, member.joined_at);
    inviter
        .verify(&payload, &sig)
        .map_err(|_| RosterError::BadSignature(member.endpoint_id.clone()))
}

/// The members of one space, as this device knows them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Roster {
    /// Keyed by endpoint id so merging two rosters is a map union.
    members: BTreeMap<String, Member>,
    /// Endpoint ids that have been revoked. Tombstones win over entries.
    revoked: BTreeMap<String, u64>,
}

impl Roster {
    /// An empty roster.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record this device as the founder of a new space.
    ///
    /// The founder signs its own entry: the trust chain has to start
    /// somewhere, and every other member traces back to it.
    pub fn found(secret: &SecretKey, name: &str) -> Self {
        let mut roster = Self::new();
        let me = secret.public().to_string();
        roster.insert_self_signed(secret, &me, name);
        roster
    }

    /// Sign and insert an entry for `endpoint_id`, vouched for by us.
    pub fn invite(&mut self, secret: &SecretKey, endpoint_id: &str, name: &str) -> Member {
        self.insert_self_signed(secret, endpoint_id, name)
    }

    fn insert_self_signed(&mut self, secret: &SecretKey, endpoint_id: &str, name: &str) -> Member {
        let inviter = secret.public().to_string();
        let joined_at = now();
        let payload = Member::signed_payload(endpoint_id, &inviter, joined_at);
        let member = Member {
            endpoint_id: endpoint_id.to_string(),
            name: name.to_string(),
            invited_by: inviter,
            signature: secret.sign(&payload).to_bytes().to_vec(),
            joined_at,
            renamed_at: joined_at,
        };
        self.members.insert(endpoint_id.to_string(), member.clone());
        member
    }

    /// Build a roster by adopting a space's membership wholesale.
    ///
    /// Signatures are checked, but membership of the inviter is not: the
    /// founder's entry is self-signed, so admitting it into an empty roster
    /// would be impossible. Trust for the whole set comes from the invite
    /// ticket that led here, not from each record individually.
    pub fn adopt(members: impl IntoIterator<Item = Member>) -> Self {
        let mut roster = Self::new();
        for member in members {
            if verify(&member).is_ok() {
                roster.members.insert(member.endpoint_id.clone(), member);
            }
        }
        roster
    }

    /// Leave the space, keeping only this device.
    ///
    /// The identity is deliberately kept: leaving is not the same as becoming
    /// a different device, and rejoining later should be recognisable as the
    /// same machine.
    pub fn leave(secret: &SecretKey, name: &str) -> Self {
        Self::found(secret, name)
    }

    /// Rebuild a roster from a peer's snapshot, verifying every signature.
    pub fn from_parts(members: Vec<Member>, revoked: Vec<(String, u64)>) -> Self {
        let mut roster = Self::adopt(members);
        for (id, at) in revoked {
            roster.revoked.insert(id, at);
        }
        roster
    }

    /// Revocations, for sending to a peer.
    pub fn tombstones(&self) -> Vec<(String, u64)> {
        self.revoked.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    /// Accept a member vouched for by someone already in the roster.
    pub fn admit(&mut self, member: Member) -> Result<(), RosterError> {
        verify(&member)?;
        if !self.members.contains_key(&member.invited_by) {
            return Err(RosterError::UnknownInviter(member.endpoint_id.clone()));
        }
        if self
            .revoked
            .get(&member.endpoint_id)
            .is_some_and(|t| *t > member.joined_at)
        {
            // A rejoin only counts if it is newer than the revocation that
            // removed it — otherwise a revoked device could replay its old
            // record. It cannot forge a newer one without a real member.
            return Ok(());
        }
        self.members.insert(member.endpoint_id.clone(), member);
        Ok(())
    }

    /// Whether this endpoint may speak on this device.
    pub fn allows(&self, endpoint_id: &EndpointId) -> bool {
        let key = endpoint_id.to_string();
        !self.revoked.contains_key(&key) && self.members.contains_key(&key)
    }

    /// Rename a member, leaving its signature valid.
    ///
    /// Names sit outside the signed payload precisely so this is possible —
    /// see [`Member::signed_payload`]. Returns whether anything changed.
    pub fn rename(&mut self, endpoint_id: &str, name: &str) -> bool {
        match self.members.get_mut(endpoint_id) {
            Some(m) if m.name != name => {
                m.name = name.to_string();
                // Stamped so a merge can tell this apart from a stale copy.
                m.renamed_at = now();
                true
            }
            _ => false,
        }
    }

    /// Give this device's own entry a rename stamp if it has none.
    ///
    /// Migration for rosters written before `renamed_at` existed: without a
    /// stamp its label can never win a merge, so a rename made before the
    /// upgrade would never reach anyone. Only a device's own entry is
    /// stamped — it is the authority on its own name.
    pub fn stamp_own_label(&mut self, endpoint_id: &str) -> bool {
        match self.members.get_mut(endpoint_id) {
            Some(m) if m.renamed_at == 0 => {
                m.renamed_at = now();
                true
            }
            _ => false,
        }
    }

    /// Look up a member by its local label.
    pub fn by_name(&self, name: &str) -> Option<&Member> {
        self.members.values().find(|m| m.name == name)
    }

    /// Every current member.
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members
            .values()
            .filter(|m| !self.revoked.contains_key(&m.endpoint_id))
    }

    /// Remove a device from the space.
    pub fn revoke(&mut self, endpoint_id: &str) {
        self.revoked.insert(endpoint_id.to_string(), now());
        self.members.remove(endpoint_id);
    }

    /// Merge another roster into this one.
    ///
    /// Add-only with tombstones, so the result does not depend on the order
    /// updates arrive in — two devices that sync in either direction converge
    /// on the same roster.
    pub fn merge(&mut self, other: &Roster) {
        for (id, at) in &other.revoked {
            let entry = self.revoked.entry(id.clone()).or_insert(*at);
            *entry = (*entry).max(*at);
        }
        for (id, member) in &other.members {
            if verify(member).is_err() {
                continue;
            }
            match self.members.get(id) {
                Some(existing) if existing.joined_at >= member.joined_at => {
                    // Same membership, but the label may have moved on. A
                    // device is authoritative about its own name, and the
                    // newer stamp is how that reaches everyone else.
                    if member.renamed_at > existing.renamed_at {
                        self.members.insert(id.clone(), member.clone());
                    }
                }
                _ => {
                    self.members.insert(id.clone(), member.clone());
                }
            }
        }
        self.members
            .retain(|id, m| !self.revoked.get(id).is_some_and(|t| *t > m.joined_at));
    }

    /// Load from disk, returning an empty roster if there is none yet.
    pub fn load(path: &Path) -> Result<Self, RosterError> {
        match std::fs::read(path) {
            Ok(bytes) => {
                ciborium::from_reader(&bytes[..]).map_err(|e| RosterError::Storage(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(RosterError::Storage(e.to_string())),
        }
    }

    /// Write to disk.
    pub fn save(&self, path: &Path) -> Result<(), RosterError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| RosterError::Storage(e.to_string()))?;
        }
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| RosterError::Storage(e.to_string()))?;
        std::fs::write(path, buf).map_err(|e| RosterError::Storage(e.to_string()))
    }

    /// Where the roster lives by default.
    pub fn default_path() -> Result<PathBuf, RosterError> {
        crate::identity::config_dir()
            .map(|d| d.join("roster.cbor"))
            .map_err(|e| RosterError::Storage(e.to_string()))
    }
}

/// Unix seconds now, or zero if the clock is before the epoch.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
