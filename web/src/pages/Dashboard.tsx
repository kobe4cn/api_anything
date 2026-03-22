import { useEffect, useState } from 'react';
import { api } from '../api/client';

export function Dashboard() {
  const [projects, setProjects] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);
  const [showCreate, setShowCreate] = useState(false);
  const [form, setForm] = useState({ name: '', description: '', owner: '', source_type: 'wsdl' });

  useEffect(() => { loadProjects(); }, []);

  async function loadProjects() {
    setLoading(true);
    try { setProjects(await api.listProjects()); }
    catch (e) { console.error(e); }
    setLoading(false);
  }

  async function createProject(e: React.FormEvent) {
    e.preventDefault();
    await api.createProject(form);
    setShowCreate(false);
    setForm({ name: '', description: '', owner: '', source_type: 'wsdl' });
    loadProjects();
  }

  async function deleteProject(id: string) {
    // confirm避免误删，因为删除操作不可逆
    if (!confirm('Delete this project?')) return;
    await api.deleteProject(id);
    loadProjects();
  }

  return (
    <div>
      <div className="flex justify-between items-center mb-6">
        <h2 className="text-2xl font-bold">Projects</h2>
        <button onClick={() => setShowCreate(true)}
          className="bg-blue-600 text-white px-4 py-2 rounded hover:bg-blue-700">
          New Project
        </button>
      </div>

      {showCreate && (
        <form onSubmit={createProject} className="bg-white p-6 rounded-lg shadow mb-6">
          <div className="grid grid-cols-2 gap-4">
            <input placeholder="Name" value={form.name} onChange={e => setForm({...form, name: e.target.value})}
              className="border p-2 rounded" required />
            <input placeholder="Owner" value={form.owner} onChange={e => setForm({...form, owner: e.target.value})}
              className="border p-2 rounded" required />
            <input placeholder="Description" value={form.description} onChange={e => setForm({...form, description: e.target.value})}
              className="border p-2 rounded col-span-2" />
            <select value={form.source_type} onChange={e => setForm({...form, source_type: e.target.value})}
              className="border p-2 rounded">
              <option value="wsdl">WSDL</option>
              <option value="cli">CLI</option>
              <option value="ssh">SSH</option>
              <option value="pty">PTY</option>
            </select>
          </div>
          <div className="mt-4 flex gap-2">
            <button type="submit" className="bg-green-600 text-white px-4 py-2 rounded">Create</button>
            <button type="button" onClick={() => setShowCreate(false)} className="bg-gray-300 px-4 py-2 rounded">Cancel</button>
          </div>
        </form>
      )}

      {loading ? <p>Loading...</p> : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {projects.map(p => (
            <div key={p.id} className="bg-white p-6 rounded-lg shadow">
              <div className="flex justify-between">
                <h3 className="font-semibold text-lg">{p.name}</h3>
                <span className="text-xs bg-blue-100 text-blue-800 px-2 py-1 rounded">{p.source_type}</span>
              </div>
              <p className="text-gray-500 text-sm mt-2">{p.description || 'No description'}</p>
              <div className="mt-4 flex justify-between items-center">
                <span className="text-xs text-gray-400">Owner: {p.owner}</span>
                <button onClick={() => deleteProject(p.id)}
                  className="text-red-500 text-sm hover:underline">Delete</button>
              </div>
            </div>
          ))}
          {projects.length === 0 && <p className="text-gray-500">No projects yet. Create one to get started.</p>}
        </div>
      )}
    </div>
  );
}
