pub mod listener;
pub mod oauth;

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, per `SystemTime::now()`. A clock somehow
/// before the epoch (never expected in practice) clamps to 0 rather than
/// panicking.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
