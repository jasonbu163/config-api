# Axum 文档修改接口 · 完整实现与打包指南（config-api）

## 0. 项目目标与工具链对照

**功能**：Windows / Linux / macOS 上跑一个 HTTP 服务，通过接口修改指定目录下 txt 文档中 `@字段名` 下面的值。目录路径、文件名、可改字段白名单全部由 `config.toml` 控制。接口采用**方案 A**（单字段 + 批量 `items` 数组）。

**项目名**：`config-api`。

**学习路线**：想按「Hello World → 接口 → 配置 → 写入 → 日志 → CORS」逐步推导，请看 [`Axum配置接口_手把手教程.md`](./Axum配置接口_手把手教程.md)。本文件只保留最终版本方案。

| Python 侧 | Rust 侧 | 用途 |
|---|---|---|
| pyenv | rustup | 管工具链版本与编译目标 |
| uv | cargo | 建项目 / 加依赖 / 编译 / 运行 / 测试 |
| pyproject.toml + uv.lock | Cargo.toml + Cargo.lock | 依赖声明与锁定 |
| venv | 无（不需要） | Rust 无全局环境污染 |
| ruff + mypy | cargo clippy + cargo fmt | 代码质检与格式化 |
| structlog / JSON formatter | tracing-subscriber `json` feature | 结构化 JSON 日志 |

**分发认知**：Rust 编译产物绑定「CPU 架构 + 操作系统 + C 运行库」。

- MSVC 动态版：目标机通常需要 VC++ Redistributable。
- MSVC `+crt-static` 或 musl 静态版：目标机不需要额外运行库。
- macOS 产物只能在 macOS 上编译；Linux / Windows 各自在自己的平台编译最省事。

---

## 1. 环境准备（按你的主力系统）

### Windows（主力，推荐）

1. 安装 **Visual Studio Build Tools**（不是完整 VS），Installer 中勾选「**使用 C++ 的桌面开发**」工作负载。它提供 MSVC 的 `link.exe`，是 Windows 编译 Rust 的硬依赖。
2. 下载并运行 `rustup-init.exe`，一路默认回车（默认 MSVC 工具链），自动安装 `cargo` / `rustc` / `rustup` 并写入 PATH。
3. 新开终端验证：`rustc --version`、`cargo --version`、`rustup --version` 均有输出即成功。

### Linux

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

需要 `build-essential`（gcc、make）：

```bash
sudo apt install build-essential
```

musl 静态构建还需要：

```bash
sudo apt install musl-tools
```

### macOS

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
xcode-select --install   # 提供 clang 与系统 SDK
```

---

## 2. 项目初始化

```bash
cargo new config-api
cd config-api
```

生成的结构：

```text
config-api/
├── .gitignore          # 已自动忽略 /target
├── Cargo.toml          # 等价于 pyproject.toml
└── src/main.rs
```

`target/` 是全部编译产物目录，体积大且可重建，**永远不要提交进 git**。

> 本方案使用 Rust `edition = "2024"`，需要 Rust 1.85+。

---

## 3. 添加依赖（逐个说明为什么需要）

```bash
cargo add axum
cargo add tokio --features full
cargo add serde --features derive
cargo add serde_json
cargo add toml
cargo add tower-http --features cors,trace
cargo add tracing
cargo add tracing-subscriber --features env-filter,json
cargo add anyhow
cargo add tempfile
```

- **axum**：Web 框架，定位类似 FastAPI。本方案只处理 JSON，不需要 `multipart` feature。
- **tokio**：异步运行时；`full` 包含 `#[tokio::main]`、net、sync 等。
- **serde / serde_json**：请求体 JSON 与 Rust 结构体转换，承担 pydantic 的解析职责。
- **toml**：解析 `config.toml`。
- **tower-http**：中间件集合；`trace` 输出每个请求的 span / 事件，`cors` 处理浏览器跨域和预检请求。
- **tracing**：在业务代码里写 `tracing::info!` 等结构化事件。
- **tracing-subscriber**：日志输出实现；`json` feature 让日志变成 JSON Lines。
- **anyhow**：简化应用层错误链。
- **tempfile**：写临时文件并 `persist` 到目标路径，实现跨平台原子替换。

接口可能由浏览器页面调用，因此默认启用 CORS，但**不允许硬编码 `*` 加凭证**。允许来源写在 `config.toml` 的 `[cors]` 段里，preflight `OPTIONS` 由 `CorsLayer` 自动处理，不需要单独写路由。

再次强调：CORS 只解决浏览器同源策略，不是鉴权。一旦监听 `0.0.0.0`，仍必须在上线前加访问控制。

首次 `cargo build` 下载并编译依赖需要几分钟属正常现象；只做类型检查用 `cargo check`，更快。

---

## 4. 配置文件 `config.toml`

放在项目根目录（与 `Cargo.toml` 同级）。**Windows 路径中的反斜杠必须双写**，因为 TOML 字符串里 `\` 是转义符。

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

[cors]
# 浏览器页面所在来源；留空数组表示只允许同源调用，不给跨域响应头。
allowed_origins = ["http://localhost:5173", "http://127.0.0.1:5173"]
# 只有使用 Cookie / Authorization 等凭证时才设为 true；此时 allowed_origins 不能包含 "*"。
allow_credentials = false
```

### 路径规则

- `base_dir` 如果是相对路径，**相对于本 `config.toml` 所在目录解析**，不是相对于当前工作目录。
- `file_name` 必须是文件名，不允许写绝对路径。
- 启动时会检查目标文档是否存在；不存在则直接退出，不等到请求来了再报错。

### CORS 规则

- `allowed_origins = []`：不启用跨域响应头，浏览器只允许同源调用。
- `allowed_origins = ["*"]`：允许任意来源；仅适合无鉴权的内网工具演示。
- `allow_credentials = true` 时，`allowed_origins` 必须是明确来源，不能写 `"*"`。
- 预检请求（`OPTIONS`）由 `CorsLayer` 自动应答，应用代码不需要写 `/api/...` 的 `options` 路由。

### 文档格式

被修改的 txt 采用「`@字段名` 换行 + 值」的成对格式：

```text
# 设备配置，请勿手改
@DataName
TT26071311A002

@Thickness
25
@Temper
T3
```

解析规则：

