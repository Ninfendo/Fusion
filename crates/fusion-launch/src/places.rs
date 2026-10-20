//! Where Fusion keeps its files.

use std::ffi::OsString;
use std::path::PathBuf;

/// Fusion's settings folder: `%APPDATA%\Fusion` on Windows,
/// `~/Library/Application Support/Fusion` on macOS, and
/// `$XDG_CONFIG_HOME/fusion` or `~/.config/fusion` elsewhere.
///
/// `os` is [`std::env::consts::OS`] and `var` reads an environment
/// variable; both are parameters so every case is testable.
pub fn config_dir(os: &str, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    match os {
        "windows" => set("APPDATA").map(|dir| dir.join("Fusion")),
        "macos" => set("HOME").map(|home| home.join("Library/Application Support/Fusion")),
        _ => set("XDG_CONFIG_HOME")
            .or_else(|| set("HOME").map(|home| home.join(".config")))
            .map(|dir| dir.join("fusion")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn each_system_has_its_place() {
        let all = env(&[
            ("APPDATA", "C:/Users/me/AppData/Roaming"),
            ("HOME", "/home/me"),
            ("XDG_CONFIG_HOME", "/home/me/conf"),
        ]);
        assert_eq!(
            config_dir("windows", &all),
            Some(PathBuf::from("C:/Users/me/AppData/Roaming/Fusion"))
        );
        assert_eq!(
            config_dir("macos", &all),
            Some(PathBuf::from("/home/me/Library/Application Support/Fusion"))
        );
        assert_eq!(
            config_dir("linux", &all),
            Some(PathBuf::from("/home/me/conf/fusion"))
        );
    }

    #[test]
    fn linux_falls_back_to_the_home_folder() {
        let home = env(&[("HOME", "/home/me"), ("XDG_CONFIG_HOME", "")]);
        assert_eq!(
            config_dir("linux", home),
            Some(PathBuf::from("/home/me/.config/fusion"))
        );
    }

    #[test]
    fn no_home_means_no_place() {
        assert_eq!(config_dir("windows", env(&[])), None);
        assert_eq!(config_dir("macos", env(&[])), None);
        assert_eq!(config_dir("linux", env(&[])), None);
    }
}
