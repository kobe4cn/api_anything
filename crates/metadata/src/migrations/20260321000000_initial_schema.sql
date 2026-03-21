-- Custom enum types
CREATE TYPE source_type AS ENUM ('wsdl', 'odata', 'cli', 'ssh', 'pty');
CREATE TYPE contract_status AS ENUM ('draft', 'active', 'deprecated');
CREATE TYPE http_method AS ENUM ('GET', 'POST', 'PUT', 'PATCH', 'DELETE');
CREATE TYPE protocol_type AS ENUM ('soap', 'http', 'cli', 'ssh', 'pty');
CREATE TYPE delivery_guarantee AS ENUM ('at_most_once', 'at_least_once', 'exactly_once');
CREATE TYPE artifact_type AS ENUM ('plugin_so', 'config_yaml', 'openapi_json', 'dockerfile', 'test_suite', 'agent_prompt');
CREATE TYPE build_status AS ENUM ('building', 'ready', 'failed');
CREATE TYPE delivery_status AS ENUM ('pending', 'delivered', 'failed', 'dead');
CREATE TYPE sandbox_mode AS ENUM ('mock', 'replay', 'proxy');

CREATE TABLE projects (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        VARCHAR(255) NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    owner       VARCHAR(255) NOT NULL,
    source_type source_type NOT NULL,
    source_config JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE contracts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version         VARCHAR(50) NOT NULL,
    status          contract_status NOT NULL DEFAULT 'draft',
    original_schema TEXT NOT NULL,
    parsed_model    JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, version)
);

CREATE TABLE backend_bindings (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    protocol                protocol_type NOT NULL,
    endpoint_config         JSONB NOT NULL DEFAULT '{}',
    connection_pool_config  JSONB NOT NULL DEFAULT '{"max_connections": 100, "idle_timeout_ms": 30000, "max_lifetime_ms": 300000}',
    circuit_breaker_config  JSONB NOT NULL DEFAULT '{"error_threshold_percent": 50, "window_duration_ms": 30000, "open_duration_ms": 60000, "half_open_max_requests": 3}',
    rate_limit_config       JSONB NOT NULL DEFAULT '{"requests_per_second": 1000, "burst_size": 100}',
    retry_config            JSONB NOT NULL DEFAULT '{"max_retries": 3, "base_delay_ms": 1000, "max_delay_ms": 30000}',
    timeout_ms              BIGINT NOT NULL DEFAULT 30000,
    auth_mapping            JSONB NOT NULL DEFAULT '{}'
);

CREATE TABLE routes (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_id         UUID NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    method              http_method NOT NULL,
    path                VARCHAR(1024) NOT NULL,
    request_schema      JSONB NOT NULL DEFAULT '{}',
    response_schema     JSONB NOT NULL DEFAULT '{}',
    transform_rules     JSONB NOT NULL DEFAULT '{}',
    backend_binding_id  UUID NOT NULL REFERENCES backend_bindings(id),
    delivery_guarantee  delivery_guarantee NOT NULL DEFAULT 'at_most_once',
    enabled             BOOLEAN NOT NULL DEFAULT true,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE artifacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_id     UUID NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    artifact_type   artifact_type NOT NULL,
    content_hash    VARCHAR(64) NOT NULL,
    storage_path    VARCHAR(1024) NOT NULL,
    build_status    build_status NOT NULL DEFAULT 'building',
    build_log       TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE delivery_records (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    route_id          UUID NOT NULL REFERENCES routes(id),
    trace_id          VARCHAR(64) NOT NULL,
    idempotency_key   VARCHAR(255),
    request_payload   JSONB NOT NULL,
    response_payload  JSONB,
    status            delivery_status NOT NULL DEFAULT 'pending',
    retry_count       INT NOT NULL DEFAULT 0,
    next_retry_at     TIMESTAMPTZ,
    error_message     TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE idempotency_keys (
    idempotency_key VARCHAR(255) PRIMARY KEY,
    route_id        UUID NOT NULL REFERENCES routes(id),
    status          VARCHAR(20) NOT NULL DEFAULT 'pending',
    response_hash   VARCHAR(64),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sandbox_sessions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    tenant_id   VARCHAR(255) NOT NULL,
    mode        sandbox_mode NOT NULL,
    config      JSONB NOT NULL DEFAULT '{}',
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE recorded_interactions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id  UUID NOT NULL REFERENCES sandbox_sessions(id) ON DELETE CASCADE,
    route_id    UUID NOT NULL REFERENCES routes(id),
    request     JSONB NOT NULL,
    response    JSONB NOT NULL,
    duration_ms INT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes
CREATE INDEX idx_contracts_project_id ON contracts(project_id);
CREATE INDEX idx_routes_contract_id ON routes(contract_id);
CREATE INDEX idx_routes_enabled ON routes(enabled) WHERE enabled = true;
CREATE INDEX idx_artifacts_contract_id ON artifacts(contract_id);
CREATE INDEX idx_delivery_records_status ON delivery_records(status) WHERE status IN ('pending', 'failed');
CREATE INDEX idx_delivery_records_next_retry ON delivery_records(next_retry_at) WHERE status = 'failed' AND next_retry_at IS NOT NULL;
CREATE INDEX idx_delivery_records_route_id ON delivery_records(route_id);
CREATE INDEX idx_sandbox_sessions_project ON sandbox_sessions(project_id);
CREATE INDEX idx_recorded_interactions_session ON recorded_interactions(session_id);

-- Auto-update trigger for updated_at
CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_projects_updated_at BEFORE UPDATE ON projects FOR EACH ROW EXECUTE FUNCTION update_updated_at();
CREATE TRIGGER trg_contracts_updated_at BEFORE UPDATE ON contracts FOR EACH ROW EXECUTE FUNCTION update_updated_at();
CREATE TRIGGER trg_routes_updated_at BEFORE UPDATE ON routes FOR EACH ROW EXECUTE FUNCTION update_updated_at();
CREATE TRIGGER trg_delivery_records_updated_at BEFORE UPDATE ON delivery_records FOR EACH ROW EXECUTE FUNCTION update_updated_at();
