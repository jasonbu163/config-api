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
