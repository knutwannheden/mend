/// The outcome of searching for a chunk's old lines in a file.
#[derive(Debug, PartialEq)]
pub(crate) enum Match {
    One(usize),
    None,
    Many(Vec<usize>),
}

/// Finds every place where the `pattern` lines occur within `lines` at or after `start`.
///
/// Matching is tried with decreasing strictness: exact, ignoring trailing whitespace,
/// ignoring leading and trailing whitespace, and with typographic punctuation folded to
/// ASCII. The strictest level that matches anywhere decides the outcome. When `eof` is
/// true, only a match that ends at the last line counts.
pub(crate) fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Match {
    if pattern.is_empty() {
        return Match::One(start);
    }
    if pattern.len() > lines.len() {
        return Match::None;
    }
    let last = lines.len() - pattern.len();
    let search_start = if eof { last.max(start) } else { start };

    let levels: [fn(&str, &str) -> bool; 4] = [
        |line, pat| line == pat,
        |line, pat| line.trim_end() == pat.trim_end(),
        |line, pat| line.trim() == pat.trim(),
        |line, pat| normalise(line) == normalise(pat),
    ];
    for same in levels {
        let found: Vec<usize> = (search_start..=last)
            .filter(|&i| {
                pattern
                    .iter()
                    .enumerate()
                    .all(|(offset, pat)| same(&lines[i + offset], pat))
            })
            .collect();
        match found.as_slice() {
            [] => {}
            [one] => return Match::One(*one),
            _ => return Match::Many(found),
        }
    }
    Match::None
}

/// Folds typographic dashes, quotes and spaces to their ASCII equivalents, so a patch
/// written in plain ASCII still applies to a file that contains them.
fn normalise(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| match c {
            // Various dash / hyphen code-points → ASCII '-'
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            // Fancy single quotes → '\''
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            // Fancy double quotes → '"'
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            // Non-breaking space and other odd spaces → normal space
            '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
            | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
            | '\u{3000}' => ' ',
            other => other,
        })
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::Match;
    use super::seek_sequence;
    use std::string::ToString;

    fn to_vec(strings: &[&str]) -> Vec<String> {
        strings.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn test_exact_match_finds_sequence() {
        let lines = to_vec(&["foo", "bar", "baz"]);
        let pattern = to_vec(&["bar", "baz"]);
        assert_eq!(
            seek_sequence(&lines, &pattern, /*start*/ 0, /*eof*/ false,),
            Match::One(1)
        );
    }

    #[test]
    fn test_rstrip_match_ignores_trailing_whitespace() {
        let lines = to_vec(&["foo   ", "bar\t\t"]);
        // Pattern omits trailing whitespace.
        let pattern = to_vec(&["foo", "bar"]);
        assert_eq!(
            seek_sequence(&lines, &pattern, /*start*/ 0, /*eof*/ false,),
            Match::One(0)
        );
    }

    #[test]
    fn test_trim_match_ignores_leading_and_trailing_whitespace() {
        let lines = to_vec(&["    foo   ", "   bar\t"]);
        // Pattern omits any additional whitespace.
        let pattern = to_vec(&["foo", "bar"]);
        assert_eq!(
            seek_sequence(&lines, &pattern, /*start*/ 0, /*eof*/ false,),
            Match::One(0)
        );
    }

    #[test]
    fn test_pattern_longer_than_input_returns_none() {
        let lines = to_vec(&["just one line"]);
        let pattern = to_vec(&["too", "many", "lines"]);
        // Should not panic – must return None when pattern cannot possibly fit.
        assert_eq!(
            seek_sequence(&lines, &pattern, /*start*/ 0, /*eof*/ false,),
            Match::None
        );
    }

    #[test]
    fn test_several_matches_are_all_reported() {
        let lines = to_vec(&["foo", "bar", "foo"]);
        let pattern = to_vec(&["foo"]);
        assert_eq!(
            seek_sequence(&lines, &pattern, /*start*/ 0, /*eof*/ false,),
            Match::Many(vec![0, 2])
        );
        assert_eq!(
            seek_sequence(&lines, &pattern, /*start*/ 1, /*eof*/ false,),
            Match::One(2)
        );
    }

    #[test]
    fn test_single_exact_match_wins_over_whitespace_insensitive_matches() {
        let lines = to_vec(&["foo  ", "  foo", "foo"]);
        let pattern = to_vec(&["foo"]);
        assert_eq!(
            seek_sequence(&lines, &pattern, /*start*/ 0, /*eof*/ false,),
            Match::One(2)
        );
    }
}
