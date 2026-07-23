# Dotall

**Translation layer between AI agents and human files.**

Dotall caches every read, versions every edit, and exposes files to LLMs through
structured, token-efficient projections — stored in a project-local `.all/`
directory (analogous to `.git/`).

This repository is the **Rust engine**: core + format handlers + CLI + stdio MCP.

## Why

Agents otherwise re-parse files every session, dump too many tokens, and break
spreadsheets on write. Dotall owns the pipeline: parse once, project for agents,
stage edits, surgically patch the source, keep history and revert.

## Status

| Area | Status |
|------|--------|
| Project-local `.all/` store | Shipped |
| XLSX read (inspect / read / deps) | Shipped |
| XLSX stage → apply → history → revert | Shipped |
| Structural XLSX ops (rows/cols/sheets, `set_range`) | Shipped |
| Stdio MCP (`dotall-mcp`) + `skills/xlsx` | Shipped |
| Demo workbook (`demo/`) | Shipped |
| DOCX / PDF / audio / SDKs | Not yet |

## Quick start

```bash
# Build
cargo build -p dotall-cli -p dotall-mcp

# Initialize a workspace (creates .all/)
cargo run -p dotall-cli -- init /path/to/project

# Inspect and read an Excel file
cargo run -p dotall-cli -- inspect /path/to/project/book.xlsx
cargo run -p dotall-cli -- read /path/to/project/book.xlsx --range 'Sheet1!A1:D20'

# Stage an edit, then apply (source unchanged until apply)
cargo run -p dotall-cli -- edit /path/to/project/book.xlsx \
  --op set_cell_value --sheet Sheet1 --address A1 --value 42
cargo run -p dotall-cli -- apply /path/to/project/book.xlsx --all

# History / revert
cargo run -p dotall-cli -- history /path/to/project/book.xlsx
cargo run -p dotall-cli -- revert /path/to/project/book.xlsx --version 1
cargo run -p dotall-cli -- apply /path/to/project/book.xlsx --all
```

Add `--json` for machine-readable output.

## MCP (for agents)

Stdio MCP server over the same engine as the CLI:

```bash
cargo run -p dotall-mcp
# or: cargo build --release -p dotall-mcp && ./target/release/dotall-mcp
```

Register the binary in your MCP client with `cwd` set to the project that contains
your files (and `.all/`). Example:

```json
{
  "mcpServers": {
    "dotall": {
      "command": "/absolute/path/to/target/release/dotall-mcp",
      "args": [],
      "cwd": "/absolute/path/to/your/project"
    }
  }
}
```

Before editing `.xlsx` files, agents should follow `skills/xlsx/SKILL.md`:

```text
capabilities / inspect → read / deps → edit (stage) → apply or flush-on-close → history / revert
```

**Flush-on-close is on by default** (staged edits apply when the MCP session ends
gracefully). Opt out with `--no-flush-on-close` or `DOTALL_MCP_FLUSH_ON_CLOSE=0`.

Stdout is MCP protocol only; logs go to stderr.

## Architecture

```text
crates/
├── dotall-core/   # store, registry, pipeline, read, history, Engine
├── dotall-xlsx/   # XLSX model, views, formula deps, surgical OOXML edits
├── dotall-cli/    # `dotall` binary
└── dotall-mcp/    # stdio MCP binary (`dotall-mcp`)
```

Formats plug in via a `FormatHandler` trait. New formats are new crates — not core
rewrites. Agents discover capabilities at runtime; they do not hardcode ops.

Round-trip fidelity for XLSX uses **surgical OOXML patching**: only target ZIP
parts change; untouched content stays byte-for-byte identical.

## Development

```bash
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Agent / contributor conventions live in [`AGENTS.md`](AGENTS.md). Specs under
`docs/specs/` and `docs/superpowers/` are the source of truth when code and docs
disagree — fix one deliberately.

## Docs map

| Doc | What |
|-----|------|
| [`docs/specs/dotall-overview.md`](docs/specs/dotall-overview.md) | Product thesis and `.all/` model |
| [`docs/specs/xlsx-engine-v0.md`](docs/specs/xlsx-engine-v0.md) | XLSX milestone definition of done |
| [`docs/specs/core-format-architecture.md`](docs/specs/core-format-architecture.md) | Core vs format-family boundaries |
| [`docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md`](docs/superpowers/specs/2026-07-21-mcp-agent-interface-design.md) | MCP tools, flush-on-close, discovery |
| [`skills/xlsx/SKILL.md`](skills/xlsx/SKILL.md) | Agent workflow for spreadsheets |

## Non-goals (v0)

No formula evaluation · no global `~/.all/cache` · no watch daemon · no multi-agent
locking · no branching history · no DOCX/PDF yet · no Python/JS SDK · no editing of
charts/pivots (preserve them, don't mutate).

## License

TBD.
