import { useEffect, useState } from 'react';
import { api } from '../api/client';

export function SandboxManager() {
  const [projects, setProjects] = useState<any[]>([]);
  const [selectedProject, setSelectedProject] = useState<string>('');
  const [sessions, setSessions] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [form, setForm] = useState({ tenant_id: '', mode: 'mock', config: '{}', expires_in_hours: 24 });

  useEffect(() => { api.listProjects().then(setProjects).catch(console.error); }, []);

  useEffect(() => {
    if (selectedProject) loadSessions();
  }, [selectedProject]);

  async function loadSessions() {
    setLoading(true);
    try { setSessions(await api.listSessions(selectedProject)); }
    catch (e) { console.error(e); }
    setLoading(false);
  }

  async function createSession(e: React.FormEvent) {
    e.preventDefault();
    let config;
    try { config = JSON.parse(form.config); } catch { config = {}; }
    await api.createSession(selectedProject, { ...form, config, expires_in_hours: Number(form.expires_in_hours) });
    setShowCreate(false);
    loadSessions();
  }

  async function deleteSession(id: string) {
    if (!confirm('Delete this sandbox session?')) return;
    await api.deleteSession(id);
    loadSessions();
  }

  function curlExample(session: any) {
    return `curl -X POST http://localhost:8080/sandbox/api/v1/... \\
  -H "Content-Type: application/json" \\
  -H "X-Sandbox-Mode: ${session.mode}" \\
  -H "X-Sandbox-Session: ${session.id}" \\
  -d '{"key": "value"}'`;
  }

  return (
    <div>
      <h2 className="text-2xl font-bold mb-4">Sandbox Manager</h2>

      <div className="mb-4">
        <label className="block text-sm font-medium mb-1">Select Project</label>
        <select value={selectedProject} onChange={e => setSelectedProject(e.target.value)}
          className="border p-2 rounded w-64">
          <option value="">-- Select --</option>
          {projects.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </div>

      {selectedProject && (
        <>
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-semibold">Sessions</h3>
            <button onClick={() => setShowCreate(true)}
              className="bg-blue-600 text-white px-4 py-2 rounded text-sm">New Session</button>
          </div>

          {showCreate && (
            <form onSubmit={createSession} className="bg-white p-6 rounded-lg shadow mb-4">
              <div className="grid grid-cols-2 gap-4">
                <input placeholder="Tenant ID" value={form.tenant_id}
                  onChange={e => setForm({...form, tenant_id: e.target.value})} className="border p-2 rounded" required />
                <select value={form.mode} onChange={e => setForm({...form, mode: e.target.value})} className="border p-2 rounded">
                  <option value="mock">Mock</option>
                  <option value="replay">Replay</option>
                  <option value="proxy">Proxy</option>
                </select>
                <input placeholder="Expires in hours" type="number" value={form.expires_in_hours}
                  onChange={e => setForm({...form, expires_in_hours: Number(e.target.value)})} className="border p-2 rounded" />
                <input placeholder="Config JSON" value={form.config}
                  onChange={e => setForm({...form, config: e.target.value})} className="border p-2 rounded" />
              </div>
              <div className="mt-4 flex gap-2">
                <button type="submit" className="bg-green-600 text-white px-4 py-2 rounded">Create</button>
                <button type="button" onClick={() => setShowCreate(false)} className="bg-gray-300 px-4 py-2 rounded">Cancel</button>
              </div>
            </form>
          )}

          {loading ? <p>Loading...</p> : (
            <div className="space-y-4">
              {sessions.map(s => (
                <div key={s.id} className="bg-white p-4 rounded-lg shadow">
                  <div className="flex justify-between">
                    <div>
                      <span className="font-mono text-sm">{s.id}</span>
                      <span className={`ml-2 text-xs px-2 py-1 rounded ${
                        s.mode === 'mock' ? 'bg-green-100 text-green-800' :
                        s.mode === 'replay' ? 'bg-yellow-100 text-yellow-800' :
                        'bg-purple-100 text-purple-800'
                      }`}>{s.mode}</span>
                    </div>
                    <button onClick={() => deleteSession(s.id)} className="text-red-500 text-sm">Delete</button>
                  </div>
                  <p className="text-sm text-gray-500 mt-1">Tenant: {s.tenant_id} | Expires: {new Date(s.expires_at).toLocaleString()}</p>
                  <details className="mt-2">
                    <summary className="text-sm text-blue-600 cursor-pointer">cURL Example</summary>
                    <pre className="mt-2 bg-gray-50 p-3 rounded text-xs overflow-x-auto">{curlExample(s)}</pre>
                  </details>
                </div>
              ))}
              {sessions.length === 0 && <p className="text-gray-500">No sandbox sessions for this project.</p>}
            </div>
          )}
        </>
      )}
    </div>
  );
}
