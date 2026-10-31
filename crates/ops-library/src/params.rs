//! `{{name:default}}` placeholders in quick-action commands and the `curl | bash` builder for remote scripts
//! (ports of `utils/quickActionParams.ts` and `utils/remoteScriptCommand.ts`).

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{([A-Za-z0-9_]+)(?::([^}]+))?\}\}").unwrap())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickActionParam {
    pub name: String,
    pub default_value: String,
}

/// Distinct placeholders in order of first appearance.
pub fn parse_quick_action_params(command: &str) -> Vec<QuickActionParam> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in placeholder_re().captures_iter(command) {
        let name = c[1].to_string();
        if seen.insert(name.clone()) {
            out.push(QuickActionParam {
                name,
                default_value: c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default(),
            });
        }
    }
    out
}

pub fn replace_quick_action_params(command: &str, values: &HashMap<String, String>) -> String {
    placeholder_re()
        .replace_all(command, |c: &regex::Captures| match values.get(&c[1]) {
            Some(v) => v.clone(),
            None => c.get(2).map(|m| m.as_str().to_string()).unwrap_or_default(),
        })
        .into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteScriptParam {
    pub name: String,
    pub default_value: String,
}

pub fn quote_shell_arg(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn build_args(params: &[RemoteScriptParam], values: &HashMap<String, String>) -> Vec<String> {
    let mut args = Vec::new();
    for p in params {
        let value = values
            .get(&p.name)
            .unwrap_or(&p.default_value)
            .trim()
            .to_string();
        if value.is_empty() {
            continue;
        }
        if p.name == "args" {
            args.push(value);
        } else if p.name.starts_with('-') {
            args.push(p.name.clone());
            if value != "true" {
                args.push(quote_shell_arg(&value));
            }
        } else {
            args.push(format!("{}={}", p.name, quote_shell_arg(&value)));
        }
    }
    args
}

pub fn build_remote_script_command(
    url: &str,
    params: &[RemoteScriptParam],
    values: &HashMap<String, String>,
) -> String {
    let args = build_args(params, values).join(" ");
    if args.is_empty() {
        format!("curl -sSL {} | bash", quote_shell_arg(url))
    } else {
        format!("curl -sSL {} | bash -s -- {}", quote_shell_arg(url), args)
    }
}

/// Replace `${NAME}` with the supplied value (or the parameter default) for every declared parameter.
pub fn resolve_command_params(
    command: &str,
    params: &[RemoteScriptParam],
    values: &HashMap<String, String>,
) -> String {
    let mut out = command.to_string();
    for p in params {
        let value = values.get(&p.name).unwrap_or(&p.default_value);
        out = out.replace(&format!("${{{}}}", p.name), value);
    }
    out
}

/// The command that is inserted into a terminal for a library entry: remote scripts become `curl | bash`.
pub fn effective_command(
    command: &str,
    url: &str,
    params: &[RemoteScriptParam],
    values: &HashMap<String, String>,
) -> String {
    if url.is_empty() {
        resolve_command_params(command, params, values)
    } else {
        build_remote_script_command(url, params, values)
    }
}

/// Fetch the source of a remote script for the preview tab.
pub async fn fetch_remote_script(url: &str) -> Result<String, String> {
    let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let s = resp.status();
        return Err(format!(
            "HTTP {} {}",
            s.as_u16(),
            s.canonical_reason().unwrap_or("")
        ));
    }
    resp.text().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> RemoteScriptParam {
        RemoteScriptParam {
            name: name.into(),
            default_value: String::new(),
        }
    }
    fn vals(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    const ECS: &str = "https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh";

    #[test]
    fn omits_empty_remote_script_parameters() {
        let c = build_remote_script_command(ECS, &[p("-m"), p("-en"), p("args")], &HashMap::new());
        assert_eq!(
            c,
            "curl -sSL 'https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh' | bash"
        );
    }

    #[test]
    fn passes_non_empty_option_parameters_to_bash_argv() {
        let c = build_remote_script_command(
            ECS,
            &[p("-m"), p("-banup"), p("args")],
            &vals(&[
                ("-m", "1"),
                ("-banup", "true"),
                ("args", "-base -ctype gb5"),
            ]),
        );
        assert_eq!(c, "curl -sSL 'https://raw.githubusercontent.com/spiritLHLS/ecs/main/ecs.sh' | bash -s -- -m '1' -banup -base -ctype gb5");
    }

    #[test]
    fn quotes_parameter_values_passed_as_argv() {
        let c = build_remote_script_command(
            "https://example.com/install.sh",
            &[p("--name"), p("--path")],
            &vals(&[("--name", "mio's app"), ("--path", "/opt/Ops Batch")]),
        );
        assert_eq!(c, "curl -sSL 'https://example.com/install.sh' | bash -s -- --name 'mio'\\''s app' --path '/opt/Ops Batch'");
    }

    #[test]
    fn named_parameters_become_assignments_and_defaults_apply() {
        let params = [RemoteScriptParam {
            name: "VERSION".into(),
            default_value: "1.2".into(),
        }];
        assert!(
            build_remote_script_command("https://x/y.sh", &params, &HashMap::new())
                .ends_with("bash -s -- VERSION='1.2'")
        );
    }

    #[test]
    fn placeholders_parse_once_in_order_with_defaults() {
        let ps = parse_quick_action_params("tail -n {{lines:100}} {{file}} && echo {{lines}}");
        assert_eq!(
            ps,
            vec![
                QuickActionParam {
                    name: "lines".into(),
                    default_value: "100".into()
                },
                QuickActionParam {
                    name: "file".into(),
                    default_value: String::new()
                }
            ]
        );
        assert!(parse_quick_action_params("ls -la").is_empty());
    }

    #[test]
    fn placeholders_are_replaced_with_values_or_fallbacks() {
        let out = replace_quick_action_params(
            "tail -n {{lines:100}} {{file}}",
            &vals(&[("file", "/var/log/syslog")]),
        );
        assert_eq!(out, "tail -n 100 /var/log/syslog");
        let out = replace_quick_action_params("echo {{x:a}}", &vals(&[("x", "")]));
        assert_eq!(
            out, "echo ",
            "an explicit empty value wins over the default"
        );
    }

    #[test]
    fn dollar_brace_parameters_are_substituted_with_defaults() {
        let params = [
            RemoteScriptParam {
                name: "NAME".into(),
                default_value: "web".into(),
            },
            RemoteScriptParam {
                name: "PORT".into(),
                default_value: "80".into(),
            },
        ];
        let v = vals(&[("PORT", "8080")]);
        assert_eq!(
            resolve_command_params("docker logs ${NAME} -p ${PORT} ${NAME}", &params, &v),
            "docker logs web -p 8080 web"
        );
        assert_eq!(
            resolve_command_params("echo $HOME ${UNDECLARED}", &params, &v),
            "echo $HOME ${UNDECLARED}"
        );
    }

    #[test]
    fn effective_command_prefers_the_remote_url() {
        let none = HashMap::new();
        assert_eq!(effective_command("ls", "", &[], &none), "ls");
        assert_eq!(
            effective_command("ignored", "https://x/y.sh", &[], &none),
            "curl -sSL 'https://x/y.sh' | bash"
        );
    }
}
