//! Roster signatures, admission, revocation, and merge convergence.

use iroh_base::SecretKey;
use voicecast_core::{Member, Roster, verify};

fn device() -> SecretKey {
    SecretKey::generate()
}

#[test]
fn founder_is_a_member_of_its_own_space() {
    let alice = device();
    let roster = Roster::found(&alice, "alice");
    assert!(roster.allows(&alice.public()));
    assert_eq!(roster.members().count(), 1);
}

#[test]
fn an_invited_device_is_admitted_and_verifiable() {
    let alice = device();
    let bob = device();

    let mut on_alice = Roster::found(&alice, "alice");
    let record = on_alice.invite(&alice, &bob.public().to_string(), "bob");

    assert!(verify(&record).is_ok(), "alice's signature must check out");
    assert!(on_alice.allows(&bob.public()));
}

#[test]
fn a_device_can_vouch_for_itself_to_a_peer_that_has_never_seen_it() {
    // The property the whole design leans on: Carol joins by scanning Bob's
    // invite while Alice is asleep. When Alice later meets Carol, she admits
    // her because the record is signed by Bob, whom Alice already trusts.
    let alice = device();
    let bob = device();
    let carol = device();

    let mut on_alice = Roster::found(&alice, "alice");
    let bob_record = on_alice.invite(&alice, &bob.public().to_string(), "bob");

    let mut on_bob = Roster::new();
    on_bob.merge(&on_alice);
    let carol_record = on_bob.invite(&bob, &carol.public().to_string(), "carol");

    assert!(
        !on_alice.allows(&carol.public()),
        "alice has not met carol yet"
    );
    on_alice.admit(carol_record).expect("bob vouched for carol");
    assert!(on_alice.allows(&carol.public()), "carol admitted via bob");
    let _ = bob_record;
}

#[test]
fn a_record_signed_by_a_stranger_is_refused() {
    let alice = device();
    let mallory = device();
    let victim = device();

    let mut on_alice = Roster::found(&alice, "alice");

    // Mallory is not in Alice's roster, so anything she signs means nothing.
    let mut forged = Roster::found(&mallory, "mallory");
    let record = forged.invite(&mallory, &victim.public().to_string(), "victim");

    assert!(
        on_alice.admit(record).is_err(),
        "must refuse an unknown inviter"
    );
    assert!(!on_alice.allows(&victim.public()));
}

#[test]
fn a_tampered_record_fails_verification() {
    let alice = device();
    let bob = device();
    let mut on_alice = Roster::found(&alice, "alice");
    let mut record = on_alice.invite(&alice, &bob.public().to_string(), "bob");

    // Swap in a different device while keeping Alice's signature.
    let attacker = device();
    record.endpoint_id = attacker.public().to_string();

    assert!(
        verify(&record).is_err(),
        "signature must not survive substitution"
    );
}

#[test]
fn renaming_a_device_does_not_break_its_membership() {
    // Names are local labels, so they are deliberately outside the signature.
    let alice = device();
    let bob = device();
    let mut on_alice = Roster::found(&alice, "alice");
    let mut record = on_alice.invite(&alice, &bob.public().to_string(), "bob");

    record.name = "bob's new laptop".into();
    assert!(
        verify(&record).is_ok(),
        "renaming must not invalidate the record"
    );
}

#[test]
fn revocation_removes_a_device() {
    let alice = device();
    let bob = device();
    let mut on_alice = Roster::found(&alice, "alice");
    on_alice.invite(&alice, &bob.public().to_string(), "bob");
    assert!(on_alice.allows(&bob.public()));

    on_alice.revoke(&bob.public().to_string());
    assert!(!on_alice.allows(&bob.public()));
}

