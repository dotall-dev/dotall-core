//! Read-only inspection of `.all/` into a tree + honest metrics snapshot.
//!
//! Never mutates the store or filesystem. Metrics are labeled estimates suitable
//! for agent-facing dashboards; they are not audit-grade accounting.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};
use crate::store::DotallStore;

/// One directory or file node under `.all/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VizNode {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub children: Vec<VizNode>,
}

/// Compact history row for the viz git graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VizHistoryEntry {
    pub path: String,
    pub version: u64,
    pub summary: String,
    pub op_count: usize,
    pub timestamp: String,
    /// Previous sequential version on the same file (`None` for v1).
    pub parent: Option<u64>,
    /// When this version reverts an earlier one, the target version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert_of: Option<u64>,
}

/// Aggregate `.all/` footprint and access-derived estimates.
///
/// `estimated_dump_tokens` is a labeled **lower bound**: compressed source file
/// byte length / 4 (same 4-chars-per-token heuristic as [`crate::read::budget`]).
/// It does not unzip packages; that keeps `dotall-core` free of a ZIP dependency
/// for viz. Uncompressed XML/text would usually yield a higher token estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VizMetrics {
    pub all_bytes: u64,
    pub snapshot_part_bytes: u64,
    pub naive_full_copy_bytes: u64,
    pub model_cache_hits: u64,
    pub view_cache_hits: u64,
    pub estimated_tokens_served: u64,
    pub estimated_dump_tokens: u64,
}

/// Full read-only viz payload: tree, history timeline, and metrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VizSnapshot {
    pub tree: VizNode,
    pub history: Vec<VizHistoryEntry>,
    pub metrics: VizMetrics,
}

/// Owned mirror of access-log JSONL fields needed for metrics.
///
/// [`crate::read::AccessRecord`] is borrow-only and Serialize-only; this type
/// deserializes the same lines without mutating anything.
#[derive(Debug, Deserialize)]
struct AccessLogEntry {
    model_cache_hit: bool,
    view_cache_hit: bool,
    #[serde(default)]
    estimated_tokens: Option<usize>,
}

/// Builds a read-only tree + metrics snapshot of `store.workspace().all_dir()`.
pub fn viz_snapshot(store: &DotallStore) -> Result<VizSnapshot> {
    let all_dir = store.workspace().all_dir();
    let tree = walk_tree(&all_dir, &all_dir)?;
    let history = collect_history(store)?;
    let metrics = collect_metrics(store, &all_dir)?;
    Ok(VizSnapshot {
        tree,
        history,
        metrics,
    })
}

