# Core Storage Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a testable Rust foundation that initializes project-local `.all/` storage, atomically persists a versioned manifest, fingerprints and registers source files, and reports fresh, stale, or missing tracked objects through `dotall init` and `dotall status`.

**Architecture:** Create one modular `dotall-core` library and one thin `dotall-cli` binary. Core owns workspace discovery, safe paths, manifest persistence, BLAKE3 fingerprints, object registration, and status calculation; the CLI only maps commands and renders human or JSON output. This is the first vertical slice from `docs/specs/core-format-architecture.md`; format detection, XLSX parsing, derived artifacts, editing, and MCP are intentionally outside this plan.

**Tech Stack:** Stable Rust, Cargo workspace, `serde`, `serde_json`, `thiserror`, `blake3`, `clap`, `tempfile`, `filetime`, `assert_cmd`, and `predicates`.

---

## Plan boundaries

This plan produces working software by itself:

```text
dotall init [path]
dotall status [path]
dotall --json init [path]
dotall --json status [path]
```

The library additionally exposes `DotallStore::register_source`, which the XLSX
read slice will call when it first opens a workbook.

Subsequent plans cover:

1. XLSX detection, typed model, cache, `inspect`, and `read`.
2. Spreadsheet formula dependency derivation.
3. Transactional XLSX edits, surgical OOXML writing, history, diff, and revert.
4. MCP tools over the stable core interface.

Commit steps below are checkpoints, not authorization. Execute them only if the user
explicitly asks for commits.

## Target file structure

```text
Cargo.toml
crates/
├── dotall-core/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── error.rs
│   │   ├── fingerprint.rs
│   │   ├── lib.rs
│   │   ├── manifest.rs
│   │   ├── status.rs
│   │   ├── workspace.rs
│   │   └── store/
│   │       ├── atomic.rs
│   │       └── mod.rs
│   └── tests/
│       └── store_contract.rs
└── dotall-cli/
    ├── Cargo.toml
    ├── src/
    │   └── main.rs
    └── tests/
        └── commands.rs
```

Each file has one responsibility:

- `workspace.rs`: project root discovery and `.all/` path construction.
- `manifest.rs`: stable serialized state and schema validation.
- `fingerprint.rs`: metadata fast path and stable BLAKE3 hashing.
- `store/atomic.rs`: durable temporary-file write and atomic rename.
- `store/mod.rs`: initialize, open, register, and persist project state.
- `status.rs`: read-only freshness reporting.
- `main.rs`: CLI parsing, output rendering, and exit codes.

### Task 1: Bootstrap the Rust workspace

**Files:**

- Create: `Cargo.toml`
- Create: `crates/dotall-core/Cargo.toml`
- Create: `crates/dotall-core/src/lib.rs`
- Create: `crates/dotall-cli/Cargo.toml`
- Create: `crates/dotall-cli/src/main.rs`
- Modify: `.gitignore`

- [ ] **Step 1: Generate both crates without nested Git repositories**

Run:

```bash
ls
mkdir -p crates
cargo new --lib crates/dotall-core --vcs none
cargo new --bin crates/dotall-cli --vcs none
```

Expected: Cargo creates both packages under the existing repository and does not
create nested `.git/` directories.

- [ ] **Step 2: Create the workspace manifest**

```toml
[workspace]
members = [
    "crates/dotall-core",
    "crates/dotall-cli",
]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
```

- [ ] **Step 3: Link the CLI to core and name the executable `dotall`**

Run:

```bash
cargo add --package dotall-cli dotall-core --path crates/dotall-core
```

Append to `crates/dotall-cli/Cargo.toml`:

```toml
# Add `autobins = false` under the existing `[package]` section.
autobins = false

[[bin]]
name = "dotall"
path = "src/main.rs"
```

Remove the `Cargo.lock` ignore entry from `.gitignore`. This workspace ships
executables, so commit the generated lockfile for reproducible CLI/MCP builds.

- [ ] **Step 4: Add the first workspace smoke test**

Replace `crates/dotall-core/src/lib.rs` with:

```rust
pub const ALL_DIR_NAME: &str = ".all";

#[cfg(test)]
mod tests {
    use super::ALL_DIR_NAME;

    #[test]
    fn all_directory_name_is_stable() {
        assert_eq!(ALL_DIR_NAME, ".all");
    }
}
```

Replace `crates/dotall-cli/src/main.rs` with:

```rust
fn main() {
    println!("dotall");
}
```

