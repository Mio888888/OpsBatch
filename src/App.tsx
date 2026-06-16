import { lazy, Suspense, useEffect, useState } from 'react';
import { HashRouter, Routes, Route, useLocation, useNavigate } from 'react-router-dom';
import { ToastProvider } from './components/ui';
import { useTranslation } from './i18n';
import KeychainNoticeModal from './components/KeychainNoticeModal';
import MainLayout from './components/MainLayout';
import WindowControls from './components/WindowControls';
import './App.css';

function applyInitialRouteToHash() {
  if (typeof window === 'undefined') return;

  const route = new URLSearchParams(window.location.search).get('route');
  if (!route || !route.startsWith('/') || route.startsWith('//')) return;

  window.history.replaceState(null, '', window.location.pathname);
  window.location.hash = route;
}

applyInitialRouteToHash();

const TerminalPage = lazy(() => import('./pages/Terminal/TerminalPage'));
const RdpPage = lazy(() => import('./pages/Rdp/RdpPage'));
const VncPage = lazy(() => import('./pages/Vnc/VncPage'));

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

function RdpRoute() {
  return (
    <div className="rdp-route-root">
      <RdpPage />
    </div>
  );
}

function VncRoute() {
  return (
    <div className="rdp-route-root">
      <VncPage />
    </div>
  );
}

function SettingsRoute() {
  return (
    <SettingsWindowFrame>
      <SettingsPage />
    </SettingsWindowFrame>
  );
}

function SettingsWindowFrame({ children }: { children: React.ReactNode }) {
  return (
    <div className="settings-window-frame">
      <div
        className="settings-window-drag-strip"
        aria-hidden="true"
        onMouseDown={(e) => {
          if (e.button !== 0) return;
          void import('@tauri-apps/api/window')
            .then(({ getCurrentWindow }) => getCurrentWindow().startDragging())
            .catch((error) => console.error('Window command failed:', error));
        }}
      />
      <div className="settings-window-control-layer">
        <WindowControls />
      </div>
      {children}
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
  const isRdp = location.pathname === '/rdp';
  const isVnc = location.pathname === '/vnc';
  const isStandalone = isEditor || isBatchTerminal || isBatchTransfer || isSettings || isGlobalLog || isRdp || isVnc;
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
          <Route path="/settings" element={<SettingsRoute />} />
          <Route path="/global-log" element={<GlobalLogRoute />} />
          <Route path="/rdp" element={<RdpRoute />} />
          <Route path="/vnc" element={<VncRoute />} />
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
  const [vaultUnlocked, setVaultUnlocked] = useState(false);

  return (
    <ToastProvider>
      <HashRouter>
        {vaultUnlocked ? <AppContent /> : <RouteLoading />}
        <KeychainNoticeModal onUnlocked={() => setVaultUnlocked(true)} />
      </HashRouter>
    </ToastProvider>
  );
}

// 分批低优先级预加载：仅预加载主窗口内可能用到的页面，
// 不预加载独立窗口页面（editor/rdp/vnc/batch-terminal/batch-transfer/settings/global-log），
// 这些由 Tauri 独立窗口入口触发 lazy 加载。
type PrefetchTask = () => Promise<unknown>;

// 第一批：主窗口 tab 页（用户最可能切换到）
const batch1: PrefetchTask[] = [
  () => import('./pages/Workflow/WorkflowPage'),
  () => import('./pages/GitHub/GitHubPage'),
];

// 第二批：主窗口内其他子页面
const batch2: PrefetchTask[] = [
  () => import('./pages/Commands/CommandsPage'),
  () => import('./pages/CommandLib/CommandLibPage'),
  () => import('./pages/ScriptLib/ScriptLibPage'),
  () => import('./pages/QuickActions/QuickActionsPage'),
];

const prefetchBatches = [batch1, batch2];

function runPrefetchBatch(batch: PrefetchTask[], delay: number) {
  setTimeout(() => {
    for (const task of batch) {
      void task();
    }
  }, delay);
}

if (typeof window !== 'undefined') {
  // 每批间隔 2 秒，让首屏资源优先加载
  prefetchBatches.forEach((batch, i) => {
    runPrefetchBatch(batch, 3000 + i * 2000);
  });
}

export default App;
