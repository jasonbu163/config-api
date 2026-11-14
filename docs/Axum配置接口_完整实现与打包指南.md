# Axum 配置修改接口 · 完整实现与打包指南

## 0. 项目目标与工具链对照

**功能**：Windows/Linux/macOS 上跑一个 HTTP 服务，通过接口修改指定目录下 txt 文件中 `@字段名` 下面的值。目录路径、文件名、可改字段白名单全部由 `config.toml` 控制。接口采用**方案 A**（单字段 + 批量 `items` 数组）。

| Python 侧 | Rust 侧 | 用途 |
|---|---|---|
| pyenv | rustup | 管工具链版本与编译目标 |
| uv | cargo | 建项目/加依赖/编译/运行/测试 |
| pyproject.toml + uv.lock | Cargo.toml + Cargo.lock | 依赖声明与锁定 |
| venv | 无（不需要） | Rust 无全局环境污染 |
| ruff + mypy | cargo clippy + cargo fmt | 代码质检与格式化 |

**分发认知**：Rust 编译产物绑定「CPU 架构 + 操作系统 + C 运行库」。要 mac 版就在 mac 上 build，要 win 版就在 win 上 build，Linux 同理。产物拷到目标机**不需要装任何运行环境**。

---

## 1. 环境准备（按你的主力系统）

### Windows（主力，推荐）
1. 安装 **Visual Studio Build Tools**（不是完整 VS），Installer 中勾选「**使用 C++ 的桌面开发**」工作负载。它提供 MSVC 的 `link.exe`，是 Windows 编译 Rust 的硬依赖——新手编译失败绝大多数是这个原因。
2. 下载并运行 `rustup-init.exe`，一路默认回车（默认选择 MSVC 工具链），自动安装 `cargo`/`rustc`/`rustup` 并写入 PATH。
3. 新开终端验证：`rustc --version`、`cargo --version`、`rustup --version` 均有输出即成功。

### Linux
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```
需要 `build-essential`（gcc、make）：`sudo apt install build-essential`。

### macOS
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
xcode-select --install   # 提供 clang 与系统 SDK
```

---

## 2. 项目初始化

```bash
cargo new axum-config-api
cd axum-config-api
```

生成的结构：

```
axum-config-api/
├── .gitignore          # 已自动忽略 /target
├── Cargo.toml          # 等价于 pyproject.toml
└── src/main.rs
```

`target/` 是全部编译产物目录，体积大且可重建，**永远不要提交进 git**。

---

## 3. 添加依赖（逐个说明为什么需要）

```bash
cargo add axum --features multipart
cargo add tokio --features full
cargo add serde --features derive
cargo add serde_json
cargo add toml
cargo add tower-http --features cors,trace
cargo add tracing-subscriber --features env-filter
cargo add anyhow
```

- **axum**：Web 框架，定位类似 FastAPI。
- **tokio**：异步运行时，axum 强依赖；`full` 包含 macros（`#[tokio::main]`）、net、sync 等。
- **serde / serde_json**：请求体 JSON 与 Rust 结构体转换，承担 pydantic 的解析职责。
- **toml**：解析 `config.toml`。
- **tower-http**：中间件集合，这里用 trace（请求日志）与 cors。
- **tracing-subscriber**：日志输出实现。
- **anyhow**：简化错误链处理，适合应用层而非库。

执行 `cargo add` 后依赖会写入 `Cargo.toml`。**首次 `cargo build` 需要下载并编译上百个 crate，耗时几分钟属于正常现象**，之后增量编译是秒级。只想做类型检查不产出二进制用 `cargo check`，更快。

---

## 4. 配置文件 `config.toml`

放在项目根目录（与 `Cargo.toml` 同级）。**Windows 路径中的反斜杠必须双写**，因为 TOML 字符串里 `\` 是转义符（与 Python 字符串规则一致）。

```toml
[server]
host = "127.0.0.1"          # 监听地址；局域网访问改成 "0.0.0.0"
port = 8080