- [ ] **Step 5: Verify the workspace**

Run:

```bash
cargo test --workspace
```

Expected: both crates compile and the one core test passes.

- [ ] **Step 6: Optional commit checkpoint**

```bash
git add Cargo.toml crates/dotall-core crates/dotall-cli
git commit -m "build: initialize Dotall Rust workspace"
```

### Task 2: Define workspace paths and structured errors

**Files:**

- Create: `crates/dotall-core/src/error.rs`
- Create: `crates/dotall-core/src/workspace.rs`
- Modify: `crates/dotall-core/src/lib.rs`
- Modify: `crates/dotall-core/Cargo.toml`

- [ ] **Step 1: Add the error dependency**

Run:

```bash
cargo add --package dotall-core thiserror
cargo add --package dotall-core --dev tempfile
```

- [ ] **Step 2: Write failing workspace tests**

Create `crates/dotall-core/src/workspace.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::error::{DotallError, Result};
use crate::ALL_DIR_NAME;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn at(root: impl AsRef<Path>) -> Result<Self> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|source| DotallError::io(root.as_ref(), source))?;
        Ok(Self { root })
    }

    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start
            .as_ref()
            .canonicalize()
            .map_err(|source| DotallError::io(start.as_ref(), source))?;
        let start = if start.is_file() {
            start.parent().map(Path::to_path_buf).ok_or_else(|| {
                DotallError::InvalidWorkspacePath(start.clone())
            })?
        } else {
            start
        };

        for candidate in start.ancestors() {
            if candidate.join(ALL_DIR_NAME).join("manifest.json").is_file() {
                return Ok(Self {
                    root: candidate.to_path_buf(),
                });
            }
        }

        Err(DotallError::WorkspaceNotInitialized(start))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn all_dir(&self) -> PathBuf {
        self.root.join(ALL_DIR_NAME)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.all_dir().join("manifest.json")
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.all_dir().join("objects")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::Workspace;

    #[test]
    fn discovers_workspace_from_nested_directory() {
        let temp = tempdir().expect("tempdir");
        let nested = temp.path().join("a/b");
        fs::create_dir_all(&nested).expect("nested directory");
        fs::create_dir_all(temp.path().join(".all")).expect(".all");
        fs::write(temp.path().join(".all/manifest.json"), "{}").expect("manifest");

        let workspace = Workspace::discover(&nested).expect("workspace");

        assert_eq!(
            workspace.root(),
            temp.path().canonicalize().expect("canonical root")
        );
    }

    #[test]
    fn rejects_directory_outside_a_workspace() {
        let temp = tempdir().expect("tempdir");

        let error = Workspace::discover(temp.path()).expect_err("not initialized");

        assert!(error.to_string().contains("not initialized"));
    }
}
```

Create `crates/dotall-core/src/error.rs`:

```rust
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DotallError>;

#[derive(Debug, Error)]
pub enum DotallError {
    #[error("I/O operation failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("Dotall is not initialized at or above {0}")]
    WorkspaceNotInitialized(PathBuf),

    #[error("invalid workspace path: {0}")]
    InvalidWorkspacePath(PathBuf),

    #[error("invalid source path {path}: {reason}")]
    InvalidSourcePath { path: PathBuf, reason: String },

    #[error("manifest at {path} is invalid: {source}")]
    InvalidManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("unsupported manifest schema {found}; this build supports {supported}")]
    UnsupportedManifestSchema { found: u32, supported: u32 },

    #[error("source changed while it was being hashed: {0}")]
    SourceChangedDuringRead(PathBuf),
}

impl DotallError {
    pub(crate) fn io(path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}
```

The `serde_json` reference will fail until the next command adds the dependency.

- [ ] **Step 3: Run the focused tests and observe the dependency failure**

Run:

```bash
cargo test -p dotall-core workspace
```

Expected: compilation fails because `serde_json` is not yet declared.

- [ ] **Step 4: Add serialization support and export the modules**

Run:

```bash
cargo add --package dotall-core serde_json
```

Replace `crates/dotall-core/src/lib.rs` with:

```rust
mod error;
mod workspace;

pub use error::{DotallError, Result};
pub use workspace::Workspace;

pub const ALL_DIR_NAME: &str = ".all";
```

- [ ] **Step 5: Verify workspace discovery**

Run:

```bash
cargo test -p dotall-core workspace
```

Expected: both workspace tests pass.

