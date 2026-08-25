//! FTS5 MATCH-expression construction from raw user queries.
//!
//! Shared by `search_text_bm25` and `search_hybrid` so both keyword and
//! hybrid pipelines parse query text identically.
//!
//! Grammar supported on top of plain terms:
//! - **Phrases**: double-quoted spans become FTS5 phrase queries, e.g.
//!   `"sea of galilee"` matches the exact token sequence. An unterminated
//!   quote runs to end-of-input. Embedded quotes are dropped.
//! - **Prefix**: a bare term ending in `*` keeps its prefix marker, e.g.
//!   `rebuk*` matches `rebuke`, `rebuked`, `rebukes`. The marker is honored
//!   only when attached to a non-empty alphanumeric term; stray `*` are
//!   stripped.
//! - Everything else non-alphanumeric is stripped (legacy sanitizer
//!   behavior), preserving unicode letters/digits.

/// Build an FTS5 boolean MATCH expression; `None` when nothing searchable
/// remains after sanitization.
pub(crate) fn build_fts_match_query(query: &str, require_all: bool) -> Option<String> {
    fn push_unit(units: &mut Vec<String>, buf: &mut String) {
        let t = buf.trim();
        if !t.is_empty() {
            units.push(t.to_string());
        }
        buf.clear();
    }

    let mut units: Vec<String> = Vec::new();
    let mut term = String::new();
    let mut phrase_buf = String::new();
    let mut in_phrase = false;

    for c in query.chars() {
        if in_phrase {
            match c {
                '"' => {
                    let inner = phrase_buf.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !inner.is_empty() {
                        units.push(format!("\"{inner}\""));
                    }
                    phrase_buf.clear();
                    in_phrase = false;
                }
                _ => {
                    if c.is_alphanumeric() || c.is_whitespace() {
                        phrase_buf.push(c);
                    }
                }
            }
            continue;
        }
        match c {
            '"' => {
                push_unit(&mut units, &mut term);
                in_phrase = true;
                phrase_buf.clear();
            }
            '*' => {
                // Prefix marker sticks to the current bare term only.
                if !term.is_empty() && term.chars().all(|ch| ch.is_alphanumeric()) {
                    term.push('*');
                }
            }
            _ if c.is_whitespace() => push_unit(&mut units, &mut term),
            _ if c.is_alphanumeric() => {
                if term.ends_with('*') {
                    // `foo*bar` is invalid FTS5; split into two units.
                    push_unit(&mut units, &mut term);
                }
                term.push(c);
            }
            _ => {} // punctuation stripped silently
        }
    }
    if in_phrase {
        // Unterminated quote: treat the remainder as a phrase.
        let inner = phrase_buf.split_whitespace().collect::<Vec<_>>().join(" ");
        if !inner.is_empty() {
            units.push(format!("\"{inner}\""));
        }
    }
    push_unit(&mut units, &mut term);

    if units.is_empty() {
        return None;
    }
    Some(units.join(if require_all { " AND " } else { " OR " }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_terms_join_with_and() {
        assert_eq!(
            build_fts_match_query("sea galilee", true),
            Some("sea AND galilee".to_string())
        );
    }

    #[test]
    fn plain_terms_join_with_or() {
        assert_eq!(
            build_fts_match_query("sea galilee", false),
            Some("sea OR galilee".to_string())
        );
    }

    #[test]
    fn legacy_sanitizer_still_strips_punctuation() {
        assert_eq!(
            build_fts_match_query("don't panic!", false),
            Some("dont OR panic".to_string())
        );
    }

    #[test]
    fn quoted_span_becomes_phrase() {
        assert_eq!(
            build_fts_match_query("\"sea of galilee\"", true),
            Some("\"sea of galilee\"".to_string())
        );
    }

    #[test]
    fn phrase_mixes_with_bare_terms() {
        assert_eq!(
            build_fts_match_query("storm \"sea of galilee\" boat", true),
            Some("storm AND \"sea of galilee\" AND boat".to_string())
        );
    }

    #[test]
    fn prefix_marker_is_preserved() {
        assert_eq!(
            build_fts_match_query("rebuk*", false),
            Some("rebuk*".to_string())
        );
    }

    #[test]
    fn stray_prefix_marker_on_empty_term_is_dropped() {
        assert_eq!(build_fts_match_query("* *", false), None);
    }

    #[test]
    fn prefix_then_letters_splits_units() {
        assert_eq!(
            build_fts_match_query("rebuk*ed", false),
            Some("rebuk* OR ed".to_string())
        );
    }

    #[test]
    fn unterminated_quote_runs_to_end() {
        assert_eq!(
            build_fts_match_query("\"calmed the sea", true),
            Some("\"calmed the sea\"".to_string())
        );
    }

    #[test]
    fn embedded_quotes_close_and_reopen_phrases() {
        assert_eq!(
            build_fts_match_query("\"he said \"stop\"\"", true),
            Some("\"he said\" AND stop".to_string())
        );
    }

    #[test]
    fn empty_and_punctuation_only_queries_yield_none() {
        assert_eq!(build_fts_match_query("", false), None);
        assert_eq!(build_fts_match_query("  !!!  ??? ", true), None);
        assert_eq!(build_fts_match_query("\"\"", true), None);
    }

    #[test]
    fn unicode_letters_survive() {
        assert_eq!(
            build_fts_match_query("πίστις hope", false),
            Some("πίστις OR hope".to_string())
        );
    }
}
