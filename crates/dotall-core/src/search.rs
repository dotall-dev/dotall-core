use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};
use crate::pipeline::{CachedArtifact, CachedView};
use crate::store::DotallStore;

/// Workspace search request over cached `.all/` views and models.
///
/// `query` is matched case-insensitively against cached view content and
/// against `named_ranges` entries stored in cached models. `glob` filters the
/// tracked paths that participate in the search; `*` matches a single path
/// segment and `**` matches across segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub glob: Option<String>,
}

/// One matching line from a cached view or named-range entry from a cached model.
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
const SELECTOR_KIND_NAMED_RANGES: &str = "named_ranges";

/// Searches cached `.all/` views and models for `request.query` without
/// scanning ZIPs, source bytes, or anything under `state/`. Objects with
/// neither a `cache/model/model.json` nor any `cache/views/*.json` are
/// reported via `not_indexed`. When `request.glob` is set, tracked paths that
/// do not match the glob are skipped entirely (never reported as
/// `not_indexed`).
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

        if model_path.is_file() {
            search_model(
                &model_path,
                &status.path,
                &status.format_id,
                &needle,
                &mut hits,
            )?;
        }

        if !views_dir.is_dir() {
            continue;
        }

        for entry in
            fs::read_dir(&views_dir).map_err(|source| DotallError::io(&views_dir, source))?
        {
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

            let bytes = fs::read(&path).map_err(|source| DotallError::io(&path, source))?;
            let view: CachedView =
                serde_json::from_slice(&bytes).map_err(|source| DotallError::Serialization {
                    context: format!("cached view at {}", path.display()),
                    source,
                })?;

            let snippet = match matching_line(&view.response.content, &needle) {
                Some(snippet) => snippet,
                None => continue,
            };

            hits.push(SearchHit {
                path: status.path.clone(),
                format_id: status.format_id.clone(),
                selector_kind: None,
                selector: None,
                snippet,
            });
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
) -> Result<()> {
    let bytes = fs::read(model_path).map_err(|source| DotallError::io(model_path, source))?;
    let cached: CachedArtifact =
        serde_json::from_slice(&bytes).map_err(|source| DotallError::Serialization {
            context: format!("cached model at {}", model_path.display()),
            source,
        })?;

    let Some(named_ranges) = cached
        .artifact
        .payload
        .get("named_ranges")
        .and_then(|v| v.as_array())
    else {
        return Ok(());
    };

    for entry in named_ranges {
        let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let formula = entry.get("formula").and_then(|v| v.as_str()).unwrap_or("");
        let haystack_name = name.to_ascii_lowercase();
        let haystack_formula = formula.to_ascii_lowercase();
        if !haystack_name.contains(needle) && !haystack_formula.contains(needle) {
            continue;
        }
        let snippet = format!("{name} \u{2192} {formula}");
        let snippet = truncate_snippet(&snippet);
        hits.push(SearchHit {
            path: status_path.to_string(),
            format_id: format_id.to_string(),
            selector_kind: Some(SELECTOR_KIND_NAMED_RANGES.to_string()),
            selector: Some(name.to_string()),
            snippet,
        });
    }
    Ok(())
}

fn truncate_snippet(value: &str) -> String {
    if value.len() > SNIPPET_MAX {
        value.chars().take(SNIPPET_MAX).collect()
    } else {
        value.to_string()
    }
}

fn matching_line(content: &str, needle: &str) -> Option<String> {
    for line in content.lines() {
        if line.to_ascii_lowercase().contains(needle) {
            let trimmed = line.trim();
            return Some(truncate_snippet(trimmed));
        }
    }
    None
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
        SELECTOR_KIND_NAMED_RANGES, SNIPPET_MAX, glob_matches, matching_line, segment_matches,
    };

    #[test]
    fn matching_line_finds_case_insensitive_match_and_trims() {
        let snippet = matching_line("  Commission Rate is 10%  \n", "rate");
        assert_eq!(snippet.as_deref(), Some("Commission Rate is 10%"));
    }

    #[test]
    fn matching_line_truncates_long_lines_to_snippet_max() {
        let long = "x".repeat(200);
        let line = format!("rate {long}");
        let snippet = matching_line(&line, "rate").expect("snippet");
        assert_eq!(snippet.len(), SNIPPET_MAX);
        assert!(snippet.starts_with("rate "));
    }

    #[test]
    fn matching_line_returns_none_when_no_match() {
        assert!(matching_line("no match here", "rate").is_none());
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
