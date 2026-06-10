import assert from 'node:assert/strict';
import test from 'node:test';
import {
  getRdpHandshakeStatusMessage,
  getRdpOverlayText,
  getRdpStartupStatusMessage,
  isH264DirectUnavailableStatus,
  resolveH264UnavailableFallback,
  resolveRdpRenderMode,
  shouldAttachRdpVideoTrack,
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

test('only attaches WebRTC video tracks to the video element', () => {
  assert.equal(shouldAttachRdpVideoTrack('video'), true);
  assert.equal(shouldAttachRdpVideoTrack('audio'), false);
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
