# XLSX Read Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add automatic XLSX detection, a typed cached workbook model, agent-oriented inspection and range reads, token budgets, and human/JSON CLI commands without reparsing a fresh workbook.

**Architecture:** Extend `dotall-core` with object-safe format contracts, a registry, cache artifact envelopes, read budgeting, and an `Engine` orchestrator. Implement the contracts in a feature-gated `dotall-xlsx` crate using `calamine`; keep workbook types and selectors inside that crate. Compose the registry in `dotall-cli`, so core never depends on XLSX.

**Tech Stack:** Stable Rust, existing core foundation, `calamine`, `serde`, `serde_json`, `clap`, `rust_xlsxwriter` for generated test workbooks, and the existing test stack.

---

## Preconditions and revision rule

Execute `2026-07-18-core-storage-foundation.md` first. Before implementation,
compare this plan's paths and signatures with the resulting code. Update this plan
deliberately where execution changed a foundation API; do not maintain duplicate
compatibility layers for an API that has never shipped.

Commit steps are checkpoints only and require explicit user authorization.

## Target file structure

```text
crates/
├── dotall-core/src/
│   ├── registry/
│   │   ├── mod.rs
│   │   └── types.rs
│   ├── pipeline/
│   │   ├── artifact.rs
│   │   └── mod.rs
│   ├── read/
│   │   ├── budget.rs
│   │   └── mod.rs
│   └── orchestrate/
│       └── mod.rs
├── dotall-xlsx/
│   ├── src/
│   │   ├── detection.rs
│   │   ├── format.rs
│   │   ├── lib.rs
│   │   ├── model.rs
│   │   ├── parser.rs
│   │   ├── projection.rs
│   │   └── selector.rs
│   └── tests/read_contract.rs
└── dotall-cli/
    ├── src/main.rs
    └── tests/xlsx_read.rs
```

### Task 1: Create the XLSX format crate and feature gate

**Files:**

- Create: `crates/dotall-xlsx/Cargo.toml`
- Create: `crates/dotall-xlsx/src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `crates/dotall-cli/Cargo.toml`

- [ ] **Step 1: Generate the crate and dependencies**

Run:

```bash
cargo new --lib crates/dotall-xlsx --vcs none
cargo add --package dotall-xlsx dotall-core --path crates/dotall-core
cargo add --package dotall-xlsx serde --features derive
cargo add --package dotall-xlsx serde_json
cargo add --package dotall-xlsx calamine
cargo add --package dotall-xlsx --dev tempfile
cargo add --package dotall-xlsx --dev rust_xlsxwriter
cargo add --package dotall-cli dotall-xlsx --path crates/dotall-xlsx --optional
```

Add `crates/dotall-xlsx` to workspace members. In
`crates/dotall-cli/Cargo.toml`, add:

```toml
[features]
default = ["xlsx"]
xlsx = ["dep:dotall-xlsx"]
```

- [ ] **Step 2: Add a crate identity smoke test**

Create `crates/dotall-xlsx/src/lib.rs`:

```rust
pub const FORMAT_ID: &str = "xlsx";

#[cfg(test)]
mod tests {
    use super::FORMAT_ID;

    #[test]
    fn format_id_is_stable() {
        assert_eq!(FORMAT_ID, "xlsx");
    }
}
```

- [ ] **Step 3: Verify feature-gated builds**

Run:

```bash
cargo test -p dotall-xlsx
cargo check -p dotall-cli --no-default-features
cargo check -p dotall-cli
```

Expected: all commands succeed; the CLI builds both with and without XLSX.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add Cargo.toml Cargo.lock crates/dotall-xlsx crates/dotall-cli/Cargo.toml
git commit -m "build: add feature-gated XLSX format crate"
```

### Task 2: Define format registry and agent-read contracts

**Files:**

- Create: `crates/dotall-core/src/registry/types.rs`
- Create: `crates/dotall-core/src/registry/mod.rs`
- Create: `crates/dotall-core/src/read/mod.rs`
- Modify: `crates/dotall-core/src/error.rs`
- Modify: `crates/dotall-core/src/lib.rs`

- [ ] **Step 1: Write registry selection tests**

Create `crates/dotall-core/src/registry/mod.rs` with tests first:

```rust
mod types;

use std::path::Path;
use std::sync::Arc;

use crate::{DotallError, Result};

pub use types::*;

pub trait FormatHandler: Send + Sync {
    fn descriptor(&self) -> FormatDescriptor;
    fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore;
    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope>;
    fn inspect(&self, model: &ArtifactEnvelope) -> Result<Inspection>;
    fn read(
        &self,
        model: &ArtifactEnvelope,
        request: &ReadRequest,
    ) -> Result<ReadResponse>;
}

#[derive(Default)]
pub struct FormatRegistry {
    handlers: Vec<Arc<dyn FormatHandler>>,
}

impl FormatRegistry {
    pub fn register(&mut self, handler: Arc<dyn FormatHandler>) {
        self.handlers.push(handler);
    }

    pub fn detect(
        &self,
        path: &Path,
        prefix: &[u8],
    ) -> Result<Arc<dyn FormatHandler>> {
        let probe = DetectionProbe { path, prefix };
        self.handlers
            .iter()
            .map(|handler| (handler.detect(&probe), Arc::clone(handler)))
            .filter(|(score, _)| score.0 > 0)
            .max_by_key(|(score, _)| score.0)
            .map(|(_, handler)| handler)
            .ok_or_else(|| DotallError::UnsupportedFormat(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;

    struct Stub(&'static str, u16);

    impl FormatHandler for Stub {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor {
                id: self.0.into(),
                version: "1".into(),
                capabilities: vec![Capability::Inspect],
            }
        }

        fn detect(&self, _probe: &DetectionProbe<'_>) -> DetectionScore {
            DetectionScore(self.1)
        }

        fn parse(&self, _source: &Path) -> Result<ArtifactEnvelope> {
            unreachable!("selection test does not parse")
        }

        fn inspect(&self, _model: &ArtifactEnvelope) -> Result<Inspection> {
            unreachable!("selection test does not inspect")
        }

        fn read(
            &self,
            _model: &ArtifactEnvelope,
            _request: &ReadRequest,
        ) -> Result<ReadResponse> {
            unreachable!("selection test does not read")
        }
    }

    #[test]
    fn highest_detection_score_wins() {
        let mut registry = FormatRegistry::default();
        registry.register(Arc::new(Stub("weak", 10)));
        registry.register(Arc::new(Stub("strong", 80)));

        let selected = registry
            .detect(Path::new("book.bin"), b"bytes")
            .expect("selected");

        assert_eq!(selected.descriptor().id, "strong");
    }
}
```