- 以 `@` 开头的行是字段名，紧随其后的下一行是值。
- 注释、空行、其他散行**原样保留**，只替换目标字段的值行。
- 行尾风格（LF / CRLF）按原文件保留，不强制重排。
- 同一个字段出现多次时拒绝修改，避免歧义。
- 值不能包含 `\n` / `\r`，去掉前导空白后不能以 `@` 开头。

---

## 5. 完整代码（逐行注释解析）

下面代码按「尽量逐行注释」的方式给出。注释解释**为什么这么写**，可以直接保留；熟手可删掉注释，不影响编译。

### 5.1 `src/config.rs` —— 配置加载与校验

```rust
use anyhow::{anyhow, Context, Result}; // anyhow! 造错；Context 给错误加上下文；Result 是 anyhow 的别名
use serde::Deserialize;                // 提供 #[derive(Deserialize)]，让结构体能从 TOML 反序列化
use std::path::{Path, PathBuf};        // Path 是借用路径；PathBuf 是拥有所有权的路径

/// 顶层配置结构：对应 config.toml 里的四个段
#[derive(Debug, Clone, Deserialize)] // Debug 便于打印；Clone 便于 Arc 共享；Deserialize 用于解析 TOML
#[serde(deny_unknown_fields)]        // 配置里写错键名直接启动失败，而不是被静默忽略
pub struct AppConfig {               // 聚合所有配置，供服务全局只读使用
    pub server: ServerConfig,        // [server] 监听地址与端口
    pub files: FilesConfig,          // [files] 目标文档位置与备份开关
    pub allowed_fields: AllowedFields, // [allowed_fields] 可写字段白名单
    #[serde(default)]                // 老配置没有 [cors] 段时，用 Default 兜底
    pub cors: CorsConfig,            // [cors] 浏览器跨域配置
}

/// 服务监听配置
#[derive(Debug, Clone, Deserialize)] // 同上：可打印、可克隆、可反序列化
#[serde(deny_unknown_fields)]        // 写错 host / port 之外的键会报错
pub struct ServerConfig {            // 只放启动服务必需的两项
    pub host: String,                // IP 字符串，如 "127.0.0.1" 或 "0.0.0.0"
    pub port: u16,                   // 端口用 u16，天然排除负数与 65535 以上的值
}

/// 目标文档配置
#[derive(Debug, Clone, Deserialize)] // 可打印、可克隆、可反序列化
#[serde(deny_unknown_fields)]        // 拼错的键不会被忽略
pub struct FilesConfig {             // 描述“改哪个文件”
    pub base_dir: String,            // 目录；相对路径会在 load() 里改写为基于配置文件目录
    pub file_name: String,           // 文件名；不允许绝对路径，避免越出 base_dir
    #[serde(default)]                // 旧配置没有 backup 时默认为 false
    pub backup: bool,                // 修改前是否写 .bak
}

/// 字段白名单
#[derive(Debug, Clone, Deserialize)] // 可打印、可克隆、可反序列化
#[serde(deny_unknown_fields)]        // 多写字段会报错
pub struct AllowedFields {           // 控制“能改哪些字段”
    pub fields: Vec<String>,         // 字段名列表，空列表在校验阶段拒绝
}

/// 浏览器跨域配置
#[derive(Debug, Clone, Default, Deserialize)] // Default 让缺失的 [cors] 等价于空配置
#[serde(deny_unknown_fields)]                // 拼错 cors 键会报错
pub struct CorsConfig {                      // 只放 CORS 必需的两项
    #[serde(default)]                        // 省略 allowed_origins 时视为 []
    pub allowed_origins: Vec<String>,        // 允许的 Origin；[] = 不启用跨域响应头
    #[serde(default)]                        // 省略 allow_credentials 时视为 false
    pub allow_credentials: bool,             // 是否允许 Cookie / Authorization 等凭证
}

impl AppConfig {                     // 把“加载 + 校验 + 取值”集中在这个 impl 里
    /// 读取配置，解析相对路径，然后立即做业务校验。
    pub fn load(path: &Path) -> Result<Self> { // 入参是配置文件路径；返回配置或错误
        let text = std::fs::read_to_string(path) // 一次性读成 UTF-8 字符串
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?; // 读失败附路径，? 提前返回
        let mut cfg: AppConfig = toml::from_str(&text) // 解析 TOML；mut 是因为后面要重写相对路径
            .with_context(|| format!("解析 TOML 失败: {}", path.display()))?; // 解析失败附路径

        // 相对 base_dir 基于配置文件所在目录解析，避免双击 exe 时 cwd 不确定。
        let base = path // 从配置文件路径开始
            .parent() // 取所在目录；如果传的是 "config.toml"，这里会得到 ""
            .filter(|p| !p.as_os_str().is_empty()) // 空目录等价于当前目录，过滤掉
            .unwrap_or_else(|| Path::new(".")); // 极端情况下回退到 "."
        if Path::new(&cfg.files.base_dir).is_relative() { // 只有相对路径才需要改写
            cfg.files.base_dir = base // 用配置文件目录作为基准
                .join(&cfg.files.base_dir) // 拼出实际目录
                .to_string_lossy() // 路径转字符串；非 UTF-8 字节会被替换成 U+FFFD
                .into_owned(); // 从 Cow 拿到 String 所有权
        }

        cfg.validate()?; // 启动期校验；失败则整个 main 直接退出
        Ok(cfg)          // 校验通过，返回最终配置
    }

    /// 启动期校验：宁可启动失败，也不要运行到一半才出错。
    fn validate(&self) -> Result<()> { // &self 只读借用，不修改配置
        if self.server.port == 0 { // 0 号端口不能用于监听
            return Err(anyhow!("server.port 不能为 0")); // 立刻返回带上下文的错误
        }
        if self.files.file_name.trim().is_empty() { // 防止全空格文件名
            return Err(anyhow!("files.file_name 不能为空")); // 明确报错
        }
        if Path::new(&self.files.file_name).is_absolute() { // 防止用绝对路径跳出 base_dir
            return Err(anyhow!("files.file_name 只能是文件名，不能是绝对路径")); // 拒绝启动
        }
        if self.allowed_fields.fields.is_empty() { // 白名单为空等于所有写请求都拒绝
            return Err(anyhow!("allowed_fields.fields 不能为空")); // 提前暴露配置错误
        }
        if self.cors.allow_credentials // 如果要求浏览器带凭证
            && self.cors.allowed_origins.iter().any(|o| o == "*") // 同时又允许任意来源
        { // 这种组合浏览器规范不允许，tower-http 也会拒绝
            return Err(anyhow!("cors.allow_credentials=true 时 allowed_origins 不能包含 *")); // 启动期拦住
        }

        let dir = Path::new(&self.files.base_dir); // 把字符串目录转成 Path 视图
        if !dir.is_dir() { // 目录不存在或不是目录
            return Err(anyhow!("base_dir 不存在或不是目录: {}", dir.display())); // 报出实际路径
        }
        let file = self.config_file_path(); // 拼出目标文档完整路径
        if !file.is_file() { // 文件不存在或不是普通文件
            return Err(anyhow!("目标文档不存在或不是文件: {}", file.display())); // 不等请求来了再失败
        }
        Ok(()) // 所有校验通过
    }

    pub fn config_file_path(&self) -> PathBuf { // 返回拥有所有权的目标路径
        Path::new(&self.files.base_dir).join(&self.files.file_name) // 目录 + 文件名
    } // 函数结束，返回 PathBuf

    pub fn is_allowed(&self, name: &str) -> bool { // 判断字段是否在白名单里
        self.allowed_fields.fields.iter().any(|f| f == name) // 任意一个相等即允许
    } // 函数结束，返回 bool
}
```

