use crate::args::{Format, Invocation};
use crate::hooks::write_settings;
use crate::pty::PtyChild;
use crate::transcript::{aggregate, stream_transcript};
use crate::trust::accept_workspace_trust;
use crate::util::{env_u64, env_usize, read_json_file};
use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::time::Duration;
use tempfile::Builder;

const PSEUDO_PRINT_SYSTEM_PROMPT: &str = "You are running through a non-interactive print-mode compatibility wrapper. The user cannot provide more input during this turn. Never ask the user questions or wait for input. Interactive dialog tools such as AskUserQuestion, EnterPlanMode, and ExitPlanMode are unavailable. Do not request confirmation, do not offer choices, and do not pause for clarification. Make reasonable assumptions and proceed. When finished, provide the final answer normally.";

pub fn run(invocation: Invocation) -> Result<()> {
    if env::var("PRAUDE_TRUST_CWD")
        .or_else(|_| env::var("CLAUDEP_TRUST_CWD"))
        .unwrap_or_else(|_| "1".to_string())
        != "0"
    {
        accept_workspace_trust()?;
    }

    let cwd = env::current_dir().context("reading current directory")?;
    let temp = Builder::new()
        .prefix(".praude.")
        .tempdir_in(&cwd)
        .context("creating temporary work directory")?;
    let workdir = temp.path().to_path_buf();
    let logfile = workdir.join("pty.log");
    let start_signal = workdir.join("start-hook.json");
    let stop_signal = workdir.join("stop-hook.json");
    let settings = workdir.join("settings.json");
    let prompt_file = workdir.join("prompt.txt");

    write_settings(&settings, &start_signal, &stop_signal, &invocation.settings)?;

    let arg_max = env_usize("PRAUDE_ARG_MAX_CHARS")
        .or_else(|| env_usize("CLAUDEP_ARG_MAX_CHARS"))
        .unwrap_or(100_000);
    let initial_prompt = if invocation.prompt.len() < arg_max {
        invocation.prompt.clone()
    } else {
        fs::write(&prompt_file, &invocation.prompt).context("writing prompt file")?;
        let rel = prompt_file
            .strip_prefix(&cwd)
            .unwrap_or(&prompt_file)
            .to_string_lossy();
        format!("The complete user request is in @{rel}. Read that file and complete the request. Do not ask questions or wait for input.")
    };

    let timeout = Duration::from_secs(
        env_u64("PRAUDE_TIMEOUT")
            .or_else(|| env_u64("CLAUDEP_TIMEOUT"))
            .unwrap_or(600),
    );
    let show_tui = env::var("PRAUDE_SHOW_TUI")
        .or_else(|_| env::var("CLAUDEP_SHOW_TUI"))
        .unwrap_or_default()
        == "1";

    let upstream_args = build_upstream_args(&settings, initial_prompt, &invocation)?;
    let mut child = PtyChild::spawn(upstream_args, &cwd, &logfile, show_tui)?;

    let status = if invocation.format == Format::StreamJson {
        let stream_result = stream_transcript(&start_signal, &stop_signal, timeout);
        let wait_result = child.wait_for_stop(&stop_signal, timeout);
        stream_result?;
        wait_result?
    } else {
        child.wait_for_stop(&stop_signal, timeout)?
    };

    if status != 0 {
        bail!("upstream CLI exited before Stop hook produced a signal");
    }

    child.join_reader();

    match invocation.format {
        Format::Text => {
            let stop_payload = read_json_file(&stop_signal)?;
            let result = aggregate(stop_payload)?;
            let text = result
                .get("result")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim_end_matches('\n');
            println!("{text}");
        }
        Format::Json => {
            let stop_payload = read_json_file(&stop_signal)?;
            let result = aggregate(stop_payload)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Format::StreamJson => {}
    }

    if env::var("PRAUDE_KEEP_LOG")
        .or_else(|_| env::var("CLAUDEP_KEEP_LOG"))
        .unwrap_or_default()
        == "1"
    {
        let kept = temp.keep();
        eprintln!("praude: kept temp dir at {}", kept.display());
    }

    Ok(())
}

fn build_upstream_args(
    settings: &std::path::Path,
    initial_prompt: String,
    invocation: &Invocation,
) -> Result<Vec<String>> {
    let mut args = vec![
        "--settings".to_string(),
        settings.to_string_lossy().into_owned(),
        "--dangerously-skip-permissions".to_string(),
    ];

    let deny_tools = env::var("PRAUDE_DENY_TOOLS")
        .or_else(|_| env::var("CLAUDEP_DENY_TOOLS"))
        .unwrap_or_else(|_| "AskUserQuestion EnterPlanMode ExitPlanMode".to_string());
    let deny: Vec<_> = deny_tools
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|tool| !tool.is_empty())
        .map(str::to_string)
        .collect();
    if !deny.is_empty() {
        args.push("--disallowedTools".to_string());
        args.extend(deny);
    }

    args.push("--append-system-prompt".to_string());
    args.push(build_append_system_prompt(&invocation));
    args.extend(invocation.upstream_args.iter().cloned());
    args.push("--".to_string());
    args.push(initial_prompt);
    Ok(args)
}

fn build_append_system_prompt(invocation: &Invocation) -> String {
    let mut wrapper_system_prompt = PSEUDO_PRINT_SYSTEM_PROMPT.to_string();
    if let Some(schema) = &invocation.json_schema {
        wrapper_system_prompt.push_str("\n\nThe caller provided this JSON Schema for the final answer. Return the final answer as JSON matching this schema, with no surrounding prose unless the user explicitly asks for prose:\n");
        wrapper_system_prompt.push_str(schema);
    }
    match &invocation.append_system_prompt {
        Some(user_prompt) => format!("{user_prompt}\n\n{wrapper_system_prompt}"),
        None => wrapper_system_prompt,
    }
}
