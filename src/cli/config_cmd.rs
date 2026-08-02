use crate::config::Config;
use crate::error::Result;

pub fn get(key: &str) -> Result<()> {
    let cfg = Config::load()?;
    match cfg.get_key(key)? {
        Some(value) => println!("{value}"),
        None => println!("<unset>"),
    }
    Ok(())
}

pub fn set(key: &str, value: &str) -> Result<()> {
    let cfg = Config::load()?.with_key(key, value)?;
    cfg.save()?;
    println!("{key} = {value}");
    Ok(())
}
