//! Window processes.

use std::collections::BTreeMap;
use std::io;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use fusion_hub::Ended;
use fusion_layout::WindowId;

/// Starts and watches the window processes.
pub trait WindowProcesses {
    /// Starts the process of `window`.
    fn start(&mut self, window: WindowId) -> io::Result<()>;
    /// The processes that ended since the last call, and how.
    fn ended(&mut self) -> Vec<(WindowId, Ended)>;
    /// Waits up to `grace` for every process to end, then kills the rest.
    /// Returns the windows it had to kill.
    fn stop_all(&mut self, grace: Duration) -> Vec<WindowId>;
}

/// Window processes as child processes of the launcher. `command` makes the
/// command line for a window, for example
/// `fusion-window --window 2 --hub fusion-hub-123.sock`.
pub struct ChildProcesses<F> {
    command: F,
    children: BTreeMap<WindowId, Child>,
}

impl<F: FnMut(WindowId) -> Command> ChildProcesses<F> {
    /// No processes yet.
    pub fn new(command: F) -> Self {
        Self {
            command,
            children: BTreeMap::new(),
        }
    }

    /// The number of processes that run.
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// True if no process runs.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

/// How often [`WindowProcesses::stop_all`] looks whether the windows ended.
const STOP_POLL: Duration = Duration::from_millis(10);

/// How a process ended, from its exit status.
fn how(status: io::Result<Option<std::process::ExitStatus>>) -> Option<Ended> {
    match status {
        Ok(None) => None,
        Ok(Some(status)) if status.success() => Some(Ended::Clean),
        Ok(Some(_)) | Err(_) => Some(Ended::Crashed),
    }
}

impl<F: FnMut(WindowId) -> Command> WindowProcesses for ChildProcesses<F> {
    fn start(&mut self, window: WindowId) -> io::Result<()> {
        let child = (self.command)(window).spawn()?;
        if let Some(mut old) = self.children.insert(window, child) {
            // Only one process per window; the supervisor never asks for two.
            let _killed = old.kill();
        }
        Ok(())
    }

    fn ended(&mut self) -> Vec<(WindowId, Ended)> {
        let mut ended = Vec::new();
        self.children
            .retain(|window, child| match how(child.try_wait()) {
                None => true,
                Some(how) => {
                    ended.push((*window, how));
                    false
                }
            });
        ended
    }

    fn stop_all(&mut self, grace: Duration) -> Vec<WindowId> {
        // Whole rounds instead of a clock, so the waiting is exact to test.
        let rounds = grace.as_millis() / STOP_POLL.as_millis();
        for _ in 0..rounds {
            self.children
                .retain(|_, child| how(child.try_wait()).is_none());
            if self.children.is_empty() {
                break;
            }
            thread::sleep(STOP_POLL);
        }
        for child in self.children.values_mut() {
            // A process that already ended cannot be killed; both are fine.
            let _killed = child.kill();
            let _reaped = child.wait();
        }
        std::mem::take(&mut self.children).into_keys().collect()
    }
}
