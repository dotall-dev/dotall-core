# MCP Agent Interface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans.

**Goal:** Stdio MCP server over Dotall Engine + XLSX skill; flush-on-close default; `capabilities` + `inspect` discovery.

**Spec authority:** `docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md`  
**Supersedes:** `docs/superpowers/plans/2026-07-18-mcp-agent-interface.md` (reuse transport/schema patterns; enforce stage-default + flush policy + capabilities tool + skills).

**Base:** `main` @ Merge 2 (`ca8cb93`+).

**Commit policy:** Per-task commits authorized when executing this plan.

---

### Task 1: Bootstrap `dotall-mcp` crate

- Workspace member; features `default = ["xlsx"]`
- Deps: `dotall-core`, optional `dotall-xlsx`, `rmcp` (server/macros/transport-io), tokio, serde, schemars
- Smoke test `SERVER_NAME == "dotall"`
- Commit: `build: add feature-gated Dotall MCP crate`

### Task 2: Params, responses, errors

- Params for all tools including `CapabilitiesParams { file }`
- `EditParams`: `expected_source_hash`, optional `transaction_id`, `actor_id`, `operations` — **no auto-apply flag**
- Structured `ToolResponse` success/error envelopes
- Commit: `feat(mcp): define agent-facing tool schemas`

### Task 3: Server + Engine wiring

- Build `FormatRegistry` (+ Xlsx when featured), open/init store, `Engine`
- `spawn_blocking` for file I/O
- Flush-on-close policy struct + CLI flags / env
- Commit: `feat(mcp): wire Engine into MCP server session`

### Task 4: Read path tools

- `init`, `status`, `inspect`, `capabilities`, `read`, `deps`
- Commit: `feat(mcp): expose inspect capabilities read and status tools`

### Task 5: Edit / version tools

- `edit`, `staged`, `apply`, `discard`, `history`, `diff`, `revert`
- Commit: `feat(mcp): expose transactional edit and version tools`

### Task 6: Stdio serve + flush-on-close + integration tests

- `main` stdio server
- On graceful shutdown / session end: if flush enabled, `apply --all` per tracked file with staged txs (best-effort; surface errors on stderr / last tool path as documented)
- Tests: list_tools; session inspect→read→edit→apply→history→revert; flush-on-close applies staged
- Commit: `feat(mcp): serve Dotall tools over stdio`

### Task 7: XLSX skill + docs

- `skills/xlsx/SKILL.md` (workflow + discovery + safety)
- Update `AGENTS.md` / roadmap pointer
- Commit: `docs: add xlsx agent skill for Dotall MCP`

### Task 8: Quality gate + PR

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Push `feat/mcp-agent-interface` and open PR to `main`.

---

## Detail reference

For rmcp macro shapes and schema test patterns, follow Tasks in `2026-07-18-mcp-agent-interface.md`, reconciled to:

- stage-only `edit`
- `capabilities` tool
- flush-on-close default
- current Engine method names (`edit`, `apply`, `staged`, `history`, `diff`, `revert`, `discard`)
