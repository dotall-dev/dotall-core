# Transactional XLSX Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let agents atomically set XLSX cell values and formulas with stale-write protection, idempotent retries, immutable semantic history, diff, crash recovery, and revert while preserving untouched OOXML parts.

**Architecture:** Core owns edit envelopes, transaction IDs, optimistic concurrency, durable journals, snapshots, atomic promotion, history, and recovery. `dotall-xlsx` owns typed cell operations, validation against its model, surgical ZIP/XML patching, output validation, and semantic diffs. The initial safe vertical slice supports `set_cell_value` and `set_cell_formula`; structural operations are covered by the following broadening plan.

**Tech Stack:** Stable Rust, existing core/read/dependency pipeline, `serde`, `serde_json`, `uuid`, `zip`, `quick-xml`, `tempfile`, and `calamine` validation.

---

## Preconditions and invariants

Execute the core foundation, XLSX read, and formula dependency plans first. Update
signatures in this plan to match execution findings before implementation.

Required invariants:

- no source mutation before validation, journal persistence, and snapshot durability;
- expected source hash checked immediately before writing;
- one local writer per tracked object;
- transaction ID makes a repeated request return the original committed result;
- source replacement is atomic and occurs on the same filesystem;
- history is append-only;
- revert creates a new history version;
- failed validation or apply leaves the source unchanged;
- interrupted transactions are classified and recovered deterministically.

Commit steps require explicit user authorization.

## Target files

```text
crates/dotall-core/src/history/
├── lock.rs
├── mod.rs
├── record.rs
└── transaction.rs
crates/dotall-core/src/store/
├── atomic.rs
└── mod.rs
crates/dotall-core/src/orchestrate/mod.rs
crates/dotall-core/tests/transactions.rs
crates/dotall-xlsx/src/edits/
├── mod.rs
├── ops.rs
├── validate.rs
└── writer/
    ├── mod.rs
    ├── package.rs
    └── worksheet.rs
crates/dotall-xlsx/tests/surgical_edit.rs
crates/dotall-cli/tests/xlsx_edit_history.rs
```

### Task 1: Define common edit and transaction contracts

**Files:**

- Create: `crates/dotall-core/src/history/record.rs`
- Create: `crates/dotall-core/src/history/transaction.rs`
- Create: `crates/dotall-core/src/history/mod.rs`
- Modify: `crates/dotall-core/src/registry/mod.rs`
- Modify: `crates/dotall-core/src/registry/types.rs`
- Modify: `crates/dotall-core/src/error.rs`
- Modify: `crates/dotall-core/src/lib.rs`

- [ ] **Step 1: Add transaction identity dependency**

Run:

```bash
cargo add --package dotall-core uuid --features v4,serde
```

- [ ] **Step 2: Define shared edit envelopes**

Add to `registry/types.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditRequest {
    pub transaction_id: uuid::Uuid,
    pub expected_source_hash: String,
    pub actor: Actor,
    pub operations: Vec<SemanticOperation>,
    pub stage_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticOperation {
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatedEdit {
    pub format_id: String,
    pub schema_id: String,
    pub schema_version: u32,
    pub operations: serde_json::Value,
    pub semantic_diff: Vec<SemanticChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticChange {
    pub target: String,
    pub description: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchedOutput {
    pub bytes: Vec<u8>,
    pub semantic_diff: Vec<SemanticChange>,
}
```

Extend `FormatHandler`:

```rust
fn validate_edit(
    &self,
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit>;

fn apply_edit(
    &self,
    source: &Path,
    edit: &ValidatedEdit,
) -> Result<PatchedOutput>;

fn validate_output(&self, bytes: &[u8]) -> Result<()>;
```

Read-only format stubs return `UnsupportedCapability` for all three methods.

- [ ] **Step 3: Define durable records**

