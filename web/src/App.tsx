import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Layout } from './components/Layout';
import { Dashboard } from './pages/Dashboard';
import { ApiDocs } from './pages/ApiDocs';
import { SandboxManager } from './pages/SandboxManager';
import { CompensationManager } from './pages/CompensationManager';

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<Layout />}>
          <Route path="/" element={<Dashboard />} />
          <Route path="/docs" element={<ApiDocs />} />
          <Route path="/sandbox" element={<SandboxManager />} />
          <Route path="/compensation" element={<CompensationManager />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
