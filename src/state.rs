use crate::config::AppConfig;
use std::sync::Arc;
use tokio::sync::RwLock;

/// axum 的 State 要求内部类型可 Clone 且克隆廉价，
/// 因此用 Arc 包住只读配置，供所有任务共享。
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    /// 写锁：保证同一时刻只有一个请求在改写目标文件，
    /// 避免并发 POST 互相覆盖或写出半截内容。
    write_lock: Arc<RwLock<()>>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
            write_lock: Arc::new(RwLock::new(())),
        }
    }

    /// 获取写权限。返回的 guard 在作用域结束时自动释放锁。
    pub async fn lock_write(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.write_lock.write().await
    }
}
