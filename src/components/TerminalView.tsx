import { useEffect, useRef, memo, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { SerializeAddon } from '@xterm/addon-serialize';
import { createTerminalOutputPump, type TerminalOutputPump } from '../utils/terminalOutput';
import { createTrackedCommand, createTrackedCommandOutputCleaner, stripTrackedCommandOutputArtifacts } from '../utils/terminalTracker';
import { getCurrentTerminalAppearance, onThemeChange } from '../stores/theme';

export interface TerminalCommandExecutionOptions {
  timeoutMs?: number;
}

export interface TerminalCommandExecutionResult {
  executionId: string;
  status: 'completed' | 'closed' | 'timeout' | 'write_failed';
  exitCode?: number;
  output: string;
}

export interface TerminalController {
  getBuffer: () => string;
  getSelection: () => string;
  pasteText: (text: string) => Promise<void>;
  insertCommand: (command: string) => Promise<void>;
  executeCommand: (command: string, options?: TerminalCommandExecutionOptions) => Promise<TerminalCommandExecutionResult>;
}

interface TerminalViewProps {
  sessionId: string;
  active?: boolean;
  onTerminalReady?: (terminal: TerminalController) => void;
}

interface PendingTerminalExecution {
  executionId: string;
  donePattern: RegExp;
  output: string;
  timeoutId: number;
  resolve: (result: TerminalCommandExecutionResult) => void;
}

const DEFAULT_COMMAND_TIMEOUT_MS = 30 * 60 * 1000;
const MAX_TRACKED_COMMAND_OUTPUT_CHARS = 120_000;
const MAX_DONE_DETECTOR_CHARS = 4096;

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function limitTrackedOutput(output: string): string {
  if (output.length <= MAX_TRACKED_COMMAND_OUTPUT_CHARS) {
    return output;
  }
  return output.slice(output.length - MAX_TRACKED_COMMAND_OUTPUT_CHARS);
}

function cleanExecutionOutput(output: string): string {
  return stripTrackedCommandOutputArtifacts(output).trim();
}

export default memo(function TerminalView({ sessionId, active = true, onTerminalReady }: TerminalViewProps) {
  const termRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const outputPumpRef = useRef<TerminalOutputPump | null>(null);
  const writeQueueRef = useRef('');
  const writeFlushTimerRef = useRef<number | null>(null);
  const latestActiveRef = useRef(active);
  const pendingExecutionsRef = useRef<Map<string, PendingTerminalExecution>>(new Map());
  const commandExecutionChainRef = useRef<Promise<void>>(Promise.resolve());
  const doneDetectorBufferRef = useRef('');
  const dangerRulesRef = useRef<{ name: string; pattern: string }[]>([]);
  const pendingDangerConfirmRef = useRef<string | null>(null);

  useEffect(() => {
    latestActiveRef.current = active;
    outputPumpRef.current?.setActive(active);
  }, [active]);

  const settleExecution = useCallback((executionId: string, status: TerminalCommandExecutionResult['status'], exitCode?: number) => {
    const pending = pendingExecutionsRef.current.get(executionId);
    if (!pending) {
      return;
    }

    pendingExecutionsRef.current.delete(executionId);
    window.clearTimeout(pending.timeoutId);
    pending.resolve({
      executionId,
      status,
      exitCode,
      output: cleanExecutionOutput(pending.output),
    });
  }, []);

  const appendExecutionOutput = useCallback((data: string) => {
    if (pendingExecutionsRef.current.size === 0 || data.length === 0) {
      return;
    }

    pendingExecutionsRef.current.forEach((pending) => {
      pending.output = limitTrackedOutput(pending.output + data);
    });
  }, []);

  const detectCompletedExecutions = useCallback((data: string) => {
    if (pendingExecutionsRef.current.size === 0 || data.length === 0) {
      return;
    }

    doneDetectorBufferRef.current = (doneDetectorBufferRef.current + data).slice(-MAX_DONE_DETECTOR_CHARS);
    pendingExecutionsRef.current.forEach((pending) => {
      const match = doneDetectorBufferRef.current.match(pending.donePattern);
      if (!match) {
        return;
      }
      const exitCode = Number(match[1]);
      settleExecution(pending.executionId, 'completed', Number.isFinite(exitCode) ? exitCode : undefined);
    });
  }, [settleExecution]);

  const failAllPendingExecutions = useCallback((status: TerminalCommandExecutionResult['status']) => {
    const ids = Array.from(pendingExecutionsRef.current.keys());
    ids.forEach((executionId) => settleExecution(executionId, status));
  }, [settleExecution]);

  const flushWriteQueue = useCallback((targetSessionId: string) => {
    if (writeFlushTimerRef.current !== null) {
      window.clearTimeout(writeFlushTimerRef.current);
      writeFlushTimerRef.current = null;
    }
    const data = writeQueueRef.current;
    if (!data) return;
    writeQueueRef.current = '';
    invoke('terminal_write', { sessionId: targetSessionId, data }).catch(() => {});
  }, []);

  const enqueueTerminalWrite = useCallback((targetSessionId: string, data: string) => {
    if (data.length === 0) return;

    if (data === '\r' || data === '\n' || data === '' || data === '') {
      flushWriteQueue(targetSessionId);
      invoke('terminal_write', { sessionId: targetSessionId, data }).catch(() => {});
      return;
    }

    writeQueueRef.current += data;
    if (writeQueueRef.current.length >= 64) {
      flushWriteQueue(targetSessionId);
      return;
    }

    if (writeFlushTimerRef.current === null) {
      writeFlushTimerRef.current = window.setTimeout(() => {
        flushWriteQueue(targetSessionId);
      }, 16);
    }
  }, [flushWriteQueue]);

  useEffect(() => {
    if (!termRef.current) return;

    let disposed = false;
    outputPumpRef.current?.dispose();

    const terminalAppearance = getCurrentTerminalAppearance();
    const terminal = new Terminal({
      cursorBlink: true,
      fontSize: terminalAppearance.fontSize,
      fontFamily: terminalAppearance.fontFamily,
      scrollback: terminalAppearance.scrollback,
      smoothScrollDuration: 0,
      theme: terminalAppearance.theme,
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();
    const serializeAddon = new SerializeAddon();
    let webglAddon: WebglAddon | null = null;
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(webLinksAddon);
    terminal.loadAddon(serializeAddon);
    terminal.open(termRef.current);

    try {
      webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => {
        webglAddon?.dispose();
        webglAddon = null;
      });
      terminal.loadAddon(webglAddon);
    } catch {
      webglAddon = null;
    }

    fitAddon.fit();
    const trackerOutputCleaner = createTrackedCommandOutputCleaner();

    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;

    if (onTerminalReady) {
      onTerminalReady({
        getBuffer: () => {
          try {
            return serializeAddon.serialize();
          } catch {
            return '';
          }
        },
        getSelection: () => {
          try {
            return terminal.hasSelection() ? terminal.getSelection() : '';
          } catch {
            return '';
          }
        },
        pasteText: async (text) => {
          if (!text || disposed) {
            return;
          }
          const bracketedPaste = `\x1b[200~${text}\x1b[201~`;
          await invoke('terminal_write', { sessionId, data: bracketedPaste });
          terminal.focus();
        },
        insertCommand: async (command) => {
          const body = command.trimEnd();
          if (!body || disposed) {
            return;
          }
          const bracketedPaste = `\x1b[200~${body}\x1b[201~`;
          await invoke('terminal_write', { sessionId, data: bracketedPaste });
          terminal.focus();
        },
        executeCommand: (command, options) => {
          const timeoutMs = Math.max(1000, options?.timeoutMs ?? DEFAULT_COMMAND_TIMEOUT_MS);

          const runExecution = () => new Promise<TerminalCommandExecutionResult>((resolve) => {
            const executionId = crypto.randomUUID().replace(/-/g, '');
            if (disposed) {
              resolve({ executionId, status: 'closed', output: '' });
              return;
            }
            const donePrefix = `__OPSBATCH_AI_DONE_${executionId}__:`;
            const donePattern = new RegExp(`${escapeRegExp(donePrefix)}(-?\\d+)__`);
            const trackedCommand = createTrackedCommand(command, donePrefix);
            const bracketedPaste = `\x15\x1b[200~${trackedCommand.visibleCommand}${trackedCommand.hiddenTracker}\x1b[201~\r`;
            let settled = false;

            const settle = (result: TerminalCommandExecutionResult) => {
              if (settled) {
                return;
              }
              settled = true;
              resolve(result);
            };

            const timeoutId = window.setTimeout(() => {
              settleExecution(executionId, 'timeout');
            }, timeoutMs);

            pendingExecutionsRef.current.set(executionId, {
              executionId,
              donePattern,
              output: '',
              timeoutId,
              resolve: settle,
            });

            invoke('terminal_write', { sessionId, data: bracketedPaste }).catch(() => {
              settleExecution(executionId, 'write_failed');
            });
          });

          const executionPromise = commandExecutionChainRef.current.then(runExecution, runExecution);
          commandExecutionChainRef.current = executionPromise.then(() => undefined, () => undefined);
          return executionPromise;
        },
      });
    }

    const outputPump = createTerminalOutputPump({
      write: (data) => terminal.write(data),
      closeMessage: '\r\n\x1b[31m--- 连接已关闭 ---\x1b[0m\r\n',
      active: latestActiveRef.current,
    });
    outputPumpRef.current = outputPump;

    const unlistenPromise = listen<string>(`terminal-output-${sessionId}`, (event) => {
      if (disposed) {
        return;
      }

      if (event.payload === '') {
        failAllPendingExecutions('closed');
        outputPump.close();
        return;
      }

      const displayPayload = trackerOutputCleaner.clean(event.payload);
      appendExecutionOutput(displayPayload);
      detectCompletedExecutions(event.payload);
      if (displayPayload.length > 0) {
        outputPump.enqueue(displayPayload);
      }
    });

    const dataDisposable = terminal.onData((data) => {
      if (data === '\r') {
        const line = terminal.buffer.active.getLine(terminal.buffer.active.cursorY);
        const lineText = line ? line.translateToString(true).trim() : '';
        const writeQueueText = writeQueueRef.current.trim();

        if (pendingDangerConfirmRef.current === lineText) {
          pendingDangerConfirmRef.current = null;
          enqueueTerminalWrite(sessionId, data);
          return;
        }

        const cmd = writeQueueText || lineText;
        if (cmd) {
          const matched: string[] = [];
          for (const rule of dangerRulesRef.current) {
            try {
              if (new RegExp(rule.pattern).test(cmd)) {
                matched.push(rule.name);
              }
            } catch { /* skip */ }
          }
          if (matched.length > 0) {
            pendingDangerConfirmRef.current = lineText;
            terminal.write(`\r\n\x1b[33m⚠ OpsBatch 危险命令拦截: ${matched.join(', ')}\x1b[0m\r\n\x1b[33m  再按一次 Enter 确认执行，或编辑命令取消\x1b[0m\r\n`);
            return;
          }
        }
      } else {
        pendingDangerConfirmRef.current = null;
      }
      enqueueTerminalWrite(sessionId, data);
    });

    const handleFit = () => {
      if (disposed) return;
      const el = termRef.current;
      if (!el || el.offsetWidth === 0 || el.offsetHeight === 0) return;
      const prevCols = terminal.cols;
      const prevRows = terminal.rows;
      fitAddon.fit();
      const { cols, rows } = terminal;
      if (cols !== prevCols || rows !== prevRows) {
        invoke('terminal_resize', { sessionId, cols, rows }).catch(() => {});
      }
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

    let resizeFrame: number | null = null;
    let lastWidth = 0;
    let lastHeight = 0;
    const scheduleFit = () => {
      if (disposed || resizeFrame !== null) return;
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = null;
        const el = termRef.current;
        if (!el) return;
        const width = el.offsetWidth;
        const height = el.offsetHeight;
        if (width === 0 || height === 0 || (width === lastWidth && height === lastHeight)) return;
        lastWidth = width;
        lastHeight = height;
        handleFit();
      });
    };

    const resizeObserver = new ResizeObserver(scheduleFit);
    resizeObserver.observe(termRef.current);

    const initialResizeTimer = window.setTimeout(() => {
      if (disposed) return;
      const el = termRef.current;
      if (!el || el.offsetWidth === 0 || el.offsetHeight === 0) return;
      fitAddon.fit();
      const { cols, rows } = terminal;
      // Always send resize to trigger shell prompt redraw — initial SSH output
      // may have been emitted before this terminal subscribed to events.
      invoke('terminal_resize', { sessionId, cols, rows }).catch(() => {});
    }, 120);

    invoke<{ id: string; name: string; pattern: string; enabled: boolean; is_builtin: boolean }[]>('list_danger_rules')
      .then((rules) => {
        dangerRulesRef.current = rules.filter((r) => r.enabled).map((r) => ({ name: r.name, pattern: r.pattern }));
      })
      .catch(() => {});

    return () => {
      disposed = true;
      failAllPendingExecutions('closed');

      if (writeFlushTimerRef.current !== null) {
        window.clearTimeout(writeFlushTimerRef.current);
        writeFlushTimerRef.current = null;
      }

      // Clear refs immediately so nothing references stale objects.
      outputPumpRef.current = null;
      terminalRef.current = null;
      fitAddonRef.current = null;

      // Defer ALL teardown to the next event loop to keep tab-close responsive.
      const capturedResizeObserver = resizeObserver;
      const capturedDataDisposable = dataDisposable;
      const capturedUnlisten = unlistenPromise;
      const capturedOutputPump = outputPump;
      const capturedWebgl = webglAddon;
      const capturedTerminal = terminal;
      const capturedContainer = termRef.current;
      const capturedResizeFrame = resizeFrame;
      const capturedInitTimer = initialResizeTimer;
      webglAddon = null;
      resizeFrame = null;

      window.setTimeout(() => {
        if (capturedResizeFrame != null) cancelAnimationFrame(capturedResizeFrame);
        window.clearTimeout(capturedInitTimer);
        unsubscribeThemeChange();
        capturedResizeObserver.disconnect();
        capturedDataDisposable.dispose();
        capturedUnlisten.then((fn) => fn());
        capturedOutputPump.dispose();
        const terminalEl = capturedTerminal.element;
        if (capturedContainer && terminalEl && capturedContainer.contains(terminalEl)) {
          capturedContainer.removeChild(terminalEl);
        }
        capturedWebgl?.dispose();
        capturedTerminal.dispose();
      }, 0);
    };
  }, [sessionId, enqueueTerminalWrite, flushWriteQueue, onTerminalReady, settleExecution, appendExecutionOutput, detectCompletedExecutions, failAllPendingExecutions]);

  useEffect(() => {
    if (!active) return;
    const frame = requestAnimationFrame(() => {
      const fitAddon = fitAddonRef.current;
      const terminal = terminalRef.current;
      const el = termRef.current;
      if (!fitAddon || !terminal || !el || el.offsetWidth === 0 || el.offsetHeight === 0) return;
      const prevCols = terminal.cols;
      const prevRows = terminal.rows;
      fitAddon.fit();
      if (terminal.cols !== prevCols || terminal.rows !== prevRows) {
        invoke('terminal_resize', { sessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => {});
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [active, sessionId]);

  return <div ref={termRef} className="terminal-view" />;
});