- [ ] **Step 2: Add versioned shared envelopes**

Create `crates/dotall-core/src/registry/types.rs`:

```rust
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DetectionScore(pub u16);

pub struct DetectionProbe<'a> {
    pub path: &'a Path,
    pub prefix: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatDescriptor {
    pub id: String,
    pub version: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Inspect,
    ReadFull,
    ReadSelector { kind: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactEnvelope {
    pub format_id: String,
    pub schema_id: String,
    pub schema_version: u32,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inspection {
    pub format_id: String,
    pub summary: serde_json::Value,
    pub capabilities: Vec<Capability>,
    pub suggested_reads: Vec<ReadSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSuggestion {
    pub description: String,
    pub selector: ReadSelector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSelector {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    pub selector: Option<ReadSelector>,
    pub max_tokens: usize,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResponse {
    pub content: String,
    pub estimated_tokens: usize,
    pub truncated: bool,
    pub continuation: Option<String>,
    pub next_actions: Vec<String>,
}
```

- [ ] **Step 3: Add structured format errors**

Add variants to `DotallError` in `crates/dotall-core/src/error.rs`:

```rust
#[error("unsupported file format: {0}")]
UnsupportedFormat(PathBuf),

#[error("format {format_id} rejected artifact schema {schema_id} v{schema_version}")]
ArtifactSchemaMismatch {
    format_id: String,
    schema_id: String,
    schema_version: u32,
},

#[error("unsupported {format_id} capability {capability}; available: {available:?}")]
UnsupportedCapability {
    format_id: String,
    capability: String,
    available: Vec<String>,
},

#[error("{format_id} processing failed for {path}: {message}")]
Format {
    format_id: String,
    path: PathBuf,
    message: String,
},

#[error("failed to serialize {context}: {source}")]
Serialization {
    context: String,
    #[source]
    source: serde_json::Error,
},
```

- [ ] **Step 4: Export modules and run tests**

Add to `crates/dotall-core/src/lib.rs`:

```rust
pub mod registry;
pub mod read;
```

Create `crates/dotall-core/src/read/mod.rs`:

```rust
mod budget;

pub use budget::apply_budget;
```

Create `crates/dotall-core/src/read/budget.rs` temporarily with:

```rust
pub fn apply_budget(content: &str, _max_tokens: usize) -> (String, bool) {
    (content.to_owned(), false)
}
```

Run:

```bash
cargo test -p dotall-core registry
```

Expected: registry selection test passes.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): define format and read contracts"
```

### Task 3: Persist and reuse versioned model artifacts

**Files:**

- Create: `crates/dotall-core/src/pipeline/artifact.rs`
- Create: `crates/dotall-core/src/pipeline/mod.rs`
- Modify: `crates/dotall-core/src/store/mod.rs`
- Modify: `crates/dotall-core/src/lib.rs`
- Create: `crates/dotall-core/tests/artifact_cache.rs`

- [ ] **Step 1: Write a cache round-trip test**

Create `crates/dotall-core/tests/artifact_cache.rs`:

```rust
use std::fs;

use dotall_core::{ArtifactEnvelope, DotallStore};
use tempfile::tempdir;

#[test]
fn model_artifact_round_trips_under_the_tracked_object() {
    let temp = tempdir().expect("tempdir");
    fs::write(temp.path().join("book.xlsx"), b"source").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("store");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");
    let artifact = ArtifactEnvelope {
        format_id: "xlsx".into(),
        schema_id: "workbook".into(),
        schema_version: 1,
        payload: serde_json::json!({"sheets": []}),
    };

    store
        .write_model("book.xlsx", &artifact)
        .expect("write model");
    let loaded = store
        .read_model("book.xlsx")
        .expect("read model")
        .expect("present model");

    assert_eq!(loaded, artifact);
}
```

- [ ] **Step 2: Implement artifact metadata**

Create `crates/dotall-core/src/pipeline/artifact.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::registry::{ArtifactEnvelope, ReadResponse};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedArtifact {
    pub source_hash: String,
    pub producer_id: String,
    pub producer_version: String,
    pub artifact: ArtifactEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedView {
    pub source_hash: String,
    pub renderer_version: String,
    pub request_hash: String,
    pub response: ReadResponse,
}
```

Create `crates/dotall-core/src/pipeline/mod.rs`:

```rust
mod artifact;

pub use artifact::{CachedArtifact, CachedView};
```

- [ ] **Step 3: Add store model-cache methods**

Add to `DotallStore` in `crates/dotall-core/src/store/mod.rs`:

```rust
pub fn write_model(
    &self,
    relative_path: &str,
    artifact: &crate::registry::ArtifactEnvelope,
) -> Result<()> {
    let object = self.manifest.objects.get(relative_path).ok_or_else(|| {
        DotallError::InvalidSourcePath {
            path: relative_path.into(),
            reason: "source is not tracked".into(),
        }
    })?;
    let cached = crate::pipeline::CachedArtifact {
        source_hash: object.fingerprint.blake3.clone(),
        producer_id: artifact.format_id.clone(),
        producer_version: artifact.schema_version.to_string(),
        artifact: artifact.clone(),
    };
    write_json(
        &self
            .workspace
            .objects_dir()
            .join(relative_path)
            .join("cache/model/model.json"),
        &cached,
    )
}

pub fn read_model(
    &self,
    relative_path: &str,
) -> Result<Option<crate::registry::ArtifactEnvelope>> {
    let object = self.manifest.objects.get(relative_path).ok_or_else(|| {
        DotallError::InvalidSourcePath {
            path: relative_path.into(),
            reason: "source is not tracked".into(),
        }
    })?;
    let path = self
        .workspace
        .objects_dir()
        .join(relative_path)
        .join("cache/model/model.json");
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|source| DotallError::io(&path, source))?;
    let cached: crate::pipeline::CachedArtifact =
        serde_json::from_slice(&bytes).map_err(|source| {
            DotallError::InvalidManifest {
                path: path.clone(),
                source,
            }
        })?;
    if cached.source_hash != object.fingerprint.blake3 {
        return Ok(None);
    }
    Ok(Some(cached.artifact))
}