### 5.2 `src/state.rs` —— 共享状态与读写锁

```rust
use crate::config::AppConfig; // 引入上一步定义的配置结构体
use std::sync::Arc;           // Arc 让多个线程/任务安全共享同一个值
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard}; // 异步读写锁及两种 guard

/// axum 的 State 要求内部类型可 Clone 且克隆廉价，
/// 因此用 Arc 包住只读配置，供所有任务共享。
#[derive(Clone)] // 派生 Clone：克隆的是 Arc 指针，不是整份配置
pub struct AppState {         // 全局共享状态
    pub config: Arc<AppConfig>, // 配置只读，多任务通过 Arc 共享
    /// 读写锁：写请求互斥；读请求也拿读锁，保证不读到替换中途的状态。
    rw_lock: Arc<RwLock<()>>, // 锁里放 ()，因为我们只需要“互斥”本身，不保护具体数据
}

impl AppState {              // 构造函数和加锁方法
    pub fn new(config: AppConfig) -> Self { // 接收配置，构造共享状态
        Self {                             // 返回结构体实例
            config: Arc::new(config),      // 把配置放进 Arc
            rw_lock: Arc::new(RwLock::new(())), // 把 () 放进异步 RwLock，再放进 Arc
        }                                  // 结构体字段初始化结束
    }                                      // new 结束

    pub async fn lock_read(&self) -> RwLockReadGuard<'_, ()> { // 获取读锁；异步等待
        self.rw_lock.read().await // await 直到拿到读锁，返回 guard
    }                             // guard 离开作用域自动释放

    pub async fn lock_write(&self) -> RwLockWriteGuard<'_, ()> { // 获取写锁；异步等待
        self.rw_lock.write().await // await 直到其他读写锁释放，返回 guard
    }                              // guard 离开作用域自动释放
}
```

### 5.3 `src/parser.rs` —— 文档解析与最小改动写入（核心）

**不要整篇 `render()` 重生成文档**：那会丢注释、空行、行尾风格。这里只替换目标字段的值行，其他内容原样保留。

