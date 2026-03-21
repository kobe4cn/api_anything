use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use api_anything_common::error::AppError;

/// 对 tokio Semaphore 的封装，提供有界并发控制；
/// Permit 通过 RAII 自动归还，无需手动释放
pub struct ConcurrencySemaphore {
    semaphore: Arc<Semaphore>,
    max_permits: u32,
}

impl ConcurrencySemaphore {
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent as usize)),
            max_permits: max_concurrent,
        }
    }

    /// 异步等待直到获取到许可；信号量被关闭时（正常情况不会发生）才返回错误
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, AppError> {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Internal("Semaphore closed".to_string()))
    }

    /// 非阻塞尝试获取许可，无可用许可时立即返回错误，适用于不愿排队等待的场景
    pub fn try_acquire(&self) -> Result<OwnedSemaphorePermit, AppError> {
        self.semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Internal("Concurrency limit reached".to_string()))
    }

    pub fn available_permits(&self) -> u32 {
        self.semaphore.available_permits() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquires_within_limit() {
        let sem = ConcurrencySemaphore::new(3);
        let _p1 = sem.acquire().await.unwrap();
        let _p2 = sem.acquire().await.unwrap();
        let _p3 = sem.acquire().await.unwrap();
        assert_eq!(sem.available_permits(), 0);
    }

    #[tokio::test]
    async fn releases_on_drop() {
        let sem = ConcurrencySemaphore::new(1);
        {
            let _permit = sem.acquire().await.unwrap();
            assert_eq!(sem.available_permits(), 0);
        }
        assert_eq!(sem.available_permits(), 1);
    }

    #[tokio::test]
    async fn try_acquire_fails_when_exhausted() {
        let sem = ConcurrencySemaphore::new(1);
        let _p1 = sem.acquire().await.unwrap();
        assert!(sem.try_acquire().is_err());
    }
}