pub fn write_view(
    &self,
    relative_path: &str,
    view: &crate::pipeline::CachedView,
) -> Result<()> {
    let path = self
        .workspace
        .objects_dir()
        .join(relative_path)
        .join("cache/views")
        .join(format!("{}.json", view.request_hash));
    write_json(&path, view)
}

pub fn read_view(
    &self,
    relative_path: &str,
    request_hash: &str,
) -> Result<Option<crate::pipeline::CachedView>> {
    let object = self.manifest.objects.get(relative_path).ok_or_else(|| {
        DotallError::InvalidSourcePath {
            path: relative_path.into(),
            reason: "source is not tracked".into(),
        }
    })?;
    let path = self
        .workspace
        .objects_dir()
        .join(relative_path)
        .join("cache/views")
        .join(format!("{request_hash}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path)
        .map_err(|source| DotallError::io(&path, source))?;
    let cached: crate::pipeline::CachedView =
        serde_json::from_slice(&bytes).map_err(|source| {
            DotallError::InvalidManifest {
                path: path.clone(),
                source,
            }
        })?;
    if cached.source_hash != object.fingerprint.blake3
        || cached.request_hash != request_hash
    {
        return Ok(None);
    }
    Ok(Some(cached))
}
```

- [ ] **Step 4: Export pipeline contracts and run test**

Add to `crates/dotall-core/src/lib.rs`:

```rust
pub mod pipeline;
pub use registry::ArtifactEnvelope;
```

Run:

```bash
cargo test -p dotall-core --test artifact_cache
```

Expected: model artifact cache round-trip passes.

Extend the test with a `CachedView`, then assert an exact request hash hits and a
different hash misses.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): persist source-bound model artifacts"
```

### Task 4: Implement the typed workbook model and parser

**Files:**

- Create: `crates/dotall-xlsx/src/model.rs`
- Create: `crates/dotall-xlsx/src/parser.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`
- Create: `crates/dotall-xlsx/tests/read_contract.rs`

- [ ] **Step 1: Define workbook types**

Create `crates/dotall-xlsx/src/model.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkbookModel {
    pub sheets: Vec<SheetModel>,
    pub named_ranges: Vec<NamedRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetModel {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub cells: Vec<CellModel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellModel {
    pub element_id: String,
    pub address: String,
    pub row: u32,
    pub column: u32,
    pub value: CellValue,
    pub formula: Option<String>,
    pub style_ref: Option<u32>,
    pub number_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CellValue {
    Empty,
    String(String),
    Float(f64),
    Integer(i64),
    Boolean(bool),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedRange {
    pub name: String,
    pub formula: String,
}

pub fn column_name(mut zero_based: u32) -> String {
    let mut result = String::new();
    loop {
        let remainder = (zero_based % 26) as u8;
        result.insert(0, (b'A' + remainder) as char);
        if zero_based < 26 {
            break;
        }
        zero_based = zero_based / 26 - 1;
    }
    result
}
```

- [ ] **Step 2: Write a generated-workbook parser test**

Create `crates/dotall-xlsx/tests/read_contract.rs`:

```rust
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

#[test]
fn parses_values_formulas_and_stable_ids() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("book.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Revenue").expect("sheet name");
    sheet.write_string(0, 0, "Month").expect("header");
    sheet.write_number(1, 0, 10.0).expect("number");
    sheet
        .write_formula(1, 1, "=A2*2")
        .expect("formula");
    workbook.save(&path).expect("save");

    let model = dotall_xlsx::parse_workbook(&path).expect("parse");

    assert_eq!(model.sheets[0].name, "Revenue");
    assert_eq!(model.sheets[0].cells[0].address, "A1");
    assert_eq!(
        model.sheets[0].cells[0].element_id,
        "sheet:Revenue/cell:A1"
    );
    assert_eq!(
        model.sheets[0]
            .cells
            .iter()
            .find(|cell| cell.address == "B2")
            .and_then(|cell| cell.formula.as_deref()),
        Some("=A2*2")
    );
}
```

- [ ] **Step 3: Implement the parser**

Create `crates/dotall-xlsx/src/parser.rs`:

```rust
use std::path::Path;

use calamine::{open_workbook_auto, Data, Reader};
use dotall_core::{DotallError, Result};

use crate::model::{
    column_name, CellModel, CellValue, SheetModel, WorkbookModel,
};
use crate::FORMAT_ID;

pub fn parse_workbook(path: &Path) -> Result<WorkbookModel> {
    let mut workbook = open_workbook_auto(path).map_err(|error| {
        DotallError::Format {
            format_id: FORMAT_ID.into(),
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let names = workbook.sheet_names().to_vec();
    let mut sheets = Vec::with_capacity(names.len());

    for name in names {
        let values = workbook.worksheet_range(&name).map_err(|error| {
            DotallError::Format {
                format_id: FORMAT_ID.into(),
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
        let formulas = workbook.worksheet_formula(&name).ok();
        let (height, width) = values.get_size();
        let mut cells = Vec::new();

        for row in 0..height {
            for column in 0..width {
                let value = values
                    .get((row, column))
                    .map(convert_value)
                    .unwrap_or(CellValue::Empty);
                let formula = formulas
                    .as_ref()
                    .and_then(|range| range.get((row, column)))
                    .map(ToString::to_string)
                    .filter(|formula| !formula.is_empty());
                if value == CellValue::Empty && formula.is_none() {
                    continue;
                }
                let address = format!("{}{}", column_name(column as u32), row + 1);
                cells.push(CellModel {
                    element_id: format!("sheet:{name}/cell:{address}"),
                    address,
                    row: row as u32 + 1,
                    column: column as u32 + 1,
                    value,
                    formula,
                    style_ref: None,
                    number_format: None,
                });
            }
        }
        sheets.push(SheetModel {
            name,
            width: width as u32,
            height: height as u32,
            cells,
        });
    }

    Ok(WorkbookModel {
        sheets,
        named_ranges: Vec::new(),
    })
}

fn convert_value(value: &Data) -> CellValue {
    match value {
        Data::Empty => CellValue::Empty,
        Data::String(value) => CellValue::String(value.clone()),
        Data::Float(value) => CellValue::Float(*value),
        Data::Int(value) => CellValue::Integer(*value),
        Data::Bool(value) => CellValue::Boolean(*value),
        Data::Error(value) => CellValue::Error(value.to_string()),
        Data::DateTime(value) => CellValue::String(value.to_string()),
        Data::DateTimeIso(value) | Data::DurationIso(value) => {
            CellValue::String(value.clone())
        }
    }
}
```

Export from `crates/dotall-xlsx/src/lib.rs`:

```rust
mod model;
mod parser;

pub use model::*;
pub use parser::parse_workbook;

pub const FORMAT_ID: &str = "xlsx";
```

- [ ] **Step 4: Run the parser contract**

Run:

```bash
cargo test -p dotall-xlsx --test read_contract
```

Expected: generated workbook test passes. If the current `calamine` API names a
formula accessor differently, update only `parser.rs` and record the exact selected
API in the plan execution notes.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-xlsx
git commit -m "feat(xlsx): parse workbooks into a typed model"
```

### Task 5: Implement XLSX detection, inspection, selectors, and projections

**Files:**

- Create: `crates/dotall-xlsx/src/detection.rs`
- Create: `crates/dotall-xlsx/src/selector.rs`
- Create: `crates/dotall-xlsx/src/projection.rs`
- Create: `crates/dotall-xlsx/src/format.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`
- Modify: `crates/dotall-xlsx/tests/read_contract.rs`

- [ ] **Step 1: Add behavior tests**

Append to `crates/dotall-xlsx/tests/read_contract.rs`:

```rust
use std::sync::Arc;

use dotall_core::{
    DetectionProbe, FormatHandler, ReadRequest, ReadSelector,
};
use dotall_xlsx::XlsxFormat;

#[test]
fn detects_zip_signature_plus_xlsx_extension() {
    let format = XlsxFormat;
    let score = format.detect(&DetectionProbe {
        path: std::path::Path::new("book.xlsx"),
        prefix: b"PK\x03\x04rest",
    });
    assert!(score.0 >= 80);
}

#[test]
fn range_read_returns_markdown_and_drill_down_hint() {
    let path = make_workbook();
    let format = XlsxFormat;
    let artifact = format.parse(&path).expect("parse");
    let response = format
        .read(
            &artifact,
            &ReadRequest {
                selector: Some(ReadSelector {
                    kind: "range".into(),
                    value: "Revenue!A1:B2".into(),
                }),
                max_tokens: 500,
                continuation: None,
            },
        )
        .expect("read");

    assert!(response.content.contains("| Month |"));
    assert!(response.content.contains("`=A2*2`"));
    assert!(!response.next_actions.is_empty());
}
```

Refactor workbook creation into a `make_workbook() -> PathBuf` helper retained by
the test's `TempDir` fixture so the file lives for the test duration.

- [ ] **Step 2: Implement detection**

Create `crates/dotall-xlsx/src/detection.rs`:

```rust
use dotall_core::{DetectionProbe, DetectionScore};

pub fn score(probe: &DetectionProbe<'_>) -> DetectionScore {
    let extension = probe
        .path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("xlsx"));
    let zip = probe.prefix.starts_with(b"PK\x03\x04");
    DetectionScore(match (extension, zip) {
        (true, true) => 100,
        (true, false) => 20,
        (false, true) => 5,
        (false, false) => 0,
    })
}
```

- [ ] **Step 3: Implement strict A1 range selectors**

Create `crates/dotall-xlsx/src/selector.rs` with:

```rust
use dotall_core::{DotallError, Result};

use crate::FORMAT_ID;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSelector {
    pub sheet: String,
    pub start_row: u32,
    pub start_column: u32,
    pub end_row: u32,
    pub end_column: u32,
}

pub fn parse_range(value: &str) -> Result<RangeSelector> {
    let (sheet, cells) = value.rsplit_once('!').ok_or_else(|| invalid(value))?;
    let (start, end) = cells.split_once(':').unwrap_or((cells, cells));
    let (start_column, start_row) = parse_cell(start).ok_or_else(|| invalid(value))?;
    let (end_column, end_row) = parse_cell(end).ok_or_else(|| invalid(value))?;
    if start_row > end_row || start_column > end_column {
        return Err(invalid(value));
    }
    Ok(RangeSelector {
        sheet: sheet.trim_matches('\'').to_owned(),
        start_row,
        start_column,
        end_row,
        end_column,
    })
}

fn parse_cell(value: &str) -> Option<(u32, u32)> {
    let value = value.replace('$', "");
    let split = value.find(|character: char| character.is_ascii_digit())?;
    let (letters, digits) = value.split_at(split);
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let column = letters.chars().try_fold(0_u32, |total, c| {
        total
            .checked_mul(26)?
            .checked_add(c.to_ascii_uppercase() as u32 - 'A' as u32 + 1)
    })?;
    let row = digits.parse::<u32>().ok()?;
    (row > 0).then_some((column, row))
}

fn invalid(value: &str) -> DotallError {
    DotallError::UnsupportedCapability {
        format_id: FORMAT_ID.into(),
        capability: format!("invalid range selector {value}"),
        available: vec!["range: Sheet1!A1:D20".into()],
    }
}
```

- [ ] **Step 4: Render inspection and Markdown ranges**

Create `crates/dotall-xlsx/src/projection.rs` implementing:

```rust
use dotall_core::{
    Capability, Inspection, ReadResponse, ReadSelector, ReadSuggestion,
};

use crate::model::{CellValue, WorkbookModel};
use crate::selector::RangeSelector;
use crate::FORMAT_ID;

pub fn inspect(model: &WorkbookModel) -> Inspection {
    let formula_count = model
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.cells)
        .filter(|cell| cell.formula.is_some())
        .count();
    Inspection {
        format_id: FORMAT_ID.into(),
        summary: serde_json::json!({
            "sheets": model.sheets.iter().map(|sheet| serde_json::json!({
                "name": sheet.name,
                "rows": sheet.height,
                "columns": sheet.width,
            })).collect::<Vec<_>>(),
            "formula_count": formula_count,
            "named_ranges": model.named_ranges,
        }),
        capabilities: capabilities(),
        suggested_reads: model.sheets.iter().map(|sheet| ReadSuggestion {
            description: format!("Preview {}", sheet.name),
            selector: ReadSelector {
                kind: "range".into(),
                value: format!("{}!A1:{}{}", sheet.name, crate::column_name(
                    sheet.width.saturating_sub(1).min(7)
                ), sheet.height.min(10)),
            },
        }).collect(),
    }
}

