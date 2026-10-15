//! `UUIDv7` ids that strictly increase, even within one millisecond and
//! when the clock goes backwards (ADR 0008).
//!
//! A `UUIDv7` is 128 bits: a 48-bit Unix millisecond timestamp, the version
//! (4 bits, `7`), 12 random bits, the variant (2 bits, `10`) and 62 random
//! bits. Leaving out the 6 fixed bits, an id is a 122-bit number
//! `v = ts · 2^74 + r` with `r` the 74 random bits, and the order of ids
//! (as numbers, or as lowercase hex strings) is the order of `v`.
//!
//! [`IdGen`] remembers the last `v` it issued or was shown and returns
//! `max(candidate, last + 1)`, where the candidate is the current time with
//! fresh random bits. So every id is greater than every earlier one. If
//! `r` overflows, the carry goes into the timestamp.

use crate::error::{Error, Result};

/// Bits of an id that are random (`rand_a` and `rand_b`).
const RANDOM_BITS: u32 = 74;
/// Bits of `rand_b`, the low random part.
const RAND_B_BITS: u32 = 62;
/// The largest timestamp a `UUIDv7` can hold (year 10889).
const MAX_TS: u128 = (1 << 48) - 1;
/// The largest 122-bit value.
const MAX_VALUE: u128 = (1 << (48 + RANDOM_BITS)) - 1;

/// Issues strictly increasing `UUIDv7` strings.
#[derive(Debug, Clone, Default)]
pub(crate) struct IdGen {
    /// The largest 122-bit value issued or observed, if any.
    last: Option<u128>,
}

impl IdGen {
    /// A generator that has issued nothing yet.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A new id for time `now_ms` (UTC milliseconds), greater than every id
    /// this generator issued or observed.
    ///
    /// # Errors
    /// [`Error::Random`] if the operating system gives no random bytes;
    /// [`Error::IdsExhausted`] after the last id of the year 10889.
    pub(crate) fn next(&mut self, now_ms: i64) -> Result<String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|err| Error::Random(Box::new(err)))?;
        self.next_with(now_ms, u128::from_le_bytes(bytes))
    }

    /// [`Self::next`] with the random bits given (only the low 74 are used).
    fn next_with(&mut self, now_ms: i64, random: u128) -> Result<String> {
        let ts = u128::try_from(now_ms).unwrap_or(0).min(MAX_TS);
        let candidate = (ts << RANDOM_BITS) | (random & ((1 << RANDOM_BITS) - 1));
        let value = match self.last {
            Some(last) if candidate <= last => last
                .checked_add(1)
                .filter(|&v| v <= MAX_VALUE)
                .ok_or(Error::IdsExhausted)?,
            _ => candidate,
        };
        self.last = Some(value);
        Ok(format_id(value))
    }

    /// Makes every later id greater than `id` (an id read from the
    /// database, possibly issued by another run with a clock further
    /// ahead).
    ///
    /// Returns `false`, and changes nothing, if `id` is not a lowercase
    /// UUID string.
    pub(crate) fn observe(&mut self, id: &str) -> bool {
        let Some(value) = parse_id(id) else {
            return false;
        };
        self.last = Some(self.last.map_or(value, |last| last.max(value)));
        true
    }
}

