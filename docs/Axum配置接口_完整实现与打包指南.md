# Axum 文档修改接口 · 完整实现与打包指南（config-api）

## 0. 项目目标与工具链对照

**功能**：Windows / Linux / macOS 上跑一个 HTTP 服务，做两件事：一是通过接口修改指定目录下 txt 文档中 `@字段名` 下面的值，目录路径、文件名、可改字段白名单全部由 `config.toml` 控制，写接口采用**方案 A**（单字段 + 批量 `items` 数组）；二是通过 `GET /download/{name}` 从配置的下载目录中取文件，通过 `POST /api/download/check` 先查询是否存在；两者共用「全名精确」和「末段精确」两级匹配，**不做模糊搜索**。

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
cargo add tokio-util --features io
cargo add utoipa --features axum_extras
cargo add utoipa-swagger-ui --features axum,vendored
```

- **axum**：Web 框架，定位类似 FastAPI。本方案只处理 JSON 请求和文件下载，不需要 `multipart` feature（那是上传用的）。
- **tokio**：异步运行时；`full` 包含 `#[tokio::main]`、net、sync 等。
- **serde / serde_json**：请求体 JSON 与 Rust 结构体转换，承担 pydantic 的解析职责。
- **toml**：解析 `config.toml`。
- **tower-http**：中间件集合；`trace` 输出每个请求的 span / 事件，`cors` 处理浏览器跨域和预检请求。
- **tracing**：在业务代码里写 `tracing::info!` 等结构化事件。
- **tracing-subscriber**：日志输出实现；`json` feature 让日志变成 JSON Lines。
- **anyhow**：简化应用层错误链。
- **tempfile**：写临时文件并 `persist` 到目标路径，实现跨平台原子替换。
- **tokio-util**：提供 `ReaderStream`，把 `tokio::fs::File` 变成异步字节流；只启用 `io` feature，下载大文件时不必一次性读进内存。
- **utoipa**：从 handler 上的 `#[utoipa::path]` 注解生成 OpenAPI JSON；`axum_extras` 让它忽略 `State`、`Path` 这类 axum 专有提取器。
- **utoipa-swagger-ui**：把 Swagger UI 页面挂到 axum 路由；`vendored` 把静态资源打进二进制，内网机器访问页面不需要外网 CDN。

接口可能由浏览器页面调用，因此默认启用 CORS，但**不允许硬编码 `*` 加凭证**。允许来源写在 `config.toml` 的 `[cors]` 段里，preflight `OPTIONS` 由 `CorsLayer` 自动处理，不需要单独写路由。

再次强调：CORS 只解决浏览器同源策略，不是鉴权。本方案不在代码里做鉴权，靠网络隔离保护：`host` 尽量绑具体内网 IP；必须写 `0.0.0.0` 时用防火墙限制来源网段，并在上线前从外网验证端口不可达。

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

[download]
enabled = true                              # false 时不注册 /download 路由
roots = ["./data/downloads"]                # 可配多个；相对路径基于本 config.toml 解析
separator = "_"                             # 末段切分符，见下方「下载匹配规则」
extensions = ["txt", "csv", "xls", "xlsx"]  # 小写、不带点；不在此列表的文件不参与匹配
case_insensitive = true                     # 统一转小写比较，抹平 Windows / Linux 差异
max_file_size = 104857600                   # 单文件大小上限（字节），0 表示不限制
allow_subdirs = false                       # false 只扫描 root 顶层，true 递归子目录
follow_symlinks = false                     # false 跳过符号链接 / junction
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

### 下载匹配规则

接口是 `GET /download/{name}`，`name` 必须是**单段文件名**。服务端先在 `roots` 里筛出候选文件，再做两级**精确比较**，没有包含匹配、正则或任意段匹配：

1. **全名精确**：`name == 文件完整文件名`，例如 `20231123_164957_AS9283331.xlsx`。
2. **末段精确**：把文件名按 `separator` 切成多段，取最后一段 `last`；满足任一即命中：
   - `name == last`，例如请求 `AS9283331.xlsx` 命中 `20231123_164957_AS9283331.xlsx` 的末段；
   - **仅当请求不带扩展名时**，`name == last` 去扩展名后的部分，例如请求 `AS9283331` 命中 `..._AS9283331.xlsx`；此时该文件的扩展名必须在 `extensions` 里。

请求带扩展名时只做“带扩展名的精确比较”，不会拿 `AS9283331.xlsx` 去命中 `..._AS9283331.csv`；`xls` 与 `xlsx` 也不会互相命中。

`case_insensitive = true` 时，上述比较统一转小写。文件名不含 `separator` 时 `last` 就是整个文件名，所以裸文件 `AS9283331.xlsx` 同样能被命中。

候选数量的处理是固定的：

| 候选数 | 状态码 | 响应 |
|---|---|---|
| 0 | 404 | `{"ok":false,"error":"未找到匹配文件: X"}` |
| 1 | 200 | 文件字节流 + `Content-Disposition: attachment` |
| ≥2 | 409 | `{"ok":false,"error":"匹配到多个文件，请使用完整文件名重试","candidates":[...]}` |

**多候选一律报错，绝不静默选第一个。** 目录里同时有 `AS9283331.xlsx` 和 `20231123_164957_AS9283331.xlsx` 时，请求 `AS9283331.xlsx` 会同时命中两级规则，返回 409 并列出两个全名。`xls` 与 `xlsx` 也必须严格区分，请求只给 `AS9283331` 而两种扩展名都存在时同样是 409。

不在 `extensions` 里的文件（`.bak`、`.tmp`、`.exe` 等）根本不进入候选集，因此既不会被匹配，也不会被下载。`extensions = []` 表示不按扩展名过滤，但那样会放大暴露面，生产环境不建议。