pub fn capabilities() -> Vec<Capability> {
    vec![
        Capability::Inspect,
        Capability::ReadFull,
        Capability::ReadSelector { kind: "sheet".into() },
        Capability::ReadSelector { kind: "range".into() },
    ]
}

pub fn render_range(model: &WorkbookModel, range: &RangeSelector) -> ReadResponse {
    let Some(sheet) = model.sheets.iter().find(|sheet| sheet.name == range.sheet)
    else {
        return ReadResponse {
            content: format!("Sheet {:?} was not found.", range.sheet),
            estimated_tokens: 8,
            truncated: false,
            continuation: None,
            next_actions: vec!["Run inspect to list available sheets.".into()],
        };
    };
    let mut output = format!("## {}\n\n", sheet.name);
    for row in range.start_row..=range.end_row {
        output.push('|');
        for column in range.start_column..=range.end_column {
            let cell = sheet.cells.iter().find(|cell| {
                cell.row == row && cell.column == column
            });
            let text = cell.map_or(String::new(), |cell| {
                if let Some(formula) = &cell.formula {
                    format!("`{formula}`")
                } else {
                    display_value(&cell.value)
                }
            });
            output.push_str(&format!(" {} |", text.replace('|', "\\|")));
        }
        output.push('\n');
    }
    ReadResponse {
        estimated_tokens: output.chars().count().div_ceil(4),
        content: output,
        truncated: false,
        continuation: None,
        next_actions: vec![
            "Read a narrower range for cell-level detail.".into(),
            "Read a larger range to continue exploring this sheet.".into(),
        ],
    }
}