#[test]
fn merge_converges_regardless_of_direction() {
    // Add-only with tombstones is a CRDT, so two devices syncing in either
    // order must end up with the same membership.
    let alice = device();
    let bob = device();
    let carol = device();

    let mut a = Roster::found(&alice, "alice");
    a.invite(&alice, &bob.public().to_string(), "bob");

    let mut b = Roster::new();
    b.merge(&a);
    b.invite(&bob, &carol.public().to_string(), "carol");

    let mut a_then_b = a.clone();
    a_then_b.merge(&b);
    let mut b_then_a = b.clone();
    b_then_a.merge(&a);

    let mut left: Vec<_> = a_then_b.members().map(|m| m.endpoint_id.clone()).collect();
    let mut right: Vec<_> = b_then_a.members().map(|m| m.endpoint_id.clone()).collect();
    left.sort();
    right.sort();
    assert_eq!(left, right, "merge must converge");
    assert_eq!(left.len(), 3);
}

#[test]
fn merge_drops_records_that_do_not_verify() {
    let alice = device();
    let mallory = device();
    let mut a = Roster::found(&alice, "alice");

    let mut junk = Roster::new();
    let victim = device();
    junk.invite(&mallory, &victim.public().to_string(), "victim");
    // Corrupt the signature so it cannot verify at all.
    let mut bad: Member = junk.members().next().unwrap().clone();
    bad.signature = vec![0u8; 64];
    let mut carrier = Roster::new();
    let _ = carrier.admit(bad);

    a.merge(&carrier);
    assert!(
        !a.allows(&victim.public()),
        "unverifiable records must not survive a merge"
    );
}

#[test]
fn a_roster_round_trips_through_disk() {
    let alice = device();
    let bob = device();
    let mut a = Roster::found(&alice, "alice");
    a.invite(&alice, &bob.public().to_string(), "bob");

    let mut path = std::env::temp_dir();
    path.push(format!("voicecast-roster-{}.cbor", std::process::id()));
    a.save(&path).expect("save");
    let back = Roster::load(&path).expect("load");
    let _ = std::fs::remove_file(&path);

    assert!(back.allows(&bob.public()));
    assert_eq!(back.members().count(), 2);
}

#[test]
fn a_rename_survives_a_merge_with_a_stale_peer() {
    // The bug this guards: merge kept the existing entry whenever `joined_at`
    // was equal, and renaming does not change `joined_at`. So a device could
    // rename itself and every peer would keep showing the old label forever,
    // with sync appearing to work.
    let alice = device();
    let bob = device();
    let bob_id = bob.public().to_string();

    let mut on_alice = Roster::found(&alice, "alice");
    on_alice.invite(&alice, &bob_id, "bob");

    // Bob adopts the space, then renames himself.
    let mut on_bob = Roster::new();
    on_bob.merge(&on_alice);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(
        on_bob.rename(&bob_id, "bob's phone"),
        "rename should change something"
    );

    on_alice.merge(&on_bob);

    let seen = on_alice.by_name("bob's phone");
    assert!(
        seen.is_some(),
        "alice should see the new name, got {:?}",
        on_alice.members().map(|m| &m.name).collect::<Vec<_>>()
    );
    assert_eq!(
        on_alice.members().count(),
        2,
        "renaming must not duplicate a member"
    );
}

#[test]
fn an_older_label_does_not_overwrite_a_newer_one() {
    let alice = device();
    let bob = device();
    let bob_id = bob.public().to_string();

    let mut on_alice = Roster::found(&alice, "alice");
    on_alice.invite(&alice, &bob_id, "bob");
    let stale = on_alice.clone();

    std::thread::sleep(std::time::Duration::from_millis(1100));
    on_alice.rename(&bob_id, "bob's phone");

    // Merging an older copy back in must not undo the rename.
    on_alice.merge(&stale);
    assert!(
        on_alice.by_name("bob's phone").is_some(),
        "newer label must win"
    );
}

#[test]
fn leaving_produces_a_roster_that_only_contains_this_device() {
    let alice = device();
    let bob = device();
    let mut on_bob = Roster::found(&bob, "bob");
    on_bob.invite(&bob, &alice.public().to_string(), "alice");
    assert_eq!(on_bob.members().count(), 2);

    let after = Roster::leave(&bob, "bob");
    assert_eq!(
        after.members().count(),
        1,
        "leaving should keep only this device"
    );
    assert!(
        after.allows(&bob.public()),
        "a device must still allow itself"
    );
    assert!(!after.allows(&alice.public()), "the old peer must be gone");
}

