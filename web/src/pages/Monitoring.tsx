import { useState } from 'react';

// Grafana 地址可通过环境变量覆盖，默认指向本地开发实例
const GRAFANA_URL = import.meta.env.VITE_GRAFANA_URL || 'http://localhost:3000';

const dashboardLinks = [
  {
    href: `${GRAFANA_URL}/d/gateway-overview`,
    title: 'Gateway Overview',
    description: 'QPS, Latency, Error Rate, Circuit Breaker',
  },
  {
    href: `${GRAFANA_URL}/explore`,
    title: 'Explore Traces',
    description: 'Distributed tracing with Tempo',
  },
];

export function Monitoring() {
  const [iframeKey, setIframeKey] = useState(0);

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <h2 className="text-2xl font-bold">Monitoring</h2>
        <button
          onClick={() => setIframeKey((k) => k + 1)}
          className="text-sm bg-gray-200 px-3 py-1 rounded hover:bg-gray-300 transition"
        >
          Refresh Dashboard
        </button>
      </div>

      <div className="grid grid-cols-2 gap-4 mb-4">
        {dashboardLinks.map((link) => (
          <a
            key={link.href}
            href={link.href}
            target="_blank"
            rel="noopener noreferrer"
            className="bg-white p-4 rounded-lg shadow hover:shadow-md transition"
          >
            <h3 className="font-semibold">{link.title}</h3>
            <p className="text-sm text-gray-500">{link.description}</p>
          </a>
        ))}
      </div>

      <iframe
        key={iframeKey}
        src={`${GRAFANA_URL}/d/gateway-overview?orgId=1&kiosk`}
        className="w-full h-[calc(100vh-250px)] rounded-lg border"
        title="Grafana Dashboard"
      />
    </div>
  );
}
