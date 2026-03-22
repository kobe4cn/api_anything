import { useState } from 'react';
import { api } from '../api/client';

const SDK_LANGUAGES = [
  { value: 'typescript', label: 'TypeScript', ext: 'ts' },
  { value: 'python', label: 'Python', ext: 'py' },
  { value: 'java', label: 'Java', ext: 'java' },
  { value: 'go', label: 'Go', ext: 'go' },
];

export function ApiDocs() {
  const [activeTab, setActiveTab] = useState<'swagger' | 'prompt' | 'sdk'>('swagger');
  const [prompt, setPrompt] = useState('');
  const [loadingPrompt, setLoadingPrompt] = useState(false);
  const [sdkLanguage, setSdkLanguage] = useState('typescript');
  const [sdkCode, setSdkCode] = useState('');
  const [loadingSdk, setLoadingSdk] = useState(false);

  async function loadPrompt() {
    setLoadingPrompt(true);
    try {
      const text = await api.getAgentPrompt();
      setPrompt(text);
    } catch (e) { console.error(e); }
    setLoadingPrompt(false);
  }

  async function loadSdk(lang?: string) {
    const language = lang || sdkLanguage;
    setLoadingSdk(true);
    try {
      const code = await api.getSdkCode(language);
      setSdkCode(code);
    } catch (e) { console.error(e); }
    setLoadingSdk(false);
  }

  function downloadSdk() {
    const langInfo = SDK_LANGUAGES.find(l => l.value === sdkLanguage);
    const blob = new Blob([sdkCode], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `api_anything_sdk.${langInfo?.ext || 'txt'}`;
    a.click();
    URL.revokeObjectURL(url);
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
        <button onClick={() => { setActiveTab('sdk'); if (!sdkCode) loadSdk(); }}
          className={`px-4 py-2 rounded ${activeTab === 'sdk' ? 'bg-blue-600 text-white' : 'bg-gray-200'}`}>
          SDK Generator
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

      {activeTab === 'sdk' && (
        <div className="bg-white p-6 rounded-lg shadow">
          <div className="flex items-center gap-3 mb-4">
            <label className="text-sm font-medium text-gray-700">Language:</label>
            <select
              value={sdkLanguage}
              onChange={e => { setSdkLanguage(e.target.value); loadSdk(e.target.value); }}
              className="border rounded px-3 py-1.5 text-sm"
            >
              {SDK_LANGUAGES.map(l => (
                <option key={l.value} value={l.value}>{l.label}</option>
              ))}
            </select>
            <button
              onClick={() => loadSdk()}
              className="px-3 py-1.5 bg-blue-600 text-white rounded text-sm hover:bg-blue-700"
            >
              Generate
            </button>
            <button
              onClick={downloadSdk}
              disabled={!sdkCode}
              className="px-3 py-1.5 bg-green-600 text-white rounded text-sm hover:bg-green-700 disabled:bg-gray-400"
            >
              Download
            </button>
          </div>
          {loadingSdk ? <p>Generating SDK...</p> : (
            <pre className="whitespace-pre-wrap text-sm font-mono bg-gray-900 text-green-400 p-4 rounded overflow-auto max-h-[calc(100vh-350px)]">
              {sdkCode || 'Click "Generate" to create SDK code.'}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
