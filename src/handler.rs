use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, extract::State};
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
pub async fn update_one(State(state): State<AppState>, Json(req): Json<UpdateRequest>) -> Response {
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
    let updates: Vec<(String, String)> = req.items.into_iter().map(|i| (i.name, i.value)).collect();

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
    if req.value.contains('\n')
        || req.value.contains('\r')
        || req.value.trim_start().starts_with('@')
    {
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
        // 备份失败不阻断主流程
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
