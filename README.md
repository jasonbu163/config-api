# config-api

**English** | [简体中文](README.zh-CN.md)

A small HTTP service that patches values in a plain-text config file. Each field is stored as an `@FieldName` line followed by its value; the service rewrites those values on request, guarded by a field whitelist.

Built with [axum](https://github.com/tokio-rs/axum) + [tokio](https://tokio.rs).

![rust](https://img.shields.io/badge/rust-1.85%2B-orange)
![axum](https://img.shields.io/badge/axum-0.8-blue)
![license](https://img.shields.io/badge/license-MIT-green)

> **Status**: early skeleton. `src/main.rs` is still the `cargo new` placeholder; the API documented below describes the intended behavior.

## Features

- Read the whole config file as structured fields: `GET /api/config`
- Update a single field: `POST /api/config/update`
- Update several fields atomically (all-or-nothing): `POST /api/config/batch-update`
- Whitelist enforcement — only fields listed in `config.toml` can be written
- Optional `.bak` backup before every write
- Single-writer guarantee via an async write lock (no interleaved writes)
- Startup validation: a broken config fails fast instead of surfacing at request time

## Requirements

- Rust 1.85+ (`edition = "2024"`)
- On Windows: **Visual Studio Build Tools** with the *Desktop development with C++* workload (provides `link.exe`, mandatory for the MSVC toolchain)
- On Linux: `build-essential`
- On macOS: `xcode-select --install`

## Quick Start

```bash
# 1. edit config.toml so that files.base_dir points at a directory that exists
# 2. run
cargo run

# 3. the service listens on http://127.0.0.1:8080 by default
curl http://127.0.0.1:8080/api/config
```

A second CLI argument overrides the config path:

```bash
cargo run -- /path/to/other-config.toml
```

## Configuration

`config.toml` lives next to `Cargo.toml`. On Windows, backslashes **must be double-escaped** in TOML strings.

```toml
[server]
host = "127.0.0.1"                   # use "0.0.0.0" to expose on the LAN
port = 8080

[files]
base_dir = "C:\\SmartView\\config"   # directory holding the target file
file_name = "SmartViewConfig.txt"
backup = true                        # write a .bak copy before modifying

[allowed_fields]
fields = ["Thickness", "Temper", "Alloy", "Entry Width", "Exit Width", "DataName"]
```

| Key | Type | Description |
|---|---|---|
| `server.host` | string | Listen address; must parse as an IP literal |
| `server.port` | integer | Must not be `0` |
| `files.base_dir` | string | Must exist and be a directory at startup |
| `files.file_name` | string | Non-empty file name, joined onto `base_dir` |
| `files.backup` | bool | Optional, defaults to `false` |
| `allowed_fields.fields` | string[] | Non-empty whitelist of writable fields |

Unknown keys are rejected (`deny_unknown_fields`), so typos fail at startup rather than being silently ignored.

## Target File Format

The file being edited pairs an `@FieldName` line with the value on the next line:

```
@DataName
TT26071311A002
@Thickness
25
@Temper
T3
```

Parsing rules: lines starting with `@` declare a field, the immediately following line is its value, and blank lines are skipped. CRLF endings are tolerated. Output is always re-rendered with `\n`.

## HTTP API

All responses are JSON.

### `GET /api/config`

Returns the current parsed content of the file.

```json
{
  "ok": true,
  "file": "C:\\SmartView\\config\\SmartViewConfig.txt",
  "fields": [
    { "name": "DataName", "value": "TT26071311A002" },
    { "name": "Thickness", "value": "25" }
  ]
}
```

### `POST /api/config/update`

Update one field.

```bash
curl -X POST http://127.0.0.1:8080/api/config/update \
  -H "Content-Type: application/json" \
  -d '{"name":"DataName","value":"TT26071311A002"}'
```

```json
{
  "ok": true,
  "file": "C:\\SmartView\\config\\SmartViewConfig.txt",
  "applied": ["DataName"],
  "count": 1
}
```

### `POST /api/config/batch-update`

Update several fields in one write. If any item fails validation or references a missing field, **nothing** is written.

```bash
curl -X POST http://127.0.0.1:8080/api/config/batch-update \
  -H "Content-Type: application/json" \
  -d '{"items":[{"name":"DataName","value":"NEW001"},{"name":"Thickness","value":"25"}]}'
```

PowerShell:

```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/api/config/update -Method Post `
  -ContentType 'application/json' -Body '{"name":"DataName","value":"TT26071311A002"}'
```

### Errors

| Status | Body | Cause |
|---|---|---|
| 400 | `{"ok":false,"error":"字段不在白名单中: X"}` | Field absent from `allowed_fields` |
| 400 | `{"ok":false,"error":"items 不能为空"}` | Empty `items` array |
| 400 | `{"ok":false,"error":"value 不能包含换行或以 @ 开头"}` | Value would corrupt the file format |
| 500 | `{"ok":false,"error":"..."}` | I/O failure, or the field does not exist in the file |

Error strings are currently emitted in Chinese by the implementation.

Value constraints apply to every request: no `\n`/`\r`, and no leading `@` after trimming.

## Build & Distribution

```bash
cargo build --release
```

Artifact: `target/release/config-api` (Windows: `target\release\config-api.exe`). Debug builds from `cargo run` are not meant for distribution.

Ship `config-api(.exe)` together with `config.toml`. The target machine needs **no runtime** — no Rust, no Python, no JVM.

Cross-platform rules worth remembering:

- A macOS binary must be built on macOS (Apple SDK licensing); building it from Windows/Linux is not viable.
- Linux → Windows works if `mingw-w64` is installed; macOS → Windows effectively does not.
- Windows binaries built with MSVC depend on the VC++ runtime. For bare machines, target `x86_64-pc-windows-gnu` or install the VC++ Redistributable.

## Continuous Integration

`.github/workflows/build.yml` builds all four targets in parallel.

- Triggered by a tag push (`v*`) or manually via *Run workflow*; pushing to `main` does **not** trigger it.
- Matrix: `windows-2022` (x86_64-pc-windows-msvc, static CRT), `ubuntu-24.04` (x86_64-unknown-linux-musl), `ubuntu-24.04-arm` (aarch64-unknown-linux-musl), `macos-15` (aarch64-apple-darwin).
- The musl builds are fully static, so they run regardless of the target machine's glibc version.
- On tags, the `release` job zips each platform bundle and attaches it to a GitHub Release.

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Project Layout

```
config-api/
├── .github/workflows/build.yml   # 4-target release pipeline
├── docs/                         # full implementation & packaging guide (Chinese)
├── src/
│   └── main.rs
├── .gitignore
├── Cargo.lock                    # committed: this is an application, not a library
└── Cargo.toml
```

Planned module split:

| File | Responsibility |
|---|---|
| `src/config.rs` | Load and validate `config.toml` |
| `src/state.rs` | Shared state + async write lock |
| `src/parser.rs` | Parse / render / patch the `@field` file format |
| `src/handler.rs` | The three HTTP handlers |
| `src/main.rs` | Bootstrap, routing, listener |

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `link.exe not found` | VS Build Tools missing the *Desktop development with C++* workload |
| First build takes minutes | Normal — hundreds of crates are compiled once; later builds are incremental |
| `Address already in use` | Port taken; change `server.port` or stop the old process |
| Exits immediately: `base_dir does not exist` | Wrong path in `config.toml`, or unescaped Windows backslashes |
| `os error 32` on write | The file is locked by another process (Notepad, sync client) |
| Garbled CJK text | The file is GBK but the service reads/writes UTF-8; re-save as UTF-8 or add `encoding_rs` |
| Unreachable from the LAN | Set `host = "0.0.0.0"` and open the port in the firewall |
| Antivirus flags the exe | Unsigned binaries are often false-flagged; code-sign for internal distribution |

## Further Reading

The full design notes, packaging matrix, and CI rationale live in [`docs/Axum配置接口_完整实现与打包指南.md`](docs/Axum配置接口_完整实现与打包指南.md) (Chinese).

Planned extensions: bearer-token auth, audit log (`timestamp<TAB>field<TAB>old<TAB>new`), `dry_run` preflight for batch updates.

## License

Released under the [MIT License](LICENSE).

Copyright (c) 2026 jasonbu163