### 存在性查询

`POST /api/download/check` 复用上面的匹配规则，只回答“在不在、有几个”，不返回文件内容。请求体：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `name` | string | 是 | 文件标识，单段，不含路径分隔符 |
| `type` | string | 否 | 扩展名，如 `xls`；必须是配置 `extensions` 里的值 |

响应固定三个字段：`exist`、`count`、`files`，其中 `files` 永远是数组，`count` 永远是数字：

```json
{"exist": true, "count": 1, "files": ["20231209_120312_A12331.xls"]}
{"exist": true, "count": 2, "files": ["20231209_120312_A12331.xls", "20231209_120312_A12331.xlsx"]}
{"exist": false, "count": 0, "files": []}
```

合法请求一律 200（`exist: false` 不是 404）；`name` 非法或 `type` 不在白名单时返回 400。带 `type` 也可能返回多个文件——比如两个不同前缀的 `.xls`——所以 `count` 和数组始终保留。`files` 按文件名去重，两个 root 下的同名文件只算一个。

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

/// 顶层配置结构：对应 config.toml 里的五个段
#[derive(Debug, Clone, Deserialize)] // Debug 便于打印；Clone 便于 Arc 共享；Deserialize 用于解析 TOML
#[serde(deny_unknown_fields)]        // 配置里写错键名直接启动失败，而不是被静默忽略
pub struct AppConfig {               // 聚合所有配置，供服务全局只读使用
    pub server: ServerConfig,        // [server] 监听地址与端口
    pub files: FilesConfig,          // [files] 目标文档位置与备份开关
    pub allowed_fields: AllowedFields, // [allowed_fields] 可写字段白名单
    #[serde(default)]                // 老配置没有 [cors] 段时，用 Default 兜底
    pub cors: CorsConfig,            // [cors] 浏览器跨域配置
    #[serde(default)]                // 老配置没有 [download] 段时视为 None，不启用下载
    pub download: Option<DownloadConfig>, // [download] 下载目录与匹配规则；None 表示不启用
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

/// 下载配置
#[derive(Debug, Clone, Deserialize)]          // 可打印、可克隆、可反序列化
#[serde(deny_unknown_fields)]                // 拼错下载键会启动失败
pub struct DownloadConfig {                  // 控制“从哪些目录、按什么规则取文件”
    #[serde(default)]                        // 省略 enabled 时视为 false，避免误开下载
    pub enabled: bool,                       // 总开关；false 时不注册路由
    pub roots: Vec<String>,                  // 允许的目录；相对路径在 load() 里改写
    #[serde(default = "default_separator")]  // 省略 separator 时用 "_"
    pub separator: String,                   // 末段切分符
    #[serde(default)]                        // 省略 extensions 时视为 []（不按类型过滤）
    pub extensions: Vec<String>,             // 小写、不带点；空数组不过滤
    #[serde(default = "default_true")]       // 省略时默认 true
    pub case_insensitive: bool,              // 比较时是否忽略大小写
    #[serde(default)]                        // 省略时视为 0，即不限制
    pub max_file_size: u64,                  // 单文件大小上限（字节）
    #[serde(default)]                        // 省略时只看顶层
    pub allow_subdirs: bool,                 // 是否递归子目录
    #[serde(default)]                        // 省略时拒绝符号链接
    pub follow_symlinks: bool,               // 是否跟随符号链接
}

fn default_separator() -> String { "_".to_string() } // serde 默认值工厂
fn default_true() -> bool { true }                   // serde 默认值工厂

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

        // 下载目录同样是相对路径时，也基于配置文件所在目录解析，规则和 base_dir 一致。
        if let Some(dl) = cfg.download.as_mut() { // 只有配置了 [download] 才处理
            for root in &mut dl.roots {           // 逐个改写
                if Path::new(root).is_relative() { // 只处理相对路径
                    *root = base                   // 配置文件所在目录
                        .join(&*root)              // 拼出实际目录
                        .to_string_lossy()         // 转字符串
                        .into_owned();             // 拿所有权
                }                                  // if 结束
            }                                      // for 结束
        }                                          // if let 结束

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

