//! Dangerous-command guard: pre-compiled regex rules checked when the user presses Enter in a terminal.

use regex::Regex;

pub use ops_core::settings::DangerRule;

#[derive(Debug, Clone)]
pub struct CompiledDangerRule {
    pub name: String,
    pub regex: Regex,
}

/// Compile enabled rules once (call again only when rules change). Invalid patterns are skipped.
pub fn compile_danger_rules(rules: &[DangerRule]) -> Vec<CompiledDangerRule> {
    rules
        .iter()
        .filter(|r| r.enabled)
        .filter_map(|r| {
            Regex::new(&r.pattern).ok().map(|regex| CompiledDangerRule {
                name: r.name.clone(),
                regex,
            })
        })
        .collect()
}

/// Names of all rules the command matches.
pub fn check_dangerous_command(compiled: &[CompiledDangerRule], cmd: &str) -> Vec<String> {
    compiled
        .iter()
        .filter(|r| r.regex.is_match(cmd))
        .map(|r| r.name.clone())
        .collect()
}

/// Built-in patterns, identical to the backend seed list. Used as the fallback so a terminal is protected
/// from its first keystroke, before the stored rules (with user additions) have been loaded.
const DEFAULT_PATTERNS: &[(&str, &str, &str)] = &[
    (
        "builtin-rm-rf-danger",
        "rm -rf (危险路径)",
        r#"rm\s+-rf\s+[/.*~]"#,
    ),
    (
        "builtin-fork-bomb",
        "fork bomb",
        r#":\(\)\{\s*:\|:\&\s*\}\s*;"#,
    ),
    ("builtin-dd", "dd overwrite", r#"dd\s+if="#),
    ("builtin-mkfs", "mkfs", r#"mkfs\."#),
    ("builtin-dev-sda", "/dev/sda redirect", r#">\s*/dev/sda"#),
    (
        "builtin-chmod-777",
        "chmod 777 /",
        r#"chmod\s+-R\s+777\s+/"#,
    ),
    ("builtin-chown-root", "chown /", r#"chown\s+-R\s+\w+\s+/"#),
    ("builtin-shutdown", "shutdown", r#"shutdown\s+"#),
    ("builtin-reboot", "reboot", r#"reboot\b"#),
    ("builtin-init", "init 0/6", r#"init\s+[06]\b"#),
    (
        "builtin-drop-db",
        "drop database",
        r#"(?:DROP|drop)\s+(?:DATABASE|database|SCHEMA|schema)"#,
    ),
    (
        "builtin-truncate",
        "truncate table",
        r#"(?:TRUNCATE|truncate)\s+(?:TABLE|table)?\s*\w"#,
    ),
    (
        "builtin-systemctl-critical",
        "systemctl stop critical",
        r#"systemctl\s+(?:stop|disable|mask)\s+(?:sshd|nginx|docker|firewalld|NetworkManager|systemd)\b"#,
    ),
    (
        "builtin-iptables-flush",
        "iptables flush",
        r#"iptables\s+-F"#,
    ),
    (
        "builtin-rm-var-log",
        "rm -rf var/log",
        r#"rm\s+-rf\s+/var/log"#,
    ),
    ("builtin-wipefs", "wipefs", r#"wipefs\s+-a"#),
    ("builtin-passwd", "> /etc/passwd", r#">\s*/etc/passwd"#),
    (
        "builtin-mv-devnull",
        "mv /* /dev/null",
        r#"mv\s+/\S*\s+/dev/null"#,
    ),
];

pub fn default_danger_rules() -> Vec<DangerRule> {
    DEFAULT_PATTERNS
        .iter()
        .map(|(id, name, pattern)| DangerRule {
            id: id.to_string(),
            name: name.to_string(),
            pattern: pattern.to_string(),
            enabled: true,
            is_builtin: true,
        })
        .collect()
}

pub fn default_compiled_danger_rules() -> Vec<CompiledDangerRule> {
    compile_danger_rules(&default_danger_rules())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(cmd: &str) -> Vec<String> {
        check_dangerous_command(&default_compiled_danger_rules(), cmd)
    }

    #[test]
    fn every_default_pattern_compiles() {
        assert_eq!(
            default_compiled_danger_rules().len(),
            DEFAULT_PATTERNS.len()
        );
        assert_eq!(DEFAULT_PATTERNS.len(), 18);
    }

    #[test]
    fn flags_destructive_commands() {
        assert!(hits("rm -rf /").iter().any(|n| n.contains("rm -rf")));
        assert!(hits("rm -rf ~/").iter().any(|n| n.contains("rm -rf")));
        assert!(hits(":(){ :|:& };:").contains(&"fork bomb".to_string()));
        assert!(hits("dd if=/dev/zero of=/dev/sda").contains(&"dd overwrite".to_string()));
        assert!(hits("mkfs.ext4 /dev/sdb1").contains(&"mkfs".to_string()));
        assert!(hits("echo x > /dev/sda").contains(&"/dev/sda redirect".to_string()));
        assert!(hits("chmod -R 777 /").contains(&"chmod 777 /".to_string()));
        assert!(hits("shutdown -h now").contains(&"shutdown".to_string()));
        assert!(hits("sudo reboot").contains(&"reboot".to_string()));
        assert!(hits("init 0").contains(&"init 0/6".to_string()));
        assert!(hits("mysql -e 'DROP DATABASE prod'").contains(&"drop database".to_string()));
        assert!(hits("systemctl stop sshd").contains(&"systemctl stop critical".to_string()));
        assert!(hits("iptables -F").contains(&"iptables flush".to_string()));
        assert!(hits("wipefs -a /dev/sda").contains(&"wipefs".to_string()));
    }

    #[test]
    fn leaves_ordinary_commands_alone() {
        for cmd in [
            "ls -la",
            "rm -rf build",
            "systemctl status nginx",
            "echo hello",
            "git reset --hard",
            "cat /etc/passwd",
        ] {
            assert!(hits(cmd).is_empty(), "{cmd} -> {:?}", hits(cmd));
        }
    }

    #[test]
    fn disabled_and_invalid_rules_are_skipped() {
        let rules = vec![
            DangerRule {
                id: "1".into(),
                name: "off".into(),
                pattern: "foo".into(),
                enabled: false,
                is_builtin: false,
            },
            DangerRule {
                id: "2".into(),
                name: "bad".into(),
                pattern: "(".into(),
                enabled: true,
                is_builtin: false,
            },
            DangerRule {
                id: "3".into(),
                name: "custom".into(),
                pattern: "curl .*\\| sh".into(),
                enabled: true,
                is_builtin: false,
            },
        ];
        let compiled = compile_danger_rules(&rules);
        assert_eq!(compiled.len(), 1);
        assert_eq!(
            check_dangerous_command(&compiled, "curl http://x | sh"),
            vec!["custom".to_string()]
        );
        assert!(check_dangerous_command(&compiled, "foo").is_empty());
    }
}
