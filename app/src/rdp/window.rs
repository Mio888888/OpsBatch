//! The RDP window (port of `pages/Rdp/RdpPage.tsx` + `useRdpConnection.ts`): connection lifecycle, the remote
//! picture with mouse/keyboard forwarding, file drop upload, status cards and the AI agent panel.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, Selectable as _, Sizable as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_ai::parser::RdpOperation;
use ops_i18n::t;
use ops_rdp::frame::decode_frame_payload;
use ops_rdp::keys::{keyboard_input_events, modifier_events, KeyModifiers};
use ops_rdp::{RdpConnectRequest, RdpInputEvent, RdpMetricsPayload, RdpStatusPayload, RdpTransportMode};

use super::agent::{execute_operations, wait_for_settle};
use super::ai_panel::RdpAiPanel;
use super::input::{button_code, text_events, utf16_len, WheelAccumulator};
use super::rdp_manager;
use crate::remote::surface::{fit_rect, map_point_fit, swap_image, RemoteSurface};
use crate::window::{self, WindowKind};

const MIN_DESKTOP: (u32, u32) = (640, 480);
const MAX_DESKTOP: (u32, u32) = (1920, 1080);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(35);
const FRAMES_PER_BATCH: usize = 24;
const AI_ASIDE_WIDTH: f32 = 360.0;
const TOOLBAR_HEIGHT: f32 = 40.0;
/// Like the original page, every fresh connection asks for H.264 (EGFX, decoded locally); the window falls back to
/// legacy bitmap delivery on a new session when the server or the decoder cannot do it.
const PREFERRED_TRANSPORT: RdpTransportMode = RdpTransportMode::H264Direct;

/// Hosts whose server did not negotiate H.264 in this run: the next connections go straight to bitmap instead of
/// logging in twice (the failed EGFX attempt plus the fall-back) every time.
static NO_AVC_HOSTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn remember_no_avc(host_id: &str) {
    let mut hosts = NO_AVC_HOSTS.lock().unwrap();
    if !hosts.iter().any(|h| h == host_id) {
        hosts.push(host_id.to_string());
    }
}

pub fn forget_no_avc(host_id: &str) {
    NO_AVC_HOSTS.lock().unwrap().retain(|h| h != host_id);
}

fn preferred_transport(host: Option<&HostRequest>) -> RdpTransportMode {
    match host {
        Some(h) if NO_AVC_HOSTS.lock().unwrap().iter().any(|id| id == &h.host_id) => RdpTransportMode::LegacyBitmap,
        _ => PREFERRED_TRANSPORT,
    }
}

/// Message of the backend's `h264DirectUnavailable` status and of H.264 decode failures that end a session.
/// A session that dropped because the network went away (not because of credentials, the server logging the user
/// off, or a decoder problem) is worth reconnecting automatically.
pub fn is_connection_lost(message: &str) -> bool {
    message.starts_with("读取 RDP 帧失败")
}

/// Automatic reconnects per drop before the user is asked, and the wait before attempt `n` (1-based): 1s, 2s, 4s.
pub const MAX_AUTO_RECONNECTS: u32 = 3;

pub fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_millis(1000u64 << attempt.saturating_sub(1).min(4))
}

pub fn is_h264_failure(message: &str) -> bool {
    message.contains("H.264")
}

/// Status line shown while switching to bitmap delivery (`resolveH264UnavailableFallback`).
pub fn fallback_message(reason: &str) -> String {
    format!("{reason} 正在切换到 legacy bitmap。")
}

#[derive(Debug, Clone)]
pub struct HostRequest {
    pub host_id: String,
    pub name: String,
    pub ip: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpState {
    Idle,
    Connecting,
    Connected,
    Disconnected,
    Terminated,
    Error,
}

impl RdpState {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "idle" => Self::Idle,
            "connecting" => Self::Connecting,
            "connected" => Self::Connected,
            "disconnected" => Self::Disconnected,
            "terminated" => Self::Terminated,
            "error" => Self::Error,
            _ => return None,
        })
    }

    fn label(self) -> String {
        match self {
            Self::Idle => t!("rdp.state.idle"),
            Self::Connecting => t!("rdp.state.connecting"),
            Self::Connected => t!("rdp.state.connected"),
            Self::Disconnected => t!("rdp.state.disconnected"),
            Self::Terminated => t!("rdp.state.terminated"),
            Self::Error => t!("rdp.state.error"),
        }
    }
}

