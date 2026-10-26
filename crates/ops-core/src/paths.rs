//! Data directory resolution. Must match Tauri's `app_data_dir()` for identifier `com.mio.OpsBatch`
//! so the GPUI build reads the same database and vault as the Tauri build.

use std::path::PathBuf;

pub const APP_IDENTIFIER: &str = "com.mio.OpsBatch";

/// `$OPSBATCH_DATA_DIR` overrides everything (tests, side-by-side dev profiles).
pub fn app_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OPSBATCH_DATA_DIR") {
        return PathBuf::from(dir);
    }
    base_data_dir().join(APP_IDENTIFIER)
}

#[cfg(target_os = "macos")]
fn base_data_dir() -> PathBuf {
    home().join("Library").join("Application Support")
}

#[cfg(target_os = "windows")]
fn base_data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn base_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local").join("share"))
}

fn home() -> PathBuf {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Directory the *running application* uses.
///
/// Until the real encrypted vault is enabled (`real-vault` feature), host passwords/keys only live in memory while
/// the database stores placeholders. Pointing that at the user's real Tauri data directory would silently strip the
/// secrets the Tauri build keeps there, so development builds use a sibling `<identifier>-dev` directory instead.
/// `OPSBATCH_DATA_DIR` always wins.
pub fn run_data_dir() -> PathBuf {
    let dir = app_data_dir();
    if std::env::var_os("OPSBATCH_DATA_DIR").is_some() || cfg!(feature = "real-vault") {
        return dir;
    }
    let mut name = dir.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push("-dev");
    dir.with_file_name(name)
}

pub fn database_path() -> PathBuf {
    app_data_dir().join("opsbatch.db")
}

pub fn diagnostics_log_path() -> PathBuf {
    app_data_dir().join("opsbatch-diagnostics.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_ends_with_identifier_and_db_name() {
        let dir = app_data_dir();
        if std::env::var_os("OPSBATCH_DATA_DIR").is_none() {
            assert!(dir.ends_with(APP_IDENTIFIER), "{dir:?}");
        }
        assert!(database_path().ends_with("opsbatch.db"));
    }

    #[test]
    fn run_dir_never_aliases_the_tauri_directory_without_the_real_vault() {
        if std::env::var_os("OPSBATCH_DATA_DIR").is_none() {
            let run = run_data_dir();
            if cfg!(feature = "real-vault") {
                assert_eq!(run, app_data_dir());
            } else {
                assert_ne!(run, app_data_dir());
                assert!(run.to_string_lossy().ends_with("com.mio.OpsBatch-dev"), "{run:?}");
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_layout_matches_tauri() {
        if std::env::var_os("OPSBATCH_DATA_DIR").is_none() {
            let s = app_data_dir().to_string_lossy().into_owned();
            assert!(s.contains("Library/Application Support/com.mio.OpsBatch"), "{s}");
        }
    }
}