Create `history/transaction.rs`:

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::registry::{Actor, ValidatedEdit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionPhase {
    Prepared,
    SourceReplaced,
    Committed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionJournal {
    pub transaction_id: Uuid,
    pub source_path: String,
    pub expected_source_hash: String,
    pub snapshot_hash: String,
    pub output_hash: String,
    pub phase: TransactionPhase,
    pub actor: Actor,
    pub validated_edit: ValidatedEdit,
}
```

Create `history/record.rs`:

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::registry::{Actor, SemanticChange};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub version: u64,
    pub transaction_id: Uuid,
    pub actor: Actor,
    pub timestamp_unix_ms: u64,
    pub before_hash: String,
    pub after_hash: String,
    pub snapshot_hash: String,
    pub semantic_diff: Vec<SemanticChange>,
    pub reverts_version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyResult {
    pub transaction_id: Uuid,
    pub version: Option<u64>,
    pub source_hash: String,
    pub semantic_diff: Vec<SemanticChange>,
    pub staged: bool,
    pub idempotent_replay: bool,
}
```

Export all types from `history/mod.rs` and `lib.rs`.

- [ ] **Step 4: Add conflict and recovery errors**

Add:

```rust
#[error("stale source {path}: expected {expected}, found {actual}")]
StaleSource {
    path: PathBuf,
    expected: String,
    actual: String,
},

#[error("transaction {0} conflicts with an existing request")]
TransactionConflict(uuid::Uuid),

#[error("object is already locked for writing: {0}")]
WriteLocked(PathBuf),

#[error("transaction {transaction_id} requires recovery from phase {phase}")]
RecoveryRequired {
    transaction_id: uuid::Uuid,
    phase: String,
},

#[error("output validation failed for {path}: {message}")]
OutputValidation { path: PathBuf, message: String },
```

- [ ] **Step 5: Compile contracts**

Run:

```bash
cargo test -p dotall-core --no-run
```

Expected: all core code and updated test stubs compile.

- [ ] **Step 6: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): define transactional edit contracts"
```

### Task 2: Implement durable journals, snapshots, and local writer locks

**Files:**

- Create: `crates/dotall-core/src/history/lock.rs`
- Modify: `crates/dotall-core/src/history/mod.rs`
- Modify: `crates/dotall-core/src/store/atomic.rs`
- Modify: `crates/dotall-core/src/store/mod.rs`
- Create: `crates/dotall-core/tests/transactions.rs`

- [ ] **Step 1: Add filesystem-lock dependency**

Run:

```bash
cargo add --package dotall-core fs2
```

- [ ] **Step 2: Write persistence tests**

Create `tests/transactions.rs` with tests that:

1. acquire a lock and prove a second acquisition returns `WriteLocked`;
2. save/load a prepared journal without changing fields;
3. snapshot identical bytes twice and assert one content-addressed snapshot exists;
4. atomically replace a source and assert either old or new complete bytes are
   visible, never a partial prefix.

Use this lock acceptance assertion:

```rust
let first = ObjectWriteLock::acquire(&lock_path).expect("first lock");
let second = ObjectWriteLock::acquire(&lock_path).expect_err("second lock");
assert!(matches!(second, DotallError::WriteLocked(_)));
drop(first);
ObjectWriteLock::acquire(&lock_path).expect("lock after release");
```

- [ ] **Step 3: Implement advisory object locks**

Create `history/lock.rs`:

```rust
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::{DotallError, Result};

pub struct ObjectWriteLock {
    file: File,
    path: PathBuf,
}

impl ObjectWriteLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| DotallError::io(parent, source))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| DotallError::io(path, source))?;
        file.try_lock_exclusive()
            .map_err(|_| DotallError::WriteLocked(path.to_path_buf()))?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }
}

