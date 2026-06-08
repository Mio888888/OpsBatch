import { useEffect, useRef } from 'react';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { createTerminalOutputPump } from '../../utils/terminalOutput';
import { getCurrentTerminalAppearance, onThemeChange } from '../../stores/theme';

interface TerminalPaneProps {
  sessionId: string | null;
  hostName: string;
  hostIp: string;
  state: 'connecting' | 'connected' | 'error';
  errorMessage?: string;
  onRetry: () => void;
  onBroadcast: (data: string) => void;
}

export default function TerminalPane({ sessionId, hostName, hostIp, state, errorMessage, onRetry, onBroadcast }: TerminalPaneProps) {
  const termRef = useRef<HTMLDivElement>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const broadcastRef = useRef(onBroadcast);
  broadcastRef.current = onBroadcast;

  useEffect(() => {
    if (!termRef.current || !sessionId) return;

    let disposed = false;

    const terminalAppearance = getCurrentTerminalAppearance();
    const terminal = new Terminal({
      cursorBlink: true,
      fontSize: terminalAppearance.fontSize,
      fontFamily: terminalAppearance.fontFamily,
      scrollback: terminalAppearance.scrollback,
      theme: terminalAppearance.theme,
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();
    let webglAddon: WebglAddon | null = null;
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(webLinksAddon);
    terminal.open(termRef.current);

    try {
      webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => { webglAddon?.dispose(); webglAddon = null; });
      terminal.loadAddon(webglAddon);
    } catch { webglAddon = null; }

    fitAddon.fit();
    fitAddonRef.current = fitAddon;

    // Broadcast keyboard input to all sessions via parent callback
    const dataDisposable = terminal.onData((data) => {
      broadcastRef.current(data);
    });

    const outputPump = createTerminalOutputPump({
      write: (data) => terminal.write(data),
      closeMessage: '\r\n\x1b[31m--- 连接已关闭 ---\x1b[0m\r\n',
    });

    let unlisten: UnlistenFn | null = null;
    listen<string>(`terminal-output-${sessionId}`, (event) => {
      if (disposed) return;
      if (event.payload === '') {
        outputPump.close();
        return;
      }
      outputPump.enqueue(event.payload);
    }).then((fn) => { unlisten = fn; });

    const handleFit = () => {
      if (disposed) return;
      const el = termRef.current;
      if (!el || el.offsetWidth === 0 || el.offsetHeight === 0) return;
      fitAddon.fit();
    };

    const unsubscribeThemeChange = onThemeChange(() => {
      if (disposed) return;
      const appearance = getCurrentTerminalAppearance();
      terminal.options.theme = appearance.theme;
      terminal.options.fontSize = appearance.fontSize;
      terminal.options.fontFamily = appearance.fontFamily;
      terminal.options.scrollback = appearance.scrollback;
      if (terminal.rows > 0) {
        terminal.refresh(0, terminal.rows - 1);
      }
      handleFit();
    });

    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    const resizeObserver = new ResizeObserver(() => {
      if (resizeTimer) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(handleFit, 100);
    });
    resizeObserver.observe(termRef.current);

    const initTimer = window.setTimeout(handleFit, 300);

    return () => {
      disposed = true;
      if (resizeTimer) clearTimeout(resizeTimer);
      window.clearTimeout(initTimer);
      unsubscribeThemeChange();
      resizeObserver.disconnect();
      unlisten?.();
      dataDisposable.dispose();
      outputPump.dispose();
      webglAddon?.dispose();
      fitAddonRef.current = null;
      terminal.dispose();
    };
  }, [sessionId]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      const el = termRef.current;
      if (!fitAddonRef.current || !el || el.offsetWidth === 0 || el.offsetHeight === 0) return;
      fitAddonRef.current.fit();
    }, 100);
    return () => window.clearTimeout(timer);
  }, [state]);

  return (
    <div className="bt-pane">
      <div className="bt-pane-header">
        <span className={`bt-pane-dot bt-pane-dot-${state}`} />
        <span className="bt-pane-name">{hostName}</span>
        {hostIp && <span className="bt-pane-ip">{hostIp}</span>}
        {state === 'connecting' && <span className="bt-pane-status bt-pane-status-connecting">连接中...</span>}
        {state === 'error' && <span className="bt-pane-status bt-pane-status-error">失败</span>}
      </div>
      <div className="bt-pane-body">
        {state === 'error' && !sessionId ? (
          <div className="bt-pane-error">
            <div className="bt-pane-error-msg">{errorMessage || '连接失败'}</div>
            <button className="bt-pane-retry" onClick={onRetry}>重试</button>
          </div>
        ) : (
          <div ref={termRef} className="bt-pane-terminal" />
        )}
      </div>
    </div>
  );
}
