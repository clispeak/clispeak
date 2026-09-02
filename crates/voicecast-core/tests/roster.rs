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
