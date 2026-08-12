# Snapshot Delta Storage Design

**Date:** 2026-08-12  
**Status:** Approved for planning  
**Related:**  
- `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md` (history spine; this doc revises snapshot storage only)  
- `docs/specs/xlsx-engine-v0.md`  
- `docs/specs/core-format-architecture.md`

## Goal

Keep edit history **lossless** (exact pre-apply source bytes for revert) while storing
**much less** than one full file copy per apply — using **binary deltas against
periodic full bases**, analogous to git’s space model without implementing git
packfiles.

## Decisions

| Topic | Choice |
|-------|--------|
| Compression model | **Binary deltas** of whole-file bytes (not ZIP-part deltas) |
| Chain policy | **Periodic full bases** every `base_every` versions (default **10**) |
| Codec | **zstd** for full objects and delta frames |
| Ownership | **`dotall-core` store** — format-agnostic |
| Compatibility | **Clean break** — replace `snapshots/<hash>.bin`; no migrator (prototyping) |
| External API | Keep `write_snapshot` / `read_snapshot` and history `snapshot_ref` = blake3 of **uncompressed** bytes |

## 1. Goals and non-goals

### Goals

- Lossless: `read_snapshot` returns byte-identical pre-apply source.
- Smaller: typical edits store a delta against a recent full base.
- Format-agnostic: XLSX (and future formats) need no snapshot-layout knowledge.
- Bounded restore: at most `base_every − 1` deltas after loading one full base.
- Dedupe: identical content hashes reuse the same object.

### Non-goals (this slice)

- Git packfiles, multipack, or `git gc`-style repack.
- ZIP-/OOXML-part-aware deltas.
- Automatic GC of unreferenced snapshot objects.
- Cross-object packing across different tracked files.
- Migrating legacy `.bin` snapshots (none in production).

## 2. On-disk layout

Under `.all/objects/<relative-path>/state/edits/history/`:

```text
snapshots/
├── objects/
│   ├── <blake3>.full.zst      # zstd-compressed full source bytes
│   └── <blake3>.delta.zst     # zstd delta vs a full base
└── index.json                 # ordered chain metadata
```

History version files (`v001.json`, …) are unchanged. They continue to store:

```json
"snapshot_ref": "<blake3 of uncompressed pre-apply bytes>"
```

### `index.json`

```json
{
  "schema_id": "dotall.snapshot-chain",
  "schema_version": 1,
  "base_every": 10,
  "entries": [
    {
      "content_hash": "aaa…",
      "kind": "full",
      "base_content_hash": null,
      "byte_len": 123456
    },
    {
      "content_hash": "bbb…",
      "kind": "delta",
      "base_content_hash": "aaa…",
      "byte_len": 123500
    }
  ]
}
```

| Field | Meaning |
|-------|---------|
| `content_hash` | blake3 of uncompressed source bytes (= `snapshot_ref`) |
| `kind` | `full` or `delta` |
| `base_content_hash` | Required for `delta`; must name a `full` entry |
| `byte_len` | Uncompressed length (debug / sanity) |
| `base_every` | Chain policy recorded on the index (default 10) |

Object writes are atomic (temp file + rename). Index updates happen under the
existing per-object apply lock.

## 3. Write path (`write_snapshot`)

Called during apply with the pre-apply source bytes:

1. `content_hash = blake3(bytes)`.
2. If an object for `content_hash` already exists → return that hash (dedupe). Append
   an index entry only if missing for this chain.
3. Else choose **full** vs **delta**:
   - **Full** when: no prior full base; or count of entries since the last full
     (including this one) would reach `base_every`; or a trial delta’s compressed
     size is ≥ ~90% of a compressed full (fallback — avoid pathological deltas).
   - **Delta** otherwise, against the **nearest prior full** base’s reconstructed
     bytes.
4. Compress and write `<hash>.full.zst` or `<hash>.delta.zst`.
5. Before committing: round-trip verify
   `reconstruct(base, delta) == bytes` and `blake3` matches.
6. Append index entry; return `content_hash`.

Engine apply / journal / history append stay as today; only snapshot persistence
changes.

## 4. Restore path (`read_snapshot`)

Given `content_hash`:

1. Resolve entry via `index.json` (or by discovering the object file).
2. **Full:** decompress → verify blake3 → return bytes.
3. **Delta:** load `base_content_hash` as a full object → decompress base → apply
   zstd delta → verify blake3 == `content_hash` → return bytes.

Corrupt or truncated objects surface a structured error (reuse
`SnapshotMissing` where appropriate, or add `SnapshotCorrupt` if distinction helps
agents).

`Engine::revert` is unchanged: it stages `restore_snapshot` with `snapshot_ref`,
then apply writes reconstructed bytes to the source.

## 5. API surface

| Layer | Change |
|-------|--------|
| `DotallStore::write_snapshot` / `read_snapshot` | Same signatures; new internals |
| History / CLI / MCP | No schema change (`snapshot_ref` meaning unchanged) |
| Config | Hardcode `base_every = 10` for v1; optional config later |

## 6. Testing

- Write full + `base_every − 1` deltas; each `read_snapshot` is byte-identical.
- Dedupe: second `write_snapshot` of identical bytes reuses the object file.
- Base boundary: every Nth distinct snapshot is `kind: full`.
- Size fallback: when delta is not smaller enough, store full.
- Corrupt object → structured error.
- Existing apply / revert / `apply_all` / MCP restore tests still pass.

## 7. Rollout

1. Implementation plan + branch off `main`.
2. Replace `.bin` expectations in store tests and docs (AGENTS layout, write-history
   design note pointing here).
3. No migration tool.

## 8. Open follow-ups (out of scope)

- Unreferenced object GC.
- Tunable `base_every` / compression level via workspace config.
- Measuring ratios on real XLSX corpora and adjusting the 90% fallback.
- Later: git-style multipack if multi-format history volume demands it.
