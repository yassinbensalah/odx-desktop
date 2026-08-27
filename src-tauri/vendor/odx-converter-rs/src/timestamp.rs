// src/timestamp.rs – Reproducible, Kotlin-compatible timestamp formatting.
//
// Mirrors Kotlin's `FileConverter.getCurrentTimeReproducible()`
// (converter/src/main/kotlin/FileConverter.kt):
//
//   private fun getCurrentTimeReproducible(): Instant {
//       val epochSeconds = System.getenv("SOURCE_DATE_EPOCH")?.toLong() ?: Instant.now().epochSecond
//       return Instant.ofEpochSecond(epochSeconds)
//   }
//
// and the string format produced by Java's `java.time.Instant.toString()`,
// which the Kotlin code relies on implicitly wherever it writes
// `getCurrentTimeReproducible().toString()` into MDD metadata:
//
//   - Always UTC, always suffixed with "Z" (never "+00:00").
//   - Fractional seconds are included only if non-zero, and are printed
//     with the *minimum* number of 3-digit groups needed (millis, then
//     micros, then nanos) — trailing all-zero groups are dropped.
//     e.g. 2026-07-15T10:51:15Z / ...15.500Z / ...15.500200Z / ...15.500200900Z
//
// `chrono`'s `to_rfc3339()` produces "+00:00" and always-6-or-9-digit
// fractional seconds, so it can't be used directly here.

use std::time::{SystemTime, UNIX_EPOCH};

/// Returns (seconds since epoch, nanosecond-of-second) for "now", or for
/// `SOURCE_DATE_EPOCH` if that environment variable is set to a valid
/// integer — exactly mirroring the Kotlin reproducible-build behaviour.
/// `SOURCE_DATE_EPOCH` only carries whole seconds, so the nanosecond part
/// is 0 in that case (matching `Instant.ofEpochSecond(epochSeconds)`, which
/// likewise has no sub-second component).
fn reproducible_now() -> (i64, u32) {
    if let Ok(val) = std::env::var("SOURCE_DATE_EPOCH") {
        if let Ok(secs) = val.trim().parse::<i64>() {
            return (secs, 0);
        }
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (now.as_secs() as i64, now.subsec_nanos())
}

/// Formats like Java's `Instant.toString()`: UTC, trailing "Z", with
/// fractional seconds only when non-zero and trimmed to the shortest
/// 3/6/9-digit group.
pub fn current_reproducible_timestamp() -> String {
    let (secs, nanos) = reproducible_now();
    format_instant(secs, nanos)
}

fn format_instant(epoch_secs: i64, nanos: u32) -> String {
    let datetime = civil_from_epoch_seconds(epoch_secs);
    let frac = format_fraction(nanos);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}Z",
        datetime.0, datetime.1, datetime.2, datetime.3, datetime.4, datetime.5, frac
    )
}

/// Millis if evenly divisible into millis, else micros, else full nanos;
/// empty string if exactly zero. Trailing zero-only groups beyond the
/// needed precision are dropped, matching Instant.toString()'s behaviour.
fn format_fraction(nanos: u32) -> String {
    if nanos == 0 {
        return String::new();
    }
    if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!(".{:06}", nanos / 1_000)
    } else {
        format!(".{:09}", nanos)
    }
}

/// Converts a Unix epoch-seconds value to (year, month, day, hour, min, sec)
/// in UTC using civil calendar math (Howard Hinnant's `civil_from_days`
/// algorithm), avoiding a dependency on chrono/time for this one conversion.
fn civil_from_epoch_seconds(epoch_secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = epoch_secs.div_euclid(86_400);
    let secs_of_day = epoch_secs.rem_euclid(86_400);
    let hour = (secs_of_day / 3600) as u32;
    let min = ((secs_of_day % 3600) / 60) as u32;
    let sec = (secs_of_day % 60) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    (year, m, d, hour, min, sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero() {
        assert_eq!(format_instant(0, 0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn no_fraction_when_zero() {
        assert_eq!(format_instant(1_752_000_000, 0), "2025-07-08T18:40:00Z");
    }

    #[test]
    fn millis_precision() {
        assert_eq!(format_instant(0, 500_000_000), "1970-01-01T00:00:00.500Z");
    }

    #[test]
    fn micros_precision() {
        assert_eq!(format_instant(0, 500_200_000), "1970-01-01T00:00:00.500200Z");
    }

    #[test]
    fn nanos_precision() {
        assert_eq!(format_instant(0, 500_200_900), "1970-01-01T00:00:00.500200900Z");
    }
}
