# Axum 文档修改接口 · 手把手进化教程

这是一份「从 Hello World 一路长成最终版本」的学习型教程。

**最终版本方案**请看：[`Axum配置接口_完整实现与打包指南.md`](./Axum配置接口_完整实现与打包指南.md)。  
那份文档是**最终答案**，本教程是**推导过程**。每一阶段都会故意留下一个小问题，下一阶段再修掉，这样你能理解每段代码为什么存在。

> 重要：教程中间阶段的代码只用于学习，不要直接拿去生产。最终以“完整指南”里的代码为准。

---

## 路线总览

| 阶段 | 本阶段新增 | 你能验证什么 | 故意留下的问题 |
|---|---|---|---|
| 0 | 起点 | `cargo run` 打印 Hello World | 还只是控制台程序 |
| 1 | axum + tokio | 浏览器 / curl 能访问 `http://127.0.0.1:8080/` | 返回纯文本 |
| 2 | JSON 响应 | 接口返回 JSON | 端口和配置写死在代码里 |
| 3 | `config.toml` | 改配置能换端口 | 还不会碰目标文档 |
| 4 | `files` 配置 + 读原文 | 能读到 `data/SmartViewConfig.txt` 原文 | 原文不适合前端展示 |
| 5 | 解析字段 | `GET /api/config` 返回字段数组 | 还不能写 |
| 6 | 单字段更新 | 能改一个字段 | 会丢空行和结尾注释 |
| 7 | 批量更新 | 一次改多个字段 | 并发请求可能互相覆盖 |
| 8 | 共享状态 + 读写锁 | 读写互斥 | 写文件仍可能半截 |
| 9 | 保真写入 | 空行 / 注释 / CRLF 都保留 | 直接覆盖仍不原子 |
| 10 | 备份 + 临时文件 + 原子替换 | 失败时原文档不变 | 日志还是普通文本 |
| 11 | JSON 日志 + 请求日志 | 每次请求一行 JSON | 浏览器跨域会被拦 |
| 12 | CORS | 前端页面可跨域调用 | CORS 不是鉴权 |
| 13 | 收口到最终版本 | 对照完整指南 | 生产还要加访问控制 |

---

## 阶段 0：起点

你已经完成：

```bash
cargo new config-api
cd config-api
cargo run
```

`src/main.rs` 是：

```rust
fn main() {
    println!("Hello, world!");
}
```

这一阶段不需要改任何东西，只要确认 `cargo run` 能跑。

---

## 阶段 1：从控制台程序变成 HTTP 服务

### 目标

把 `println!` 换成一个真正能被浏览器访问的地址。

### 加依赖

```bash
cargo add axum
cargo add tokio --features full
cargo add anyhow
```

### 改 `src/main.rs`

```rust
use axum::{routing::get, Router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(|| async { "Hello, axum!" }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

### 逐段理解

- `#[tokio::main]`：把 `async fn main` 交给 Tokio 运行时执行。
- `Router`：相当于路由表，`/` 对应一个处理函数。
- `get(|| async { ... })`：一个最小 handler，返回字符串。
- `TcpListener::bind`：监听 `127.0.0.1:8080`。
- `axum::serve`：把 HTTP 请求交给 `app` 处理。

### 验证

```bash
cargo run
curl http://127.0.0.1:8080/
```

看到 `Hello, axum!` 即成功。

### 本阶段的遗留问题

返回的是纯文本。真实接口通常返回 JSON，而且端口写死在代码里。

---

## 阶段 2：返回 JSON

### 目标

让接口返回结构化响应，为后面的 `{ "ok": true, ... }` 打底。

### 加依赖

```bash
cargo add serde_json
```

### 改 `src/main.rs`

