const BASE_URL = '';

// 统一的HTTP请求封装：错误时优先使用后端返回的detail/title字段描述错误
async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const resp = await fetch(`${BASE_URL}${path}`, {
    headers: { 'Content-Type': 'application/json', ...options?.headers },
    ...options,
  });
  if (!resp.ok) {
    const error = await resp.json().catch(() => ({ detail: resp.statusText }));
    throw new Error(error.detail || error.title || `HTTP ${resp.status}`);
  }
  return resp.json();
}

export const api = {
  // Projects
  listProjects: () => request<any[]>('/api/v1/projects'),
  createProject: (data: any) => request<any>('/api/v1/projects', { method: 'POST', body: JSON.stringify(data) }),
  getProject: (id: string) => request<any>(`/api/v1/projects/${id}`),
  deleteProject: (id: string) => request<void>(`/api/v1/projects/${id}`, { method: 'DELETE' }),

  // Sandbox Sessions
  listSessions: (projectId: string) => request<any[]>(`/api/v1/projects/${projectId}/sandbox-sessions`),
  createSession: (projectId: string, data: any) => request<any>(`/api/v1/projects/${projectId}/sandbox-sessions`, { method: 'POST', body: JSON.stringify(data) }),
  deleteSession: (id: string) => request<void>(`/api/v1/sandbox-sessions/${id}`, { method: 'DELETE' }),

  // Compensation
  listDeadLetters: (params?: string) => request<any[]>(`/api/v1/compensation/dead-letters${params ? '?' + params : ''}`),
  retryDeadLetter: (id: string) => request<void>(`/api/v1/compensation/dead-letters/${id}/retry`, { method: 'POST' }),
  resolveDeadLetter: (id: string) => request<void>(`/api/v1/compensation/dead-letters/${id}/resolve`, { method: 'POST' }),
  batchRetry: (ids: string[]) => request<any>('/api/v1/compensation/dead-letters/batch-retry', { method: 'POST', body: JSON.stringify({ ids }) }),

  // Webhooks
  listWebhooks: () => request<any[]>('/api/v1/webhooks'),
  createWebhook: (data: { url: string; event_types: string[]; description: string }) =>
    request<any>('/api/v1/webhooks', { method: 'POST', body: JSON.stringify(data) }),
  deleteWebhook: (id: string) => request<void>(`/api/v1/webhooks/${id}`, { method: 'DELETE' }),

  // Docs
  getAgentPrompt: () => fetch('/api/v1/docs/agent-prompt').then(r => r.text()),

  // Health
  health: () => request<any>('/health'),
};