        // 下载配置：只有 enabled=true 才严格要求，避免老配置升级后启动失败。
        if let Some(dl) = &self.download { // 取下载配置的只读引用
            if dl.enabled {                // 只有启用才校验细节
                if dl.roots.is_empty() {   // 没有目录等于接口永远 404
                    return Err(anyhow!("download.roots 不能为空")); // 提前暴露错误
                }
                if dl.separator.is_empty() { // 空分隔符会让末段切分失去意义
                    return Err(anyhow!("download.separator 不能为空")); // 拒绝启动
                }
                for ext in &dl.extensions { // 逐项检查扩展名
                    if ext.is_empty() || ext.contains('.') { // 要求小写、不带点
                        return Err(anyhow!("download.extensions 必须是小写且不带点的扩展名: {ext}")); // 报错
                    }                      // if 结束
                }                          // for 结束
                for root in &dl.roots {    // 逐个检查目录
                    let dir = Path::new(root); // 转成 Path 视图
                    if !dir.is_dir() {     // 目录不存在或不是目录
                        return Err(anyhow!("download.roots 不存在或不是目录: {}", dir.display())); // 明确报错
                    }                      // if 结束
                    dir.canonicalize()     // 解析成绝对路径，供运行时做前缀校验
                        .with_context(|| format!("解析下载目录失败: {}", dir.display()))?; // 失败附路径
                }                          // for 结束
            }                              // if 结束
        }                                  // if let 结束
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

### 5.4 `src/download.rs` —— 文件名校验与候选收集

这一层不依赖 axum，只做纯逻辑：先判断请求名是否安全，再在配置的目录里收集所有命中的文件。**它不负责选哪个，只负责给出候选集合**，三态判定留给 handler。

```rust
use crate::config::DownloadConfig; // 下载配置
use std::fs;                       // 读目录与元数据
use std::path::{Path, PathBuf};    // 路径类型

/// 请求文件名不合法的原因
#[derive(Debug)]                   // 便于日志打印
pub enum NameError {               // 所有拒绝理由
    Empty,                         // 空字符串
    DotOrDotDot,                   // . 或 ..
    HasSeparator,                  // 含 / 或 \
    HasControl,                    // 含控制字符或 NUL
    DeviceName,                    // Windows 保留设备名
}

impl std::fmt::Display for NameError { // 实现 Display 才能 to_string()
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { // 格式化入口
        match self {               // 按原因分支
            Self::Empty => write!(f, "文件名不能为空"), // 空串
            Self::DotOrDotDot => write!(f, "文件名不能是 . 或 .."), // 相对路径 tricks
            Self::HasSeparator => write!(f, "文件名不能包含路径分隔符"), // 防目录穿越
            Self::HasControl => write!(f, "文件名不能包含控制字符"), // 防 NUL 注入
            Self::DeviceName => write!(f, "文件名不能使用 Windows 保留设备名"), // 设备名
        }                          // match 结束
    }                              // fmt 结束
}                                  // impl 结束

/// 请求名必须是单段安全文件名。路由只匹配 {name}，这里再兜一层。
pub fn validate_requested_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {           // 空串
        return Err(NameError::Empty); // 拒绝
    }                              // if 结束
    if name == "." || name == ".." { // 相对路径
        return Err(NameError::DotOrDotDot); // 拒绝
    }                              // if 结束
    if name.contains('/') || name.contains('\\') { // 两种分隔符都拦
        return Err(NameError::HasSeparator); // 拒绝
    }                              // if 结束
    if name.chars().any(|c| c.is_control()) { // NUL 与其他控制字符
        return Err(NameError::HasControl); // 拒绝
    }                              // if 结束
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase(); // 设备名只看主名
    const DEVICES: &[&str] = &[     // Windows 保留设备名
        "CON", "PRN", "AUX", "NUL", // 四个基础名
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", // 串口
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", // 并口
    ];                             // 常量结束
    if DEVICES.contains(&stem.as_str()) { // 命中保留名
        return Err(NameError::DeviceName); // 拒绝
    }                              // if 结束
    Ok(())                         // 校验通过
}                                  // 函数结束

/// 去掉最后一个点及其后的扩展名；没有点或以点开头时原样返回。
fn strip_ext(name: &str) -> &str { // 输入文件名，输出主名切片
    match name.rfind('.') {        // 找最后一个点
        Some(i) if i > 0 => &name[..i], // 有扩展名，截掉
        _ => name,                 // 没有或不合法，原样
    }                              // match 结束
}                                  // 函数结束

/// 按配置决定是否转小写，统一比较口径。
fn compare_key(s: &str, case_insensitive: bool) -> String { // 返回比较用的字符串
    if case_insensitive { s.to_lowercase() } else { s.to_string() } // 分支
}                                  // 函数结束

/// 请求名是否自带扩展名。决定是否允许“去扩展名比较”。
fn has_extension(name: &str) -> bool { // 输入请求名
    name.rfind('.').map(|i| i > 0).unwrap_or(false) // 点不在开头才算扩展名
}                                  // 函数结束

/// 收集所有命中文件；返回路径已 canonicalize，且都在某个 root 内。
/// type_filter 为 Some 时只用该扩展名（check 接口用），None 时用配置的 extensions。
/// 出错只表示目录不可读，不表示“没找到”。
pub fn find_candidates(            // 候选收集主函数
    config: &DownloadConfig,       // 下载配置
    requested: &str,               // 请求名
    type_filter: Option<&str>,     // 可选扩展名过滤；GET 下载传 None
) -> Result<Vec<PathBuf>, String> { // 成功返回候选路径，失败返回服务端错误文案
    let wanted = compare_key(requested, config.case_insensitive); // 全名/末段的比较键
    let wanted_stem = compare_key(strip_ext(requested), config.case_insensitive); // 主名比较键
    let allow_stem_match = !has_extension(requested); // 只有请求不带扩展名才允许主名匹配

    // 生效的扩展名白名单：带 type 时只用它，否则用配置值；两种都统一小写。
    let ext_filter: Vec<String> = match type_filter { // 按是否带 type 分支
        Some(t) => vec![t.to_ascii_lowercase()],     // 只用请求指定的类型
        None => config.extensions.iter().map(|e| e.to_ascii_lowercase()).collect(), // 用配置白名单
    };                                 // match 结束
    let mut out = Vec::new();       // 收集候选

    for root in &config.roots {     // 逐个目录扫描
        let root_abs = Path::new(root) // 配置里已解析成绝对路径
            .canonicalize()          // 再规范化一次，确保前缀判断可靠
            .map_err(|e| format!("下载目录不可用: {}: {e}", root))?; // 失败转服务端错误
        scan_dir(&root_abs, &root_abs, config, &ext_filter, &wanted, &wanted_stem, allow_stem_match, &mut out)?; // 扫描
    }                              // for 结束

    out.sort();                    // 固定顺序，保证 409 的 candidates 稳定
    out.dedup();                   // 多个 root 指向同一文件时去重
    Ok(out)                        // 返回候选
}                                  // 函数结束

/// 递归扫描；dir 是当前目录，root_abs 是最初的允许根目录。
fn scan_dir(                       // 内部扫描函数
    dir: &Path,                    // 当前目录
    root_abs: &Path,               // 允许的根目录，用于越界检查
    config: &DownloadConfig,       // 下载配置
    ext_filter: &[String],         // 生效的扩展名白名单（已小写）
    wanted: &str,                  // 请求名比较键
    wanted_stem: &str,             // 请求主名比较键
    allow_stem_match: bool,        // 是否允许去扩展名匹配
    out: &mut Vec<PathBuf>,        // 候选收集器
) -> Result<(), String> {          // 成功或服务端错误
    let entries = fs::read_dir(dir) // 读目录项
        .map_err(|e| format!("读取下载目录失败: {}: {e}", dir.display()))?; // 失败转错误
    for entry in entries {         // 逐项处理
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?; // 单项读取失败
        let path = entry.path();   // 完整路径
        let meta = fs::symlink_metadata(&path) // 用 symlink_metadata 才看得出符号链接
            .map_err(|e| format!("读取文件元数据失败: {}: {e}", path.display()))?; // 失败转错误

        if meta.file_type().is_symlink() && !config.follow_symlinks { // 默认跳过符号链接
            continue;              // 防止跳到目录外
        }                          // if 结束
        if meta.is_dir() {         // 子目录
            if config.allow_subdirs { // 允许递归
                let child = path.canonicalize() // 先规范化
                    .map_err(|e| format!("解析子目录失败: {}: {e}", path.display()))?; // 失败转错误
                if !child.starts_with(root_abs) { // 越界保护
                    continue;      // 跳过
                }                  // if 结束
                scan_dir(&child, root_abs, config, ext_filter, wanted, wanted_stem, allow_stem_match, out)?; // 递归
            }                      // if 结束
            continue;              // 目录本身不是候选
        }                          // if 结束
        if !meta.is_file() {       // 只要普通文件
            continue;              // 跳过设备、管道等
        }                          // if 结束
        if config.max_file_size > 0 && meta.len() > config.max_file_size { // 超过上限
            tracing::warn!(file = %path.display(), "文件超过 download.max_file_size，已跳过"); // 留痕
            continue;              // 不进入候选
        }                          // if 结束

        let file_name = entry.file_name().to_string_lossy().into_owned(); // 文件名转 String
        let ext = Path::new(&file_name) // 用路径视图取扩展名
            .extension()           // 可能是 None
            .and_then(|s| s.to_str()) // 非 UTF-8 则跳过该项
            .unwrap_or("")         // 没有扩展名视为空串
            .to_ascii_lowercase(); // 统一小写比较
        if !ext_filter.is_empty()  // 白名单非空时
            && !ext_filter.iter().any(|e| e == &ext) // 不在生效列表里（已统一小写）
        {                          // 组合条件
            continue;              // 不进入候选
        }                          // if 结束

        let last = file_name       // 取末段
            .rsplit(config.separator.as_str()) // 按分隔符从右切
            .next()                // 第一段就是末段
            .unwrap_or(&file_name); // 理论上不会失败
        let full_hit = compare_key(&file_name, config.case_insensitive) == wanted; // 规则 A
        let last_hit = compare_key(last, config.case_insensitive) == wanted // 规则 B-1
            || (allow_stem_match // 规则 B-2：仅请求不带扩展名时启用
                && compare_key(strip_ext(last), config.case_insensitive) == wanted_stem); // 主名相等
        if !full_hit && !last_hit { // 两条规则都没命中
            continue;              // 跳过
        }                          // if 结束

        let abs = path.canonicalize() // 命中后再规范化一次
            .map_err(|e| format!("解析候选文件失败: {}: {e}", path.display()))?; // 失败转错误
        if !abs.starts_with(root_abs) { // 最终越界检查
            continue;              // 拒绝目录外文件
        }                          // if 结束
        out.push(abs);             // 加入候选
    }                              // for 结束
    Ok(())                         // 扫描完成
}                                  // 函数结束
```

### 5.5 `src/handler.rs` —— 五个接口

```rust
use axum::body::Body;             // 流式响应体
use axum::extract::{Json, Path as AxumPath, State}; // Path 提取器用别名，避开 std::path::Path
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE}; // 下载响应头
use axum::http::{HeaderValue, StatusCode}; // HeaderValue 构造响应头；StatusCode 是状态码
use axum::response::{IntoResponse, Response}; // 把元组 / Json 转成统一 Response
use serde::Deserialize;           // 请求体反序列化派生宏
use serde_json::json;             // 快速构造 JSON 值
use std::fs;                      // 同步文件读写
use std::io::Write;               // write_all 方法所在 trait
use std::path::Path;              // 路径借用类型
use tempfile::NamedTempFile;      // 同目录临时文件，用于原子替换
use tokio::fs::File;              // 异步打开文件，供流式下载
use tokio_util::io::ReaderStream; // 把 File 包装成异步字节流
use utoipa::ToSchema;             // 从结构体生成 OpenAPI schema

use crate::download;              // 文件名校验与候选收集模块
use crate::parser;                // 文档解析与最小改动写入模块
use crate::state::AppState;       // 共享状态类型

/// 单字段请求体：{ "name": "DataName", "value": "TT26071311A002" }
#[derive(Debug, Deserialize, ToSchema)]     // 可打印；从 JSON 请求体反序列化
pub struct UpdateRequest {        // 对应单字段更新接口
    pub name: String,             // 字段名，必须在白名单里
    pub value: String,            // 新值，不能含换行或以 @ 开头
}                                 // 结构体结束

/// 批量请求体（方案 A）：{ "items": [ {name, value}, ... ] }
#[derive(Debug, Deserialize, ToSchema)]     // 同上
pub struct BatchRequest {         // 对应批量更新接口
    pub items: Vec<UpdateRequest>, // 多条更新；空数组会被拒绝
}                                 // 结构体结束

/// 下载检查请求体：{ "name": "A12331", "type": "xls" }
#[derive(Debug, Deserialize, ToSchema)]     // 可打印；从 JSON 请求体反序列化
pub struct CheckRequest {         // 对应 POST /api/download/check
    pub name: String,             // 文件标识；必填，单段安全文件名
    #[serde(rename = "type", default)] // type 是 Rust 关键字，且允许省略
    pub file_type: Option<String>, // 可选扩展名；必须是配置 extensions 的子集
}                                 // 结构体结束

/// GET /api/config —— 读取当前文档解析结果
#[utoipa::path(
    get,
    path = "/api/config",
    tag = "config",
    responses(
        (status = 200, description = "读取成功"),
        (status = 500, description = "读取文档失败")
    )
)]
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
#[utoipa::path(
    post,
    path = "/api/config/update",
    tag = "config",
    request_body = UpdateRequest,
    responses(
        (status = 200, description = "修改成功"),
        (status = 400, description = "参数或白名单校验失败"),
        (status = 500, description = "读写文件失败")
    )
)]
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
#[utoipa::path(
    post,
    path = "/api/config/batch-update",
    tag = "config",
    request_body = BatchRequest,
    responses(
        (status = 200, description = "批量修改成功"),
        (status = 400, description = "批量校验失败"),
        (status = 500, description = "读写文件失败")
    )
)]
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

/// GET /download/{name} —— 从配置目录取文件。
/// 两级精确匹配；0 个候选 404，1 个流式返回，多个 409 不猜。
#[utoipa::path(
    get,
    path = "/download/{name}",
    tag = "download",
    params(
        ("name" = String, Path, description = "文件名或编号，单段")
    ),
    responses(
        (status = 200, description = "文件字节流", content_type = "application/octet-stream"),
        (status = 400, description = "文件名非法"),
        (status = 404, description = "未找到匹配文件"),
        (status = 409, description = "匹配到多个文件"),
        (status = 500, description = "读取文件失败")
    )
)]
pub async fn download_file(                             // 下载 handler
    State(state): State<AppState>,                      // 注入共享状态
    AxumPath(name): AxumPath<String>,                   // 单段文件名，由 axum 先做路径切分
) -> Response {                                          // 返回统一 Response
    let Some(dl) = state.config.download.as_ref().filter(|d| d.enabled) else { // 未启用时兜底
        return not_found("下载接口未启用");              // 404
    };                                                  // let-else 结束

    if let Err(e) = download::validate_requested_name(&name) { // 文件名合法性
        return bad_request(&format!("文件名非法: {e}")); // 400
    }                                                   // if 结束

    let candidates = match download::find_candidates(dl, &name, None) { // 收集候选；GET 不额外限定类型
        Ok(v) => v,                                     // 收集成功
        Err(e) => return server_error(&e),              // 目录不可读转 500
    };                                                  // match 结束

    match candidates.len() {                            // 三态判定
        0 => {                                          // 没有候选
            tracing::info!(action = "download.miss", name = %name, "未找到匹配文件"); // 留痕
            not_found(&format!("未找到匹配文件: {name}")) // 404
        }                                               // 分支结束
        1 => {                                          // 唯一候选
            let path = &candidates[0];                  // 借用候选路径
            let file = match File::open(path).await {   // 异步打开文件
                Ok(f) => f,                             // 打开成功
                Err(e) => return server_error(&format!("打开下载文件失败: {e}")), // 失败转 500
            };                                          // match 结束
            let body = Body::from_stream(ReaderStream::new(file)); // 流式响应体，不整文件入内存
            let file_name = path                        // 取文件名用于响应头
                .file_name()                            // Option<&OsStr>
                .and_then(|s| s.to_str())               // 非 UTF-8 时用兜底名
                .unwrap_or("download");                 // 兜底
            let mime = mime_for(path);                  // 按扩展名选 Content-Type

            let mut resp = Response::new(body);         // 构造 200 响应
            *resp.status_mut() = StatusCode::OK;        // 显式设置状态码
            resp.headers_mut().insert(                  // 写 Content-Type
                CONTENT_TYPE,                           // 头名
                HeaderValue::from_static(mime),         // 静态值
            );                                          // insert 结束
            resp.headers_mut().insert(                  // 写 Content-Disposition
                CONTENT_DISPOSITION,                    // 头名
                HeaderValue::from_str(&content_disposition(file_name)) // 动态值可能含非 ASCII
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment")), // 极端情况兜底
            );                                          // insert 结束

            tracing::info!(                             // 业务日志
                action = "download.ok",                 // 事件名
                name = %name,                           // 请求名
                file = %path.display(),                 // 实际文件
                "文件下载成功"                           // 正文
            );                                          // 宏结束
            resp                                        // 返回响应
        }                                               // 分支结束
        _ => {                                          // 多个候选
            let list: Vec<String> = candidates.iter()   // 遍历候选
                .map(|p| p                              // 每个路径
                    .file_name()                        // 取文件名
                    .and_then(|s| s.to_str())           // 转 &str
                    .unwrap_or("")                      // 兜底空串
                    .to_string())                       // 转 String
                .collect();                             // 收集成列表
            tracing::warn!(                             // 歧义也要留痕
                action = "download.ambiguous",          // 事件名
                name = %name,                           // 请求名
                candidates = ?list,                     // 候选列表
                "匹配到多个文件"                         // 正文
            );                                          // 宏结束
            (                                           // 元组响应
                StatusCode::CONFLICT,                   // 409
                Json(json!({                            // 错误体
                    "ok": false,                        // 业务失败
                    "error": "匹配到多个文件，请使用完整文件名重试", // 提示
                    "candidates": list,                 // 候选全名，供客户端重试
                })),                                    // Json 结束
            )                                           // 元组结束
                .into_response()                        // 转 Response
        }                                               // 分支结束
    }                                                   // match 结束
}                                                       // download_file 结束

/// POST /api/download/check —— 查询文件是否存在，不返回文件内容。
/// 复用 download::find_candidates，type 只是额外的扩展名过滤。
#[utoipa::path(
    post,
    path = "/api/download/check",
    tag = "download",
    request_body = CheckRequest,
    responses(
        (status = 200, description = "查询完成，结果见 exist / count / files"),
        (status = 400, description = "参数非法或 type 不在允许列表"),
        (status = 404, description = "下载接口未启用"),
        (status = 500, description = "目录不可读")
    )
)]
pub async fn check_download(                             // check handler
    State(state): State<AppState>,                      // 注入共享状态
    Json(req): Json<CheckRequest>,                      // 注入请求体
) -> Response {                                          // 返回统一 Response
    let Some(dl) = state.config.download.as_ref().filter(|d| d.enabled) else { // 未启用兜底
        return not_found("下载接口未启用");              // 404
    };                                                  // let-else 结束

    if let Err(e) = download::validate_requested_name(&req.name) { // 复用文件名校验
        return bad_request(&format!("name 非法: {e}")); // 400
    }                                                   // if 结束

    // type 必须是配置白名单的子集，避免绕过 extensions 限制。
    if let Some(t) = &req.file_type {                    // 请求带了 type
        let t_lower = t.trim().to_ascii_lowercase();    // 去空白并统一小写
        let allowed = dl.extensions.iter().any(|e| e.eq_ignore_ascii_case(&t_lower)); // 查白名单
        if !allowed {                                   // 不在白名单
            return bad_request(&format!("type 不在允许列表中: {t}")); // 400
        }                                               // if 结束
    }                                                   // if let 结束

    let type_filter = req.file_type.as_deref()          // 比较用的小写类型
        .map(|t| t.trim().to_ascii_lowercase());        // Option<&str> -> Option<String>
    let candidates = match download::find_candidates(dl, &req.name, type_filter.as_deref()) { // 复用候选收集
        Ok(v) => v,                                     // 成功
        Err(e) => return server_error(&e),              // 目录不可读转 500
    };                                                  // match 结束

    let mut files: Vec<String> = candidates.iter()      // 只要文件名，不要路径
        .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(|s| s.to_string())) // 转文件名
        .collect();                                     // 收集
    files.sort();                                       // 稳定顺序
    files.dedup();                                      // 按文件名去重；不同 root 同名只算一个
    let count = files.len();                            // 命中数量
    let exist = count > 0;                              // 是否存在

    tracing::info!(                                     // 业务日志
        action = "download.check",                      // 事件名
        name = %req.name,                               // 请求名
        file_type = ?req.file_type,                     // 请求类型
        count = count,                                  // 命中数量
        "下载检查完成"                                   // 正文
    );                                                  // 宏结束

    Json(json!({                                        // 固定三字段响应
        "exist": exist,                                 // 是否存在
        "count": count,                                 // 命中数量
        "files": files,                                 // 文件名数组，永远是数组
    })).into_response()                                 // 200 + JSON
}                                                       // check_download 结束

/// 按扩展名给一个尽量准确的 Content-Type。
fn mime_for(path: &Path) -> &'static str {              // 输入文件路径
    match path.extension()                              // 取扩展名
        .and_then(|s| s.to_str())                       // 转 &str
        .unwrap_or("")                                  // 没有则空串
        .to_ascii_lowercase()                           // 统一小写
        .as_str()                                       // 借用比较
    {                                                   // match 开始
        "txt" => "text/plain; charset=utf-8",           // 纯文本
        "csv" => "text/csv; charset=utf-8",             // CSV
        "xls" => "application/vnd.ms-excel",            // 老 Excel
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", // 新 Excel
        _ => "application/octet-stream",                // 默认二进制
    }                                                   // match 结束
}                                                       // mime_for 结束

/// 按 RFC 5987 生成 Content-Disposition，中文名不乱码。
fn content_disposition(file_name: &str) -> String {     // 输入文件名
    let ascii: String = file_name.chars()               // 逐字符处理
        .map(|c| if c.is_ascii() && c != '"' && c != '\\' { c } else { '_' }) // 非 ASCII 和引号换成下划线
        .collect();                                     // 收集成安全 ASCII 名
    format!(                                            // 两个 filename 参数并存
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", // 兼容旧客户端 + 新标准
        percent_encode(file_name)                       // 原名做百分号编码
    )                                                   // format 结束
}                                                       // content_disposition 结束

/// 最小百分号编码，只保留 RFC 3986 未保留字符。
fn percent_encode(s: &str) -> String {                  // 输入任意字符串
    let mut out = String::new();                        // 结果缓冲
    for b in s.as_bytes() {                             // 按字节处理，UTF-8 自然覆盖
        match b {                                       // 分支
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char), // 未保留字符原样
            _ => out.push_str(&format!("%{b:02X}")),    // 其余转义成 %XX
        }                                               // match 结束
    }                                                   // for 结束
    out                                                 // 返回编码结果
}                                                       // percent_encode 结束

fn not_found(msg: &str) -> Response {                   // 统一 404 JSON 错误
    (                                                   // 元组：(状态码, Json)
        StatusCode::NOT_FOUND,                          // 404 Not Found
        Json(json!({ "ok": false, "error": msg })),     // 错误体
    )                                                   // 元组结束
        .into_response()                                // 转成 Response
}                                                       // not_found 结束

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

### 5.6 `src/main.rs` —— 入口、路由、CORS 与 JSON 日志

```rust
// Rust 不会自动发现源文件，模块必须显式声明。
mod config;   // 配置加载与校验
mod download; // 文件名校验与候选收集
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
use utoipa::OpenApi;                    // 从注解生成 OpenAPI 文档
use utoipa_swagger_ui::SwaggerUi;       // Swagger UI 页面

/// OpenAPI 文档入口：聚合所有带 utoipa::path 注解的 handler。
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
    let download_enabled = config                   // 在 config 被移入 Arc 前取出下载开关
        .download                                   // Option<DownloadConfig>
        .as_ref()                                   // 借用
        .is_some_and(|d| d.enabled);                // 只有配置且启用才为 true
    let state = state::AppState::new(config);       // config 被移入 Arc，之后不能再直接用

