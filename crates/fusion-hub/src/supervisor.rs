//! When the launcher starts, restarts and closes window processes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use fusion_layout::WindowId;

/// A window that crashes more often than this within [`CRASH_SPAN`] is not
/// started again.
pub const MAX_CRASHES: usize = 3;

/// The span [`MAX_CRASHES`] counts over.
pub const CRASH_SPAN: Duration = Duration::from_secs(30);

/// How a window process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// It closed normally, for example because the user closed it.
    Clean,
    /// It crashed or was killed.
    Crashed,
}

/// What the launcher does after a window process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterExit {
    /// Nothing: the window is no longer in the layout.
    Nothing,
    /// Start the window again. Its panels come back from the hub.
    Restart,
    /// Tell the hub the window closed, so its panels pop back in.
    CloseWindow,
    /// End the app.
    Quit,
}

/// Keeps one process per window in the layout. It decides only; the
/// launcher starts and watches the processes.
#[derive(Debug, Clone, Default)]
pub struct Supervisor {
    running: BTreeSet<WindowId>,
    crashes: BTreeMap<WindowId, Vec<Duration>>,
}

impl Supervisor {
    /// The windows in `wanted` that have no process yet. They count as
    /// running from now on, so start each of them.
    pub fn start_missing(&mut self, wanted: &[WindowId]) -> Vec<WindowId> {
        wanted
            .iter()
            .copied()
            .filter(|window| self.running.insert(*window))
            .collect()
    }

    /// A window process ended at `now`, the time since the launcher
    /// started. `wanted` is true if the window is still in the layout.
    ///
    /// A clean end means the user closed the window: its panels pop back in,
    /// or the app ends if it was the main window. A crash restarts the
    /// window, unless it crashes too often.
    pub fn ended(
        &mut self,
        window: WindowId,
        how: Ended,
        wanted: bool,
        now: Duration,
    ) -> AfterExit {
        self.running.remove(&window);
        if !wanted {
            return AfterExit::Nothing;
        }
        let give_up = how == Ended::Clean || self.crashed_too_often(window, now);
        match (give_up, window == WindowId::MAIN) {
            (false, _) => {
                self.running.insert(window);
                AfterExit::Restart
            }
            (true, true) => AfterExit::Quit,
            (true, false) => AfterExit::CloseWindow,
        }
    }

    fn crashed_too_often(&mut self, window: WindowId, now: Duration) -> bool {
        let times = self.crashes.entry(window).or_default();
        times.retain(|at| now.saturating_sub(*at) < CRASH_SPAN);
        times.push(now);
        times.len() > MAX_CRASHES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W2: WindowId = WindowId(2);

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn every_window_gets_one_process() {
        let mut sup = Supervisor::default();
        assert_eq!(
            sup.start_missing(&[WindowId::MAIN, W2]),
            [WindowId::MAIN, W2]
        );
        assert!(sup.start_missing(&[WindowId::MAIN, W2]).is_empty());
    }

    #[test]
    fn a_crashed_window_starts_again() {
        let mut sup = Supervisor::default();
        sup.start_missing(&[W2]);
        assert_eq!(
            sup.ended(W2, Ended::Crashed, true, secs(1)),
            AfterExit::Restart
        );
        assert!(sup.start_missing(&[W2]).is_empty(), "it counts as running");
    }

    #[test]
    fn a_window_that_keeps_crashing_closes() {
        let mut sup = Supervisor::default();
        for s in 1..=3 {
            assert_eq!(
                sup.ended(W2, Ended::Crashed, true, secs(s)),
                AfterExit::Restart
            );
        }
        assert_eq!(
            sup.ended(W2, Ended::Crashed, true, secs(4)),
            AfterExit::CloseWindow
        );
        assert_eq!(
            sup.ended(WindowId::MAIN, Ended::Crashed, true, secs(4)),
            AfterExit::Restart,
            "windows count their crashes apart"
        );
    }

    #[test]
    fn old_crashes_are_forgotten() {
        let mut sup = Supervisor::default();
        for s in [0, 10, 20] {
            sup.ended(W2, Ended::Crashed, true, secs(s));
        }
        assert_eq!(
            sup.ended(W2, Ended::Crashed, true, secs(30)),
            AfterExit::Restart,
            "the crash at 0 s is 30 s old"
        );
        assert_eq!(
            sup.ended(W2, Ended::Crashed, true, secs(31)),
            AfterExit::CloseWindow
        );
    }

    #[test]
    fn the_main_window_that_keeps_crashing_ends_the_app() {
        let mut sup = Supervisor::default();
        for s in 1..=3 {
            sup.ended(WindowId::MAIN, Ended::Crashed, true, secs(s));
        }
        assert_eq!(
            sup.ended(WindowId::MAIN, Ended::Crashed, true, secs(4)),
            AfterExit::Quit
        );
    }

    #[test]
    fn a_closed_window_pops_its_panels_in() {
        let mut sup = Supervisor::default();
        sup.start_missing(&[W2]);
        assert_eq!(
            sup.ended(W2, Ended::Clean, true, secs(1)),
            AfterExit::CloseWindow
        );
        assert_eq!(sup.start_missing(&[W2]), [W2], "no longer running");
        assert_eq!(
            sup.ended(WindowId::MAIN, Ended::Clean, true, secs(1)),
            AfterExit::Quit
        );
    }

    #[test]
    fn a_window_gone_from_the_layout_needs_nothing() {
        let mut sup = Supervisor::default();
        sup.start_missing(&[W2]);
        assert_eq!(
            sup.ended(W2, Ended::Crashed, false, secs(1)),
            AfterExit::Nothing
        );
        assert_eq!(sup.start_missing(&[W2]), [W2]);
    }
}
