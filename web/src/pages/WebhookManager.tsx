import { useEffect, useState } from 'react';
import { api } from '../api/client';

const EVENT_TYPE_OPTIONS = [
  'delivery.dead',
  'delivery.failed',
  'delivery.delivered',
  'route.created',
  'route.updated',
  'project.created',
  'project.deleted',
];

export function WebhookManager() {
  const [subscriptions, setSubscriptions] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [url, setUrl] = useState('');
  const [description, setDescription] = useState('');
  const [selectedEvents, setSelectedEvents] = useState<Set<string>>(new Set());
  const [creating, setCreating] = useState(false);

  useEffect(() => { loadSubscriptions(); }, []);

  async function loadSubscriptions() {
    setLoading(true);
    try { setSubscriptions(await api.listWebhooks()); }
    catch (e) { console.error(e); }
    setLoading(false);
  }

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!url) return;
    setCreating(true);
    try {
      await api.createWebhook({
        url,
        event_types: Array.from(selectedEvents),
        description,
      });
      setUrl('');
      setDescription('');
      setSelectedEvents(new Set());
      setShowForm(false);
      loadSubscriptions();
    } catch (err: any) {
      alert(err.message);
    }
    setCreating(false);
  }

  async function handleDelete(id: string) {
    if (!confirm('Delete this webhook subscription?')) return;
    try {
      await api.deleteWebhook(id);
      loadSubscriptions();
    } catch (err: any) {
      alert(err.message);
    }
  }

  function toggleEvent(evt: string) {
    const next = new Set(selectedEvents);
    if (next.has(evt)) next.delete(evt); else next.add(evt);
    setSelectedEvents(next);
  }

  return (
    <div>
      <div className="flex justify-between items-center mb-6">
        <h2 className="text-2xl font-bold">Webhook Subscriptions</h2>
        <div className="flex gap-2">
          <button onClick={loadSubscriptions} className="bg-gray-200 px-4 py-2 rounded text-sm">Refresh</button>
          <button onClick={() => setShowForm(!showForm)} className="bg-blue-600 text-white px-4 py-2 rounded text-sm">
            {showForm ? 'Cancel' : 'New Subscription'}
          </button>
        </div>
      </div>

      {showForm && (
        <form onSubmit={handleCreate} className="bg-white rounded-lg shadow p-6 mb-6">
          <h3 className="font-semibold text-lg mb-4">Create Webhook Subscription</h3>
          <div className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">Webhook URL</label>
              <input
                type="url"
                value={url}
                onChange={e => setUrl(e.target.value)}
                placeholder="https://example.com/webhook"
                className="w-full border rounded px-3 py-2 text-sm"
                required
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">Description</label>
              <input
                type="text"
                value={description}
                onChange={e => setDescription(e.target.value)}
                placeholder="e.g., Slack notification for dead letters"
                className="w-full border rounded px-3 py-2 text-sm"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-2">Event Types (empty = all events)</label>
              <div className="flex flex-wrap gap-2">
                {EVENT_TYPE_OPTIONS.map(evt => (
                  <label key={evt} className="inline-flex items-center gap-1 text-sm bg-gray-100 px-3 py-1 rounded cursor-pointer">
                    <input
                      type="checkbox"
                      checked={selectedEvents.has(evt)}
                      onChange={() => toggleEvent(evt)}
                    />
                    {evt}
                  </label>
                ))}
              </div>
            </div>
            <button
              type="submit"
              disabled={creating || !url}
              className="bg-blue-600 text-white px-6 py-2 rounded text-sm disabled:opacity-50"
            >
              {creating ? 'Creating...' : 'Create'}
            </button>
          </div>
        </form>
      )}

      {loading ? <p>Loading...</p> : (
        <div className="bg-white rounded-lg shadow overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-gray-50">
              <tr>
                <th className="p-3 text-left">URL</th>
                <th className="p-3 text-left">Event Types</th>
                <th className="p-3 text-left">Description</th>
                <th className="p-3 text-left">Active</th>
                <th className="p-3 text-left">Created</th>
                <th className="p-3 text-left">Actions</th>
              </tr>
            </thead>
            <tbody>
              {subscriptions.map(sub => (
                <tr key={sub.id} className="border-t hover:bg-gray-50">
                  <td className="p-3 font-mono text-xs max-w-xs truncate">{sub.url}</td>
                  <td className="p-3">
                    <div className="flex flex-wrap gap-1">
                      {(Array.isArray(sub.event_types) && sub.event_types.length > 0)
                        ? sub.event_types.map((et: string) => (
                            <span key={et} className="bg-blue-100 text-blue-800 text-xs px-2 py-0.5 rounded">{et}</span>
                          ))
                        : <span className="text-gray-400 text-xs">All events</span>
                      }
                    </div>
                  </td>
                  <td className="p-3 text-gray-600">{sub.description || '-'}</td>
                  <td className="p-3">
                    <span className={`text-xs px-2 py-1 rounded ${sub.active ? 'bg-green-100 text-green-800' : 'bg-gray-100 text-gray-600'}`}>
                      {sub.active ? 'Active' : 'Inactive'}
                    </span>
                  </td>
                  <td className="p-3 text-xs">{sub.created_at ? new Date(sub.created_at).toLocaleString() : '-'}</td>
                  <td className="p-3">
                    <button
                      onClick={() => handleDelete(sub.id)}
                      className="text-red-600 text-xs hover:underline"
                    >
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
              {subscriptions.length === 0 && (
                <tr><td colSpan={6} className="p-8 text-center text-gray-500">No webhook subscriptions configured.</td></tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
