import { useState } from 'react';
import { api } from '../api/client';

export function ApiDocs() {
  const [activeTab, setActiveTab] = useState<'swagger' | 'prompt'>('swagger');
  const [prompt, setPrompt] = useState('');
  const [loadingPrompt, setLoadingPrompt] = useState(false);

  async function loadPrompt() {
    setLoadingPrompt(true);
    try {
      const text = await api.getAgentPrompt();
      setPrompt(text);
    } catch (e) { console.error(e); }
    setLoadingPrompt(false);
  }

  return (
    <div>
      <h2 className="text-2xl font-bold mb-4">API Documentation</h2>

      <div className="flex gap-2 mb-4">
        <button onClick={() => setActiveTab('swagger')}
          className={`px-4 py-2 rounded ${activeTab === 'swagger' ? 'bg-blue-600 text-white' : 'bg-gray-200'}`}>
          Swagger UI
        </button>
        <button onClick={() => { setActiveTab('prompt'); if (!prompt) loadPrompt(); }}
          className={`px-4 py-2 rounded ${activeTab === 'prompt' ? 'bg-blue-600 text-white' : 'bg-gray-200'}`}>
          Agent Prompt
        </button>
        <a href="/api/v1/docs/openapi.json" download
          className="px-4 py-2 rounded bg-gray-200 hover:bg-gray-300 ml-auto">
          Download OpenAPI JSON
        </a>
      </div>

      {activeTab === 'swagger' && (
        <iframe src="/api/v1/docs" className="w-full h-[calc(100vh-200px)] rounded-lg border" />
      )}

      {activeTab === 'prompt' && (
        <div className="bg-white p-6 rounded-lg shadow">
          {loadingPrompt ? <p>Loading...</p> : (
            <pre className="whitespace-pre-wrap text-sm font-mono bg-gray-50 p-4 rounded overflow-auto max-h-[calc(100vh-300px)]">
              {prompt || 'No routes configured yet.'}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
