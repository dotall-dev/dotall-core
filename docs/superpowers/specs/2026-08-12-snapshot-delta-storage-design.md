# OOXML Part Snapshot Storage Design

**Date:** 2026-08-12  
**Status:** Approved for planning (revises earlier whole-file binary-delta draft)  
**Related:**  
- `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md`  
- `docs/specs/core-format-architecture.md`  
- `docs/specs/xlsx-engine-v0.md`

## Goal

Keep edit history **lossless** (exact pre-apply `.xlsx` bytes for revert) while
storing **far less** than one full workbook copy per apply — by content-addressing
**individual OOXML ZIP parts**, the same grain surgical patching already uses.

This supersedes the whole-file zstd binary-delta approach in the prior draft of
this workstream. Whole-file binary deltas remain a possible fallback for
non-OOXML formats later; they are **not** the XLSX design.

## Decisions

| Topic | Choice |
|-------|--------|
| Granularity | **OOXML ZIP entry / part** (not whole-file binary delta) |
| Dedup model | **Content-addressed parts** shared across versions (git-blob style) |
| Per-version record | **Manifest** mapping entry name → part hash (+ ZIP metadata) |
| Ownership | **Encode/decode in `dotall-xlsx`**; **object store in `dotall-core`** |
| External API | History `snapshot_ref` stays blake3 of **reconstructed full package** bytes |
| Compatibility | Clean break; replace `snapshots/<hash>.bin` (prototyping) |
| Other formats | Default: keep opaque full-blob snapshot until they grow their own strategy |

## 1. Why part-level (not whole-file binary delta)

Surgical applies already leave untouched ZIP entries **byte-identical**. A cell
edit typically changes one worksheet (and maybe shared strings / calc chain) while
dozens of other parts are unchanged.

| Approach | What is stored per apply |
|----------|--------------------------|
| Full `.bin` (today) | Entire workbook again (dedupe only if identical) |
| Whole-file binary delta | Diff of two large ZIPs — still heavy for small XML edits |
| **Part manifests** | New/changed parts only; unchanged parts reuse prior hashes |

Part-level matches the product moat and maximizes savings for XLSX.

## 2. Goals and non-goals

### Goals

- Lossless: reconstructed package bytes are identical to the pre-apply source
  (including ZIP entry compression method, compressed payload, and CRC where the
  surgical writer already preserves them).
- Smaller: N applies that touch few parts store ~N × changed-parts, not N × file.
- Align with surgical writer: same entry names / fidelity expectations as golden
  round-trip tests.
- Stable history API: `snapshot_ref`, revert, MCP unchanged at the agent surface.

### Non-goals

- Whole-file git packfiles / zstd package deltas for XLSX.
- Editing chart/pivot/VBA parts (still preserve-only); they are still snapshotted
  as ordinary ZIP entries when present.
- Automatic GC of unreferenced parts (follow-up).
- Audio/DOCX part strategies (later; default full-blob until then).

## 3. Data model

### Part object

Content-addressed blob for one ZIP entry’s **stored** bytes (the compressed
payload as it appears in the package, plus enough metadata to rebuild the local
file header / central directory fields the fidelity tests care about).

```text
parts/<part_hash>
```

`part_hash = blake3(canonical_part_encoding)` where the encoding includes at
least:

- entry name (normalized ZIP path)
- compression method
- compressed bytes
- uncompressed size / CRC32 (as recorded)

Exact serialization is an implementation detail owned by `dotall-xlsx`, versioned
under a schema id such as `xlsx.snapshot-part` v1.

### Package manifest (one per distinct package snapshot)

```json
{
  "schema_id": "xlsx.snapshot-manifest",
  "schema_version": 1,
  "package_hash": "<blake3 of full reconstructed .xlsx bytes>",
  "entries": [
    {
      "name": "xl/worksheets/sheet1.xml",
      "part_hash": "…",
      "compression": "deflate",
      "crc32": 123,
      "compressed_size": 456,
      "uncompressed_size": 789
    }
  ]
}
```