pub fn render_sheet(model: &WorkbookModel, name: &str) -> ReadResponse {
    let Some(sheet) = model.sheets.iter().find(|sheet| sheet.name == name) else {
        return ReadResponse {
            content: format!("Sheet {name:?} was not found."),
            estimated_tokens: 8,
            truncated: false,
            continuation: None,
            next_actions: vec!["Run inspect to list available sheets.".into()],
        };
    };
    render_range(model, &RangeSelector {
        sheet: name.to_owned(),
        start_row: 1,
        start_column: 1,
        end_row: sheet.height.max(1),
        end_column: sheet.width.max(1),
    })
}

pub fn render_full(model: &WorkbookModel) -> ReadResponse {
    combine(model.sheets.iter().map(|sheet| {
        render_sheet(model, &sheet.name)
    }))
}

pub fn render_preview(model: &WorkbookModel) -> ReadResponse {
    combine(model.sheets.iter().map(|sheet| {
        render_range(model, &RangeSelector {
            sheet: sheet.name.clone(),
            start_row: 1,
            start_column: 1,
            end_row: sheet.height.clamp(1, 10),
            end_column: sheet.width.clamp(1, 8),
        })
    }))
}

fn combine(responses: impl Iterator<Item = ReadResponse>) -> ReadResponse {
    let content = responses
        .map(|response| response.content)
        .collect::<Vec<_>>()
        .join("\n");
    ReadResponse {
        estimated_tokens: content.chars().count().div_ceil(4),
        content,
        truncated: false,
        continuation: None,
        next_actions: vec![
            "Use a sheet or range selector to narrow the next read.".into(),
        ],
    }
}

fn display_value(value: &CellValue) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::String(value) => value.clone(),
        CellValue::Float(value) => value.to_string(),
        CellValue::Integer(value) => value.to_string(),
        CellValue::Boolean(value) => value.to_string(),
        CellValue::Error(value) => value.clone(),
    }
}
```

- [ ] **Step 5: Implement the format handler**

Create `crates/dotall-xlsx/src/format.rs`:

```rust
use std::path::Path;

use dotall_core::{
    ArtifactEnvelope, DetectionProbe, DetectionScore, DotallError,
    FormatDescriptor, FormatHandler, Inspection, ReadRequest, ReadResponse, Result,
};

