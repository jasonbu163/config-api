// Rust 不会自动发现源文件，模块必须显式声明
mod config;
mod handler;
mod parser;
mod state;

use anyhow::Context;
use axum::Router;
use axum::routing::{get, post};
use std::net::{IpAddr, SocketAddr};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 日志：默认 info 级别，可用 RUST_LOG=debug 覆盖
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // 配置文件路径：支持命令行第一个参数覆盖（cargo run -- 路径），默认 ./config.toml
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

    let target = config.config_file_path();
    let state = state::AppState::new(config);

    let app = Router::new()
        .route("/api/config", get(handler::read_config))
        .route("/api/config/update", post(handler::update_one))
        .route("/api/config/batch-update", post(handler::batch_update))
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("绑定地址失败: {addr}"))?;

    tracing::info!("目标文件: {}", target.display());
    tracing::info!("服务已启动: http://{addr}");
    tracing::info!("  GET  /api/config");
    tracing::info!("  POST /api/config/update");
    tracing::info!("  POST /api/config/batch-update");
    axum::serve(listener, app).await?;
    Ok(())
}
