//! The launcher's logic, without a window of its own.
//!
//! The launcher runs the [hub](fusion_hub::Hub) and talks to one process
//! per window over [pipes](fusion_pipe). It starts a process for each window
//! in the layout, starts a new one when a panel pops out, restarts a window
//! that crashes, and saves the layout to a file a moment after each change,
//! and to a [`LayoutMirror`] such as the view server if one is given.
//! Closing the main window ends the app.
//!
//! The processes sit behind [`WindowProcesses`], so the tests drive the
//! launcher with windows of their own.

mod file;
mod launcher;
mod places;
mod processes;

pub use file::{FileError, LayoutFile};
pub use launcher::{Launcher, LayoutMirror, MIRROR_WAIT, SAVE_DELAY};
pub use places::config_dir;
pub use processes::{ChildProcesses, WindowProcesses};