use crate::{detection, projection, selector, WorkbookModel, FORMAT_ID};

pub struct XlsxFormat;

impl FormatHandler for XlsxFormat {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            id: FORMAT_ID.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            capabilities: projection::capabilities(),
        }
    }

    fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore {
        detection::score(probe)
    }

    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope> {
        Ok(ArtifactEnvelope {
            format_id: FORMAT_ID.into(),
            schema_id: "xlsx.workbook".into(),
            schema_version: 1,
            payload: serde_json::to_value(crate::parse_workbook(source)).map_err(
                |error| DotallError::Format {
                    format_id: FORMAT_ID.into(),
                    path: source.to_path_buf(),
                    message: error.to_string(),
                },
            )?,
        })
    }

    fn inspect(&self, model: &ArtifactEnvelope) -> Result<Inspection> {
        Ok(projection::inspect(&decode(model)?))
    }

    fn read(
        &self,
        model: &ArtifactEnvelope,
        request: &ReadRequest,
    ) -> Result<ReadResponse> {
        let workbook = decode(model)?;
        let response = match request.selector.as_ref() {
            Some(value) if value.kind == "range" => {
                projection::render_range(
                    &workbook,
                    &selector::parse_range(&value.value)?,
                )
            }
            Some(value) if value.kind == "sheet" => {
                projection::render_sheet(&workbook, &value.value)
            }
            Some(value) if value.kind == "full" => {
                projection::render_full(&workbook)
            }
            None => projection::render_preview(&workbook),
            selector => {
                return Err(DotallError::UnsupportedCapability {
                    format_id: FORMAT_ID.into(),
                    capability: format!("{selector:?}"),
                    available: vec![
                        "range: Sheet1!A1:D20".into(),
                        "sheet: Sheet1".into(),
                        "full".into(),
                    ],
                });
            }
        };
        Ok(response)
    }
}

