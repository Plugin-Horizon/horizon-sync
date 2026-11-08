//! UUIDs, versions and times as HuskSync writes them.

use std::cmp::Ordering;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// `UUID.toString()`: lowercase, hyphenated.
pub fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// `UUID.fromString` for the canonical form (also accepts no hyphens).
pub fn parse_uuid(s: &str) -> Option<u128> {
    let hex: String = s.trim().chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok()
}

/// A random (version 4) UUID from the system random source (`getrandom`, in Horizon's lock).
pub fn random_uuid() -> u128 {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // The system has no random source: still unique, as before.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut hi = RandomState::new().build_hasher();
        hi.write_u64(COUNTER.fetch_add(1, AtomicOrdering::Relaxed));
        hi.write_u128(now_nanos());
        bytes = (((hi.finish() as u128) << 64) | RandomState::new().build_hasher().finish() as u128).to_be_bytes();
    }
    let mut v = u128::from_be_bytes(bytes);
    v &= !(0xf000u128 << 64);
    v |= 0x4000u128 << 64; // version 4
    v &= !(0xc000u128 << 48);
    v |= 0x8000u128 << 48; // IETF variant
    v
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// desertwell's `Version`: dot-separated numbers, compared piecewise; anything after `-` is
/// metadata and ignored.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |s: &str| -> Vec<u64> {
        s.split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i).copied().unwrap_or(0), pb.get(i).copied().unwrap_or(0));
        match x.cmp(&y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    Ordering::Equal
}

/// `Version.toStringWithoutMetadata`.
pub fn strip_version_metadata(v: &str) -> &str {
    v.split('-').next().unwrap_or(v)
}

/// HuskSync's `DataVersionSupplier.getDataVersion`: the data version of a Minecraft release, for
/// items saved without their own `DataVersion`.
pub fn data_version_of(mc: &str) -> i32 {
    match strip_version_metadata(mc) {
        "1.16" | "1.16.1" | "1.16.2" | "1.16.3" | "1.16.4" | "1.16.5" => 2586,
        "1.17" | "1.17.1" => 2730,
        "1.18" | "1.18.1" | "1.18.2" => 2975,
        "1.19" | "1.19.1" | "1.19.2" => 3120,
        "1.19.4" => 3337,
        "1.20" | "1.20.1" => 3465,
        "1.20.2" => 3578,
        "1.20.4" => 3700,
        "1.20.5" | "1.20.6" => 3837,
        "1.21" | "1.21.1" => 3955,
        "1.21.2" | "1.21.3" => 4082,
        "1.21.4" => 4189,
        "1.21.5" => 4323,
        "1.21.6" => 4435,
        "1.21.7" | "1.21.8" => 4438,
        "1.21.9" => 4554,
        "1.21.10" => 4556,
        "1.21.11" => 4671,
        "26.1" | "26.1.1" | "26.1.2" => 4786,
        "26.2" => 4903,
        _ => 5023,
    }
}

/// The current local time with its offset, as `OffsetDateTime.now()`.
pub fn now() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Local::now().fixed_offset()
}

/// gson-javatime-serialisers: `DateTimeFormatter.ISO_OFFSET_DATE_TIME`.
pub fn format_timestamp(t: &chrono::DateTime<chrono::FixedOffset>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, false)
}

pub fn parse_timestamp(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids() {
        let u = parse_uuid("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        assert_eq!(uuid_string(u), "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        let r = random_uuid();
        assert_eq!(uuid_string(r).chars().nth(14), Some('4'));
        assert_ne!(random_uuid(), r);
    }

    #[test]
    fn versions() {
        assert_eq!(compare_versions("26.3", "26.3-R0.1-SNAPSHOT"), Ordering::Equal);
        assert_eq!(compare_versions("1.21.11", "26.3"), Ordering::Less);
        assert_eq!(compare_versions("26.3.1", "26.3"), Ordering::Greater);
        assert_eq!(data_version_of("1.21.11"), 4671);
    }

    #[test]
    fn timestamps() {
        let t = parse_timestamp("2026-11-06T06:00:00.5+01:00").unwrap();
        assert_eq!(format_timestamp(&t), "2026-11-06T06:00:00.500+01:00");
        assert!(parse_timestamp("2026-11-06T06:00:00Z").is_some());
    }
}
