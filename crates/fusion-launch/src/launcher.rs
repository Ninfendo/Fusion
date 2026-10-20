//! The launcher loop.

use std::time::{Duration, Instant};

use fusion_hub::{Address, AfterExit, Ended, FromHub, Hub, Reply, Supervisor, Switchboard, ToHub};
use fusion_layout::{Layout, WindowId};
use fusion_pipe::{HubServer, PipeEvent};

use crate::file::LayoutFile;
use crate::processes::WindowProcesses;

/// How long after a change the layout is saved, so a drag saves once a
/// second at most, not on every frame.
pub const SAVE_DELAY: Duration = Duration::from_secs(1);

/// The launcher: the hub, the pipes, the window processes and the layout
/// file.
pub struct Launcher<P> {
    hub: Hub,
    board: Switchboard,
    supervisor: Supervisor,
    server: HubServer,
    processes: P,
    file: LayoutFile,
    started: Instant,
    /// When the layout first changed since it was last saved.
    changed_at: Option<Instant>,
    log: Vec<String>,
    quit: bool,
}

impl<P: WindowProcesses> Launcher<P> {
    /// A launcher for `layout`. Nothing runs until [`Launcher::start`].
    pub fn new(layout: Layout, server: HubServer, processes: P, file: LayoutFile) -> Self {
        Self {
            hub: Hub::new(layout),
            board: Switchboard::default(),
            supervisor: Supervisor::default(),
            server,
            processes,
            file,
            started: Instant::now(),
            changed_at: None,
            log: Vec::new(),
            quit: false,
        }
    }

    /// The hub.
    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The window processes.
    pub fn processes(&self) -> &P {
        &self.processes
    }

    /// Starts a process for every window in the layout.
    pub fn start(&mut self) {
        let windows = self.hub.window_ids();
        self.start_windows(&windows);
    }

    /// One turn: waits up to `wait` for messages from the windows and
    /// handles them, then handles processes that ended, then saves the
    /// layout if it is due. Returns false once the app should end.
    pub fn turn(&mut self, wait: Duration) -> bool {
        let mut next = self.server.next(wait);
        while let Some(event) = next {
            self.on_event(event);
            next = self.server.next(Duration::ZERO);
        }
        for (window, how) in self.processes.ended() {
            self.on_ended(window, how);
        }
        self.save_if_due(false);
        !self.quit
    }

    /// Ends the app: every window quits, and the layout is saved. Windows
    /// still open after `grace` are killed.
    pub fn finish(&mut self, grace: Duration) {
        let everyone = self.board.recipients(Address::All);
        self.server.send(&everyone, &FromHub::Quit);
        self.save_if_due(true);
        for window in self.processes.stop_all(grace) {
            self.log.push(format!(
                "Window {window} did not close in time and was stopped"
            ));
        }
    }

    /// Takes the lines the launcher logged since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    fn on_event(&mut self, event: PipeEvent) {
        match event {
            PipeEvent::Message(conn, message) => {
                if let Some(window) = self.board.sender(conn, &message) {
                    let reply = self.hub.handle(window, message);
                    self.apply(reply);
                }
            }
            PipeEvent::BadLine(conn, error) => self.log.push(format!(
                "Connection {}: a line is not a message: {error}",
                conn.0
            )),
            PipeEvent::Gone(conn) => {
                self.board.gone(conn);
            }
        }
    }

    /// Carries out a hub reply: sends its messages and starts the windows
    /// it opened.
    fn apply(&mut self, reply: Reply) {
        for output in &reply.messages {
            // The hub sends the layout to everyone only when it changed.
            if output.to == Address::All && matches!(output.message, FromHub::Layout { .. }) {
                self.changed_at.get_or_insert_with(Instant::now);
            }
            let to = self.board.recipients(output.to);
            self.server.send(&to, &output.message);
        }
        self.start_windows(&reply.opened);
        self.quit |= reply.quit;
    }

    fn start_windows(&mut self, windows: &[WindowId]) {
        for window in self.supervisor.start_missing(windows) {
            self.spawn(window);
        }
    }

    fn spawn(&mut self, window: WindowId) {
        if let Err(err) = self.processes.start(window) {
            self.log
                .push(format!("Window {window} could not start: {err}"));
            // It never ran, so its panels pop back in, or the app ends.
            let after = self
                .supervisor
                .ended(window, Ended::Clean, true, self.started.elapsed());
            self.after_exit(window, after);
        }
    }

    fn on_ended(&mut self, window: WindowId, how: Ended) {
        let wanted = self.hub.layout().window(window).is_some();
        if how == Ended::Crashed {
            self.log.push(format!("Window {window} crashed"));
        }
        let after = self
            .supervisor
            .ended(window, how, wanted, self.started.elapsed());
        self.after_exit(window, after);
    }

    fn after_exit(&mut self, window: WindowId, after: AfterExit) {
        match after {
            AfterExit::Nothing => {}
            AfterExit::Restart => self.spawn(window),
            AfterExit::CloseWindow => {
                let reply = self.hub.handle(window, ToHub::CloseWindow { window });
                self.apply(reply);
            }
            AfterExit::Quit => self.quit = true,
        }
    }

    /// Saves the layout once [`SAVE_DELAY`] has passed since it changed,
    /// or at once if `now` is set. An unchanged layout is not saved.
    fn save_if_due(&mut self, now: bool) {
        let due = self
            .changed_at
            .is_some_and(|at| now || at.elapsed() >= SAVE_DELAY);
        if !due {
            return;
        }
        self.changed_at = None;
        if let Err(err) = self.file.save(self.hub.layout()) {
            self.log
                .push(format!("The layout could not be saved: {err}"));
        }
    }
}
