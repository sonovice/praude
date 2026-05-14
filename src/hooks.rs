use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub fn write_settings(
    settings: &Path,
    start_signal: &Path,
    stop_signal: &Path,
    user_settings: &[String],
) -> Result<()> {
    let exe = env::current_exe().context("locating current executable")?;
    let hook_start = hook_invocation(&exe, &["__hook-write", &start_signal.to_string_lossy()]);
    let hook_stop = hook_invocation(&exe, &["__hook-write", &stop_signal.to_string_lossy()]);
    let permission = hook_invocation(&exe, &["__hook-control", "PermissionRequest"]);
    let elicitation = hook_invocation(&exe, &["__hook-control", "Elicitation"]);
    let wrapper_json = json!({
        "hooks": {
            "UserPromptSubmit": [{ "hooks": [hook_command(&hook_start)] }],
            "Stop": [{ "hooks": [hook_command(&hook_stop)] }],
            "StopFailure": [{ "hooks": [hook_command(&hook_stop)] }],
            "PermissionRequest": [{ "hooks": [hook_command(&permission)] }],
            "Elicitation": [{ "hooks": [hook_command(&elicitation)] }]
        }
    });

    let mut settings_json = wrapper_json.clone();
    for setting in user_settings {
        let mut user_json = read_settings_value(setting)?;
        merge_wrapper_hooks(&mut user_json, &wrapper_json)?;
        deep_merge(&mut settings_json, user_json);
    }
    fs::write(settings, serde_json::to_vec(&settings_json)?).context("writing settings file")
}

pub fn hook_write(path: &Path) -> Result<()> {
    let mut payload = String::new();
    io::stdin()
        .read_to_string(&mut payload)
        .context("reading hook payload")?;
    let value = serde_json::from_str::<Value>(payload.trim()).unwrap_or_else(|error| {
        json!({
            "hook_parse_error": error.to_string(),
            "raw_stdin": payload
        })
    });
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &tmp,
        format!("{}\n", serde_json::to_string(&value).unwrap()),
    )
    .context("writing hook payload")?;
    fs::rename(&tmp, path).context("publishing hook payload")
}

pub fn hook_control(event: &str) -> Result<()> {
    let mut payload = String::new();
    let _ = io::stdin().read_to_string(&mut payload);
    if !payload.trim().is_empty() {
        let _ = serde_json::from_str::<Value>(payload.trim());
    }

    let response = match event {
        "PermissionRequest" => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow" }
            }
        }),
        "Elicitation" => json!({
            "reason": "MCP elicitation requires interactive input; praude declined it to stay unattended.",
            "hookSpecificOutput": {
                "hookEventName": "Elicitation",
                "action": "decline"
            }
        }),
        _ => json!({}),
    };
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

fn read_settings_value(value: &str) -> Result<Value> {
    let path = Path::new(value);
    let text = if path.exists() {
        fs::read_to_string(path).with_context(|| format!("reading settings {}", path.display()))?
    } else {
        value.to_string()
    };
    serde_json::from_str(&text).with_context(|| format!("parsing settings value {value}"))
}

fn merge_wrapper_hooks(user_json: &mut Value, wrapper_json: &Value) -> Result<()> {
    let user_hooks = user_json
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings root must be an object"))?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let user_hooks = user_hooks
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings hooks field must be an object"))?;
    let wrapper_hooks = wrapper_json
        .get("hooks")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("wrapper hooks are malformed"))?;

    for (event, matchers) in wrapper_hooks {
        let target = user_hooks.entry(event.clone()).or_insert_with(|| json!([]));
        let target = target
            .as_array_mut()
            .ok_or_else(|| anyhow!("settings hooks.{event} must be an array"))?;
        if let Some(matchers) = matchers.as_array() {
            target.extend(matchers.iter().cloned());
        }
    }
    Ok(())
}

fn deep_merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(target), Value::Object(source)) => {
            for (key, value) in source {
                match target.get_mut(&key) {
                    Some(target_value) => deep_merge(target_value, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, source) => {
            *target = source;
        }
    }
}

fn hook_command(command: &str) -> Value {
    if cfg!(windows) {
        json!({ "type": "command", "command": command, "timeout": 10, "shell": "powershell" })
    } else {
        json!({ "type": "command", "command": command, "timeout": 10 })
    }
}

fn hook_invocation(program: &Path, args: &[&str]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(shell_quote(&program.to_string_lossy()));
    parts.extend(args.iter().map(|arg| shell_quote(arg)));
    if cfg!(windows) {
        format!("& {}", parts.join(" "))
    } else {
        parts.join(" ")
    }
}

fn shell_quote(value: &str) -> String {
    if cfg!(windows) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