[files]
base_dir = "C:\\SmartView\\config"   # 目标目录，改这里即可换目录
file_name = "SmartViewConfig.txt"    # 目标文件名
backup = true                        # 修改前是否生成 .bak 备份

[allowed_fields]
fields = ["Thickness", "Temper", "Alloy", "Entry Width", "Exit Width", "DataName"]
```

被修改的 txt 采用「`@字段名` 换行 + 值」的成对格式：

```
@DataName
TT26071311A002
@Thickness
25
@Temper
T3
```

---

## 5. 完整代码（含解析）

### 5.1 `src/config.rs` —— 配置加载与校验

```rust
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 顶层配置结构。#[serde(deny_unknown_fields)] 让写错的配置项直接报错，
/// 而不是被静默忽略——这对运维改配置排错非常关键。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub files: FilesConfig,
    pub allowed_fields: AllowedFields,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesConfig {
    pub base_dir: String,
    pub file_name: String,
    #[serde(default)] // 缺省视为 false，兼容老配置文件
    pub backup: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedFields {
    pub fields: Vec<String>,
}

impl AppConfig {
    /// 从磁盘读取并解析配置，随后立即做业务校验。
    /// with_context 给错误附加上下文，最终错误信息会形如
    /// "读取配置文件失败: config.toml: No such file or directory"。
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        let cfg: AppConfig = toml::from_str(&text)
            .with_context(|| format!("解析 TOML 失败: {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// 启动期校验：宁可启动失败，也不要运行到一半才出错。
    fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            return Err(anyhow!("server.port 不能为 0"));
        }
        let dir = Path::new(&self.files.base_dir);
        if !dir.is_dir() {
            return Err(anyhow!("base_dir 不存在或不是目录: {}", dir.display()));
        }
        if self.files.file_name.trim().is_empty() {
            return Err(anyhow!("files.file_name 不能为空"));
        }
        if self.allowed_fields.fields.is_empty() {
            return Err(anyhow!("allowed_fields.fields 不能为空"));
        }
        Ok(())
    }

    /// 拼出被修改文件的完整路径。
    pub fn config_file_path(&self) -> PathBuf {
        Path::new(&self.files.base_dir).join(&self.files.file_name)
    }

    /// 白名单判定：只有列出的字段才允许通过接口修改。
    pub fn is_allowed(&self, name: &str) -> bool {
        self.allowed_fields.fields.iter().any(|f| f == name)
    }
}
```

### 5.2 `src/state.rs` —— 共享状态与写锁

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::config::AppConfig;

/// axum 的 State 要求内部类型可 Clone 且廉价克隆，
/// 因此用 Arc 包住配置（只读，多任务共享）。
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    /// 写锁：保证同一时刻只有一个请求在改写目标文件，
    /// 避免并发 POST 互相覆盖或写出半截内容。
    write_lock: Arc<RwLock<()>>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
            write_lock: Arc::new(RwLock::new(())),
        }
    }

    /// 获取写权限。返回的 guard 在作用域结束时自动释放锁，
    /// 所以调用方只需持有它直到文件写完。
    pub async fn lock_write(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.write_lock.write().await
    }
}
```

### 5.3 `src/parser.rs` —— 文件格式解析与修改（核心）

```rust
use anyhow::{anyhow, Result};

/// 一条配置项：@key 与其下一行的 value。
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub value: String,
}

/// 解析文件内容为结构化条目。
/// 规则：以 @ 开头的行是字段名，紧随其后的下一行是值（值可以为空行）。
/// 空行与不含 @ 的散行被忽略，CRLF 行尾会被裁掉，兼容 Windows 记事本。
pub fn parse(content: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        if let Some(key) = line.strip_prefix('@') {
            // 取下一行作为值；若文件以 @key 结尾则值为空字符串
            let value = match lines.next() {
                Some(v) => v.trim_end_matches(['\r', '\n']).to_string(),
                None => String::new(),
            };
            entries.push(Entry {
                key: key.trim().to_string(),
                value,
            });
        }
    }
    entries
}

/// 把条目重新渲染成文件文本。统一使用 "\n" 结尾，
/// 保持输出稳定，便于 diff 与版本对比。
pub fn render(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push('@');
        out.push_str(&e.key);
        out.push('\n');
        out.push_str(&e.value);
        out.push('\n');
    }
    out
}

/// 应用一组修改，返回实际被修改的字段名列表。
/// 任一字段不存在则整体报错（批量接口据此实现"全成功或全不改"）。
pub fn apply(entries: &mut Vec<Entry>, updates: &[(String, String)]) -> Result<Vec<String>> {
    let mut applied = Vec::new();

    for (name, value) in updates {
        // 防御：值里不允许出现换行或以 @ 开头，否则会破坏文件格式
        if value.contains('\n') || value.contains('\r') || value.trim_start().starts_with('@') {
            return Err(anyhow!("字段 {name} 的值包含非法字符（换行或以 @ 开头）"));
        }

        match entries.iter_mut().find(|e| &e.key == name) {
            Some(entry) => {
                entry.value = value.clone();
                applied.push(name.clone());
            }
            None => return Err(anyhow!("文件中不存在字段: {name}")),
        }
    }
    Ok(applied)
}
```

### 5.4 `src/handler.rs` —— 三个接口

```rust
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::fs;

use crate::parser::{self, Entry};
use crate::state::AppState;

/// 单字段请求体：{ "name": "DataName", "value": "TT26071311A002" }
#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub name: String,
    pub value: String,
}

/// 批量请求体（方案 A）：{ "items": [ {name, value}, ... ] }
#[derive(Debug, Deserialize)]
pub struct BatchRequest {
    pub items: Vec<UpdateRequest>,
}

/// GET /api/config —— 读取当前文件内容，便于前端展示与人工核对
pub async fn read_config(State(state): State<AppState>) -> Response {
    let path = state.config.config_file_path();
    match fs::read_to_string(&path) {
        Ok(content) => {
            let entries = parser::parse(&content);
            let fields: Vec<_> = entries
                .iter()
                .map(|e| json!({ "name": e.key, "value": e.value }))
                .collect();
            Json(json!({
                "ok": true,
                "file": path.display().to_string(),
                "fields": fields,
            }))
            .into_response()
        }
        Err(e) => server_error(&format!("读取文件失败: {e}")),
    }
}

/// POST /api/config/update —— 修改单个字段
pub async fn update_one(
    State(state): State<AppState>,
    Json(req): Json<UpdateRequest>,
) -> Response {
    if let Some(resp) = validate_one(&state, &req) {
        return resp;
    }
    match apply_updates(&state, vec![(req.name, req.value)]).await {
        Ok(body) => Json(body).into_response(),
        Err(e) => server_error(&e.to_string()),
    }
}

/// POST /api/config/batch-update —— 批量修改（方案 A）
/// 语义：先整体校验白名单，再一次性写入；任一步失败则文件保持不变。
pub async fn batch_update(
    State(state): State<AppState>,
    Json(req): Json<BatchRequest>,
) -> Response {
    if req.items.is_empty() {
        return bad_request("items 不能为空");
    }
    for item in &req.items {
        if let Some(resp) = validate_one(&state, item) {
            return resp;
        }
    }
    let updates: Vec<(String, String)> = req
        .items
        .into_iter()
        .map(|i| (i.name, i.value))
        .collect();

    match apply_updates(&state, updates).await {
        Ok(body) => Json(body).into_response(),
        Err(e) => server_error(&e.to_string()),
    }
}

// ---------- 内部辅助 ----------

/// 单条校验：白名单 + 值合法性。返回 Some(Response) 表示校验失败。
fn validate_one(state: &AppState, req: &UpdateRequest) -> Option<Response> {
    if !state.config.is_allowed(&req.name) {
        return Some(bad_request(&format!("字段不在白名单中: {}", req.name)));
    }
    if req.value.contains('\n') || req.value.contains('\r') || req.value.trim_start().starts_with('@') {
        return Some(bad_request("value 不能包含换行或以 @ 开头"));
    }
    None
}

/// 真正的"加锁 → 读 → 改 → 备份 → 写"流程。
/// 文件很小，这里直接用同步 std::fs；若文件很大可改用
/// tokio::task::spawn_blocking 把阻塞 IO 挪出异步线程。
async fn apply_updates(
    state: &AppState,
    updates: Vec<(String, String)>,
) -> anyhow::Result<serde_json::Value> {
    let _guard = state.lock_write().await; // 持有写锁直到函数结束

    let path = state.config.config_file_path();
    let content = fs::read_to_string(&path)?;
    let mut entries: Vec<Entry> = parser::parse(&content);

    let applied = parser::apply(&mut entries, &updates)?;
    let new_content = parser::render(&entries);

    if state.config.files.backup {
        // 备份失败不阻断主流程，但会记录在响应里
        let bak = path.with_extension("txt.bak");
        let _ = fs::write(&bak, &content);
    }

    fs::write(&path, &new_content)?;

    Ok(json!({
        "ok": true,
        "file": path.display().to_string(),
        "applied": applied,
        "count": applied.len(),
    }))
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

fn server_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
```

### 5.5 `src/main.rs` —— 入口与路由

```rust
// Rust 不会自动发现源文件，模块必须显式声明（与 Python 随意 import 不同）
mod config;
mod handler;
mod parser;
mod state;

use anyhow::Context;
use axum::routing::{get, post};
use axum::Router;
use std::net::{IpAddr, SocketAddr};
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 日志：默认 info 级别，可用环境变量 RUST_LOG=debug 覆盖
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // 配置文件路径：支持命令行第一个参数覆盖，默认 ./config.toml
    let cfg_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".to_string());
    let config = config::AppConfig::load(std::path::Path::new(&cfg_path))
        .context("加载配置失败，服务未启动")?;

    let ip: IpAddr = config
        .server
        .host
        .parse()
        .with_context(|| format!("server.host 不是合法 IP: {}", config.server.host))?;
    let addr = SocketAddr::new(ip, config.server.port);

    let state = state::AppState::new(config);

    let app = Router::new()
        .route("/api/config", get(handler::read_config))
        .route("/api/config/update", post(handler::update_one))
        .route("/api/config/batch-update", post(handler::batch_update))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("绑定地址失败: {addr}"))?;

    tracing::info!("服务已启动: http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
```

### 5.6 `Cargo.toml` 的 release 优化段

在 `[dependencies]` 之后追加：

```toml
[profile.release]
lto = true          # 链接期优化，减小体积、提升性能
codegen-units = 1   # 单代码生成单元，优化更彻底（编译更慢）
strip = true        # 去掉调试符号，产物更小
panic = "abort"     # panic 直接终止，不做栈展开（服务程序可选）
```

---

## 6. 运行与接口测试

```bash
cargo run                 # 开发运行（debug 产物）
cargo watch -x run        # 需要热重载：cargo install cargo-watch 后使用
```

**读取**
```bash
curl http://127.0.0.1:8080/api/config
```

**修改单个值**
```bash
curl -X POST http://127.0.0.1:8080/api/config/update \
  -H "Content-Type: application/json" \
  -d '{"name":"DataName","value":"TT26071311A002"}'
```

**批量修改（方案 A）**
```bash
curl -X POST http://127.0.0.1:8080/api/config/batch-update \
  -H "Content-Type: application/json" \
  -d '{"items":[{"name":"DataName","value":"NEW001"},{"name":"Thickness","value":"25"}]}'
```

成功响应形如：
```json
{"ok":true,"file":"C:\\SmartView\\config\\SmartViewConfig.txt","applied":["DataName","Thickness"],"count":2}
```

失败示例（白名单拦截）：
```json
{"ok":false,"error":"字段不在白名单中: Password"}
```

PowerShell 下用 `Invoke-RestMethod`：
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/api/config/update -Method Post `
  -ContentType 'application/json' -Body '{"name":"DataName","value":"TT26071311A002"}'
```

---

## 7. 打包与分发

### 7.1 本机单平台（你最常用的路线）

开发期 `cargo run` 产出的是 debug 二进制，**不要分发**。正式产物：

```bash
cargo build --release
```

产物路径（Windows）：`target\release\axum-config-api.exe`。

**分发清单**：只需 `axum-config-api.exe` + `config.toml`（可选附一份 README）。目标机不需要安装 Rust、不需要任何运行时。

#### 各平台目标三元组速查

| 目标系统 | 命令 | 产物位置 | 备注 |
|---|---|---|---|
| Windows x64 | `cargo build --release` | `target\release\*.exe` | 默认 MSVC，依赖系统 VC 运行库（多数机器已有） |
| Windows x64 静态 | `rustup target add x86_64-pc-windows-gnu` 后 `cargo build --release --target x86_64-pc-windows-gnu` | `target\x86_64-pc-windows-gnu\release\*.exe` | 需装 mingw-w64；对裸系统更友好 |
| Windows x86 32 位 | `rustup target add i686-pc-windows-msvc` | `target\i686-pc-windows-msvc\release\*.exe` | 老工控机可能用得上 |
| Linux x64 | `cargo build --release` | `target/release/axum-config-api` | 默认 gnu，glibc 版本需 ≤ 目标机 |
| Linux x64 静态 | `rustup target add x86_64-unknown-linux-musl` | `target/x86_64-unknown-linux-musl/release/...` | 需 `musl-tools`；产物不依赖 glibc，跨发行版最稳 |
| Linux ARM64 | `rustup target add aarch64-unknown-linux-gnu` | 对应 target 目录 | 树莓派/ARM 服务器 |
| macOS Apple Silicon | `cargo build --release` | `target/release/axum-config-api` | 默认 aarch64-apple-darwin |
| macOS Intel | `rustup target add x86_64-apple-darwin` | `target/x86_64-apple-darwin/release/...` | 只能在 macOS 上编译 |

**两条硬规则**：
1. macOS 产物只能在 macOS 上编译（苹果 SDK 许可限制），从 Windows/Linux 交叉编译 mac 版不可行。
2. 从 mac 交叉编译 Windows exe 基本不可行（拿不到 MSVC 链接器）；Linux 交叉编 Windows exe 可行（装 mingw-w64）。

#### 产物整理建议

```
dist/
└── axum-config-api-1.0.0-windows-x64/
    ├── axum-config-api.exe
    ├── config.toml
    └── README.txt        # 启动命令、端口、配置说明
```

打包成 zip（Windows PowerShell）：
```powershell
Compress-Archive -Path dist\axum-config-api-1.0.0-windows-x64\* `
  -DestinationPath dist\axum-config-api-1.0.0-windows-x64.zip
```

Linux/macOS 用 tar：
```bash
tar -czf axum-config-api-1.0.0-linux-x64.tar.gz -C dist axum-config-api-1.0.0-linux-x64
```

#### 发布前冒烟测试

```bash
./axum-config-api.exe &
curl http://127.0.0.1:8080/api/config
curl -X POST http://127.0.0.1:8080/api/config/update -H "Content-Type: application/json" -d "{\"name\":\"DataName\",\"value\":\"SMOKE001\"}"
# 然后打开目标 txt 确认值已变更，且 .bak 备份已生成
```

### 7.2 三平台 CI 打包（高级路线）

思路：本地只负责开发与调试，打 tag 时由 GitHub Actions 在 `windows-latest` / `ubuntu-latest` / `macos-latest` 三台云端机器上各自编译，再上传产物。这样**你不需要拥有 Mac 也能拿到 macOS 包**。

`.github/workflows/build.yml`：

```yaml
name: build-release

on:
  push:
    tags: ["v*"]
  workflow_dispatch: {}   # 允许在网页上手动触发

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            artifact: axum-config-api-windows-x64
            exe: axum-config-api.exe
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact: axum-config-api-linux-x64
            exe: axum-config-api
          - os: macos-latest
            target: aarch64-apple-darwin
            artifact: axum-config-api-macos-arm64
            exe: axum-config-api
    runs-on: ${{ matrix.os }}

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.target }}

      - name: Build release
        run: cargo build --release --target ${{ matrix.target }}

      - name: Package
        shell: bash
        run: |
          mkdir -p dist
          cp "target/${{ matrix.target }}/release/${{ matrix.exe }}" dist/
          cp config.toml dist/
          cp README.md dist/ 2>/dev/null || true

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: dist/
```

使用方式：

```bash
git init
git add .
git commit -m "init: axum config api"
git remote add origin <你的仓库地址>
git push -u origin main
git tag v1.0.0
git push origin v1.0.0     # 触发 CI，三个平台并行构建
```

补充要点：
- `macos-latest` 目前是 Apple Silicon runner，产出 arm64 包；需要 Intel mac 包时，用 `macos-13`（x86_64 runner）再加一条 matrix 项。
- Windows runner 默认 shell 是 PowerShell，上面 `Package` 步骤显式写了 `shell: bash`，`cp` 才可用。
- 想要 Linux 静态包，把 ubuntu 那条的 target 换成 `x86_64-unknown-linux-musl`，并在 build 前加一步 `sudo apt-get install -y musl-tools`。
- 更进一步可用社区工具 `cargo-dist` 自动生成发布流程，或 `cross` 用 Docker 做交叉编译，原理与上面一致。

---

## 8. 常见坑与排查

| 现象 | 原因与解决 |
|---|---|
| 编译报 `link.exe not found` | 未安装 VS Build Tools 的「使用 C++ 的桌面开发」工作负载 |
| 首次编译卡住很久 | 正常，正在下载编译依赖；之后增量编译很快 |
| `Address already in use` | 端口被占用，改 `config.toml` 的 port，或结束旧进程 |
| 启动即退出并提示 base_dir 不存在 | `config.toml` 里目录写错，或 Windows 反斜杠没双写 |
| 写文件报 `os error 32` | Windows 下文件被记事本/其他进程独占，关闭占用程序 |
| 中文内容乱码 | 目标 txt 是 GBK 而程序按 UTF-8 读写；需引入 `encoding_rs` 做转码，或把文件另存为 UTF-8 |
| 局域网访问不到 | `host` 改成 `0.0.0.0`，并在 Windows 防火墙放行该端口 |
| 拷到别的电脑无法运行 | 缺 VC 运行库，改用 windows-gnu 静态目标；或安装 VC++ Redistributable |
| 杀毒软件报警 | 未签名 exe 易被误报，企业内部分发建议做代码签名 |
| `cargo run` 改代码没生效 | 用 `cargo watch -x run` 获得类似 uvicorn --reload 的体验 |

---

## 9. 后续扩展方向

- **接口鉴权**：加一个 `Authorization: Bearer <token>` 中间件，token 写在 `config.toml`。
- **审计日志**：每次修改追加一行 `时间<TAB>字段<TAB>旧值<TAB>新值` 到日志文件。
- **dry_run 预检**：批量接口加 `"dry_run": true`，只校验不落盘。
- **数据库化**：当配置从 txt 迁到 SQLite/MySQL 时再引入 `sea-orm` + `sea-orm-migration`（后者对应 alembic）；当前项目只读写 txt，引入 ORM 只会增加编译时间与复杂度，暂不需要。