fn decode(model: &ArtifactEnvelope) -> Result<WorkbookModel> {
    if model.format_id != FORMAT_ID
        || model.schema_id != "xlsx.workbook"
        || model.schema_version != 1
    {
        return Err(DotallError::ArtifactSchemaMismatch {
            format_id: model.format_id.clone(),
            schema_id: model.schema_id.clone(),
            schema_version: model.schema_version,
        });
    }
    serde_json::from_value(model.payload.clone()).map_err(|error| {
        DotallError::Format {
            format_id: FORMAT_ID.into(),
            path: "<cached model>".into(),
            message: error.to_string(),
        }
    })
}
```

Export all modules from `lib.rs`.

- [ ] **Step 6: Run XLSX read tests**

Run:

```bash
cargo test -p dotall-xlsx --test read_contract
```

Expected: detection, parsing, inspection, range, sheet, and full read tests pass.

- [ ] **Step 7: Optional commit checkpoint**

```bash
git add crates/dotall-xlsx
git commit -m "feat(xlsx): expose agent-oriented workbook reads"
```

### Task 6: Enforce token budgets and continuations in core

**Files:**

- Replace: `crates/dotall-core/src/read/budget.rs`
- Modify: `crates/dotall-xlsx/src/format.rs`
- Modify: `crates/dotall-xlsx/tests/read_contract.rs`

- [ ] **Step 1: Write budget tests**

Replace `crates/dotall-core/src/read/budget.rs` with:

```rust
pub fn apply_budget(
    content: &str,
    max_tokens: usize,
    offset: usize,
) -> (String, bool, Option<String>) {
    let max_chars = max_tokens.saturating_mul(4);
    let remaining = content.get(offset..).unwrap_or_default();
    if remaining.chars().count() <= max_chars {
        return (remaining.to_owned(), false, None);
    }

    let mut end = 0;
    for (index, character) in remaining.char_indices() {
        if index >= max_chars {
            break;
        }
        end = index + character.len_utf8();
    }
    let preferred = remaining[..end]
        .rfind('\n')
        .filter(|position| *position > max_chars / 2)
        .unwrap_or(end);
    let next = offset + preferred;
    (
        remaining[..preferred].to_owned(),
        true,
        Some(next.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::apply_budget;

    #[test]
    fn truncates_at_a_line_boundary_and_returns_cursor() {
        let content = "row one\nrow two\nrow three\n";

        let (first, truncated, cursor) = apply_budget(content, 3, 0);
        let (second, _, _) = apply_budget(
            content,
            20,
            cursor
                .as_deref()
                .expect("cursor")
                .parse()
                .expect("numeric cursor"),
        );

        assert!(truncated);
        assert_eq!(format!("{first}{second}"), content);
    }
}
```

- [ ] **Step 2: Apply budgets after format rendering**

At the end of `XlsxFormat::read`, pass the complete projection to:

```rust
let offset = request
    .continuation
    .as_deref()
    .unwrap_or("0")
    .parse::<usize>()
    .map_err(|_| DotallError::UnsupportedCapability {
        format_id: FORMAT_ID.into(),
        capability: "invalid continuation cursor".into(),
        available: vec!["reuse the cursor returned by the previous read".into()],
    })?;
let (content, truncated, continuation) =
    dotall_core::read::apply_budget(
        &response.content,
        request.max_tokens,
        offset,
    );
response.content = content;
response.estimated_tokens = response.content.chars().count().div_ceil(4);
response.truncated = truncated;
response.continuation = continuation;
```

Keep rendering format-owned and budgeting core-owned.

- [ ] **Step 3: Verify lossless continuation**

Add an XLSX test that reads a range with `max_tokens: 10`, follows every returned
cursor, concatenates content, and equals the unlimited projection.

Run:

```bash
cargo test -p dotall-core read::budget
cargo test -p dotall-xlsx --test read_contract
```

Expected: all budget and continuation tests pass.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add crates/dotall-core crates/dotall-xlsx
git commit -m "feat(core): enforce read budgets with continuations"
```

### Task 7: Orchestrate detect, cold parse, and warm cache hit

**Files:**

- Create: `crates/dotall-core/src/orchestrate/mod.rs`
- Modify: `crates/dotall-core/src/lib.rs`
- Create: `crates/dotall-core/tests/read_engine.rs`

- [ ] **Step 1: Write a counting-handler cache test**

Create `crates/dotall-core/tests/read_engine.rs` with a `CountingFormat` that stores
an `AtomicUsize` parse count, detects a `.stub` file, returns a fixed model, and
implements fixed inspection/read responses. Test:

```rust
#[test]
fn second_inspection_uses_cached_model_without_reparse() {
    let fixture = EngineFixture::new();

    fixture.engine.inspect("sample.stub").expect("cold inspect");
    fixture.engine.inspect("sample.stub").expect("warm inspect");

    assert_eq!(fixture.parse_count.load(Ordering::SeqCst), 1);
}
```

The fixture initializes a store, writes `sample.stub`, and registers the counting
handler in the registry.

- [ ] **Step 2: Implement the engine**

Create `crates/dotall-core/src/orchestrate/mod.rs`:

```rust
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::registry::{
    FormatRegistry, Inspection, ReadRequest, ReadResponse,
};
use crate::{DotallError, DotallStore, Result};

#[derive(Debug, serde::Serialize)]
pub struct InspectResult {
    pub source_hash: String,
    pub model_cache_hit: bool,
    pub inspection: Inspection,
}

#[derive(Debug, serde::Serialize)]
pub struct FileReadResult {
    pub source_hash: String,
    pub model_cache_hit: bool,
    pub view_cache_hit: bool,
    pub response: ReadResponse,
}

pub struct Engine {
    store: DotallStore,
    registry: FormatRegistry,
}

impl Engine {
    pub fn new(store: DotallStore, registry: FormatRegistry) -> Self {
        Self { store, registry }
    }

    pub fn inspect(&mut self, relative: &str) -> Result<InspectResult> {
        let (handler, model, model_cache_hit, source_hash) =
            self.model(relative)?;
        Ok(InspectResult {
            source_hash,
            model_cache_hit,
            inspection: handler.inspect(&model)?,
        })
    }

    pub fn read(
        &mut self,
        relative: &str,
        request: &ReadRequest,
    ) -> Result<FileReadResult> {
        let (handler, model, model_cache_hit, source_hash) =
            self.model(relative)?;
        let renderer_version = handler.descriptor().version;
        let request_bytes = serde_json::to_vec(&(
            &renderer_version,
            request,
        ))
        .map_err(|source| DotallError::Serialization {
            context: "read request cache key".into(),
            source,
        })?;
        let request_hash =
            blake3::hash(&request_bytes).to_hex().to_string();
        if let Some(view) = self.store.read_view(relative, &request_hash)? {
            if view.renderer_version == renderer_version {
                return Ok(FileReadResult {
                    source_hash,
                    model_cache_hit,
                    view_cache_hit: true,
                    response: view.response,
                });
            }
        }
        let response = handler.read(&model, request)?;
        self.store.write_view(
            relative,
            &crate::pipeline::CachedView {
                source_hash: source_hash.clone(),
                renderer_version,
                request_hash,
                response: response.clone(),
            },
        )?;
        Ok(FileReadResult {
            source_hash,
            model_cache_hit,
            view_cache_hit: false,
            response,
        })
    }

    fn model(
        &mut self,
        relative: &str,
    ) -> Result<(
        std::sync::Arc<dyn crate::registry::FormatHandler>,
        crate::registry::ArtifactEnvelope,
        bool,
        String,
    )> {
        let source = self.store.workspace().root().join(relative);
        let mut file =
            File::open(&source).map_err(|error| DotallError::io(&source, error))?;
        let mut prefix = [0_u8; 16];
        let read =
            file.read(&mut prefix).map_err(|error| DotallError::io(&source, error))?;
        let handler = self.registry.detect(Path::new(relative), &prefix[..read])?;

        let tracked = self.store.manifest().objects.contains_key(relative);
        if tracked {
            let state = self
                .store
                .status()?
                .into_iter()
                .find(|object| object.path == relative)
                .map(|object| object.state);
            if matches!(
                state,
                Some(crate::ObjectState::FreshFastPath)
                    | Some(crate::ObjectState::FreshAfterHash)
            ) {
                if let Some(model) = self.store.read_model(relative)? {
                    let source_hash = self.store.manifest().objects[relative]
                        .fingerprint
                        .blake3
                        .clone();
                    return Ok((handler, model, true, source_hash));
                }
            }
        }

        self.store
            .register_source(relative, handler.descriptor().id.clone())?;
        let model = handler.parse(&source)?;
        self.store.write_model(relative, &model)?;
        let source_hash = self.store.manifest().objects[relative]
            .fingerprint
            .blake3
            .clone();
        Ok((handler, model, false, source_hash))
    }
}
```

The stale-source test must overwrite the fixture and assert parse count increments
from one to two.

- [ ] **Step 3: Export and verify**

Add:

```rust
pub mod orchestrate;
pub use orchestrate::{Engine, FileReadResult, InspectResult};
```

Run:

```bash
cargo test -p dotall-core --test read_engine
```

Expected: cold/warm and stale-reparse tests pass.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): orchestrate cached format reads"
```

### Task 8: Record read provenance

**Files:**

- Create: `crates/dotall-core/src/read/access.rs`
- Modify: `crates/dotall-core/src/read/mod.rs`
- Modify: `crates/dotall-core/src/store/mod.rs`
- Modify: `crates/dotall-core/src/orchestrate/mod.rs`
- Modify: `crates/dotall-core/Cargo.toml`
- Modify: `crates/dotall-core/tests/read_engine.rs`

- [ ] **Step 1: Add a cross-process access-log lock**

Run:

```bash
cargo add --package dotall-core fs2
```

- [ ] **Step 2: Define access records**

Create `read/access.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessRecord {
    pub timestamp_unix_ms: u64,
    pub operation: String,
    pub selector: Option<String>,
    pub source_hash: String,
    pub model_cache_hit: bool,
    pub view_cache_hit: bool,
    pub estimated_tokens: usize,
    pub truncated: bool,
}

pub fn now_unix_ms() -> crate::Result<u64> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| crate::DotallError::InvalidWorkspacePath(
            "<system clock before Unix epoch>".into(),
        ))?;
    u64::try_from(duration.as_millis()).map_err(|_| {
        crate::DotallError::InvalidWorkspacePath(
            "<system clock overflow>".into(),
        )
    })
}
```

Export `AccessRecord` and `now_unix_ms` from `read/mod.rs`.

- [ ] **Step 3: Append one durable JSON line**

Add `DotallStore::append_access(relative, record)`. It must:

1. acquire an exclusive `fs2` lock on `state/access/log.lock`;
2. serialize the full record before opening the log;
3. append the JSON bytes and one newline to `state/access/log.jsonl` in one write;
4. call `sync_data`;
5. release the lock.

Return serialization and I/O failures; do not silently discard audit data.

- [ ] **Step 4: Record inspect and read results**

Change `Engine::inspect` to append an `operation: "inspect"` record after producing
the result. Change `Engine::read` to append:

```rust
AccessRecord {
    timestamp_unix_ms: now_unix_ms()?,
    operation: "read".into(),
    selector: request.selector.as_ref().map(|selector| {
        format!("{}:{}", selector.kind, selector.value)
    }),
    source_hash: source_hash.clone(),
    model_cache_hit,
    view_cache_hit,
    estimated_tokens: response.estimated_tokens,
    truncated: response.truncated,
}
```

Only return the result after the record is durable.

- [ ] **Step 5: Test cold/warm provenance**

Call inspect twice and the same read twice. Parse each JSONL line as `AccessRecord`
and assert:

- four records exist in order;
- first inspection has `model_cache_hit: false`;
- second inspection and read have `model_cache_hit: true`;
- the second identical read has `view_cache_hit: true`;
- the read records selector and estimated tokens.

Run:

```bash
cargo test -p dotall-core --test read_engine provenance
```

Expected: provenance test passes.

- [ ] **Step 6: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): record durable read provenance"
```