Manifests are stored content-addressed by `package_hash` (same value as today’s
`snapshot_ref`):

```text
snapshots/manifests/<package_hash>.json
```

### Layout under an object

```text
state/edits/history/
├── snapshots/
│   ├── manifests/
│   │   └── <package_hash>.json
│   └── parts/
│       └── <part_hash>          # opaque bytes from xlsx encoder
└── v001.json …
```

Optional later: zstd-compress part payloads on disk; must not change `part_hash`
definition (hash the logical encoding, not the on-disk wrapper).

## 4. Ownership split

```text
dotall-xlsx                          dotall-core store
─────────────────────────────        ─────────────────────────────
explode .xlsx → parts + manifest     put_part(hash, bytes)
assemble parts + manifest → .xlsx    get_part(hash) → bytes
fidelity / ZIP metadata rules        put_manifest(package_hash, json)
                                     get_manifest(package_hash)
                                     write_snapshot / read_snapshot façade
```

### FormatHandler hook (recommended)

```text
fn encode_snapshot(&self, source_bytes: &[u8]) -> Result<EncodedSnapshot>
fn decode_snapshot(&self, encoded: &EncodedSnapshot) -> Result<Vec<u8>>
```

- `XlsxFormat` implements part explode/assemble.
- Default on `FormatHandler`: single opaque full blob (preserves non-XLSX behavior).
- `Engine::apply` continues to call `store.write_snapshot(relative, &before_bytes)`;
  the store (or engine) uses the registered handler for that object’s `format_id`
  when encoding.

`EncodedSnapshot` is a small envelope: `{ package_hash, manifest_json, parts: [(hash, bytes)] }`.

## 5. Write path

On apply, with pre-apply source bytes:

1. `package_hash = blake3(bytes)`.
2. If manifest `package_hash` already exists → return it (full-package dedupe).
3. Else `handler.encode_snapshot(bytes)` → manifest + part list.
4. For each part: if `parts/<part_hash>` missing, write it; else reuse.
5. Write `manifests/<package_hash>.json`.
6. Verify: `decode_snapshot` → bytes equal input and blake3 matches.
7. Return `package_hash` as `snapshot_ref`.

No separate “delta vs full base” policy is required for XLSX: **every version is a
full manifest**; storage savings come from **shared parts**. (Periodic full bases
from the binary-delta design are unnecessary here.)

## 6. Restore path

`read_snapshot(relative, package_hash)`:

1. Load manifest.
2. Load each `part_hash`.
3. `handler.decode_snapshot` → assemble ZIP.
4. Verify `blake3(bytes) == package_hash`.
5. Return bytes for revert apply.

## 7. Agent / Engine surface

Unchanged:

- `history` / `diff` / `revert` / MCP tools
- `snapshot_ref` meaning: hash of full package bytes
- Stage `restore_snapshot` then apply

## 8. Testing

- Cell edit apply: new version adds few parts; unchanged part files are not
  rewritten (same path + hash).
- Two applies changing different sheets: shared parts (e.g. styles, other sheets)
  appear once on disk.
- `read_snapshot` after each apply equals pre-apply bytes (byte-identical ZIP
  fidelity for preserved entries — align with existing surgical goldens).
- Dedupe: re-snapshot identical package reuses manifest + parts.
- Corrupt / missing part → structured error.
- Existing apply / revert / `apply_all` / MCP restore tests pass.
- Optional metric test: after N small edits, total `parts/` + `manifests/` size
  ≪ N × file size.

## 9. Rollout

1. Rewrite implementation plan against this spec (not the binary-delta draft).
2. Branch off `main`; replace `.bin` snapshot tests.
3. Update AGENTS / write-history docs to point at part manifests.
4. No migrator.

## 10. Follow-ups

- GC unreferenced parts/manifests.
- Compress part objects on disk.
- DOCX can reuse the same ZIP-part pattern (same OOXML container family).
- Non-ZIP formats: keep default full-blob encode until they need a strategy.
