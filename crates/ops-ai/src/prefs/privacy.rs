//! "Privacy protection": private IPs, configured CIDR ranges and custom patterns are swapped for stable placeholders
//! (`[OB_IP_1]`, `[OB_TEXT_1]`) before text leaves for a model, and swapped back in the answer so commands the model
//! writes still address the real hosts (`AIPrivacySection` / `modelPrivacy.ts`).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, LazyLock, Mutex};

use dashmap::DashMap;

use super::PrivacySettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((a, p)) => (a.trim(), Some(p.trim())),
            None => (text, None),
        };
        let addr: IpAddr = addr.parse().map_err(|_| format!("无效的 IP 地址：{text}"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(|| format!("无效的前缀长度：{text}"))?,
            None => max,
        };
        Ok(Self { addr, prefix })
    }

    pub fn contains(&self, ip: &IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                let mask = if self.prefix == 0 { 0 } else { u32::MAX << (32 - self.prefix as u32) };
                u32::from(a) & mask == u32::from(*b) & mask
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => {
                let mask = if self.prefix == 0 { 0 } else { u128::MAX << (128 - self.prefix as u32) };
                u128::from(a) & mask == u128::from(*b) & mask
            }
            _ => false,
        }
    }
}

/// Every line must be a CIDR (or bare address); returns the first complaint.
pub fn validate_cidrs(lines: &[String]) -> Result<(), String> {
    lines.iter().try_for_each(|l| Cidr::parse(l).map(|_| ()))
}

/// Every pattern must be a valid regular expression.
pub fn validate_patterns(lines: &[String]) -> Result<(), String> {
    for line in lines {
        regex::Regex::new(line).map_err(|e| format!("无效的正则 `{line}`：{e}"))?;
    }
    Ok(())
}

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => a.is_private() || a.is_loopback() || a.is_link_local() || (a.octets()[0] == 100 && (a.octets()[1] & 0xc0) == 64),
        IpAddr::V6(a) => a.is_loopback() || (a.segments()[0] & 0xfe00) == 0xfc00 || (a.segments()[0] & 0xffc0) == 0xfe80,
    }
}

static IPV4: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap());
static IPV6: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[0-9A-Fa-f:]*:[0-9A-Fa-f:]*:[0-9A-Fa-f:.]*").unwrap());
static TOKEN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\[OB_(?:IP|TEXT)_\d+\]").unwrap());

pub struct Redactor {
    private_ips: bool,
    cidrs: Vec<Cidr>,
    patterns: Vec<regex::Regex>,
    forward: HashMap<String, String>,
    reverse: HashMap<String, String>,
    ip_count: usize,
    text_count: usize,
}

impl Redactor {
    /// `None` when the protection is switched off.
    pub fn new(settings: &PrivacySettings) -> Option<Self> {
        if !settings.enabled {
            return None;
        }
        Some(Self {
            private_ips: settings.redact_private_ips,
            cidrs: settings.cidrs.iter().filter_map(|c| Cidr::parse(c).ok()).collect(),
            patterns: settings.patterns.iter().filter(|p| !p.trim().is_empty()).filter_map(|p| regex::Regex::new(p).ok()).collect(),
            forward: HashMap::new(),
            reverse: HashMap::new(),
            ip_count: 0,
            text_count: 0,
        })
    }

    fn token_for(&mut self, original: &str, ip: bool) -> String {
        if let Some(t) = self.forward.get(original) {
            return t.clone();
        }
        let token = if ip {
            self.ip_count += 1;
            format!("[OB_IP_{}]", self.ip_count)
        } else {
            self.text_count += 1;
            format!("[OB_TEXT_{}]", self.text_count)
        };
        self.forward.insert(original.to_string(), token.clone());
        self.reverse.insert(token.clone(), original.to_string());
        token
    }

    fn ip_should_hide(&self, ip: &IpAddr) -> bool {
        (self.private_ips && is_private(ip)) || self.cidrs.iter().any(|c| c.contains(ip))
    }

