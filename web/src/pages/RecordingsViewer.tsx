import { useEffect, useState } from 'react';
import { api } from '../api/client';

interface Recording {
  id: string;
  session_id: string;
  route_id: string;
  request: unknown;
  response: unknown;
  duration_ms: number;
  recorded_at: string;
}

interface RecordingsViewerProps {
  sessionId: string;
  onClose: () => void;
}

/**
 * 独立的录制数据浏览组件，展示指定沙箱会话的所有请求/响应录制记录。
 * 以独立文件存在，避免与其他代理对 SandboxManager.tsx 的并行修改冲突。
 */
export function RecordingsViewer({ sessionId, onClose }: RecordingsViewerProps) {
  const [recordings, setRecordings] = useState<Recording[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => {
    loadRecordings();
  }, [sessionId]);

  async function loadRecordings() {
    setLoading(true);
    try {
      const data = await api.listRecordings(sessionId);
      setRecordings(data);
    } catch (e) {
      console.error('Failed to load recordings:', e);
    }
    setLoading(false);
  }

  async function clearRecordings() {
    if (!confirm('Clear all recordings for this session?')) return;
    try {
      await api.clearRecordings(sessionId);
      setRecordings([]);
    } catch (e) {
      console.error('Failed to clear recordings:', e);
    }
  }

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg shadow-xl w-full max-w-4xl max-h-[80vh] overflow-hidden flex flex-col">
        <div className="flex justify-between items-center p-4 border-b">
          <h3 className="text-lg font-semibold">
            Recordings
            <span className="text-sm text-gray-500 ml-2">Session: {sessionId.slice(0, 8)}...</span>
          </h3>
          <div className="flex gap-2">
            {recordings.length > 0 && (
              <button
                onClick={clearRecordings}
                className="text-sm bg-red-50 text-red-600 px-3 py-1 rounded hover:bg-red-100 transition"
              >
                Clear All
              </button>
            )}
            <button onClick={onClose} className="text-sm bg-gray-200 px-3 py-1 rounded hover:bg-gray-300 transition">
              Close
            </button>
          </div>
        </div>

        <div className="overflow-y-auto flex-1 p-4">
          {loading ? (
            <p className="text-gray-500">Loading recordings...</p>
          ) : recordings.length === 0 ? (
            <p className="text-gray-500">No recordings found for this session.</p>
          ) : (
            <div className="space-y-3">
              {recordings.map((rec) => (
                <div key={rec.id} className="border rounded-lg">
                  <button
                    onClick={() => setExpandedId(expandedId === rec.id ? null : rec.id)}
                    className="w-full p-3 flex justify-between items-center hover:bg-gray-50 transition text-left"
                  >
                    <div>
                      <span className="font-mono text-sm">{rec.route_id.slice(0, 8)}...</span>
                      <span className="ml-2 text-xs text-gray-500">
                        {new Date(rec.recorded_at).toLocaleString()}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-xs bg-blue-100 text-blue-800 px-2 py-1 rounded">
                        {rec.duration_ms}ms
                      </span>
                      <span className="text-gray-400">{expandedId === rec.id ? '\u25B2' : '\u25BC'}</span>
                    </div>
                  </button>
                  {expandedId === rec.id && (
                    <div className="p-3 border-t bg-gray-50">
                      <div className="grid grid-cols-2 gap-4">
                        <div>
                          <h4 className="text-sm font-semibold mb-1 text-gray-700">Request</h4>
                          <pre className="bg-white p-2 rounded border text-xs overflow-x-auto max-h-60 overflow-y-auto">
                            {JSON.stringify(rec.request, null, 2)}
                          </pre>
                        </div>
                        <div>
                          <h4 className="text-sm font-semibold mb-1 text-gray-700">Response</h4>
                          <pre className="bg-white p-2 rounded border text-xs overflow-x-auto max-h-60 overflow-y-auto">
                            {JSON.stringify(rec.response, null, 2)}
                          </pre>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