```rust
use std::error::Error; // 让自定义错误能当成标准错误使用
use std::fmt;          // 实现 Display 需要这个模块

/// 一条配置项：@key 与其下一行的 value。仅用于 GET 展示。
#[derive(Debug, Clone, PartialEq, Eq)] // 便于打印、克隆和测试断言
pub struct Entry {                     // 一个字段 + 一个值
    pub key: String,                   // 字段名，不含前面的 @
    pub value: String,                 // 字段下一行的值
}                                      // 结构体结束

/// 解析文档为结构化条目；非 @ 行被跳过，但写回时不会删除。
pub fn parse(content: &str) -> Vec<Entry> { // 输入文档全文，输出字段列表
    let lines: Vec<&str> = content.lines().collect(); // 按行切分；lines() 会吃掉 \n 和 \r\n
    let mut entries = Vec::new();                     // 准备装解析结果
    let mut i = 0;                                    // 当前扫描到的行号

    while i < lines.len() {                           // 逐行扫描，直到末尾
        let line = lines[i].trim_end_matches('\r');    // 再去掉个别残留的 \r，兼容 Windows
        if let Some(key) = line.strip_prefix('@') {    // 只有 @ 开头才是字段行
            let value = lines                          // 值在下一行
                .get(i + 1)                            // 用 get 避免越界 panic
                .map(|v| v.trim_end_matches('\r').to_string()) // 去 \r 并转成拥有所有权的 String
                .unwrap_or_default();                  // 文件以 @key 结尾时，值视为空串
            entries.push(Entry {                       // 记录一条字段
                key: key.trim().to_string(),           // 去掉字段名两侧空白
                value,                                 // 上一步取到的值
            });                                        // push 结束
            i += 2;                                    // 字段行 + 值行都处理完，跳两行
        } else {                                       // 注释、空行、散行
            i += 1;                                    // 不关心，只看下一行
        }                                              // if / else 结束
    }                                                  // while 结束
    entries                                            // 返回字段列表
}                                                      // parse 结束

/// 去掉行尾的 \n / \r\n，只保留行内容。
fn body(line: &str) -> &str { // 输入可能带行尾，输出纯内容
    let line = line.strip_suffix('\n').unwrap_or(line); // 先去掉 \n，没有就原样
    line.strip_suffix('\r').unwrap_or(line)             // 再去掉 \r，兼容 CRLF
}                                                       // 返回切片，不分配新字符串

/// 取原行尾，写回时保持不变。
fn ending(line: &str) -> &str { // 返回 "\r\n"、"\n" 或 ""
    if line.ends_with("\r\n") { // Windows 行尾
        "\r\n"                  // 原样返回 CRLF
    } else if line.ends_with('\n') { // Unix 行尾
        "\n"                    // 返回 LF
    } else {                    // 最后一行没有行尾
        ""                      // 返回空串，不凭空加换行
    }                           // if / else 结束
}                               // ending 结束

/// 自定义错误：让 handler 能区分客户端错误，而不是全归为 500。
#[derive(Debug)] // 便于日志打印
pub enum ApplyError {           // 所有“改文档”可能失败的原因
    InvalidValue(String),      // 值里有换行或以 @ 开头，字段名存在 String 里
    MissingField(String),      // 文档里找不到该字段
    DuplicateField(String),    // 同名字段出现多次，拒绝猜
    MissingValueLine(String),  // @key 后面没有值行
}                              // enum 结束

impl fmt::Display for ApplyError { // 实现 Display，才能写 e.to_string()
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { // 格式化入口
        match self {             // 按错误类型分支
            Self::InvalidValue(name) => { // 解出字段名
                write!(f, "字段 {name} 的值包含非法字符（换行或以 @ 开头）") // 写错误文案
            }                    // 分支结束
            Self::MissingField(name) => write!(f, "文档中不存在字段: {name}"), // 字段不存在
            Self::DuplicateField(name) => write!(f, "字段 {name} 在文档中重复出现，拒绝修改"), // 重复字段
            Self::MissingValueLine(name) => write!(f, "字段 {name} 缺少值行"), // 缺值行
        }                        // match 结束
    }                            // fmt 结束，返回 fmt::Result
}                                // impl 结束

impl Error for ApplyError {}    // 标记为标准错误，便于接入 anyhow / 日志

/// 应用修改，返回新文档文本和实际修改的字段名。
/// 所有校验先完成，再返回新文本；由调用方做原子替换。
pub fn apply_to_text(                       // 主函数：返回 (新文本, 已改字段)
    content: &str,                          // 原文档全文
    updates: &[(String, String)],           // 待改列表：(字段名, 新值)
) -> Result<(String, Vec<String>), ApplyError> { // 成功返回文本+字段名，失败返回 ApplyError
    for (name, value) in updates {          // 第一遍：先校验所有值，避免改到一半才失败
        if value.contains('\n') || value.contains('\r') || value.trim_start().starts_with('@') { // 换行或伪字段行都危险
            return Err(ApplyError::InvalidValue(name.clone())); // 立即返回客户端错误
        }                                   // if 结束
    }                                       // for 结束

    let mut lines: Vec<String> = content.split_inclusive('\n').map(str::to_string).collect(); // 按行切且保留行尾
    let mut applied = Vec::new();            // 记录实际改了哪些字段

    for (name, value) in updates {           // 第二遍：逐字段处理
        let mut hits = 0usize;               // 记录该字段命中几处
        let mut i = 0usize;                  // 当前行号

        while i < lines.len() {              // 扫描全文
            let is_target = body(&lines[i])  // 取当前行纯内容
                .strip_prefix('@')           // 只看 @ 开头
                .map(str::trim)              // 去掉字段名两侧空白
                == Some(name.as_str());      // 和请求里的字段名比较

            if !is_target {                  // 不是目标字段
                i += 1;                      // 看下一行
                continue;                    // 跳过后续处理
            }                                // if 结束

            hits += 1;                       // 命中一次
            if i + 1 >= lines.len() || body(&lines[i + 1]).starts_with('@') { // 没有值行，或下一行又是字段行
                return Err(ApplyError::MissingValueLine(name.clone())); // 文档格式不合法
            }                                // if 结束

            let e = ending(&lines[i + 1]);   // 取出原值行的行尾
            lines[i + 1] = format!("{value}{e}"); // 只替换值内容，行尾保持原样
            i += 2;                          // 字段行和值行都处理过，跳两行
        }                                    // while 结束

        match hits {                         // 根据命中次数决定成败
            0 => return Err(ApplyError::MissingField(name.clone())), // 没找到字段
            1 => applied.push(name.clone()), // 正好一次，记录已改
            _ => return Err(ApplyError::DuplicateField(name.clone())), // 多次，拒绝修改
        }                                    // match 结束
    }                                        // for 结束

    Ok((lines.concat(), applied))            // 把行重新拼成文档，连同已改字段返回
}                                            // apply_to_text 结束
```

### 5.4 `src/handler.rs` —— 三个接口