- [ ] **Step 6: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): add workspace discovery and errors"
```

### Task 3: Add the versioned manifest and atomic JSON persistence

**Files:**

- Create: `crates/dotall-core/src/fingerprint.rs`
- Create: `crates/dotall-core/src/manifest.rs`
- Create: `crates/dotall-core/src/store/atomic.rs`
- Create: `crates/dotall-core/src/store/mod.rs`
- Modify: `crates/dotall-core/src/lib.rs`
- Modify: `crates/dotall-core/Cargo.toml`

- [ ] **Step 1: Add serialization dependencies**

Run:

```bash
cargo add --package dotall-core serde --features derive
```

- [ ] **Step 2: Define the serialized fingerprint contract**

Create `crates/dotall-core/src/fingerprint.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub size: u64,
    pub modified_unix_nanos: u64,
    pub blake3: String,
}
```

- [ ] **Step 3: Write the manifest model and its tests**

Create `crates/dotall-core/src/manifest.rs`:

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};
use crate::fingerprint::SourceFingerprint;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub objects: BTreeMap<String, TrackedObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedObject {
    pub format_id: String,
    pub fingerprint: SourceFingerprint,
    pub version_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMeta {
    pub schema_version: u32,
    pub format_id: String,
    pub fingerprint: SourceFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalRef {
    pub relative_path: String,
    pub source_hash: String,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            objects: BTreeMap::new(),
        }
    }
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(DotallError::UnsupportedManifestSchema {
                found: self.schema_version,
                supported: MANIFEST_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Manifest, MANIFEST_SCHEMA_VERSION};

    #[test]
    fn default_manifest_uses_current_schema_and_no_objects() {
        let manifest = Manifest::default();

        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert!(manifest.objects.is_empty());
        manifest.validate().expect("valid manifest");
    }

    #[test]
    fn rejects_unknown_schema() {
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION + 1,
            ..Manifest::default()
        };

        let error = manifest.validate().expect_err("unsupported schema");

        assert!(error.to_string().contains("unsupported manifest schema"));
    }
}
```

- [ ] **Step 4: Implement atomic JSON writes**

Create `crates/dotall-core/src/store/atomic.rs`:

```rust
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::Serialize;

use crate::error::{DotallError, Result};

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or_else(|| DotallError::InvalidWorkspacePath(
        path.to_path_buf(),
    ))?;
    fs::create_dir_all(parent).map_err(|source| DotallError::io(parent, source))?;

    let file_name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
        DotallError::InvalidWorkspacePath(path.to_path_buf())
    })?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));

    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| DotallError::io(&temporary, source))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(|source| {
            DotallError::InvalidManifest {
                path: temporary.clone(),
                source,
            }
        })?;
        writer
            .write_all(b"\n")
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .flush()
            .map_err(|source| DotallError::io(&temporary, source))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|source| DotallError::io(&temporary, source))?;

        fs::rename(&temporary, path).map_err(|source| DotallError::io(path, source))?;
        sync_parent(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<()> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| DotallError::io(parent, source))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<()> {
    Ok(())
}
```

Create `crates/dotall-core/src/store/mod.rs`:

```rust
mod atomic;

pub(crate) use atomic::write_json;
```

- [ ] **Step 5: Export the contracts**

Replace `crates/dotall-core/src/lib.rs` with:

```rust
mod error;
mod fingerprint;
mod manifest;
mod store;
mod workspace;

pub use error::{DotallError, Result};
pub use fingerprint::SourceFingerprint;
pub use manifest::{
    Manifest, ObjectMeta, OriginalRef, TrackedObject,
    MANIFEST_SCHEMA_VERSION,
};
pub use workspace::Workspace;

pub const ALL_DIR_NAME: &str = ".all";
```

- [ ] **Step 6: Add an atomic persistence unit test**

Append to `crates/dotall-core/src/store/atomic.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::manifest::Manifest;

    use super::write_json;

    #[test]
    fn writes_complete_json_and_removes_temporary_file() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("manifest.json");

        write_json(&path, &Manifest::default()).expect("write manifest");

        let parsed: Manifest = serde_json::from_slice(
            &fs::read(&path).expect("read manifest"),
        )
        .expect("parse manifest");
        assert_eq!(parsed, Manifest::default());
        assert!(!temp.path().join(format!(
            ".manifest.json.tmp-{}",
            std::process::id()
        )).exists());
    }
}
```

- [ ] **Step 7: Run manifest and atomic-write tests**

Run:

```bash
cargo test -p dotall-core manifest
cargo test -p dotall-core writes_complete_json
```

