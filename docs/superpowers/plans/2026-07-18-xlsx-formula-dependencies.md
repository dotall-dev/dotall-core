# XLSX Formula Dependencies Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Parse Excel reference syntax without evaluating formulas, persist a source-bound dependency graph, and let agents ask what a cell depends on or what depends on it.

**Architecture:** Add generic derived-artifact recipes and cache storage to core. Keep formula reference lexing, workbook-specific graph types, graph construction, and dependency projections inside `dotall-xlsx`. Cache the graph by source hash, processor ID/version, configuration hash, and model artifact hash.

**Tech Stack:** Stable Rust, existing read pipeline, `serde`, `serde_json`, `blake3`, and property/unit tests; no formula evaluation engine.

---

## Preconditions and scope

Execute the core foundation and XLSX read plans first. Reconcile signatures with the
implemented code before starting.

This plan supports:

- direct A1 references with optional `$`;
- A1 ranges;
- quoted and unquoted cross-sheet references;
- named-range references when the name exists in the workbook model;
- forward and reverse dependency queries;
- formulas containing strings without treating string contents as references.

It does not calculate values, expand ranges into every cell, resolve external
workbooks, or rewrite formulas.

Commit steps require explicit user authorization.

## Target files

```text
crates/dotall-core/src/pipeline/
├── artifact.rs
├── mod.rs
└── recipe.rs
crates/dotall-core/tests/derived_cache.rs
crates/dotall-xlsx/src/
├── dependencies/
│   ├── graph.rs
│   ├── lexer.rs
│   └── mod.rs
├── format.rs
├── model.rs
└── projection.rs
crates/dotall-xlsx/tests/dependencies.rs
crates/dotall-cli/tests/xlsx_dependencies.rs
```

### Task 1: Add deterministic derived-artifact recipes to core

**Files:**

- Create: `crates/dotall-core/src/pipeline/recipe.rs`
- Modify: `crates/dotall-core/src/pipeline/mod.rs`
- Modify: `crates/dotall-core/src/pipeline/artifact.rs`
- Modify: `crates/dotall-core/Cargo.toml`

- [ ] **Step 1: Write recipe-key tests**

Create `crates/dotall-core/src/pipeline/recipe.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationRecipe {
    pub source_hash: String,
    pub processor_id: String,
    pub processor_version: String,
    pub config_hash: String,
    pub input_hashes: Vec<String>,
}

impl DerivationRecipe {
    pub fn key(&self) -> crate::Result<String> {
        let bytes = serde_json::to_vec(self).map_err(|source| {
            crate::DotallError::Serialization {
                context: "derivation recipe".into(),
                source,
            }
        })?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::DerivationRecipe;

    fn recipe() -> DerivationRecipe {
        DerivationRecipe {
            source_hash: "source".into(),
            processor_id: "xlsx.formula-dependencies".into(),
            processor_version: "1".into(),
            config_hash: "config".into(),
            input_hashes: vec!["model".into()],
        }
    }

    #[test]
    fn key_is_deterministic_and_changes_with_an_input() {
        let first = recipe();
        let mut changed = recipe();
        changed.processor_version = "2".into();

        assert_eq!(
            first.key().expect("first key"),
            recipe().key().expect("second key")
        );
        assert_ne!(
            first.key().expect("first key"),
            changed.key().expect("changed key")
        );
    }
}
```

- [ ] **Step 2: Add the serialization error used by recipe keys**

Add a `DotallError::Serialization { context: String, source:
serde_json::Error }` variant. This preserves the repository rule against
`unwrap`/`expect` in library code outside tests.

- [ ] **Step 3: Define derived cache records**

Add to `pipeline/artifact.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedArtifact {
    pub recipe: crate::pipeline::DerivationRecipe,
    pub artifact: ArtifactEnvelope,
}
```

Export both types from `pipeline/mod.rs`, then run:

```bash
cargo test -p dotall-core pipeline::recipe
```

