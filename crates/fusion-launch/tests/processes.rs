//! Real child processes and the layout file. The child processes are this
//! test program itself, running one of the ignored helper tests below.

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use fusion_hub::Ended;
use fusion_launch::{ChildProcesses, LayoutFile, WindowProcesses};
use fusion_layout::{Layout, WindowId};

/// A command that runs one helper test of this program.
fn helper(test: &'static str) -> impl FnMut(WindowId) -> Command {
    move |_| {
        let mut command = Command::new(std::env::current_exe().expect("this program"));
        command
            .args(["--ignored", "--exact", test])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

fn wait_for_ended<F: FnMut(WindowId) -> Command>(
    processes: &mut ChildProcesses<F>,
) -> Vec<(WindowId, Ended)> {
    let start = Instant::now();
    loop {
        let ended = processes.ended();
        if !ended.is_empty() || start.elapsed() > Duration::from_secs(10) {
            return ended;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "a helper process for the tests below"]
fn helper_passes() {}

#[test]
#[ignore = "a helper process for the tests below"]
fn helper_fails() {
    panic!("on purpose");
}

#[test]
#[ignore = "a helper process for the tests below"]
fn helper_sleeps() {
    // Short, so a broken stop does not leave it running for long.
    thread::sleep(Duration::from_secs(1));
}

#[test]
fn a_window_that_closes_ends_clean() {
    let mut processes = ChildProcesses::new(helper("helper_passes"));
    assert!(processes.is_empty());
    processes.start(WindowId(2)).expect("starts");
    assert_eq!(processes.len(), 1);
    assert!(!processes.is_empty());
    assert_eq!(
        wait_for_ended(&mut processes),
        [(WindowId(2), Ended::Clean)]
    );
    assert!(processes.is_empty());
    assert_eq!(processes.len(), 0);
}

#[test]
fn a_window_that_fails_crashed() {
    let mut processes = ChildProcesses::new(helper("helper_fails"));
    processes.start(WindowId(3)).expect("starts");
    assert_eq!(
        wait_for_ended(&mut processes),
        [(WindowId(3), Ended::Crashed)]
    );
}

#[test]
fn stopping_kills_what_does_not_end() {
    let mut processes = ChildProcesses::new(helper("helper_sleeps"));
    processes.start(WindowId(2)).expect("starts");
    processes
        .start(WindowId(2))
        .expect("starts again, replacing it");
    assert_eq!(processes.len(), 1);
    processes.start(WindowId(3)).expect("starts");
    assert_eq!(processes.len(), 2);
    assert!(processes.ended().is_empty(), "still running");
    let start = Instant::now();
    assert_eq!(
        processes.stop_all(Duration::from_millis(50)),
        [WindowId(2), WindowId(3)]
    );
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "it waits about the grace, then kills"
    );
    assert!(processes.is_empty());
}

#[test]
fn stopping_waits_for_windows_that_end() {
    let mut processes = ChildProcesses::new(helper("helper_passes"));
    processes.start(WindowId(2)).expect("starts");
    let start = Instant::now();
    assert!(
        processes.stop_all(Duration::from_secs(10)).is_empty(),
        "it ended on its own, nothing was killed"
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "it stops waiting once the window ended"
    );
    assert!(processes.is_empty());
}

#[test]
fn a_program_that_is_not_there_does_not_start() {
    let mut processes = ChildProcesses::new(|_| Command::new("fusion-no-such-program-anywhere"));
    assert!(processes.start(WindowId(2)).is_err());
    assert!(processes.is_empty());
}

// --- The layout file ---------------------------------------------------------

fn temp(test: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("fusion-file-test-{}-{test}", std::process::id()))
}

#[test]
fn a_saved_layout_loads_again() {
    let folder = temp("roundtrip");
    let file = LayoutFile::new(folder.join("deep").join("layout.json"));
    assert!(file.load().expect("no file is fine").is_none());
    let layout = Layout::new("Fusion");
    file.save(&layout).expect("saves into a new folder");
    file.save(&layout).expect("saves over the old file");
    assert_eq!(file.load().expect("loads"), Some(layout));
    assert!(file.path().ends_with("layout.json"));
    // The folder is this test's own; a leftover is harmless.
    let _removed = std::fs::remove_dir_all(folder);
}

#[test]
fn a_broken_file_says_why() {
    let folder = temp("broken");
    std::fs::create_dir_all(&folder).expect("folder");
    let path = folder.join("layout.json");
    std::fs::write(&path, "{ nope").expect("writes");
    let err = LayoutFile::new(&path).load().expect_err("refused");
    assert!(!err.to_string().is_empty());

    // A folder where the file should be: the file system refuses.
    let folder_as_file = LayoutFile::new(&folder);
    let err = folder_as_file.load().expect_err("refused");
    assert!(!err.to_string().is_empty());
    assert!(folder_as_file.save(&Layout::new("Fusion")).is_err());

    // A file where the folder should be.
    let under_a_file = LayoutFile::new(path.join("layout.json"));
    assert!(under_a_file.save(&Layout::new("Fusion")).is_err());
    let _removed = std::fs::remove_dir_all(folder);
}
