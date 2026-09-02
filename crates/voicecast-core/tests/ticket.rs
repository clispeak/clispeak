//! Invite tickets: round-tripping, expiry, and the QR rendering.

use voicecast_core::{Ticket, qr_svg};

#[test]
fn a_ticket_round_trips_through_its_url() {
    let ticket = Ticket::mint("abc123".into(), None);
    let url = ticket.to_url().expect("encode");
    assert!(url.starts_with("voicecast://join/"));

    let back = Ticket::parse(&url).expect("decode");
    assert_eq!(back.endpoint_id, "abc123");
    assert_eq!(back.token, ticket.token);
}

#[test]
fn the_url_prefix_is_optional() {
    let url = Ticket::mint("abc123".into(), None).to_url().unwrap();
    let bare = url.strip_prefix("voicecast://join/").unwrap();
    assert!(Ticket::parse(bare).is_ok(), "a bare code should still work");
}

#[test]
fn unhelpful_input_gets_a_helpful_error() {
    // These messages are read by a person who just mis-pasted something, so
    // they must say what to do rather than name a decoder.
    let empty = Ticket::parse("   ").unwrap_err().to_string();
    assert!(empty.contains("paste an invite"), "got {empty:?}");

    let junk = Ticket::parse("not-a-ticket!!").unwrap_err().to_string();
    assert!(junk.contains("does not look like"), "got {junk:?}");
}

#[test]
fn a_real_invite_fits_in_a_qr_code() {
    // The payload is a base32 blob a couple of hundred characters long. If it
    // ever outgrows what a QR can hold, pairing silently loses its primary
    // flow — so pin that here rather than finding out on a phone.
    let url = Ticket::mint(
        "5dc736e3b1b945e3c46bdc4e09438021230d55ac8c4e98e993b41a8f93422eba".into(),
        None,
    )
    .to_url()
    .unwrap();

    let svg = qr_svg(&url).expect("a real invite must encode");
    // The renderer emits an XML declaration first, which is valid SVG.
    assert!(
        svg.contains("<svg"),
        "expected svg, got {:?}",
        &svg[..40.min(svg.len())]
    );
    assert!(
        svg.len() > 500,
        "suspiciously small QR: {} bytes",
        svg.len()
    );
}
