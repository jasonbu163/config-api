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
