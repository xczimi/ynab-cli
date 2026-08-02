use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const VALID_KEYS: &[&str] = &["cache", "default_budget"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_budget: Option<String>,
}

impl Config {
    pub fn dir() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("YNAB_CLI_CONFIG_DIR") {
            return Ok(PathBuf::from(dir));
        }
        directories::ProjectDirs::from("", "", "ynab-cli")
            .map(|d| d.config_dir().to_path_buf())
            .ok_or_else(|| Error::Config("cannot determine config directory".into()))
    }

    fn file_path() -> Result<PathBuf> {
        Ok(Self::dir()?.join("config.toml"))
    }

    pub fn load() -> Result<Config> {
        Self::load_from(&Self::file_path()?)
    }

    pub fn load_from(path: &Path) -> Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|e| Error::Config(format!("invalid config file: {e}")))
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::file_path()?)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("cannot serialize config: {e}")))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn cache_enabled(&self) -> bool {
        self.cache.unwrap_or(true)
    }

    pub fn get_key(&self, key: &str) -> Result<Option<String>> {
        match key {
            "cache" => Ok(self.cache.map(|b| b.to_string())),
            "default_budget" => Ok(self.default_budget.clone()),
            _ => Err(Error::Config(format!(
                "unknown key: {key} (valid keys: {})",
                VALID_KEYS.join(", ")
            ))),
        }
    }

    pub fn with_key(self, key: &str, value: &str) -> Result<Config> {
        match key {
            "cache" => {
                let parsed: bool = value
                    .parse()
                    .map_err(|_| Error::Config("cache must be true or false".into()))?;
                Ok(Config { cache: Some(parsed), ..self })
            }
            "default_budget" => Ok(Config { default_budget: Some(value.to_string()), ..self }),
            _ => Err(Error::Config(format!(
                "unknown key: {key} (valid keys: {})",
                VALID_KEYS.join(", ")
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing() {
        let cfg = Config::default();
        assert!(cfg.cache_enabled());
        assert!(cfg.default_budget.is_none());
    }

    #[test]
    fn toml_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config { cache: Some(false), default_budget: Some("b-1".into()) };
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.cache, Some(false));
        assert_eq!(loaded.default_budget.as_deref(), Some("b-1"));
    }

    #[test]
    fn load_from_missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Config::load_from(&dir.path().join("nope.toml")).unwrap();
        assert!(loaded.cache_enabled());
    }

    #[test]
    fn key_access() {
        let cfg = Config::default();
        assert_eq!(cfg.get_key("cache").unwrap(), None);
        let cfg = cfg.with_key("cache", "false").unwrap();
        assert_eq!(cfg.get_key("cache").unwrap().as_deref(), Some("false"));
        let cfg = cfg.with_key("default_budget", "b-9").unwrap();
        assert_eq!(cfg.get_key("default_budget").unwrap().as_deref(), Some("b-9"));

        assert!(cfg.get_key("nope").is_err());
        assert!(cfg.clone().with_key("cache", "maybe").is_err());
    }
}
