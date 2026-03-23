use api_anything_common::error::AppError;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};

/// JWT 解码后的 claims 结构，注入到请求 extensions 中供下游 handler 消费
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Claims {
    pub sub: String,
    pub role: Option<String>,
    pub exp: u64,
}

/// 认证配置，从环境变量初始化；enabled=false 时整个中间件透传请求，
/// 使开发环境零配置可用，生产环境通过 AUTH_ENABLED=true 启用强制认证
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub enabled: bool,
    pub jwt_secret: String,
    pub skip_paths: Vec<String>,
}

impl AuthConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("AUTH_ENABLED")
                .map(|v| v == "true")
                .unwrap_or(false),
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "dev-secret-change-in-production".to_string()),
            // 健康检查和文档端点跳过认证，保证 K8s 探针和 Swagger UI 不需要 token
            skip_paths: vec![
                "/health".to_string(),
                "/health/ready".to_string(),
                "/api/v1/docs".to_string(),
            ],
        }
    }
}

/// Axum 中间件：验证 JWT Bearer Token 并将 Claims 注入请求 extensions
pub async fn auth_middleware(
    config: AuthConfig,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    if !config.enabled {
        return Ok(next.run(req).await);
    }

    let path = req.uri().path().to_string();
    if config.skip_paths.iter().any(|p| path.starts_with(p)) {
        return Ok(next.run(req).await);
    }

    // 从 Authorization header 中提取 Bearer token
    let token = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;

    let key = DecodingKey::from_secret(config.jwt_secret.as_bytes());
    let validation = Validation::new(Algorithm::HS256);
    let token_data = decode::<Claims>(token, &key, &validation)
        .map_err(|_| AppError::Unauthorized)?;

    // 将解码后的 claims 注入 extensions，下游 handler 可通过 Extension<Claims> 提取
    req.extensions_mut().insert(token_data.claims);

    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use tower::ServiceExt;

    fn make_auth_config(enabled: bool, secret: &str) -> AuthConfig {
        AuthConfig {
            enabled,
            jwt_secret: secret.to_string(),
            skip_paths: vec![
                "/health".to_string(),
                "/health/ready".to_string(),
                "/api/v1/docs".to_string(),
            ],
        }
    }

    fn make_token(secret: &str, sub: &str, exp: u64) -> String {
        let claims = Claims {
            sub: sub.to_string(),
            role: Some("admin".to_string()),
            exp,
        };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
            .unwrap()
    }

    fn build_test_app(config: AuthConfig) -> Router {
        Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/api/v1/data", get(|| async { "data" }))
            .layer(middleware::from_fn(move |req, next| {
                let cfg = config.clone();
                auth_middleware(cfg, req, next)
            }))
    }

    #[test]
    fn auth_config_defaults_to_disabled() {
        // 未设置 AUTH_ENABLED 时默认关闭，保证开发环境零配置可用
        let config = AuthConfig::from_env();
        assert!(!config.enabled);
    }

    #[tokio::test]
    async fn auth_disabled_passes_all_requests() {
        let app = build_test_app(make_auth_config(false, "secret"));
        let req = HttpRequest::builder()
            .uri("/api/v1/data")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_enabled_rejects_missing_token() {
        let app = build_test_app(make_auth_config(true, "secret"));
        let req = HttpRequest::builder()
            .uri("/api/v1/data")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_enabled_rejects_invalid_token() {
        let app = build_test_app(make_auth_config(true, "secret"));
        let req = HttpRequest::builder()
            .uri("/api/v1/data")
            .header("Authorization", "Bearer invalid-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_enabled_accepts_valid_token() {
        let secret = "test-secret-key";
        // exp 设为未来时间，确保 token 有效
        let exp = chrono::Utc::now().timestamp() as u64 + 3600;
        let token = make_token(secret, "user-1", exp);

        let app = build_test_app(make_auth_config(true, secret));
        let req = HttpRequest::builder()
            .uri("/api/v1/data")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_enabled_rejects_expired_token() {
        let secret = "test-secret-key";
        // exp 设为过去时间，触发过期校验
        let token = make_token(secret, "user-1", 1000);

        let app = build_test_app(make_auth_config(true, secret));
        let req = HttpRequest::builder()
            .uri("/api/v1/data")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_enabled_skips_health_endpoint() {
        // 白名单路径即使启用认证也不要求 token，保证 K8s 探针正常工作
        let app = build_test_app(make_auth_config(true, "secret"));
        let req = HttpRequest::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_enabled_rejects_wrong_secret() {
        // 用不同 secret 签发的 token 应被拒绝
        let exp = chrono::Utc::now().timestamp() as u64 + 3600;
        let token = make_token("wrong-secret", "user-1", exp);

        let app = build_test_app(make_auth_config(true, "correct-secret"));
        let req = HttpRequest::builder()
            .uri("/api/v1/data")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
