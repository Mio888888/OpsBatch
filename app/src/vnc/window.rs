//! The VNC window (port of `pages/Vnc/VncPage.tsx` + `vncClipboard.ts`): native RFB session, scaled picture,
//! pointer/keyboard forwarding, two-way clipboard sync, Ctrl+Alt+Del and the connection state cards.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, Sizable as _, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_vnc::keys::{keysym_for_key, modifier_changes, Mods};
use ops_vnc::{start_session, VncCommand, VncEvent, VncSession};

use crate::rdp::input::{WheelAccumulator, NOTCH};
use crate::remote::surface::{contain_rect, map_point, RemoteSurface, TileLayer};
use crate::window::{self, WindowKind};

/// How long a connection attempt may take before the window gives up (`VNC_CONNECT_WATCHDOG_MS`).
pub const CONNECT_WATCHDOG: Duration = Duration::from_secs(15);
const CLIPBOARD_POLL: Duration = Duration::from_millis(500);
pub const CLIPBOARD_MAX_CHARS: usize = 1_000_000;

const BUTTON_LEFT: u8 = 1;
const BUTTON_MIDDLE: u8 = 2;
const BUTTON_RIGHT: u8 = 4;
const WHEEL_UP: u8 = 8;
const WHEEL_DOWN: u8 = 16;
const WHEEL_LEFT: u8 = 32;
const WHEEL_RIGHT: u8 = 64;

