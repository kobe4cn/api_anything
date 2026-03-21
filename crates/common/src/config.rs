use std::env;

// 所有配置项均提供合理的本地开发默认值，保证开箱即用，
// 生产部署时通过环境变量覆盖，无需修改代码
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub kafka_brokers: String,
    pub otel_endpoint: String,
    pub api_host: String,
    pub api_port: u16,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string()),
            kafka_brokers: env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "localhost:9092".to_string()),
            otel_endpoint: env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:4317".to_string()),
            api_host: env::var("API_HOST")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            // API_PORT 解析失败时回退到 8080，而非 panic，保证启动健壮性
            api_port: env::var("API_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
        }
    }
}
