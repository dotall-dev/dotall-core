# Core and Format-Family Architecture

Status: approved
Decision date: 2026-07-18

This spec defines what belongs to the shared Dotall engine, what belongs to each
format family, how the repository is organized, and which operational guarantees
apply when agents read and edit files.

It supersedes earlier proposed crate layouts that split cache, model, semantics,
views, and edits into separate crates. Dotall will begin with one modular core crate
and one crate per format family, then extract core modules into crates only when
implementation evidence justifies it.

## 1. Product goal

Dotall is the translation layer between agents and human files. It prevents agents
from repeatedly writing one-off parsing code, rerunning transcription or OCR, and
recomputing file structure in every session.

For every supported format, Dotall provides:

- persistent structured models and derived calculations;
- token-efficient, progressively disclosed agent views;
- a semantic reading and editing interface;
- safe, versioned changes with diff, history, and revert;
- provenance and cache invalidation;
- format-preserving round trips.

The primary API consumer is an autonomous agent. Internal formats and schemas must
not leak into normal agent workflows.

## 2. Scope

The architecture is designed for many format families, but the first implementation
supports XLSX only. A substantially different second format will be used to discover
which abstractions are genuinely shared.

For v0:

- storage is project-local in `.all/`;
- the distribution is one Rust binary with XLSX enabled;
- format crates are compile-time dependencies;
- Cargo feature gates permit slimmer builds as more formats are added;
- no universal document model is imposed;
- no remote artifact sharing, global local cache, distributed synchronization,
  branching history, or automatic conflict merging is implemented.

Remote, content-addressed artifact reuse is deferred in
`docs/ideas/remote-artifact-sharing.md`.

## 3. Architecture approach

Dotall uses one shared core crate plus one crate per format family.

The core crate has strict internal module boundaries. A module becomes a separate
crate only when it needs an independent dependency, compilation, testing, ownership,
or feature-gating boundary.

Related extensions may share one format-family crate when their model, parser, and
editing behavior are substantially the same. Detection uses extension, MIME type,
and file signature; an extension alone is not authoritative.

## 4. Responsibilities

### 4.1 Shared core

`dotall-core` owns behavior that must be consistent for every format:

- project discovery and `.all/` storage;
- manifests, normalized source paths, hashes, freshness, and invalidation;
- separation of regenerable cache artifacts from durable state;
- format detection, registration, and capability discovery;
- derivation scheduling, cache keys, provenance, and processor invalidation;
- the common read protocol, token budgets, pagination, and continuation cursors;
- access provenance;
- transaction staging, atomic apply, history, semantic diff storage, and revert;
- local writer isolation and optimistic concurrency;
- schema-version handling and crash recovery;
- orchestration consumed by CLI and MCP.

Core does not parse format internals, define format-specific models, interpret
format-specific selectors, or implement format-specific edits.

### 4.2 Format families

Each format-family crate owns:

- its typed canonical model;
- parsing and model serialization;
- reliable detection evidence for the family;
- format-specific read selectors;
- summaries and agent-readable projections;
- semantic processors and derived artifacts;
- semantic edit operation types and validation;
- surgical writing and round-trip validation;
- format-specific semantic diffs;
- golden fixtures and fidelity tests.

Examples include spreadsheet ranges and formula dependencies, document pages and
paragraphs, media timestamps and transcripts, and CAD components and assemblies.

Dotall starts with format-specific models. Shared model or graph primitives will be
extracted only after at least two contrasting formats demonstrate the same stable
need.

## 5. Repository layout

```text
dotall-core/
├── Cargo.toml
├── AGENTS.md
├── docs/
│   ├── specs/
│   └── ideas/
├── crates/
│   ├── dotall-core/
│   │   └── src/
│   │       ├── store/
│   │       ├── registry/
│   │       ├── pipeline/
│   │       ├── read/
│   │       ├── history/
│   │       └── orchestrate/
│   ├── dotall-xlsx/
│   ├── dotall-pptx/
│   ├── dotall-docx/
│   ├── dotall-pdf/
│   ├── dotall-ooxml/
│   ├── dotall-cli/
│   └── dotall-mcp/
└── fixtures/
    └── xlsx/
```

The CLI and MCP crates compose the registry. A format crate depends on the core
contracts; core never depends on a concrete format crate. Initially, CLI and MCP
enable XLSX by default. Future formats are optional Cargo features.

Likely internal modules in `dotall-xlsx` include model, parser, detection,
processors, selectors, projections, edits, and writer. These remain modules, not
separate crates, until a real boundary appears.

## 6. `.all/` storage model

`.all/` is project-local and gitignored. Each object is keyed through a manifest
entry containing its normalized relative path and source hash.

```text
.all/
├── manifest.json
└── objects/<relative-path>/
    ├── meta.json
    ├── original.ref
    ├── cache/
    │   ├── model/
    │   ├── derived/
    │   └── views/
    └── state/
        ├── access/log.jsonl
        ├── transactions/
        └── edits/
            ├── staging/
            └── history/
                └── snapshots/
```

The exact encoding of unsafe or non-portable path components is an implementation
detail recorded by the manifest.

### 6.1 Regenerable cache

Models, derived artifacts, and views can be deleted and rebuilt from the source.
They are invalidated when their inputs, processor implementation, schema, or
configuration changes.

A derived artifact key includes at least:

```text
(source_hash, processor_id, processor_version, config_hash)
```

Dependency-aware processors may also include hashes of upstream artifacts.

### 6.2 Durable state

