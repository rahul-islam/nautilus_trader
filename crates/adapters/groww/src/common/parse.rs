// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Parsing helpers shared by the HTTP and streaming paths.

use jiff::{Timestamp, civil::Date, tz::TimeZone};
use nautilus_core::UnixNanos;

use crate::http::error::{Error, Result};

/// IANA identifier of the Indian market time zone.
///
/// Indian exchanges publish contract expiries and session boundaries as local calendar dates with
/// no offset, so converting them needs an explicit zone. India observes no daylight saving, but
/// resolving through the zone database keeps that an observation rather than an assumption.
pub const INDIA_TZ: &str = "Asia/Kolkata";

/// Returns the Indian market time zone.
///
/// # Errors
///
/// Returns an error if the time zone is unavailable in the platform's zone database.
pub fn india_tz() -> Result<TimeZone> {
    TimeZone::get(INDIA_TZ).map_err(|e| Error::decode(format!("unknown time zone {INDIA_TZ}: {e}")))
}

/// Converts milliseconds since the UNIX epoch to [`UnixNanos`].
///
/// # Errors
///
/// Returns an error if `millis` is negative or overflows when scaled to nanoseconds.
pub fn parse_millis(millis: i64) -> Result<UnixNanos> {
    let nanos = millis
        .checked_mul(1_000_000)
        .ok_or_else(|| Error::decode(format!("timestamp {millis}ms overflows nanoseconds")))?;
    u64::try_from(nanos)
        .map(UnixNanos::from)
        .map_err(|_| Error::decode(format!("timestamp {millis}ms precedes the UNIX epoch")))
}

/// Converts a floating-point millisecond timestamp to [`UnixNanos`].
///
/// The feed types every numeric field as a `double`, including timestamps, so millisecond values
/// arrive as floats and are truncated rather than rounded to stay monotone with the venue's own
/// integer milliseconds.
///
/// # Errors
///
/// Returns an error if the value is not finite, is negative, or exceeds the representable range.
pub fn parse_millis_f64(millis: f64) -> Result<UnixNanos> {
    // `i64::MAX` milliseconds overflows nanoseconds, so bound on the nanosecond range directly.
    const MAX_MILLIS: f64 = (u64::MAX / 1_000_000) as f64;

    if !millis.is_finite() {
        return Err(Error::decode(format!("non-finite timestamp: {millis}")));
    }
    if millis < 0.0 {
        return Err(Error::decode(format!(
            "timestamp {millis}ms precedes the UNIX epoch"
        )));
    }
    if millis > MAX_MILLIS {
        return Err(Error::decode(format!(
            "timestamp {millis}ms overflows nanoseconds"
        )));
    }
    Ok(UnixNanos::from((millis * 1_000_000.0) as u64))
}

/// Parses an RFC 3339 timestamp into [`UnixNanos`].
///
/// # Errors
///
/// Returns an error if the value is not a valid RFC 3339 timestamp.
pub fn parse_rfc3339(value: &str) -> Result<UnixNanos> {
    let timestamp: Timestamp = value
        .parse()
        .map_err(|e| Error::decode(format!("invalid RFC 3339 timestamp `{value}`: {e}")))?;
    let nanos = timestamp.as_nanosecond();
    u64::try_from(nanos)
        .map(UnixNanos::from)
        .map_err(|_| Error::decode(format!("timestamp `{value}` precedes the UNIX epoch")))
}

/// Parses a naive `YYYY-MM-DDTHH:MM:SS` timestamp reported in Indian local time.
///
/// Several Groww responses carry local wall-clock timestamps with no offset. Interpreting them as
/// UTC would shift every event by the India offset, so they are resolved through [`india_tz`].
///
/// # Errors
///
/// Returns an error if the value cannot be parsed or does not exist in the Indian time zone.
pub fn parse_naive_india(value: &str) -> Result<UnixNanos> {
    let civil: jiff::civil::DateTime = value
        .parse()
        .map_err(|e| Error::decode(format!("invalid local timestamp `{value}`: {e}")))?;
    let zoned = civil
        .to_zoned(india_tz()?)
        .map_err(|e| Error::decode(format!("unresolvable local timestamp `{value}`: {e}")))?;
    let nanos = zoned.timestamp().as_nanosecond();
    u64::try_from(nanos)
        .map(UnixNanos::from)
        .map_err(|_| Error::decode(format!("timestamp `{value}` precedes the UNIX epoch")))
}

/// Parses a `YYYY-MM-DD` expiry date into the UNIX nanoseconds of its Indian-market close.
///
/// Nautilus expires a contract at an instant, while the instrument master publishes only the
/// expiry date. The instant is taken as the end of that day in the Indian time zone, so a contract
/// stays live for the whole of its final session.
///
/// # Errors
///
/// Returns an error if the value is not a valid date.
pub fn parse_expiry_date(value: &str) -> Result<UnixNanos> {
    let date: Date = value
        .parse()
        .map_err(|e| Error::decode(format!("invalid expiry date `{value}`: {e}")))?;
    let zoned = date
        .to_datetime(jiff::civil::time(23, 59, 59, 0))
        .to_zoned(india_tz()?)
        .map_err(|e| Error::decode(format!("unresolvable expiry date `{value}`: {e}")))?;
    let nanos = zoned.timestamp().as_nanosecond();
    u64::try_from(nanos)
        .map(UnixNanos::from)
        .map_err(|_| Error::decode(format!("expiry `{value}` precedes the UNIX epoch")))
}

/// Returns the number of decimal places implied by a decimal string.
///
/// The instrument master expresses tick size as a decimal string (`0.05`, `0.1`), which fixes the
/// price precision for that instrument. Deriving precision from the string preserves a trailing
/// zero that a float round-trip would discard.
#[must_use]
pub fn precision_from_str(value: &str) -> u8 {
    match value.trim().split_once('.') {
        Some((_, frac)) => {
            let frac = frac.trim_end_matches(|c: char| !c.is_ascii_digit());
            u8::try_from(frac.len()).unwrap_or(u8::MAX)
        }
        None => 0,
    }
}