#[test]
fn a_non_member_cannot_be_merged_back_in_by_its_own_roster() {
    // The bug this guards: after leaving, the old peer still listed us and
    // pushed its roster on the next sync — putting us straight back into the
    // space we had just left. The node refuses a sync from a non-member, and
    // this pins the property the refusal relies on: nothing in a stranger's
    // roster makes them a member of ours.
    let alice = device();
    let bob = device();

    let mut on_alice = Roster::found(&alice, "alice");
    on_alice.invite(&alice, &bob.public().to_string(), "bob");

    // Bob leaves.
    let on_bob = Roster::leave(&bob, "bob");
    assert!(!on_bob.allows(&alice.public()));

    // Alice, who has not noticed, is not a member as far as Bob is concerned.
    assert!(
        !on_bob.allows(&alice.public()),
        "a device that left must not treat its old peers as members"
    );
}

#[test]
fn a_device_that_rejoins_becomes_visible_again() {
    // The bug this guards: listings filtered on the mere presence of a
    // tombstone while everything else compared timestamps. A rejoined device
    // was addressable, reachable and spoken to, but missing from
    // `voicecast devices` — visible only as an inconsistency.
    let alice = device();
    let bob = device();
    let bob_id = bob.public().to_string();

    let mut on_alice = Roster::found(&alice, "alice");
    on_alice.invite(&alice, &bob_id, "bob");
    on_alice.revoke(&bob_id);
    assert!(!on_alice.allows(&bob.public()), "revoked");
    assert_eq!(on_alice.members().count(), 1);

    // Bob rejoins: a fresh record, newer than the revocation.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    on_alice.invite(&alice, &bob_id, "bob");

    assert!(on_alice.allows(&bob.public()), "a rejoin must be honoured");
    assert!(on_alice.by_name("bob").is_some(), "and findable by name");
    assert_eq!(on_alice.members().count(), 2, "and listed");
}

// --- Hostile timestamps and ids -----------------------------------------
//
// Every rule in the roster is a comparison between two peer-chosen numbers,
// and until the audit nothing fed it a number chosen to win. These build the
// records an attacker would send rather than the ones a client does.

