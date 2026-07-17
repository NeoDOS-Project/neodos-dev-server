# NeoDOS Dev Server

Unified developer tools for [NeoDOS](https://github.com/NeoDOS-Project/NeoDOS).

Merges the old `neodos-lsp` (Rust) and `scripts/mcp_server` (Python) into a single Rust workspace.

## Components

| Crate | Type | Description |
|-------|------|-------------|
| `neodos-toolkit` | library | Shared: database, indexer, parsers (ELF, NEM, NeoFS, Registry), analysis |
| `neodos-lsp` | binary | LSP server — IDE support (completion, goto-def, hover, rename, diagnostics) |
| `neodos-mcp` | binary | MCP server — kernel introspection, module analysis, ABI checking, consistency validation |

## Build

```bash
cargo build --release    # builds neodos-lsp + neodos-mcp
cargo test --workspace   # 17+ tests
```

## Usage

```bash
# LSP mode (for editors like VS Code, Neovim, Helix)
neodos-lsp

# MCP mode (for AI tools via stdio JSON-RPC 2.0)
neodos-mcp
```

### MCP tools available

| Tool | Description |
|------|-------------|
| `kernel_index` | List kernel source files grouped by subsystem |
| `search_symbol` | Search function/struct/trait/const definitions |
| `get_kernel_architecture` | Memory layout, boot phases, subsystem boundaries |
| `get_build_errors` | Validate build artifacts (kernel.elf, bootloader.efi, disk image) |
| `list_loaded_modules` | List NEM drivers and NXL libraries |
| `check_abi_compatibility` | Check NEM driver ABI vs kernel ABI |
| `check_consistency` | Validate architectural invariants |
| `boot_phases` | Show kernel boot phase sequence |
| `memory_layout` | Show kernel memory regions and allocators |
| `security_info` | Show SID/Token/ACL/SAM structure |
| `scheduler_info` | Show priority scheduler, run queues, SMP |
| `ipc_info` | Show pipes, IRP, work queue, event bus |

## Architecture

```
neodos-dev-server/
├── Cargo.toml              # workspace root
├── neodos-toolkit/         # shared library
│   ├── database.rs         # DashMap-backed symbol store
│   ├── indexer.rs          # Rust source parser + NeoDOS patterns
│   ├── cache.rs            # LRU document cache
│   ├── workspace.rs        # File discovery + polling watcher
│   ├── config.rs           # Environment-based configuration
│   ├── parsers/            # ELF, NEM v3, NeoFS v2, Registry Hive
│   └── analysis/           # kernel introspection, ABI, invariants
├── neodos-lsp/             # LSP transport + handlers (thin)
│   ├── server.rs           # Content-Length framed JSON-RPC
│   └── handlers.rs         # LSP request handlers → toolkit
└── neodos-mcp/             # MCP transport + tools (thin)
    ├── server.rs           # JSON-RPC 2.0 stdio engine
    └── tools.rs            # 12 MCP tool implementations → toolkit
```

## Why Rust instead of Python?

The old MCP server was 5,434 lines of Python across 18 files. The Rust version is ~1,200 lines across 4 files, reuses the existing LSP indexer and database, and eliminates the Python runtime dependency from the NeoDOS development toolchain.

## License

MIT — same as NeoDOS.