Expected: all selected tests pass.

- [ ] **Step 8: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): add atomic versioned manifest storage"
```

### Task 4: Implement stable source fingerprints and freshness checks

**Files:**

- Modify: `crates/dotall-core/src/fingerprint.rs`
- Modify: `crates/dotall-core/src/lib.rs`
- Modify: `crates/dotall-core/Cargo.toml`

- [ ] **Step 1: Add BLAKE3 and deterministic mtime testing**

Run:

```bash
cargo add --package dotall-core blake3
cargo add --package dotall-core --dev filetime
```

- [ ] **Step 2: Replace the fingerprint file with tests and implementation**

Replace `crates/dotall-core/src/fingerprint.rs` with:

```rust
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::error::{DotallError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub size: u64,
    pub modified_unix_nanos: u64,
    pub blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    FreshFastPath,
    FreshAfterHash(SourceFingerprint),
    Stale(SourceFingerprint),
}

pub fn fingerprint(path: &Path) -> Result<SourceFingerprint> {
    let before = fs::metadata(path).map_err(|source| DotallError::io(path, source))?;
    let hash = hash_file(path)?;
    let after = fs::metadata(path).map_err(|source| DotallError::io(path, source))?;

    if metadata_tuple(&before)? != metadata_tuple(&after)? {
        return Err(DotallError::SourceChangedDuringRead(path.to_path_buf()));
    }

    Ok(SourceFingerprint {
        size: after.len(),
        modified_unix_nanos: modified_unix_nanos(&after)?,
        blake3: hash,
    })
}

