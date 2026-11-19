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
| 12.5 | Swagger UI | 浏览器打开 `/docs/` 看五个接口的文档 | 文档需随代码维护 |
| 12.8 | 文件下载 + 存在性查询 | 短名下载文件，POST 查询是否存在及数量 | 每次请求都扫目录，且没有 Range |
| 13 | 收口到最终版本 | 对照完整指南 | 生产要确认网络边界 |

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

CORS 只解决浏览器同源策略，**不是鉴权**。任何 curl / Postman 都可以无视 CORS 直接调接口。本方案不在代码里做鉴权，靠网络层隔离：`host` 绑具体内网 IP，或用防火墙只放行内网网段，上线前从外网验证端口不可达。

---

## 阶段 12.5：补一个 Swagger UI 页面

### 先回答一个常见疑问

**axum 不天然支持 Swagger UI。**

- FastAPI 自带 `/docs` 和 OpenAPI 生成。
- axum 只负责路由、提取器、中间件，定位更底层。
- 想要 Swagger UI，需要自己引入 OpenAPI 生成库和静态页面库。

常用组合：

- `utoipa`：用宏从 Rust 代码生成 OpenAPI JSON。
- `utoipa-swagger-ui`：把 Swagger UI 页面挂到 axum 路由上。

这个阶段现在属于**最终版本**：五个接口都有注解，浏览器打开就能看到完整文档。

### 加依赖

```bash
cargo add utoipa --features axum_extras
cargo add utoipa-swagger-ui --features axum,vendored
```

`axum_extras` 让 utoipa 忽略 `State`、`AxumPath` 这类 axum 专有提取器，只分析真正的请求体。

**`vendored` 很关键**：它会把 Swagger UI 的静态资源打包进二进制。如果不加，页面可能要从 CDN 拉 JS / CSS，内网机器没外网就打不开。本软件跑在内网，所以必须加。

### 给请求结构体加 `ToSchema`

在 `handler.rs`，三个请求体都加上 `ToSchema`：

```rust
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRequest {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchRequest {
    pub items: Vec<UpdateRequest>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckRequest {
    pub name: String,
    #[serde(rename = "type", default)]
    pub file_type: Option<String>,
}
```

`#[serde(rename = "type")]` 也会被 utoipa 识别，所以文档里显示的字段名就是 `type`，和实际请求一致。

### 给五个 handler 加接口注解

```rust
#[utoipa::path(
    get,
    path = "/api/config",
    tag = "config",
    responses(
        (status = 200, description = "读取成功"),
        (status = 500, description = "读取文档失败")
    )
)]
pub async fn read_config(/* ... */) -> Response { /* ... */ }

#[utoipa::path(
    post,
    path = "/api/config/update",
    tag = "config",
    request_body = UpdateRequest,
    responses(
        (status = 200, description = "修改成功"),
        (status = 400, description = "参数或白名单校验失败")
    )
)]
pub async fn update_one(/* ... */) -> Response { /* ... */ }

#[utoipa::path(
    post,
    path = "/api/config/batch-update",
    tag = "config",
    request_body = BatchRequest,
    responses(
        (status = 200, description = "批量修改成功"),
        (status = 400, description = "批量校验失败")
    )
)]
pub async fn batch_update(/* ... */) -> Response { /* ... */ }

#[utoipa::path(
    get,
    path = "/download/{name}",
    tag = "download",
    params(("name" = String, Path, description = "文件名或编号，单段")),
    responses(
        (status = 200, description = "文件字节流"),
        (status = 404, description = "未找到匹配文件"),
        (status = 409, description = "匹配到多个文件")
    )
)]
pub async fn download_file(/* ... */) -> Response { /* ... */ }

#[utoipa::path(
    post,
    path = "/api/download/check",
    tag = "download",
    request_body = CheckRequest,
    responses(
        (status = 200, description = "查询完成，结果见 exist / count / files"),
        (status = 400, description = "参数非法")
    )
)]
pub async fn check_download(/* ... */) -> Response { /* ... */ }
```

### 定义 OpenAPI 文档入口

在 `main.rs`：

```rust
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    paths(
        handler::read_config,
        handler::update_one,
        handler::batch_update,
        handler::download_file,
        handler::check_download
    ),
    components(schemas(
        handler::UpdateRequest,
        handler::BatchRequest,
        handler::CheckRequest
    )),
    tags(
        (name = "config", description = "配置文件读写接口"),
        (name = "download", description = "文件下载与存在性查询接口")
    )
)]
struct ApiDoc;
```

