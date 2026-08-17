use std::fs;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DotallError, Result};
use crate::pipeline::{CachedArtifact, CachedView};
use crate::status::ObjectState;
use crate::store::DotallStore;

/// Workspace search request over cached `.all/` views and models.
///
/// `query` is matched case-insensitively against cached view content and
/// against string values in cached model payloads (including `named_ranges`).
/// `glob` filters the tracked paths that participate in the search; `*` matches
/// a single path segment and `**` matches across segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub glob: Option<String>,
}

/// One matching line from a cached view or string entry from a cached model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub format_id: String,
    pub selector_kind: Option<String>,
    pub selector: Option<String>,
    pub snippet: String,
}

/// Search results split into hits over indexed caches and paths with no cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub not_indexed: Vec<String>,
}

const SNIPPET_MAX: usize = 160;
const MAX_HITS_PER_FILE: usize = 32;
const SELECTOR_KIND_NAMED_RANGES: &str = "named_ranges";

/// Searches cached `.all/` views and models for `request.query` without
/// scanning ZIPs, source bytes, or anything under `state/`. Objects with
/// neither a `cache/model/model.json` nor any `cache/views/*.json` are
/// reported via `not_indexed`. Stale or missing objects are also treated as
/// `not_indexed` (their caches are not served). When `request.glob` is set,
/// tracked paths that do not match the glob are skipped entirely (never
/// reported as `not_indexed`).
pub fn search_store(store: &DotallStore, request: &SearchRequest) -> Result<SearchResults> {
    let query = request.query.trim();
    if query.is_empty() {
        return Err(DotallError::InvalidArgument {
            reason: "search query must not be empty".into(),
        });
    }
    let needle = query.to_ascii_lowercase();
    let glob = request
        .glob
        .as_deref()
        .map(|g| g.trim())
        .filter(|g| !g.is_empty());

    let mut hits = Vec::new();
    let mut not_indexed = Vec::new();

    for status in store.status()? {
        if let Some(pattern) = glob
            && !glob_matches(pattern, &status.path)
        {
            continue;
        }

        if matches!(status.state, ObjectState::Stale | ObjectState::Missing) {
            not_indexed.push(status.path.clone());
            continue;
        }

        let object_dir = store.workspace().objects_dir().join(&status.path);
        let cache_dir = object_dir.join("cache");
        let model_path = cache_dir.join("model/model.json");
        let views_dir = cache_dir.join("views");

        let has_views = views_dir.is_dir()
            && fs::read_dir(&views_dir)
                .map_err(|source| DotallError::io(&views_dir, source))?
                .any(|entry| {
                    entry
                        .map(|e| {
                            e.path()
                                .extension()
                                .and_then(|ext| ext.to_str())
                                .map(|ext| ext.eq_ignore_ascii_case("json"))
                                .unwrap_or(false)
                        })
                        .unwrap_or(false)
                });
        let indexed = model_path.is_file() || has_views;
        if !indexed {
            not_indexed.push(status.path.clone());
            continue;
        }

        let mut remaining = MAX_HITS_PER_FILE;

        if model_path.is_file() {
            search_model(
                &model_path,
                &status.path,
                &status.format_id,
                &needle,
                &mut hits,
                &mut remaining,
            )?;
        }

        if remaining == 0 || !views_dir.is_dir() {
            continue;
        }

        for entry in
            fs::read_dir(&views_dir).map_err(|source| DotallError::io(&views_dir, source))?
        {
            if remaining == 0 {
                break;
            }
            let entry = entry.map_err(|source| DotallError::io(&views_dir, source))?;
            let file_type = entry
                .file_type()
                .map_err(|source| DotallError::io(entry.path(), source))?;
            if file_type.is_symlink() {
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let view: CachedView = match serde_json::from_slice(&bytes) {
                Ok(view) => view,
                Err(_) => continue,
            };

            for snippet in matching_lines(&view.response.content, &needle) {
                if remaining == 0 {
                    break;
                }
                hits.push(SearchHit {
                    path: status.path.clone(),
                    format_id: status.format_id.clone(),
                    selector_kind: None,
                    selector: None,
                    snippet,
                });
                remaining = remaining.saturating_sub(1);
            }
        }
    }

    Ok(SearchResults { hits, not_indexed })
}

fn search_model(
    model_path: &std::path::Path,
    status_path: &str,
    format_id: &str,
    needle: &str,
    hits: &mut Vec<SearchHit>,
    remaining: &mut usize,
) -> Result<()> {
    if *remaining == 0 {
        return Ok(());
    }
    let bytes = fs::read(model_path).map_err(|source| DotallError::io(model_path, source))?;
    let cached: CachedArtifact =
        serde_json::from_slice(&bytes).map_err(|source| DotallError::Serialization {
            context: format!("cached model at {}", model_path.display()),
            source,
        })?;

    walk_model_value(
        &cached.artifact.payload,
        &[],
        status_path,
        format_id,
        needle,
        hits,
        remaining,
    );
    Ok(())
}

fn walk_model_value(
    value: &Value,
    path: &[&str],
    status_path: &str,
    format_id: &str,
    needle: &str,
    hits: &mut Vec<SearchHit>,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }

    if path.last() == Some(&"named_ranges")
        && let Some(entries) = value.as_array()
    {
        for entry in entries {
            if *remaining == 0 {
                return;
            }
            push_named_range_hit(entry, status_path, format_id, needle, hits, remaining);
        }
        return;
    }

    match value {
        Value::String(text) => {
            if text.to_ascii_lowercase().contains(needle) {
                hits.push(SearchHit {
                    path: status_path.to_string(),
                    format_id: format_id.to_string(),
                    selector_kind: None,
                    selector: None,
                    snippet: truncate_snippet(text.trim()),
                });
                *remaining = remaining.saturating_sub(1);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_model_value(item, path, status_path, format_id, needle, hits, remaining);
                if *remaining == 0 {
                    return;
                }
            }
        }
        Value::Object(map) => {
            for (key, child) in map {
                let mut child_path = path.to_vec();
                child_path.push(key.as_str());
                walk_model_value(
                    child,
                    &child_path,
                    status_path,
                    format_id,
                    needle,
                    hits,
                    remaining,
                );
                if *remaining == 0 {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn push_named_range_hit(
    entry: &Value,
    status_path: &str,
    format_id: &str,
    needle: &str,
    hits: &mut Vec<SearchHit>,
    remaining: &mut usize,
) {
    let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
        return;
    };
    let formula = entry.get("formula").and_then(|v| v.as_str()).unwrap_or("");
    let haystack_name = name.to_ascii_lowercase();
    let haystack_formula = formula.to_ascii_lowercase();
    if !haystack_name.contains(needle) && !haystack_formula.contains(needle) {
        return;
    }
    let snippet = truncate_snippet(&format!("{name} \u{2192} {formula}"));
    hits.push(SearchHit {
        path: status_path.to_string(),
        format_id: format_id.to_string(),
        selector_kind: Some(SELECTOR_KIND_NAMED_RANGES.to_string()),
        selector: Some(name.to_string()),
        snippet,
    });
    *remaining = remaining.saturating_sub(1);
}

fn truncate_snippet(value: &str) -> String {
    if value.len() > SNIPPET_MAX {
        value.chars().take(SNIPPET_MAX).collect()
    } else {
        value.to_string()
    }
}

fn matching_lines(content: &str, needle: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in content.lines() {
        if line.to_ascii_lowercase().contains(needle) {
            lines.push(truncate_snippet(line.trim()));
        }
    }
    lines
}

/// Matches a glob pattern against a `/`-separated relative path.
///
/// `*` matches any sequence within a single path segment (no `/`), and `**`
/// matches zero or more whole path segments. A `**` must occupy its own
/// segment to act as a cross-segment wildcard; otherwise its characters are
/// matched literally within the segment.
fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    let path_segments: Vec<&str> = path.split('/').collect();
    glob_match_segments(&pattern_segments, &path_segments)
}

fn glob_match_segments(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    if pattern[0] == "**" {
        for consumed in 0..=path.len() {
            if glob_match_segments(&pattern[1..], &path[consumed..]) {
                return true;
            }
        }
        return false;
    }
    if path.is_empty() {
        return false;
    }
    if segment_matches(pattern[0], path[0]) {
        glob_match_segments(&pattern[1..], &path[1..])
    } else {
        false
    }
}

fn segment_matches(pattern: &str, segment: &str) -> bool {
    let pat = pattern.as_bytes();
    let seg = segment.as_bytes();
    let mut p = 0usize;
    let mut s = 0usize;
    let mut star_p: Option<usize> = None;
    let mut star_s = 0usize;
    while s < seg.len() {
        if p < pat.len() && pat[p] == b'*' {
            star_p = Some(p);
            star_s = s;
            p += 1;
        } else if p < pat.len() && pat[p] == seg[s] {
            p += 1;
            s += 1;
        } else if let Some(sp) = star_p {
            p = sp + 1;
            star_s += 1;
            s = star_s;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

#[cfg(test)]
mod tests {
    use super::{
        SELECTOR_KIND_NAMED_RANGES, SNIPPET_MAX, glob_matches, matching_lines, segment_matches,
    };

    #[test]
    fn matching_lines_finds_case_insensitive_match_and_trims() {
        let snippets = matching_lines("  Commission Rate is 10%  \n", "rate");
        assert_eq!(snippets, vec!["Commission Rate is 10%".to_string()]);
    }

    #[test]
    fn matching_lines_returns_all_matches() {
        let content = "Prior year channel mix stayed at 10%.\nQ3 commission rate: 10%.\nnope\n";
        let snippets = matching_lines(content, "10%");
        assert_eq!(
            snippets,
            vec![
                "Prior year channel mix stayed at 10%.".to_string(),
                "Q3 commission rate: 10%.".to_string(),
            ]
        );
    }

    #[test]
    fn matching_lines_truncates_long_lines_to_snippet_max() {
        let long = "x".repeat(200);
        let line = format!("rate {long}");
        let snippets = matching_lines(&line, "rate");
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].len(), SNIPPET_MAX);
        assert!(snippets[0].starts_with("rate "));
    }

    #[test]
    fn matching_lines_returns_empty_when_no_match() {
        assert!(matching_lines("no match here", "rate").is_empty());
    }

    #[test]
    fn glob_star_matches_one_segment_only() {
        assert!(glob_matches("q3-pack/*", "q3-pack/pack.xlsx"));
        assert!(!glob_matches("q3-pack/*", "other.xlsx"));
        assert!(!glob_matches("q3-pack/*", "q3-pack/nested/pack.xlsx"));
    }

    #[test]
    fn glob_double_star_matches_across_segments() {
        assert!(glob_matches("**", "a/b/c.xlsx"));
        assert!(glob_matches("**/pack.xlsx", "q3-pack/pack.xlsx"));
        assert!(glob_matches("a/**", "a/b/c/d.xlsx"));
        assert!(glob_matches("a/**/c", "a/b/c"));
        assert!(glob_matches("a/**/c", "a/c"));
    }

    #[test]
    fn glob_star_within_segment_matches_partial() {
        assert!(segment_matches("*.xlsx", "pack.xlsx"));
        assert!(!segment_matches("*.xlsx", "pack.csv"));
        assert!(segment_matches("pack*", "package"));
    }

    #[test]
    fn named_range_selector_kind_is_stable_string() {
        assert_eq!(SELECTOR_KIND_NAMED_RANGES, "named_ranges");
    }
}