Expected: deterministic-key tests pass.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): define deterministic derivation recipes"
```

### Task 2: Persist derived artifacts by recipe key

**Files:**

- Modify: `crates/dotall-core/src/store/mod.rs`
- Create: `crates/dotall-core/tests/derived_cache.rs`

- [ ] **Step 1: Write the derived-cache contract**

Create `crates/dotall-core/tests/derived_cache.rs`:

```rust
use std::fs;

use dotall_core::{
    ArtifactEnvelope, DerivationRecipe, DerivedArtifact, DotallStore,
};
use tempfile::tempdir;

#[test]
fn derived_artifact_hits_only_for_the_exact_recipe() {
    let temp = tempdir().expect("tempdir");
    fs::write(temp.path().join("book.xlsx"), b"source").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("store");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");
    let recipe = DerivationRecipe {
        source_hash: store.manifest().objects["book.xlsx"]
            .fingerprint
            .blake3
            .clone(),
        processor_id: "xlsx.formula-dependencies".into(),
        processor_version: "1".into(),
        config_hash: blake3::hash(b"{}").to_hex().to_string(),
        input_hashes: vec!["model-hash".into()],
    };
    let derived = DerivedArtifact {
        recipe: recipe.clone(),
        artifact: ArtifactEnvelope {
            format_id: "xlsx".into(),
            schema_id: "xlsx.formula-dependencies".into(),
            schema_version: 1,
            payload: serde_json::json!({"edges": []}),
        },
    };

    store
        .write_derived("book.xlsx", &derived)
        .expect("write derived");

    assert_eq!(
        store
            .read_derived("book.xlsx", &recipe)
            .expect("read exact"),
        Some(derived.artifact)
    );
    let mut changed = recipe;
    changed.config_hash = "different".into();
    assert_eq!(
        store
            .read_derived("book.xlsx", &changed)
            .expect("read changed"),
        None
    );
}
```

- [ ] **Step 2: Implement exact-key storage**

Add to `DotallStore`:

```rust
pub fn write_derived(
    &self,
    relative_path: &str,
    derived: &crate::pipeline::DerivedArtifact,
) -> Result<()> {
    let key = derived.recipe.key()?;
    let path = self
        .workspace
        .objects_dir()
        .join(relative_path)
        .join("cache/derived")
        .join(&derived.recipe.processor_id)
        .join(format!("{key}.json"));
    write_json(&path, derived)
}

pub fn read_derived(
    &self,
    relative_path: &str,
    recipe: &crate::pipeline::DerivationRecipe,
) -> Result<Option<crate::registry::ArtifactEnvelope>> {
    let key = recipe.key()?;
    let path = self
        .workspace
        .objects_dir()
        .join(relative_path)
        .join("cache/derived")
        .join(&recipe.processor_id)
        .join(format!("{key}.json"));
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .map_err(|source| DotallError::io(&path, source))?;
    let cached: crate::pipeline::DerivedArtifact =
        serde_json::from_slice(&bytes).map_err(|source| {
            DotallError::InvalidManifest {
                path: path.clone(),
                source,
            }
        })?;
    if cached.recipe != *recipe {
        return Ok(None);
    }
    Ok(Some(cached.artifact))
}
```

- [ ] **Step 3: Verify the contract**

Run:

```bash
cargo test -p dotall-core --test derived_cache
```

Expected: exact recipe hits and changed recipe misses.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add crates/dotall-core
git commit -m "feat(core): cache derived artifacts by recipe"
```

### Task 3: Implement an Excel reference lexer

**Files:**

- Create: `crates/dotall-xlsx/src/dependencies/lexer.rs`
- Create: `crates/dotall-xlsx/src/dependencies/mod.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`

- [ ] **Step 1: Define reference tokens and exhaustive examples**

Create `dependencies/lexer.rs` with:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaReference {
    pub sheet: Option<String>,
    pub start: String,
    pub end: Option<String>,
}

pub fn references(formula: &str) -> Vec<FormulaReference> {
    Scanner::new(formula).collect()
}

struct Scanner<'a> {
    input: &'a str,
    cursor: usize,
}