pub fn create_session_id(host_id: &str) -> String {
    format!("rdp-{host_id}-{}-{}", chrono::Utc::now().timestamp_millis(), &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// The remote desktop size to request for a stage of this size (never above 1920×1080, never below 640×480).
pub fn desktop_request_size(stage_w: f32, stage_h: f32) -> (u16, u16) {
    let (w, h) = (f64::from(stage_w.max(1.0)), f64::from(stage_h.max(1.0)));
    let scale = (f64::from(MAX_DESKTOP.0) / w).min(f64::from(MAX_DESKTOP.1) / h).min(1.0);
    let clamp = |v: f64, lo: u32, hi: u32| ((v * scale).round() as u32).clamp(lo, hi) as u16;
    (clamp(w, MIN_DESKTOP.0, MAX_DESKTOP.0), clamp(h, MIN_DESKTOP.1, MAX_DESKTOP.1))
}

struct FpsCounter {
    frames: u32,
    since: Instant,
    shown: Option<u32>,
}

pub struct RdpWindow {
    host: Option<HostRequest>,
    query_host_id: String,
    state: RdpState,
    status_message: Option<String>,
    session_id: Option<String>,
    desktop: (u16, u16),
    surface: RemoteSurface,
    image: Option<Arc<RenderImage>>,
    focus: FocusHandle,
    stage_bounds: Rc<Cell<(f32, f32, f32, f32)>>,
    fps: FpsCounter,
    metrics: Option<RdpMetricsPayload>,
    ai_open: bool,
    ai: Entity<RdpAiPanel>,
    toast: Option<String>,
    last_mods: Modifiers,
    wheel: WheelAccumulator,
    /// Text the input method is composing (shown under the picture until it is committed).
    marked: Option<String>,
    /// Where the pointer last was, in window coordinates: the input method puts its candidate list there.
    pointer: Option<Point<Pixels>>,
    pending_move: Option<(u16, u16)>,
    move_scheduled: bool,
    generation: u64,
    transport: RdpTransportMode,
    fell_back: bool,
    /// when the picture last changed in a way that matters (milliseconds since `clock_base`); read by the AI executor
    activity: Arc<std::sync::atomic::AtomicU64>,
    clock_base: Instant,
    /// automatic reconnects since the last time the session stayed up for a while
    auto_retries: u32,
    connected_at: Option<Instant>,
    tasks: Vec<Task<()>>,
}

impl RdpWindow {
    pub fn open(host: HostRequest, cx: &mut App) {
        let options = window::window_options(WindowKind::Rdp, &t!("rdp.windowTitle"), cx);
        let label = format!("rdp-{}", host.host_id);
        window::open_or_focus(&label, options, cx, move |window, cx| cx.new(|cx| Self::new(Some(host), String::new(), window, cx)));
    }

    /// Window without a host (the original's `/rdp` page opened without a target).
    pub fn open_empty(query_host_id: &str, cx: &mut App) {
        let options = window::window_options(WindowKind::Rdp, &t!("rdp.windowTitle"), cx);
        let q = query_host_id.to_string();
        window::open_or_focus("rdp", options, cx, move |window, cx| cx.new(|cx| Self::new(None, q, window, cx)));
    }

    pub fn new(host: Option<HostRequest>, query_host_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let weak = cx.entity().downgrade();
        let ai = cx.new(|cx| RdpAiPanel::new(weak, host.as_ref().map(|h| h.host_id.clone()).unwrap_or_default(), window, cx));
        let transport = preferred_transport(host.as_ref());
        let mut this = Self {
            host,
            query_host_id,
            state: RdpState::Idle,
            status_message: None,
            session_id: None,
            desktop: (1280, 720),
            surface: RemoteSurface::default(),
            image: None,
            focus: cx.focus_handle(),
            stage_bounds: Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))),
            fps: FpsCounter { frames: 0, since: Instant::now(), shown: None },
            metrics: None,
            // the original opens the assistant with the window; here it starts closed (the toolbar button opens it)
            ai_open: false,
            ai,
            toast: None,
            last_mods: Modifiers::default(),
            wheel: WheelAccumulator::default(),
            marked: None,
            pointer: None,
            pending_move: None,
            move_scheduled: false,
            generation: 0,
            transport,
            fell_back: false,
            activity: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            clock_base: Instant::now(),
            auto_retries: 0,
            connected_at: None,
            tasks: Vec::new(),
        };
        if this.host.is_some() {
            this.connect(window, cx);
        }
        this
    }

    // ---- accessors (tests / AI panel) ----

    pub fn state(&self) -> RdpState {
        self.state
    }
    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    pub fn desktop(&self) -> (u16, u16) {
        self.desktop
    }
    pub fn surface(&self) -> &RemoteSurface {
        &self.surface
    }
    pub fn ai_panel(&self) -> &Entity<RdpAiPanel> {
        &self.ai
    }
    pub fn ai_open(&self) -> bool {
        self.ai_open
    }
    pub fn host(&self) -> Option<&HostRequest> {
        self.host.as_ref()
    }
    pub fn metrics(&self) -> Option<&RdpMetricsPayload> {
        self.metrics.as_ref()
    }
    pub fn toast(&self) -> Option<&str> {
        self.toast.as_deref()
    }
    /// Transport the current session was requested with.
    pub fn transport(&self) -> RdpTransportMode {
        self.transport
    }
    /// Whether this window already switched from H.264 to bitmap delivery.
    pub fn fell_back(&self) -> bool {
        self.fell_back
    }

    /// Test seam: pretend a live session exists (see `RdpManager::register_test_session`).
    #[doc(hidden)]
    pub fn debug_attach_test_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.session_id = Some(session_id);
        self.state = RdpState::Connected;
        self.status_message = None;
        cx.notify();
    }

    pub fn set_ai_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.ai_open = open;
        cx.notify();
    }

    /// Screenshot for the AI (JPEG data URL, ≤1280×720).
    pub fn screenshot(&self) -> Option<String> {
        self.surface.screenshot_data_url(1280, 720, 72)
    }

    // ---- connection ----

    fn stage_size(&self, window: &Window) -> (f32, f32) {
        let vp = window.viewport_size();
        let aside = if self.ai_open { AI_ASIDE_WIDTH } else { 0.0 };
        (f32::from(vp.width) - aside, f32::from(vp.height) - TOOLBAR_HEIGHT)
    }

    pub fn disconnect_active(&mut self) {
        if let Some(sid) = self.session_id.take() {
            let _ = rdp_manager_for_drop().disconnect(&sid);
        }
        self.tasks.clear();
    }

    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else { return };
        self.disconnect_active();
        self.generation += 1;
        let generation = self.generation;
        let sid = create_session_id(&host.host_id);
        let (stage_w, stage_h) = self.stage_size(window);
        let (width, height) = desktop_request_size(stage_w, stage_h);
        self.session_id = Some(sid.clone());
        self.desktop = (width, height);
        self.state = RdpState::Connecting;
        self.status_message = Some(if let Some(m) = self.status_message.take().filter(|_| self.fell_back && self.transport == RdpTransportMode::LegacyBitmap) {
            m
        } else if self.transport == RdpTransportMode::H264Direct {
            "正在进行 RDP TCP、TLS、NLA 与 EGFX 握手".to_string()
        } else {
            "正在进行 RDP TCP、TLS 与 NLA 握手".to_string()
        });
        self.surface.clear();
        self.image = None;
        self.metrics = None;
        self.fps = FpsCounter { frames: 0, since: Instant::now(), shown: None };
        cx.notify();

        let ctx = crate::services::ctx(cx);
        let (frame_tx, frame_rx) = flume::unbounded::<Vec<u8>>();
        let status_rx = ctx.events.subscribe(format!("rdp-status-{sid}"));
        let metrics_rx = ctx.events.subscribe(format!("rdp-metrics-{sid}"));
        let transfer_rx = ctx.events.subscribe(format!("rdp-file-transfer-{sid}"));

        // Frames: drain in batches so a backlog is painted in few UI frames.
        self.tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = frame_rx.recv_async().await {
                let mut batch = vec![first];
                while batch.len() < FRAMES_PER_BATCH {
                    match frame_rx.try_recv() {
                        Ok(m) => batch.push(m),
                        Err(_) => break,
                    }
                }
                if this.update_in(cx, |this, _, cx| this.apply_frames(generation, batch, cx)).is_err() {
                    break;
                }
            }
        }));
        self.tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(ev) = status_rx.recv_async().await {
                let Some(payload) = ev.parse::<RdpStatusPayload>() else { continue };
                if this.update_in(cx, |this, window, cx| this.on_status(generation, payload, window, cx)).is_err() {
                    break;
                }
            }
        }));
        self.tasks.push(cx.spawn(async move |this, cx| {
            while let Ok(ev) = metrics_rx.recv_async().await {
                let Some(payload) = ev.parse::<RdpMetricsPayload>() else { continue };
                if this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.metrics = Some(payload);
                        cx.notify();
                    }
                })
                .is_err()
                {
                    break;
                }
            }
        }));
        self.tasks.push(cx.spawn(async move |this, cx| {
            while let Ok(ev) = transfer_rx.recv_async().await {
                let completed = ev.payload["completed"].as_array().map_or(0, Vec::len);
                let failed = ev.payload["failed"].as_array().map_or(0, Vec::len);
                let dir = ev.payload["downloadDir"].as_str().unwrap_or("").to_string();
                let mut parts = Vec::new();
                if completed > 0 {
                    parts.push(format!("已下载 {completed} 个文件到 {dir}"));
                }
                if failed > 0 {
                    parts.push(format!("{failed} 个文件下载失败"));
                }
                if !parts.is_empty() && this.update(cx, |this, cx| this.show_toast(parts.join("，"), Duration::from_secs(5), cx)).is_err() {
                    break;
                }
            }
        }));

        let request = RdpConnectRequest { host_id: host.host_id.clone(), session_id: Some(sid.clone()), width: Some(width), height: Some(height), domain: None, transport_mode: Some(self.transport) };
        let manager = rdp_manager(cx);
        let core = ctx.core.clone();
        self.tasks.push(cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move {
                match tokio::time::timeout(CONNECT_TIMEOUT, manager.connect_host(&core, request, frame_tx)).await {
                    Ok(r) => r,
                    Err(_) => Err("RDP 连接超时".to_string()),
                }
            })
            .await
            .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(resp) => {
                        this.desktop = (resp.width, resp.height);
                        if !matches!(this.state, RdpState::Error | RdpState::Terminated | RdpState::Disconnected) {
                            this.state = RdpState::Connected;
                            this.status_message = None;
                        }
                    }
                    Err(e) => {
                        this.state = RdpState::Error;
                        this.status_message = Some(e);
                    }
                }
                cx.notify();
            });
        }));
    }

    pub fn on_status(&mut self, generation: u64, payload: RdpStatusPayload, window: &mut Window, cx: &mut Context<Self>) {
        if self.generation != generation {
            return;
        }
        let unavailable = matches!(payload.detail, Some(ops_rdp::RdpStatusDetail::H264DirectUnavailable { .. }));
        if unavailable {
            if let Some(host) = &self.host {
                remember_no_avc(&host.host_id);
            }
        }
        let decode_failed = payload.state == "error" && payload.message.as_deref().is_some_and(is_h264_failure);
        if self.transport == RdpTransportMode::H264Direct && (unavailable || decode_failed) {
            let reason = payload.message.clone().unwrap_or_else(|| "服务器不支持 H.264 直显。".to_string());
            self.fall_back_to_bitmap(&reason, window, cx);
            return;
        }
        if payload.state == "connected" {
            self.connected_at = Some(Instant::now());
        }
        if payload.state == "error" && payload.message.as_deref().is_some_and(is_connection_lost) {
            // a session that held for 30s starts the count again
            if self.connected_at.is_some_and(|t| t.elapsed() > Duration::from_secs(30)) {
                self.auto_retries = 0;
            }
            if self.connected_at.is_some() && self.auto_retries < MAX_AUTO_RECONNECTS {
                self.auto_retries += 1;
                let (attempt, delay) = (self.auto_retries, reconnect_delay(self.auto_retries));
                let message = format!("连接中断，{} 秒后自动重连（{attempt}/{MAX_AUTO_RECONNECTS}）", delay.as_secs());
                self.state = RdpState::Connecting;
                self.status_message = Some(message.clone());
                self.show_toast(message, delay + Duration::from_secs(2), cx);
                let sid = self.session_id.clone();
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(delay).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        // still the dropped session (no manual reconnect, no other attempt in between)
                        if this.generation == generation && this.session_id == sid {
                            this.connect(window, cx);
                        }
                    });
                })
                .detach();
                cx.notify();
                return;
            }
        }
        if let Some(state) = RdpState::parse(&payload.state) {
            self.state = state;
            self.status_message = payload.message;
            cx.notify();
        }
    }

    /// Start a fresh bitmap session (the H.264 one cannot be converted), telling the user why.
    fn fall_back_to_bitmap(&mut self, reason: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.transport = RdpTransportMode::LegacyBitmap;
        self.fell_back = true;
        let message = fallback_message(reason);
        self.show_toast(message.clone(), Duration::from_secs(6), cx);
        // `connect` replaces the tasks that are running this handler, so let the current update finish first.
        cx.defer_in(window, move |this, window, cx| {
            this.connect(window, cx);
            this.status_message = Some(message);
            cx.notify();
        });
    }

    /// Paint a batch of frame messages into the surface.
    pub fn apply_frames(&mut self, generation: u64, batch: Vec<Vec<u8>>, cx: &mut Context<Self>) {
        if self.generation != generation {
            return;
        }
        let mut drew = 0u32;
        for message in batch {
            match decode_frame_payload(&message) {
                Ok(f) => {
                    // The cursor the client draws itself moves with every mouse event; only larger repaints
                    // (windows, menus, page loads) count as the remote screen reacting.
                    if f.region_width > 72 || f.region_height > 72 {
                        self.activity.store(self.clock_base.elapsed().as_millis() as u64, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.desktop = (f.width, f.height);
                    match self.surface.apply_rgba((f.width, f.height), f.x, f.y, f.region_width, f.region_height, &f.rgba) {
                        Ok(()) => drew += 1,
                        Err(_) => self.status_message = Some(t!("rdp.invalidFrame")),
                    }
                }
                Err(_) => self.status_message = Some(t!("rdp.invalidFrame")),
            }
        }
        if drew > 0 {
            self.fps.frames += drew;
            let elapsed = self.fps.since.elapsed();
            if elapsed >= Duration::from_secs(1) {
                self.fps.shown = Some((f64::from(self.fps.frames) / elapsed.as_secs_f64()).round() as u32);
                self.fps.frames = 0;
                self.fps.since = Instant::now();
            }
            cx.notify();
        }
    }

    /// Test hook: feed a frame message as if it came from the session.
    pub fn inject_frame(&mut self, message: Vec<u8>, cx: &mut Context<Self>) {
        let generation = self.generation;
        self.apply_frames(generation, vec![message], cx);
    }

    fn show_toast(&mut self, text: String, hold: Duration, cx: &mut Context<Self>) {
        self.toast = Some(text.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(hold).await;
            let _ = this.update(cx, |this, cx| {
                if this.toast.as_deref() == Some(text.as_str()) {
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ---- input ----

    fn connected_session(&self) -> Option<&str> {
        (self.state == RdpState::Connected).then_some(self.session_id.as_deref()).flatten()
    }

    fn send_input(&self, event: RdpInputEvent, cx: &App) {
        let Some(sid) = self.connected_session() else { return };
        let ctx = crate::services::ctx(cx);
        if let Err(e) = rdp_manager(cx).send_input(&ctx.core, sid, event) {
            log::warn!("rdp input failed: {e}");
        }
    }

    fn remote_point(&self, pos: Point<Pixels>) -> Option<(u16, u16)> {
        map_point_fit(self.stage_bounds.get(), self.desktop, (f32::from(pos.x), f32::from(pos.y)), false)
    }

    fn mouse_button(&mut self, button: MouseButton, pos: Point<Pixels>, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if down {
            window.focus(&self.focus, cx);
        }
        self.pointer = Some(pos);
        let Some((x, y)) = self.remote_point(pos) else { return };
        let Some(code) = button_code(button) else { return };
        self.send_input(RdpInputEvent::MouseButton { x, y, button: code, down }, cx);
    }

    fn mouse_moved(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.pointer = Some(pos);
        let Some(point) = self.remote_point(pos) else { return };
        self.pending_move = Some(point);
        if self.move_scheduled {
            return;
        }
        self.move_scheduled = true;
        let this = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = this.update(cx, |this, cx| {
                this.move_scheduled = false;
                if let Some((x, y)) = this.pending_move.take() {
                    this.send_input(RdpInputEvent::MouseMove { x, y }, cx);
                }
            });
        });
        cx.notify();
    }

    fn wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some((x, y)) = self.remote_point(ev.position) else {
            self.wheel.reset();
            return;
        };
        let delta = ev.delta.pixel_delta(px(super::input::PIXELS_PER_NOTCH));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        // GPUI scrolls content down for positive deltas, i.e. the wheel turned "up"; RDP positive = up as well.
        // Sideways it is the other way round: content moving right means scrolling left.
        for (vertical, delta) in self.wheel.push(dx, dy) {
            self.send_input(RdpInputEvent::Wheel { x, y, delta, vertical }, cx);
        }
        cx.stop_propagation();
    }

    fn key(&mut self, ev: &KeyDownEvent, down: bool, cx: &mut Context<Self>) {
        let m = ev.keystroke.modifiers;
        let mods = KeyModifiers { control: m.control, alt: m.alt, platform: m.platform };
        let events = keyboard_input_events(&ev.keystroke.key, ev.keystroke.key_char.as_deref(), mods, down, ev.is_held);
        if events.is_empty() {
            return;
        }
        for e in events {
            self.send_input(e, cx);
        }
        cx.stop_propagation();
    }

    fn modifiers_changed(&mut self, now: Modifiers, cx: &mut Context<Self>) {
        let km = |m: Modifiers| KeyModifiers { control: m.control, alt: m.alt, platform: m.platform };
        for e in modifier_events(self.last_mods.shift, now.shift, km(self.last_mods), km(now)) {
            self.send_input(e, cx);
        }
        self.last_mods = now;
    }

    fn drop_files(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sid) = self.session_id.clone().filter(|_| self.state == RdpState::Connected) else { return };
        let list: Vec<String> = paths.paths().iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if list.is_empty() {
            return;
        }
        let position = self.remote_point(window.mouse_position());
        self.show_toast(format!("正在上传 {} 个文件...", list.len()), Duration::from_secs(4), cx);
        let count = list.len();
        match rdp_manager(cx).upload_files(&sid, list, position) {
            Ok(()) => self.show_toast(format!("正在粘贴 {count} 个文件到远程桌面..."), Duration::from_secs(4), cx),
            Err(e) => self.show_toast(format!("上传失败: {e}"), Duration::from_secs(5), cx),
        }
    }

    // ---- AI agent ----

    /// Play AI operations on the remote desktop; resolves when the plan (including the settle pause) is done.
    pub fn execute_operations(&self, ops: Vec<RdpOperation>, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        let Some(sid) = self.session_id.clone().filter(|_| self.state == RdpState::Connected) else {
            return Task::ready(Err("RDP 会话未连接，无法执行 AI 操作".to_string()));
        };
        let manager = rdp_manager(cx);
        let core = crate::services::ctx(cx).core.clone();
        let executor = cx.background_executor().clone();
        let (activity, base) = (self.activity.clone(), self.clock_base);
        cx.background_spawn(async move {
            let sleeper = executor.clone();
            execute_operations(
                &ops,
                |event| manager.send_input(&core, &sid, event),
                move |d| {
                    let executor = sleeper.clone();
                    Box::pin(async move { executor.timer(d).await })
                },
                move |settle| {
                    let (executor, activity) = (executor.clone(), activity.clone());
                    Box::pin(async move {
                        wait_for_settle(
                            settle,
                            move || base.elapsed().as_millis() as u64,
                            move || activity.load(std::sync::atomic::Ordering::Relaxed),
                            move |d| {
                                let executor = executor.clone();
                                Box::pin(async move { executor.timer(d).await })
                            },
                        )
                        .await
                    })
                },
            )
            .await
        })
    }

    // ---- actions ----

    fn reconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.auto_retries = 0;
        self.transport = preferred_transport(self.host.as_ref());
        self.fell_back = false;
        self.connect(window, cx);
    }

    fn close_window(&mut self, window: &mut Window) {
        self.disconnect_active();
        window.remove_window();
    }

    /// Close this window and ask the main window to open the asset sidebar.
    fn close_and_open_assets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect_active();
        crate::window::request_open_assets(cx);
        window.remove_window();
    }

    /// `.rdp-status-pill`: (text, fill, border) per state. The original sets these light colours for every theme.
    fn pill_style(&self, cx: &App) -> (Hsla, Hsla, Hsla) {
        let ui = crate::theme::ui(cx);
        match self.state {
            RdpState::Connecting => (rgb(0x5a4312).into(), rgb(0xfff4d6).into(), rgb(0xf6d786).into()),
            RdpState::Error | RdpState::Terminated => (rgb(0x8c1d18).into(), rgb(0xfde1df).into(), rgb(0xf6aaa5).into()),
            RdpState::Connected => (rgb(0x0f5132).into(), rgb(0xd9f7e7).into(), rgb(0xa7e6c2).into()),
            _ => (ui.text_secondary, ui.main, ui.border_strong),
        }
    }
}