pub fn check_freshness(
    path: &Path,
    expected: &SourceFingerprint,
) -> Result<Freshness> {
    let metadata =
        fs::metadata(path).map_err(|source| DotallError::io(path, source))?;
    if metadata.len() == expected.size
        && modified_unix_nanos(&metadata)? == expected.modified_unix_nanos
    {
        return Ok(Freshness::FreshFastPath);
    }

    let actual = fingerprint(path)?;
    if actual.blake3 == expected.blake3 {
        Ok(Freshness::FreshAfterHash(actual))
    } else {
        Ok(Freshness::Stale(actual))
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| DotallError::io(path, source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| DotallError::io(path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn metadata_tuple(metadata: &Metadata) -> Result<(u64, u64)> {
    Ok((metadata.len(), modified_unix_nanos(metadata)?))
}

fn modified_unix_nanos(metadata: &Metadata) -> Result<u64> {
    let modified = metadata.modified().map_err(|source| {
        DotallError::io("<source metadata>", source)
    })?;
    let duration = modified.duration_since(UNIX_EPOCH).map_err(|_| {
        DotallError::InvalidWorkspacePath("<pre-epoch mtime>".into())
    })?;
    u64::try_from(duration.as_nanos()).map_err(|_| {
        DotallError::InvalidWorkspacePath("<mtime overflow>".into())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use filetime::{set_file_mtime, FileTime};
    use tempfile::tempdir;

    use super::{check_freshness, fingerprint, Freshness};

    #[test]
    fn unchanged_metadata_uses_fast_path() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, b"same").expect("source");
        let expected = fingerprint(&source).expect("fingerprint");

        let result = check_freshness(&source, &expected).expect("freshness");

        assert_eq!(result, Freshness::FreshFastPath);
    }

    #[test]
    fn changed_content_is_stale() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, b"before").expect("source");
        let expected = fingerprint(&source).expect("fingerprint");
        fs::write(&source, b"after and larger").expect("changed source");

        let result = check_freshness(&source, &expected).expect("freshness");

        assert!(matches!(result, Freshness::Stale(_)));
    }

    #[test]
    fn metadata_only_change_rehashes_to_fresh() {
        let temp = tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, b"same").expect("source");
        let expected = fingerprint(&source).expect("fingerprint");
        let changed_time = SystemTime::now() + Duration::from_secs(5);
        set_file_mtime(&source, FileTime::from_system_time(changed_time))
            .expect("set mtime");

        let result = check_freshness(&source, &expected).expect("freshness");

        assert!(matches!(result, Freshness::FreshAfterHash(_)));
    }
}
```

- [ ] **Step 3: Export freshness APIs**

Change the fingerprint export in `crates/dotall-core/src/lib.rs` to:

```rust
pub use fingerprint::{
    check_freshness, fingerprint, Freshness, SourceFingerprint,
};
```

- [ ] **Step 4: Run fingerprint tests**

Run:

```bash
cargo test -p dotall-core fingerprint
```

Expected: the fast-path, stale-content, and metadata-only tests pass.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): fingerprint sources with a metadata fast path"
```

### Task 5: Build the project-local store and status model

**Files:**

- Create: `crates/dotall-core/src/status.rs`
- Replace: `crates/dotall-core/src/store/mod.rs`
- Modify: `crates/dotall-core/src/lib.rs`
- Create: `crates/dotall-core/tests/store_contract.rs`

- [ ] **Step 1: Write the store contract tests**

Create `crates/dotall-core/tests/store_contract.rs`:

```rust
use std::fs;

use dotall_core::{DotallStore, ObjectState};
use tempfile::tempdir;

#[test]
fn init_is_idempotent_and_creates_required_layout() {
    let temp = tempdir().expect("tempdir");

    DotallStore::init(temp.path()).expect("first init");
    DotallStore::init(temp.path()).expect("second init");

    assert!(temp.path().join(".all/manifest.json").is_file());
    assert!(temp.path().join(".all/objects").is_dir());
}

#[test]
fn registered_source_moves_from_fresh_to_stale() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("book.xlsx");
    fs::write(&source, b"initial").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");

    assert!(temp.path().join(".all/objects/book.xlsx/meta.json").is_file());
    assert!(temp.path().join(".all/objects/book.xlsx/original.ref").is_file());
    let fresh = store.status().expect("fresh status");
    assert_eq!(fresh[0].state, ObjectState::FreshFastPath);

    fs::write(&source, b"changed and larger").expect("change source");
    let stale = store.status().expect("stale status");
    assert_eq!(stale[0].state, ObjectState::Stale);
}

#[test]
fn status_reports_deleted_source_as_missing() {
    let temp = tempdir().expect("tempdir");
    let source = temp.path().join("book.xlsx");
    fs::write(&source, b"initial").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("init");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");
    fs::remove_file(source).expect("remove source");

    let status = store.status().expect("status");

    assert_eq!(status[0].state, ObjectState::Missing);
}

#[test]
fn parent_traversal_cannot_escape_the_workspace() {
    let temp = tempdir().expect("tempdir");
    let mut store = DotallStore::init(temp.path()).expect("init");

    let error = store
        .register_source("../outside.xlsx", "xlsx")
        .expect_err("unsafe path");

    assert!(error.to_string().contains("invalid source path"));
}
```

- [ ] **Step 2: Run the contract tests and verify they fail**

Run:

```bash
cargo test -p dotall-core --test store_contract
```

Expected: compilation fails because `DotallStore` and `ObjectState` do not exist.

- [ ] **Step 3: Add the status types**

Create `crates/dotall-core/src/status.rs`:

```rust
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectState {
    FreshFastPath,
    FreshAfterHash,
    Stale,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectStatus {
    pub path: String,
    pub format_id: String,
    pub state: ObjectState,
}
```

- [ ] **Step 4: Implement store initialization, registration, and status**

Replace `crates/dotall-core/src/store/mod.rs` with:

```rust
mod atomic;

use std::fs;
use std::path::{Component, Path};

use crate::error::{DotallError, Result};
use crate::fingerprint::{check_freshness, fingerprint, Freshness};
use crate::manifest::{
    Manifest, ObjectMeta, OriginalRef, TrackedObject,
    MANIFEST_SCHEMA_VERSION,
};
use crate::status::{ObjectState, ObjectStatus};
use crate::workspace::Workspace;

use atomic::write_json;

#[derive(Debug)]
pub struct DotallStore {
    workspace: Workspace,
    manifest: Manifest,
}

impl DotallStore {
    pub fn init(root: impl AsRef<Path>) -> Result<Self> {
        let workspace = Workspace::at(root)?;
        let all_dir = workspace.all_dir();
        if all_dir.exists() && !all_dir.is_dir() {
            return Err(DotallError::InvalidWorkspacePath(all_dir));
        }
        fs::create_dir_all(workspace.objects_dir())
            .map_err(|source| DotallError::io(workspace.objects_dir(), source))?;

        let manifest = if workspace.manifest_path().is_file() {
            load_manifest(&workspace)?
        } else {
            let manifest = Manifest::default();
            write_json(&workspace.manifest_path(), &manifest)?;
            manifest
        };

        Ok(Self {
            workspace,
            manifest,
        })
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let workspace = Workspace::discover(root)?;
        let manifest = load_manifest(&workspace)?;
        Ok(Self {
            workspace,
            manifest,
        })
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn register_source(
        &mut self,
        relative_path: impl AsRef<Path>,
        format_id: impl Into<String>,
    ) -> Result<()> {
        let (key, source) = resolve_source(&self.workspace, relative_path.as_ref())?;
        let fingerprint = fingerprint(&source)?;
        let format_id = format_id.into();
        let version_count = self
            .manifest
            .objects
            .get(&key)
            .map_or(0, |object| object.version_count);

        self.manifest.objects.insert(
            key.clone(),
            TrackedObject {
                format_id: format_id.clone(),
                fingerprint: fingerprint.clone(),
                version_count,
            },
        );

        let object_dir = self.workspace.objects_dir().join(&key);
        for directory in [
            object_dir.join("cache/model"),
            object_dir.join("cache/derived"),
            object_dir.join("cache/views"),
            object_dir.join("state/access"),
            object_dir.join("state/transactions"),
            object_dir.join("state/edits/staging"),
            object_dir.join("state/edits/history/snapshots"),
        ] {
            fs::create_dir_all(&directory)
                .map_err(|source| DotallError::io(&directory, source))?;
        }

        write_json(
            &object_dir.join("meta.json"),
            &ObjectMeta {
                schema_version: MANIFEST_SCHEMA_VERSION,
                format_id,
                fingerprint: fingerprint.clone(),
            },
        )?;
        write_json(
            &object_dir.join("original.ref"),
            &OriginalRef {
                relative_path: key,
                source_hash: fingerprint.blake3,
            },
        )?;
        write_json(&self.workspace.manifest_path(), &self.manifest)
    }

    pub fn status(&self) -> Result<Vec<ObjectStatus>> {
        self.manifest
            .objects
            .iter()
            .map(|(path, object)| {
                let source = self.workspace.root().join(path);
                let state = if !source.is_file() {
                    ObjectState::Missing
                } else {
                    match check_freshness(&source, &object.fingerprint)? {
                        Freshness::FreshFastPath => ObjectState::FreshFastPath,
                        Freshness::FreshAfterHash(_) => {
                            ObjectState::FreshAfterHash
                        }
                        Freshness::Stale(_) => ObjectState::Stale,
                    }
                };
                Ok(ObjectStatus {
                    path: path.clone(),
                    format_id: object.format_id.clone(),
                    state,
                })
            })
            .collect()
    }
}

fn load_manifest(workspace: &Workspace) -> Result<Manifest> {
    let path = workspace.manifest_path();
    let bytes = fs::read(&path).map_err(|source| DotallError::io(&path, source))?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|source| {
            DotallError::InvalidManifest {
                path: path.clone(),
                source,
            }
        })?;
    manifest.validate()?;
    Ok(manifest)
}

fn resolve_source(
    workspace: &Workspace,
    relative: &Path,
) -> Result<(String, std::path::PathBuf)> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
        || relative
            .components()
            .next()
            .is_some_and(|component| component.as_os_str() == ".all")
    {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "path must be relative, remain inside the workspace, and not target .all"
                .to_owned(),
        });
    }

    let key = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    if key.is_empty() || key.len() != relative.components().filter(
        |component| matches!(component, Component::Normal(_))
    ).count() {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "path contains a non-UTF-8 or unsupported component".to_owned(),
        });
    }
    let key = key.join("/");
    let source = workspace.root().join(&key);
    if !source.is_file() {
        return Err(DotallError::InvalidSourcePath {
            path: relative.to_path_buf(),
            reason: "source is not a regular file".to_owned(),
        });
    }
    Ok((key, source))
}
```

- [ ] **Step 5: Export the store and status types**

Replace `crates/dotall-core/src/lib.rs` with:

```rust
mod error;
mod fingerprint;
mod manifest;
mod status;
mod store;
mod workspace;

pub use error::{DotallError, Result};
pub use fingerprint::{
    check_freshness, fingerprint, Freshness, SourceFingerprint,
};
pub use manifest::{
    Manifest, ObjectMeta, OriginalRef, TrackedObject,
    MANIFEST_SCHEMA_VERSION,
};
pub use status::{ObjectState, ObjectStatus};
pub use store::DotallStore;
pub use workspace::Workspace;

pub const ALL_DIR_NAME: &str = ".all";
```

- [ ] **Step 6: Run the store contract**

Run:

```bash
cargo test -p dotall-core --test store_contract
```

Expected: all four contract tests pass.

- [ ] **Step 7: Run all core tests**

Run:

```bash
cargo test -p dotall-core
```

Expected: all unit and integration tests pass.

- [ ] **Step 8: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): initialize and inspect project storage"
```

### Task 6: Add agent-friendly `init` and `status` CLI commands

**Files:**

- Replace: `crates/dotall-cli/src/main.rs`
- Create: `crates/dotall-cli/tests/commands.rs`
- Modify: `crates/dotall-cli/Cargo.toml`

- [ ] **Step 1: Add CLI and test dependencies**

Run:

```bash
cargo add --package dotall-cli clap --features derive
cargo add --package dotall-cli serde --features derive
cargo add --package dotall-cli serde_json
cargo add --package dotall-cli --dev assert_cmd
cargo add --package dotall-cli --dev predicates
cargo add --package dotall-cli --dev tempfile
```

- [ ] **Step 2: Write failing CLI integration tests**

Create `crates/dotall-cli/tests/commands.rs`:

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn init_creates_project_storage() {
    let temp = tempdir().expect("tempdir");

    Command::cargo_bin("dotall")
        .expect("binary")
        .args(["init", temp.path().to_str().expect("UTF-8 path")])
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized Dotall"));

    assert!(temp.path().join(".all/manifest.json").is_file());
}

#[test]
fn json_status_is_machine_readable() {
    let temp = tempdir().expect("tempdir");
    Command::cargo_bin("dotall")
        .expect("binary")
        .args(["init", temp.path().to_str().expect("UTF-8 path")])
        .assert()
        .success();

    let output = Command::cargo_bin("dotall")
        .expect("binary")
        .args([
            "--json",
            "status",
            temp.path().to_str().expect("UTF-8 path"),
        ])
        .output()
        .expect("status output");

    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(value["tracked_count"], 0);
    assert_eq!(value["objects"], serde_json::json!([]));
}

#[test]
fn status_outside_workspace_has_actionable_error() {
    let temp = tempdir().expect("tempdir");

    Command::cargo_bin("dotall")
        .expect("binary")
        .args(["status", temp.path().to_str().expect("UTF-8 path")])
        .assert()
        .failure()
        .stderr(predicate::str::contains("dotall init"));
}
```

