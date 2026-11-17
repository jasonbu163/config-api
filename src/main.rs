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
    Json(json!({
        "ok": true,
        "message": "Hello, axum!"
    }))
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