```rust
use axum::extract::{Json, State}; // Json 解析请求体；State 取出共享状态
use axum::http::StatusCode;       // HTTP 状态码，如 400 / 500
use axum::response::{IntoResponse, Response}; // 把元组 / Json 转成统一 Response
use serde::Deserialize;           // 请求体反序列化派生宏
use serde_json::json;             // 快速构造 JSON 值
use std::fs;                      // 同步文件读写
use std::io::Write;               // write_all 方法所在 trait
use std::path::Path;              // 路径借用类型
use tempfile::NamedTempFile;      // 同目录临时文件，用于原子替换

use crate::parser;                // 文档解析与最小改动写入模块
use crate::state::AppState;       // 共享状态类型

/// 单字段请求体：{ "name": "DataName", "value": "TT26071311A002" }
#[derive(Debug, Deserialize)]     // 可打印；从 JSON 请求体反序列化
pub struct UpdateRequest {        // 对应单字段更新接口
    pub name: String,             // 字段名，必须在白名单里
    pub value: String,            // 新值，不能含换行或以 @ 开头
}                                 // 结构体结束

/// 批量请求体（方案 A）：{ "items": [ {name, value}, ... ] }
#[derive(Debug, Deserialize)]     // 同上
pub struct BatchRequest {         // 对应批量更新接口
    pub items: Vec<UpdateRequest>, // 多条更新；空数组会被拒绝
}                                 // 结构体结束

/// GET /api/config —— 读取当前文档解析结果
pub async fn read_config(State(state): State<AppState>) -> Response { // axum handler 签名
    let _guard = state.lock_read().await; // 拿读锁；下划线开头表示只用它的生命周期
    let path = state.config.config_file_path(); // 得到目标文档完整路径

    match fs::read_to_string(&path) { // 同步读取；失败走 Err 分支
        Ok(content) => {              // 读取成功
            let fields: Vec<_> = parser::parse(&content) // 解析出字段列表
                .iter()                                 // 借每个 Entry
                .map(|e| json!({ "name": e.key, "value": e.value })) // 转成 JSON 对象
                .collect();                             // 收集成 Vec
            Json(json!({                                // 构造成功响应体
                "ok": true,                             // 业务成功标志
                "file": path.display().to_string(),     // 文档路径，便于前端核对
                "fields": fields,                       // 字段数组
            }))                                         // Json<Value>
            .into_response()                            // 转成统一 Response
        }                                               // Ok 分支结束
        Err(e) => server_error(&format!("读取文档失败: {e}")), // 读失败返回 500 JSON
    }                                                   // match 结束
}                                                       // read_config 结束

/// POST /api/config/update —— 修改单个字段
pub async fn update_one(                                // handler 签名
    State(state): State<AppState>,                      // 注入共享状态
    Json(req): Json<UpdateRequest>,                     // 注入解析后的请求体
) -> Response {                                         // 返回统一 Response
    if let Some(resp) = validate_one(&state, &req) {     // 白名单校验；如果是 Some 说明失败
        return resp;                                    // 直接返回错误响应
    }                                                   // if 结束
    match apply_updates(&state, vec![(req.name, req.value)]).await { // 包装成 updates 列表
        Ok(body) => Json(body).into_response(),         // 成功返回 JSON
        Err(resp) => resp,                              // 失败返回已构造好的错误响应
    }                                                   // match 结束
}                                                       // update_one 结束

/// POST /api/config/batch-update —— 批量修改
/// 语义：白名单校验 → 生成新文本 → 备份 → 原子替换；任一步失败则原文档不变。
pub async fn batch_update(                              // handler 签名
    State(state): State<AppState>,                      // 注入共享状态
    Json(req): Json<BatchRequest>,                      // 注入批量请求体
) -> Response {                                         // 返回统一 Response
    if req.items.is_empty() {                           // 空批量没有意义
        return bad_request("items 不能为空");            // 返回 400
    }                                                   // if 结束
    for item in &req.items {                            // 先逐条校验白名单
        if let Some(resp) = validate_one(&state, item) { // 发现非法字段
            return resp;                                // 整体失败，不写文档
        }                                               // if 结束
    }                                                   // for 结束
    let updates: Vec<(String, String)> = req            // 把请求体转换成 parser 需要的格式
        .items                                          // 取出 Vec<UpdateRequest>
        .into_iter()                                    // 消费迭代器，避免克隆
        .map(|i| (i.name, i.value))                     // 拆成 (字段名, 新值)
        .collect();                                     // 收集成 Vec

    match apply_updates(&state, updates).await {        // 统一走写入流程
        Ok(body) => Json(body).into_response(),         // 成功返回 JSON
        Err(resp) => resp,                              // 失败返回错误响应
    }                                                   // match 结束
}                                                       // batch_update 结束

/// 单条校验：这里只做白名单；值合法性由 parser 统一负责。
fn validate_one(state: &AppState, req: &UpdateRequest) -> Option<Response> { // 失败返回 Some(响应)
    if !state.config.is_allowed(&req.name) {            // 查白名单
        return Some(bad_request(&format!("字段不在白名单中: {}", req.name))); // 400 + JSON
    }                                                   // if 结束
    None                                                // 通过校验
}                                                       // validate_one 结束

/// 加锁 → 读 → 改 → 备份 → 临时文件 → 原子替换。
async fn apply_updates(                                 // 写文档的唯一入口
    state: &AppState,                                   // 共享状态借用
    updates: Vec<(String, String)>,                     // 待改列表
) -> Result<serde_json::Value, Response> {              // 成功返回 JSON，失败返回 Response
    let _guard = state.lock_write().await;              // 拿写锁；guard 存活到函数结束

    let path = state.config.config_file_path();         // 目标文档路径
    let content = fs::read_to_string(&path)             // 读取原内容，供 parser 使用
        .map_err(|e| server_error(&format!("读取文档失败: {e}")))?; // 读失败转 500

    let (new_content, applied) = parser::apply_to_text(&content, &updates) // 最小改动写入
        .map_err(|e| bad_request(&e.to_string()))?;     // 客户端错误转 400

    // 备份失败默认阻断主流程，避免“改成功了但没有回滚点”。
    if state.config.files.backup {                      // 配置要求备份
        let bak = path.with_extension("txt.bak");       // 生成 .bak 路径
        fs::write(&bak, &content)                       // 写入修改前的原内容
            .map_err(|e| server_error(&format!("写备份失败: {}: {}", bak.display(), e)))?; // 失败转 500
    }                                                   // if 结束

    let parent = path.parent().unwrap_or_else(|| Path::new(".")); // 临时文件必须与目标同目录
    let mut tmp = NamedTempFile::new_in(parent)         // 在目标目录创建临时文件
        .map_err(|e| server_error(&format!("创建临时文件失败: {e}")))?; // 失败转 500
    tmp.write_all(new_content.as_bytes())               // 把新文档写进临时文件
        .map_err(|e| server_error(&format!("写临时文件失败: {e}")))?; // 失败转 500
    tmp.as_file_mut()                                   // 拿到底层 &File
        .sync_all()                                     // 强制刷盘，降低断电丢数据风险
        .map_err(|e| server_error(&format!("同步临时文件失败: {e}")))?; // 失败转 500
    tmp.persist(&path)                                  // 原子替换目标文档
        .map_err(|e| server_error(&format!("替换文档失败: {}", e.error)))?; // 失败转 500

    tracing::info!(                                     // 结构化业务日志
        action = "config.updated",                      // 事件名，便于日志检索
        file = %path.display(),                         // 被修改的文档路径
        fields = ?applied,                              // 已改字段名列表，Debug 格式
        count = applied.len(),                          // 修改数量
        "文档更新成功"                                    // 日志正文
    );                                                  // 宏调用结束

    Ok(json!({                                          // 构造成功响应
        "ok": true,                                     // 业务成功标志
        "file": path.display().to_string(),             // 文档路径
        "applied": applied,                             // 实际修改的字段
        "count": applied.len(),                         // 修改数量
    }))                                                 // Ok(Value)
}                                                       // apply_updates 结束

fn bad_request(msg: &str) -> Response {                 // 统一 400 JSON 错误
    (                                                   // 元组：(状态码, Json)
        StatusCode::BAD_REQUEST,                        // 400 Bad Request
        Json(json!({ "ok": false, "error": msg })),     // 错误体
    )                                                   // 元组结束
        .into_response()                                // 转成 Response
}                                                       // bad_request 结束

fn server_error(msg: &str) -> Response {                // 统一 500 JSON 错误
    (                                                   // 元组：(状态码, Json)
        StatusCode::INTERNAL_SERVER_ERROR,              // 500 Internal Server Error
        Json(json!({ "ok": false, "error": msg })),     // 错误体
    )                                                   // 元组结束
        .into_response()                                // 转成 Response
}                                                       // server_error 结束
```

