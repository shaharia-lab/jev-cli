//! SIGINT and SIGTERM for a command that runs for long: the first asks it to stop cleanly, and a
//! second ends the process at once with exit code 130.

use std::future::Future;
use std::io;

use crate::exit::Exit;

/// Resolves at the first SIGINT or SIGTERM (Ctrl-C on Windows). At the second, `on_second` runs
/// and the process exits with 130 straight away.
///
/// It must be called inside a Tokio runtime, and listens for as long as that runtime runs. When
/// signals cannot be caught, it never resolves, and a signal ends the process the usual way.
pub(crate) fn first_of_two(
    on_second: impl FnOnce() + Send + 'static,
) -> impl Future<Output = ()> + Send {
    let (first, heard) = tokio::sync::oneshot::channel::<()>();
    match Signals::listen() {
        Ok(mut signals) => {
            tokio::spawn(async move {
                signals.recv().await;
                let _ = first.send(());
                signals.recv().await;
                on_second();
                std::process::exit(i32::from(Exit::Interrupted.code()));
            });
        }
        Err(error) => {
            tracing::debug!(%error, "signals cannot be caught; an interrupt ends the process at once");
            drop(first);
        }
    }
    async move {
        if heard.await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(unix)]
struct Signals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Signals {
    fn listen() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};

        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    async fn recv(&mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
    }
}

#[cfg(windows)]
struct Signals {
    ctrl_c: tokio::signal::windows::CtrlC,
}

#[cfg(windows)]
impl Signals {
    fn listen() -> io::Result<Self> {
        Ok(Self {
            ctrl_c: tokio::signal::windows::ctrl_c()?,
        })
    }

    async fn recv(&mut self) {
        self.ctrl_c.recv().await;
    }
}
