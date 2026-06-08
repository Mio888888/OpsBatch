import { lazy, Suspense, useEffect } from 'react';
import { BrowserRouter, Routes, Route, useLocation, useNavigate } from 'react-router-dom';
import { ToastProvider } from './components/ui';
import { useTranslation } from './i18n';
import MainLayout from './components/MainLayout';
import './App.css';

const TerminalPage = lazy(() => import('./pages/Terminal/TerminalPage'));

const EditorWindow = lazy(() => import('./pages/Editor/EditorWindow'));
const BatchTerminalWindow = lazy(() => import('./pages/Commands/BatchTerminalWindow'));
const BatchTransferWindow = lazy(() => import('./pages/Transfer/BatchTransferWindow'));
const CommandsPage = lazy(() => import('./pages/Commands/CommandsPage'));
const CommandLibPage = lazy(() => import('./pages/CommandLib/CommandLibPage'));
const ScriptLibPage = lazy(() => import('./pages/ScriptLib/ScriptLibPage'));
const QuickActionsPage = lazy(() => import('./pages/QuickActions/QuickActionsPage'));
const WorkflowPage = lazy(() => import('./pages/Workflow/WorkflowPage'));
const GitHubPage = lazy(() => import('./pages/GitHub/GitHubPage'));
const SettingsPage = lazy(() => import('./pages/Settings/SettingsPage'));
const GlobalLogPage = lazy(() => import('./pages/Log/GlobalLogPage'));

function EditorRoute() {
  return (
    <div className="editor-route-root">
      <EditorWindow />
    </div>
  );
}

function BatchTerminalRoute() {
  return (
    <div className="batch-route-root">
      <BatchTerminalWindow />
    </div>
  );
}

function BatchTransferRoute() {
  return (
    <div className="batch-route-root">
      <BatchTransferWindow />
    </div>
  );
}

function GlobalLogRoute() {
  return (
    <div className="global-log-route-root">
      <GlobalLogPage />
    </div>
  );
}

function RouteLoading() {
  const { t } = useTranslation();
  return <div className="route-loading">{t('app.loading')}</div>;
}

function AppContent() {
  const location = useLocation();
  const navigate = useNavigate();

  const isEditor = location.pathname === '/editor';
  const isBatchTerminal = location.pathname === '/batch-terminal';
  const isBatchTransfer = location.pathname === '/batch-transfer';
  const isSettings = location.pathname === '/settings';
  const isGlobalLog = location.pathname === '/global-log';
  const isStandalone = isEditor || isBatchTerminal || isBatchTransfer || isSettings || isGlobalLog;
  const isTerminal = !isStandalone && (
    location.pathname === '/terminal'
    || location.pathname === '/assets'
    || location.pathname === '/'
  );

  useEffect(() => {
    if (location.pathname === '/') {
      navigate('/terminal', { replace: true });
    } else if (location.pathname === '/assets') {
      navigate('/terminal?assets=1', { replace: true });
    }
  }, [location.pathname, navigate]);

  if (isStandalone) {
    return (
      <Suspense fallback={<RouteLoading />}>
        <Routes>
          <Route path="/editor" element={<EditorRoute />} />
          <Route path="/batch-terminal" element={<BatchTerminalRoute />} />
          <Route path="/batch-transfer" element={<BatchTransferRoute />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="/global-log" element={<GlobalLogRoute />} />
        </Routes>
      </Suspense>
    );
  }

  return (
    <MainLayout>
      <Suspense fallback={<RouteLoading />}>
        <div style={{ display: isTerminal ? 'contents' : 'none' }}>
          <TerminalPage visible={isTerminal} />
        </div>
      </Suspense>
      {!isTerminal && (
        <Suspense fallback={<RouteLoading />}>
          <Routes>
            <Route path="/commands" element={<CommandsPage />} />
            <Route path="/command-lib" element={<CommandLibPage />} />
            <Route path="/script-lib" element={<ScriptLibPage />} />
            <Route path="/quick-actions" element={<QuickActionsPage />} />
            <Route path="/workflow" element={<WorkflowPage />} />
            <Route path="/github" element={<GitHubPage />} />
          </Routes>
        </Suspense>
      )}
    </MainLayout>
  );
}

function App() {
  return (
    <ToastProvider>
      <BrowserRouter>
        <AppContent />
      </BrowserRouter>
    </ToastProvider>
  );
}

// Preload all lazy pages during idle time so navigation is instant.
const prefetchPages = () => {
  void import('./pages/Terminal/TerminalPage');
  void import('./pages/Editor/EditorWindow');
  void import('./pages/Commands/BatchTerminalWindow');
  void import('./pages/Transfer/BatchTransferWindow');
  void import('./pages/Commands/CommandsPage');
  void import('./pages/CommandLib/CommandLibPage');
  void import('./pages/ScriptLib/ScriptLibPage');
  void import('./pages/QuickActions/QuickActionsPage');
  void import('./pages/Workflow/WorkflowPage');
  void import('./pages/GitHub/GitHubPage');
  void import('./pages/Settings/SettingsPage');
  void import('./pages/Log/GlobalLogPage');
};

if (typeof window !== 'undefined') {
  if (typeof window.requestIdleCallback === 'function') {
    window.requestIdleCallback(prefetchPages);
  } else {
    setTimeout(prefetchPages, 1000);
  }
}

export default App;
