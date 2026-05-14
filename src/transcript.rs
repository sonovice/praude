use crate::util::{emit_json, read_json_file};
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub fn stream_transcript(start_path: &Path, stop_path: &Path, timeout: Duration) -> Result<()> {
    let signal_path = wait_for_file(start_path, Some(stop_path), timeout)?;
    let payload = read_json_file(&signal_path)?;
    let transcript = payload
        .get("transcript_path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("hook payload did not include transcript_path"))?;
    let transcript_path = PathBuf::from(transcript);
    emit_json(&build_init_message(&payload, &transcript_path), false)?;

    let mut offset = 0_u64;
    let mut partial = String::new();
    let mut seen = HashSet::new();
    loop {
        if transcript_path.exists() {
            let metadata = fs::metadata(&transcript_path)?;
            if metadata.len() < offset {
                offset = 0;
                partial.clear();
            }
            if metadata.len() > offset {
                let mut file = File::open(&transcript_path)?;
                file.seek(std::io::SeekFrom::Start(offset))?;
                let mut chunk = String::new();
                file.read_to_string(&mut chunk)?;
                offset = metadata.len();
                let combined = format!("{partial}{chunk}");
                let mut lines: Vec<&str> = combined.split('\n').collect();
                partial = lines.pop().unwrap_or_default().to_string();
                for line in lines {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(item) = serde_json::from_str::<Value>(line.trim_end_matches('\r')) {
                        if let Some(message) = to_sdk_message(&item) {
                            let key = json!([
                                message.get("type"),
                                message.get("uuid"),
                                message.get("message").and_then(|m| m.get("id"))
                            ])
                            .to_string();
                            if seen.insert(key) {
                                emit_json(&message, false)?;
                            }
                        }
                    }
                }
            }
        }

        if stop_path.exists() {
            thread::sleep(Duration::from_millis(200));
            if transcript_path.exists() && fs::metadata(&transcript_path)?.len() > offset {
                continue;
            }
            let stop_payload = read_json_file(stop_path)?;
            let result = aggregate(stop_payload)?;
            emit_json(&result, false)?;
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn aggregate(stop_payload: Value) -> Result<Value> {
    let transcript = stop_payload
        .get("transcript_path")
        .and_then(Value::as_str)
        .unwrap_or("");
    let transcript_path = Path::new(transcript);
    let mut assistant_text = Vec::new();
    let mut usage = empty_usage();
    let mut session_id = stop_payload
        .get("session_id")
        .cloned()
        .unwrap_or(Value::Null);
    let mut stop_reason = Value::Null;
    let mut first_timestamp: Option<String> = None;
    let mut last_timestamp: Option<String> = None;
    let mut num_turns = 1_i64;
    let mut seen_usage = HashSet::new();

    if transcript_path.exists() {
        let text = fs::read_to_string(transcript_path)?;
        for line in text.lines() {
            let Ok(item) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(id) = item.get("sessionId").or_else(|| item.get("session_id")) {
                session_id = id.clone();
            }
            if let Some(ts) = item.get("timestamp").and_then(Value::as_str) {
                if first_timestamp.is_none() {
                    first_timestamp = Some(ts.to_string());
                }
                last_timestamp = Some(ts.to_string());
            }
            if item.get("type").and_then(Value::as_str) == Some("assistant") {
                if let Some(message) = item.get("message") {
                    assistant_text.push(text_from_content(
                        message.get("content").unwrap_or(&Value::Null),
                    ));
                    if let Some(reason) = message.get("stop_reason") {
                        stop_reason = reason.clone();
                    }
                    let usage_key = message
                        .get("id")
                        .or_else(|| item.get("uuid"))
                        .map(Value::to_string)
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    if seen_usage.insert(usage_key) {
                        accumulate_usage(&mut usage, message.get("usage"));
                    }
                }
            } else if is_tool_result_user(&item) {
                num_turns += 1;
            }
        }
    }

    let duration_ms = match (first_timestamp, last_timestamp) {
        (Some(first), Some(last)) => {
            let first = DateTime::parse_from_rfc3339(&first).map(|v| v.with_timezone(&Utc));
            let last = DateTime::parse_from_rfc3339(&last).map(|v| v.with_timezone(&Utc));
            match (first, last) {
                (Ok(first), Ok(last)) => (last - first).num_milliseconds().max(0),
                _ => 0,
            }
        }
        _ => 0,
    };

    let result_text = stop_payload
        .get("last_assistant_message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| assistant_text.join(""));

    Ok(json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "api_error_status": Value::Null,
        "duration_ms": duration_ms,
        "duration_api_ms": duration_ms,
        "num_turns": num_turns,
        "result": result_text,
        "stop_reason": stop_reason,
        "session_id": session_id,
        "total_cost_usd": 0,
        "usage": usage,
        "modelUsage": {},
        "permission_denials": [],
        "terminal_reason": "completed",
        "fast_mode_state": "off",
        "uuid": Uuid::new_v4().to_string(),
    }))
}

fn wait_for_file(path: &Path, fallback: Option<&Path>, timeout: Duration) -> Result<PathBuf> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        if let Some(fallback) = fallback {
            if fallback.exists() {
                return Ok(fallback.to_path_buf());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("timed out waiting for {}", path.display())
}

fn to_sdk_message(item: &Value) -> Option<Value> {
    let session_id = item
        .get("sessionId")
        .or_else(|| item.get("session_id"))
        .cloned();
    if item.get("type").and_then(Value::as_str) == Some("assistant") {
        let message = item.get("message")?;
        let mut message = message.clone();
        if let Some(map) = message.as_object_mut() {
            map.insert("stop_reason".to_string(), Value::Null);
        }
        return Some(json!({
            "type": "assistant",
            "message": message,
            "parent_tool_use_id": Value::Null,
            "session_id": session_id,
            "uuid": item.get("uuid").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| Uuid::new_v4().to_string()),
        }));
    }

    if is_tool_result_user(item) {
        let mut out = Map::new();
        out.insert("type".to_string(), json!("user"));
        out.insert("message".to_string(), item.get("message")?.clone());
        out.insert("parent_tool_use_id".to_string(), Value::Null);
        out.insert("session_id".to_string(), session_id.unwrap_or(Value::Null));
        out.insert(
            "uuid".to_string(),
            item.get("uuid")
                .and_then(Value::as_str)
                .map(|s| json!(s))
                .unwrap_or_else(|| json!(Uuid::new_v4().to_string())),
        );
        if let Some(timestamp) = item.get("timestamp") {
            out.insert("timestamp".to_string(), timestamp.clone());
        }
        if let Some(tool_use_result) = item.get("toolUseResult") {
            out.insert("tool_use_result".to_string(), tool_use_result.clone());
        }
        return Some(Value::Object(out));
    }

    None
}

fn build_init_message(payload: &Value, transcript_path: &Path) -> Value {
    let mut version = "unknown".to_string();
    let mut tools = HashSet::new();
    if let Ok(text) = fs::read_to_string(transcript_path) {
        for line in text.lines() {
            let Ok(item) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(item_version) = item.get("version").and_then(Value::as_str) {
                version = item_version.to_string();
            }
            if let Some(names) = item
                .get("attachment")
                .and_then(|a| a.get("addedNames"))
                .and_then(Value::as_array)
            {
                for name in names.iter().filter_map(Value::as_str) {
                    tools.insert(if name == "Agent" { "Task" } else { name }.to_string());
                }
            }
            if item.get("type").and_then(Value::as_str) == Some("assistant") {
                if let Some(blocks) = item
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            if let Some(name) = block.get("name").and_then(Value::as_str) {
                                tools.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    let mut tools: Vec<_> = tools.into_iter().collect();
    tools.sort();
    json!({
        "type": "system",
        "subtype": "init",
        "cwd": payload.get("cwd").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()),
        "session_id": payload.get("session_id").cloned().unwrap_or(Value::Null),
        "tools": tools,
        "mcp_servers": [],
        "model": env::var("PRAUDE_MODEL").or_else(|_| env::var("CLAUDEP_MODEL")).unwrap_or_else(|_| "unknown".to_string()),
        "permissionMode": payload.get("permission_mode").and_then(Value::as_str).unwrap_or("bypassPermissions"),
        "slash_commands": [],
        "apiKeySource": "none",
        "claude_code_version": version,
        "output_style": "default",
        "agents": [],
        "skills": [],
        "plugins": [],
        "fast_mode_state": "off",
        "uuid": Uuid::new_v4().to_string(),
    })
}

fn empty_usage() -> Value {
    json!({
        "input_tokens": 0,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "output_tokens": 0,
        "server_tool_use": {
            "web_search_requests": 0,
            "web_fetch_requests": 0
        },
        "service_tier": "standard",
        "cache_creation": {
            "ephemeral_1h_input_tokens": 0,
            "ephemeral_5m_input_tokens": 0
        },
        "inference_geo": "",
        "iterations": []
    })
}

fn accumulate_usage(total: &mut Value, usage: Option<&Value>) {
    let Some(usage) = usage.and_then(Value::as_object) else {
        return;
    };
    for key in [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ] {
        add_number(total, key, usage.get(key));
    }
    if let Some(server_tool_use) = usage.get("server_tool_use").and_then(Value::as_object) {
        for (key, value) in server_tool_use {
            add_nested_number(total, "server_tool_use", key, Some(value));
        }
    }
    if let Some(cache_creation) = usage.get("cache_creation").and_then(Value::as_object) {
        for (key, value) in cache_creation {
            add_nested_number(total, "cache_creation", key, Some(value));
        }
    }
    for key in ["service_tier", "inference_geo", "speed"] {
        if let Some(value) = usage.get(key).and_then(Value::as_str) {
            total[key] = json!(value);
        }
    }
    if let Some(iterations) = usage.get("iterations").and_then(Value::as_array) {
        if let Some(target) = total.get_mut("iterations").and_then(Value::as_array_mut) {
            target.extend(iterations.iter().cloned());
        }
    }
}

fn add_number(total: &mut Value, key: &str, value: Option<&Value>) {
    if let Some(value) = value.and_then(Value::as_i64) {
        let current = total.get(key).and_then(Value::as_i64).unwrap_or(0);
        total[key] = json!(current + value);
    }
}

fn add_nested_number(total: &mut Value, parent: &str, key: &str, value: Option<&Value>) {
    if let Some(value) = value.and_then(Value::as_i64) {
        let current = total
            .get(parent)
            .and_then(|v| v.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        total[parent][key] = json!(current + value);
    }
}

fn text_from_content(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn is_tool_result_user(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("user")
        && item
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            })
}
