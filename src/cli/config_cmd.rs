use crate::config::Config;
use crate::error::Result;

pub fn get(key: &str) -> Result<()> {
    let cfg = Config::load()?;
    match cfg.get_key(key)? {
        Some(value) => crate::output::print_line(&value)?,
        None => crate::output::print_line("<unset>")?,
    }
    Ok(())
}

pub fn set(key: &str, value: &str) -> Result<()> {
    let cfg = Config::load()?.with_key(key, value)?;
    cfg.save()?;
    crate::output::print_line(&format!("{key} = {value}"))?;
    Ok(())
}
