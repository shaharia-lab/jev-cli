//! Running a program with one output stream on a pseudo-terminal, and reading all of it.
//!
//! Reading the terminal side until EOF races the program's exit on macOS: a session leader that
//! exits revokes its controlling terminal, and the last close of the other side flushes whatever
//! was not read yet, so a fast program's output can come back empty. So the program here gets the
//! terminal as a plain file descriptor (no new session, no controlling terminal), the test keeps
//! its own descriptor for that side open, and once the program has exited it writes an end marker
//! through it. Everything before the marker is the program's output, in order, on every Unix.
// Clippy's test allowances cover `#[test]` functions only, not the helpers they share.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pty_process::Size;
use pty_process::blocking::open;

/// Never printed by `jev`, and untouched by terminal output processing (no newline).
const END: &str = "\u{1}end-of-pty-capture\u{1}";

/// How long a test waits for the program, and then for its output, before failing.
const PATIENCE: Duration = Duration::from_secs(60);

/// The output stream that goes to the pseudo-terminal.
#[derive(Clone, Copy)]
#[allow(
    dead_code,
    reason = "each test binary puts only one of them on a terminal"
)]
pub(crate) enum Stream {
    Stdout,
    Stderr,
}

/// Runs `command` with `stream` on a 40×120 pseudo-terminal and returns its exit status and
/// everything it wrote there, with the terminal's "\r\n" left as it is.
///
/// The caller decides the other two streams; stdin should be null.
pub(crate) fn run(mut command: Command, stream: Stream) -> (ExitStatus, String) {
    let (mut pty, pts) = open().unwrap();
    pty.resize(Size::new(40, 120)).unwrap();
    let terminal = Stdio::from(pts.as_fd().try_clone_to_owned().unwrap());
    match stream {
        Stream::Stdout => command.stdout(terminal),
        Stream::Stderr => command.stderr(terminal),
    };
    let mut child = command.spawn().unwrap();
    drop(command);
    // Dropped only after the reader has seen the marker, so nothing is ever flushed unread.
    let mut kept = std::fs::File::from(pts.as_fd().try_clone_to_owned().unwrap());
    drop(pts);

    // Read while the program runs, so that it never blocks on a full terminal buffer.
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut seen = Vec::new();
        let mut buffer = [0_u8; 4096];
        let result = loop {
            match pty.read(&mut buffer) {
                Ok(0) => break Err("the terminal closed before the end marker".to_owned()),
                Ok(read) => seen.extend(buffer.iter().take(read)),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => break Err(format!("reading the terminal failed: {error}")),
            }
            if seen.ends_with(END.as_bytes()) {
                seen.truncate(seen.len() - END.len());
                break Ok(seen);
            }
        };
        let _ = sender.send(result);
    });

    let deadline = Instant::now() + PATIENCE;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the program did not exit within {PATIENCE:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    kept.write_all(END.as_bytes()).unwrap();
    let seen = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .unwrap_or_else(|_| panic!("the output did not end within {PATIENCE:?}"))
        .unwrap();
    drop(kept);
    (status, String::from_utf8(seen).unwrap())
}
