use crate::config::AppConfig; // 引入上一步定义的配置结构体
use std::sync::Arc;           // Arc 让多个线程/任务安全共享同一个值
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard}; // 异步读写锁及两种 guard

/// axum 的 State 要求内部类型可 Clone 且克隆廉价，
/// 因此用 Arc 包住只读配置，供所有任务共享。
#[derive(Clone)] // 派生 Clone：克隆的是 Arc 指针，不是整份配置
pub struct AppState {         // 全局共享状态
    pub config: Arc<AppConfig>, // 配置只读，多任务通过 Arc 共享
    /// 读写锁：写请求互斥；读请求也拿读锁，保证不读到替换中途的状态。
    rw_lock: Arc<RwLock<()>>, // 锁里放 ()，因为我们只需要“互斥”本身，不保护具体数据
}

impl AppState {              // 构造函数和加锁方法
    pub fn new(config: AppConfig) -> Self { // 接收配置，构造共享状态
        Self {                             // 返回结构体实例
            config: Arc::new(config),      // 把配置放进 Arc
            rw_lock: Arc::new(RwLock::new(())), // 把 () 放进异步 RwLock，再放进 Arc
        }                                  // 结构体字段初始化结束
    }                                      // new 结束

    pub async fn lock_read(&self) -> RwLockReadGuard<'_, ()> { // 获取读锁；异步等待
        self.rw_lock.read().await // await 直到拿到读锁，返回 guard
    }                             // guard 离开作用域自动释放

    pub async fn lock_write(&self) -> RwLockWriteGuard<'_, ()> { // 获取写锁；异步等待
        self.rw_lock.write().await // await 直到其他读写锁释放，返回 guard
    }                              // guard 离开作用域自动释放
}
