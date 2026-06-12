import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import test from 'node:test';
import {
  buildRdpSettings,
  isVncRemoteDesktopHost,
} from '../src/utils/rdpSettings.ts';
import {
  createVncSessionId,
  vncDefaultResolution,
  vncPresentationSize,
  vncResolutionLimit,
} from '../src/pages/Vnc/vncProtocol.ts';
import type { Host } from '../src/types/index.ts';

function readVncBackendSource() {
  const rootSource = readFileSync('src-tauri/src/commands/vnc.rs', 'utf8');
  const moduleSources = existsSync('src-tauri/src/commands/vnc')
    ? readdirSync('src-tauri/src/commands/vnc')
      .filter((entry) => entry.endsWith('.rs'))
      .map((entry) => readFileSync(`src-tauri/src/commands/vnc/${entry}`, 'utf8'))
      .join('\n')
    : '';
  return `${rootSource}\n${moduleSources}`;
}

function readVncFrontendSource() {
  return [
    'src/pages/Vnc/VncPage.tsx',
    'src/pages/Vnc/vncProtocol.ts',
  ].map((path) => readFileSync(path, 'utf8')).join('\n');
}

test('builds VNC settings when VNC is selected as the host system', () => {
  const settings = buildRdpSettings({
    vncPort: 5901,
    vncUsername: '  alice  ',
    vncPassword: '  secret  ',
    vncViewOnly: true,
    vncShared: false,
  }, 'vnc');

  assert.deepEqual(settings, {
    protocol: 'vnc',
    vncPort: 5901,
    vncUsername: 'alice',
    vncPassword: 'secret',
    vncViewOnly: true,
    vncShared: false,
  });
});

test('maps Linux and Windows systems to their default remote access behavior', () => {
  assert.equal(buildRdpSettings({}, 'linux'), undefined);
  assert.deepEqual(buildRdpSettings({}, 'windows'), {
    protocol: 'rdp',
    enableClipboard: true,
    enableAudio: true,
    mapDisk: false,
  });
});

test('identifies hosts that should open through VNC instead of terminal or RDP', () => {
  const host = {
    os: 'vnc',
    rdpSettings: { protocol: 'vnc' },
  } as Pick<Host, 'os' | 'rdpSettings'>;

  assert.equal(isVncRemoteDesktopHost(host), true);
});

test('selects VNC from the system field instead of a remote access field', () => {
  const source = readFileSync('src/components/MainLayout.tsx', 'utf8');

  assert.match(source, /assets\.openVnc/);
  assert.match(source, /\{\s*value:\s*'vnc',\s*label:\s*t\('assets\.vncHost'\)\s*\}/);
  assert.doesNotMatch(source, /remoteDesktopProtocol/);
  assert.match(source, /vncPort/);
  assert.match(source, /kind:\s*'vnc'/);
});

test('opens VNC before prompting for SSH keychain secrets', () => {
  const source = readFileSync('src/components/MainLayout.tsx', 'utf8');
  const vncBranchIndex = source.indexOf('if (isVncHost(host))');
  const keychainNoticeIndex = source.indexOf('if (hostUsesStoredSecret(host)');

  assert.notEqual(vncBranchIndex, -1);
  assert.notEqual(keychainNoticeIndex, -1);
  assert.ok(vncBranchIndex < keychainNoticeIndex);
});

test('registers a managed VNC window and backend VNC commands', () => {
  const windowSource = readFileSync('src-tauri/src/commands/window.rs', 'utf8');
  const libSource = readFileSync('src-tauri/src/lib.rs', 'utf8');
  const appSource = readFileSync('src/App.tsx', 'utf8');
  const capabilitySource = readFileSync('src-tauri/capabilities/default.json', 'utf8');

  assert.match(windowSource, /Vnc/);
  assert.match(windowSource, /"vnc"/);
  assert.match(windowSource, /\/vnc\?hostId=/);
  assert.match(libSource, /commands::vnc::vnc_connect/);
  assert.match(libSource, /commands::vnc::vnc_disconnect/);
  assert.match(libSource, /commands::vnc::get_vnc_session_status/);
  assert.match(libSource, /commands::vnc::send_vnc_ctrl_alt_delete/);
  assert.match(appSource, /VncPage/);
  assert.match(capabilitySource, /"vnc-\*"/);
});

