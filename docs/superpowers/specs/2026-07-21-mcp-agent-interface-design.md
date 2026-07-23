# Dotall MCP Agent Interface Design

**Date:** 2026-07-21  
**Status:** Approved  
**Related:**  
- `docs/superpowers/plans/2026-07-18-mcp-agent-interface.md` (superseded by revised plan)  
- `docs/superpowers/specs/2026-07-20-xlsx-write-history-design.md` (flush-on-close)  
- Merge 2 on `main` (`ca8cb93`)

## Goal

Expose Dotall as a local **stdio MCP server** so agents can inspect, read, edit, version, and revert files without format-specific protocol tools. Pair with **per–format-family skills** that teach agents the workflow for each extension class (starting with XLSX).

## Decisions

| Topic | Choice |
|-------|--------|
| Transport | stdio only; stdout = MCP protocol; logs → stderr |
| Crate | Feature-gated `dotall-mcp`; thin adapter over `Engine` |
| Edit default | Stage-only (`edit` never auto-applies) |
| Session close | **Flush-on-close by default**; opt out via `--no-flush-on-close` or `DOTALL_MCP_FLUSH_ON_CLOSE=0` |
| Discovery | **B:** `inspect` + dedicated `capabilities` tool |
| Format knowledge | Skills per family (`skills/xlsx/SKILL.md`); not separate MCP tools per extension |
| SDK | Official `rmcp` (pin at implement time via `cargo add`) |

## Tools

Stable, format-agnostic verbs:

| Tool | Maps to |
|------|---------|
| `init` | `DotallStore::init` |
| `status` | store status |
| `inspect` | `Engine::inspect` (summary + capabilities + `source_hash`) |
| `capabilities` | Detect + return `format_id`, read selectors, `edit_capabilities` (+ examples); lighter than full inspect when possible |
| `read` | `Engine::read` |
| `deps` | formula dependency query (xlsx) |
| `edit` | `Engine::edit` (stage) |
| `staged` | list staged |
| `apply` | `Engine::apply` |
| `discard` | `Engine::discard` |
| `history` | `Engine::history` |
| `diff` | `Engine::diff` |
| `revert` | `Engine::revert` (stages restore; apply or flush still required unless flush applies it) |

Tool descriptions must say: call `capabilities` or `inspect` before `edit`.

## Capabilities discovery

Agents must not hardcode extension → ops.

1. `capabilities(file)` or `inspect(file)` → `format_id`, suggested reads, `edit_capabilities[]`  
2. `edit` with `operations: [{ kind, payload }]` validated by the format handler  
3. Structured errors for unsupported / impact rejection / hash mismatch  

## Skills

```text
skills/
  xlsx/
    SKILL.md
```

Each skill teaches:

- Prefer Dotall MCP over raw OOXML/zip hacking  
- Workflow: `capabilities`/`inspect` → `read`/`deps` → `edit` → `apply` (or session flush) → `history`/`revert`  
- Use `edit_capabilities` examples for payloads  
- Safety: stage-default, `expected_source_hash`, structural reject rules  

Skills are agent instructions, not a second API. Future: `skills/docx/`, etc.

## Non-goals (this slice)

- HTTP/SSE MCP transport  
- Multi-agent locking  
- Format logic inside `dotall-mcp`  
- Auto-apply on every `edit`  

## Implementation note

Revise and execute `docs/superpowers/plans/2026-07-21-mcp-agent-interface.md`. Drop obsolete `stage_only: false` auto-apply defaults from the 2026-07-18 plan.