fn collect_history(store: &DotallStore) -> Result<Vec<VizHistoryEntry>> {
    let mut entries = Vec::new();
    for status in store.status()? {
        for summary in store.list_history(&status.path)? {
            let record = store.get_history(&status.path, summary.version)?;
            entries.push(VizHistoryEntry {
                path: status.path.clone(),
                version: summary.version,
                summary: summary.summary,
                op_count: summary.op_count,
                timestamp: summary.timestamp,
                parent: (summary.version > 1).then_some(summary.version - 1),
                revert_of: record.revert_of,
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.version.cmp(&b.version)));
    Ok(entries)
}

fn collect_metrics(store: &DotallStore, all_dir: &Path) -> Result<VizMetrics> {
    let all_bytes = sum_file_bytes(all_dir)?;
    let mut snapshot_part_bytes = 0_u64;
    let mut naive_full_copy_bytes = 0_u64;
    let mut estimated_dump_tokens = 0_u64;
    let mut model_cache_hits = 0_u64;
    let mut view_cache_hits = 0_u64;
    let mut estimated_tokens_served = 0_u64;

    for (path, object) in &store.manifest().objects {
        let object_dir = store.workspace().objects_dir().join(path);
        let parts_dir = object_dir.join("state/edits/history/snapshots/parts");
        snapshot_part_bytes = snapshot_part_bytes.saturating_add(sum_file_bytes(&parts_dir)?);

        let source_len =
            source_file_len(store.workspace().root().join(path), object.fingerprint.size)?;
        let versions = object.version_count.max(1);
        naive_full_copy_bytes =
            naive_full_copy_bytes.saturating_add(versions.saturating_mul(source_len));
        estimated_dump_tokens = estimated_dump_tokens.saturating_add(source_len / 4);

        let access_path = object_dir.join("state/access/log.jsonl");
        let access = parse_access_log(&access_path)?;
        model_cache_hits = model_cache_hits.saturating_add(access.model_cache_hits);
        view_cache_hits = view_cache_hits.saturating_add(access.view_cache_hits);
        estimated_tokens_served =
            estimated_tokens_served.saturating_add(access.estimated_tokens_served);
    }

    Ok(VizMetrics {
        all_bytes,
        snapshot_part_bytes,
        naive_full_copy_bytes,
        model_cache_hits,
        view_cache_hits,
        estimated_tokens_served,
        estimated_dump_tokens,
    })
}

fn source_file_len(path: PathBuf, fingerprint_size: u64) -> Result<u64> {
    if !path.exists() {
        return Ok(fingerprint_size);
    }
    if path
        .symlink_metadata()
        .map_err(|source| DotallError::io(&path, source))?
        .file_type()
        .is_symlink()
    {
        return Ok(fingerprint_size);
    }
    if !path.is_file() {
        return Ok(fingerprint_size);
    }
    Ok(fs::metadata(&path)
        .map_err(|source| DotallError::io(&path, source))?
        .len())
}

struct AccessTotals {
    model_cache_hits: u64,
    view_cache_hits: u64,
    estimated_tokens_served: u64,
}

fn parse_access_log(path: &Path) -> Result<AccessTotals> {
    let mut totals = AccessTotals {
        model_cache_hits: 0,
        view_cache_hits: 0,
        estimated_tokens_served: 0,
    };
    if !path.is_file() {
        return Ok(totals);
    }
    if path
        .symlink_metadata()
        .map_err(|source| DotallError::io(path, source))?
        .file_type()
        .is_symlink()
    {
        return Ok(totals);
    }

    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return Ok(totals),
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip corrupt lines so one bad access-log entry cannot fail viz.
        let Ok(entry) = serde_json::from_str::<AccessLogEntry>(line) else {
            continue;
        };
        if entry.model_cache_hit {
            totals.model_cache_hits = totals.model_cache_hits.saturating_add(1);
        }
        if entry.view_cache_hit {
            totals.view_cache_hits = totals.view_cache_hits.saturating_add(1);
        }
        if let Some(tokens) = entry.estimated_tokens {
            totals.estimated_tokens_served =
                totals.estimated_tokens_served.saturating_add(tokens as u64);
        }
    }
    Ok(totals)
}

fn walk_tree(root: &Path, path: &Path) -> Result<VizNode> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".")
        .to_owned();
    let relative = relative_display(root, path);

    let meta = path
        .symlink_metadata()
        .map_err(|source| DotallError::io(path, source))?;
    if meta.file_type().is_symlink() {
        return Ok(VizNode {
            name,
            path: relative,
            bytes: 0,
            children: Vec::new(),
        });
    }

    if meta.is_file() {
        return Ok(VizNode {
            name,
            path: relative,
            bytes: meta.len(),
            children: Vec::new(),
        });
    }

    if !meta.is_dir() {
        return Ok(VizNode {
            name,
            path: relative,
            bytes: 0,
            children: Vec::new(),
        });
    }

    let mut children = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(path)
        .map_err(|source| DotallError::io(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| DotallError::io(path, source))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let child_path = entry.path();
        let child_meta = child_path
            .symlink_metadata()
            .map_err(|source| DotallError::io(&child_path, source))?;
        if child_meta.file_type().is_symlink() {
            continue;
        }
        children.push(walk_tree(root, &child_path)?);
    }

    let bytes = children.iter().map(|child| child.bytes).sum();
    Ok(VizNode {
        name,
        path: relative,
        bytes,
        children,
    })
}

fn sum_file_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let meta = path
        .symlink_metadata()
        .map_err(|source| DotallError::io(path, source))?;
    if meta.file_type().is_symlink() {
        return Ok(0);
    }
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Ok(0);
    }

    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(|source| DotallError::io(path, source))? {
        let entry = entry.map_err(|source| DotallError::io(path, source))?;
        let child = entry.path();
        let child_meta = child
            .symlink_metadata()
            .map_err(|source| DotallError::io(&child, source))?;
        if child_meta.file_type().is_symlink() {
            continue;
        }
        if child_meta.is_file() {
            total = total.saturating_add(child_meta.len());
        } else if child_meta.is_dir() {
            total = total.saturating_add(sum_file_bytes(&child)?);
        }
    }
    Ok(total)
}

fn relative_display(root: &Path, path: &Path) -> String {
    if path == root {
        return ".".into();
    }
    path.strip_prefix(root)
        .map(|stripped| stripped.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}
