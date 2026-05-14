use crate::util::home_dir;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn accept_workspace_trust() -> Result<()> {
    let config_home = env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(home_dir)
        .ok_or_else(|| anyhow!("cannot determine home directory"))?;
    let legacy = config_home.join(".config.json");
    let config = if legacy.exists() {
        legacy
    } else {
        config_home.join(".claude.json")
    };

    let mut data = if config.exists() && config.metadata()?.len() > 0 {
        serde_json::from_slice::<Value>(&fs::read(&config)?)
            .with_context(|| format!("cannot parse upstream config {}", config.display()))?
    } else {
        json!({})
    };

    let key = project_key()?;
    let root = data
        .as_object_mut()
        .ok_or_else(|| anyhow!("upstream config root is not an object"))?;
    let projects = root
        .entry("projects")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("upstream config projects field is not an object"))?;
    let project = projects
        .entry(key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("upstream project config is not an object"))?;
    project.insert("hasTrustDialogAccepted".to_string(), json!(true));

    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = config.with_extension(format!("praude.{}.tmp", std::process::id()));
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(&data)?))?;
    fs::rename(tmp, config)?;
    Ok(())
}

fn project_key() -> Result<String> {
    let cwd = env::current_dir()?;
    let output = Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .stderr(Stdio::null())
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !root.is_empty() {
                return Ok(fs::canonicalize(root)?.to_string_lossy().into_owned());
            }
        }
    }
    Ok(fs::canonicalize(cwd)?.to_string_lossy().into_owned())
}
