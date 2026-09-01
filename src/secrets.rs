use keyring::Entry;
use secrecy::{ExposeSecret, SecretString};

use crate::error::Result;

const SERVICE: &str = "ynab-cli";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    Pat,
    /// All OAuth state — app credentials plus the token pair — as one JSON
    /// document. It is deliberately a single entry: keychain ACLs are
    /// per-item, so every extra entry is another authorization prompt the
    /// user has to grant (see `auth::oauth`).
    Oauth,
    CacheKey,
    /// Written by installs that predate the single-entry `Oauth` layout.
    /// Read only by the migration in `auth::oauth`, and deleted by it and
    /// by `auth logout`; nothing else may use these.
    LegacyOauthClientId,
    LegacyOauthClientSecret,
    LegacyOauthAccessToken,
    LegacyOauthRefreshToken,
}

/// Every legacy entry, in one place so migration and logout can't drift.
pub const LEGACY_OAUTH_KINDS: [SecretKind; 4] = [
    SecretKind::LegacyOauthClientId,
    SecretKind::LegacyOauthClientSecret,
    SecretKind::LegacyOauthAccessToken,
    SecretKind::LegacyOauthRefreshToken,
];

impl SecretKind {
    fn entry_name(self) -> &'static str {
        match self {
            SecretKind::Pat => "pat",
            SecretKind::Oauth => "oauth",
            SecretKind::CacheKey => "cache-key",
            SecretKind::LegacyOauthClientId => "oauth-client-id",
            SecretKind::LegacyOauthClientSecret => "oauth-client-secret",
            SecretKind::LegacyOauthAccessToken => "oauth-access-token",
            SecretKind::LegacyOauthRefreshToken => "oauth-refresh-token",
        }
    }
}

/// Holds one keyring Entry per SecretKind, created eagerly. Entries are
/// reused (not recreated per call) because keyring's mock store — used in
/// tests — keeps credential state per-Entry instance.
pub struct SecretStore {
    entries: [Entry; 7],
}

impl SecretStore {
    pub fn new() -> Result<Self> {
        // Array order MUST match SecretKind discriminant order (`as usize`).
        let mk = |kind: SecretKind| Entry::new(SERVICE, kind.entry_name());
        Ok(SecretStore {
            entries: [
                mk(SecretKind::Pat)?,
                mk(SecretKind::Oauth)?,
                mk(SecretKind::CacheKey)?,
                mk(SecretKind::LegacyOauthClientId)?,
                mk(SecretKind::LegacyOauthClientSecret)?,
                mk(SecretKind::LegacyOauthAccessToken)?,
                mk(SecretKind::LegacyOauthRefreshToken)?,
            ],
        })
    }

    fn entry(&self, kind: SecretKind) -> &Entry {
        &self.entries[kind as usize]
    }

    pub fn get(&self, kind: SecretKind) -> Result<Option<SecretString>> {
        match self.entry(kind).get_password() {
            Ok(value) => Ok(Some(SecretString::from(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set(&self, kind: SecretKind, value: SecretString) -> Result<()> {
        self.entry(kind).set_password(value.expose_secret())?;
        Ok(())
    }

    pub fn delete(&self, kind: SecretKind) -> Result<()> {
        match self.entry(kind).delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::{ExposeSecret, SecretString};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn set_get_delete_roundtrip() {
        let store = mock_store();
        assert!(store.get(SecretKind::Pat).unwrap().is_none());

        store
            .set(SecretKind::Pat, SecretString::from("tok-123"))
            .unwrap();
        let got = store.get(SecretKind::Pat).unwrap().unwrap();
        assert_eq!(got.expose_secret(), "tok-123");

        store.delete(SecretKind::Pat).unwrap();
        assert!(store.get(SecretKind::Pat).unwrap().is_none());
        // deleting again is not an error
        store.delete(SecretKind::Pat).unwrap();
    }

    #[test]
    fn kinds_are_separate_entries() {
        let store = mock_store();
        store.set(SecretKind::Pat, SecretString::from("a")).unwrap();
        store
            .set(SecretKind::CacheKey, SecretString::from("b"))
            .unwrap();
        assert_eq!(
            store.get(SecretKind::Pat).unwrap().unwrap().expose_secret(),
            "a"
        );
        assert_eq!(
            store
                .get(SecretKind::CacheKey)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "b"
        );
    }
}
