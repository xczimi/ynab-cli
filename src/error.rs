#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not logged in — run `ynab auth login` (or `ynab auth login --oauth`)")]
    NotAuthenticated,
    #[error("rate limited by YNAB (200 requests/hour) — resets within the hour")]
    RateLimited,
    #[error("YNAB API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("unexpected API response: {0}")]
    Decode(String),
    #[error("cache error: {0}")]
    Cache(String),
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// stdout was closed by the reader (`ynab ... | head`). Not a
    /// failure: the caller got what it asked for and hung up.
    #[error("broken pipe")]
    BrokenPipe,
}

pub type Result<T> = std::result::Result<T, Error>;

/// True for `Error::Cache` — a corrupted/undecryptable cache is never a
/// user-facing error (CLAUDE.md). Budget-scoped list commands use this to
/// fall back to a direct API fetch when the sync path fails mid-operation,
/// instead of propagating the error to the user.
pub fn cache_error(e: &Error) -> bool {
    matches!(e, Error::Cache(_))
}

/// True when the reader closed stdout on us. `main` exits 0 and silent
/// on this, matching how every other Unix filter behaves under `| head`.
pub fn broken_pipe(e: &Error) -> bool {
    matches!(e, Error::BrokenPipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_messages() {
        assert_eq!(
            Error::NotAuthenticated.to_string(),
            "not logged in — run `ynab auth login` (or `ynab auth login --oauth`)"
        );
        assert_eq!(
            Error::RateLimited.to_string(),
            "rate limited by YNAB (200 requests/hour) — resets within the hour"
        );
        assert_eq!(
            Error::Api {
                status: 500,
                message: "boom".into()
            }
            .to_string(),
            "YNAB API error (500): boom"
        );
        assert_eq!(
            Error::Config("bad key".into()).to_string(),
            "config error: bad key"
        );
    }

    #[test]
    fn cache_error_matches_only_cache_variant() {
        assert!(cache_error(&Error::Cache("boom".into())));
        assert!(!cache_error(&Error::NotAuthenticated));
        assert!(!cache_error(&Error::RateLimited));
        assert!(!cache_error(&Error::Config("x".into())));
    }
}
