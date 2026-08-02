use keyring::Entry;
use secrecy::{ExposeSecret, SecretString};

use crate::error::Result;

const SERVICE: &str = "ynab-cli";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    Pat,
    OauthClientId,
    OauthClientSecret,
    OauthAccessToken,
    OauthRefreshToken,
    CacheKey,
}

impl SecretKind {
    fn entry_name(self) -> &'static str {
        match self {
            SecretKind::Pat => "pat",
            SecretKind::OauthClientId => "oauth-client-id",
            SecretKind::OauthClientSecret => "oauth-client-secret",
            SecretKind::OauthAccessToken => "oauth-access-token",
            SecretKind::OauthRefreshToken => "oauth-refresh-token",
            SecretKind::CacheKey => "cache-key",
        }
    }
}

/// Holds one keyring Entry per SecretKind, created eagerly. Entries are
/// reused (not recreated per call) because keyring's mock store — used in
/// tests — keeps credential state per-Entry instance.
pub struct SecretStore {
    entries: [Entry; 6],
}

impl SecretStore {
    pub fn new() -> Result<Self> {
        // Array order MUST match SecretKind discriminant order (`as usize`).
        let mk = |kind: SecretKind| Entry::new(SERVICE, kind.entry_name());
        Ok(SecretStore {
            entries: [
                mk(SecretKind::Pat)?,
                mk(SecretKind::OauthClientId)?,
                mk(SecretKind::OauthClientSecret)?,
                mk(SecretKind::OauthAccessToken)?,
                mk(SecretKind::OauthRefreshToken)?,
                mk(SecretKind::CacheKey)?,
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

        store.set(SecretKind::Pat, SecretString::from("tok-123")).unwrap();
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
        store.set(SecretKind::CacheKey, SecretString::from("b")).unwrap();
        assert_eq!(store.get(SecretKind::Pat).unwrap().unwrap().expose_secret(), "a");
        assert_eq!(store.get(SecretKind::CacheKey).unwrap().unwrap().expose_secret(), "b");
    }
}