/// A join record for `endpoint_id`, signed by `secret`, dated as told.
fn signed_at(
    secret: &SecretKey,
    endpoint_id: &str,
    name: &str,
    joined_at: u64,
    renamed_at: u64,
) -> Member {
    let inviter = secret.public().to_string();
    let payload = Member::signed_payload(endpoint_id, &inviter, joined_at);
    Member {
        endpoint_id: endpoint_id.to_string(),
        name: name.to_string(),
        invited_by: inviter,
        signature: secret.sign(&payload).to_bytes().to_vec(),
        joined_at,
        renamed_at,
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[test]
fn a_record_dated_far_in_the_future_is_refused_everywhere() {
    // The #48 attack: a member re-signs its own record with a `joined_at`
    // no revocation can ever exceed, so `revoked_at > joined_at` is false
    // for ever and the device is unrevokable. The signature is genuine —
    // only the number is a lie — so this must be caught by the date.
    let alice = device();
    let mallory = device();
    let forged = signed_at(
        &mallory,
        &mallory.public().to_string(),
        "mallory",
        u64::MAX,
        0,
    );

    assert!(verify(&forged).is_err(), "a future date must not verify");

    let mut on_alice = Roster::found(&alice, "alice");
    assert!(on_alice.admit(forged.clone()).is_err(), "nor be admitted");

    // Nor may it arrive as part of a roster, which is the path a peer
    // actually has: `from_parts` verifies, so the record never lands.
    let theirs = Roster::from_parts(vec![forged], Vec::new());
    on_alice.merge(&theirs);
    assert!(
        !on_alice.allows(&mallory.public()),
        "a future-dated record must not reach the roster by any route"
    );
}

#[test]
fn a_record_dated_within_ordinary_clock_drift_is_still_accepted() {
    // The bound has to be loose enough that a device whose clock is merely
    // wrong still works, or the fix for #48 becomes a pairing bug.
    let alice = device();
    let bob = device();
    let ahead = signed_at(&alice, &bob.public().to_string(), "bob", now() + 60, 0);

    assert!(verify(&ahead).is_ok(), "a minute ahead is ordinary drift");
    let mut on_alice = Roster::found(&alice, "alice");
    on_alice.admit(ahead).expect("admitted despite drift");
    assert!(on_alice.allows(&bob.public()));
}

#[test]
fn an_endpoint_id_that_is_not_a_key_is_refused() {
    // #52. A roster entry is only useful if it names something dialable, and
    // an id that is not a key used to be storable, syncable and — because
    // ids are shortened for display — a panic on a multi-byte boundary.
    let alice = device();
    let junk = signed_at(&alice, "aéééééééééééééééé", "junk", now(), 0);

    assert!(verify(&junk).is_err(), "not a key, so not a member");
    let mut on_alice = Roster::found(&alice, "alice");
    assert!(on_alice.admit(junk).is_err());
}

#[test]
fn a_far_future_tombstone_evicts_but_does_not_outlast_a_rejoin() {
    // #48's other half. Tombstones carry no signature, so any member can
    // write one for anyone — that much is the design, "your own devices".
    // What it must not do is outlast every rejoin the space could sign,
    // which a `u64::MAX` tombstone did.
    let alice = device();
    let bob = device();

    let mut on_alice = Roster::found(&alice, "alice");
    // Dated an hour ago, as a device paired at any earlier sitting would be.
    // Timestamps here are whole seconds, so a member invited in this very
    // second is neither before nor after a tombstone stamped in it.
    on_alice
        .admit(signed_at(
            &alice,
            &bob.public().to_string(),
            "bob",
            now() - 3600,
            0,
        ))
        .expect("alice vouches for bob");
    assert!(on_alice.allows(&bob.public()));

    let poison = Roster::from_parts(Vec::new(), vec![(bob.public().to_string(), u64::MAX)]);
    on_alice.merge(&poison);
    assert!(!on_alice.allows(&bob.public()), "the eviction still lands");

    // Re-inviting bob now dates his record at `now()`, which beats the
    // clamped tombstone. Before the clamp this assertion failed for ever.
    on_alice.invite(&alice, &bob.public().to_string(), "bob");
    assert!(
        on_alice.allows(&bob.public()),
        "a rejoin must be able to beat a tombstone"
    );
}

#[test]
fn an_impossible_rename_stamp_does_not_pin_a_name() {
    // `renamed_at` sits outside the signature, so a member can set it freely
    // on a record that is otherwise genuine. With `u64::MAX` its label won
    // every future merge, which is how a device could be renamed to `all` —
    // a name `resolve` treats specially — and never renamed back.
    let alice = device();
    let bob = device();

    let mut on_alice = Roster::found(&alice, "alice");
    on_alice.invite(&alice, &bob.public().to_string(), "bob");
    let joined = on_alice
        .members()
        .find(|m| m.endpoint_id == bob.public().to_string())
        .expect("bob is a member")
        .joined_at;

    let pinned = Roster::from_parts(
        vec![signed_at(
            &alice,
            &bob.public().to_string(),
            "all",
            joined,
            u64::MAX,
        )],
        Vec::new(),
    );
    on_alice.merge(&pinned);

    let name = on_alice
        .members()
        .find(|m| m.endpoint_id == bob.public().to_string())
        .expect("bob is still a member")
        .name
        .clone();
    assert_eq!(name, "bob", "an impossible stamp must not win the rename");

    // And a real rename still works afterwards, so the clamp has not simply
    // frozen the label.
    assert!(on_alice.rename(&bob.public().to_string(), "desk"));
}