/// The lowercase hyphenated `UUIDv7` string of the 122-bit `value`.
fn format_id(value: u128) -> String {
    let ts = value >> RANDOM_BITS;
    let rand_a = (value >> RAND_B_BITS) & 0xfff;
    let rand_b = value & ((1 << RAND_B_BITS) - 1);
    let uuid = (ts << 80) | (0x7 << 76) | (rand_a << 64) | (0b10 << 62) | rand_b;
    let hex = format!("{uuid:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The 122-bit value of a lowercase hyphenated UUID string (the version
/// and variant bits are dropped); `None` if `id` is not one.
fn parse_id(id: &str) -> Option<u128> {
    let bytes = id.as_bytes();
    let well_formed = bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        });
    if !well_formed {
        return None;
    }
    let hex: String = id.chars().filter(|&c| c != '-').collect();
    let uuid = u128::from_str_radix(&hex, 16).ok()?;
    let ts = uuid >> 80;
    let rand_a = (uuid >> 64) & 0xfff;
    let rand_b = uuid & ((1 << RAND_B_BITS) - 1);
    Some((ts << RANDOM_BITS) | (rand_a << RAND_B_BITS) | rand_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-15T12:34:56.789Z
    const T0: i64 = 1_792_067_696_789;

    fn ts_of(id: &str) -> i64 {
        i64::from_str_radix(&id.replace('-', "")[..12], 16).unwrap()
    }

    #[test]
    fn ids_are_well_formed_uuidv7() {
        let mut g = IdGen::new();
        let id = g.next(T0).unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(id, id.to_lowercase());
        assert_eq!(&id[14..15], "7", "version nibble: {id}");
        assert!("89ab".contains(&id[19..20]), "variant nibble: {id}");
        assert_eq!(ts_of(&id), T0);
        assert_eq!(parse_id(&id).map(format_id), Some(id));
    }

    #[test]
    fn format_and_parse_are_inverse() {
        for v in [
            0,
            1,
            MAX_VALUE,
            MAX_VALUE >> 1,
            0x1234_5678_9abc_def0_1234_5678_9abc,
        ] {
            assert_eq!(parse_id(&format_id(v)), Some(v), "{v:x}");
        }
    }

    #[test]
    fn parse_rejects_malformed_ids() {
        let ok = format_id(0xabc_def0_1234_5678);
        assert!(parse_id(&ok).is_some());
        for bad in [
            String::new(),
            ok.to_uppercase(),
            ok.replace('-', "_"),
            ok[..35].to_owned(),
            format!("{ok}0"),
            ok.replacen('0', "g", 1),
        ] {
            assert_eq!(parse_id(&bad), None, "{bad}");
        }
    }

    #[test]
    fn many_ids_in_one_millisecond_strictly_increase() {
        let mut g = IdGen::new();
        let mut prev = g.next(T0).unwrap();
        for _ in 0..100_000 {
            let id = g.next(T0).unwrap();
            assert!(id > prev, "{id} <= {prev}");
            prev = id;
        }
    }

    #[test]
    fn ids_increase_when_the_clock_goes_backwards() {
        let mut g = IdGen::new();
        let ids: Vec<String> = [T0, T0 - 60_000, 0, -5, T0 + 1]
            .into_iter()
            .map(|t| g.next(t).unwrap())
            .collect();
        for pair in ids.windows(2) {
            assert!(pair[0] < pair[1], "{pair:?}");
        }
        // Once the clock is ahead again, ids carry its time.
        assert_eq!(ts_of(&ids[4]), T0 + 1);
    }

    #[test]
    fn smallest_random_bits_still_increase() {
        let mut g = IdGen::new();
        let a = g.next_with(T0, u128::MAX).unwrap();
        let b = g.next_with(T0, 0).unwrap();
        assert!(a < b);
    }

    #[test]
    fn random_overflow_carries_into_the_timestamp() {
        let mut g = IdGen::new();
        let a = g.next_with(T0, u128::MAX).unwrap();
        assert_eq!(ts_of(&a), T0);
        let b = g.next_with(T0, u128::MAX).unwrap();
        assert!(a < b);
        assert_eq!(ts_of(&b), T0 + 1);
        assert_eq!(parse_id(&b), Some(parse_id(&a).unwrap() + 1));
    }

    #[test]
    fn observed_ids_bound_later_ones() {
        let mut ahead = IdGen::new();
        let future = ahead.next(T0 + 3_600_000).unwrap();

        // A fresh generator (a restart) whose clock is behind.
        let mut g = IdGen::new();
        assert!(g.observe(&future));
        let id = g.next(T0).unwrap();
        assert!(id > future);

        // Observing an older id changes nothing.
        let old = format_id(1);
        assert!(g.observe(&old));
        assert!(g.next(T0).unwrap() > id);

        assert!(!g.observe("not an id"));
    }

    #[test]
    fn the_last_id_is_refused_not_repeated() {
        let mut g = IdGen::new();
        assert!(g.observe(&format_id(MAX_VALUE)));
        assert!(matches!(g.next(T0), Err(Error::IdsExhausted)));
    }

    #[test]
    fn time_beyond_the_48_bit_range_is_clamped() {
        let mut g = IdGen::new();
        let id = g.next_with(i64::MAX, 0).unwrap();
        assert_eq!(id, "ffffffff-ffff-7000-8000-000000000000");
    }
}