    pub fn redact(&mut self, text: &str) -> String {
        // (start, end, is_ip); patterns first so a secret that contains an address stays one token
        let mut spans: Vec<(usize, usize, bool)> = Vec::new();
        for re in &self.patterns {
            spans.extend(re.find_iter(text).filter(|m| !m.as_str().is_empty()).map(|m| (m.start(), m.end(), false)));
        }
        let free = |spans: &[(usize, usize, bool)], s: usize, e: usize| spans.iter().all(|(a, b, _)| e <= *a || s >= *b);
        for m in IPV4.find_iter(text) {
            if let Ok(ip) = m.as_str().parse::<Ipv4Addr>() {
                if self.ip_should_hide(&IpAddr::V4(ip)) && free(&spans, m.start(), m.end()) {
                    spans.push((m.start(), m.end(), true));
                }
            }
        }
        for m in IPV6.find_iter(text) {
            if let Ok(ip) = m.as_str().parse::<Ipv6Addr>() {
                if self.ip_should_hide(&IpAddr::V6(ip)) && free(&spans, m.start(), m.end()) {
                    spans.push((m.start(), m.end(), true));
                }
            }
        }
        spans.sort_by_key(|s| s.0);
        // drop spans overlapping an earlier one
        let mut kept: Vec<(usize, usize, bool)> = Vec::new();
        for s in spans {
            if kept.last().is_none_or(|l| s.0 >= l.1) {
                kept.push(s);
            }
        }
        let mut out = String::new();
        let mut cursor = 0;
        for (s, e, ip) in kept {
            out.push_str(&text[cursor..s]);
            let token = self.token_for(&text[s..e], ip);
            out.push_str(&token);
            cursor = e;
        }
        out.push_str(&text[cursor..]);
        out
    }

    /// Puts the originals back wherever a known placeholder appears.
    pub fn restore(&self, text: &str) -> String {
        TOKEN.replace_all(text, |c: &regex::Captures| self.reverse.get(&c[0]).cloned().unwrap_or_else(|| c[0].to_string())).into_owned()
    }

    /// [`restore`](Self::restore) for a growing stream: a trailing, possibly unfinished placeholder is held back.
    pub fn restore_ready(&self, text: &str) -> String {
        let hold = match text.rfind('[') {
            Some(i) => {
                let tail = &text[i..];
                (!tail.contains(']') && ("[OB_".starts_with(tail) || tail.starts_with("[OB_"))).then_some(i)
            }
            None => None,
        };
        self.restore(&text[..hold.unwrap_or(text.len())])
    }
}

static SESSIONS: LazyLock<DashMap<String, Arc<Mutex<Redactor>>>> = LazyLock::new(DashMap::new);