### Task 9: Expose `inspect` and `read` through the CLI

**Files:**

- Modify: `crates/dotall-cli/src/main.rs`
- Create: `crates/dotall-cli/tests/xlsx_read.rs`

- [ ] **Step 1: Add CLI tests**

Generate an XLSX workbook in a temporary workspace, initialize Dotall, then assert:

```rust
Command::cargo_bin("dotall")
    .expect("binary")
    .args([
        "--json",
        "inspect",
        workbook.to_str().expect("path"),
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("\"formula_count\": 1"));

Command::cargo_bin("dotall")
    .expect("binary")
    .args([
        "read",
        workbook.to_str().expect("path"),
        "--range",
        "Revenue!A1:B2",
        "--max-tokens",
        "500",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("| Month |"));
```

- [ ] **Step 2: Compose the default registry**

Add:

```rust
fn registry() -> dotall_core::registry::FormatRegistry {
    let mut registry = dotall_core::registry::FormatRegistry::default();
    #[cfg(feature = "xlsx")]
    registry.register(std::sync::Arc::new(dotall_xlsx::XlsxFormat));
    registry
}
```

Add `Inspect { file }` and `Read { file, range, sheet, full, max_tokens,
continuation }` subcommands. Discover/open the workspace from the file path, convert
the requested scope to `ReadSelector`, call `Engine`, and serialize the returned
inspection or read response directly in JSON mode.

- [ ] **Step 3: Make unsupported builds actionable**

With `--no-default-features`, attempting to inspect XLSX must exit nonzero and say:

```text
unsupported file format: <path>
next: install or build Dotall with XLSX support
```

Add the suggested action based on `UnsupportedFormat`.

- [ ] **Step 4: Verify CLI and cache hit**

Run:

```bash
cargo test -p dotall-cli --test xlsx_read
workspace="$(mktemp -d)"
cargo run -p dotall-cli -- init "$workspace"
# Generate or copy a test workbook to "$workspace/book.xlsx".
cargo run -p dotall-cli -- inspect "$workspace/book.xlsx"
cargo run -p dotall-cli -- inspect "$workspace/book.xlsx"
```

Expected: both inspections return the same summary and the second run reads
`.all/objects/book.xlsx/cache/model/model.json` without reparsing.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-cli
git commit -m "feat(cli): inspect and read XLSX workbooks"
```

### Task 10: Quality and fidelity gate

**Files:**

- Modify only files needed to correct failed checks.

- [ ] **Step 1: Run all verification**

Run:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p dotall-cli --no-default-features
git diff --check
```

Expected: every command succeeds.

- [ ] **Step 2: Verify cache provenance manually**

After one inspection, record the model file modification time, inspect again, and
assert it did not change:

```bash
model="$workspace/.all/objects/book.xlsx/cache/model/model.json"
before="$(stat -f %m "$model")"
cargo run -p dotall-cli -- inspect "$workspace/book.xlsx"
after="$(stat -f %m "$model")"
test "$before" = "$after"
```

Expected: exit zero, proving the warm read did not rewrite or reparse the model.

- [ ] **Step 3: Optional final commit checkpoint**

```bash
git add Cargo.toml Cargo.lock crates
git commit -m "feat: add cached agent-native XLSX reads"
```

## Self-review record

- Covers format detection, object-safe registration, typed XLSX model, cached model
  artifact, inspection, range/sheet/full projections, budgets, continuations,
  cold/warm behavior, source invalidation, feature gating, and CLI integration.
- Formula dependency derivation and all writes remain in their dedicated plans.
- Shared contracts do not expose XLSX model types to core.
