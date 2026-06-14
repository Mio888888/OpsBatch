import { invoke } from '@tauri-apps/api/core';

type ConsoleMethod = 'log' | 'info' | 'warn' | 'error';

const INSTALLED_FLAG = '__opsbatchGlobalLogHandlerInstalled';
const MAX_LOG_MESSAGE_LENGTH = 12000;
const CONSOLE_METHODS: ConsoleMethod[] = ['log', 'info', 'warn', 'error'];

declare global {
  interface Window {
    __opsbatchGlobalLogHandlerInstalled?: boolean;
  }
}

export function sanitizeLogMessage(message: string) {
  return message
    .replace(/(Bearer\s+)[^\s"']+/gi, '$1***')
    .replace(/((?:api[_-]?key|token|password)\s*[:=]\s*)[^\s,"']+/gi, '$1***')
    .slice(0, MAX_LOG_MESSAGE_LENGTH);
}

export function formatLogArguments(args: unknown[]) {
  return sanitizeLogMessage(args.map(formatLogValue).join(' '));
}

export function getConsoleLogLevel(method: string) {
  if (method === 'warn') return 'warn';
  if (method === 'error') return 'error';
  return 'info';
}

export function formatUnhandledReason(reason: unknown) {
  return formatLogArguments([reason]);
}

export function logHandledError(source: string, error: unknown, level = 'error') {
  return emitFrontendGlobalLog(level, source, formatLogArguments([error]));
}

export function emitFrontendGlobalLog(level: string, source: string, message: string) {
  const redacted = sanitizeLogMessage(message);
  return invoke('emit_frontend_log', {
    level,
    source,
    message: redacted,
  }).catch(() => undefined);
}

export function installGlobalLogHandler() {
  if (typeof window === 'undefined') return;
  if (window[INSTALLED_FLAG]) return;
  window[INSTALLED_FLAG] = true;

  for (const method of CONSOLE_METHODS) {
    const original = console[method].bind(console);
    console[method] = (...args: unknown[]) => {
      original(...args);
      void emitFrontendGlobalLog(getConsoleLogLevel(method), 'console', formatLogArguments(args));
    };
  }

  window.addEventListener('error', (event) => {
    const location = event.filename
      ? ` (${event.filename}:${event.lineno}:${event.colno})`
      : '';
    void emitFrontendGlobalLog('error', 'window', `${event.message}${location}`);
  });

  window.addEventListener('unhandledrejection', (event) => {
    void emitFrontendGlobalLog('error', 'promise', formatUnhandledReason(event.reason));
  });
}

function formatLogValue(value: unknown): string {
  if (value instanceof Error) {
    return value.stack || `${value.name}: ${value.message}`;
  }
  if (typeof value === 'string') {
    return value;
  }
  if (typeof value === 'undefined') {
    return 'undefined';
  }

  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