### 5.5 `src/main.rs` —— 入口、路由、CORS 与 JSON 日志

```rust
// Rust 不会自动发现源文件，模块必须显式声明。
mod config;   // 配置加载与校验
mod handler;  // HTTP handler
mod parser;   // 文档解析与最小改动写入
mod state;    // 共享状态与读写锁

use anyhow::Context; // 给错误附加上下文的 trait
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE}; // CORS 允许的请求头
use axum::http::{HeaderValue, Method}; // Origin 解析类型 + HTTP 方法枚举
use axum::routing::{get, post};        // 路由方法构造器
use axum::Router;                      // 路由器类型
use std::net::{IpAddr, SocketAddr};    // IP 与 IP+端口
use tower_http::cors::{Any, CorsLayer}; // CORS 中间件
use tower_http::trace::TraceLayer;      // 请求日志中间件
use tracing_subscriber::EnvFilter;      // 从 RUST_LOG 读取日志级别

/// 根据配置构造 CORS 中间件；非法 Origin 会让服务启动失败。
fn build_cors(config: &config::AppConfig) -> anyhow::Result<CorsLayer> { // 返回 CorsLayer 或错误
    if config.cors.allowed_origins.is_empty() { // 空数组：只允许同源，不加 CORS 响应头
        return Ok(CorsLayer::new());            // 返回空 layer，不影响同源请求
    }                                           // if 结束

    let methods = [Method::GET, Method::POST, Method::OPTIONS]; // 允许的 HTTP 方法
    let headers = [AUTHORIZATION, CONTENT_TYPE];                // 允许浏览器带 Authorization 和 Content-Type

    if config.cors.allowed_origins.iter().any(|o| o == "*") { // 配置了通配符
        let layer = CorsLayer::new() // 从空 layer 开始
            .allow_origin(Any)      // 允许任意来源；仅适合无凭证场景
            .allow_methods(methods.clone()) // 克隆数组；后面明确来源分支还要用
            .allow_headers(headers.clone()); // 同理克隆请求头数组
        return Ok(layer);           // allow_credentials 已被配置校验排除，这里安全
    }                               // if 结束

    let origins = config.cors.allowed_origins.iter() // 遍历明确来源列表
        .map(|o| o.parse::<HeaderValue>())           // 每个 Origin 解析成 HeaderValue
        .collect::<Result<Vec<_>, _>>()              // 收集；遇到非法值就返回 Err
        .context("cors.allowed_origins 含有非法 Origin")?; // 加上下文并用 ? 冒泡

    let layer = CorsLayer::new()                      // 从空 layer 开始
        .allow_origin(origins)                        // 只放行列表里的来源
        .allow_methods(methods)                       // 允许 GET / POST / OPTIONS
        .allow_headers(headers)                       // 允许指定请求头
        .allow_credentials(config.cors.allow_credentials); // 是否允许凭证
    Ok(layer)                                         // 返回构造好的 CORS layer
}                                                     // build_cors 结束

#[tokio::main]                                      // 宏：启动 Tokio 异步运行时
async fn main() -> anyhow::Result<()> {              // 程序入口；错误由 main 返回
    // JSON Lines 日志。RUST_LOG 可覆盖默认值。
    // tower_http=debug 才能看到 TraceLayer 的每次请求日志。
    tracing_subscriber::fmt()                       // 创建日志构建器
        .json()                                     // 输出 JSON Lines，不是普通文本
        .with_env_filter(                           // 设置过滤规则
            EnvFilter::try_from_default_env()       // 优先读环境变量 RUST_LOG
                .unwrap_or_else(|_| "info,tower_http=debug".into()), // 没设置时用默认值
        )                                           // with_env_filter 结束
        .with_target(false)                         // 不在每行打 target，日志更短
        .init();                                    // 注册为全局 subscriber，只能调用一次

    // 配置文件路径：第一个命令行参数可覆盖，默认 ./config.toml。
    let cfg_path = std::env::args()                 // 读取命令行参数迭代器
        .nth(1)                                     // 取第一个参数（第 0 个是程序名）
        .unwrap_or_else(|| "config.toml".to_string()); // 没有就用默认路径
    let config = config::AppConfig::load(std::path::Path::new(&cfg_path)) // 加载并校验配置
        .context("加载配置失败，服务未启动")?;       // 失败直接退出并打印原因

    let ip: IpAddr = config                         // 从配置取 host
        .server                                     // server 段
        .host                                       // host 字符串
        .parse()                                    // 解析成 IP 字面量
        .with_context(|| format!("server.host 不是合法 IP: {}", config.server.host))?; // 失败附原因
    let addr = SocketAddr::new(ip, config.server.port); // 组合 IP 和端口

    let cors = build_cors(&config)?;                // 在 config 被移动前构造 CORS layer
    let target = config.config_file_path();         // 先记下目标文档路径用于日志
    let state = state::AppState::new(config);       // config 被移入 Arc，之后不能再直接用

    let app = Router::new()                         // 构造路由器
        .route("/api/config", get(handler::read_config)) // GET：读取文档
        .route("/api/config/update", post(handler::update_one)) // POST：单字段更新
        .route("/api/config/batch-update", post(handler::batch_update)) // POST：批量更新
        .with_state(state)                          // 注入共享状态
        .layer(cors)                                // 挂 CORS；OPTIONS 预检由它自动应答
        .layer(TraceLayer::new_for_http());         // 挂请求日志；配合 tower_http=debug

    let listener = tokio::net::TcpListener::bind(addr) // 绑定监听地址
        .await                                      // 异步等待绑定完成
        .with_context(|| format!("绑定地址失败: {addr}"))?; // 端口占用等错误附地址

    tracing::info!(file = %target.display(), "目标文档"); // JSON 日志：目标文档
    tracing::info!(addr = %addr, "服务已启动");           // JSON 日志：监听地址

    axum::serve(listener, app).await?;              // 启动 HTTP 服务，直到进程退出
    Ok(())                                          // 正常退出返回 Ok
}                                                   // main 结束
```

### 5.6 `Cargo.toml` 的 release 优化段

在 `[dependencies]` 之后追加：

```toml
[profile.release]
lto = true          # 链接期优化，减小体积、提升性能
codegen-units = 1   # 单代码生成单元，优化更彻底（编译更慢）
strip = true        # 去掉调试符号，产物更小
panic = "abort"     # panic 直接终止，不做栈展开
```

