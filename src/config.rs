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
