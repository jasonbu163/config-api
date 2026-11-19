use crate::config::DownloadConfig; // 下载配置
use std::fs;                       // 读目录与元数据
use std::path::{Path, PathBuf};    // 路径类型

/// 请求文件名不合法的原因
#[derive(Debug)]                   // 便于日志打印
pub enum NameError {               // 所有拒绝理由
    Empty,                         // 空串
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
