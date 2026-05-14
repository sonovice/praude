use anyhow::{bail, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

pub struct PtyChild {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    read_handle: Option<thread::JoinHandle<()>>,
}

impl PtyChild {
    pub fn spawn(args: Vec<String>, cwd: &Path, logfile: &Path, show_tui: bool) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 50,
                cols: 240,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("opening PTY")?;

        let mut cmd = CommandBuilder::new("claude");
        cmd.cwd(cwd);
        for arg in args {
            cmd.arg(arg);
        }
        let child = pair.slave.spawn_command(cmd).context("spawning claude")?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("opening PTY reader")?;
        let mut log = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(logfile)
            .context("opening PTY log")?;
        let read_handle = thread::spawn(move || {
            let mut buf = [0_u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = log.write_all(&buf[..n]);
                        if show_tui {
                            let _ = io::stderr().write_all(&buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            read_handle: Some(read_handle),
        })
    }

    pub fn wait_for_stop(&mut self, stop_signal: &Path, timeout: Duration) -> Result<i32> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if stop_signal.exists() {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Ok(0);
            }
            if let Some(status) = self.child.try_wait().context("waiting for upstream CLI")? {
                if stop_signal.exists() {
                    return Ok(0);
                }
                return Ok(status.exit_code() as i32);
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        bail!("timed out waiting for Stop hook");
    }

    pub fn join_reader(&mut self) {
        if let Some(handle) = self.read_handle.take() {
            let _ = handle.join();
        }
    }
}
