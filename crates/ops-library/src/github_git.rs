use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn build_auth_callbacks(token: &str) -> Result<git2::RemoteCallbacks<'static>, String> {
    let mut callbacks = git2::RemoteCallbacks::new();
    let token = token.to_string();
    callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
        git2::Cred::userpass_plaintext("x-access-token", &token)
    });
    Ok(callbacks)
}

pub fn should_retry_with_system_git(error: &git2::Error) -> bool {
    error.class() == git2::ErrorClass::Ssl
        || error
            .message()
            .to_ascii_lowercase()
            .contains("ssl handshake")
}

fn build_git_clone_args(url: &str, branch: &str, local_path: &Path) -> Vec<OsString> {
    vec![
        OsString::from("clone"),
        OsString::from("--branch"),
        OsString::from(branch),
        OsString::from("--"),
        OsString::from(url),
        local_path.as_os_str().to_os_string(),
    ]
}

fn build_git_update_steps(branch: &str) -> Vec<Vec<OsString>> {
    vec![
        vec![
            OsString::from("fetch"),
            OsString::from("--prune"),
            OsString::from("origin"),
            OsString::from(branch),
        ],
        vec![
            OsString::from("checkout"),
            OsString::from("-B"),
            OsString::from(branch),
            OsString::from("FETCH_HEAD"),
        ],
        vec![
            OsString::from("reset"),
            OsString::from("--hard"),
            OsString::from("FETCH_HEAD"),
        ],
    ]
}

fn create_git_askpass_script(temp_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(temp_dir).map_err(|e| e.to_string())?;

    #[cfg(windows)]
    let (script_path, script) = (
        temp_dir.join("opsbatch-git-askpass.bat"),
        "@echo off\r\necho %1 | findstr /I \"username\" >nul\r\nif %errorlevel%==0 (\r\n  echo %OPSBATCH_GIT_USERNAME%\r\n) else (\r\n  echo %OPSBATCH_GIT_PASSWORD%\r\n)\r\n",
    );

    #[cfg(not(windows))]
    let (script_path, script) = (
        temp_dir.join("opsbatch-git-askpass.sh"),
        "#!/bin/sh\ncase \"$1\" in\n  *Username*|*username*) printf '%s\\n' \"$OPSBATCH_GIT_USERNAME\" ;;\n  *) printf '%s\\n' \"$OPSBATCH_GIT_PASSWORD\" ;;\nesac\n",
    );

    std::fs::write(&script_path, script).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(script_path)
}

struct GitCliAuth {
    temp_dir: Option<PathBuf>,
    askpass_path: Option<PathBuf>,
    token: Option<String>,
}

impl GitCliAuth {
    fn new(token: Option<&str>) -> Result<Self, String> {
        let token = token.filter(|value| !value.is_empty()).map(str::to_string);
        let Some(_) = token else {
            return Ok(Self {
                temp_dir: None,
                askpass_path: None,
                token: None,
            });
        };

        let temp_dir = env::temp_dir().join(format!("opsbatch-git-{}", uuid::Uuid::new_v4()));
        let askpass_path = create_git_askpass_script(&temp_dir)?;
        Ok(Self {
            temp_dir: Some(temp_dir),
            askpass_path: Some(askpass_path),
            token,
        })
    }

    fn apply_to(&self, command: &mut Command) {
        command.env("GIT_TERMINAL_PROMPT", "0");
        if let (Some(askpass_path), Some(token)) = (&self.askpass_path, &self.token) {
            command.env("GIT_ASKPASS", askpass_path);
            command.env("OPSBATCH_GIT_USERNAME", "x-access-token");
            command.env("OPSBATCH_GIT_PASSWORD", token);
        }
    }
}

impl Drop for GitCliAuth {
    fn drop(&mut self) {
        if let Some(temp_dir) = &self.temp_dir {
            let _ = std::fs::remove_dir_all(temp_dir);
        }
    }
}

fn run_git_command(
    args: &[OsString],
    current_dir: Option<&Path>,
    auth: &GitCliAuth,
) -> Result<(), String> {
    let mut command = Command::new("git");
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    command.args(args);
    auth.apply_to(&mut command);

    let output = command
        .output()
        .map_err(|e| format!("run git failed: {}", e))?;
    if output.status.success() {
        return Ok(());
    }

    Err(format_git_failure(args, &output, auth.token.as_deref()))
}

fn format_git_failure(args: &[OsString], output: &Output, token: Option<&str>) -> String {
    let rendered_args = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let stderr = redact_secret(String::from_utf8_lossy(&output.stderr).trim(), token);
    let stdout = redact_secret(String::from_utf8_lossy(&output.stdout).trim(), token);
    let detail = if stderr.is_empty() { stdout } else { stderr };
    format!(
        "git {} failed ({}): {}",
        rendered_args, output.status, detail
    )
}

fn redact_secret(text: &str, secret: Option<&str>) -> String {
    match secret.filter(|value| !value.is_empty()) {
        Some(secret) => text.replace(secret, "[redacted]"),
        None => text.to_string(),
    }
}

pub fn run_system_git_clone(
    url: &str,
    branch: &str,
    local_path: &Path,
    token: Option<&str>,
) -> Result<(), String> {
    let auth = GitCliAuth::new(token)?;
    let args = build_git_clone_args(url, branch, local_path);
    run_git_command(&args, None, &auth)
}

pub fn run_system_git_update(
    local_path: &Path,
    branch: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let auth = GitCliAuth::new(token)?;
    for args in build_git_update_steps(branch) {
        run_git_command(&args, Some(local_path), &auth)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_args_keep_url_branch_and_path_as_separate_arguments() {
        let local_path = PathBuf::from("/tmp/ops batch/repos/abcd");
        let args = build_git_clone_args(
            "https://github.com/mio/ops-library.git",
            "release/2026.06",
            &local_path,
        );

        assert_eq!("clone", args[0].to_string_lossy());
        assert_eq!("--branch", args[1].to_string_lossy());
        assert_eq!("release/2026.06", args[2].to_string_lossy());
        assert_eq!("--", args[3].to_string_lossy());
        assert_eq!(
            "https://github.com/mio/ops-library.git",
            args[4].to_string_lossy()
        );
        assert_eq!(local_path.as_os_str(), args[5].as_os_str());
    }

    #[test]
    fn update_steps_fetch_branch_then_force_worktree_to_fetch_head() {
        let steps = build_git_update_steps("main");

        let fetch = steps[0]
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let checkout = steps[1]
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let reset = steps[2]
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(vec!["fetch", "--prune", "origin", "main"], fetch);
        assert_eq!(vec!["checkout", "-B", "main", "FETCH_HEAD"], checkout);
        assert_eq!(vec!["reset", "--hard", "FETCH_HEAD"], reset);
    }

    #[test]
    fn askpass_script_uses_environment_without_embedding_token() {
        let temp_dir =
            std::env::temp_dir().join(format!("opsbatch-git-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let script_path = create_git_askpass_script(&temp_dir).unwrap();
        let script = std::fs::read_to_string(&script_path).unwrap();

        assert!(script.contains("OPSBATCH_GIT_PASSWORD"));
        assert!(!script.contains("ghp_secret_token"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
