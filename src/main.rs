mod args;
mod hooks;
mod pty;
mod runner;
mod transcript;
mod trust;
mod util;

use anyhow::{bail, Result};
use std::env;
use std::path::Path;

fn main() {
    if let Err(error) = run() {
        eprintln!("praude: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args: Vec<String> = env::args().skip(1).collect();

    if args.first().is_some_and(|arg| arg == "__hook-write") {
        if args.len() != 2 {
            bail!("usage: praude __hook-write <path>");
        }
        return hooks::hook_write(Path::new(&args[1]));
    }

    if args.first().is_some_and(|arg| arg == "__hook-control") {
        if args.len() != 2 {
            bail!("usage: praude __hook-control <event>");
        }
        return hooks::hook_control(&args[1]);
    }

    let invocation = args::parse_invocation(std::mem::take(&mut args))?;
    runner::run(invocation)
}
