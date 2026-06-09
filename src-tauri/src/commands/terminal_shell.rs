#[derive(Clone, Copy)]
pub enum ShellPlatform {
    Windows,
    Macos,
    Linux,
}

pub fn current_shell_platform() -> ShellPlatform {
    if cfg!(target_os = "windows") {
        ShellPlatform::Windows
    } else if cfg!(target_os = "macos") {
        ShellPlatform::Macos
    } else {
        ShellPlatform::Linux
    }
}

pub fn select_local_shell<I, K, V, F>(platform: ShellPlatform, env: I, exists: F) -> String
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
    F: Fn(&str) -> bool,
{
    let env: Vec<(String, String)> = env
        .into_iter()
        .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().trim().to_string()))
        .collect();

    let env_value = |name: &str| {
        env.iter()
            .find(|(key, value)| {
                !value.is_empty()
                    && if matches!(platform, ShellPlatform::Windows) {
                        key.eq_ignore_ascii_case(name)
                    } else {
                        key == name
                    }
            })
            .map(|(_, value)| value.as_str())
    };

    if !matches!(platform, ShellPlatform::Windows) {
        if let Some(shell) = env_value("SHELL").filter(|shell| exists(shell)) {
            return shell.to_string();
        }
    }

    match platform {
        ShellPlatform::Windows => {
            if let Some(comspec) = env_value("COMSPEC").filter(|shell| exists(shell)) {
                return comspec.to_string();
            }

            if let Some(system_root) = env_value("SystemRoot") {
                let cmd = format!("{}\\System32\\cmd.exe", system_root.trim_end_matches('\\'));
                if exists(&cmd) {
                    return cmd;
                }
            }

            "cmd.exe".to_string()
        }
        ShellPlatform::Macos => ["/bin/zsh", "/bin/bash", "/bin/sh"]
            .into_iter()
            .find(|shell| exists(shell))
            .unwrap_or("/bin/sh")
            .to_string(),
        ShellPlatform::Linux => ["/bin/bash", "/usr/bin/bash", "/bin/sh", "/usr/bin/sh"]
            .into_iter()
            .find(|shell| exists(shell))
            .unwrap_or("/bin/sh")
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{select_local_shell, ShellPlatform};

    fn exists(path: &str) -> bool {
        matches!(
            path,
            "C:\\Windows\\System32\\cmd.exe"
                | "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"
                | "/bin/zsh"
                | "/bin/bash"
                | "/bin/sh"
        )
    }

    #[test]
    fn windows_ignores_unix_shell_env_and_uses_windows_shell() {
        let shell = select_local_shell(
            ShellPlatform::Windows,
            [
                ("SHELL", "/bin/zsh"),
                ("COMSPEC", "C:\\Windows\\System32\\cmd.exe"),
                ("SystemRoot", "C:\\Windows"),
            ],
            exists,
        );

        assert_eq!("C:\\Windows\\System32\\cmd.exe", shell);
    }

    #[test]
    fn windows_reads_comspec_case_insensitively() {
        let shell = select_local_shell(
            ShellPlatform::Windows,
            [("ComSpec", "C:\\Windows\\System32\\cmd.exe")],
            exists,
        );

        assert_eq!("C:\\Windows\\System32\\cmd.exe", shell);
    }

    #[test]
    fn macos_uses_shell_env_when_it_is_a_unix_executable() {
        let shell = select_local_shell(ShellPlatform::Macos, [("SHELL", "/bin/zsh")], exists);

        assert_eq!("/bin/zsh", shell);
    }

    #[test]
    fn linux_falls_back_to_bash_when_zsh_is_unavailable() {
        let shell = select_local_shell(ShellPlatform::Linux, [("SHELL", "/missing/zsh")], |path| {
            matches!(path, "/bin/bash" | "/bin/sh")
        });

        assert_eq!("/bin/bash", shell);
    }
}
