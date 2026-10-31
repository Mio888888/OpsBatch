//! Clickable URLs in terminal output (what xterm's WebLinksAddon provided).

use std::sync::OnceLock;

use regex::Regex;

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"https?://[^\s<>"'`\\^{}|]+"#).unwrap())
}

/// Drop punctuation that usually follows a URL in prose, keeping balanced `)`/`]`.
fn trim_url(url: &str) -> &str {
    let mut end = url.len();
    loop {
        let s = &url[..end];
        let Some(last) = s.chars().last() else { break };
        let strip = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' => s.matches('(').count() < s.matches(')').count(),
            ']' => s.matches('[').count() < s.matches(']').count(),
            _ => false,
        };
        if !strip {
            break;
        }
        end -= last.len_utf8();
    }
    &url[..end]
}

/// The URL covering terminal column `col` of a row. `text` holds one char per non-spacer cell and `columns` the
/// cell column of each char (wide characters occupy two columns).
pub fn url_at(text: &str, columns: &[usize], col: usize) -> Option<String> {
    // char index under the cell: the last char that starts at or before `col`
    let idx = columns.iter().rposition(|&c| c <= col)?;
    let chars: Vec<char> = text.chars().collect();
    if idx >= chars.len() {
        return None;
    }
    for m in url_regex().find_iter(text) {
        let url = trim_url(m.as_str());
        let start = text[..m.start()].chars().count();
        let end = start + url.chars().count();
        if (start..end).contains(&idx) {
            return Some(url.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::url_at;

    fn cols(text: &str) -> Vec<usize> {
        let mut c = 0;
        text.chars()
            .map(|ch| {
                let at = c;
                c += if ch.len_utf8() > 2 { 2 } else { 1 };
                at
            })
            .collect()
    }

    #[test]
    fn finds_the_url_under_the_cell() {
        let t = "see https://example.com/a?b=1, then";
        let c = cols(t);
        assert_eq!(
            url_at(t, &c, 6).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(
            url_at(t, &c, 4 + "https://example.com/a?b=1".len() - 1).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(url_at(t, &c, 1), None, "outside the url");
        assert_eq!(
            url_at(t, &c, 4 + "https://example.com/a?b=1".len()),
            None,
            "the trailing comma is not part of it"
        );
    }

    #[test]
    fn keeps_balanced_brackets_and_trims_prose_punctuation() {
        let wiki = "(https://en.wikipedia.org/wiki/Rust_(language)).";
        let c = cols(wiki);
        assert_eq!(
            url_at(wiki, &c, 10).as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_(language)")
        );
        let q = "go to http://localhost:8080/path!";
        assert_eq!(
            url_at(q, &cols(q), 12).as_deref(),
            Some("http://localhost:8080/path")
        );
        assert_eq!(url_at("ftp://nope", &cols("ftp://nope"), 3), None);
    }

    #[test]
    fn wide_characters_shift_the_columns() {
        let t = "中文 https://a.test/x";
        let c = cols(t); // 中=0 文=2 ' '=4 h=5
        assert_eq!(url_at(t, &c, 5).as_deref(), Some("https://a.test/x"));
        assert_eq!(url_at(t, &c, 1), None);
    }
}