- [ ] **Step 3: Run the CLI tests and verify they fail**

Run:

```bash
cargo test -p dotall-cli --test commands
```

Expected: tests fail because the binary does not accept `init`, `status`, or
`--json`.

- [ ] **Step 4: Implement the thin CLI**

Replace `crates/dotall-cli/src/main.rs` with:

```rust
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use dotall_core::{DotallError, DotallStore, ObjectStatus};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "dotall", about = "Agent-native file access and editing")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Serialize)]
struct InitOutput {
    workspace: PathBuf,
    initialized: bool,
}

#[derive(Debug, Serialize)]
struct StatusOutput {
    workspace: PathBuf,
    tracked_count: usize,
    objects: Vec<ObjectStatus>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            render_error(&error, cli.json);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> dotall_core::Result<()> {
    match &cli.command {
        Command::Init { path } => {
            let already_initialized =
                path.join(".all/manifest.json").is_file();
            let store = DotallStore::init(path)?;
            let output = InitOutput {
                workspace: store.workspace().root().to_path_buf(),
                initialized: !already_initialized,
            };
            if cli.json {
                print_json(&output);
            } else if output.initialized {
                println!("Initialized Dotall in {}", output.workspace.display());
            } else {
                println!(
                    "Dotall already initialized in {}",
                    output.workspace.display()
                );
            }
        }
        Command::Status { path } => {
            let store = DotallStore::open(path)?;
            let objects = store.status()?;
            let output = StatusOutput {
                workspace: store.workspace().root().to_path_buf(),
                tracked_count: objects.len(),
                objects,
            };
            if cli.json {
                print_json(&output);
            } else {
                println!(
                    "{} tracked file(s) in {}",
                    output.tracked_count,
                    output.workspace.display()
                );
                for object in output.objects {
                    println!(
                        "{:?}\t{}\t{}",
                        object.state, object.format_id, object.path
                    );
                }
            }
        }
    }
    Ok(())
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!(
            "{{\"error\":\"failed to serialize CLI output: {error}\"}}"
        ),
    }
}

fn render_error(error: &DotallError, json: bool) {
    let next_action = match error {
        DotallError::WorkspaceNotInitialized(_) => {
            "Run `dotall init <workspace>` first."
        }
        _ => "Inspect the path and retry the operation.",
    };
    if json {
        let value = serde_json::json!({
            "error": error.to_string(),
            "retryable": false,
            "next_action": next_action,
        });
        eprintln!("{value}");
    } else {
        eprintln!("error: {error}");
        eprintln!("next: {next_action}");
    }
}
```

