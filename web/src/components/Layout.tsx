import { Link, Outlet, useLocation } from 'react-router-dom';

const navItems = [
  { path: '/', label: 'Dashboard', icon: '📊' },
  { path: '/docs', label: 'API Docs', icon: '📖' },
  { path: '/sandbox', label: 'Sandbox', icon: '🧪' },
  { path: '/compensation', label: 'Compensation', icon: '🔄' },
];

export function Layout() {
  const location = useLocation();
  return (
    <div className="flex h-screen bg-gray-100">
      <aside className="w-64 bg-gray-900 text-white">
        <div className="p-6">
          <h1 className="text-xl font-bold">API-Anything</h1>
          <p className="text-gray-400 text-sm mt-1">Gateway Platform</p>
        </div>
        <nav className="mt-4">
          {navItems.map(item => (
            <Link key={item.path} to={item.path}
              className={`flex items-center px-6 py-3 text-sm ${
                location.pathname === item.path ? 'bg-gray-800 text-white' : 'text-gray-300 hover:bg-gray-800'
              }`}>
              <span className="mr-3">{item.icon}</span>
              {item.label}
            </Link>
          ))}
        </nav>
      </aside>
      <main className="flex-1 overflow-auto p-8">
        <Outlet />
      </main>
    </div>
  );
}
