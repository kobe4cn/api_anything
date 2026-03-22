import { useEffect, useState } from 'react';
import { api } from '../api/client';

export function CompensationManager() {
  const [deadLetters, setDeadLetters] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => { loadDeadLetters(); }, []);

  async function loadDeadLetters() {
    setLoading(true);
    try { setDeadLetters(await api.listDeadLetters('limit=50')); }
    catch (e) { console.error(e); }
    setLoading(false);
  }

  async function retryOne(id: string) {
    try {
      await api.retryDeadLetter(id);
      loadDeadLetters();
    } catch (e: any) { alert(e.message); }
  }

  async function resolveOne(id: string) {
    try {
      await api.resolveDeadLetter(id);
      loadDeadLetters();
    } catch (e: any) { alert(e.message); }
  }

  async function batchRetry() {
    if (selected.size === 0) return;
    try {
      const result = await api.batchRetry(Array.from(selected));
      alert(`Retried ${result.retried} items`);
      setSelected(new Set());
      loadDeadLetters();
    } catch (e: any) { alert(e.message); }
  }

  function toggleSelect(id: string) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id); else next.add(id);
    setSelected(next);
  }

  return (
    <div>
      <div className="flex justify-between items-center mb-6">
        <h2 className="text-2xl font-bold">Dead Letter Queue</h2>
        <div className="flex gap-2">
          <button onClick={loadDeadLetters} className="bg-gray-200 px-4 py-2 rounded text-sm">Refresh</button>
          {selected.size > 0 && (
            <button onClick={batchRetry} className="bg-orange-500 text-white px-4 py-2 rounded text-sm">
              Retry Selected ({selected.size})
            </button>
          )}
        </div>
      </div>

      {loading ? <p>Loading...</p> : (
        <div className="bg-white rounded-lg shadow overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-gray-50">
              <tr>
                <th className="p-3 text-left w-8"><input type="checkbox" onChange={e => {
                  if (e.target.checked) setSelected(new Set(deadLetters.map(d => d.id)));
                  else setSelected(new Set());
                }} /></th>
                <th className="p-3 text-left">ID</th>
                <th className="p-3 text-left">Route</th>
                <th className="p-3 text-left">Status</th>
                <th className="p-3 text-left">Retries</th>
                <th className="p-3 text-left">Error</th>
                <th className="p-3 text-left">Updated</th>
                <th className="p-3 text-left">Actions</th>
              </tr>
            </thead>
            <tbody>
              {deadLetters.map(d => (
                <tr key={d.id} className="border-t hover:bg-gray-50">
                  <td className="p-3"><input type="checkbox" checked={selected.has(d.id)} onChange={() => toggleSelect(d.id)} /></td>
                  <td className="p-3 font-mono text-xs cursor-pointer text-blue-600"
                    onClick={() => setExpandedId(expandedId === d.id ? null : d.id)}>
                    {d.id.slice(0, 8)}...
                  </td>
                  <td className="p-3 font-mono text-xs">{d.route_id?.slice(0, 8)}...</td>
                  <td className="p-3"><span className="bg-red-100 text-red-800 text-xs px-2 py-1 rounded">{d.status}</span></td>
                  <td className="p-3">{d.retry_count}</td>
                  <td className="p-3 text-xs text-gray-500 max-w-xs truncate">{d.error_message || '-'}</td>
                  <td className="p-3 text-xs">{d.updated_at ? new Date(d.updated_at).toLocaleString() : '-'}</td>
                  <td className="p-3 flex gap-2">
                    <button onClick={() => retryOne(d.id)} className="text-orange-600 text-xs hover:underline">Retry</button>
                    <button onClick={() => resolveOne(d.id)} className="text-green-600 text-xs hover:underline">Resolve</button>
                  </td>
                </tr>
              ))}
              {deadLetters.length === 0 && (
                <tr><td colSpan={8} className="p-8 text-center text-gray-500">No dead letters. All clear!</td></tr>
              )}
            </tbody>
          </table>

          {expandedId && (
            <div className="p-4 bg-gray-50 border-t">
              <h4 className="font-semibold mb-2">Payload</h4>
              <pre className="text-xs bg-white p-3 rounded border overflow-auto max-h-48">
                {JSON.stringify(deadLetters.find(d => d.id === expandedId)?.request_payload, null, 2)}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
