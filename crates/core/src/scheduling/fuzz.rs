//! Deterministic interval fuzz.
//!
//! Fuzz spreads cards that were learned together over neighbouring days. It
//! must be reproducible: replaying the review log has to give the same due
//! dates. So the "random" number is a hash of the card id and the number of
//! reviews before this one, never a real random source.
//!
//! The ranges are Anki's (`rslib/src/scheduler/states/fuzz.rs`): no fuzz
//! below 2.5 days; the spread is 1 day, plus 15 % of the part of the
//! interval between 2.5 and 7 days, 10 % of the part between 7 and 20 days,
//! and 5 % of the part above 20 days.

/// Longest interval, in days (100 years; FSRS's own stability cap).
pub(crate) const MAX_INTERVAL_DAYS: u32 = 36_500;

/// 2^53 as `f64` (exact).
const TWO_POW_53: f64 = 9_007_199_254_740_992.0;

/// `(start, end, factor)` in days.
const FUZZ_RANGES: [(f64, f64, f64); 3] = [
    (2.5, 7.0, 0.15),
    (7.0, 20.0, 0.10),
    (20.0, f64::INFINITY, 0.05),
];

/// A number in `[0, 1)` that depends only on `card_id` and `reps_before`.
///
/// FNV-1a over the id bytes and `reps_before` (little-endian), mixed with
/// the `SplitMix64` finaliser. `std`'s `Hasher` is not used because its
/// output may change between Rust versions, which would reschedule cards
/// on a rebuild. `fuzz_factor_is_pinned` pins the output.
pub(crate) fn fuzz_factor(card_id: &str, reps_before: u32) -> f64 {
    let fnv = card_id
        .bytes()
        .chain(reps_before.to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| {
            (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    let mut z = fnv.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    // The top 53 bits fill an f64 mantissa exactly: uniform on [0, 1).
    #[allow(clippy::cast_precision_loss)] // z >> 11 < 2^53: exactly representable.
    let top = (z >> 11) as f64;
    top / TWO_POW_53
}

/// The days `interval` may be fuzzed to, inclusive: `(lower, upper)`.
/// `interval` is FSRS's unrounded interval in days.
pub(crate) fn fuzz_range(interval: f64) -> (u32, u32) {
    let max = f64::from(MAX_INTERVAL_DAYS);
    let interval = if interval.is_nan() {
        1.0
    } else {
        interval.clamp(1.0, max)
    };
    if interval < 2.5 {
        let days = to_days(interval.round());
        return (days, days);
    }
    let delta = FUZZ_RANGES
        .iter()
        .fold(1.0, |delta, &(start, end, factor)| {
            delta + factor * (interval.min(end) - start).max(0.0)
        });
    let upper = to_days((interval + delta).round().min(max));
    let lower = to_days((interval - delta).round().max(2.0)).min(upper);
    (lower, upper)
}

/// The fuzzed interval in days, in `1..=MAX_INTERVAL_DAYS`. `factor` is in
/// `[0, 1)` (from [`fuzz_factor`]); each day of the range is equally likely.
pub(crate) fn fuzzed_days(interval: f64, factor: f64) -> u32 {
    let (lower, upper) = fuzz_range(interval);
    let span = f64::from(upper - lower + 1);
    let offset = to_days((factor.clamp(0.0, 1.0) * span).floor());
    (lower + offset).min(upper)
}

/// A whole, non-negative day count that fits in `u32` to `u32`.
fn to_days(days: f64) -> u32 {
    debug_assert!(days >= 0.0 && days <= f64::from(u32::MAX) && days.fract() == 0.0);
    // Callers only pass whole numbers already clamped to
    // [0, MAX_INTERVAL_DAYS + 1], so nothing is truncated or wrapped.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let days = days as u32;
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_intervals_are_not_fuzzed() {
        assert_eq!(fuzz_range(0.2), (1, 1));
        assert_eq!(fuzz_range(1.0), (1, 1));
        assert_eq!(fuzz_range(1.6), (2, 2));
        assert_eq!(fuzz_range(2.49), (2, 2));
        assert_eq!(fuzz_range(f64::NAN), (1, 1));
    }

    #[test]
    fn ranges_match_anki() {
        // delta = 1 + 0.15 * (3 - 2.5) = 1.075
        assert_eq!(fuzz_range(3.0), (2, 4));
        // delta = 1 + 0.15 * 4.5 + 0.1 * 3 = 1.975
        assert_eq!(fuzz_range(10.0), (8, 12));
        // delta = 1 + 0.675 + 1.3 + 0.05 * 80 = 6.975
        assert_eq!(fuzz_range(100.0), (93, 107));
        // Clamped to the maximum first: delta = 1 + 0.675 + 1.3 + 0.05 * 36_480.
        assert_eq!(fuzz_range(1e9), (34_673, 36_500));
    }

    #[test]
    fn fuzzed_days_cover_the_range_evenly() {
        let (lower, upper) = fuzz_range(10.0);
        let mut counts = [0_u32; 5];
        for i in 0..5000 {
            let d = fuzzed_days(10.0, f64::from(i) / 5000.0);
            counts[(d - lower) as usize] += 1;
        }
        assert_eq!(counts, [1000; 5]);
        assert_eq!(fuzzed_days(10.0, 0.0), lower);
        assert_eq!(fuzzed_days(10.0, 0.999_999), upper);
    }

    /// If this fails, the fuzz hash changed: every rebuild would move due
    /// dates. Only change these values on purpose, with an ADR.
    #[test]
    fn fuzz_factor_is_pinned() {
        let id = "01929d4e-7a3c-7b12-8f00-0123456789ab";
        let got: Vec<u64> = (0..3).map(|reps| fuzz_factor(id, reps).to_bits()).collect();
        assert_eq!(got, PINNED);
    }
    /// Cross-checked with an independent Python implementation:
    /// 0.6456126346934299, 0.8330961480805513, 0.694816238533654.
    const PINNED: [u64; 3] = [
        4_603_990_381_187_338_446,
        4_605_679_082_549_395_149,
        4_604_433_567_851_178_684,
    ];

    #[test]
    fn fuzz_factor_is_in_unit_interval_and_varies() {
        let values: Vec<f64> = (0..1000).map(|r| fuzz_factor("card", r)).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = values.iter().sum::<f64>() / 1000.0;
        assert!((0.45..0.55).contains(&mean), "mean {mean}");
        assert_ne!(fuzz_factor("a", 0), fuzz_factor("b", 0));
    }
}