#[derive(Debug, Clone)]
pub struct VncHost {
    pub host_id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VncState {
    Idle,
    Connecting,
    Connected,
    Disconnected,
    Error,
}

impl VncState {
    fn label(self) -> String {
        match self {
            Self::Idle => t!("vnc.state.idle"),
            Self::Connecting => t!("vnc.state.connecting"),
            Self::Connected => t!("vnc.state.connected"),
            Self::Disconnected => t!("vnc.state.disconnected"),
            Self::Error => t!("vnc.state.error"),
        }
    }
}

/// Text from the other side is only mirrored when it is small enough and actually new.
pub fn is_syncable_clipboard_text(text: &str, last: Option<&str>) -> bool {
    text.chars().count() <= CLIPBOARD_MAX_CHARS && last != Some(text)
}

pub struct VncWindow {
    host: Option<VncHost>,
    query_host_id: String,
    state: VncState,
    status_message: Option<String>,
    session: Option<VncSession>,
    view_only: bool,
    desktop: (u16, u16),
    surface: RemoteSurface,
    /// The picture on screen, one image per tile; only the tiles that changed are replaced.
    tiles: TileLayer,
    focus: FocusHandle,
    stage_bounds: Rc<Cell<(f32, f32, f32, f32)>>,
    last_mods: Modifiers,
    buttons: u8,
    pointer: (u16, u16),
    down_keys: HashMap<String, u32>,
    last_clipboard: Option<String>,
    /// The remote is a Mac (ARD login or Apple's RFB version): Command stays Command. Otherwise it is sent as Ctrl.
    remote_is_mac: bool,
    /// A trackpad sends many small scroll deltas; the remote wants whole notches.
    wheel: WheelAccumulator,
    generation: u64,
    tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl VncWindow {
    pub fn open(host: VncHost, cx: &mut App) {
        let options = window::window_options(WindowKind::Vnc, &t!("vnc.windowTitle"), cx);
        let label = format!("vnc-{}", host.host_id);
        window::open_or_focus(&label, options, cx, move |window, cx| cx.new(|cx| Self::new(Some(host), String::new(), window, cx)));
    }

    pub fn open_empty(query_host_id: &str, cx: &mut App) {
        let options = window::window_options(WindowKind::Vnc, &t!("vnc.windowTitle"), cx);
        let q = query_host_id.to_string();
        window::open_or_focus("vnc", options, cx, move |window, cx| cx.new(|cx| Self::new(None, q, window, cx)));
    }

    pub fn new(host: Option<VncHost>, query_host_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        // Leaving the window (or the picture) must not leave keys and buttons held down on the remote machine.
        let subscriptions = vec![
            cx.on_blur(&focus, window, |this, _, _| this.release_all_input()),
            cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.release_all_input();
                }
            }),
        ];
        let mut this = Self {
            host,
            query_host_id,
            state: VncState::Idle,
            status_message: None,
            session: None,
            view_only: false,
            desktop: (0, 0),
            surface: RemoteSurface::default(),
            tiles: TileLayer::default(),
            focus,
            stage_bounds: Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))),
            last_mods: Modifiers::default(),
            buttons: 0,
            pointer: (0, 0),
            down_keys: HashMap::new(),
            last_clipboard: None,
            remote_is_mac: false,
            wheel: WheelAccumulator::default(),
            generation: 0,
            tasks: Vec::new(),
            _subscriptions: subscriptions,
        };
        if this.host.is_some() {
            this.connect(window, cx);
        }
        this
    }

    // ---- accessors ----

    pub fn state(&self) -> VncState {
        self.state
    }
    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }
    pub fn desktop(&self) -> (u16, u16) {
        self.desktop
    }
    pub fn surface(&self) -> &RemoteSurface {
        &self.surface
    }
    pub fn view_only(&self) -> bool {
        self.view_only
    }
    /// How many tiles of the picture have an image on screen (all of them once a frame has been drawn).
    pub fn tile_images(&self) -> usize {
        self.tiles.count()
    }

    // ---- connection ----

    pub fn disconnect_active(&mut self) {
        if let Some(session) = self.session.take() {
            session.disconnect();
        }
        self.tasks.clear();
    }

    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else { return };
        self.disconnect_active();
        self.generation += 1;
        let generation = self.generation;
        self.state = VncState::Connecting;
        self.status_message = Some(t!("vnc.state.connecting"));
        self.surface.clear();
        self.tiles.clear(window);
        self.buttons = 0;
        self.down_keys.clear();
        self.wheel.reset();
        self.last_clipboard = None;
        cx.notify();

        let ctx = crate::services::ctx(cx);
        let host_id = host.host_id.clone();
        self.tasks.push(cx.spawn_in(window, async move |this, cx| {
            let loaded = ops_core::rt::run(async move {
                let db = ctx.db.clone();
                ops_core::rt::spawn_blocking(move || ops_vnc::load_vnc_host_config(&db, &host_id)).await.map_err(|e| e.to_string())?
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                match loaded {
                    Ok(config) => this.start(config, generation, window, cx),
                    Err(e) => {
                        this.state = VncState::Error;
                        this.status_message = Some(e);
                        cx.notify();
                    }
                }
            });
        }));

        // The watchdog: a handshake that never finishes becomes an error card.
        self.tasks.push(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CONNECT_WATCHDOG).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && this.state == VncState::Connecting {
                    this.disconnect_session_only();
                    this.state = VncState::Error;
                    this.status_message = Some(t!("vnc.connectTimeout"));
                    cx.notify();
                }
            });
        }));
    }

    fn disconnect_session_only(&mut self) {
        if let Some(session) = self.session.take() {
            session.disconnect();
        }
    }

    fn start(&mut self, config: ops_vnc::VncHostConfig, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.view_only = config.view_only;
        // Set from the server's "RFB 003.889" once connected.
        self.remote_is_mac = false;
        let session = start_session(config);
        let events = session.events.clone();
        self.session = Some(session);
        self.tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = events.recv_async().await {
                let mut batch = vec![first];
                while batch.len() < 32 {
                    match events.try_recv() {
                        Ok(e) => batch.push(e),
                        Err(_) => break,
                    }
                }
                if this.update_in(cx, |this, _, cx| this.on_events(generation, batch, cx)).is_err() {
                    break;
                }
            }
        }));
        // Local clipboard → remote while the window has focus.
        self.tasks.push(cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(CLIPBOARD_POLL).await;
            if this.update_in(cx, |this, window, cx| this.sync_local_clipboard(generation, window, cx)).is_err() {
                break;
            }
        }));
    }

    pub fn on_events(&mut self, generation: u64, batch: Vec<VncEvent>, cx: &mut Context<Self>) {
        if self.generation != generation {
            return;
        }
        for event in batch {
            match event {
                VncEvent::Connected { width, height, name, apple } => {
                    self.remote_is_mac |= apple;
                    self.desktop = (width, height);
                    self.surface.resize(width, height);
                    self.state = VncState::Connected;
                    self.status_message = (!name.is_empty()).then_some(name);
                }
                VncEvent::Frame(regions) => {
                    for (rect, bgra) in regions {
                        if self.surface.apply_bgra(self.desktop, rect.x, rect.y, rect.w, rect.h, &bgra).is_err() {
                            log::warn!("vnc: dropped an out-of-range update");
                        }
                    }
                }
                VncEvent::Resized { width, height } => {
                    self.desktop = (width, height);
                    self.surface.resize(width, height);
                }
                VncEvent::Clipboard(text) => {
                    if is_syncable_clipboard_text(&text, self.last_clipboard.as_deref()) {
                        self.last_clipboard = Some(text.clone());
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                VncEvent::Bell => {}
                VncEvent::Closed { clean, reason } => {
                    if matches!(self.state, VncState::Disconnected | VncState::Error) {
                        continue;
                    }
                    self.state = if clean { VncState::Disconnected } else { VncState::Error };
                    self.status_message = Some(if clean { t!("vnc.state.disconnected") } else { reason.unwrap_or_else(|| "VNC connection closed unexpectedly".into()) });
                    self.session = None;
                }
            }
        }
        cx.notify();
    }

    fn sync_local_clipboard(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.generation != generation || self.state != VncState::Connected || self.view_only || !window.is_window_active() {
            return;
        }
        self.push_local_clipboard(cx);
    }

    /// Sends the local clipboard text to the remote if it is new. Pasting calls this right before the key goes out,
    /// so the remote never pastes what it had before (the poll above only runs every half second).
    fn push_local_clipboard(&mut self, cx: &mut Context<Self>) {
        if self.view_only {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        if !is_syncable_clipboard_text(&text, self.last_clipboard.as_deref()) {
            return;
        }
        if let Some(session) = &self.session {
            session.send(VncCommand::Clipboard(text.clone()));
            self.last_clipboard = Some(text);
        }
    }

    // ---- input ----

    fn can_send(&self) -> bool {
        self.state == VncState::Connected && self.session.is_some()
    }

    fn send(&self, command: VncCommand) {
        if self.can_send() && !self.view_only {
            if let Some(session) = &self.session {
                session.send(command);
            }
        }
    }

    fn remote_point(&self, pos: Point<Pixels>) -> Option<(u16, u16)> {
        map_point(self.stage_bounds.get(), self.desktop, (f32::from(pos.x), f32::from(pos.y)))
    }

    fn send_pointer(&mut self, point: (u16, u16)) {
        self.pointer = point;
        self.send(VncCommand::Pointer { x: point.0, y: point.1, buttons: self.buttons });
    }

    fn mouse_button(&mut self, button: MouseButton, pos: Point<Pixels>, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if down {
            window.focus(&self.focus, cx);
        }
        let bit = match button {
            MouseButton::Left => BUTTON_LEFT,
            MouseButton::Middle => BUTTON_MIDDLE,
            MouseButton::Right => BUTTON_RIGHT,
            _ => return,
        };
        let Some(point) = self.remote_point(pos) else { return };
        if down {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        self.send_pointer(point);
    }

    /// Sent at once: the session merges the moves that pile up while a write is in flight, and nothing here needs a
    /// repaint (the remote picture shows the pointer itself), so a move costs neither a frame of delay nor a redraw.
    fn mouse_moved(&mut self, pos: Point<Pixels>) {
        let Some(point) = self.remote_point(pos) else { return };
        if point != self.pointer {
            self.send_pointer(point);
        }
    }

    fn wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(point) = self.remote_point(ev.position) else { return };
        let delta = ev.delta.pixel_delta(px(20.));
        // Same accumulator as the RDP window: a mouse wheel click is one notch, a trackpad adds up to whole notches.
        for (vertical, amount) in self.wheel.push(f32::from(delta.x), f32::from(delta.y)) {
            let bit = match (vertical, amount > 0) {
                (true, true) => WHEEL_UP,
                (true, false) => WHEEL_DOWN,
                (false, false) => WHEEL_LEFT,
                (false, true) => WHEEL_RIGHT,
            };
            for _ in 0..amount.unsigned_abs() / NOTCH as u16 {
                // A wheel notch is a press and release of a virtual button.
                self.send(VncCommand::Pointer { x: point.0, y: point.1, buttons: self.buttons | bit });
                self.send(VncCommand::Pointer { x: point.0, y: point.1, buttons: self.buttons });
            }
        }
        cx.stop_propagation();
    }

    fn key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = k.modifiers;
        let shortcut = m.control || m.alt || m.platform;
        // With a shortcut modifier the key's own symbol is what the remote expects (Ctrl+C, not the control char).
        let shifted_letter;
        let text = if shortcut {
            shifted_letter = (m.shift && k.key.chars().count() == 1).then(|| k.key.to_uppercase());
            shifted_letter.as_deref().or(Some(k.key.as_str()))
        } else {
            k.key_char.as_deref()
        };
        let Some(sym) = keysym_for_key(&k.key, text) else { return };
        let pastes = (k.key == "v" && (m.platform || m.control)) || (k.key == "insert" && m.shift);
        if pastes && self.can_send() {
            self.push_local_clipboard(cx);
        }
        self.send(VncCommand::Key { keysym: sym, down: true });
        if m.platform {
            // macOS swallows the key-up of a key pressed with Command held: release it now, or the remote keeps it down
            self.send(VncCommand::Key { keysym: sym, down: false });
        } else {
            self.down_keys.insert(k.key.clone(), sym);
        }
        cx.stop_propagation();
    }

    fn key_up(&mut self, ev: &KeyUpEvent) {
        if let Some(sym) = self.down_keys.remove(&ev.keystroke.key) {
            self.send(VncCommand::Key { keysym: sym, down: false });
        }
    }

    fn modifiers_changed(&mut self, now: Modifiers) {
        let cmd_as_ctrl = !self.remote_is_mac;
        let mods = |m: Modifiers| Mods { shift: m.shift, control: m.control, alt: m.alt, platform: m.platform }.for_remote(cmd_as_ctrl);
        for (sym, down) in modifier_changes(mods(self.last_mods), mods(now)) {
            self.send(VncCommand::Key { keysym: sym, down });
        }
        self.last_mods = now;
    }

    /// Lets go of everything held down: keys, modifiers and mouse buttons (focus left, or the mouse button was
    /// released outside the picture, where no release event reaches it).
    pub fn release_all_input(&mut self) {
        let keys: Vec<u32> = self.down_keys.drain().map(|(_, sym)| sym).collect();
        for sym in keys {
            self.send(VncCommand::Key { keysym: sym, down: false });
        }
        self.modifiers_changed(Modifiers::default());
        if self.buttons != 0 {
            self.buttons = 0;
            let p = self.pointer;
            self.send_pointer(p);
        }
    }

    pub fn send_ctrl_alt_del(&mut self) {
        self.send(VncCommand::CtrlAltDel);
    }

    // ---- actions ----

    fn close_window(&mut self, window: &mut Window) {
        self.disconnect_active();
        window.remove_window();
    }

    fn close_and_open_assets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect_active();
        crate::window::request_open_assets(cx);
        window.remove_window();
    }

    fn pill_color(&self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self.state {
            VncState::Connected => theme.success,
            VncState::Connecting => theme.warning,
            VncState::Error => theme.danger,
            _ => theme.muted_foreground,
        }
    }
}

