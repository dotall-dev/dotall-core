use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};
use crate::pipeline::CachedView;
use crate::store::DotallStore;

/// Workspace search request over cached `.all/` views and models.
///
/// `query` is matched case-insensitively against cached view content. `glob`
/// is reserved for Task 2 and ignored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub glob: Option<String>,
}

/// One matching line from a cached view.
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

/// Searches cached `.all/` views for `request.query` without scanning ZIPs or
/// source bytes. Objects with neither a `cache/model/model.json` nor any
/// `cache/views/*.json` are reported via `not_indexed`.
pub fn search_store(store: &DotallStore, request: &SearchRequest) -> Result<SearchResults> {
    let query = request.query.trim();
    if query.is_empty() {
        return Err(DotallError::InvalidArgument {
            reason: "search query must not be empty".into(),
        });
    }
    let needle = query.to_ascii_lowercase();

    let mut hits = Vec::new();
    let mut not_indexed = Vec::new();

    for status in store.status()? {
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

fn matching_line(content: &str, needle: &str) -> Option<String> {
    for line in content.lines() {
        if line.to_ascii_lowercase().contains(needle) {
            let trimmed = line.trim();
            let snippet = if trimmed.len() > SNIPPET_MAX {
                trimmed.chars().take(SNIPPET_MAX).collect::<String>()
            } else {
                trimmed.to_string()
            };
            return Some(snippet);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{SNIPPET_MAX, matching_line};

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
}
