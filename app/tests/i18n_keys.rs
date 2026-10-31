//! Every `t!("...")` key used by the app exists in both dictionaries.

use std::path::Path;

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn all_translation_keys_used_in_the_app_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    let re = regex_lite_keys();
    let mut missing = Vec::new();
    let mut seen = 0;
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for key in re(&text) {
            seen += 1;
            for lang in [ops_i18n::Language::ZhCn, ops_i18n::Language::EnUs] {
                if ops_i18n::template(lang, &key) == key {
                    missing.push(format!(
                        "{}: {key} ({})",
                        f.file_name().unwrap().to_string_lossy(),
                        lang.code()
                    ));
                }
            }
        }
    }
    assert!(seen > 50, "scanner found only {seen} keys");
    assert!(
        missing.is_empty(),
        "missing translations:\n{}",
        missing.join("\n")
    );
    // keys returned by label_key() helpers
    for m in opsbatch_app::workbench::Mode::ALL {
        assert_ne!(
            ops_i18n::template(ops_i18n::Language::ZhCn, m.label_key()),
            m.label_key()
        );
    }
}

/// Finds `t!("literal"` occurrences without pulling in a regex dependency.
fn regex_lite_keys() -> impl Fn(&str) -> Vec<String> {
    |text: &str| {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(i) = rest.find("t!(\"") {
            let after = &rest[i + 4..];
            if let Some(end) = after.find('"') {
                let key = &after[..end];
                let before_ok = i == 0
                    || !rest.as_bytes()[i - 1].is_ascii_alphanumeric()
                        && rest.as_bytes()[i - 1] != b'_';
                if before_ok
                    && key.contains('.')
                    && key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                {
                    out.push(key.to_string());
                }
                rest = &after[end..];
            } else {
                break;
            }
        }
        out
    }
}