排障期建议先注释掉 `panic = "abort"`，否则 panic 信息会变少。

### 5.7 日志规范

**访问日志**和**审计日志**分开：

- 访问日志：每次 HTTP 请求一条，来自 `TraceLayer`，字段含 `method` / `uri` / `status` / `latency` 等。
- 业务日志：来自 `tracing::info!(action = "config.updated", ...)`，记录改了哪些字段。
- 审计日志：如果将来要追责，另行写 `audit.jsonl`，记录旧值 / 新值 / 操作来源；默认访问日志不记录 `value`，避免敏感内容进日志。

典型 JSON 行：

```json
{"timestamp":"2026-11-16T10:00:00Z","level":"INFO","action":"config.updated","file":"C:\\SmartView\\config\\SmartViewConfig.txt","fields":["DataName"],"count":1,"ok":true}
```

如果希望 JSON 请求体解析失败也返回统一 JSON 错误体，需要给 axum 的 `JsonRejection` 写自定义 rejection handler；否则框架默认返回纯文本 400。

---

## 6. 运行与接口测试

```bash
RUST_LOG=info,tower_http=debug cargo run
# 指定另一个配置文件
cargo run -- /path/to/other-config.toml
# 需要热重载：cargo install cargo-watch
cargo watch -x run
```

Windows PowerShell 写法：

```powershell
$env:RUST_LOG = "info,tower_http=debug"
cargo run
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

成功响应：

```json
{"ok":true,"file":"C:\\SmartView\\config\\SmartViewConfig.txt","applied":["DataName","Thickness"],"count":2}
```

失败示例（白名单拦截）：

```json
{"ok":false,"error":"字段不在白名单中: Password"}
```

PowerShell 调用：

```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/api/config/update -Method Post `
  -ContentType 'application/json' -Body '{"name":"DataName","value":"TT26071311A002"}'
```

启动后终端应持续输出 JSON Lines；如果没有每次请求日志，检查 `RUST_LOG` 是否包含 `tower_http=debug`。

---

## 7. 打包与分发

### 7.1 本机单平台（最常用路线）

开发期 `cargo run` 产出的是 debug 二进制，**不要分发**。正式产物：

```bash
cargo build --release
```

产物路径（Windows）：`target\release\config-api.exe`。

**分发清单**：`config-api.exe` + `config.toml`（可选附 README）。MSVC 动态版目标机需要 VC++ Redistributable；静态 CRT / musl 版不需要额外运行库。

> 因为相对 `base_dir` 基于 `config.toml` 解析，分发目录要保持这两个文件在一起；或者启动时显式传配置文件路径。

#### 各平台目标三元组速查

| 目标系统 | 命令 | 产物位置 | 备注 |
|---|---|---|---|
| Windows x64 | `cargo build --release` | `target\release\config-api.exe` | 默认 MSVC，依赖 VC++ 运行库 |
| Windows x64 静态 CRT | `$env:RUSTFLAGS="-C target-feature=+crt-static"; cargo build --release --target x86_64-pc-windows-msvc` | `target\x86_64-pc-windows-msvc\release\config-api.exe` | 推荐分发；不依赖 VC++ Redistributable |
| Windows x64 GNU | `rustup target add x86_64-pc-windows-gnu` 后 `cargo build --release --target x86_64-pc-windows-gnu` | `target\x86_64-pc-windows-gnu\release\config-api.exe` | 需 mingw-w64；另一条静态路线 |
| Windows x86 32 位 | `rustup target add i686-pc-windows-msvc` | `target\i686-pc-windows-msvc\release\config-api.exe` | 老工控机可能用得上 |
| Linux x64 | `cargo build --release` | `target/release/config-api` | 默认 gnu，glibc 版本需 ≤ 目标机 |
| Linux x64 静态 | `rustup target add x86_64-unknown-linux-musl` | `target/x86_64-unknown-linux-musl/release/config-api` | 需 `musl-tools`；跨发行版最稳 |
| Linux ARM64 静态 | `rustup target add aarch64-unknown-linux-musl` | `target/aarch64-unknown-linux-musl/release/config-api` | 鲲鹏 / 飞腾 / ARM 服务器 |
| macOS Apple Silicon | `cargo build --release` | `target/release/config-api` | 默认 aarch64-apple-darwin |
| macOS Intel | `rustup target add x86_64-apple-darwin` | `target/x86_64-apple-darwin/release/config-api` | 只能在 macOS 上编译 |

**两条硬规则**：

1. macOS 产物只能在 macOS 上编译（苹果 SDK 许可限制），从 Windows / Linux 交叉编译 mac 版不可行。
2. MSVC 链接器只在 Windows 上可用；Linux 交叉编 Windows exe 要走 mingw-w64（gnu 目标），mac 上交叉编 MSVC 版不现实。

#### 产物整理建议

```text
dist/
└── config-api-1.0.0-windows-x64/
    ├── config-api.exe
    ├── config.toml
    └── README.txt        # 启动命令、端口、配置说明
```

打包成 zip（Windows PowerShell）：

```powershell
Compress-Archive -Path dist\config-api-1.0.0-windows-x64\* `
  -DestinationPath dist\config-api-1.0.0-windows-x64.zip
```

Linux / macOS 用 tar：

```bash
tar -czf config-api-1.0.0-linux-x64.tar.gz -C dist config-api-1.0.0-linux-x64
```

#### 发布前冒烟测试

```bash
./config-api.exe &
curl http://127.0.0.1:8080/api/config
curl -X POST http://127.0.0.1:8080/api/config/update -H "Content-Type: application/json" -d "{\"name\":\"DataName\",\"value\":\"SMOKE001\"}"
# 确认：目标文档值已变、注释/空行仍在、.bak 已生成、终端输出 JSON 日志
```

### 7.2 四目标 CI 打包（高级路线）

思路：本地只负责开发与调试，打 tag 时由 GitHub Actions 在 Windows / Linux x64 / Linux ARM64 / macOS 四台云端机器上各自编译，再上传 Release。这样**不需要拥有 Mac 也能拿到 macOS 包**。

`.github/workflows/build.yml`：

```yaml
name: build-release

on:
  push:
    tags: ["v*"]
  workflow_dispatch: {}

