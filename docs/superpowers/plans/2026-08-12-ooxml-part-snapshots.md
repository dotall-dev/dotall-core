# OOXML Part Snapshot Storage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace full-file `.bin` snapshots with content-addressed OOXML ZIP parts + per-package manifests so revert stays lossless while disk use scales with changed parts.

**Architecture:** `dotall-xlsx` encodes/decodes packages into parts+manifest; `dotall-core` stores content-addressed parts and manifests; `FormatHandler` gains `encode_snapshot` / `decode_snapshot` (default = opaque full blob); Engine persists via the handler for the file’s format.

**Tech Stack:** Rust, existing `zip` crate, blake3, serde_json, Dotall store atomic writes.

## Global Constraints

- `snapshot_ref` = blake3 of reconstructed **full package** bytes (agent/history API unchanged)
- Lossless: `decode(encode(bytes))` must equal `bytes` (byte-identical package)
- XLSX uses part manifests; other formats keep opaque blob default
- Clean break: no `.bin` migrator
- Layout: `snapshots/manifests/<package_hash>.json` + `snapshots/parts/<part_hash>`
- Schema ids: `xlsx.snapshot-manifest` v1, part encoding owned by xlsx

---

### Task 1: Core EncodedSnapshot + FormatHandler hooks

**Files:**
- Modify: `crates/dotall-core/src/registry/types.rs`
- Modify: `crates/dotall-core/src/registry/mod.rs`
- Modify: `crates/dotall-core/src/lib.rs` (exports)

- [ ] Add `EncodedSnapshot { package_hash, format_id, manifest: Value, parts: Vec<SnapshotPart> }` and `SnapshotPart { hash, bytes }`
- [ ] Default `encode_snapshot` / `decode_snapshot` on `FormatHandler`: single opaque part, manifest records kind `opaque`
- [ ] Unit test: default round-trip preserves bytes + hash
- [ ] Commit: `feat(core): add EncodedSnapshot format handler hooks`

### Task 2: Store part/manifest persistence

**Files:**
- Modify: `crates/dotall-core/src/store/mod.rs`
- Modify: `crates/dotall-core/tests/edit_persistence.rs`

- [ ] Replace `snapshots/<hash>.bin` with `put_part` / `get_part` / `put_manifest` / `get_manifest`
- [ ] `write_encoded_snapshot(relative, &EncodedSnapshot) -> package_hash` (dedupe parts + manifest)
- [ ] `read_encoded_snapshot(relative, package_hash) -> EncodedSnapshot`
- [ ] Keep thin `write_snapshot`/`read_snapshot` only if still needed for opaque path; prefer encoded API from Engine
- [ ] Update persistence tests
- [ ] Commit: `feat(core): store content-addressed snapshot parts and manifests`

### Task 3: XLSX ZIP part encode/decode (bit-exact)

**Files:**
- Create: `crates/dotall-xlsx/src/snapshot.rs`
- Modify: `crates/dotall-xlsx/src/lib.rs`, `format.rs`

- [ ] Explode package: for each ZIP entry store raw compressed payload + metadata needed for `raw_copy`-equivalent rebuild; `part_hash = blake3(canonical encoding)`
- [ ] Prefer bit-exact rebuild (entry order + raw payloads). If ZipWriter cannot match whole-file bytes, store local-file byte ranges from the original package (slice-based parts) + central directory tail so concat reconstructs exactly
- [ ] Implement `XlsxFormat::encode_snapshot` / `decode_snapshot`
- [ ] Tests: encode/decode identity on a fixture; two packages sharing a sheet share part hashes
- [ ] Commit: `feat(xlsx): encode snapshots as content-addressed OOXML parts`

### Task 4: Wire Engine apply/revert

**Files:**
- Modify: `crates/dotall-core/src/orchestrate/mod.rs`

- [ ] On apply: `handler.encode_snapshot(&before_bytes)` → `store.write_encoded_snapshot`
- [ ] On restore path: `read_encoded_snapshot` → `handler.decode_snapshot` (resolve handler via format_id on manifest / tracked object)
- [ ] Existing transaction/revert tests pass
- [ ] Commit: `feat(core): persist apply snapshots via format encode hooks`

### Task 5: Savings regression + docs

**Files:**
- Modify: `crates/dotall-core/tests/transactions.rs` or new `crates/dotall-xlsx/tests/snapshot_parts.rs`
- Modify: `AGENTS.md`, write-history design pointer, xlsx-engine-v0 snapshot blurb

- [ ] Test: two cell edits on different sheets → shared parts counted once on disk; total snapshot dir ≪ 2× file size
- [ ] Docs: `.all/` layout shows manifests/parts
- [ ] `cargo test --workspace` + clippy clean
- [ ] Commit: `test(xlsx): assert part snapshot dedup savings`

---

## Detail notes

- Reuse patterns from `edits/writer/package.rs` (`raw_copy_file`, entry iteration).
- Atomic writes via existing `write_bytes` / `write_json`.
- Missing part → `SnapshotMissing` or `SnapshotCorrupt`.