impl Drop for ObjectWriteLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = std::fs::remove_file(&self.path);
    }
}
```

- [ ] **Step 4: Add store transaction paths and persistence**

Add store methods with these exact paths:

```text
state/transactions/<transaction-id>.json
state/edits/staging/<transaction-id>.json
state/edits/history/v000001.json
state/edits/history/snapshots/<blake3>.xlsx
state/write.lock
```

Implement:

```rust
pub fn write_journal(&self, path: &str, journal: &TransactionJournal) -> Result<()>;
pub fn read_journal(&self, path: &str, id: Uuid) -> Result<Option<TransactionJournal>>;
pub fn list_incomplete_journals(&self, path: &str) -> Result<Vec<TransactionJournal>>;
pub fn write_snapshot(&self, path: &str, bytes: &[u8]) -> Result<String>;
pub fn append_history(&mut self, path: &str, record: &HistoryRecord) -> Result<()>;
pub fn history(&self, path: &str) -> Result<Vec<HistoryRecord>>;
pub fn acquire_write_lock(&self, path: &str) -> Result<ObjectWriteLock>;
```

`append_history` must reject an existing version path, atomically write the record,
then atomically persist the incremented `version_count` in the manifest.

- [ ] **Step 5: Add atomic source replacement**

Add to `store/atomic.rs`:

```rust
pub(crate) fn replace_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        DotallError::InvalidWorkspacePath(path.to_path_buf())
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DotallError::InvalidWorkspacePath(
            path.to_path_buf(),
        ))?;
    let temporary = parent.join(format!(
        ".{}.dotall-{}.tmp",
        file_name,
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| DotallError::io(&temporary, source))?;
        file.write_all(bytes)
            .map_err(|source| DotallError::io(&temporary, source))?;
        file.sync_all()
            .map_err(|source| DotallError::io(&temporary, source))?;
        fs::rename(&temporary, path)
            .map_err(|source| DotallError::io(path, source))?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
```

- [ ] **Step 6: Run transaction persistence tests**

Run:

```bash
cargo test -p dotall-core --test transactions
```

Expected: lock, journal, snapshot deduplication, and complete replacement tests pass.

- [ ] **Step 7: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): persist edit journals and snapshots"
```

### Task 3: Define and validate typed XLSX cell edits

**Files:**

- Create: `crates/dotall-xlsx/src/edits/ops.rs`
- Create: `crates/dotall-xlsx/src/edits/validate.rs`
- Create: `crates/dotall-xlsx/src/edits/mod.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`
- Create: `crates/dotall-xlsx/tests/edit_validation.rs`

- [ ] **Step 1: Define operations**

Create `edits/ops.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum XlsxEditOp {
    SetCellValue {
        sheet: String,
        cell: String,
        value: EditableValue,
    },
    SetCellFormula {
        sheet: String,
        cell: String,
        formula: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EditableValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Blank,
}
```

- [ ] **Step 2: Write validation tests**

Test:

- known sheet/cell succeeds;
- unknown sheet lists actual sheet names;
- invalid A1 address fails;
- formula is normalized to a leading `=`;
- two operations targeting the same cell fail as ambiguous;
- semantic diff includes old value/formula and requested new value/formula.

Use:

```rust
let validated = validate(
    &model,
    &[SemanticOperation {
        kind: "set_cell_formula".into(),
        payload: serde_json::json!({
            "sheet": "Summary",
            "cell": "B12",
            "formula": "SUM(A1:A10)"
        }),
    }],
).expect("valid edit");

assert_eq!(
    validated.semantic_diff[0].after.as_deref(),
    Some("=SUM(A1:A10)")
);
```

- [ ] **Step 3: Implement validation**

`validate.rs` must:

1. deserialize each semantic operation by matching `kind`;
2. reject unknown operation kinds with examples;
3. parse cell addresses using the same strict A1 parser as reads;
4. require the sheet to exist;
5. normalize formulas;
6. capture before state from `WorkbookModel`;
7. sort operations by `(sheet, row, column)`;
8. reject duplicate targets;
9. return `ValidatedEdit` schema `xlsx.cell-edits` v1 with serialized typed ops.

No source bytes are touched during validation.

- [ ] **Step 4: Export and verify**

Create `edits/mod.rs`:

```rust
mod ops;
mod validate;

pub use ops::{EditableValue, XlsxEditOp};
pub use validate::validate;
```

Run:

```bash
cargo test -p dotall-xlsx --test edit_validation
```

Expected: all validation and semantic-diff tests pass.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-xlsx
git commit -m "feat(xlsx): validate semantic cell edits"
```

### Task 4: Implement surgical OOXML cell patching

**Files:**

- Create: `crates/dotall-xlsx/src/edits/writer/package.rs`
- Create: `crates/dotall-xlsx/src/edits/writer/worksheet.rs`
- Create: `crates/dotall-xlsx/src/edits/writer/mod.rs`
- Modify: `crates/dotall-xlsx/src/edits/mod.rs`
- Modify: `crates/dotall-xlsx/src/format.rs`
- Modify: `crates/dotall-xlsx/Cargo.toml`
- Create: `crates/dotall-xlsx/tests/surgical_edit.rs`

- [ ] **Step 1: Add OOXML dependencies**

Run:

```bash
cargo add --package dotall-xlsx zip
cargo add --package dotall-xlsx quick-xml
cargo add --package dotall-xlsx tempfile
```

- [ ] **Step 2: Write golden surgical tests**

Generate a workbook containing:

- `Inputs` and `Summary` sheets;
- styles on both sheets;
- one formula;
- a chart or drawing part generated by `rust_xlsxwriter`.

Read every ZIP entry's uncompressed bytes before apply. Apply one value edit to
`Inputs!A2`, then assert:

```rust
assert_eq!(after["xl/styles.xml"], before["xl/styles.xml"]);
assert_eq!(after["xl/worksheets/sheet2.xml"], before["xl/worksheets/sheet2.xml"]);
assert_eq!(after["xl/charts/chart1.xml"], before["xl/charts/chart1.xml"]);
assert_ne!(after["xl/worksheets/sheet1.xml"], before["xl/worksheets/sheet1.xml"]);
```

Open the output with `calamine` and assert the new value. Add a formula edit test
that asserts the formula and verifies any calc-chain relationship/content-type
changes are intentional.

- [ ] **Step 3: Resolve logical sheets to OOXML parts**

`package.rs` must parse:

1. `xl/workbook.xml` for sheet name and relationship ID;
2. `xl/_rels/workbook.xml.rels` for relationship ID to worksheet target;
3. normalize the target under `xl/`;
4. reject missing, duplicate, or path-traversing targets.

Return `BTreeMap<String, String>` from logical sheet name to ZIP part path.

- [ ] **Step 4: Patch only target worksheet XML**

`worksheet.rs` must use `quick-xml` events to:

- locate or insert `<row r="N">` in numeric order;
- locate or insert `<c r="A1">` in column order;
- preserve unrelated row/cell events;
- remove old `<v>`, `<f>`, and `<is>` children only for the target cell;
- write numbers as `<c><v>…</v></c>`;
- write booleans as `<c t="b"><v>0|1</v></c>`;
- write strings as `<c t="inlineStr"><is><t>…</t></is></c>`;
- write formulas without the leading `=` as `<c><f>…</f></c>`;
- write blank as an empty cell while retaining existing style attribute `s`;
- update worksheet dimension when a new cell extends it.

Process sorted operations in one pass per worksheet. Do not serialize untouched
worksheet parts.

- [ ] **Step 5: Rebuild the ZIP with raw copies**

`writer/mod.rs` must:

1. open the source ZIP;
2. map edits to target worksheet parts;
3. patch target XML into memory;
4. create an output ZIP;
5. use the `zip` crate's raw-copy API for untouched entries so compressed payloads
   and metadata are preserved where supported;
6. write only patched entries normally;
7. on formula changes, remove `xl/calcChain.xml` if present and remove its explicit
   relationship and content-type override;
8. return output bytes.

The acceptance criterion is byte identity of every untouched **uncompressed OOXML
part**, not identity of the ZIP central directory.

- [ ] **Step 6: Validate outputs through the format handler**

Implement:

```rust
fn validate_edit(
    &self,
    model: &ArtifactEnvelope,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    crate::edits::validate(&decode(model)?, operations)
}

fn apply_edit(
    &self,
    source: &Path,
    edit: &ValidatedEdit,
) -> Result<PatchedOutput> {
    crate::edits::writer::apply(source, edit)
}

fn validate_output(&self, bytes: &[u8]) -> Result<()> {
    let temporary = tempfile::NamedTempFile::new().map_err(|source| {
        DotallError::Format {
            format_id: FORMAT_ID.into(),
            path: "<output-validation>".into(),
            message: source.to_string(),
        }
    })?;
    std::fs::write(temporary.path(), bytes).map_err(|source| {
        DotallError::Format {
            format_id: FORMAT_ID.into(),
            path: temporary.path().to_path_buf(),
            message: source.to_string(),
        }
    })?;
    crate::parse_workbook(temporary.path()).map(|_| ())
}
```

- [ ] **Step 7: Run surgical tests**

Run:

```bash
cargo test -p dotall-xlsx --test surgical_edit
```

Expected: new values/formulas are correct and every untouched OOXML part is
byte-identical.

- [ ] **Step 8: Optional commit checkpoint**

```bash
git add crates/dotall-xlsx
git commit -m "feat(xlsx): surgically patch cell values and formulas"
```

### Task 5: Orchestrate atomic apply, idempotency, and crash recovery

**Files:**

- Modify: `crates/dotall-core/src/orchestrate/mod.rs`
- Modify: `crates/dotall-core/src/store/mod.rs`
- Modify: `crates/dotall-core/tests/transactions.rs`

- [ ] **Step 1: Add transaction state-machine tests**

Test these cases with a stub format:

1. stale expected hash fails before snapshot or source mutation;
2. validation failure leaves source and history unchanged;
3. successful apply creates snapshot, committed journal, history v1, and new hash;
4. same transaction ID and identical request returns v1 with
   `idempotent_replay: true`;
5. same transaction ID and different request returns `TransactionConflict`;
6. `stage_only` writes staging data but does not mutate source;
7. `Prepared` journal with unchanged source becomes `Aborted`;
8. `SourceReplaced` journal whose source matches output hash is finalized into
   history exactly once.

- [ ] **Step 2: Implement `Engine::edit` in this order**

```text
acquire object lock
-> look up transaction ID
-> return committed replay or reject conflicting payload
-> fingerprint source and compare expected hash
-> obtain fresh model and format handler
-> validate semantic operations
-> if stage-only, persist staging record and return
-> read source bytes
-> persist content-addressed snapshot
-> ask format to produce patched bytes
-> ask format to validate patched bytes
-> hash output bytes
-> persist Prepared journal
-> re-fingerprint source and recheck expected hash
-> atomically replace source
-> persist SourceReplaced journal
-> refresh registration/model/derived artifacts
-> append history
-> persist Committed journal
-> return ApplyResult
```

Every transition uses atomic JSON persistence.

- [ ] **Step 3: Implement `Engine::recover`**

For each incomplete journal under an object lock:

- `Prepared` + source at expected hash: mark `Aborted`; no source restoration.
- `Prepared` + source at output hash: promote to `SourceReplaced`, then finalize.
- `Prepared` + any other hash: return `RecoveryRequired` without mutation.
- `SourceReplaced` + source at output hash: append history if absent, refresh
  artifacts, mark committed.
- `SourceReplaced` + source at expected hash: mark aborted.
- `SourceReplaced` + any other hash: return `RecoveryRequired`.

Run recovery when opening an object for edit, history, diff, or revert.

- [ ] **Step 4: Verify state-machine tests**

Run:

```bash
cargo test -p dotall-core --test transactions
```

Expected: all concurrency, ordering, idempotency, stage-only, and recovery tests
pass.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): apply edits through durable transactions"
```

### Task 6: Add history, semantic diff, and revert

**Files:**

- Modify: `crates/dotall-core/src/orchestrate/mod.rs`
- Modify: `crates/dotall-core/src/store/mod.rs`
- Modify: `crates/dotall-core/tests/transactions.rs`

- [ ] **Step 1: Add history/revert tests**

After two edits:

```rust
let history = engine.history("book.xlsx").expect("history");
assert_eq!(history.iter().map(|record| record.version).collect::<Vec<_>>(), [1, 2]);

let diff = engine.diff("book.xlsx", 1, 2).expect("diff");
assert_eq!(diff, history[1].semantic_diff);

let reverted = engine
    .revert("book.xlsx", 1, expected_current_hash, actor)
    .expect("revert");
assert_eq!(reverted.version, Some(3));
assert_eq!(
    engine.history("book.xlsx").expect("history")[2].reverts_version,
    Some(1)
);
```

Verify source bytes after revert equal the snapshot associated with the selected
version's before-state.

- [ ] **Step 2: Implement read-only history and diff**

`history` returns records sorted numerically by version, never lexically.

`diff(from, to)` returns semantic changes recorded between the two committed states.
For adjacent versions, return the later record's semantic diff. For non-adjacent
versions, concatenate ordered changes and label each with its version; do not
pretend to collapse conflicting changes.

- [ ] **Step 3: Implement revert as a transaction**

Revert:

1. requires expected current source hash;
2. resolves the target version's pre-apply snapshot;
3. validates the snapshot through the registered format;
4. executes the same journal/snapshot/atomic replace/history state machine;
5. records `reverts_version`;
6. produces a new version rather than deleting records.

- [ ] **Step 4: Verify history tests**

Run:

```bash
cargo test -p dotall-core --test transactions history
cargo test -p dotall-core --test transactions revert
```

Expected: immutable versions, semantic diffs, and revert-as-new-version pass.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): add immutable history diff and revert"
```

### Task 7: Expose edit lifecycle through the CLI

**Files:**

- Modify: `crates/dotall-cli/src/main.rs`
- Create: `crates/dotall-cli/tests/xlsx_edit_history.rs`

- [ ] **Step 1: Add end-to-end CLI tests**

Test:

```text
dotall edit book.xlsx --expected-hash HASH \
  --set-formula 'Summary!B12==SUM(A1:A10)' --actor agent:test
