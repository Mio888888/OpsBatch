import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import * as protocol from '../src/pages/Rdp/rdpProtocol.ts';
import {
  getRdpHandshakeStatusMessage,
  getRdpOverlayText,
  getRdpStartupStatusMessage,
  isH264DirectUnavailableStatus,
  resolveH264UnavailableFallback,
  resolveRdpRenderMode,
  withRdpTimeout,
} from '../src/pages/Rdp/rdpProtocol.ts';

test('uses WebRTC video rendering when h264 direct is requested and ready', () => {
  const state = resolveRdpRenderMode({
    requested: 'h264Direct',
    webRtcReady: true,
  });

  assert.equal(state.renderMode, 'h264Direct');
  assert.equal(state.usesVideoElement, true);
});

test('falls back to legacy bitmap rendering when h264 direct is not ready', () => {
  const state = resolveRdpRenderMode({
    requested: 'h264Direct',
    webRtcReady: false,
    bitmapFallbackActive: false,
  });

  assert.equal(state.renderMode, 'legacyBitmap');
  assert.equal(state.usesVideoElement, false);
});

test('uses canvas rendering when h264 direct receives bitmap fallback frames', () => {
  const state = resolveRdpRenderMode({
    requested: 'h264Direct',
    webRtcReady: true,
    bitmapFallbackActive: true,
  });

  assert.equal(state.renderMode, 'legacyBitmap');
  assert.equal(state.usesVideoElement, false);
});

test('attaches WebRTC video and audio tracks to the RDP media element', () => {
  const shouldAttachRdpMediaTrack = (protocol as {
    shouldAttachRdpMediaTrack?: (kind: string) => boolean;
  }).shouldAttachRdpMediaTrack;

  assert.equal(typeof shouldAttachRdpMediaTrack, 'function');
  assert.equal(shouldAttachRdpMediaTrack('video'), true);
  assert.equal(shouldAttachRdpMediaTrack('audio'), true);
  assert.equal(shouldAttachRdpMediaTrack('data'), false);
});

test('maps command paste to a remote Windows ctrl+v shortcut', () => {
  const getRdpKeyboardInputEvents = (protocol as {
    getRdpKeyboardInputEvents?: (
      event: {
        key: string;
        code: string;
        ctrlKey?: boolean;
        metaKey?: boolean;
        altKey?: boolean;
        repeat?: boolean;
      },
      down: boolean,
    ) => unknown[];
  }).getRdpKeyboardInputEvents;

  assert.equal(typeof getRdpKeyboardInputEvents, 'function');
  assert.deepEqual(getRdpKeyboardInputEvents({
    key: 'v',
    code: 'KeyV',
    metaKey: true,
  }, true), [
    { type: 'key_scancode', code: 0x1d, extended: false, down: true },
    { type: 'key_scancode', code: 0x2f, extended: false, down: true },
  ]);
  assert.deepEqual(getRdpKeyboardInputEvents({
    key: 'v',
    code: 'KeyV',
    metaKey: true,
  }, false), [
    { type: 'key_scancode', code: 0x2f, extended: false, down: false },
    { type: 'key_scancode', code: 0x1d, extended: false, down: false },
  ]);
});

test('maps control copy to the remote c scancode without synthetic ctrl', () => {
  const getRdpKeyboardInputEvents = (protocol as {
    getRdpKeyboardInputEvents?: (
      event: {
        key: string;
        code: string;
        ctrlKey?: boolean;
        metaKey?: boolean;
        altKey?: boolean;
        repeat?: boolean;
      },
      down: boolean,
    ) => unknown[];
  }).getRdpKeyboardInputEvents;

  assert.equal(typeof getRdpKeyboardInputEvents, 'function');
  assert.deepEqual(getRdpKeyboardInputEvents({
    key: 'c',
    code: 'KeyC',
    ctrlKey: true,
  }, true), [
    { type: 'key_scancode', code: 0x2e, extended: false, down: true },
  ]);
});

test('maps control key presses and function keys to scancodes', () => {
  const getRdpKeyboardInputEvents = (protocol as {
    getRdpKeyboardInputEvents?: (
      event: {
        key: string;
        code: string;
        ctrlKey?: boolean;
        metaKey?: boolean;
        altKey?: boolean;
        repeat?: boolean;
      },
      down: boolean,
    ) => unknown[];
  }).getRdpKeyboardInputEvents;

  assert.equal(typeof getRdpKeyboardInputEvents, 'function');
  assert.deepEqual(getRdpKeyboardInputEvents({
    key: 'Control',
    code: 'ControlLeft',
  }, true), [
    { type: 'key_scancode', code: 0x1d, extended: false, down: true },
  ]);
  assert.deepEqual(getRdpKeyboardInputEvents({
    key: 'F5',
    code: 'F5',
  }, true), [
    { type: 'key_scancode', code: 0x3f, extended: false, down: true },
  ]);
});

test('maps alt tab as a remote tab scancode instead of unicode text', () => {
  const getRdpKeyboardInputEvents = (protocol as {
    getRdpKeyboardInputEvents?: (
      event: {
        key: string;
        code: string;
        ctrlKey?: boolean;
        metaKey?: boolean;
        altKey?: boolean;
        repeat?: boolean;
      },
      down: boolean,
    ) => unknown[];
  }).getRdpKeyboardInputEvents;

  assert.equal(typeof getRdpKeyboardInputEvents, 'function');
  assert.deepEqual(getRdpKeyboardInputEvents({
    key: 'Tab',
    code: 'Tab',
    altKey: true,
  }, true), [
    { type: 'key_scancode', code: 0x0f, extended: false, down: true },
  ]);
});