- [ ] **Step 5: Run CLI integration tests**

Run:

```bash
cargo test -p dotall-cli --test commands
```

Expected: all three command tests pass.

- [ ] **Step 6: Manually exercise human and JSON output**

Run:

```bash
workspace="$(mktemp -d)"
cargo run -p dotall-cli -- init "$workspace"
cargo run -p dotall-cli -- status "$workspace"
cargo run -p dotall-cli -- --json status "$workspace"
```

Expected:

```text
Initialized Dotall in <temporary path>
0 tracked file(s) in <temporary path>
```

The final command prints a JSON object with `tracked_count: 0` and `objects: []`.

- [ ] **Step 7: Optional commit checkpoint**

```bash
git add crates/dotall-cli
git commit -m "feat(cli): add init and status commands"
```

### Task 7: Align active repository documentation

**Files:**

- Modify: `AGENTS.md`
- Modify: `docs/specs/dotall-overview.md`
- Modify: `docs/specs/xlsx-engine-v0.md`

- [ ] **Step 1: Update the target repository tree**

In all three files, replace the earlier separate cache/model/semantics/views/edit
crate tree with:

```text
crates/
├── dotall-core/    # store, registry, pipeline, read, history, orchestration
├── dotall-xlsx/    # typed XLSX model, processors, views, semantic edits, writer
├── dotall-cli/     # `dotall` binary
└── dotall-mcp/     # stdio MCP binary after the core loop is solid
```

