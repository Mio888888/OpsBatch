#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLine<'a> {
    pub row: usize,
    pub text: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub row: usize,
    /// UTF-16 free: character column (terminal cells are counted in chars by the caller).
    pub column: usize,
    pub length: usize,
}

/// Case-insensitive, non-overlapping search over terminal lines. Column/length are in `char`s of the lower-cased
/// text — identical to the TS version for BMP text, and stable for CJK.
pub fn find_terminal_search_matches(lines: &[SearchLine<'_>], query: &str) -> Vec<SearchMatch> {
    let needle: Vec<char> = query.trim().to_lowercase().chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for line in lines {
        let hay: Vec<char> = line.text.to_lowercase().chars().collect();
        let mut col = 0usize;
        while col + needle.len() <= hay.len() {
            if hay[col..col + needle.len()] == needle[..] {
                matches.push(SearchMatch { row: line.row, column: col, length: needle.len() });
                col += needle.len();
            } else {
                col += 1;
            }
        }
    }
    matches
}

/// Next/previous index with wraparound (`from_index` is the current match, if any).
pub fn next_search_index(match_count: usize, backwards: bool, from_index: Option<usize>) -> Option<usize> {
    if match_count == 0 {
        return None;
    }
    match from_index.filter(|i| *i < match_count) {
        None => Some(if backwards { match_count - 1 } else { 0 }),
        Some(i) => Some(if backwards { (i + match_count - 1) % match_count } else { (i + 1) % match_count }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(row: usize, text: &str) -> SearchLine<'_> {
        SearchLine { row, text }
    }

    #[test]
    fn finds_matches_case_insensitively_across_rows() {
        let m = find_terminal_search_matches(&[l(3, "Deploy OK"), l(4, "error: service failed"), l(5, "next ERROR marker")], "error");
        assert_eq!(m, vec![SearchMatch { row: 4, column: 0, length: 5 }, SearchMatch { row: 5, column: 5, length: 5 }]);
    }

    #[test]
    fn returns_every_occurrence_on_same_row() {
        let m = find_terminal_search_matches(&[l(8, "foo bar foo")], "foo");
        assert_eq!(m, vec![SearchMatch { row: 8, column: 0, length: 3 }, SearchMatch { row: 8, column: 8, length: 3 }]);
    }

    #[test]
    fn ignores_empty_queries() {
        assert!(find_terminal_search_matches(&[l(1, "anything")], "  ").is_empty());
    }

    #[test]
    fn cjk_columns_are_character_based_and_matches_do_not_overlap() {
        let m = find_terminal_search_matches(&[l(0, "部署成功 部署失败"), l(1, "aaaa")], "部署");
        assert_eq!(m.iter().map(|x| x.column).collect::<Vec<_>>(), vec![0, 5]);
        assert_eq!(find_terminal_search_matches(&[l(1, "aaaa")], "aa").len(), 2);
    }

    #[test]
    fn next_index_wraps_in_both_directions() {
        assert_eq!(next_search_index(0, false, None), None);
        assert_eq!(next_search_index(3, false, None), Some(0));
        assert_eq!(next_search_index(3, true, None), Some(2));
        assert_eq!(next_search_index(3, false, Some(2)), Some(0));
        assert_eq!(next_search_index(3, true, Some(0)), Some(2));
        assert_eq!(next_search_index(3, false, Some(99)), Some(0), "stale index restarts");
    }
}
