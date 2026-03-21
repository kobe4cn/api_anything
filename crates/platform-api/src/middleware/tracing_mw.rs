use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::TracerProvider;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

// 初始化 tracing + OTel：
// - JSON 格式日志便于生产环境日志聚合系统（如 Loki、ELK）解析
// - OTel span 通过 batch exporter 异步发送，避免阻塞请求路径
// - 若 OTel 初始化失败（如 collector 未就绪），降级为纯 tracing-subscriber，保证进程可正常启动
pub fn init_tracing(otel_endpoint: &str) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer().json();

    // 尝试初始化 OTel；若失败则降级，不因遥测初始化而中断服务启动
    match try_build_tracer(otel_endpoint) {
        Ok(tracer) => {
            // OTel layer 需要放在最外层（最后 .with()），
            // 因为它依赖底层 subscriber 的 LookupSpan 实现，而该实现来自 Registry
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .with(OpenTelemetryLayer::new(tracer))
                .init();
        }
        Err(e) => {
            // OTel 不可用时仅打印 stderr 警告（此时 tracing 还未初始化），
            // 服务仍然可以运行并输出结构化日志
            eprintln!("WARN: OTel tracing init failed, falling back to tracing-subscriber only: {e}");
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
    }
}

fn try_build_tracer(
    otel_endpoint: &str,
) -> Result<opentelemetry_sdk::trace::Tracer, Box<dyn std::error::Error>> {
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(otel_endpoint)
        .build()?;

    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();

    global::set_tracer_provider(provider.clone());

    let tracer = provider.tracer("api-anything-platform-api");
    Ok(tracer)
}
