use arc_swap::ArcSwap;
use axum::http::Method;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// 路由表 — 支持 {param} 占位符的方法+路径 → route_id 映射
pub struct RouteTable {
    routes: Vec<RouteEntry>,
}

struct RouteEntry {
    method: Method,
    segments: Vec<PathSegment>,
    route_id: Uuid,
}

enum PathSegment {
    Literal(String),
    /// 动态段，花括号内的名称作为参数 key
    Param(String),
}

impl RouteTable {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    pub fn insert(&mut self, method: Method, path: &str, route_id: Uuid) {
        let segments = Self::parse_segments(path);
        self.routes.push(RouteEntry { method, segments, route_id });
    }

    /// 线性扫描所有条目；路由数量通常较少，无需额外索引结构
    pub fn match_route(
        &self,
        method: &Method,
        path: &str,
    ) -> Option<(Uuid, HashMap<String, String>)> {
        let request_segments: Vec<&str> =
            path.split('/').filter(|s| !s.is_empty()).collect();

        for entry in &self.routes {
            if &entry.method != method {
                continue;
            }
            // 段数不同时直接跳过，避免无意义的逐段比较
            if entry.segments.len() != request_segments.len() {
                continue;
            }

            let mut params = HashMap::new();
            let mut matched = true;
            for (seg, req_seg) in entry.segments.iter().zip(&request_segments) {
                match seg {
                    PathSegment::Literal(lit) => {
                        if lit != req_seg {
                            matched = false;
                            break;
                        }
                    }
                    PathSegment::Param(name) => {
                        params.insert(name.clone(), req_seg.to_string());
                    }
                }
            }
            if matched {
                return Some((entry.route_id, params));
            }
        }
        None
    }

    /// 将路径字符串拆分为静态字面量段与动态参数段
    fn parse_segments(path: &str) -> Vec<PathSegment> {
        path.split('/')
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with('{') && s.ends_with('}') {
                    PathSegment::Param(s[1..s.len() - 1].to_string())
                } else {
                    PathSegment::Literal(s.to_string())
                }
            })
            .collect()
    }
}

/// 动态路由器 — 通过 ArcSwap 实现 RCU（读拷贝更新），路由表可原子替换，不中断进行中的请求
pub struct DynamicRouter {
    route_table: ArcSwap<RouteTable>,
}

impl DynamicRouter {
    pub fn new() -> Self {
        Self {
            route_table: ArcSwap::new(Arc::new(RouteTable::new())),
        }
    }

    /// 用新的路由表原子替换旧表；旧表在所有持有引用释放后才被回收
    pub fn update(&self, table: RouteTable) {
        self.route_table.store(Arc::new(table));
    }

    pub fn match_route(
        &self,
        method: &Method,
        path: &str,
    ) -> Option<(Uuid, HashMap<String, String>)> {
        let table = self.route_table.load();
        table.match_route(method, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_path() {
        let mut table = RouteTable::new();
        let id = Uuid::new_v4();
        table.insert(Method::GET, "/api/v1/orders", id);
        assert_eq!(table.match_route(&Method::GET, "/api/v1/orders").unwrap().0, id);
    }

    #[test]
    fn matches_path_with_params() {
        let mut table = RouteTable::new();
        let id = Uuid::new_v4();
        table.insert(Method::GET, "/api/v1/orders/{id}", id);
        let (matched_id, params) =
            table.match_route(&Method::GET, "/api/v1/orders/abc-123").unwrap();
        assert_eq!(matched_id, id);
        assert_eq!(params.get("id").unwrap(), "abc-123");
    }

    #[test]
    fn returns_none_for_unmatched() {
        let table = RouteTable::new();
        assert!(table.match_route(&Method::GET, "/unknown").is_none());
    }

    #[test]
    fn distinguishes_http_methods() {
        let mut table = RouteTable::new();
        let get_id = Uuid::new_v4();
        let post_id = Uuid::new_v4();
        table.insert(Method::GET, "/api/v1/orders", get_id);
        table.insert(Method::POST, "/api/v1/orders", post_id);
        assert_eq!(
            table.match_route(&Method::GET, "/api/v1/orders").unwrap().0,
            get_id
        );
        assert_eq!(
            table.match_route(&Method::POST, "/api/v1/orders").unwrap().0,
            post_id
        );
    }
}