    let app = Router::new()                         // 构造路由器
        .route("/api/config", get(handler::read_config)) // GET：读取文档
        .route("/api/config/update", post(handler::update_one)) // POST：单字段更新
        .route("/api/config/batch-update", post(handler::batch_update)); // POST：批量更新

    let app = if download_enabled {                 // 按开关决定是否注册下载相关路由
        app                                         // 接住基础路由
            .route("/download/{name}", get(handler::download_file)) // 单段文件名
            .route("/api/download/check", post(handler::check_download)) // 存在性查询
    } else {                                        // 未启用
        app                                         // 保持原路由
    };                                              // if 结束

    let app = app                                   // 接上注入与中间件
        .with_state(state)                          // 注入共享状态
        .merge(                                     // 挂 Swagger UI 与 OpenAPI JSON
            SwaggerUi::new("/docs")           // 浏览器打开这个地址
                .url("/openapi.json", ApiDoc::openapi()) // 文档数据
        )
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

### Swagger UI 配置要点

Swagger UI 不是 axum 的内置能力，最终版本通过三个动作接入：

1. 请求体结构体 `UpdateRequest` / `BatchRequest` / `CheckRequest` 派生 `ToSchema`，生成 `components.schemas`。`CheckRequest` 的 `#[serde(rename = "type")]` 会被 utoipa 识别，文档里显示 `type` 而不是 `file_type`。
2. 五个 handler 加 `#[utoipa::path]` 注解，声明路径、请求体和响应码。`axum_extras` feature 让 utoipa 忽略 `State`、`AxumPath` 等 axum 专有提取器。
3. `main.rs` 用 `#[derive(OpenApi)]` 聚合 `paths` 和 `schemas`，再用 `SwaggerUi::new("/docs").url("/openapi.json", ...)` 通过 `.merge()` 挂到 Router 上。`merge` 必须在 `with_state` 之后、`layer` 之前。

访问地址：

- 页面：`http://127.0.0.1:18981/docs/`（末尾 `/` 不要省）
- OpenAPI JSON：`http://127.0.0.1:18981/openapi.json`

当前锁定版本是 `utoipa 6.x` + `utoipa-swagger-ui 10.x`，直接写 `SwaggerUi::new("/docs")` 即可，不需要旧版的 `/{_:.*}` 通配。`vendored` feature 把页面静态资源编进二进制，内网机器访问页面不依赖外网 CDN。

两点提醒：本方案不做鉴权，页面和接口一样靠网络隔离保护；如果 `download.enabled = false`，下载路由没有注册，但 OpenAPI 里仍会列出这两个接口，看文档时要结合实际配置。

### 5.7 `Cargo.toml` 的 release 优化段

在 `[dependencies]` 之后追加：

```toml
[profile.release]
lto = true          # 链接期优化，减小体积、提升性能
codegen-units = 1   # 单代码生成单元，优化更彻底（编译更慢）
strip = true        # 去掉调试符号，产物更小
panic = "abort"     # panic 直接终止，不做栈展开
```

排障期建议先注释掉 `panic = "abort"`，否则 panic 信息会变少。

### 5.8 日志规范

**访问日志**和**审计日志**分开：

- 访问日志：每次 HTTP 请求一条，来自 `TraceLayer`，字段含 `method` / `uri` / `status` / `latency` 等。
- 业务日志：来自 `tracing::info!(action = "config.updated", ...)`，记录改了哪些字段；下载侧对应 `download.ok`、`download.miss`、`download.ambiguous` 三个事件，其中 ambiguous 用 `warn` 级别，方便统计误请求。
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

**下载文件**

先放一个测试文件到 `data/downloads/20231123_164957_AS9283331.xlsx`，然后：

```bash
# 1) 末段 + 扩展名（推荐）
curl -OJ http://127.0.0.1:8080/download/AS9283331.xlsx

# 2) 只给 ID，由 extensions 白名单限定候选
curl -OJ http://127.0.0.1:8080/download/AS9283331

# 3) 完整文件名
curl -OJ http://127.0.0.1:8080/download/20231123_164957_AS9283331.xlsx
```

唯一命中时返回 200、文件字节流和 `Content-Disposition: attachment`，`-OJ` 让 curl 按响应头里的文件名保存。如果同目录下再放一个 `20231123_164957_AS9283331.csv`，只给 ID 的请求就会变成 409：

```bash
curl -i http://127.0.0.1:8080/download/AS9283331
```

```json
{"ok":false,"error":"匹配到多个文件，请使用完整文件名重试","candidates":["20231123_164957_AS9283331.csv","20231123_164957_AS9283331.xlsx"]}
```

PowerShell 保存到指定路径：

```powershell
Invoke-WebRequest -Uri http://127.0.0.1:8080/download/AS9283331.xlsx -OutFile AS9283331.xlsx
```

**查询是否存在（不下载）**

```bash
# 带类型
curl -X POST http://127.0.0.1:8080/api/download/check \
  -H "Content-Type: application/json" \
  -d '{"name":"A12331","type":"xls"}'

# 不带类型
curl -X POST http://127.0.0.1:8080/api/download/check \
  -H "Content-Type: application/json" \
  -d '{"name":"A12331"}'
```

响应始终是 `exist` / `count` / `files` 三个字段，`exist: false` 时也是 200：

```json
{"exist":true,"count":2,"files":["20231209_120312_A12331.xls","20231209_120312_A12331.xlsx"]}
```

启动后终端应持续输出 JSON Lines；如果没有每次请求日志，检查 `RUST_LOG` 是否包含 `tower_http=debug`。

**Swagger UI 接口文档**

```bash
# 页面：浏览器打开 http://127.0.0.1:18981/docs/
curl -I http://127.0.0.1:18981/docs/          # 200

# OpenAPI JSON
curl http://127.0.0.1:18981/openapi.json
```

JSON 里应有 5 个 path（`/api/config`、`/api/config/update`、`/api/config/batch-update`、`/download/{name}`、`/api/download/check`）和 3 个 schema（`UpdateRequest`、`BatchRequest`、`CheckRequest`）。

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
| 局域网访问不到 | `host` 改成具体内网 IP 或 `0.0.0.0`，放行防火墙；改完从外网验证一次端口不可达 |
| 拷到别的电脑无法运行 | MSVC 动态版缺 VC++ 运行库；改用静态 CRT / musl 版，或安装 VC++ Redistributable |
| 杀毒软件报警 | 未签名 exe 易被误报，企业内部分发建议做代码签名 |
| 下载接口返回 404 | 先确认 `[download].enabled = true`；再检查 `roots` 是否存在、扩展名是否在 `extensions` 里、请求名是否和末段一致 |
| 下载返回 409 而不是文件 | 目录里有多个候选；按 `candidates` 里的完整文件名重新请求，不要依赖服务端挑一个 |
| 请求 `AS9283331.xlsx` 命中了 `.csv` | 不会：请求带扩展名时只做带扩展名的精确比较；确认用的是 5.4 的 `find_candidates` |
| 下载中文文件名乱码 | 确认响应带 `filename*=UTF-8''...`；老客户端只能读 `filename=` 的 ASCII 兜底名 |
| 下载大文件超时 | 检查 `max_file_size` 是否把文件跳过了（日志里有 warn）；流式返回已避免整文件入内存 |
| 下载路径报越界或读到目录外文件 | 保持 `follow_symlinks = false`、`allow_subdirs = false`；不要手工改 `canonicalize` 前缀检查 |
| check 接口返回 400 | `name` 含非法字符，或 `type` 不在 `extensions` 白名单里 |
| check 返回 200 但 `exist=false` | 查询成功但没命中；核对 `name` 是否等于末段或完整文件名，以及是否漏了扩展名 |
| `cargo run` 改代码没生效 | 用 `cargo watch -x run` 获得类似 uvicorn `--reload` 的体验 |

---

## 9. 上线前必备与后续扩展

### 上线前必备

- **网络边界（本方案不做代码鉴权）**：`host` 优先写具体内网 IP，不要直接用 `0.0.0.0`；必须全网卡监听时用防火墙只放行内网网段。上线前从外网或另一网段实际访问一次，确认端口不可达。下载接口会把文件交出去，check 接口会暴露文件是否存在，二者都依赖这层网络隔离。Windows 防火墙示例（管理员 PowerShell，网段和端口按实际改）：