Edit history, snapshots required for revert, transaction recovery records, and
retained audit metadata are not removed by ordinary cache cleanup. Durable data is
deleted only by an explicit retention or garbage-collection operation.

When a source changes outside Dotall, regenerable artifacts are invalidated while
history is preserved.

## 7. Format contract

Runtime registration uses object-safe shared envelopes with versioned,
format-owned payloads. This preserves typed models inside each format crate without
forcing all files into a universal Rust type.

A format handler provides:

```text
identity + version + detection evidence
capability description
parse(source) -> model artifact
derive(model, processor request) -> derived artifact
read(model, selector, budget) -> agent projection
validate(model, semantic operations) -> validated operations
apply(source, validated operations) -> patched output + semantic diff
```

Every payload records its format ID, schema ID, and schema version. The format crate
serializes and deserializes its typed payloads. Core stores and routes envelopes but
does not interpret format internals.

Capability descriptors make optional selectors, processors, and edit operations
discoverable. Unsupported behavior returns an explicit capability error and never
falls back to unsafe generic node mutation.

## 8. Agent-facing interface

Agents receive a small common interface:

```text
inspect(file)
read(file, selector, budget)
edit(file, semantic_operations)
status(file)
history(file)
diff(file, versions)
revert(file, version)
```

Behavioral requirements:

- Format detection is automatic.
- `inspect` returns a useful summary, available capabilities, and suggested next
  reads with examples.
- Selectors use concepts natural to the file: cell ranges, pages, timestamps,
  regions, or component names.
- Edits use semantic names such as `set_cell_formula` rather than generic property
  mutation.
- Agents do not need to run Python, unzip files, invoke transcription tools, or
  understand internal schemas.
- Reads use progressive disclosure, token budgets, pagination, and continuation
  cursors.
- Unsupported requests identify supported alternatives and include an example.
- Successful edits return a concise semantic diff, resulting version and source
  hash, and a direct revert path.
- Agent workflows auto-apply by default. Sensitive workflows can require staging
  for review.

The CLI and MCP expose the same core behavior. Their transport schemas may differ,
but neither reimplements storage, parsing, or edit semantics.

## 9. Read and edit flows

### 9.1 Read

```text
request
-> detect format and validate selector capability
-> verify source freshness
-> load or produce only the missing model and derived artifacts
-> ask the format to render the projection
-> enforce budget and pagination
-> record provenance
-> return content with useful drill-down hints
```

### 9.2 Edit

```text
request with expected source hash and transaction ID
-> format validates semantic operations against the current model
-> core durably stages the transaction and required snapshot
-> format produces patched output
-> validate output and atomically replace source
-> refresh invalidated artifacts
-> append semantic history
-> return version, diff, resulting hash, and revert information
```

No successful history entry is written for a failed apply. Recovery metadata for an
interrupted transaction remains available until recovery completes.

## 10. Operational invariants

Dotall adopts relevant database and Git guarantees:

- **Atomicity:** an edit fully commits or the source remains unchanged.
- **Optimistic concurrency:** writes require the expected source hash; stale writes
  fail rather than overwrite newer content.
- **Durability:** required snapshot and transaction metadata are persisted before
  source replacement.
- **Crash recovery:** incomplete transactions are detectable and recoverable on the
  next operation.
- **Immutable history:** applied versions are append-only. Revert creates a new
  version instead of erasing history.
- **Integrity:** hashes verify sources, snapshots, and generated artifacts.
- **Idempotency:** transaction IDs make agent retries safe.
- **Isolation:** a file has one local writer; readers observe only committed state.
- **Schema evolution:** stored artifacts are versioned and are migrated or safely
  regenerated.
- **Auditability:** history records operation, actor or agent, timestamp, inputs,
  semantic diff, and resulting hash.
- **Controlled garbage collection:** caches are disposable; durable snapshots obey
  explicit retention policy.

Distributed locking, branching, automatic merges, and remote synchronization remain
deferred.

## 11. Errors and recovery

Errors are structured for agent recovery and include the cause, whether retrying is
safe, the recommended next action, and supported alternatives where relevant.

Important error classes include unsupported capability, invalid selector or edit,
stale source, unavailable processor, corrupt source, failed output validation,
transaction conflict, and interrupted transaction.

Writes are produced in temporary output, validated, and atomically promoted. A
failure never silently degrades fidelity or leaves a partially written source.

## 12. Testing

Every format family must pass a reusable core contract suite:

- format detection and capability discovery;
- cold read followed by a cache-hit read;
- source, processor-version, and configuration invalidation;
- token budgeting, pagination, and continuation;
- stale-write rejection and idempotent retry;
- atomic apply, failed-apply rollback, and crash recovery;
- semantic diff and immutable history;
- revert fidelity.

Format-specific suites cover model correctness, projections, processors, selectors,
semantic operations, and round-trip preservation.

XLSX golden tests use real workbooks with formulas, styles, charts, multiple sheets,
and named ranges. A targeted edit must be correct, the workbook must remain valid,
and untouched OOXML ZIP parts must remain byte-identical.

## 13. Evolution rules

The architecture is intentionally revisable as implementation teaches us more.
Changes must be deliberate and reflected in the specs.

Extract a core module into its own crate only when at least one of these is true:

- consumers need it without the rest of core;
- it needs independently gated heavy dependencies;
- compile or test isolation has measurable value;
- multiple maintainers need a hard ownership boundary.

Introduce a shared model or graph primitive only after multiple format families
demonstrate compatible semantics. Do not generalize from XLSX alone.
