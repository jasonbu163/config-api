use anyhow::{Result, anyhow};

/// 一条配置项：`@key` 与其下一行的 value。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: String,
}

/// 解析文件内容为结构化条目。
/// 规则：以 @ 开头的行是字段名，紧随其后的下一行是值（值可以为空行）。
/// 空行与不含 @ 的散行被忽略，CRLF 行尾会被裁掉，兼容 Windows 记事本。
pub fn parse(content: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        if let Some(key) = line.strip_prefix('@') {
            // 取下一行作为值；若文件以 @key 结尾则值为空字符串
            let value = match lines.next() {
                Some(v) => v.trim_end_matches(['\r', '\n']).to_string(),
                None => String::new(),
            };
            entries.push(Entry {
                key: key.trim().to_string(),
                value,
            });
        }
    }
    entries
}

/// 把条目重新渲染成文件文本。统一使用 "\n" 结尾，
/// 输出稳定，便于 diff 与版本对比。
pub fn render(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push('@');
        out.push_str(&e.key);
        out.push('\n');
        out.push_str(&e.value);
        out.push('\n');
    }
    out
}

/// 应用一组修改，返回实际被修改的字段名列表。
/// 任一字段不存在则整体报错（批量接口据此实现"全成功或全不改"）。
pub fn apply(entries: &mut [Entry], updates: &[(String, String)]) -> Result<Vec<String>> {
    let mut applied = Vec::new();

    for (name, value) in updates {
        // 防御：值里不允许出现换行或以 @ 开头，否则会破坏文件格式
        if value.contains('\n') || value.contains('\r') || value.trim_start().starts_with('@') {
            return Err(anyhow!("字段 {name} 的值包含非法字符（换行或以 @ 开头）"));
        }

        match entries.iter_mut().find(|e| &e.key == name) {
            Some(entry) => {
                entry.value = value.clone();
                applied.push(name.clone());
            }
            None => return Err(anyhow!("文件中不存在字段: {name}")),
        }
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "@DataName\r\nTT26071311A002\r\n@Thickness\r\n25\r\n@Temper\r\nT3\r\n";

    #[test]
    fn parses_crlf_and_ignores_blank_lines() {
        let entries = parse(SAMPLE);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key, "DataName");
        assert_eq!(entries[0].value, "TT26071311A002");
        assert_eq!(entries[2].key, "Temper");
        assert_eq!(entries[2].value, "T3");
    }

    #[test]
    fn render_roundtrip_is_stable() {
        assert_eq!(render(&parse(SAMPLE)), SAMPLE.replace("\r\n", "\n"));
    }

    #[test]
    fn apply_updates_existing_field() {
        let mut entries = parse(SAMPLE);
        let applied = apply(&mut entries, &[("Thickness".into(), "30".into())]).unwrap();
        assert_eq!(applied, vec!["Thickness".to_string()]);
        assert!(render(&entries).contains("@Thickness\n30\n"));
    }

    #[test]
    fn apply_rejects_missing_field() {
        let mut entries = parse(SAMPLE);
        assert!(apply(&mut entries, &[("Nope".into(), "1".into())]).is_err());
    }

    #[test]
    fn apply_rejects_illegal_value() {
        let mut entries = parse(SAMPLE);
        assert!(apply(&mut entries, &[("Temper".into(), "T3\nevil".into())]).is_err());
        assert!(apply(&mut entries, &[("Temper".into(), "@evil".into())]).is_err());
    }
}
