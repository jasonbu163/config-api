# config-api

[English](README.md) | **简体中文**

一个用 HTTP 接口改写纯文本配置文件的小服务。文件里每个字段是「`@字段名` + 下一行的值」的成对格式，服务按请求替换这些值，并用字段白名单兜底。

技术栈：[axum](https://github.com/tokio-rs/axum) + [tokio](https://tokio.rs)。

![rust](https://img.shields.io/badge/rust-1.85%2B-orange)
![axum](https://img.shields.io/badge/axum-0.8-blue)
![license](https://img.shields.io/badge/license-MIT-green)

> **当前状态**：仅为初始骨架。`src/main.rs` 还是 `cargo new` 生成的占位内容，下面的接口描述的是目标行为。

## 功能

- 把整个配置文件读成结构化字段：`GET /api/config`
- 修改单个字段：`POST /api/config/update`
- 批量修改，全成功或全不改：`POST /api/config/batch-update`
- 白名单校验——只有 `config.toml` 里列出的字段允许写
- 写入前可选生成 `.bak` 备份
- 异步写锁保证同一时刻只有一个请求在改文件，不会写出半截内容
- 启动期校验：配置写错立刻失败，而不是运行到一半才报错

## 环境要求

- Rust 1.85+（`edition = "2024"`）
- Windows：安装 **Visual Studio Build Tools**，勾选「使用 C++ 的桌面开发」工作负载（提供 `link.exe`，MSVC 工具链的硬依赖）
- Linux：`build-essential`
- macOS：`xcode-select --install`

## 快速开始

```bash
# 1. 先改 config.toml，让 files.base_dir 指向一个已存在的目录
# 2. 启动
cargo run

# 3. 默认监听 http://127.0.0.1:8080
curl http://127.0.0.1:8080/api/config
```

配置文件路径可以用第一个命令行参数覆盖：

```bash
cargo run -- /path/to/other-config.toml
```

## 配置说明

`config.toml` 放在项目根目录（与 `Cargo.toml` 同级）。**Windows 路径中的反斜杠必须双写**，因为 TOML 字符串里 `\` 是转义符。

```toml
[server]
host = "127.0.0.1"                   # 局域网访问改成 "0.0.0.0"
port = 8080

[files]
base_dir = "C:\\SmartView\\config"   # 目标文件所在目录
file_name = "SmartViewConfig.txt"
backup = true                        # 修改前生成 .bak 备份

[allowed_fields]
fields = ["Thickness", "Temper", "Alloy", "Entry Width", "Exit Width", "DataName"]
```

| 配置项 | 类型 | 说明 |
|---|---|---|
| `server.host` | 字符串 | 监听地址，必须能解析为 IP 字面量 |
| `server.port` | 整数 | 不能为 `0` |
| `files.base_dir` | 字符串 | 启动时必须存在且是目录 |
| `files.file_name` | 字符串 | 非空文件名，与 `base_dir` 拼接 |
| `files.backup` | 布尔 | 可省略，缺省为 `false` |
| `allowed_fields.fields` | 字符串数组 | 非空，可写字段白名单 |

未知配置项会直接报错（`deny_unknown_fields`），拼错的键不会被静默忽略——这是给运维排错留的余地。

## 目标文件格式

被修改的 txt 采用「`@字段名` 换行 + 值」的成对格式：

```
@DataName
TT26071311A002
@Thickness
25
@Temper
T3
```

解析规则：以 `@` 开头的行是字段名，紧随其后的下一行是值，空行跳过，CRLF 行尾兼容。输出统一重排为 `\n` 结尾。

## HTTP 接口

所有响应均为 JSON。

### `GET /api/config`

返回文件当前的解析结果。

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

修改单个字段。

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

一次写入多个字段。任一项校验失败或字段在文件中不存在，**文件保持原样，不做任何修改**。

```bash
curl -X POST http://127.0.0.1:8080/api/config/batch-update \
  -H "Content-Type: application/json" \
  -d '{"items":[{"name":"DataName","value":"NEW001"},{"name":"Thickness","value":"25"}]}'
```

PowerShell 写法：

```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/api/config/update -Method Post `
  -ContentType 'application/json' -Body '{"name":"DataName","value":"TT26071311A002"}'
```

### 错误响应

| 状态码 | 响应体 | 触发条件 |
|---|---|---|
| 400 | `{"ok":false,"error":"字段不在白名单中: X"}` | 字段不在 `allowed_fields` 里 |
| 400 | `{"ok":false,"error":"items 不能为空"}` | `items` 是空数组 |
| 400 | `{"ok":false,"error":"value 不能包含换行或以 @ 开头"}` | 值会破坏文件格式 |
| 500 | `{"ok":false,"error":"..."}` | 读写文件失败，或字段在文件中不存在 |

所有请求都要满足值约束：不能含 `\n`/`\r`，去掉前导空白后不能以 `@` 开头。

## 构建与分发

```bash
cargo build --release
```

产物：`target/release/config-api`（Windows 为 `target\release\config-api.exe`）。`cargo run` 产出的 debug 二进制不要分发。

分发内容就是 `config-api(.exe)` + `config.toml` 两份，目标机**不需要装任何运行环境**——不用 Rust、不用 Python、不用 JVM。

跨平台编译的三条硬规则：

- macOS 产物只能在 macOS 上编（苹果 SDK 许可限制），从 Windows/Linux 交叉编译不可行
- Linux → Windows 可行（需装 `mingw-w64`）；macOS → Windows 基本不可行
- Windows MSVC 产物依赖 VC++ 运行库；裸机场景改用 `x86_64-pc-windows-gnu`，或安装 VC++ Redistributable

## 持续集成

`.github/workflows/build.yml` 让四个目标并行构建。

- 触发条件是推送 tag（`v*`）或手动点 *Run workflow*；**推 `main` 分支不会触发**
- matrix：`windows-2022`（x86_64-pc-windows-msvc，静态 CRT）、`ubuntu-24.04`（x86_64-unknown-linux-musl）、`ubuntu-24.04-arm`（aarch64-unknown-linux-musl）、`macos-15`（aarch64-apple-darwin）
- musl 产物完全静态，不依赖目标机的 glibc 版本，可直接在麒麟、统信、openEuler 上运行
- 推送 tag 时，`release` job 会把四个平台的包打成 zip 并挂到 GitHub Release

```bash
git tag v0.1.0
git push origin v0.1.0
```

## 项目结构

```
config-api/
├── .github/workflows/build.yml   # 四目标发布流水线
├── docs/                         # 完整实现与打包指南
├── src/
│   └── main.rs
├── .gitignore
├── Cargo.lock                    # 必须提交：这是应用而非库
└── Cargo.toml
```

规划中的模块划分：

| 文件 | 职责 |
|---|---|
| `src/config.rs` | 加载并校验 `config.toml` |
| `src/state.rs` | 共享状态与异步写锁 |
| `src/parser.rs` | `@字段` 格式的解析、渲染、改写 |
| `src/handler.rs` | 三个接口的处理函数 |
| `src/main.rs` | 初始化、路由、监听 |

## 常见问题

| 现象 | 原因与解决 |
|---|---|
| 编译报 `link.exe not found` | 缺 VS Build Tools 的「使用 C++ 的桌面开发」工作负载 |
| 首次编译卡住很久 | 正常，正在下载编译上百个 crate；之后增量编译是秒级 |
| `Address already in use` | 端口被占用，改 `server.port` 或结束旧进程 |
| 启动即退出并提示 base_dir 不存在 | 路径写错，或 Windows 反斜杠没双写 |
| 写文件报 `os error 32` | 文件被其他进程独占（记事本、同步盘），关掉再试 |
| 中文内容乱码 | 目标 txt 是 GBK 而程序按 UTF-8 读写；另存为 UTF-8 或引入 `encoding_rs` 转码 |
| 局域网访问不到 | `host` 改成 `0.0.0.0`，并在防火墙放行该端口 |
| 杀毒软件报警 | 未签名 exe 易被误报，企业内部分发建议做代码签名 |

## 延伸阅读

完整的设计说明、打包矩阵与 CI 原理见 [`docs/Axum配置接口_完整实现与打包指南.md`](docs/Axum配置接口_完整实现与打包指南.md)。

后续可扩展：Bearer token 鉴权、审计日志（`时间<TAB>字段<TAB>旧值<TAB>新值`）、批量接口的 `dry_run` 预检。

## 许可证

本项目基于 [MIT 协议](LICENSE) 发布。

Copyright (c) 2026 jasonbu163