  ```powershell
  New-NetFirewallRule -DisplayName "config-api-18981" `
    -Direction Inbound -Protocol TCP -LocalPort 18981 `
    -RemoteAddress 192.168.1.0/24 -Action Allow
  ```
- **下载目录收口**：`roots` 只写确实要公开的目录，不要写整个磁盘根目录或用户目录。`extensions` 保持明确白名单，`follow_symlinks` 和 `allow_subdirs` 默认保持 `false`；`max_file_size` 设为业务实际需要的量级。
- **CORS 收口**：`[cors].allowed_origins` 只写真正需要的来源；CORS 不是访问控制，别用 `"*"` 当安全边界，也不要在 `allow_credentials=true` 时配通配符。
- **JSON 日志落盘**：用环境变量或启动脚本把 stdout 重定向到日志文件，例如 `RUST_LOG=info,tower_http=debug .\config-api.exe >> logs\config-api.jsonl 2>&1`。
- **时间同步**：JSON 日志和审计依赖时间字段，目标机应开 NTP。

### 后续扩展

- **审计日志**：单独写 `audit.jsonl`，一行一条 JSON，字段含时间、来源 IP、字段名、旧值、新值、结果。
- **dry_run 预检**：批量接口加 `"dry_run": true`，只校验不落盘。
- **Range 请求**：当前下载固定返回整文件（配合 `attachment` 强制下载，不做内联预览），因此没有实现 `206 Partial Content`。若要支持浏览器断点续传或 PDF 内联预览，再加单段 Range 解析与 `Accept-Ranges` 头。
- **目录索引缓存**：现在每次下载请求都会扫描 `roots`。目录文件数上千时，可在启动时建索引，并用定时刷新或文件变更事件更新；注意刷新同样要做符号链接与越界检查。
- **Swagger UI**：axum 不自带 Swagger，但**最终版本已集成**：页面 `/docs/`、数据 `/openapi.json`，`vendored` 资源离线可用。配置步骤见 5.6 之后的「Swagger UI 配置要点」，教学推导见手把手教程阶段 12.5。
- **schema 版本**：在文档头部加 `# schema=v1`，解析时校验版本，避免文档格式悄悄变化。
- **数据库化**：当数据从 txt 迁到 SQLite / MySQL 时再引入 `sea-orm` + `sea-orm-migration`；当前只读写 txt，引入 ORM 只会增加编译时间与复杂度。
