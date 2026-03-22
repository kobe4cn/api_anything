import { useEffect, useState } from 'react';
import { api } from '../api/client';

interface RouteInfo {
  method: string;
  path: string;
  requestSchema?: any;
  responseSchema?: any;
}

interface HistoryEntry {
  method: string;
  path: string;
  status: number;
  duration: number;
  timestamp: string;
  requestBody?: any;
  responseBody?: any;
}

const HISTORY_KEY = 'api-explorer-history';
const MAX_HISTORY = 20;

// 状态码颜色映射：2xx 绿色 / 4xx 黄色 / 5xx 红色 / 其他灰色
function statusColor(status: number): string {
  if (status >= 200 && status < 300) return 'text-green-600';
  if (status >= 400 && status < 500) return 'text-yellow-600';
  if (status >= 500) return 'text-red-600';
  return 'text-gray-600';
}

function methodColor(method: string): string {
  const colors: Record<string, string> = {
    get: 'bg-green-100 text-green-800',
    post: 'bg-blue-100 text-blue-800',
    put: 'bg-orange-100 text-orange-800',
    patch: 'bg-yellow-100 text-yellow-800',
    delete: 'bg-red-100 text-red-800',
  };
  return colors[method.toLowerCase()] || 'bg-gray-100 text-gray-800';
}