dotall history book.xlsx --json
dotall diff book.xlsx 0 1
dotall revert book.xlsx 1 --expected-hash NEW_HASH --actor human:test
```

Use a generated workbook and assert formulas with `calamine`, JSON version IDs, and
revert output.

- [ ] **Step 2: Implement command schemas**

Prefer repeated structured flags over an ad hoc expression parser:

```text
dotall edit book.xlsx --expected-hash HASH \
  --op '{"kind":"set_cell_formula","payload":{"sheet":"Summary","cell":"B12","formula":"=SUM(A1:A10)"}}' \
  --actor-kind agent --actor-id cursor --transaction-id UUID
```

Support multiple `--op` values. Generate a UUID when omitted and always print it.
Add `--stage-only`, `history`, `diff`, and `revert`.

- [ ] **Step 3: Make every result agent-actionable**

Success output includes transaction ID, version, resulting hash, concise semantic
diff, and exact revert command. Stale errors include actual hash and an exact
`inspect`/retry suggestion. JSON preserves the structured `ApplyResult`.

- [ ] **Step 4: Run CLI tests**

```bash
cargo test -p dotall-cli --test xlsx_edit_history
```

Expected: edit, idempotent retry, stale rejection, stage-only, history, diff, and
revert pass end to end.

- [ ] **Step 5: Optional commit checkpoint**

```bash
git add crates/dotall-cli
git commit -m "feat(cli): expose transactional XLSX edits and history"
```

### Task 8: Golden and quality gate

**Files:**

- Add: `fixtures/xlsx/` corpus files approved for the repository
- Modify only code required to correct verification failures.

- [ ] **Step 1: Run the real-workbook corpus**

For each approved workbook containing formulas, styles, charts, multiple sheets,
named ranges, shared strings, and merges:

1. apply a no-op validation;
2. apply one targeted value edit;
3. apply one targeted formula edit;
4. open each output with `calamine`;
5. compare every untouched uncompressed ZIP part byte-for-byte;
6. revert and compare full source bytes with the expected snapshot.

- [ ] **Step 2: Run all quality checks**

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

Expected: every command succeeds.

- [ ] **Step 3: Inspect transaction residue**

After successful tests, verify:

```bash
rg '"phase": "prepared"|"phase": "source_replaced"' .all fixtures -g '*.json'
```

Expected: no incomplete journals in test-created persistent fixtures.

- [ ] **Step 4: Optional final commit checkpoint**

```bash
git add crates fixtures/xlsx
git commit -m "feat: complete transactional XLSX cell editing"
```

## Self-review record

- Covers semantic value/formula operations, validation, surgical writing, stale
  protection, local locking, idempotency, snapshots, durable state transitions,
  recovery, append-only history, semantic diff, revert, CLI behavior, and golden
  fidelity.
- Structural spreadsheet operations remain intentionally separate so the first
  write loop is testable before formula/reference-shifting complexity is added.