impl<'a> Scanner<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, cursor: 0 }
    }
}

impl Iterator for Scanner<'_> {
    type Item = FormulaReference;

    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.input.len() {
            let start = self.cursor;
            let character = self.input[start..].chars().next()?;
            if character == '"' {
                self.cursor = skip_string(self.input, start);
                continue;
            }
            let preceded_by_identifier = self.input[..start]
                .chars()
                .next_back()
                .is_some_and(is_identifier);
            if !preceded_by_identifier {
                if let Some((reference, end)) =
                    parse_reference(&self.input[start..])
                {
                    let remaining = &self.input[start + end..];
                    let followed_by_identifier = remaining
                        .chars()
                        .next()
                        .is_some_and(is_identifier);
                    let followed_by_call = remaining
                        .trim_start()
                        .starts_with('(');
                    if !followed_by_identifier && !followed_by_call {
                        self.cursor = start + end;
                        return Some(reference);
                    }
                }
            }
            self.cursor += character.len_utf8();
        }
        None
    }
}

fn skip_string(input: &str, start: usize) -> usize {
    let bytes = input.as_bytes();
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' {
            if bytes.get(cursor + 1) == Some(&b'"') {
                cursor += 2;
                continue;
            }
            return cursor + 1;
        }
        cursor += 1;
    }
    bytes.len()
}

fn parse_reference(input: &str) -> Option<(FormulaReference, usize)> {
    let (sheet, sheet_length) = parse_sheet(input)?;
    let cell_input = &input[sheet_length..];
    let (start, start_length) = parse_cell(cell_input)?;
    let mut consumed = sheet_length + start_length;
    let end = if input[consumed..].starts_with(':') {
        let (end, end_length) = parse_cell(&input[consumed + 1..])?;
        consumed += 1 + end_length;
        Some(end)
    } else {
        None
    };
    Some((FormulaReference { sheet, start, end }, consumed))
}

fn parse_sheet(input: &str) -> Option<(Option<String>, usize)> {
    if let Some(rest) = input.strip_prefix('\'') {
        let bytes = rest.as_bytes();
        let mut cursor = 0;
        let mut name = String::new();
        while cursor < bytes.len() {
            if bytes[cursor] == b'\'' {
                if bytes.get(cursor + 1) == Some(&b'\'') {
                    name.push('\'');
                    cursor += 2;
                    continue;
                }
                let consumed = cursor + 3;
                if rest.as_bytes().get(cursor + 1) == Some(&b'!') {
                    return Some((Some(name), consumed));
                }
                return None;
            }
            let character = rest[cursor..].chars().next()?;
            name.push(character);
            cursor += character.len_utf8();
        }
        return None;
    }

    let prefix_length = input
        .char_indices()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric()
                || matches!(character, '_' | '.')
        })
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    if prefix_length > 0 && input[prefix_length..].starts_with('!') {
        return Some((
            Some(input[..prefix_length].to_owned()),
            prefix_length + 1,
        ));
    }
    Some((None, 0))
}

fn parse_cell(input: &str) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    let mut cursor = usize::from(bytes.first() == Some(&b'$'));
    let letters_start = cursor;
    while bytes
        .get(cursor)
        .is_some_and(u8::is_ascii_alphabetic)
        && cursor - letters_start < 3
    {
        cursor += 1;
    }
    if cursor == letters_start
        || bytes.get(cursor).is_some_and(u8::is_ascii_alphabetic)
    {
        return None;
    }
    let letters = input[letters_start..cursor].to_ascii_uppercase();
    if bytes.get(cursor) == Some(&b'$') {
        cursor += 1;
    }
    let digits_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }
    let row = input[digits_start..cursor].parse::<u32>().ok()?;
    let column = letters.bytes().try_fold(0_u32, |total, letter| {
        total
            .checked_mul(26)?
            .checked_add(u32::from(letter - b'A' + 1))
    })?;
    if row == 0 || row > 1_048_576 || column > 16_384 {
        return None;
    }
    Some((format!("{letters}{row}"), cursor))
}