- [ ] **Step 2: Make the evolution rule explicit**

Add this paragraph beside the tree in each architecture-oriented document:

```markdown
Core concerns begin as strict internal modules. Promote one to a separate crate only
when independent dependencies, feature gating, test isolation, or ownership make
the boundary valuable. Do not generalize a universal model or graph from XLSX
alone.
```

- [ ] **Step 3: Align `.all/` examples**

Update active `.all/` examples to place regenerable artifacts under
`cache/{model,derived,views}` and durable records under
`state/{access,transactions,edits}`. Preserve the v0 decisions that storage is
project-local, `.all/` is gitignored, and the original is referenced rather than
copied by default.

- [ ] **Step 4: Verify no active spec retains the superseded crate names**

Run:

```bash
rg "dotall-cache|dotall-model|dotall-semantics|dotall-views|dotall-edit" \
  AGENTS.md docs/specs
```

Expected: no matches.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add AGENTS.md docs/specs
git commit -m "docs: align specs with modular core architecture"
```

### Task 8: Run the foundation quality gate

**Files:**

- Modify only files required to correct failures from these checks.

- [ ] **Step 1: Format the workspace**

Run:

```bash
cargo fmt --all --check
```

Expected: success. If formatting differs, run `cargo fmt --all`, inspect the diff,
then rerun the check.

- [ ] **Step 2: Run every test**

Run:

```bash
cargo test --workspace
```

Expected: all core unit tests, store contract tests, and CLI command tests pass.

- [ ] **Step 3: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: success with no warnings.

- [ ] **Step 4: Verify the CLI behavior once more**

Run:

```bash
workspace="$(mktemp -d)"
cargo run -p dotall-cli -- init "$workspace"
test -f "$workspace/.all/manifest.json"
cargo run -p dotall-cli -- --json status "$workspace"
```

Expected: every command exits zero and JSON status reports no tracked objects.

- [ ] **Step 5: Inspect the final diff**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only the planned workspace, core, CLI, and documentation files changed;
`git diff --check` reports no whitespace errors.

- [ ] **Step 6: Optional final commit checkpoint**

Only when the user explicitly requests commits and earlier checkpoints were not
created:

```bash
git add Cargo.toml crates AGENTS.md docs/specs
git commit -m "feat: establish project-local Dotall storage foundation"
```

## Self-review record

- Spec coverage: this plan covers the first buildable core slice—workspace,
  project-local storage, manifest schema, hashing fast path, cache/state directories,
  source registration, status, structured errors, and CLI/JSON access.
- Deferred scope is explicit and assigned to subsequent plans.
- Type names and signatures are consistent across library, integration tests, and
  CLI.
- Every implementation step includes exact files, code, commands, and expected
  results.
