//! Rate conversions, kept where they can be tested.
//!
//! Every engine takes a speaking rate in its own units and the trait speaks
//! in multipliers of normal. The conversion is arithmetic and has nothing
//! platform-specific about it *except* which platform's numbers it targets —
//! so putting it beside the platform code would be tidier and would mean it
//! is never run.
//!
//! `cargo test` runs on `ubuntu-latest` and nowhere else. A test inside
//! `#[cfg(windows)]` is not a test that runs on Windows; it is a test that
//! runs nowhere, which is worse than no test because the count says
//! otherwise. `CLAUDE.md` makes the same point about cross-platform
//! assertions being executed on Linux only — this is the sharper version of
//! it, where the number is zero.

/// A multiplier of normal as SAPI 5's rate.
///
/// SAPI takes an integer from -10 to 10 where 0 is normal, and the scale is
/// multiplicative rather than linear — each step is a fixed ratio, not a
/// fixed number of words. `log2` is therefore the honest mapping, and it
/// lands exactly on the ends: half speed is -10, double is +10, normal is 0.
///
/// Clamped rather than extrapolated, because SAPI has no rate outside that
/// range and a value it refuses is worse than a value it merely rounds.
// Its only caller is `sapi.rs`, which exists on Windows alone — so on every
// other target this is a function with no callers, and `-D warnings` makes
// that an error rather than a warning. Allowed exactly where it is genuinely
// uncalled, rather than everywhere, so that a *real* dead function here is
// still caught on the platform that uses it.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn sapi_rate(multiplier: f32) -> i32 {
    let m = multiplier.clamp(0.5, 2.0);
    (m.log2() * 10.0).round().clamp(-10.0, 10.0) as i32
}

#[cfg(test)]
mod tests {
    use super::sapi_rate;

    #[test]
    fn the_sapi_rate_scale_lands_on_its_ends_and_its_middle() {
        assert_eq!(sapi_rate(1.0), 0, "normal is zero");
        assert_eq!(sapi_rate(2.0), 10, "double is the top");
        assert_eq!(sapi_rate(0.5), -10, "half is the floor");

        // Clamped, not extrapolated.
        assert_eq!(sapi_rate(8.0), 10);
        assert_eq!(sapi_rate(0.01), -10);

        // Multiplicative, so the square root of two is halfway to double in
        // the way a listener hears it, rather than 1.5 being halfway.
        assert_eq!(sapi_rate(std::f32::consts::SQRT_2), 5);

        // Monotonic across the whole range, which is the property a person
        // actually notices: dragging a slider one way must never speed
        // speech up and then slow it down.
        let mut last = i32::MIN;
        for step in 0..=40 {
            let m = 0.4 + (step as f32) * 0.05;
            let rate = sapi_rate(m);
            assert!(
                rate >= last,
                "rate went backwards at {m}: {rate} after {last}"
            );
            last = rate;
        }
    }
}
