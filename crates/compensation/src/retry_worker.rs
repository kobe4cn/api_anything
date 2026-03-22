use crate::config::RetryConfig;
use api_anything_common::models::*;
use api_anything_metadata::repo::MetadataRepo;
use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::types::GatewayRequest;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;
use axum::http::{HeaderMap, Method};
use std::collections::HashMap;

pub struct RetryWorker<R: MetadataRepo> {
    repo: Arc<R>,
    dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>>,
    config: RetryConfig,
}

impl<R: MetadataRepo + 'static> RetryWorker<R> {
    pub fn new(
        repo: Arc<R>,
        dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>>,
        config: RetryConfig,
    ) -> Self {
        Self { repo, dispatchers, config }
    }

    /// 持续轮询 pending_retries，批量处理后休眠 poll_interval；
    /// 无限循环设计使 worker 天然具备自愈能力，单批失败不会终止整个 worker
    pub async fn run(&self) {
        tracing::info!("Retry worker started");
        loop {
            if let Err(e) = self.process_batch().await {
                tracing::error!(error = %e, "Retry worker batch error");
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    async fn process_batch(&self) -> Result<(), anyhow::Error> {
        let records = self.repo.list_pending_retries(100).await?;
        for record in records {
            self.retry_one(&record).await;
        }
        Ok(())
    }

    async fn retry_one(&self, record: &DeliveryRecord) {
        // dispatcher 不存在意味着路由已被删除或网关重启未重载，
        // 继续重试没有意义，直接转为 dead 状态等待人工处理
        let dispatcher = match self.dispatchers.get(&record.route_id) {
            Some(d) => d.clone(),
            None => {
                tracing::warn!(route_id = %record.route_id, "No dispatcher for route, moving to dead letter");
                let _ = self.repo.update_delivery_status(
                    record.id,
                    DeliveryStatus::Dead,
                    Some("No dispatcher available for route"),
                    None,
                ).await;
                return;
            }
        };

        // 从持久化的请求体重建 GatewayRequest；
        // method 和 path 在 AtLeastOnce/ExactlyOnce 场景下主要用于透传，
        // 实际协议适配由 adapter.transform_request 处理，故使用占位值不影响业务正确性
        let gateway_req = GatewayRequest {
            route_id: record.route_id,
            method: Method::POST,
            path: String::new(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: Some(record.request_payload.clone()),
            trace_id: record.trace_id.clone(),
        };

        match dispatcher.dispatch(gateway_req).await {
            Ok(_resp) => {
                tracing::info!(record_id = %record.id, "Retry succeeded");
                let _ = self.repo.update_delivery_status(
                    record.id,
                    DeliveryStatus::Delivered,
                    None,
                    None,
                ).await;
                // 重试成功后同步更新幂等键状态，确保后续相同 key 的请求
                // 能在 check_idempotency 阶段得到 delivered 而非 pending
                if let Some(key) = &record.idempotency_key {
                    let _ = self.repo.mark_idempotency_delivered(key, "retry-success").await;
                }
            }
            Err(e) => {
                let next_attempt = record.retry_count as u32 + 1;
                if next_attempt >= self.config.max_retries {
                    // 超出最大重试次数，进入 dead letter 队列，
                    // 后续由运维人员通过管理 API 手动干预
                    tracing::error!(
                        record_id = %record.id,
                        attempts = next_attempt,
                        "Max retries exceeded, moving to dead letter"
                    );
                    let _ = self.repo.update_delivery_status(
                        record.id,
                        DeliveryStatus::Dead,
                        Some(&format!("{e}")),
                        None,
                    ).await;
                } else {
                    // 按指数退避调度下次重试时间，避免对已故障的后端产生持续压力
                    let delay = self.config.delay_for_attempt(next_attempt);
                    let next_retry = chrono::Utc::now()
                        + chrono::Duration::from_std(delay)
                            .unwrap_or(chrono::Duration::seconds(60));
                    tracing::warn!(
                        record_id = %record.id,
                        attempt = next_attempt,
                        "Retry failed, scheduling next"
                    );
                    let _ = self.repo.update_delivery_status(
                        record.id,
                        DeliveryStatus::Failed,
                        Some(&format!("{e}")),
                        Some(next_retry),
                    ).await;
                }
            }
        }
    }
}