```rust
use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

async fn hello() -> Json<Value> {
    Json(json!({
        "ok": true,
        "message": "Hello, axum!"
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(hello));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

### 为什么把闭包换成函数

阶段 1 的闭包只适合玩具示例。后面 handler 会越来越长，要加配置、读文件、校验参数，所以从现在开始改成独立函数。

### 验证

```bash
curl http://127.0.0.1:8080/
```

返回：

```json
{"ok":true,"message":"Hello, axum!"}
```

### 本阶段的遗留问题

`127.0.0.1:8080` 写死在源码里，改端口要重新编译。

---

## 阶段 3：把监听地址放进 `config.toml`

### 目标

让服务启动时读取配置，改端口不需要改代码。

### 加依赖

```bash
cargo add serde --features derive
cargo add toml
```

### 新建 `config.toml`

这一阶段只放 `server`：

```toml
[server]
host = "127.0.0.1"
port = 8080
```

### 改 `src/main.rs`

```rust
use anyhow::Context;
use axum::{routing::get, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;

#[derive(Debug, Deserialize)]
struct AppConfig {
    server: ServerConfig,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

async fn hello() -> Json<Value> {
    Json(json!({ "ok": true, "message": "Hello, axum!" }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let text = std::fs::read_to_string("config.toml")
        .context("读取 config.toml 失败")?;
    let config: AppConfig = toml::from_str(&text)
        .context("解析 config.toml 失败")?;

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .context("server.host / server.port 不合法")?;

    let app = Router::new()
        .route("/", get(hello));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

### 验证

把 `config.toml` 的 `port` 改成 `8081`，重新 `cargo run`，然后访问：

```bash
curl http://127.0.0.1:8081/
```

### 本阶段的遗留问题

服务还不知道要操作哪个文档。

---

## 阶段 4：加入目标文档配置，先原样读出来

### 目标

让服务知道 `base_dir + file_name`，并提供一个“读原文”的接口。

### 更新 `config.toml`

```toml
[server]
host = "127.0.0.1"
port = 8080

[files]
base_dir = "./data"
file_name = "SmartViewConfig.txt"
```

### 准备目标文档

在仓库根目录创建 `data/SmartViewConfig.txt`：

```text
@DataName
BATCH001

@Thickness
30

@Temper
T4

# End of File
```

### 改动点

在阶段 3 的 `main.rs` 里加：

```rust
use axum::extract::State;
use axum::http::StatusCode;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct FilesConfig {
    base_dir: String,
    file_name: String,
}

impl AppConfig {
    fn target_path(&self) -> PathBuf {
        PathBuf::from(&self.files.base_dir).join(&self.files.file_name)
    }
}
```

同时把 `AppConfig` 改成：

```rust
#[derive(Debug, Deserialize)]
struct AppConfig {
    server: ServerConfig,
    files: FilesConfig,
}
```

新增 handler：

```rust
async fn read_raw(
    State(config): State<Arc<AppConfig>>,
) -> Result<String, (StatusCode, String)> {
    std::fs::read_to_string(config.target_path())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
```

路由改成：

```rust
let app = Router::new()
    .route("/", get(hello))
    .route("/api/config/raw", get(read_raw))
    .with_state(Arc::new(config));
```

### 为什么要 `Arc<AppConfig>`

`State` 会被所有请求共享。`Arc` 让每个 handler 拿到的是“同一个配置的廉价引用”，而不是复制整份配置。

### 验证

```bash
curl http://127.0.0.1:8080/api/config/raw
```

应该看到文档原文，包括空行和 `# End of File`。

### 本阶段的遗留问题

原文直接返回给前端不好用，前端更想要字段数组。

---

## 阶段 5：把文档解析成字段数组

### 目标

新增 `GET /api/config`，返回：

```json
{
  "ok": true,
  "fields": [
    { "name": "DataName", "value": "BATCH001" }
  ]
}
```

### 新建 `src/parser.rs`

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub key: String,
    pub value: String,
}

/// 以 @ 开头的行是字段名，下一行是值；其他行暂时跳过。
pub fn parse(content: &str) -> Vec<Entry> {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if let Some(key) = line.strip_prefix('@') {
            let value = lines
                .get(i + 1)
                .map(|v| v.trim_end_matches('\r').to_string())
                .unwrap_or_default();

            out.push(Entry {
                key: key.trim().to_string(),
                value,
            });
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}
```

### 改 `src/main.rs`

顶部加：

```rust
mod parser;
```

新增 handler：

```rust
async fn read_config(
    State(config): State<Arc<AppConfig>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let content = std::fs::read_to_string(config.target_path())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "ok": true,
        "file": config.target_path().display().to_string(),
        "fields": parser::parse(&content),
    })))
}
```

路由加一条：

```rust
.route("/api/config", get(read_config))
```

### 验证

```bash
curl http://127.0.0.1:8080/api/config
```

应该看到六个字段，`# End of File` 不出现在 JSON 里。

### 本阶段的遗留问题

只能读，不能写。

---

## 阶段 6：加入单字段更新

### 目标

实现：

```bash
curl -X POST http://127.0.0.1:8080/api/config/update \
  -H "Content-Type: application/json" \
  -d '{"name":"Thickness","value":"30"}'
```

### 更新 `config.toml`

```toml
[allowed_fields]
fields = ["Thickness", "Temper", "Alloy", "Entry Width", "Exit Width", "DataName"]
```

### 更新 `AppConfig`

```rust
#[derive(Debug, Deserialize)]
struct AppConfig {
    server: ServerConfig,
    files: FilesConfig,
    allowed_fields: AllowedFields,
}

#[derive(Debug, Deserialize)]
struct AllowedFields {
    fields: Vec<String>,
}
```

### 给 `parser.rs` 加两个函数

```rust
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

pub fn apply(
    entries: &mut [Entry],
    updates: &[(String, String)],
) -> Result<Vec<String>, String> {
    let mut applied = Vec::new();

    for (name, value) in updates {
        if value.contains('\n') || value.contains('\r') || value.trim_start().starts_with('@') {
            return Err(format!("字段 {name} 的值非法"));
        }
        match entries.iter_mut().find(|e| &e.key == name) {
            Some(entry) => {
                entry.value = value.clone();
                applied.push(name.clone());
            }
            None => return Err(format!("文档中不存在字段: {name}")),
        }
    }
    Ok(applied)
}
```

### 新增请求体和 handler

```rust
#[derive(Debug, Deserialize)]
struct UpdateRequest {
    name: String,
    value: String,
}

async fn update_one(
    State(config): State<Arc<AppConfig>>,
    Json(req): Json<UpdateRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !config.allowed_fields.fields.contains(&req.name) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "字段不在白名单中" })),
        ));
    }

    let path = config.target_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    let mut entries = parser::parse(&content);
    let applied = parser::apply(&mut entries, &[(req.name, req.value)])
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e }))))?;

    std::fs::write(&path, parser::render(&entries))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    Ok(Json(json!({ "ok": true, "applied": applied })))
}
```

别忘了加路由：

```rust
.route("/api/config/update", post(update_one))
```

并从 axum 引入 `post`：

```rust
use axum::routing::{get, post};
```

### 故意观察一个问题

修改一次 `Thickness` 后再打开 `data/SmartViewConfig.txt`，你会发现：

- 空行没了。
- `# End of File` 没了。

这是 `render()` 整篇重生成导致的。**这是故意保留的中间形态**，下一阶段我们会先加批量，再修这个问题。

---

## 阶段 7：加入批量更新

### 目标

一次请求改多个字段，并且先全部校验通过再写。

### 新增请求体

```rust
#[derive(Debug, Deserialize)]
struct BatchRequest {
    items: Vec<UpdateRequest>,
}
```

### 新增 handler

```rust
async fn batch_update(
    State(config): State<Arc<AppConfig>>,
    Json(req): Json<BatchRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if req.items.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "items 不能为空" })),
        ));
    }

    for item in &req.items {
        if !config.allowed_fields.fields.contains(&item.name) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": format!("字段不在白名单中: {}", item.name) })),
            ));
        }
    }

    let updates: Vec<(String, String)> = req
        .items
        .into_iter()
        .map(|i| (i.name, i.value))
        .collect();

    let path = config.target_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    let mut entries = parser::parse(&content);
    let applied = parser::apply(&mut entries, &updates)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e }))))?;

    std::fs::write(&path, parser::render(&entries))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    Ok(Json(json!({ "ok": true, "applied": applied })))
}
```

加路由：

```rust
.route("/api/config/batch-update", post(batch_update))
```

### 验证

```bash
curl -X POST http://127.0.0.1:8080/api/config/batch-update \
  -H "Content-Type: application/json" \
  -d '{"items":[{"name":"Thickness","value":"30"},{"name":"Temper","value":"T4"}]}'
```

### 本阶段的遗留问题

两个并发请求可能同时读写同一个文件，互相覆盖。

---

## 阶段 8：把配置挪进 `config.rs`，加入共享状态和读写锁

### 目标

把“配置”和“并发控制”从 `main.rs` 里拆出来。

### 新建 `src/config.rs`

把阶段 7 里的 `AppConfig`、`ServerConfig`、`FilesConfig`、`AllowedFields` 搬过去，并加上：

```rust
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub files: FilesConfig,
    pub allowed_fields: AllowedFields,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        let cfg: AppConfig = toml::from_str(&text)
            .with_context(|| format!("解析 TOML 失败: {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            return Err(anyhow!("server.port 不能为 0"));
        }
        Ok(())
    }

    pub fn config_file_path(&self) -> PathBuf {
        Path::new(&self.files.base_dir).join(&self.files.file_name)
    }

    pub fn is_allowed(&self, name: &str) -> bool {
        self.allowed_fields.fields.iter().any(|f| f == name)
    }
}
```

> 这里为了教学先看主流程，字段校验会在阶段 13 补齐到最终版。

### 新建 `src/state.rs`

```rust
use crate::config::AppConfig;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    rw_lock: Arc<RwLock<()>>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
            rw_lock: Arc::new(RwLock::new(())),
        }
    }

    pub async fn lock_read(&self) -> RwLockReadGuard<'_, ()> {
        self.rw_lock.read().await
    }

    pub async fn lock_write(&self) -> RwLockWriteGuard<'_, ()> {
        self.rw_lock.write().await
    }
}
```

### 改 `main.rs` 的关键点

顶部加：

```rust
mod config;
mod state;

use crate::config::AppConfig;
use crate::state::AppState;
```

启动时：

```rust
let config = AppConfig::load(std::path::Path::new("config.toml"))?;
let state = AppState::new(config);
```

路由：

```rust
.with_state(state)
```

handler 签名从：

```rust
State(config): State<Arc<AppConfig>>
```

改成：

```rust
State(state): State<AppState>
```

然后用 `state.config` 代替原来的 `config`。

读写前分别加：

```rust
let _guard = state.lock_read().await;  // 读接口
let _guard = state.lock_write().await; // 写接口
```

### 验证

同时发两个批量更新请求，确认不会写出互相覆盖的半截状态。

### 本阶段的遗留问题

并发可控了，但 `fs::write` 仍然是“直接覆盖”。进程崩溃或磁盘满时可能留下半截文档。

---

## 阶段 9：改成保真写入，保住空行和结尾注释

### 目标

让写入后：

- 空行还在。
- `# End of File` 还在。
- CRLF / LF 行尾保持原样。
- 只替换目标字段的值行。

### 替换 `parser.rs` 的写入部分

保留阶段 5 的 `parse` 和 `Entry`，把 `render` / `apply` 换成：

```rust
fn body(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn ending(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

#[derive(Debug)]
pub enum ApplyError {
    InvalidValue(String),
    MissingField(String),
    DuplicateField(String),
    MissingValueLine(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue(name) => write!(f, "字段 {name} 的值非法"),
            Self::MissingField(name) => write!(f, "文档中不存在字段: {name}"),
            Self::DuplicateField(name) => write!(f, "字段 {name} 重复出现"),
            Self::MissingValueLine(name) => write!(f, "字段 {name} 缺少值行"),
        }
    }
}

pub fn apply_to_text(
    content: &str,
    updates: &[(String, String)],
) -> Result<(String, Vec<String>), ApplyError> {
    for (name, value) in updates {
        if value.contains('\n') || value.contains('\r') || value.trim_start().starts_with('@') {
            return Err(ApplyError::InvalidValue(name.clone()));
        }
    }

    let mut lines: Vec<String> = content
        .split_inclusive('\n')
        .map(str::to_string)
        .collect();
    let mut applied = Vec::new();

    for (name, value) in updates {
        let mut hits = 0usize;
        let mut i = 0usize;

        while i < lines.len() {
            let is_target = body(&lines[i])
                .strip_prefix('@')
                .map(str::trim)
                == Some(name.as_str());

            if !is_target {
                i += 1;
                continue;
            }

            hits += 1;
            if i + 1 >= lines.len() || body(&lines[i + 1]).starts_with('@') {
                return Err(ApplyError::MissingValueLine(name.clone()));
            }

            let e = ending(&lines[i + 1]);
            lines[i + 1] = format!("{value}{e}");
            i += 2;
        }

        match hits {
            0 => return Err(ApplyError::MissingField(name.clone())),
            1 => applied.push(name.clone()),
            _ => return Err(ApplyError::DuplicateField(name.clone())),
        }
    }

    Ok((lines.concat(), applied))
}
```

### handler 里改调用方式

原来：

```rust
let mut entries = parser::parse(&content);
let applied = parser::apply(&mut entries, &updates)?;
std::fs::write(&path, parser::render(&entries))?;
```

改成：

```rust
let (new_content, applied) = parser::apply_to_text(&content, &updates)
    .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e.to_string() }))))?;

std::fs::write(&path, new_content)
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;
```

### 验证

改一次 `Thickness`，再打开文档：

```text
@DataName
BATCH001

@Thickness
30
...
# End of File
```

空行和 `# End of File` 都还在，说明保真成功。

### 本阶段的遗留问题

`fs::write` 直接覆盖目标文件，还不算真正的“全成功或全不改”。

---

## 阶段 10：加入备份、临时文件和原子替换

### 目标

把写入流程升级为：

```text
加锁 → 读 → 改 → 写 .bak → 写临时文件 → sync_all → persist 替换
```

### 加依赖

```bash
cargo add tempfile
```

### 统一写入函数

在 `handler.rs` 里把两个写接口都改成调用这个函数：

```rust
use std::io::Write;
use tempfile::NamedTempFile;

async fn apply_updates(
    state: &AppState,
    updates: Vec<(String, String)>,
) -> Result<serde_json::Value, (StatusCode, Json<serde_json::Value>)> {
    let _guard = state.lock_write().await;

    let path = state.config.config_file_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    let (new_content, applied) = parser::apply_to_text(&content, &updates)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    if state.config.files.backup {
        let bak = path.with_extension("txt.bak");
        std::fs::write(&bak, &content)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": format!("写备份失败: {e}") }))))?;
    }

    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut tmp = NamedTempFile::new_in(parent)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    tmp.write_all(new_content.as_bytes())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    tmp.as_file_mut()
        .sync_all()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    tmp.persist(&path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))))?;

    Ok(json!({
        "ok": true,
        "file": path.display().to_string(),
        "applied": applied,
        "count": applied.len(),
    }))
}
```

然后 `update_one` 和 `batch_update` 末尾都改成：

```rust
apply_updates(&state, updates).await
```

### 为什么临时文件要在同目录

`persist` 本质是同目录下的原子 `rename`。跨磁盘分区 rename 不保证原子，所以临时文件必须和目标文档在同一个目录。

### 验证

- 修改成功：`.bak` 生成。
- 人为让备份目录不可写：请求返回 500，原文档不变。
- 打开文档：空行和结尾注释仍在。

### 本阶段的遗留问题

日志还是普通文本，不方便被日志系统采集。

---

## 阶段 11：把日志换成 JSON Lines

### 目标

每次请求都能输出一行 JSON，例如：

```json
{"timestamp":"2026-11-16T10:00:00Z","level":"INFO","method":"POST","uri":"/api/config/update","status":"200 OK","latency":"2ms"}
```

### 加依赖

```bash
cargo add tracing
cargo add tracing-subscriber --features env-filter,json
```

### 改 `main.rs` 的日志初始化

```rust
use tracing_subscriber::EnvFilter;

tracing_subscriber::fmt()
    .json()
    .with_env_filter(
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,tower_http=debug".into()),
    )
    .with_target(false)
    .init();
```

### 挂请求日志中间件

```rust
use tower_http::trace::TraceLayer;

let app = Router::new()
    // ...routes...
    .layer(TraceLayer::new_for_http());
```

### 加业务事件

在 `apply_updates` 成功替换后加：

```rust
tracing::info!(
    action = "config.updated",
    file = %path.display(),
    fields = ?applied,
    count = applied.len(),
    "文档更新成功"
);
```

### 验证

```bash
RUST_LOG=info,tower_http=debug cargo run
```

然后发一个请求，终端应输出多行 JSON。

> 如果看不到请求日志，九成是 `RUST_LOG` 没包含 `tower_http=debug`。

### 本阶段的遗留问题

浏览器页面调用接口时，会被同源策略拦截。

---

## 阶段 12：加入 CORS

### 目标

允许指定前端来源跨域调用，并自动处理 `OPTIONS` 预检。

### 加 CORS feature

```bash
cargo add tower-http --features cors,trace
```

### 更新 `config.toml`

```toml
[cors]
allowed_origins = ["http://localhost:5173", "http://127.0.0.1:5173"]
allow_credentials = false
```

### 更新 `config.rs`

```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorsConfig {
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub allow_credentials: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub files: FilesConfig,
    pub allowed_fields: AllowedFields,
    #[serde(default)]
    pub cors: CorsConfig,
}
```

并在 `validate()` 里加：

```rust
if self.cors.allow_credentials
    && self.cors.allowed_origins.iter().any(|o| o == "*")
{
    return Err(anyhow!("cors.allow_credentials=true 时 allowed_origins 不能包含 *"));
}
```

### 在 `main.rs` 里构造 CORS layer

```rust
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use tower_http::cors::{Any, CorsLayer};

fn build_cors(config: &config::AppConfig) -> anyhow::Result<CorsLayer> {
    if config.cors.allowed_origins.is_empty() {
        return Ok(CorsLayer::new());
    }

    let methods = [Method::GET, Method::POST, Method::OPTIONS];
    let headers = [AUTHORIZATION, CONTENT_TYPE];

    if config.cors.allowed_origins.iter().any(|o| o == "*") {
        return Ok(CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(methods.clone())
            .allow_headers(headers.clone()));
    }

    let origins = config.cors.allowed_origins.iter()
        .map(|o| o.parse::<HeaderValue>())
        .collect::<Result<Vec<_>, _>>()
        .context("cors.allowed_origins 含有非法 Origin")?;

    Ok(CorsLayer::new()
        .allow_origin(origins)
        .allow_methods(methods)
        .allow_headers(headers)
        .allow_credentials(config.cors.allow_credentials))
}
```

### 挂到 Router

```rust
let cors = build_cors(&config)?;

let app = Router::new()
    // ...routes...
    .with_state(state)
    .layer(cors)
    .layer(TraceLayer::new_for_http());
```

### 验证

用浏览器或一个带 `Origin` 头的请求测试：

```bash
curl -i http://127.0.0.1:8080/api/config \
  -H "Origin: http://localhost:5173"
```

响应里应出现：

```text
access-control-allow-origin: http://localhost:5173
```

预检请求也应由 `CorsLayer` 自动应答：

```bash
curl -i -X OPTIONS http://127.0.0.1:8080/api/config/update \
  -H "Origin: http://localhost:5173" \
  -H "Access-Control-Request-Method: POST" \
  -H "Access-Control-Request-Headers: content-type"
```

### 重要提醒

CORS 只解决浏览器同源策略，**不是鉴权**。任何 curl / Postman 都可以无视 CORS 直接调接口。上线前仍需 Bearer token 或网络层访问控制。

---

## 阶段 13：收口到最终版本

到这里，你已经一步一步实现了：

1. HTTP 服务。
2. JSON 响应。
3. 配置文件驱动。
4. 文档读取。
5. 字段解析。
6. 单字段更新。
7. 批量更新。
8. 共享状态与读写锁。
9. 保真写入。
10. 备份 + 临时文件 + 原子替换。
11. JSON 日志 + 请求日志。
12. CORS。

接下来请把教程代码**整体替换成最终版本**：

- 最终文档：[`Axum配置接口_完整实现与打包指南.md`](./Axum配置接口_完整实现与打包指南.md)
- 最终版本包含阶段 12 没有补齐的内容：
  - 相对 `base_dir` 基于 `config.toml` 解析。
  - 启动期检查目标文档是否存在。
  - 重复的 `@字段` 直接拒绝修改。
  - 值不能包含换行或以 `@` 开头。
  - 统一的 JSON 错误响应。
  - 完整的逐行中文注释。
  - 四平台 CI 与 Release 打包。

从现在开始，**只维护最终文档里的版本**，不要再回头维护本教程中的中间代码。

---

## 建议的学习方式

不要一次看完。按这个节奏来：

1. 一个阶段只做一件事。
2. 每个阶段都先 `cargo run`，再 `curl` 验证。
3. 每阶段改完后 `git commit` 一次，例如：
   - `stage01: hello axum`
   - `stage02: json response`
   - `stage03: config driven port`
4. 遇到问题时用 `git diff` 看这一阶段到底改了什么。
5. 阶段 6 和阶段 9 要特别认真看，它们分别演示了“错误写法”和“正确写法”。

这样走一遍，你不只是拿到一套能跑的代码，而是知道每一行为什么在那里。
