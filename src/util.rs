use anyhow::{Context, Result};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn read_json_file(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("parsing {}", path.display()))
}

pub fn emit_json(value: &Value, pretty: bool) -> Result<()> {
    if pretty {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", serde_json::to_string(value)?);
    }
    io::stdout().flush()?;
    Ok(())
}

pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }
    if cfg!(windows) {
        if let Some(profile) = env::var_os("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        match (env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH")) {
            (Some(drive), Some(path)) => {
                let mut home = PathBuf::from(drive);
                home.push(path);
                Some(home)
            }
            _ => None,
        }
    } else {
        None
    }
}

pub fn env_u64(name: &str) -> Option<u64> {
    env::var(name).ok()?.parse().ok()
}

pub fn env_usize(name: &str) -> Option<usize> {
    env::var(name).ok()?.parse().ok()
}