/// The redactor of a conversation (placeholders stay stable across its turns); `None` while protection is off.
pub fn session(conversation_id: &str, settings: &PrivacySettings) -> Option<Arc<Mutex<Redactor>>> {
    if !settings.enabled {
        SESSIONS.remove(conversation_id);
        return None;
    }
    Some(SESSIONS.entry(conversation_id.to_string()).or_insert_with(|| Arc::new(Mutex::new(Redactor::new(settings).expect("enabled")))).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(private: bool, cidrs: &[&str], patterns: &[&str]) -> PrivacySettings {
        PrivacySettings { enabled: true, redact_private_ips: private, cidrs: cidrs.iter().map(|s| s.to_string()).collect(), patterns: patterns.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn cidr_parsing_and_containment() {
        let c = Cidr::parse("192.0.2.0/24").unwrap();
        assert!(c.contains(&"192.0.2.77".parse().unwrap()));
        assert!(!c.contains(&"192.0.3.1".parse().unwrap()));
        assert!(Cidr::parse("2001:db8::/32").unwrap().contains(&"2001:db8:1::5".parse().unwrap()));
        assert!(Cidr::parse("10.1.2.3").unwrap().contains(&"10.1.2.3".parse().unwrap()));
        assert!(Cidr::parse("0.0.0.0/0").unwrap().contains(&"8.8.8.8".parse().unwrap()));
        assert!(Cidr::parse("10.0.0.0/33").is_err());
        assert!(Cidr::parse("nope").is_err());
        assert!(!Cidr::parse("10.0.0.0/8").unwrap().contains(&"::1".parse().unwrap()));
        assert!(validate_cidrs(&["10.0.0.0/8".into(), "x".into()]).is_err());
        assert!(validate_patterns(&["INTERNAL_[A-Z0-9]+".into()]).is_ok());
        assert!(validate_patterns(&["(".into()]).is_err());
    }

    #[test]
    fn disabled_means_no_redactor() {
        assert!(Redactor::new(&PrivacySettings::default()).is_none());
        assert!(session("c-off", &PrivacySettings::default()).is_none());
    }

    #[test]
    fn private_ips_are_swapped_for_stable_tokens_and_restored() {
        let mut r = Redactor::new(&settings(true, &[], &[])).unwrap();
        let out = r.redact("ssh 10.0.0.5 then 10.0.0.5 and 8.8.8.8 via 192.168.1.1");
        assert_eq!(out, "ssh [OB_IP_1] then [OB_IP_1] and 8.8.8.8 via [OB_IP_2]");
        assert_eq!(r.restore("ping [OB_IP_2] && ssh [OB_IP_1]"), "ping 192.168.1.1 && ssh 10.0.0.5");
        assert_eq!(r.restore("[OB_IP_9] unknown"), "[OB_IP_9] unknown");
    }

    #[test]
    fn public_addresses_are_only_hidden_when_configured() {
        let mut r = Redactor::new(&settings(false, &["203.0.113.0/24"], &[])).unwrap();
        assert_eq!(r.redact("a 203.0.113.9 b 10.0.0.1"), "a [OB_IP_1] b 10.0.0.1");
    }

    #[test]
    fn ipv6_addresses_are_handled() {
        let mut r = Redactor::new(&settings(true, &[], &[])).unwrap();
        assert_eq!(r.redact("host fd00::1 and ::1 and 2606:4700::1111"), "host [OB_IP_1] and [OB_IP_2] and 2606:4700::1111");
    }

    #[test]
    fn custom_patterns_win_over_ips_they_contain() {
        let mut r = Redactor::new(&settings(true, &[], &["INTERNAL_[A-Z0-9]+", r"token=\S+"])).unwrap();
        let out = r.redact("INTERNAL_DB at 10.0.0.2 token=abc10.0.0.3");
        assert_eq!(out, "[OB_TEXT_1] at [OB_IP_1] [OB_TEXT_2]");
        assert_eq!(r.restore(&out), "INTERNAL_DB at 10.0.0.2 token=abc10.0.0.3");
    }

    #[test]
    fn broken_patterns_and_cidrs_are_ignored_at_runtime() {
        let mut r = Redactor::new(&settings(false, &["bogus"], &["(", "  "])).unwrap();
        assert_eq!(r.redact("10.0.0.1 text"), "10.0.0.1 text");
    }

    #[test]
    fn streaming_restore_holds_back_unfinished_placeholders() {
        let mut r = Redactor::new(&settings(true, &[], &[])).unwrap();
        r.redact("10.0.0.5");
        assert_eq!(r.restore_ready("connect [OB_IP_1] now"), "connect 10.0.0.5 now");
        assert_eq!(r.restore_ready("connect [OB_"), "connect ");
        assert_eq!(r.restore_ready("connect [OB_IP_1"), "connect ");
        assert_eq!(r.restore_ready("array[0"), "array[0");
        assert_eq!(r.restore_ready("connect ["), "connect ");
        // the restored text only ever grows
        let a = r.restore_ready("go [OB_IP_");
        let b = r.restore_ready("go [OB_IP_1] ok");
        assert!(b.starts_with(&a));
    }

    #[test]
    fn sessions_keep_tokens_stable_across_turns() {
        let s = settings(true, &[], &[]);
        let one = session("conv-stable", &s).unwrap();
        assert_eq!(one.lock().unwrap().redact("10.1.1.1"), "[OB_IP_1]");
        let two = session("conv-stable", &s).unwrap();
        assert_eq!(two.lock().unwrap().redact("10.2.2.2 10.1.1.1"), "[OB_IP_2] [OB_IP_1]");
        assert!(session("conv-stable", &PrivacySettings::default()).is_none());
    }
}