fn is_identifier(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.')
}
```

- [ ] **Step 2: Add lexer tests before completing helpers**

```rust
#[cfg(test)]
mod tests {
    use super::{references, FormulaReference};

    fn cell(sheet: Option<&str>, start: &str, end: Option<&str>) -> FormulaReference {
        FormulaReference {
            sheet: sheet.map(str::to_owned),
            start: start.into(),
            end: end.map(str::to_owned),
        }
    }

    #[test]
    fn parses_local_absolute_range() {
        assert_eq!(
            references("=SUM($A$1:B10)"),
            vec![cell(None, "A1", Some("B10"))]
        );
    }

    #[test]
    fn parses_quoted_cross_sheet_reference() {
        assert_eq!(
            references("='Sales 2026'!C7*2"),
            vec![cell(Some("Sales 2026"), "C7", None)]
        );
    }

    #[test]
    fn ignores_references_inside_strings() {
        assert!(references(r#"="A1:" & B2"#).eq([
            cell(None, "B2", None)
        ]));
    }

    #[test]
    fn does_not_treat_function_names_as_cells() {
        assert!(references("=LOG10(A1)").eq([
            cell(None, "A1", None)
        ]));
    }
}
```

- [ ] **Step 3: Export and verify**

Create `dependencies/mod.rs`:

```rust
mod lexer;

pub use lexer::{references, FormulaReference};
```

Add `pub mod dependencies;` to `lib.rs`.

Run:

```bash
cargo test -p dotall-xlsx dependencies::lexer
```

Expected: every lexical edge case passes.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add crates/dotall-xlsx
git commit -m "feat(xlsx): lex formula cell and range references"
```

### Task 4: Build the typed dependency graph

**Files:**

- Create: `crates/dotall-xlsx/src/dependencies/graph.rs`
- Modify: `crates/dotall-xlsx/src/dependencies/mod.rs`
- Create: `crates/dotall-xlsx/tests/dependencies.rs`

- [ ] **Step 1: Define graph types**

Create `dependencies/graph.rs`:

```rust
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::dependencies::references;
use crate::WorkbookModel;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencyTarget {
    Cell { sheet: String, address: String },
    Range { sheet: String, start: String, end: String },
    NamedRange { name: String, formula: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub dependent: String,
    pub target: DependencyTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub edges: Vec<DependencyEdge>,
    pub forward: BTreeMap<String, Vec<DependencyTarget>>,
    pub reverse_cells: BTreeMap<String, Vec<String>>,
}

pub fn build(model: &WorkbookModel) -> DependencyGraph {
    let named_ranges = model
        .named_ranges
        .iter()
        .map(|range| (range.name.as_str(), range.formula.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut unique = BTreeSet::new();

    for sheet in &model.sheets {
        for cell in &sheet.cells {
            let Some(formula) = &cell.formula else {
                continue;
            };
            let dependent = format!("{}!{}", sheet.name, cell.address);
            for reference in references(formula) {
                let target_sheet =
                    reference.sheet.unwrap_or_else(|| sheet.name.clone());
                let target = match reference.end {
                    Some(end) => DependencyTarget::Range {
                        sheet: target_sheet,
                        start: reference.start,
                        end,
                    },
                    None => DependencyTarget::Cell {
                        sheet: target_sheet,
                        address: reference.start,
                    },
                };
                unique.insert((dependent.clone(), target));
            }
            for (name, definition) in &named_ranges {
                if contains_identifier(formula, name) {
                    unique.insert((
                        dependent.clone(),
                        DependencyTarget::NamedRange {
                            name: (*name).to_owned(),
                            formula: (*definition).to_owned(),
                        },
                    ));
                }
            }
        }
    }

    let edges = unique
        .into_iter()
        .map(|(dependent, target)| DependencyEdge { dependent, target })
        .collect::<Vec<_>>();
    index(edges)
}

fn contains_identifier(formula: &str, expected: &str) -> bool {
    formula
        .split(|character: char| {
            !(character.is_ascii_alphanumeric()
                || character == '_'
                || character == '.')
        })
        .any(|token| token.eq_ignore_ascii_case(expected))
}

fn index(edges: Vec<DependencyEdge>) -> DependencyGraph {
    let mut forward: BTreeMap<String, Vec<DependencyTarget>> = BTreeMap::new();
    let mut reverse_cells: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in &edges {
        forward
            .entry(edge.dependent.clone())
            .or_default()
            .push(edge.target.clone());
        if let DependencyTarget::Cell { sheet, address } = &edge.target {
            reverse_cells
                .entry(format!("{sheet}!{address}"))
                .or_default()
                .push(edge.dependent.clone());
        }
    }
    DependencyGraph {
        edges,
        forward,
        reverse_cells,
    }
}

impl DependencyGraph {
    pub fn dependents_of(&self, sheet: &str, address: &str) -> Vec<String> {
        let Some((column, row)) = coordinates(address) else {
            return Vec::new();
        };
        let mut dependents = self
            .edges
            .iter()
            .filter(|edge| match &edge.target {
                DependencyTarget::Cell {
                    sheet: target_sheet,
                    address: target_address,
                } => {
                    target_sheet == sheet
                        && target_address.eq_ignore_ascii_case(address)
                }
                DependencyTarget::Range {
                    sheet: target_sheet,
                    start,
                    end,
                } => {
                    let bounds = coordinates(start).zip(coordinates(end));
                    target_sheet == sheet
                        && bounds.is_some_and(|((start_column, start_row), (end_column, end_row))| {
                            (start_column..=end_column).contains(&column)
                                && (start_row..=end_row).contains(&row)
                        })
                }
                DependencyTarget::NamedRange { .. } => false,
            })
            .map(|edge| edge.dependent.clone())
            .collect::<Vec<_>>();
        dependents.sort();
        dependents.dedup();
        dependents
    }
}

fn coordinates(address: &str) -> Option<(u32, u32)> {
    let split = address.find(|character: char| character.is_ascii_digit())?;
    let (letters, digits) = address.split_at(split);
    let column = letters.bytes().try_fold(0_u32, |total, letter| {
        total
            .checked_mul(26)?
            .checked_add(u32::from(letter.to_ascii_uppercase() - b'A' + 1))
    })?;
    Some((column, digits.parse().ok()?))
}
```

- [ ] **Step 2: Add graph acceptance tests**

Create `crates/dotall-xlsx/tests/dependencies.rs` using an in-memory
`WorkbookModel` with:

- `Inputs!A1`;
- `Summary!B2 = Inputs!A1`;
- `Summary!C2 = SUM(Inputs!A1:A10)`;
- `Summary!D2 = TaxRate * C2`;
- named range `TaxRate = Inputs!$B$1`.

Assert:

```rust
let graph = dotall_xlsx::dependencies::build(&model);

assert!(graph.forward["Summary!B2"].iter().any(|target| matches!(
    target,
    DependencyTarget::Cell { sheet, address }
        if sheet == "Inputs" && address == "A1"
)));
assert_eq!(
    graph.dependents_of("Inputs", "A1"),
    vec!["Summary!B2".to_owned(), "Summary!C2".to_owned()]
);
assert!(graph.forward["Summary!D2"].iter().any(|target| matches!(
    target,
    DependencyTarget::NamedRange { name, .. } if name == "TaxRate"
)));
```

- [ ] **Step 3: Correct graph construction and run tests**

Remove any unused helper introduced by the initial implementation, ensure formulas
are scanned once, and sort all index vectors for deterministic JSON.

Run:

```bash
cargo test -p dotall-xlsx --test dependencies
```

Expected: local, range, cross-sheet, named-range, forward, and reverse tests pass.

- [ ] **Step 4: Optional commit checkpoint**

```bash
git add crates/dotall-xlsx
git commit -m "feat(xlsx): build formula dependency graph"
```

### Task 5: Cache and expose the dependency processor

**Files:**

