//! Repository sync page (M6-11) and the startup update (M6-12).

pub mod page;

use ops_library::github::PullResult;

/// (added, updated, deleted, errors, total) of a sync result.
pub fn result_stats(r: &PullResult) -> (usize, usize, usize, usize, usize) {
    (
        r.added.len(),
        r.updated.len(),
        r.deleted.len(),
        r.errors.len(),
        r.added.len() + r.updated.len() + r.deleted.len() + r.errors.len(),
    )
}

/// Lines of the result list in display order, each with its sigil: `+` added, `~` updated, `-` deleted, `!` error.
pub fn result_lines(r: &PullResult) -> Vec<(char, String)> {
    let mut out = Vec::new();
    out.extend(r.added.iter().map(|s| ('+', s.clone())));
    out.extend(r.updated.iter().map(|s| ('~', s.clone())));
    out.extend(r.deleted.iter().map(|s| ('-', s.clone())));
    out.extend(r.errors.iter().map(|s| ('!', s.clone())));
    out
}

/// Toast count after a manual sync: new + updated items.
pub fn change_count(r: &PullResult) -> usize {
    r.added.len() + r.updated.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> PullResult {
        PullResult {
            added: vec!["a".into(), "b".into()],
            updated: vec!["c".into()],
            deleted: vec!["3 items removed".into()],
            errors: vec!["bad".into()],
        }
    }

    #[test]
    fn stats_and_lines_follow_the_original_sections() {
        let r = result();
        assert_eq!(result_stats(&r), (2, 1, 1, 1, 5));
        assert_eq!(change_count(&r), 3);
        let lines = result_lines(&r);
        assert_eq!(lines.iter().map(|(c, _)| *c).collect::<String>(), "++~-!");
        assert_eq!(
            result_stats(&PullResult {
                added: vec![],
                updated: vec![],
                deleted: vec![],
                errors: vec![]
            })
            .4,
            0
        );
    }
}