### 把页面合并进 Router

`merge` 要放在 `with_state` 之后、`layer` 之前：

```rust
let app = app
    .with_state(state)
    .merge(
        SwaggerUi::new("/docs")
            .url("/openapi.json", ApiDoc::openapi())
    )
    .layer(cors)
    .layer(TraceLayer::new_for_http());
```

当前版本（`utoipa-swagger-ui` 10.x）直接写 `SwaggerUi::new("/docs")` 即可，不需要 `/{_:.*}` 通配。

### 验证

```bash
cargo run
```

浏览器打开：

```text
http://127.0.0.1:18981/docs/
```

也可以直接拿 OpenAPI JSON：

```bash
curl http://127.0.0.1:18981/openapi.json
```

应该看到 5 个 path 和 3 个 schema。注意 URL 末尾的 `/` 不要省。

### 三个注意点

1. Swagger UI 页面和 API 同源，所以它本身不需要 CORS；CORS 是给外部前端页面用的。
2. Swagger UI 静态资源已通过 `vendored` 打进二进制，内网机器不用访问外网 CDN。
3. 本方案不做鉴权，页面和接口一样靠网络隔离保护。如果 `download.enabled = false`，下载接口没有注册，但 OpenAPI 里仍会列出它们——看文档时要留意实际配置。

### 本阶段的遗留问题

OpenAPI 文档和代码是“半自动同步”：结构体改了要重新生成，handler 注解也要跟着改。它很有用，但不是 axum 的内置能力。

---

## 阶段 12.8：加入文件下载接口

### 目标

目录里是 `data/downloads/20231123_164957_AS9283331.xlsx`，调用方只知道 `AS9283331.xlsx` 或 `AS9283331`（也知道类型只有 txt / csv / xls / xlsx），希望一个 GET 就能取到文件。再加一个 `POST /api/download/check`：先问“在不在、有几个”，拿到文件名列表后再决定要不要下载，它和下载接口共用同一套匹配规则。

明确一件事：**不做模糊搜索**。不做“文件名包含就算”、不做正则、不做任意段匹配。只做两级精确比较：

1. 请求名 == 文件完整文件名。
2. 请求名 == 文件名按 `_` 切分后的最后一段；如果请求没带扩展名，再允许和“末段去扩展名”比较，且扩展名必须在白名单里。

两条规则都命中（比如目录里同时有裸文件和带前缀文件）算**多个候选**，返回 409 并列出全名，绝不让服务端偷偷挑一个。挑错了对方拿到打不开的文件，还很难排查。

### 加依赖

```bash
cargo add tokio-util --features io
```

`ReaderStream` 把 `tokio::fs::File` 变成异步字节流，下载时不用把整个文件读进内存。

### 更新 `config.toml`

```toml
[download]
enabled = true
roots = ["./data/downloads"]                # 可配多个；相对路径基于 config.toml 解析
separator = "_"                             # 末段切分符
extensions = ["txt", "csv", "xls", "xlsx"]  # 小写、不带点；不在此列表的文件不参与匹配
case_insensitive = true                     # 统一转小写，抹平 Win / Linux 差异
max_file_size = 104857600                   # 100 MiB，0 表示不限制
allow_subdirs = false                       # 只扫顶层
follow_symlinks = false                     # 跳过符号链接
```

`extensions` 同时是安全白名单：`.bak`、`.tmp`、`.exe` 根本不进入候选集。这里没有 `strategy` 开关——行为已经定死，少一个配置项就少一种配错的可能。

### 新建 `src/download.rs`

这一层不碰 axum，只负责“校验请求名 + 收集候选”，**不负责挑选**。