- Modify: `crates/dotall-core/src/registry/types.rs`
- Modify: `crates/dotall-core/src/registry/mod.rs`
- Modify: `crates/dotall-core/src/orchestrate/mod.rs`
- Modify: `crates/dotall-xlsx/src/format.rs`
- Modify: `crates/dotall-xlsx/src/projection.rs`
- Modify: `crates/dotall-cli/src/main.rs`
- Create: `crates/dotall-cli/tests/xlsx_dependencies.rs`

- [ ] **Step 1: Add processor contracts**

Define:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessorDescriptor {
    pub id: String,
    pub version: String,
    pub output_schema: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeriveRequest {
    pub processor_id: String,
    pub config: serde_json::Value,
}
```

Extend `FormatHandler` with:

```rust
fn processors(&self) -> Vec<ProcessorDescriptor>;

fn derive(
    &self,
    model: &ArtifactEnvelope,
    request: &DeriveRequest,
) -> Result<ArtifactEnvelope>;
```

Update test stubs to return an empty processor list and an actionable
`UnsupportedCapability` error.

- [ ] **Step 2: Implement the XLSX processor**

`XlsxFormat::processors` returns:

```rust
vec![ProcessorDescriptor {
    id: "xlsx.formula-dependencies".into(),
    version: "1".into(),
    output_schema: "xlsx.formula-dependencies".into(),
}]
```

`derive` decodes the workbook, calls `dependencies::build`, and returns schema
version 1 in an `ArtifactEnvelope`.

- [ ] **Step 3: Add `Engine::derive`**

The method:

1. obtains the fresh cached model;
2. serializes and hashes the model envelope;
3. hashes canonical JSON configuration;
4. creates the exact `DerivationRecipe`;
5. returns a cache hit when present;
6. calls the matching format processor on a miss;
7. persists and returns the derived artifact.

Add a counting-processor integration test proving two identical calls derive once
and a changed configuration derives again.

- [ ] **Step 4: Add dependency projections**

Support a read selector:

```json
{"kind":"dependencies","value":"Summary!B2"}
```

Return:

```text
Summary!B2 depends on:
- Inputs!A1

Cells depending on Summary!B2:
- Summary!E2
```

For ranges, display the range as one semantic dependency rather than expanding it.
For missing nodes, return the nearest available formula-cell examples.

- [ ] **Step 5: Add CLI access**

Add:

```text
dotall read book.xlsx --dependencies "Summary!B2"
```

The CLI calls `Engine::derive` and supplies the result to the XLSX dependency
projection. Its JSON response includes `processor_id`, `recipe_key`, forward
targets, reverse dependents, and suggested next cells.

- [ ] **Step 6: Verify cache behavior**

Run:

```bash
cargo test -p dotall-core --test derived_cache
cargo test -p dotall-xlsx --test dependencies
cargo test -p dotall-cli --test xlsx_dependencies
```

Expected: all tests pass and repeated CLI dependency reads leave the derived
artifact mtime unchanged.

- [ ] **Step 7: Optional commit checkpoint**

```bash
git add crates
git commit -m "feat: cache and query XLSX formula dependencies"
```

### Task 6: Quality gate

**Files:**

- Modify only files required to fix failed checks.

- [ ] **Step 1: Run full verification**

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

Expected: every command succeeds.

- [ ] **Step 2: Verify no evaluation behavior entered scope**

Run:

```bash
rg "evaluate|calculated_value|recalculate" crates/dotall-xlsx/src/dependencies
```

Expected: no implementation of formula evaluation; references to the explicit
non-goal in comments are acceptable.

- [ ] **Step 3: Optional final commit checkpoint**

```bash
git add crates
git commit -m "feat: add XLSX semantic dependency artifacts"
```

## Self-review record

- Covers reference lexing, local/cross-sheet/range/named references, deterministic
  graph construction, generic derivation recipes, exact cache hits, agent queries,
  and CLI exposure.
- Formula calculation, external workbooks, range expansion, and formula rewriting
  remain excluded.
