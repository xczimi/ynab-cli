#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not logged in — run `ynab auth login`")]
    NotAuthenticated,
    #[error("rate limited by YNAB (200 requests/hour) — resets within the hour")]
    RateLimited,
    #[error("YNAB API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_messages() {
        assert_eq!(
            Error::NotAuthenticated.to_string(),
            "not logged in — run `ynab auth login`"
        );
        assert_eq!(
            Error::RateLimited.to_string(),
            "rate limited by YNAB (200 requests/hour) — resets within the hour"
        );
        assert_eq!(
            Error::Api { status: 500, message: "boom".into() }.to_string(),
            "YNAB API error (500): boom"
        );
        assert_eq!(
            Error::Config("bad key".into()).to_string(),
            "config error: bad key"
        );
    }
}
