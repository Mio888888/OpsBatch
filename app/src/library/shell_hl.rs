//! Lightweight shell highlighter for command previews (port of `tokenizeShell` in CommandsInlinePanel.tsx).

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Builtin,
    Str,
    Variable,
    Operator,
    Comment,
    Number,
    Text,
}

const KEYWORDS: &[&str] = &[
    "if",
    "then",
    "else",
    "elif",
    "fi",
    "for",
    "while",
    "until",
    "do",
    "done",
    "case",
    "esac",
    "in",
    "function",
    "select",
    "time",
    "coproc",
    "return",
    "exit",
    "export",
    "local",
    "declare",
    "readonly",
    "set",
    "unset",
    "shift",
    "source",
    "alias",
    "unalias",
    "trap",
    "eval",
    "exec",
    "read",
    "echo",
    "printf",
    "test",
    "true",
    "false",
    "let",
    "typeset",
    "hash",
    "type",
    "command",
    "builtin",
    "break",
    "continue",
    "getopts",
    "wait",
    "jobs",
    "bg",
    "fg",
    "disown",
    "suspend",
    "logout",
    "mapfile",
    "readarray",
];

const BUILTINS: &[&str] = &[
    "sudo",
    "apt",
    "yum",
    "dnf",
    "rpm",
    "dpkg",
    "snap",
    "pip",
    "npm",
    "npx",
    "git",
    "docker",
    "kubectl",
    "ssh",
    "scp",
    "rsync",
    "curl",
    "wget",
    "systemctl",
    "journalctl",
    "service",
    "crontab",
    "chmod",
    "chown",
    "mkdir",
    "rmdir",
    "rm",
    "cp",
    "mv",
    "ln",
    "touch",
    "find",
    "grep",
    "awk",
    "sed",
    "sort",
    "uniq",
    "wc",
    "head",
    "tail",
    "cat",
    "tee",
    "xargs",
    "tr",
    "cut",
    "paste",
    "diff",
    "patch",
    "tar",
    "gzip",
    "gunzip",
    "zip",
    "unzip",
    "mount",
    "umount",
    "fdisk",
    "mkfs",
    "fsck",
    "du",
    "df",
    "ls",
    "lsblk",
    "blkid",
    "top",
    "htop",
    "ps",
    "kill",
    "killall",
    "nice",
    "renice",
    "nohup",
    "timeout",
    "sleep",
    "date",
    "cal",
    "uptime",
    "free",
    "vmstat",
    "iostat",
    "mpstat",
    "lsof",
    "ss",
    "ip",
    "ifconfig",
    "ping",
    "traceroute",
    "dig",
    "nslookup",
    "host",
    "netstat",
    "nc",
    "iptables",
    "ufw",
    "firewall-cmd",
    "badblocks",
    "hostnamectl",
    "uname",
    "whoami",
    "id",
    "w",
    "who",
    "last",
    "lastb",
    "dmesg",
    "lscpu",
    "lsmod",
    "modprobe",
    "insmod",
    "rmmod",
    "dmidecode",
];