permissions:
  contents: write

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: windows-2022
            target: x86_64-pc-windows-msvc
            artifact: config-api-windows-x64
            bin: config-api.exe
            rustflags: "-C target-feature=+crt-static"
          - os: ubuntu-24.04
            target: x86_64-unknown-linux-musl
            artifact: config-api-linux-x64-musl
            bin: config-api
            rustflags: ""
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-musl
            artifact: config-api-linux-arm64-musl
            bin: config-api
            rustflags: ""
          - os: macos-15
            target: aarch64-apple-darwin
            artifact: config-api-macos-arm64
            bin: config-api
            rustflags: ""
    runs-on: ${{ matrix.os }}

    steps:
      - uses: actions/checkout@v4

      - name: Install musl tools
        if: contains(matrix.target, 'musl')
        run: sudo apt-get update && sudo apt-get install -y musl-tools

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Cache cargo
        uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.target }}

      - name: Build release
        env:
          RUSTFLAGS: ${{ matrix.rustflags }}
        run: cargo build --release --locked --target ${{ matrix.target }}

      - name: Package
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p dist
          cp "target/${{ matrix.target }}/release/${{ matrix.bin }}" dist/
          if [ -f config.toml ]; then cp config.toml dist/; fi
          if [ -f README.md ]; then cp README.md dist/; fi
          ls -l dist

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: dist/
          if-no-files-found: error

  release:
    needs: build
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest

    steps:
      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts

      - name: Zip each platform
        run: |
          set -euo pipefail
          cd artifacts
          for d in */; do
            name="${d%/}"
            (cd "$name" && zip -r "../${name}.zip" .)
          done
          ls -l *.zip

      - name: Upload to GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: artifacts/*.zip
          generate_release_notes: true
```

使用方式：

```bash
git init
git add .
git commit -m "init: config-api"
git remote add origin <你的仓库地址>
git push -u origin main
git tag v1.0.0
git push origin v1.0.0     # 触发 CI，四个目标并行构建并发布
```

补充要点：

- `macos-15` 是 Apple Silicon runner，产出 arm64 包；需要 Intel mac 包时，把 matrix 改成 `macos-13` + `x86_64-apple-darwin` 再加一项。
- Windows runner 的 `Package` 步骤显式写了 `shell: bash`，`cp` 才可用。
- Windows 用 `RUSTFLAGS=-C target-feature=+crt-static` 生成静态 CRT 版，避免目标机缺 VC++ 运行库。
- `cargo build --locked` 强制使用 `Cargo.lock`，保证 Release 里的版本与本地一致。
- 更进一步可用 `cargo-dist` 自动生成发布流程，或 `cross` 用 Docker 做交叉编译。

---

## 8. 常见坑与排查

| 现象 | 原因与解决 |
|---|---|
| 编译报 `link.exe not found` | 未安装 VS Build Tools 的「使用 C++ 的桌面开发」工作负载 |
| 首次编译卡住很久 | 正常，正在下载编译依赖；之后增量编译很快 |
| `cargo check` 报找不到 `tracing` | 第 3 节漏了 `cargo add tracing` |
| 日志不是 JSON | 漏了 `tracing-subscriber` 的 `json` feature，或没有调用 `.json()` |
| 看不到每次请求日志 | `RUST_LOG` 默认没开 `tower_http=debug`；改成 `info,tower_http=debug` |
| `Address already in use` | 端口被占用，改 `config.toml` 的 port，或结束旧进程 |
| 启动即退出并提示 base_dir / 目标文档不存在 | 路径写错，或 Windows 反斜杠没双写；相对路径基于 `config.toml` 解析 |
| 文档注释 / 空行被删 | 用了整篇 `render()` 旧方案；改用 5.3 的最小改动写入 |
| 提示字段重复 | 文档中同名 `@字段` 出现多次；当前策略是拒绝修改，先清理重复项 |
| 备份失败导致请求 500 | 这是刻意设计：没有回滚点就不改文档；检查 `.bak` 路径权限 |
| 写文件报 `os error 32` | Windows 下文件被记事本 / 其他进程独占，关闭占用程序 |
| 中文内容乱码 | 目标文档是 GBK 而程序按 UTF-8 读写；另存为 UTF-8 或引入 `encoding_rs` |
| 浏览器报 `CORS policy: No 'Access-Control-Allow-Origin' header` | 前端来源没写进 `config.toml` 的 `[cors].allowed_origins`，或该数组为空 |
| 预检 `OPTIONS` 返回 403 / 405 | 没挂 `CorsLayer`，或 `allowed_origins` 配错；确认使用的是 5.5 的路由代码 |
| 带 `Authorization` 的跨域请求被拦 | `allow_headers` 没包含 `AUTHORIZATION`，或 `allow_credentials=true` 时来源写了 `"*"` |
| 局域网访问不到 | `host` 改成 `0.0.0.0` 并放行防火墙；同时务必先加鉴权 |
| 拷到别的电脑无法运行 | MSVC 动态版缺 VC++ 运行库；改用静态 CRT / musl 版，或安装 VC++ Redistributable |
| 杀毒软件报警 | 未签名 exe 易被误报，企业内部分发建议做代码签名 |
| `cargo run` 改代码没生效 | 用 `cargo watch -x run` 获得类似 uvicorn `--reload` 的体验 |

---

## 9. 上线前必备与后续扩展

### 上线前必备

- **访问控制**：不要把无鉴权接口直接绑到 `0.0.0.0`。至少加 Bearer token，或限制来源 IP / 放在内网反向代理后面。
- **CORS 收口**：`[cors].allowed_origins` 只写真正需要的来源；不要用 `"*"` 代替鉴权，也不要在 `allow_credentials=true` 时配通配符。
- **JSON 日志落盘**：用环境变量或启动脚本把 stdout 重定向到日志文件，例如 `RUST_LOG=info,tower_http=debug .\config-api.exe >> logs\config-api.jsonl 2>&1`。
- **时间同步**：JSON 日志和审计依赖时间字段，目标机应开 NTP。

### 后续扩展

- **审计日志**：单独写 `audit.jsonl`，一行一条 JSON，字段含时间、来源 IP、字段名、旧值、新值、结果。
- **dry_run 预检**：批量接口加 `"dry_run": true`，只校验不落盘。
- **schema 版本**：在文档头部加 `# schema=v1`，解析时校验版本，避免文档格式悄悄变化。
- **数据库化**：当数据从 txt 迁到 SQLite / MySQL 时再引入 `sea-orm` + `sea-orm-migration`；当前只读写 txt，引入 ORM 只会增加编译时间与复杂度。
