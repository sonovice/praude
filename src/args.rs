use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    StreamJson,
}

impl Format {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "stream-json" => Ok(Self::StreamJson),
            _ => bail!("invalid output format '{value}' (expected text, json, or stream-json)"),
        }
    }
}

pub struct Invocation {
    pub upstream_args: Vec<String>,
    pub prompt: String,
    pub format: Format,
    pub append_system_prompt: Option<String>,
    pub settings: Vec<String>,
    pub json_schema: Option<String>,
}

pub fn parse_invocation(args: Vec<String>) -> Result<Invocation> {
    parse_invocation_with_stdin(args, read_stdin_prompt()?)
}

fn parse_invocation_with_stdin(
    args: Vec<String>,
    stdin_prompt: Option<String>,
) -> Result<Invocation> {
    let mut format = env::var("PRAUDE_FORMAT")
        .or_else(|_| env::var("CLAUDEP_FORMAT"))
        .ok()
        .map(|value| Format::parse(&value))
        .transpose()?
        .unwrap_or(Format::Text);

    let mut cleaned = Vec::new();
    let mut append_system_prompts = Vec::new();
    let mut settings = Vec::new();
    let mut json_schema = None;
    let mut iter = args.into_iter().peekable();

    while let Some(arg) = iter.next() {
        if arg == "--" {
            cleaned.push(arg);
            cleaned.extend(iter);
            break;
        }
        if arg == "-p" || arg == "--print" {
            continue;
        }
        if arg == "--output-format" {
            let value = iter
                .next()
                .ok_or_else(|| anyhow!("--output-format requires a value"))?;
            format = Format::parse(&value)?;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--output-format=") {
            format = Format::parse(value)?;
            continue;
        }
        if arg == "--settings" {
            let value = iter
                .next()
                .ok_or_else(|| anyhow!("--settings requires a value"))?;
            settings.push(value);
            continue;
        }
        if let Some(value) = arg.strip_prefix("--settings=") {
            settings.push(value.to_string());
            continue;
        }
        if arg == "--json-schema" {
            json_schema = Some(
                iter.next()
                    .ok_or_else(|| anyhow!("--json-schema requires a value"))?,
            );
            continue;
        }
        if let Some(value) = arg.strip_prefix("--json-schema=") {
            json_schema = Some(value.to_string());
            continue;
        }
        if arg == "--include-partial-messages" || arg == "--no-session-persistence" {
            continue;
        }
        if arg == "--input-format" {
            let value = iter
                .next()
                .ok_or_else(|| anyhow!("--input-format requires a value"))?;
            if value != "text" {
                bail!("--input-format={value} cannot be emulated without native batch mode");
            }
            continue;
        }
        if let Some(value) = arg.strip_prefix("--input-format=") {
            if value != "text" {
                bail!("--input-format={value} cannot be emulated without native batch mode");
            }
            continue;
        }
        if arg == "--append-system-prompt" {
            let value = iter
                .next()
                .ok_or_else(|| anyhow!("--append-system-prompt requires a value"))?;
            append_system_prompts.push(value);
            continue;
        }
        if let Some(value) = arg.strip_prefix("--append-system-prompt=") {
            append_system_prompts.push(value.to_string());
            continue;
        }
        if arg == "--append-system-prompt-file" {
            let value = iter
                .next()
                .ok_or_else(|| anyhow!("--append-system-prompt-file requires a value"))?;
            append_system_prompts.push(
                fs::read_to_string(&value)
                    .with_context(|| format!("reading append system prompt file {value}"))?,
            );
            continue;
        }
        if let Some(value) = arg.strip_prefix("--append-system-prompt-file=") {
            append_system_prompts.push(
                fs::read_to_string(value)
                    .with_context(|| format!("reading append system prompt file {value}"))?,
            );
            continue;
        }
        cleaned.push(arg);
    }

    let (upstream_args, prompt) = if let Some(input) = stdin_prompt {
        (cleaned, input)
    } else if let Some(separator) = cleaned.iter().position(|arg| arg == "--") {
        let prompt = cleaned[separator + 1..].join(" ");
        (cleaned[..separator].to_vec(), prompt)
    } else if !cleaned.is_empty() {
        let mut upstream_args = cleaned;
        let prompt = upstream_args.pop().unwrap();
        (upstream_args, prompt)
    } else {
        bail!("usage: praude [upstream options] \"prompt\"   or:   echo prompt | praude [upstream options]");
    };

    if prompt.trim().is_empty() {
        bail!("prompt is empty");
    }

    Ok(Invocation {
        upstream_args,
        prompt,
        format,
        append_system_prompt: if append_system_prompts.is_empty() {
            None
        } else {
            Some(append_system_prompts.join("\n\n"))
        },
        settings,
        json_schema,
    })
}

fn read_stdin_prompt() -> Result<Option<String>> {
    if io::stdin().is_terminal() {
        return Ok(None);
    }

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("reading prompt from stdin")?;
    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input))
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_invocation_with_stdin, Format};

    #[test]
    fn separator_stops_wrapper_option_parsing() {
        let invocation = parse_invocation_with_stdin(
            vec!["--".into(), "--output-format".into(), "json".into()],
            None,
        )
        .unwrap();

        assert_eq!(invocation.prompt, "--output-format json");
        assert!(matches!(invocation.format, Format::Text));
        assert!(invocation.upstream_args.is_empty());
    }

    #[test]
    fn separator_preserves_dash_p_as_prompt() {
        let invocation = parse_invocation_with_stdin(vec!["--".into(), "-p".into()], None).unwrap();

        assert_eq!(invocation.prompt, "-p");
        assert!(matches!(invocation.format, Format::Text));
        assert!(invocation.upstream_args.is_empty());
    }
}
