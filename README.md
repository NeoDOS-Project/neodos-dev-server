# NeoDOS Dev Server

Unified developer tools for [NeoDOS](https://github.com/NeoDOS-Project/NeoDOS).

## Components

| Crate | Type | Description |
|-------|------|-------------|
| `neodos-toolkit` | library | Shared: database, indexer, parsers (ELF, NEM, NeoFS, Registry), analysis |
| `neodos-lsp` | binary | LSP server — IDE support (completion, goto-def, hover, diagnostics) |
| `neodos-mcp` | binary | MCP server — kernel introspection, VFS, module analysis for AI tools |

## Build

```bash
cargo build --release
```

## Usage

```bash
# LSP mode (for editors)
neodos-lsp

# MCP mode (for AI tools)
neodos-mcp
```

## Architecture

```
neodos-dev-server/
├── neodos-toolkit/      # shared library
│   ├── database/        # DashMap-backed symbol database
│   ├── indexer/         # Rust source parser + NeoDOS pattern detection
│   ├── parsers/         # ELF, NEM v3, NeoFS v2, Registry Hive
│   └── analysis/        # kernel introspection, ABI checking, invariants
├── neodos-lsp/          # LSP transport + handlers (thin)
└── neodos-mcp/          # MCP transport + tools (thin)
```

Replaces the old Python `scripts/mcp_server/` and consolidates with `neodos-lsp`.
