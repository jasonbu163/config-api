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

use crate::download;              // 文件名校验与候选收集模块
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

/// 下载检查请求体：{ "name": "A12331", "type": "xls" }
#[derive(Debug, Deserialize)]     // 可打印；从 JSON 请求体反序列化
pub struct CheckRequest {         // 对应 POST /api/download/check
    pub name: String,             // 文件标识；必填，单段安全文件名
    #[serde(rename = "type", default)] // type 是 Rust 关键字，且允许省略
    pub file_type: Option<String>, // 可选扩展名；必须是配置 extensions 的子集
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

/// GET /download/{name} —— 从配置目录取文件。
/// 两级精确匹配；0 个候选 404，1 个流式返回，多个 409 不猜。
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