/// The faint 18px grid behind the picture (`.rdp-stage` background).
fn stage_grid(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let mut x = 0.0;
            while x < w {
                window.paint_quad(fill(Bounds::new(point(gpui_kit::px(ox + x), gpui_kit::px(oy)), size(gpui_kit::px(1.), gpui_kit::px(h))), color));
                x += 18.0;
            }
            let mut y = 0.0;
            while y < h {
                window.paint_quad(fill(Bounds::new(point(gpui_kit::px(ox), gpui_kit::px(oy + y)), size(gpui_kit::px(w), gpui_kit::px(1.))), color));
                y += 18.0;
            }
        },
    )
    .absolute()
    .size_full()
}

fn rdp_manager_for_drop() -> Arc<ops_rdp::RdpManager> {
    super::global_manager()
}

/// The local input method (Pinyin, Kana, Hangul...) composes text; what the user commits is typed on the remote
/// desktop as Unicode. While composing, the text is shown under the picture (the remote has no notion of it).
impl EntityInputHandler for RdpWindow {
    fn text_for_range(&mut self, range: std::ops::Range<usize>, adjusted: &mut Option<std::ops::Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        // the only text this view holds is the composition
        let text = self.marked.as_deref()?;
        let units: Vec<u16> = text.encode_utf16().collect();
        let (start, end) = (range.start.min(units.len()), range.end.min(units.len()));
        *adjusted = Some(start..end);
        String::from_utf16(&units[start..end]).ok()
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let end = self.marked.as_deref().map(utf16_len).unwrap_or(0);
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        self.marked.as_deref().filter(|m| !m.is_empty()).map(|m| 0..utf16_len(m))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(&mut self, _: Option<std::ops::Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        for e in text_events(text) {
            self.send_input(e, cx);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, _: Option<std::ops::Range<usize>>, new_text: &str, _: Option<std::ops::Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = Some(new_text.to_string()).filter(|t| !t.is_empty());
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: std::ops::Range<usize>, element_bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        // the candidate list opens next to the pointer (where the user is most likely typing), else at the bottom
        let at = self.pointer.unwrap_or_else(|| point(element_bounds.origin.x + element_bounds.size.width / 2.0, element_bounds.origin.y + element_bounds.size.height - px(48.)));
        Some(Bounds { origin: at, size: size(px(2.), px(20.)) })
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(0)
    }

    /// Only a connected session takes text; otherwise the input method stays out of the way.
    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.state == RdpState::Connected
    }
}

impl Drop for RdpWindow {
    fn drop(&mut self) {
        self.disconnect_active();
    }
}

impl Render for RdpWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        if let Some(img) = self.surface.take_image() {
            swap_image(&mut self.image, img, window);
        }
        // Keep pulling frames while a session is live (inactive RDP windows run at full rate, see WindowKind).
        if self.state == RdpState::Connected {
            window.request_animation_frame();
        }