/// Token spans (byte ranges, covering the whole string) with their class.
pub fn tokenize(code: &str) -> Vec<(Range<usize>, Tok)> {
    let b = code.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, Tok)> = Vec::new();
    let push = |out: &mut Vec<(Range<usize>, Tok)>, r: Range<usize>, t: Tok| out.push((r, t));
    let mut i = 0;
    while i < n {
        let c = b[i];
        if c == b'#' {
            let s = i;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            push(&mut out, s..i, Tok::Comment);
        } else if c == b'\'' {
            let s = i;
            i += 1;
            while i < n && b[i] != b'\'' {
                i += 1;
            }
            if i < n {
                i += 1;
            }
            push(&mut out, s..i, Tok::Str);
        } else if c == b'"' {
            let s = i;
            i += 1;
            while i < n && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < n {
                    i += 1;
                }
                i += 1;
            }
            if i < n {
                i += 1;
            }
            push(&mut out, s..i.min(n), Tok::Str);
        } else if c == b'$' && i + 1 < n && b[i + 1] == b'{' {
            let s = i;
            i += 2;
            while i < n && b[i] != b'}' {
                i += 1;
            }
            if i < n {
                i += 1;
            }
            push(&mut out, s..i, Tok::Variable);
        } else if c == b'$'
            && i + 1 < n
            && (matches!(b[i + 1], b'?' | b'!' | b'$' | b'-' | b'_')
                || b[i + 1].is_ascii_alphabetic())
        {
            let s = i;
            i += 1;
            if b[i].is_ascii_alphabetic() || b[i] == b'_' {
                while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
            } else {
                i += 1;
            }
            push(&mut out, s..i, Tok::Variable);
        } else if matches!(c, b'|' | b'&' | b';' | b'>' | b'<') {
            let s = i;
            i += 1;
            if i < n && b[i] == c && c != b';' {
                i += 1;
            }
            push(&mut out, s..i, Tok::Operator);
        } else if c.is_ascii_digit()
            && (i == 0
                || b[i - 1].is_ascii_whitespace()
                || matches!(b[i - 1], b'/' | b'=' | b'<' | b'>'))
        {
            let s = i;
            while i < n && b[i].is_ascii_digit() {
                i += 1;
            }
            push(&mut out, s..i, Tok::Number);
        } else if c.is_ascii_alphabetic() || matches!(c, b'_' | b'.' | b'/' | b'~' | b'-') {
            let s = i;
            while i < n
                && (b[i].is_ascii_alphanumeric()
                    || matches!(b[i], b'_' | b'.' | b'/' | b'~' | b'-'))
            {
                i += 1;
            }
            let w = &code[s..i];
            let t = if KEYWORDS.contains(&w) {
                Tok::Keyword
            } else if BUILTINS.contains(&w) {
                Tok::Builtin
            } else {
                Tok::Text
            };
            push(&mut out, s..i, t);
        } else {
            // One (possibly multi-byte) character of plain text.
            let ch_len = code[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            push(&mut out, i..i + ch_len, Tok::Text);
            i += ch_len;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(code: &str) -> Vec<(&str, Tok)> {
        tokenize(code)
            .into_iter()
            .filter(|(_, t)| *t != Tok::Text)
            .map(|(r, t)| (&code[r], t))
            .collect()
    }

    #[test]
    fn classifies_common_shell_constructs() {
        let k = kinds("sudo docker ps | grep \"web\" && echo $HOME ${NAME} # done");
        assert_eq!(
            k,
            vec![
                ("sudo", Tok::Builtin),
                ("docker", Tok::Builtin),
                ("ps", Tok::Builtin),
                ("|", Tok::Operator),
                ("grep", Tok::Builtin),
                ("\"web\"", Tok::Str),
                ("&&", Tok::Operator),
                ("echo", Tok::Keyword),
                ("$HOME", Tok::Variable),
                ("${NAME}", Tok::Variable),
                ("# done", Tok::Comment),
            ]
        );
    }

    #[test]
    fn spans_cover_the_whole_input_even_with_unterminated_quotes_and_unicode() {
        for code in [
            "echo 'unterminated",
            "echo \"half",
            "printf '你好' | tr a b",
            "$",
            "a;;b",
            "x > /tmp/1 2>&1",
        ] {
            let t = tokenize(code);
            let mut at = 0;
            for (r, _) in &t {
                assert_eq!(r.start, at, "gap in {code:?}");
                at = r.end;
            }
            assert_eq!(at, code.len(), "{code:?}");
        }
    }

    #[test]
    fn numbers_only_after_separators() {
        assert_eq!(
            kinds("sleep 10"),
            vec![("sleep", Tok::Builtin), ("10", Tok::Number)]
        );
        assert!(kinds("file2").is_empty());
    }
}