test('raises the RDP paint budget when the frame queue is backed up', () => {
  const getRdpFramePaintBudget = (protocol as {
    getRdpFramePaintBudget?: (queueLength: number) => number;
  }).getRdpFramePaintBudget;

  assert.equal(typeof getRdpFramePaintBudget, 'function');
  assert.equal(getRdpFramePaintBudget(0), 0);
  assert.equal(getRdpFramePaintBudget(4), 4);
  assert.equal(getRdpFramePaintBudget(12), 8);
  assert.equal(getRdpFramePaintBudget(64), 24);
});

test('detects backend status when Windows does not negotiate RDPGFX H.264', () => {
  assert.equal(isH264DirectUnavailableStatus({
    sessionId: 'rdp-1',
    state: 'connected',
    message: 'Windows RDPGFX 未协商 AVC/H.264',
    detail: {
      type: 'h264DirectUnavailable',
      reason: 'Windows RDPGFX 未协商 AVC/H.264，服务端正在发送 ClearCodec/bitmap。',
    },
  }), true);

  assert.equal(isH264DirectUnavailableStatus({
    sessionId: 'rdp-1',
    state: 'connected',
  }), false);
});

test('switches to legacy bitmap when H.264 direct is unavailable', () => {
  const fallback = resolveH264UnavailableFallback({
    sessionId: 'rdp-1',
    state: 'h264DirectUnavailable',
    message: 'Windows RDPGFX 未协商 AVC/H.264',
    detail: {
      type: 'h264DirectUnavailable',
      reason: 'Windows RDPGFX 未协商 AVC/H.264，服务端正在发送 ClearCodec/bitmap。',
    },
  }, 'h264Direct');

  assert.deepEqual(fallback, {
    nextTransportMode: 'legacyBitmap',
    requiresFreshSession: true,
    statusMessage: 'Windows RDPGFX 未协商 AVC/H.264，服务端正在发送 ClearCodec/bitmap。 正在切换到 legacy bitmap。',
  });
  assert.equal(resolveH264UnavailableFallback({
    sessionId: 'rdp-1',
    state: 'h264DirectUnavailable',
  }, 'legacyBitmap'), null);
});

test('rejects a stalled RDP promise with a clear timeout error', async () => {
  await assert.rejects(
    withRdpTimeout(new Promise(() => {}), 1, 'WebRTC signaling timed out'),
    /WebRTC signaling timed out/,
  );
});

test('describes the h264 direct startup phase before async listeners resolve', () => {
  assert.equal(getRdpStartupStatusMessage('h264Direct'), '正在创建 WebRTC H.264/音频通道');
});

test('describes the correct RDP handshake phase for each transport', () => {
  assert.equal(getRdpHandshakeStatusMessage('h264Direct'), '正在进行 RDP TCP、TLS、NLA 与 EGFX 握手');
  assert.equal(getRdpHandshakeStatusMessage('legacyBitmap'), '正在进行 RDP TCP、TLS 与 NLA 握手');
});

test('describes connected sessions that are still waiting for the first video frame', () => {
  const overlay = getRdpOverlayText({
    connectionState: 'connected',
    hasFrame: false,
    statusMessage: undefined,
    renderMode: 'h264Direct',
  });

  assert.equal(overlay.title, '正在等待远程画面');
  assert.equal(overlay.subtitle, 'RDP 已连接，正在等待 RDPGFX H.264 首帧。');
});

test('describes legacy bitmap sessions without implying H.264 video frames', () => {
  const overlay = getRdpOverlayText({
    connectionState: 'connected',
    hasFrame: false,
    statusMessage: undefined,
    renderMode: 'legacyBitmap',
  });

  assert.equal(overlay.title, '正在等待远程画面');
  assert.equal(overlay.subtitle, 'RDP 已连接，正在等待 bitmap/ClearCodec 首帧。');
});

test('RDP AI execution does not silently succeed without an active session', () => {
  const rdpPageSource = readFileSync('src/pages/Rdp/RdpPage.tsx', 'utf8');
  const aiChatSource = readFileSync('src/stores/aiChat.ts', 'utf8');
  const executorSource = readFileSync('src/utils/rdpAgentExecutor.ts', 'utf8');

  // 锚定到 AI 执行回调本身：无会话时必须 throw 而非静默 return。
  // 注意不要扫描整个文件——文件拖拽等无关路径存在合法的 `if (!rdpSessionId) return;`。
  const aiHandlerMatch = rdpPageSource.match(
    /const handleExecuteRdpOperations = useCallback\(\s*async \(ops: RdpOperation\[\]\) => \{([\s\S]*?)\},\s*\[/,
  );
  assert.ok(aiHandlerMatch, 'handleExecuteRdpOperations 回调应存在');
  const aiHandlerBody = aiHandlerMatch[1];
  assert.doesNotMatch(aiHandlerBody, /if \(!rdpSessionId\) return;/);
  assert.match(aiHandlerBody, /if \(!rdpSessionId\) \{\s*throw new Error/);
  assert.match(rdpPageSource, /RDP 会话未连接，无法执行 AI 操作/);
  assert.match(aiChatSource, /logHandledError\(\s*'rdp\.ai\.execute'/);
  assert.match(aiChatSource, /emitFrontendGlobalLog\(\s*'info',\s*'rdp\.ai\.execute'/);
  assert.match(aiChatSource, /stage=approve_start/);
  assert.match(aiChatSource, /stage=no_executor/);
  assert.match(aiChatSource, /stage=executor_done/);
  assert.match(executorSource, /emitFrontendGlobalLog\(\s*'info',\s*'rdp\.ai\.execute'/);
  assert.match(executorSource, /batchEvents=/);
});
