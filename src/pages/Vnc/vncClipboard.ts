import type RFB from '@novnc/novnc';

export const VNC_CLIPBOARD_POLL_INTERVAL_MS = 500;
export const VNC_CLIPBOARD_MAX_TEXT_CHARS = 1_000_000;
const EXTENDED_CLIPBOARD_FORMAT_TEXT = 1;
const EXTENDED_CLIPBOARD_ACTION_REQUEST = 1 << 25;
const EXTENDED_CLIPBOARD_ACTION_NOTIFY = 1 << 27;
const EXTENDED_CLIPBOARD_ACTION_PROVIDE = 1 << 28;

type ClipboardRfb = Pick<RFB, 'addEventListener' | 'removeEventListener' | 'clipboardPasteFrom' | 'viewOnly'>;

type ClipboardEventDetail = {
  text?: string;
};

type TimerId = ReturnType<typeof window.setInterval>;
type NoVncClipboardDebugState = {
  _clipboardServerCapabilitiesFormats?: Record<string, unknown>;
  _clipboardServerCapabilitiesActions?: Record<string, unknown>;
};

export interface VncClipboardBridgeOptions {
  hostId: string;
  sessionId: string;
  readLocalClipboardText: () => Promise<string | null>;
  writeLocalClipboardText: (text: string) => Promise<void>;
  canSendLocalClipboard?: () => boolean;
  writeDiagnosticLog?: (message: string) => void;
  setIntervalFn?: typeof window.setInterval;
  clearIntervalFn?: typeof window.clearInterval;
}

export interface VncClipboardBridge {
  dispose: () => void;
  syncLocalToRemote: () => Promise<void>;
}

export function isSyncableVncClipboardText(text: string | null | undefined): text is string {
  return typeof text === 'string' && text.length <= VNC_CLIPBOARD_MAX_TEXT_CHARS;
}

export function describeVncClipboardCapabilities(rfb: unknown): string {
  const debugState = rfb as NoVncClipboardDebugState;
  const formats = debugState._clipboardServerCapabilitiesFormats ?? {};
  const actions = debugState._clipboardServerCapabilitiesActions ?? {};
  const hasFormat = (flag: number) => Boolean(formats[String(flag)]);
  const hasAction = (flag: number) => Boolean(actions[String(flag)]);

  return [
    `extendedText=${hasFormat(EXTENDED_CLIPBOARD_FORMAT_TEXT)}`,
    `request=${hasAction(EXTENDED_CLIPBOARD_ACTION_REQUEST)}`,
    `notify=${hasAction(EXTENDED_CLIPBOARD_ACTION_NOTIFY)}`,
    `provide=${hasAction(EXTENDED_CLIPBOARD_ACTION_PROVIDE)}`,
  ].join(' ');
}

export function createVncClipboardBridge(
  rfb: ClipboardRfb,
  options: VncClipboardBridgeOptions,
): VncClipboardBridge {
  const setIntervalFn = options.setIntervalFn ?? window.setInterval.bind(window);
  const clearIntervalFn = options.clearIntervalFn ?? window.clearInterval.bind(window);
  let disposed = false;
  let lastText: string | null = null;

  const log = (message: string) => {
    options.writeDiagnosticLog?.(
      `novnc clipboard hostId=${options.hostId} sessionId=${options.sessionId} ${message}`,
    );
  };

  const syncLocalToRemote = async () => {
    try {
      const text = await options.readLocalClipboardText();
      if (disposed || !isSyncableVncClipboardText(text) || text === lastText) return;
      if (rfb.viewOnly || options.canSendLocalClipboard?.() === false) return;
      rfb.clipboardPasteFrom(text);
      lastText = text;
      log(`localToRemote chars=${text.length} ${describeVncClipboardCapabilities(rfb)}`);
    } catch (error) {
      log(`localReadFailed error=${String(error)}`);
    }
  };

  const writeRemoteTextToLocal = (event: Event) => {
    const text = (event as CustomEvent<ClipboardEventDetail>).detail?.text;
    if (!isSyncableVncClipboardText(text) || text === lastText) return;
    lastText = text;
    void options.writeLocalClipboardText(text)
      .then(() => log(`remoteToLocal chars=${text.length}`))
      .catch((error: unknown) => log(`localWriteFailed error=${String(error)}`));
  };

  rfb.addEventListener('clipboard', writeRemoteTextToLocal);
  const interval: TimerId = setIntervalFn(() => {
    void syncLocalToRemote();
  }, VNC_CLIPBOARD_POLL_INTERVAL_MS);
  void syncLocalToRemote();

  return {
    dispose: () => {
      disposed = true;
      clearIntervalFn(interval);
      rfb.removeEventListener('clipboard', writeRemoteTextToLocal);
    },
    syncLocalToRemote,
  };
}
