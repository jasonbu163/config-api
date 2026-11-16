use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 顶层配置结构。
/// `deny_unknown_fields` 让写错的配置项在启动期直接报错，
/// 而不是被静默忽略——对运维排错非常关键。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub files: FilesConfig,
    pub allowed_fields: AllowedFields,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesConfig {
    pub base_dir: String,
    pub file_name: String,
    #[serde(default)]
    pub backup: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedFields {
    pub fields: Vec<String>,
}

impl AppConfig {
    /// 读取磁盘配置并立即做业务校验。
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        let cfg: AppConfig =
            toml::from_str(&text).with_context(|| format!("解析 TOML 失败: {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// 启动期校验：宁可启动失败，也不要运行到一半才出错。
    fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            return Err(anyhow!("server.port 不能为 0"));
        }
        let dir = Path::new(&self.files.base_dir);
        if !dir.is_dir() {
            return Err(anyhow!("base_dir 不存在或不是目录: {}", dir.display()));
        }
        if self.files.file_name.trim().is_empty() {
            return Err(anyhow!("files.file_name 不能为空"));
        }
        if self.allowed_fields.fields.is_empty() {
            return Err(anyhow!("allowed_fields.fields 不能为空"));
        }
        Ok(())
    }

    /// 拼出被修改文件的完整路径。
    pub fn config_file_path(&self) -> PathBuf {
        Path::new(&self.files.base_dir).join(&self.files.file_name)
    }

    /// 白名单判定：只有列出的字段才允许通过接口修改。
    pub fn is_allowed(&self, name: &str) -> bool {
        self.allowed_fields.fields.iter().any(|f| f == name)
    }
}