export function ApiExplorer() {
  const [routes, setRoutes] = useState<RouteInfo[]>([]);
  const [selectedRoute, setSelectedRoute] = useState<RouteInfo | null>(null);
  const [formFields, setFormFields] = useState<Record<string, string>>({});
  const [target, setTarget] = useState<'gw' | 'sandbox-mock' | 'sandbox-replay'>('gw');
  const [response, setResponse] = useState<any>(null);
  const [loading, setLoading] = useState(false);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [showHistory, setShowHistory] = useState(false);

  useEffect(() => {
    loadRoutes();
    loadHistory();
  }, []);

  async function loadRoutes() {
    try {
      const spec = await api.getOpenApiSpec();
      const parsed: RouteInfo[] = [];
      if (spec.paths) {
        for (const [path, methods] of Object.entries(spec.paths as Record<string, any>)) {
          for (const [method, operation] of Object.entries(methods as Record<string, any>)) {
            const schema = operation.requestBody?.content?.['application/json']?.schema;
            const respSchema = operation.responses?.['200']?.content?.['application/json']?.schema;
            parsed.push({
              method: method.toUpperCase(),
              path,
              requestSchema: schema,
              responseSchema: respSchema,
            });
          }
        }
      }
      setRoutes(parsed);
    } catch (e) {
      console.error('Failed to load OpenAPI spec:', e);
    }
  }

  function loadHistory() {
    try {
      const saved = localStorage.getItem(HISTORY_KEY);
      if (saved) setHistory(JSON.parse(saved));
    } catch { /* localStorage 不可用时忽略 */ }
  }

  function saveHistory(entry: HistoryEntry) {
    const updated = [entry, ...history].slice(0, MAX_HISTORY);
    setHistory(updated);
    try { localStorage.setItem(HISTORY_KEY, JSON.stringify(updated)); }
    catch { /* 忽略 */ }
  }

  function selectRoute(route: RouteInfo) {
    setSelectedRoute(route);
    setResponse(null);
    // 根据 schema 自动生成表单初始值
    const fields: Record<string, string> = {};
    if (route.requestSchema?.properties) {
      for (const [name] of Object.entries(route.requestSchema.properties)) {
        fields[name] = '';
      }
    }
    setFormFields(fields);
  }

  async function sendRequest() {
    if (!selectedRoute) return;
    setLoading(true);
    setResponse(null);

    // 根据目标选择路径前缀
    let basePath = selectedRoute.path;
    const headers: Record<string, string> = {};
    if (target === 'sandbox-mock') {
      basePath = selectedRoute.path.replace(/^\/gw/, '/sandbox');
      headers['X-Sandbox-Mode'] = 'mock';
    } else if (target === 'sandbox-replay') {
      basePath = selectedRoute.path.replace(/^\/gw/, '/sandbox');
      headers['X-Sandbox-Mode'] = 'replay';
    }

    // 将表单字段转换为请求体，尝试解析数值类型
    let body: any = undefined;
    if (Object.keys(formFields).length > 0 && selectedRoute.method !== 'GET' && selectedRoute.method !== 'DELETE') {
      body = {};
      for (const [key, val] of Object.entries(formFields)) {
        const propType = selectedRoute.requestSchema?.properties?.[key]?.type;
        if (propType === 'integer' || propType === 'number') {
          body[key] = Number(val) || 0;
        } else if (propType === 'boolean') {
          body[key] = val === 'true';
        } else {
          body[key] = val;
        }
      }
    }

    try {
      const result = await api.sendRequest(selectedRoute.method, basePath, body, headers);
      setResponse(result);

      saveHistory({
        method: selectedRoute.method,
        path: basePath,
        status: result.status,
        duration: result.duration,
        timestamp: new Date().toISOString(),
        requestBody: body,
        responseBody: result.body,
      });
    } catch (e: any) {
      setResponse({ status: 0, statusText: 'Network Error', body: e.message, duration: 0 });
    }
    setLoading(false);
  }

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <h2 className="text-2xl font-bold">API Explorer</h2>
        <button
          onClick={() => setShowHistory(!showHistory)}
          className="px-3 py-1 rounded bg-gray-200 hover:bg-gray-300 text-sm"
        >
          {showHistory ? 'Hide History' : `History (${history.length})`}
        </button>
      </div>

      {showHistory && (
        <div className="bg-white rounded-lg shadow mb-4 p-4 max-h-60 overflow-auto">
          <h3 className="font-semibold mb-2">Recent Requests</h3>
          {history.length === 0 ? (
            <p className="text-gray-400 text-sm">No history yet.</p>
          ) : (
            <div className="space-y-1">
              {history.map((h, i) => (
                <div key={i} className="flex items-center gap-2 text-sm py-1 border-b border-gray-100">
                  <span className={`px-1.5 py-0.5 rounded text-xs font-mono ${methodColor(h.method)}`}>
                    {h.method}
                  </span>
                  <span className="font-mono text-gray-700 flex-1 truncate">{h.path}</span>
                  <span className={`font-mono ${statusColor(h.status)}`}>{h.status}</span>
                  <span className="text-gray-400">{h.duration}ms</span>
                  <span className="text-gray-300 text-xs">{new Date(h.timestamp).toLocaleTimeString()}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      <div className="flex gap-4" style={{ height: 'calc(100vh - 240px)' }}>
        {/* 左侧路由列表 */}
        <div className="w-80 bg-white rounded-lg shadow overflow-auto">
          <div className="p-3 bg-gray-50 border-b font-semibold text-sm">
            Routes ({routes.length})
          </div>
          {routes.length === 0 && (
            <p className="p-4 text-gray-400 text-sm">No routes available. Configure routes first.</p>
          )}
          {routes.map((r, i) => (
            <div
              key={i}
              onClick={() => selectRoute(r)}
              className={`flex items-center gap-2 px-3 py-2.5 cursor-pointer border-b border-gray-50 hover:bg-blue-50 ${
                selectedRoute === r ? 'bg-blue-50 border-l-4 border-l-blue-500' : ''
              }`}
            >
              <span className={`px-1.5 py-0.5 rounded text-xs font-mono font-semibold ${methodColor(r.method)}`}>
                {r.method}
              </span>
              <span className="font-mono text-sm truncate">{r.path}</span>
            </div>
          ))}
        </div>

        {/* 右侧请求构建器 */}
        <div className="flex-1 bg-white rounded-lg shadow overflow-auto p-4">
          {!selectedRoute ? (
            <div className="flex items-center justify-center h-full text-gray-400">
              Select a route from the left panel
            </div>
          ) : (
            <div className="space-y-4">
              {/* 路由信息 */}
              <div className="flex items-center gap-2">
                <span className={`px-2 py-1 rounded text-sm font-mono font-bold ${methodColor(selectedRoute.method)}`}>
                  {selectedRoute.method}
                </span>
                <span className="font-mono text-lg">{selectedRoute.path}</span>
              </div>

              {/* 目标选择 */}
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">Target</label>
                <div className="flex gap-2">
                  {[
                    { value: 'gw' as const, label: 'Production Gateway (/gw)' },
                    { value: 'sandbox-mock' as const, label: 'Sandbox Mock' },
                    { value: 'sandbox-replay' as const, label: 'Sandbox Replay' },
                  ].map(t => (
                    <button
                      key={t.value}
                      onClick={() => setTarget(t.value)}
                      className={`px-3 py-1.5 rounded text-sm ${
                        target === t.value
                          ? 'bg-blue-600 text-white'
                          : 'bg-gray-100 hover:bg-gray-200'
                      }`}
                    >
                      {t.label}
                    </button>
                  ))}
                </div>
              </div>

              {/* 请求体表单 */}
              {Object.keys(formFields).length > 0 && (
                <div>
                  <label className="block text-sm font-medium text-gray-700 mb-1">Request Body</label>
                  <div className="space-y-2">
                    {Object.entries(formFields).map(([name, value]) => {
                      const propType = selectedRoute.requestSchema?.properties?.[name]?.type || 'string';
                      return (
                        <div key={name} className="flex items-center gap-2">
                          <label className="w-32 text-sm font-mono text-gray-600">{name}</label>
                          <span className="text-xs text-gray-400 w-16">({propType})</span>
                          <input
                            type={propType === 'integer' || propType === 'number' ? 'number' : 'text'}
                            value={value}
                            onChange={e => setFormFields({ ...formFields, [name]: e.target.value })}
                            className="flex-1 border rounded px-2 py-1 text-sm font-mono"
                            placeholder={`Enter ${name}`}
                          />
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Send 按钮 */}
              <button
                onClick={sendRequest}
                disabled={loading}
                className="bg-blue-600 text-white px-6 py-2 rounded hover:bg-blue-700 disabled:bg-gray-400"
              >
                {loading ? 'Sending...' : 'Send Request'}
              </button>

              {/* 响应展示 */}
              {response && (
                <div className="border rounded-lg overflow-hidden">
                  <div className="flex items-center gap-4 p-3 bg-gray-50 border-b">
                    <span className={`text-lg font-bold ${statusColor(response.status)}`}>
                      {response.status} {response.statusText}
                    </span>
                    <span className="text-sm text-gray-500">{response.duration}ms</span>
                  </div>
                  <pre className="p-4 text-sm font-mono bg-gray-900 text-green-400 overflow-auto max-h-96 whitespace-pre-wrap">
                    {typeof response.body === 'object'
                      ? JSON.stringify(response.body, null, 2)
                      : String(response.body)}
                  </pre>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