```rust
use crate::config::DownloadConfig;
use std::fs;
use std::path::{Path, PathBuf};

/// 请求名必须是单段安全文件名。路由只匹配 {name}，这里再兜一层。
pub fn validate_requested_name(name: &str) -> Result<(), String> {
    if name.is_empty() { return Err("文件名不能为空".into()); }
    if name == "." || name == ".." { return Err("文件名不能是 . 或 ..".into()); }
    if name.contains('/') || name.contains('\\') { return Err("文件名不能包含路径分隔符".into()); }
    if name.chars().any(|c| c.is_control()) { return Err("文件名不能包含控制字符".into()); }
    // 完整版还会拦 Windows 保留设备名 CON / NUL / COM1 ...，见完整指南 5.4。
    Ok(())
}

fn strip_ext(name: &str) -> &str {          // 去掉最后一个点之后的部分
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

fn key(s: &str, ci: bool) -> String {       // 按配置统一大小写
    if ci { s.to_lowercase() } else { s.to_string() }
}

/// 收集所有命中文件；返回的路径都在某个 root 内。
/// type_filter 为 Some 时只用该扩展名（check 接口用），None 时用配置白名单。
pub fn find_candidates(
    cfg: &DownloadConfig, requested: &str, type_filter: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    let wanted = key(requested, cfg.case_insensitive);
    let wanted_stem = key(strip_ext(requested), cfg.case_insensitive);
    // 只有请求不带扩展名，才允许“去扩展名比较”，否则 AS9283331.xlsx 会误命中 .csv。
    let allow_stem = !requested.rfind('.').map(|i| i > 0).unwrap_or(false);
    let ext_filter: Vec<String> = match type_filter { // 生效的扩展名列表，统一小写
        Some(t) => vec![t.to_ascii_lowercase()],
        None => cfg.extensions.iter().map(|e| e.to_ascii_lowercase()).collect(),
    };
    let mut out = Vec::new();

    for root in &cfg.roots {
        let root_abs = Path::new(root).canonicalize()   // 解析成绝对路径
            .map_err(|e| format!("下载目录不可用: {}: {e}", root))?;
        scan(&root_abs, &root_abs, cfg, &ext_filter, &wanted, &wanted_stem, allow_stem, &mut out)?;
    }
    out.sort();                                        // 固定顺序，409 的列表才稳定
    out.dedup();                                       // 多个 root 指向同一文件时去重
    Ok(out)
}

fn scan(                                               // 递归扫描
    dir: &Path, root: &Path, cfg: &DownloadConfig, ext_filter: &[String],
    wanted: &str, wanted_stem: &str, allow_stem: bool,
    out: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        // 用 symlink_metadata 才看得出符号链接；默认跳过，防止跳到目录外。
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() && !cfg.follow_symlinks { continue; }

        if meta.is_dir() {
            if cfg.allow_subdirs {
                let child = path.canonicalize().map_err(|e| e.to_string())?;
                if child.starts_with(root) {           // 递归前也查一次越界
                    scan(&child, root, cfg, ext_filter, wanted, wanted_stem, allow_stem, out)?;
                }
            }
            continue;
        }
        if !meta.is_file() { continue; }               // 只要普通文件
        if cfg.max_file_size > 0 && meta.len() > cfg.max_file_size { continue; }

        let name = entry.file_name().to_string_lossy().into_owned();
        let ext = Path::new(&name).extension().and_then(|s| s.to_str())
            .unwrap_or("").to_ascii_lowercase();
        if !ext_filter.is_empty()
            && !ext_filter.iter().any(|e| e == &ext)
        { continue; }                                  // 生效的扩展名白名单

        let last = name.rsplit(cfg.separator.as_str()).next().unwrap_or(&name);
        let full_hit = key(&name, cfg.case_insensitive) == wanted;             // 规则 A
        let last_hit = key(last, cfg.case_insensitive) == wanted               // 规则 B-1
            || (allow_stem && key(strip_ext(last), cfg.case_insensitive) == wanted_stem); // B-2
        if !full_hit && !last_hit { continue; }

        let abs = path.canonicalize().map_err(|e| e.to_string())?;
        if abs.starts_with(root) { out.push(abs); }    // 命中后最终越界检查
    }
    Ok(())
}
```

### 改 `src/config.rs`

顶部配置结构体加一个字段：

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub files: FilesConfig,
    pub allowed_fields: AllowedFields,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]                            // 老配置没有 [download] 就是 None
    pub download: Option<DownloadConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadConfig {
    #[serde(default)]
    pub enabled: bool,
    pub roots: Vec<String>,
    #[serde(default = "default_separator")]
    pub separator: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default = "default_true")]
    pub case_insensitive: bool,
    #[serde(default)]
    pub max_file_size: u64,
    #[serde(default)]
    pub allow_subdirs: bool,
    #[serde(default)]
    pub follow_symlinks: bool,
}

