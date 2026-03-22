use std::time::Duration;

/// 重试调度策略配置；delays 数组按重试次数索引对应等待时间，
/// 超出索引范围时使用最后一项，实现"封顶"指数退避而非无限增长
pub struct RetryConfig {
    pub max_retries: u32,
    pub delays: Vec<Duration>,
    /// worker 轮询 pending_retries 的间隔，过短会增加 DB 压力，过长会延迟重试
    pub poll_interval: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        // 延迟序列参照 §6.2 规格：1s → 5s → 30s → 5min → 30min，
        // 与 max_retries = 5 对应，超出后进入 dead 队列
        Self {
            max_retries: 5,
            delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(5),
                Duration::from_secs(30),
                Duration::from_secs(300),
                Duration::from_secs(1800),
            ],
            poll_interval: Duration::from_secs(5),
        }
    }
}

impl RetryConfig {
    /// 按重试次数获取等待时长；attempt 从 0 开始，
    /// 超出预设序列时封顶到最后一项（最大退避），避免无效索引 panic
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        self.delays
            .get(attempt as usize)
            .copied()
            .unwrap_or(*self.delays.last().unwrap_or(&Duration::from_secs(1800)))
    }
}