impl Drop for VncWindow {
    fn drop(&mut self) {
        self.disconnect_active();
    }
}

impl Render for VncWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        self.tiles.refresh(&mut self.surface, window);

        let Some(host) = self.host.clone() else {
            return v_flex()
                .id("vnc-window")
                .test_support()
                .size_full()
                .bg(theme.background)
                .text_color(theme.foreground)
                .child(TitleBar::new().child(div().text_size(px(13.)).child(format!("{} · {}", t!("vnc.windowTitle"), if self.query_host_id.is_empty() { t!("vnc.state.idle") } else { self.query_host_id.clone() }))))
                .child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("vnc.emptyTitle")))
                        .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("vnc.emptySubtitle")))
                        .child(Button::new("vnc-open-assets").primary().label(t!("vnc.openAssets")).on_click(cx.listener(|this, _, window, cx| this.close_and_open_assets(window, cx)))),
                )
                .into_any_element();
        };

        let connecting = self.state == VncState::Connecting;
        let failed = self.state == VncState::Error;
        let pill = self.pill_color(cx);
        let connected = self.state == VncState::Connected;
        // How much the picture is scaled to fit the window, and what the remote machine calls itself.
        let scale_percent = connected.then(|| contain_rect(self.stage_bounds.get(), self.desktop)).flatten().map(|(_, _, scale)| (scale * 100.0).round() as u32);
        let server_name = connected.then(|| self.status_message.clone()).flatten();
        let chip = |text: String| div().flex_none().px_1p5().rounded(px(4.)).bg(theme.muted).text_size(px(10.)).text_color(theme.muted_foreground).child(text);
        let target = if host.ip.is_empty() { host.name.clone() } else { host.ip.clone() };
        let bounds_cell = self.stage_bounds.clone();
        let (tiles, desktop, show_picture) = (self.tiles.snapshot(), self.desktop, !connecting && !failed);

        let stage = div()
            .id("vnc-stage")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(rgb(0x101417))
            .track_focus(&self.focus)
            .key_context("VncStage")
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Left, ev.position, true, window, cx)))
            .on_mouse_down(MouseButton::Middle, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Middle, ev.position, true, window, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.mouse_button(MouseButton::Right, ev.position, true, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Left, ev.position, false, window, cx)))
            .on_mouse_up(MouseButton::Middle, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Middle, ev.position, false, window, cx)))
            .on_mouse_up(MouseButton::Right, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Right, ev.position, false, window, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Left, ev.position, false, window, cx)))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Middle, ev.position, false, window, cx)))
            .on_mouse_up_out(MouseButton::Right, cx.listener(|this, ev: &MouseUpEvent, window, cx| this.mouse_button(MouseButton::Right, ev.position, false, window, cx)))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, _| this.mouse_moved(ev.position)))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| this.wheel(ev, cx)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.key_down(ev, cx)))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, _| this.key_up(ev)))
            .on_modifiers_changed(cx.listener(|this, ev: &ModifiersChangedEvent, _, _| this.modifiers_changed(ev.modifiers)))
            .child(
                canvas(
                    move |bounds, _, _| {
                        bounds_cell.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height)));
                        bounds
                    },
                    move |_, bounds, window, _| {
                        if show_picture {
                            let stage = (f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height));
                            if let Some((ox, oy, scale)) = contain_rect(stage, desktop) {
                                tiles.paint(window, desktop, ox, oy, scale);
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .when(connecting, |d| {
                d.child(
                    v_flex()
                        .id("vnc-connecting")
                        .test_support()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            v_flex()
                                .w(px(460.))
                                .gap_2()
                                .p_4()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .child(h_flex().gap_2().items_center().text_size(px(12.)).text_color(theme.muted_foreground).child(div().size(px(8.)).rounded_full().bg(theme.warning)).child(t!("vnc.connectingTitle")))
                                .child(h_flex().gap_3().items_center().child(Spinner::new()).child(v_flex().child(div().font_weight(FontWeight::SEMIBOLD).child(t!("vnc.connectTo", target = target.clone()))).child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("vnc.connectingSubtitle")))))
                                .child(div().p_2().rounded(px(6.)).bg(theme.muted).font_family("Menlo").text_size(px(11.)).child(format!("$ opsbatch vnc connect --target {target}"))),
                        ),
                )
            })
            .when(failed, |d| {
                d.child(
                    v_flex()
                        .id("vnc-error")
                        .test_support()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .child(
                            v_flex()
                                .w(px(460.))
                                .gap_2()
                                .p_4()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(theme.danger)
                                .bg(theme.popover)
                                .child(h_flex().gap_2().items_center().text_size(px(12.)).text_color(theme.danger).child(div().size(px(8.)).rounded_full().bg(theme.danger)).child(t!("vnc.connectFailed")))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(t!("vnc.sessionStartFailed")))
                                .child(div().id("vnc-error-message").test_support().text_size(px(12.)).text_color(theme.muted_foreground).child(self.status_message.clone().unwrap_or_else(|| t!("vnc.errorFallback"))))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(Button::new("vnc-reconnect-card").primary().label(t!("vnc.reconnect")).on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))))
                                        .child(Button::new("vnc-open-assets").label(t!("vnc.openAssets")).on_click(cx.listener(|this, _, window, cx| this.close_and_open_assets(window, cx)))),
                                ),
                        ),
                )
            });

        v_flex()
            .id("vnc-window")
            .test_support()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .items_center()
                        .child(
                            v_flex()
                                .min_w_0()
                                .max_w(px(260.))
                                .child(div().id("vnc-host-name").test_support().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(host.name.clone()))
                                .child(div().id("vnc-host-address").test_support().truncate().text_size(px(10.)).text_color(theme.muted_foreground).child(format!("{}:{}", host.ip, host.port))),
                        )
                        .child(h_flex().id("vnc-status").test_support().flex_none().gap_1().items_center().text_size(px(11.)).text_color(pill).child(div().size(px(6.)).rounded_full().bg(pill)).child(self.state.label()))
                        .when(connected, |bar| {
                            bar.child(
                                h_flex()
                                    .id("vnc-info")
                                    .test_support()
                                    .min_w_0()
                                    .gap_1p5()
                                    .items_center()
                                    .child(chip(format!("{} × {}", self.desktop.0, self.desktop.1)))
                                    .when_some(scale_percent, |row, percent| row.child(chip(format!("{percent}%"))))
                                    .when(self.view_only, |row| row.child(chip(t!("vnc.viewOnly")).text_color(theme.warning)))
                                    .when_some(server_name.clone(), |row, name| row.child(div().id("vnc-server-name").test_support().truncate().max_w(px(200.)).text_size(px(10.)).text_color(theme.muted_foreground).child(name))),
                            )
                        })
                        .child(div().flex_1())
                        .child(Button::new("vnc-reconnect").small().icon(Icon::new(IconName::RefreshCw)).label(t!("vnc.reconnect")).disabled(connecting).on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))))
                        .child(Button::new("vnc-cad").small().label("Ctrl Alt Del").disabled(self.state != VncState::Connected || self.view_only).on_click(cx.listener(|this, _, _, _| this.send_ctrl_alt_del())))
                        .child(Button::new("vnc-disconnect").small().icon(Icon::new(IconName::X)).label(t!("vnc.disconnect")).on_click(cx.listener(|this, _, window, _| this.close_window(window)))),
                ),
            )
            .child(stage)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_syncable_clipboard_text, CLIPBOARD_MAX_CHARS};

    #[test]
    fn clipboard_text_is_synced_only_when_new_and_not_huge() {
        assert!(is_syncable_clipboard_text("hello", None));
        assert!(!is_syncable_clipboard_text("hello", Some("hello")));
        assert!(is_syncable_clipboard_text("hello", Some("other")));
        assert!(!is_syncable_clipboard_text(&"x".repeat(CLIPBOARD_MAX_CHARS + 1), None));
        assert!(is_syncable_clipboard_text(&"x".repeat(CLIPBOARD_MAX_CHARS), None));
    }
}