fn default_separator() -> String { "_".to_string() }
fn default_true() -> bool { true }
```

本阶段的路径解析沿用完整指南的做法：在 `load()` 里先算出配置文件所在目录 `base`，再把 `base_dir` 和 `roots` 都基于它解析。完整 `load()` 如下（相比阶段 8 只多了 `base` 和 `roots` 两块）：

```rust
pub fn load(path: &Path) -> Result<Self> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let mut cfg: AppConfig = toml::from_str(&text)
        .with_context(|| format!("解析 TOML 失败: {}", path.display()))?;

    let base = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if Path::new(&cfg.files.base_dir).is_relative() {
        cfg.files.base_dir = base.join(&cfg.files.base_dir).to_string_lossy().into_owned();
    }
    if let Some(dl) = cfg.download.as_mut() {
        for root in &mut dl.roots {
            if Path::new(root).is_relative() {
                *root = base.join(&*root).to_string_lossy().into_owned();
            }
        }
    }

    cfg.validate()?;
    Ok(cfg)
}
```

在 `validate()` 里，只有 `enabled = true` 才严格校验：目录存在、`separator` 非空、扩展名小写且不带点。这样老配置升级后不会因为多了个可选段就启动失败。

### 改 `src/handler.rs`

顶部补充引用：

```rust
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};   // 别名避开 std::path::Path
use axum::http::header::CONTENT_DISPOSITION;
use axum::http::{HeaderValue, StatusCode};
use tokio::fs::File;
use tokio_util::io::ReaderStream;

use crate::download;
```

请求体结构体（`type` 是 Rust 关键字，用 serde 改名；`default` 让它可以省略）：

```rust
#[derive(Debug, Deserialize)]
pub struct CheckRequest {
    pub name: String,
    #[serde(rename = "type", default)]
    pub file_type: Option<String>,
}
```

新增 handler：

```rust
pub async fn download_file(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Some(dl) = state.config.download.as_ref().filter(|d| d.enabled) else {
        return not_found("下载接口未启用");
    };
    if let Err(e) = download::validate_requested_name(&name) {
        return bad_request(&format!("文件名非法: {e}"));
    }

    let candidates = match download::find_candidates(dl, &name, None) { // GET 不额外限定类型
        Ok(v) => v,
        Err(e) => return server_error(&e),
    };

    match candidates.len() {                       // 三态判定
        0 => not_found(&format!("未找到匹配文件: {name}")),
        1 => {
            let path = &candidates[0];
            let file = match File::open(path).await {
                Ok(f) => f,
                Err(e) => return server_error(&format!("打开下载文件失败: {e}")),
            };
            let mut resp = Response::new(Body::from_stream(ReaderStream::new(file)));
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("download");
            let disposition = format!("attachment; filename=\"{file_name}\"");
            resp.headers_mut().insert(
                CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition).unwrap_or(HeaderValue::from_static("attachment")),
            );
            resp
        }
        _ => {
            let list: Vec<String> = candidates.iter()
                .map(|p| p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string())
                .collect();
            (
                StatusCode::CONFLICT,
                Json(json!({
                    "ok": false,
                    "error": "匹配到多个文件，请使用完整文件名重试",
                    "candidates": list,
                })),
            ).into_response()
        }
    }
}

/// POST /api/download/check —— 只回答“在不在、有几个”，复用同一套匹配规则。
pub async fn check_download(
    State(state): State<AppState>,
    Json(req): Json<CheckRequest>,
) -> Response {
    let Some(dl) = state.config.download.as_ref().filter(|d| d.enabled) else {
        return not_found("下载接口未启用");
    };
    if let Err(e) = download::validate_requested_name(&req.name) {
        return bad_request(&format!("name 非法: {e}"));
    }
    // type 必须是白名单的子集，不能绕过 extensions 限制。
    if let Some(t) = &req.file_type {
        if !dl.extensions.iter().any(|e| e.eq_ignore_ascii_case(t.trim())) {
            return bad_request(&format!("type 不在允许列表中: {t}"));
        }
    }
    let type_filter = req.file_type.as_deref().map(|t| t.trim().to_ascii_lowercase());
    let candidates = match download::find_candidates(dl, &req.name, type_filter.as_deref()) {
        Ok(v) => v,
        Err(e) => return server_error(&e),
    };

    let mut files: Vec<String> = candidates.iter()
        .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(|s| s.to_string()))
        .collect();
    files.sort();
    files.dedup();                                     // 按文件名去重，不同 root 同名只算一个
    let count = files.len();
    let exist = count > 0;

    Json(json!({                                       // 固定三字段，exist=false 也是 200
        "exist": exist,
        "count": count,
        "files": files,
    })).into_response()
}

fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": msg }))).into_response()
}
```

### 改 `src/main.rs`

模块声明加 `mod download;`。路由要在 `with_state` 之前按开关决定是否注册，因为 `state` 随后会被移走：

```rust
let download_enabled = config
    .download.as_ref()
    .is_some_and(|d| d.enabled);
let state = AppState::new(config);                  // config 被移入 Arc

let app = Router::new()
    .route("/api/config", get(handler::read_config))
    .route("/api/config/update", post(handler::update_one))
    .route("/api/config/batch-update", post(handler::batch_update));

let app = if download_enabled {
    app.route("/download/{name}", get(handler::download_file))
       .route("/api/download/check", post(handler::check_download))
} else {
    app
};

let app = app
    .with_state(state)
    .layer(cors)
    .layer(TraceLayer::new_for_http());
```

### 验证

准备 `data/downloads/20231123_164957_AS9283331.xlsx`，然后：

```bash
# 末段 + 扩展名 → 200
curl -OJ http://127.0.0.1:8080/download/AS9283331.xlsx

# 只给 ID → 200（扩展名由白名单限定）
curl -OJ http://127.0.0.1:8080/download/AS9283331

# 完整文件名 → 200
curl -OJ http://127.0.0.1:8080/download/20231123_164957_AS9283331.xlsx
```

再放一个 `20231123_164957_AS9283331.csv`，只给 ID 的请求应该变成 409：

```json
{"ok":false,"error":"匹配到多个文件，请使用完整文件名重试","candidates":["20231123_164957_AS9283331.csv","20231123_164957_AS9283331.xlsx"]}
```

查询接口不下载文件，只返回清单，`files` 永远是数组：

```bash
curl -X POST http://127.0.0.1:8080/api/download/check \
  -H "Content-Type: application/json" \
  -d '{"name":"AS9283331","type":"xlsx"}'
```

```json
{"exist":true,"count":1,"files":["20231123_164957_AS9283331.xlsx"]}
```

`type` 省略时返回所有匹配类型；`exist: false` 时是 `{"exist":false,"count":0,"files":[]}`，状态码仍是 200。

### 本阶段的遗留问题

- 每次请求都重新扫描 `roots`，目录文件上千时会慢；最终版可以加启动期索引，但刷新时仍要做符号链接与越界检查。
- 不支持 Range，浏览器 PDF 内联预览会受限；当前用 `attachment` 强制下载，所以不阻塞主流程。
- 中文文件名还没做 RFC 5987 编码，完整指南里用 `filename*=` 补上了。
- 下载和 check 接口都不做代码鉴权，靠网络隔离：`host` 绑具体内网 IP，或用防火墙只放行内网网段；上线前从外网验证端口不可达。

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
13. Swagger UI / OpenAPI：五个接口自动生成文档，`vendored` 静态资源离线可用。
14. 文件下载与存在性查询：全名精确 + 末段精确两级匹配，多候选返回 409；check 接口复用同一套规则，带可选 `type` 过滤。

接下来请把教程代码**整体替换成最终版本**：

- 最终文档：[`Axum配置接口_完整实现与打包指南.md`](./Axum配置接口_完整实现与打包指南.md)
- 最终版本包含中间阶段没有补齐的内容：
  - 相对 `base_dir` 基于 `config.toml` 解析。
  - 启动期检查目标文档是否存在。
  - 重复的 `@字段` 直接拒绝修改。
  - 值不能包含换行或以 `@` 开头。
  - 统一的 JSON 错误响应。
  - 下载侧额外拦截 Windows 保留设备名，中文文件名按 RFC 5987 编码。
  - 下载候选路径的符号链接 / 越界检查与 `download.*` 日志留痕齐全。
  - Swagger UI 使用 `vendored` feature，静态资源随二进制分发，内网无需外网 CDN。
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
