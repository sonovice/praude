# praude

`praude` is a batch-style wrapper around an installed interactive coding-assistant
CLI. It is intended for scripts, pipes, editor integrations, and other unattended
workflows that want print-style behavior while avoiding the upstream batch-mode
entrypoint and its native structured-output transport.

The binary starts the upstream program in a pseudo-terminal, injects temporary
hooks through a generated settings file, waits for the assistant turn to finish,
then reads the normal session transcript to produce text or JSON output.

## Goals

- Provide a `praude` binary for one-shot prompts.
- Accept prompt text either as the final argument or on stdin.
- Forward ordinary upstream CLI flags transparently.
- Strip print-mode flags before launching the upstream process.
- Implement `text`, `json`, and `stream-json` output in `praude` itself.
- Avoid using the upstream native streaming JSON output mode.
- Stay unattended by denying interactive question and plan tools.
- Merge caller-provided settings with the wrapper hooks instead of replacing
  either side.
- Work on macOS, Linux, and Windows where the upstream executable is available.

## Install

```sh
cargo install --path .
```

Or run from the repository:

```sh
cargo run -- "Summarize this repository"
```

## Basic Usage

Prompt as an argument:

```sh
praude "Explain the current project structure"
```

Prompt from stdin:

```sh
printf '%s\n' "Write a release note for the latest changes" | praude
```

Pass upstream flags before the prompt:

```sh
praude --model some-model-name "Return exactly OK"
```

Use `--` when the prompt itself starts with a dash:

```sh
praude -- --explain this literal leading dash
```

## Output Modes

The default output is plain text:

```sh
praude "Return exactly OK"
```

Single JSON result:

```sh
praude --output-format json "Return exactly OK"
```

Line-delimited streaming JSON:

```sh
praude --output-format stream-json "Return exactly OK"
```

These modes are implemented by `praude`; the matching upstream output flag is
not forwarded.

You can also set the default with:

```sh
PRAUDE_FORMAT=json praude "Return exactly OK"
```

Accepted values are `text`, `json`, and `stream-json`.

## Flag Handling

`praude` removes or consumes a small set of flags so it can emulate batch-mode
behavior itself:

- `-p`, `--print`: stripped.
- `--output-format`: consumed by `praude`.
- `--settings`: loaded, merged with wrapper settings, then replaced with a
  temporary generated settings file.
- `--append-system-prompt`: loaded and combined with the wrapper’s unattended
  execution instruction.
- `--append-system-prompt-file`: read and combined the same way.
- `--json-schema`: converted into an instruction appended to the system prompt.
- `--input-format text`: accepted and stripped.
- `--input-format stream-json`: rejected because it depends on native batch
  mode.
- `--include-partial-messages`, `--no-session-persistence`: stripped because
  they are native batch-mode controls.

All other arguments are forwarded unchanged.

## Environment

Primary environment variables:

- `PRAUDE_FORMAT`: `text`, `json`, or `stream-json`.
- `PRAUDE_TIMEOUT`: seconds to wait for completion, default `600`.
- `PRAUDE_SHOW_TUI`: set to `1` to mirror the hidden terminal UI to stderr.
- `PRAUDE_KEEP_LOG`: set to `1` to keep the temporary directory after exit.
- `PRAUDE_DENY_TOOLS`: space- or comma-separated tool names to deny.
- `PRAUDE_TRUST_CWD`: set to `0` to avoid pre-accepting workspace trust.
- `PRAUDE_ARG_MAX_CHARS`: prompt length threshold before using a temp file.
- `PRAUDE_MODEL`: model name to report in synthetic init messages when known.

For migration from the old shell wrapper, matching legacy environment aliases
are still accepted internally, but new scripts should use the `PRAUDE_*` names.

## Settings Merge

The wrapper must install temporary hooks to know when a turn starts and ends.
When you pass `--settings`, `praude` parses the file or JSON string, appends its
own hook entries, and writes a temporary merged settings file for the upstream
process.

This means user settings still apply, while the wrapper retains the control
signals it needs for reliable completion detection.

## Unattended Behavior

`praude` tries to keep the upstream session from waiting on keyboard input:

- tool permissions are bypassed for the spawned process;
- permission hook requests are allowed;
- elicitation requests are declined;
- interactive question and plan tools are denied by default;
- an extra system instruction tells the assistant to make reasonable assumptions
  and finish normally.

The default denied tools are:

```text
AskUserQuestion EnterPlanMode ExitPlanMode
```

Override them with:

```sh
PRAUDE_DENY_TOOLS="AskUserQuestion,EnterPlanMode,ExitPlanMode,SomeOtherTool" praude "..."
```

## Trust Handling

By default, `praude` marks the current project as trusted in the upstream config
before starting the hidden terminal session. This mirrors the expectations of
unattended batch use, where an interactive trust dialog would otherwise block.

Disable this with:

```sh
PRAUDE_TRUST_CWD=0 praude "..."
```

## Temporary Files

Each run creates a temporary `.praude.*` directory in the current working
directory. It contains:

- generated settings;
- hook signal files;
- the raw PTY log;
- a prompt file when the prompt is too large for argv.

The directory is removed on success or failure unless `PRAUDE_KEEP_LOG=1` is set.

## Platform Notes

`praude` uses a pseudo-terminal library instead of shell automation. The spawned
upstream executable is resolved from `PATH`.

Hook commands are emitted with platform-specific shell rules:

- POSIX platforms use the default shell hook behavior.
- Windows hook commands explicitly request PowerShell so native paths are handled
  correctly.

## Limitations

`praude` does not flip the upstream program’s private internal batch-mode flag.
That flag affects some behavior inside the upstream process, so this wrapper
approximates the user-visible pieces from outside: prompt shaping, hook control,
tool denial, transcript parsing, and output formatting.

Features that depend on the native batch-mode input loop are intentionally not
implemented, especially streaming input.

Structured output via `--json-schema` is prompt-enforced rather than backed by
the upstream process’s native synthetic tool.

## Development

Check and format:

```sh
cargo fmt
cargo check
```

Optional Linux target check:

```sh
cargo check --target x86_64-unknown-linux-gnu
```

Quick smoke test:

```sh
PRAUDE_TIMEOUT=120 cargo run --quiet -- --model some-model-name "Reply with exactly OK"
```