        let Some(host) = self.host.clone() else {
            return v_flex()
                .id("rdp-window")
                .test_support()
                .size_full()
                .bg(theme.background)
                .text_color(theme.foreground)
                .child(TitleBar::new().child(div().text_size(px(13.)).child(format!("{} · {}", t!("rdp.windowTitle"), if self.query_host_id.is_empty() { t!("rdp.state.idle") } else { self.query_host_id.clone() }))))
                .child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("rdp.emptyTitle")))
                        .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("rdp.emptySubtitle")))
                        .child(Button::new("rdp-open-assets").primary().label(t!("rdp.openAssets")).on_click(cx.listener(|this, _, window, cx| this.close_and_open_assets(window, cx)))),
                )
                .into_any_element();
        };

        let has_frame = self.surface.has_frame();
        let showing_connect = self.state == RdpState::Connecting || (self.state == RdpState::Connected && !has_frame);
        let showing_error = matches!(self.state, RdpState::Error | RdpState::Terminated);
        let meta = {
            let mut s = host.ip.clone();
            if self.session_id.is_some() {
                s += &format!(" · {}x{} · {}", self.desktop.0, self.desktop.1, if self.transport == RdpTransportMode::H264Direct { "H.264" } else { "bitmap" });
            }
            if let Some(fps) = self.fps.shown {
                s += &format!(" · {fps} FPS");
            }
            if let Some(m) = &self.metrics {
                s += &format!(" · srv {}/s · tx {}/s · {:.1} MB/s", m.server_updates_per_second, m.sent_frames_per_second, m.sent_mbytes_per_second);
            }
            s
        };
        let (pill_fg, pill_bg, pill_border) = self.pill_style(cx);
        let ui = crate::theme::ui(cx);
        // `.rdp-page` / `.rdp-stage` / `.rdp-overlay` (dark and light variants)
        let (page_bg, stage_bg, grid_color, overlay_bg): (Hsla, Hsla, Hsla, Hsla) = if ui.is_dark {
            (rgb(0x181b1d).into(), rgb(0x111416).into(), rgba(0xb4cdc40d).into(), rgba(0x111416d6).into())
        } else {
            (rgb(0xf5f7f6).into(), rgb(0xeef3f0).into(), rgba(0x19302a0a).into(), rgba(0xf5f7f6d1).into())
        };
        // the picture: natural size (never stretched), centred, on its own dark panel with a soft shadow
        let stage_rect = self.stage_bounds.get();
        let picture = fit_rect(stage_rect, self.desktop, false).map(|(x, y, scale)| (x - stage_rect.0, y - stage_rect.1, f32::from(self.desktop.0) * scale, f32::from(self.desktop.1) * scale));
        let bounds_cell = self.stage_bounds.clone();
        let (ime_focus, ime_view) = (self.focus.clone(), cx.entity());

        let stage = div()
            .id("rdp-stage")
            .group("rdp-stage")
            .test_support()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(stage_bg)
            .track_focus(&self.focus)
            .key_context("RdpStage")
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Left, ev.position, true, window, cx)))
            .on_mouse_down(MouseButton::Middle, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Middle, ev.position, true, window, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Right, ev.position, true, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Left, ev.position, false, window, cx)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Middle, ev.position, false, window, cx)))
            .on_mouse_up(MouseButton::Right, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Right, ev.position, false, window, cx)))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Navigate(NavigationDirection::Back), ev.position, true, window, cx)))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Forward), cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Navigate(NavigationDirection::Forward), ev.position, true, window, cx)))
            .on_mouse_up(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Navigate(NavigationDirection::Back), ev.position, false, window, cx)))
            .on_mouse_up(MouseButton::Navigate(NavigationDirection::Forward), cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Navigate(NavigationDirection::Forward), ev.position, false, window, cx)))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| this.mouse_moved(ev.position, window, cx)))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| this.wheel(ev, cx)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.key(ev, true, cx)))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| {
                let m = ev.keystroke.modifiers;
                let mods = KeyModifiers { control: m.control, alt: m.alt, platform: m.platform };
                let events = keyboard_input_events(&ev.keystroke.key, ev.keystroke.key_char.as_deref(), mods, false, false);
                for e in events {
                    this.send_input(e, cx);
                }
            }))
            .on_modifiers_changed(cx.listener(|this, ev: &ModifiersChangedEvent, _, cx| this.modifiers_changed(ev.modifiers, cx)))
            .drag_over::<ExternalPaths>(|s, _, _, cx| s.border_2().border_color(cx.theme().primary))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.drop_files(paths, window, cx)))
            // Records where the stage is, for mouse mapping.
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height))),
                    // the local input method composes text for the remote desktop through this handler
                    move |bounds, _, window, cx| window.handle_input(&ime_focus, ElementInputHandler::new(bounds, ime_view.clone()), cx),
                )
                .absolute()
                .size_full(),
            )
            .child(stage_grid(grid_color))
            .when_some(self.marked.clone().filter(|m| !m.is_empty()), |d, text| {
                d.child(
                    div().absolute().bottom(px(18.)).left_0().right_0().flex().justify_center().child(
                        div().id("rdp-composition").test_support().px(px(10.)).py(px(4.)).rounded(px(6.)).bg(overlay_bg).border_1().border_color(grid_color).text_size(px(13.)).text_color(theme.foreground).child(text),
                    ),
                )
            })
            .when_some(self.image.clone().zip(picture), |d, (image, (x, y, w, h))| {
                d.child(
                    div()
                        .absolute()
                        .left(gpui_kit::px(x))
                        .top(gpui_kit::px(y))
                        .w(gpui_kit::px(w))
                        .h(gpui_kit::px(h))
                        .bg(rgb(0x101417))
                        .shadow(vec![BoxShadow { color: rgba(0x1c282429).into(), offset: point(px(0.), px(18.)), blur_radius: px(38.), spread_radius: px(0.), inset: false }])
                        .child(img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Fill)),
                )
            })
            .when(showing_connect, |d| {
                d.child(
                    // `.rdp-overlay`: 24px padding, 12px gap, strong 15px + 12px text
                    h_flex()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .gap(px(12.))
                        .p(px(24.))
                        .bg(overlay_bg)
                        .child(Spinner::new())
                        .child(
                            v_flex()
                                .gap(px(4.))
                                .child(div().id("rdp-overlay-title").test_support().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(ui.text).child(if self.state == RdpState::Connected { "正在等待远程画面".to_string() } else { t!("rdp.connectingTitle") }))
                                .child(div().text_size(px(12.)).text_color(ui.text_secondary).child(self.status_message.clone().unwrap_or_else(|| t!("rdp.connectingSubtitle")))),
                        ),
                )
            })
            .when(showing_error, |d| {
                d.child(
                    v_flex()
                        .id("rdp-error")
                        .test_support()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .gap(px(12.))
                        .p(px(24.))
                        .bg(overlay_bg)
                        .child(v_flex().items_center().gap(px(4.)).text_center().child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(ui.text).child(self.status_message.clone().unwrap_or_else(|| t!("rdp.connectFailed")))).child(div().text_size(px(12.)).text_color(ui.text_secondary).child(t!("rdp.errorFallback"))))
                        .child(
                            h_flex()
                                .gap(px(8.))
                                .child(Button::new("rdp-reconnect-card").primary().icon(Icon::new(IconName::RefreshCw)).label(t!("rdp.reconnect")).on_click(cx.listener(|this, _, window, cx| this.reconnect(window, cx))))
                                .child(Button::new("rdp-open-assets").label(t!("rdp.openAssets")).on_click(cx.listener(|this, _, window, cx| this.close_and_open_assets(window, cx)))),
                        ),
                )
            })
            .child(
                v_flex()
                    .id("rdp-drop-zone")
                    .test_support()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .bg(theme.background.opacity(0.8))
                    .border_2()
                    .border_color(theme.primary)
                    .invisible()
                    .group_drag_over::<ExternalPaths>("rdp-stage", |s| s.visible())
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("松开以上传文件到远程桌面"))
                    .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child("文件将通过剪贴板粘贴到远程")),
            )
            .when_some(self.toast.clone(), |d, text| {
                d.child(div().id("rdp-toast").test_support().absolute().bottom_4().left_4().px_3().py_1p5().rounded(px(6.)).bg(theme.popover).border_1().border_color(theme.border).text_size(px(12.)).child(text))
            });

        v_flex()
            .id("rdp-window")
            .test_support()
            .size_full()
            .bg(page_bg)
            .text_color(ui.text)
            .child(
                // `.rdp-toolbar`: 40px, 70px of window buttons, name + meta, status pill, small actions
                TitleBar::new().h(px(TOOLBAR_HEIGHT)).pl(px(if cfg!(target_os = "macos") { 82. } else { 12. })).pr(px(12.)).bg(ui.card).child(
                    h_flex()
                        .w_full()
                        .gap(px(10.))
                        .items_center()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(1.))
                                .child(div().truncate().text_size(px(12.)).font_weight(FontWeight::BOLD).text_color(ui.text).child(host.name.clone()))
                                .child(div().id("rdp-meta").test_support().truncate().text_size(px(12.)).text_color(ui.text_muted).child(meta)),
                        )
                        .child(
                            div()
                                .id("rdp-status")
                                .test_support()
                                .flex_none()
                                .min_w(px(68.))
                                .h(px(24.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(pill_border)
                                .bg(pill_bg)
                                .text_color(pill_fg)
                                .text_size(px(11.))
                                .font_weight(FontWeight::BOLD)
                                .child(self.state.label()),
                        )
                        .child(
                            h_flex()
                                .gap(px(6.))
                                .items_center()
                                .child(Button::new("rdp-ai-toggle").small().icon(Icon::new(IconName::Bot)).label(t!("rdp.aiToggle")).selected(self.ai_open).on_click(cx.listener(|this, _, _, cx| {
                                    let open = !this.ai_open;
                                    this.set_ai_open(open, cx);
                                })))
                                .child(Button::new("rdp-reconnect").small().icon(Icon::new(IconName::RefreshCw)).label(t!("rdp.reconnect")).on_click(cx.listener(|this, _, window, cx| this.reconnect(window, cx))))
                                .child(Button::new("rdp-disconnect").small().icon(Icon::new(IconName::X)).label(t!("rdp.disconnect")).on_click(cx.listener(|this, _, window, _| this.close_window(window)))),
                        ),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(stage)
                    .when(self.ai_open, |d| d.child(div().id("rdp-ai-aside").test_support().w(px(AI_ASIDE_WIDTH)).h_full().flex_none().border_l_1().border_color(ui.border).bg(ui.card).child(self.ai.clone()))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{create_session_id, desktop_request_size, fallback_message, is_h264_failure, RdpState};

    #[test]
    fn requested_desktop_follows_the_stage_within_limits() {
        assert_eq!(desktop_request_size(1280.0, 720.0), (1280, 720));
        assert_eq!(desktop_request_size(3840.0, 2160.0), (1920, 1080), "scaled down to the maximum");
        assert_eq!(desktop_request_size(300.0, 200.0), (640, 480), "raised to the minimum");
        assert_eq!(desktop_request_size(2560.0, 1080.0), (1920, 810));
    }

    #[test]
    fn session_ids_are_unique_and_prefixed() {
        let a = create_session_id("h1");
        let b = create_session_id("h1");
        assert!(a.starts_with("rdp-h1-") && a != b);
    }

    #[test]
    fn h264_failures_are_recognised_and_explained() {
        assert!(is_h264_failure("H.264 解码失败: x"));
        assert!(is_h264_failure("初始化 H.264 解码器失败: y"));
        assert!(!is_h264_failure("RDP 密码不能为空"), "authentication problems must not trigger a second login");
        assert_eq!(fallback_message("服务器不支持。"), "服务器不支持。 正在切换到 legacy bitmap。");
    }

    #[test]
    fn states_parse_from_backend_names() {
        assert_eq!(RdpState::parse("terminated"), Some(RdpState::Terminated));
        assert_eq!(RdpState::parse("h264DirectUnavailable"), None);
    }
}