test('backend defaults missing VNC ports to 5900 instead of SSH or RDP ports', () => {
  const source = readVncBackendSource();

  assert.match(source, /const DEFAULT_VNC_PORT:\s*u16\s*=\s*5900/);
  assert.doesNotMatch(source, /\.or_else\(\|\| u16::try_from\(_?fallback\)\.ok\(\)\)/);
});

test('backend uses dedicated VNC username settings instead of the host SSH username', () => {
  const source = readVncBackendSource();

  assert.match(source, /vnc_username/);
  assert.doesNotMatch(source, /SELECT ip,\s*username,\s*port/);
  assert.doesNotMatch(source, /username:\s*username/);
});

test('backend exposes a local WebSocket TCP bridge for noVNC instead of decoding pixels', () => {
  const source = readVncBackendSource();
  const cargo = readFileSync('src-tauri/Cargo.toml', 'utf8');

  assert.match(cargo, /tokio-tungstenite/);
  assert.doesNotMatch(cargo, /vnc\s*=\s*\{\s*package\s*=\s*"vnc-rs"/);
  assert.match(source, /struct VncSessionManager/);
  assert.match(source, /TcpListener::bind\(\("127\.0\.0\.1",\s*0\)\)/);
  assert.match(source, /ws:\/\/127\.0\.0\.1:\{\}\/vnc\/\{\}/);
  assert.match(source, /accept_async/);
  assert.match(source, /Message::Binary/);
  assert.match(source, /ssh::connect_tcp_stream/);
  assert.match(source, /set_nodelay\(true\)/);
  assert.match(source, /tcpNoDelay=true/);
  assert.match(source, /bridge metrics/);
  assert.match(source, /BridgeTransferMetrics/);
  assert.match(source, /OPSBATCH_VNC_BRIDGE_METRICS/);
  assert.match(source, /VNC_BRIDGE_READ_BUFFER_BYTES:\s*usize\s*=\s*128\s*\*\s*1024/);
  assert.match(source, /websocket_url/);
  assert.match(source, /passwordSet=/);
  assert.match(source, /rfb security types/);
  assert.match(source, /prefer_vnc_auth_security_types/);
  assert.doesNotMatch(source, /tauri::ipc::\{Channel,\s*Response\}/);
  assert.doesNotMatch(source, /Channel<Response>/);
  assert.doesNotMatch(source, /build_vnc_frame_message/);
  assert.doesNotMatch(source, /raw_pixels_to_rgba/);
  assert.doesNotMatch(source, /VncEncoding/);
  assert.doesNotMatch(source, /password=/);
});

test('backend bridge sessions are closed through the existing VNC session manager API', () => {
  const source = readVncBackendSource();

  assert.match(source, /Arc<Mutex<HashMap<String,\s*VncBridgeSession>>>/);
  assert.match(source, /oneshot::Sender<\(\)>/);
  assert.match(source, /sessions\.remove\(&request\.session_id\)/);
  assert.match(source, /get_vnc_session_status/);
});

test('frontend uses noVNC RFB directly and does not keep the legacy pixel pipeline', () => {
  const source = readVncFrontendSource();

  assert.match(source, /import RFB from '@novnc\/novnc'/);
  assert.match(source, /new RFB\(/);
  assert.match(source, /websocketUrl/);
  assert.match(source, /scaleViewport\s*=\s*true/);
  assert.match(source, /resizeSession\s*=\s*true/);
  assert.match(source, /VNC_INTERACTIVE_QUALITY_LEVEL\s*=\s*2/);
  assert.match(source, /VNC_INTERACTIVE_COMPRESSION_LEVEL\s*=\s*0/);
  assert.match(source, /qualityLevel\s*=\s*VNC_INTERACTIVE_QUALITY_LEVEL/);
  assert.match(source, /compressionLevel\s*=\s*VNC_INTERACTIVE_COMPRESSION_LEVEL/);
  assert.match(source, /focusOnClick\s*=\s*true/);
  assert.match(source, /describeRfbRuntime/);
  assert.match(source, /installVncPerformanceDiagnostics/);
  assert.match(source, /installVncInputDiagnostics/);
  assert.match(source, /isVncDebugEnabled/);
  assert.match(source, /vncDebugEnabled\) \{\s*disposePerformanceDiagnostics = installVncPerformanceDiagnostics/);
  assert.match(source, /novnc perf/);
  assert.match(source, /novnc input/);
  assert.match(source, /sendCtrlAltDel\(\)/);
  assert.match(source, /credentialsrequired/);
  assert.match(source, /securityfailure/);
  assert.match(source, /desktopname/);
  assert.doesNotMatch(source, /new Channel<ArrayBuffer \| Uint8Array>/);
  assert.doesNotMatch(source, /decodeVncFramePayload/);
  assert.doesNotMatch(source, /VncWebglRenderer/);
  assert.doesNotMatch(source, /send_vnc_pointer_event/);
  assert.doesNotMatch(source, /send_vnc_key_event/);
  assert.doesNotMatch(source, /texSubImage2D/);
});

test('frontend closes stale noVNC sessions after asynchronous effect cleanup', () => {
  const source = readVncFrontendSource();

  assert.match(source, /\/\* @refresh reset \*\//);
  assert.match(source, /const activeHostId = host\?\.id \?\? ''/);
  assert.match(source, /const tTextRef = useRef\(tText\)/);
  assert.match(source, /tTextRef\.current = tText/);
  assert.match(source, /createVncSessionId\(activeHostId\)/);
  assert.match(source, /hostId: activeHostId/);
  assert.match(source, /if \(disposed\) \{\s*closeVncSession\(nextSessionId, false\);/);
  assert.match(source, /rfbRef\.current\?\.disconnect\(\)/);
  assert.match(source, /\}, \[activeHostId, connectNonce, closeVncSession, updateSessionId, updateStatus, vncDebugEnabled\]\);/);
  assert.match(source, /manual reconnect requested/);
  assert.doesNotMatch(source, /\}, \[[^\]]*\bhost\b[^\]]*\]\);/);
  assert.doesNotMatch(source, /\}, \[[^\]]*\btText\b[^\]]*\]\);/);
  assert.doesNotMatch(source, /\}, \[activeHostId,\s*connectNonce,[^\]]*hosts\.length/);
  assert.doesNotMatch(source, /\}, \[activeHostId,\s*connectNonce,[^\]]*queryHostId/);
});

test('VNC connection path writes diagnostics for frontend and backend handoff points', () => {
  const frontendSource = readVncFrontendSource();
  const backendSource = readVncBackendSource();

  assert.match(frontendSource, /writeVncDiagnosticLog/);
  assert.match(frontendSource, /write_diagnostic_log/);
  assert.match(frontendSource, /vnc-frontend/);
  assert.match(frontendSource, /hasRequiredVncCredentials/);
  assert.match(frontendSource, /effect start hostId=/);
  assert.match(frontendSource, /invoke vnc_connect start/);
  assert.match(frontendSource, /invoke vnc_connect failed/);
  assert.match(frontendSource, /novnc connect/);
  assert.match(frontendSource, /novnc disconnect/);
  assert.match(frontendSource, /effect cleanup/);

  assert.match(backendSource, /append_vnc_diagnostic_log/);
  assert.match(backendSource, /vnc-backend/);
  assert.match(backendSource, /command received hostId=/);
  assert.match(backendSource, /config loaded hostId=/);
  assert.match(backendSource, /bridge listening sessionId=/);
  assert.match(backendSource, /bridge tcp connect failed sessionId=/);
  assert.match(backendSource, /bridge closed sessionId=/);
  assert.match(backendSource, /passwordSet=/);
  assert.doesNotMatch(backendSource, /password=/);
});

test('frontend constrains VNC display defaults and maximum presentation size', () => {
  const source = readVncFrontendSource();

  assert.deepEqual(vncDefaultResolution(), { width: 1280, height: 720 });
  assert.deepEqual(vncResolutionLimit(3840, 2160), { width: 1920, height: 1080 });
  assert.deepEqual(vncResolutionLimit(1600, 900), { width: 1600, height: 900 });
  assert.deepEqual(vncPresentationSize(1920, 1200), { width: 1728, height: 1080 });
  assert.deepEqual(vncPresentationSize(3456, 2234), { width: 1671, height: 1080 });
  assert.match(source, /vncDefaultResolution/);
  assert.match(source, /vncResolutionLimit/);
  assert.match(source, /vncPresentationSize/);
  assert.match(source, /rdp-vnc-screen/);
});

test('VNC session ids remain local and URL-safe for the bridge path', () => {
  const sessionId = createVncSessionId('host-1');

  assert.match(sessionId, /^vnc-host-1-\d+-[a-z0-9]+$/);
  assert.doesNotMatch(sessionId, /[/?#]/);
});
