//! "Add host" / "Edit host" dialog (port of the host Modal in MainLayout.tsx): basic tab plus one advanced tab whose
//! content depends on the system type (Linux: jump chain + proxy, Windows: RDP + proxy, VNC: proxy).

use std::collections::HashMap;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_ssh::hosts::Host;

use crate::asset_tree::{group_choices, HostKind, DEFAULT_GROUP_ID};
use crate::host_form::{AuthKind, Field, HostFormValues, OsKind, ProxyKind, VncAuth};
use crate::stores::assets::AssetsStore;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::form::{field, section, switch_row};
use crate::widgets::toast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Basic,
    Advanced,
}

pub struct Inputs {
    pub name: Entity<InputState>,
    pub ip: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub private_key: Entity<TextareaState>,
    pub remark: Entity<InputState>,
    pub rdp_domain: Entity<InputState>,
    pub rdp_width: Entity<InputState>,
    pub rdp_height: Entity<InputState>,
    pub rdp_disk_path: Entity<InputState>,
    pub vnc_port: Entity<InputState>,
    pub vnc_username: Entity<InputState>,
    pub vnc_password: Entity<InputState>,
    pub proxy_host: Entity<InputState>,
    pub proxy_port: Entity<InputState>,
    pub proxy_username: Entity<InputState>,
    pub proxy_password: Entity<InputState>,
}

pub struct Selects {
    pub os: Entity<ChoiceSelect>,
    pub auth: Entity<ChoiceSelect>,
    pub group: Entity<ChoiceSelect>,
    pub vnc_auth: Entity<ChoiceSelect>,
    pub proxy_type: Entity<ChoiceSelect>,
}

pub struct HostDialog {
    store: Entity<AssetsStore>,
    editing: Option<Host>,
    values: HostFormValues,
    jump_candidates: Vec<(String, String)>,
    tab: Tab,
    pub inputs: Inputs,
    pub selects: Selects,
    errors: Vec<Field>,
    saving: bool,
    error: Option<String>,
    _subs: Vec<Subscription>,
}

fn text_input(window: &mut Window, cx: &mut Context<HostDialog>, placeholder: String, value: &str, masked: bool) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(|cx| {
        let s = InputState::new(window, cx).placeholder(placeholder).default_value(value);
        if masked { s.masked(true) } else { s }
    })
}

fn os_choices() -> Vec<Choice> {
    vec![
        Choice::new("linux", t!("assets.linuxHost")),
        Choice::new("windows", t!("assets.windowsHost")),
        Choice::new("vnc", t!("assets.vncHost")),
    ]
}

fn auth_choices(os: OsKind) -> Vec<Choice> {
    let mut v = vec![Choice::new("password", t!("assets.passwordAuth"))];
    if os == OsKind::Linux {
        v.push(Choice::new("key", t!("assets.keyAuth")));
    }
    v
}

impl HostDialog {
    /// Open the dialog on the current window. `editing = None` adds a host (optionally into `initial_group`).
    pub fn open(store: Entity<AssetsStore>,
        editing: Option<Host>,
        initial_group: Option<String>,
        window: &mut Window,
        cx: &mut App,) {
        let _ = Self::open_with_handle(store, editing, initial_group, window, cx);
    }

    pub fn open_with_handle(
        store: Entity<AssetsStore>,
        editing: Option<Host>,
        initial_group: Option<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<HostDialog> {
        let title: SharedString = if editing.is_some() { t!("assets.editHost") } else { t!("assets.addHost") }.into();
        let view = cx.new(|cx| HostDialog::new(store, editing, initial_group, window, cx));
        let shown = view.clone();
        window.open_dialog(cx, move |dialog, _, _| dialog.title(title.clone()).w(px(520.)).child(shown.clone()));
        let first = view.read(cx).inputs.name.clone();
        first.update(cx, |s, cx| s.focus(window, cx));
        view
    }

    fn new(
        store: Entity<AssetsStore>,
        editing: Option<Host>,
        initial_group: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let values = match &editing {
            Some(h) => HostFormValues::from_host(h),
            None => HostFormValues::new_default(initial_group.as_deref()),
        };
        let (groups, hosts, default_name) = {
            let s = store.read(cx);
            let name = if s.default_group_name.trim().is_empty() { t!("assets.defaultGroup") } else { s.default_group_name.clone() };
            (s.groups.clone(), s.hosts.clone(), name)
        };
        let group_options: Vec<Choice> = group_choices(&groups, None)
            .into_iter()
            .map(|(id, depth)| {
                let name = if id == DEFAULT_GROUP_ID {
                    default_name.clone()
                } else {
                    groups.iter().find(|g| g.id == id).map(|g| g.name.clone()).unwrap_or_default()
                };
                let indent = "　".repeat(depth);
                Choice::new(id, format!("{indent}{name}"))
            })
            .collect();
        let jump_candidates: Vec<(String, String)> = hosts
            .iter()
            .filter(|h| h.os == "linux" && HostKind::of(h) == HostKind::Linux && Some(&h.id) != editing.as_ref().map(|e| &e.id))
            .map(|h| (h.id.clone(), format!("{} ({})", h.name, h.ip)))
            .collect();

        let inputs = Inputs {
            name: text_input(window, cx, t!("assets.hostNamePlaceholder"), &values.name, false),
            ip: text_input(window, cx, t!("assets.hostAddressPlaceholder"), &values.ip, false),
            port: text_input(window, cx, "22".into(), &values.port, false),
            username: text_input(window, cx, "root".into(), &values.username, false),
            password: text_input(
                window,
                cx,
                if editing.is_some() { t!("assets.passwordKeepPlaceholder") } else { t!("assets.passwordPlaceholder") },
                "",
                true,
            ),
            private_key: cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("assets.pastePrivateKey")).auto_grow(2, 6)),
            remark: text_input(window, cx, t!("assets.remarkPlaceholder"), &values.remark, false),
            rdp_domain: text_input(window, cx, t!("assets.rdpDomainPlaceholder"), &values.rdp_domain, false),
            rdp_width: text_input(window, cx, "1280".into(), &values.rdp_width, false),
            rdp_height: text_input(window, cx, "720".into(), &values.rdp_height, false),
            rdp_disk_path: text_input(window, cx, t!("assets.rdpDiskPathPlaceholder"), &values.rdp_disk_path, false),
            vnc_port: text_input(window, cx, "5900".into(), &values.vnc_port, false),
            vnc_username: text_input(window, cx, t!("assets.vncUsernamePlaceholder"), &values.vnc_username, false),
            vnc_password: text_input(window, cx, t!("assets.vncPasswordPlaceholder"), &values.vnc_password, true),
            proxy_host: text_input(window, cx, t!("assets.proxyHostPlaceholder"), &values.proxy_host, false),
            proxy_port: text_input(window, cx, "1080".into(), &values.proxy_port, false),
            proxy_username: text_input(window, cx, t!("assets.proxyUsernamePlaceholder"), &values.proxy_username, false),
            proxy_password: text_input(window, cx, t!("assets.proxyPasswordPlaceholder"), &values.proxy_password, true),
        };
        let selects = Selects {
            os: cx.new(|cx| new_select(os_choices(), values.os.as_str(), window, cx)),
            auth: cx.new(|cx| new_select(auth_choices(values.os), if values.auth == AuthKind::Key { "key" } else { "password" }, window, cx)),
            group: cx.new(|cx| new_select(group_options, &values.group_id, window, cx)),
            vnc_auth: cx.new(|cx| {
                new_select(
                    vec![Choice::new("vnc", t!("assets.vncAuthMethodVnc")), Choice::new("ard", t!("assets.vncAuthMethodArd"))],
                    if values.vnc_auth == VncAuth::Ard { "ard" } else { "vnc" },
                    window,
                    cx,
                )
            }),
            proxy_type: cx.new(|cx| {
                new_select(
                    vec![Choice::new("socks5", "SOCKS5"), Choice::new("http", "HTTP")],
                    if values.proxy_type == ProxyKind::Http { "http" } else { "socks5" },
                    window,
                    cx,
                )
            }),
        };

        let subs = vec![
            cx.subscribe_in(&selects.os, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.change_os(OsKind::parse(&v), window, cx);
                }
            }),
            cx.subscribe_in(&selects.auth, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.values.auth = if v == "key" { AuthKind::Key } else { AuthKind::Password };
                    cx.notify();
                }
            }),
            cx.subscribe_in(&selects.group, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.values.group_id = v;
                }
                cx.notify();
            }),
            cx.subscribe_in(&selects.vnc_auth, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.values.vnc_auth = if v == "ard" { VncAuth::Ard } else { VncAuth::Vnc };
                }
                cx.notify();
            }),
            cx.subscribe_in(&selects.proxy_type, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    let kind = if v == "http" { ProxyKind::Http } else { ProxyKind::Socks5 };
                    this.values.proxy_port = this.inputs.proxy_port.read(cx).value().to_string();
                    this.values.change_proxy_type(kind);
                    let port = this.values.proxy_port.clone();
                    this.inputs.proxy_port.update(cx, |s, cx| s.set_value(port, window, cx));
                }
                cx.notify();
            }),
        ];

        Self { store, editing, values, jump_candidates, tab: Tab::Basic, inputs, selects, errors: Vec::new(), saving: false, error: None, _subs: subs }
    }

    /// Current values: toggles/selects from `self.values`, text from the input widgets.
    pub fn collect(&self, cx: &App) -> HostFormValues {
        let i = &self.inputs;
        let text = |e: &Entity<InputState>| e.read(cx).value().to_string();
        let mut v = self.values.clone();
        v.name = text(&i.name);
        v.ip = text(&i.ip);
        v.port = text(&i.port);
        v.username = text(&i.username);
        v.password = text(&i.password);
        v.private_key = i.private_key.read(cx).value().to_string();
        v.remark = text(&i.remark);
        v.rdp_domain = text(&i.rdp_domain);
        v.rdp_width = text(&i.rdp_width);
        v.rdp_height = text(&i.rdp_height);
        v.rdp_disk_path = text(&i.rdp_disk_path);
        v.vnc_port = text(&i.vnc_port);
        v.vnc_username = text(&i.vnc_username);
        v.vnc_password = text(&i.vnc_password);
        v.proxy_host = text(&i.proxy_host);
        v.proxy_port = text(&i.proxy_port);
        v.proxy_username = text(&i.proxy_username);
        v.proxy_password = text(&i.proxy_password);
        v
    }

    fn set_text(entity: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let value = value.to_string();
        entity.update(cx, |s, cx| s.set_value(value, window, cx));
    }

    pub fn change_os(&mut self, os: OsKind, window: &mut Window, cx: &mut Context<Self>) {
        let mut v = self.collect(cx);
        let back_to_basic = v.change_os(os);
        // Push the retargeted values back into the widgets.
        Self::set_text(&self.inputs.port, &v.port, window, cx);
        Self::set_text(&self.inputs.username, &v.username, window, cx);
        Self::set_text(&self.inputs.rdp_domain, &v.rdp_domain, window, cx);
        Self::set_text(&self.inputs.rdp_disk_path, &v.rdp_disk_path, window, cx);
        Self::set_text(&self.inputs.rdp_width, &v.rdp_width, window, cx);
        Self::set_text(&self.inputs.rdp_height, &v.rdp_height, window, cx);
        Self::set_text(&self.inputs.vnc_port, &v.vnc_port, window, cx);
        if os == OsKind::Vnc {
            Self::set_text(&self.inputs.password, "", window, cx);
        }
        if os != OsKind::Linux {
            self.inputs.private_key.update(cx, |s, cx| s.set_value("", window, cx));
        }
        // Authentication choices depend on the system (Windows/VNC only allow a password).
        let auth_value = if v.auth == AuthKind::Key { "key" } else { "password" };
        self.selects.auth.update(cx, |s, cx| {
            s.set_items(auth_choices(os), window, cx);
            s.set_selected_value(&auth_value.to_string(), window, cx);
        });
        self.values = v;
        if back_to_basic {
            self.tab = Tab::Basic;
        }
        cx.notify();
    }

    fn error_for(&self, f: Field) -> Option<SharedString> {
        self.errors.contains(&f).then(|| t!("common.required").into())
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn errors(&self) -> &[Field] {
        &self.errors
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn values(&self) -> &HostFormValues {
        &self.values
    }

    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let v = self.collect(cx);
        self.errors = v.validate();
        if !self.errors.is_empty() {
            self.tab = Tab::Basic;
            cx.notify();
            return;
        }
        self.saving = true;
        self.error = None;
        cx.notify();
        let editing = self.editing.clone();
        let task = self.store.update(cx, |s, cx| match &editing {
            Some(h) => {
                let t = s.update_host(v.to_update_host(&h.id, &h.tags), cx);
                cx.spawn(async move |_, _| t.await.map(|_| ()))
            }
            None => {
                let t = s.add_host(v.to_new_host(), cx);
                cx.spawn(async move |_, _| t.await.map(|_| ()))
            }
        });
        let editing = self.editing.is_some();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        toast::success(window, cx, if editing { t!("assets.hostUpdated") } else { t!("assets.hostAdded") });
                        window.close_dialog(cx);
                    }
                    Err(e) => {
                        this.error = Some(t!("common.operationFailed", error = e));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    // ---- rendering ----

    /// `.ui-tabs-trigger`: 34px high, 6/12 padding, 2px underline in the primary colour when active.
    fn tab_button(&self, id: &'static str, label: String, tab: Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui(cx);
        let active = self.tab == tab;
        div()
            .id(id)
            .min_h(px(34.))
            .px(px(12.))
            .py(px(6.))
            .flex()
            .items_center()
            .cursor_pointer()
            .text_size(px(13.))
            .border_b_2()
            .border_color(if active { ui.primary } else { gpui_kit::transparent_black() })
            .text_color(if active { ui.primary } else { ui.text_secondary })
            .when(!active, |d| d.hover(move |d| d.text_color(ui.text).bg(ui.text.opacity(0.06))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.tab = tab;
                cx.notify();
            }))
            .child(label)
            .test_support()
    }

    fn basic_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let os = self.values.os;
        let i = &self.inputs;
        let auth_row = (os != OsKind::Vnc).then(|| {
            let key_auth = os == OsKind::Linux && self.values.auth == AuthKind::Key;
            h_flex()
                .gap(px(10.))
                .items_start()
                .child(field(t!("assets.auth"), Select::new(&self.selects.auth), None, cx).w(px(88.)).flex_none())
                .child(field(t!("assets.username"), Input::new(&i.username), self.error_for(Field::Username), cx).flex_1())
                .child(if key_auth {
                    field(t!("assets.sshPrivateKey"), Textarea::new(&i.private_key), None, cx).flex_1()
                } else {
                    field(t!("assets.password"), Input::new(&i.password), None, cx).flex_1()
                })
        });
        let vnc = (os == OsKind::Vnc).then(|| {
            v_flex()
                .gap_2()
                .child(section(t!("assets.vncSettings"), t!("assets.vncSettingsExtra"), cx))
                .child(
                    h_flex()
                        .gap(px(10.))
                        .items_start()
                        .child(field(t!("assets.vncPort"), crate::widgets::form::spin(Input::new(&i.vnc_port), "host-vnc-port", &i.vnc_port, 1, Some(65535), crate::theme::ui(cx)), None, cx).w(px(96.)).flex_none())
                        .child(field(t!("assets.vncAuthMethod"), Select::new(&self.selects.vnc_auth), None, cx).flex_1())
                        .child(field(t!("assets.vncUsername"), Input::new(&i.vnc_username), None, cx).flex_1())
                        .child(field(t!("assets.vncPassword"), Input::new(&i.vnc_password), None, cx).flex_1()),
                )
                .child(
                    h_flex()
                        .gap_4()
                        .child(switch_row(
                            "vnc-view-only",
                            self.values.vnc_view_only,
                            t!("assets.vncViewOnly"),
                            t!("assets.vncViewOnlyExtra"),
                            cx.listener_for_bool(|this, v| this.values.vnc_view_only = v),
                            cx,
                        ))
                        .child(switch_row(
                            "vnc-shared",
                            self.values.vnc_shared,
                            t!("assets.vncShared"),
                            t!("assets.vncSharedExtra"),
                            cx.listener_for_bool(|this, v| this.values.vnc_shared = v),
                            cx,
                        )),
                )
        });
        v_flex()
            .gap(px(10.))
            .child(
                h_flex()
                    .gap(px(10.))
                    .items_start()
                    .child(field(t!("assets.hostName"), Input::new(&i.name), self.error_for(Field::Name), cx).flex_1())
                    .child(field(t!("assets.system"), Select::new(&self.selects.os), None, cx).w(px(132.)).flex_none()),
            )
            .child(
                h_flex()
                    .gap(px(10.))
                    .items_start()
                    .child(field(t!("assets.hostAddress"), Input::new(&i.ip), self.error_for(Field::Ip), cx).flex_1())
                    .child(field(t!("assets.port"), crate::widgets::form::spin(Input::new(&i.port), "host-port", &i.port, 1, Some(65535), crate::theme::ui(cx)), self.error_for(Field::Port), cx).w(px(78.)).flex_none()),
            )
            .children(auth_row)
            .child(
                h_flex()
                    .gap(px(10.))
                    .items_start()
                    .child(field(t!("assets.group"), Select::new(&self.selects.group).icon(gpui_kit::component::Icon::new(gpui_kit::assets::IconName::ChevronDown)), None, cx).flex_1())
                    .child(field(t!("assets.remark"), Input::new(&i.remark), None, cx).flex_1()),
            )
            .children(vnc)
    }

    fn proxy_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let i = &self.inputs;
        v_flex()
            .gap_2()
            .child(section(t!("assets.proxySettings"), t!("assets.proxySettingsExtra"), cx))
            .child(switch_row(
                "proxy-enabled",
                self.values.proxy_enabled,
                t!("assets.proxyEnabled"),
                t!("assets.proxyEnabledExtra"),
                cx.listener_for_bool(|this, v| this.values.proxy_enabled = v),
                cx,
            ))
            .when(self.values.proxy_enabled, |d| {
                d.child(
                    h_flex()
                        .gap(px(10.))
                        .items_start()
                        .child(field(t!("assets.proxyType"), Select::new(&self.selects.proxy_type), None, cx).w(px(128.)).flex_none())
                        .child(field(t!("assets.proxyHost"), Input::new(&i.proxy_host), None, cx).flex_1())
                        .child(field(t!("assets.proxyPort"), crate::widgets::form::spin(Input::new(&i.proxy_port), "host-proxy-port", &i.proxy_port, 1, Some(65535), crate::theme::ui(cx)), None, cx).w(px(96.)).flex_none()),
                )
                .child(
                    h_flex()
                        .gap(px(10.))
                        .items_start()
                        .child(field(t!("assets.proxyUsername"), Input::new(&i.proxy_username), None, cx).flex_1())
                        .child(field(t!("assets.proxyPassword"), Input::new(&i.proxy_password), None, cx).flex_1()),
                )
            })
    }

    fn advanced_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let i = &self.inputs;
        match self.values.os {
            OsKind::Linux => {
                let chain = self.values.jump_chain.clone();
                let boxes: Vec<_> = self
                    .jump_candidates
                    .iter()
                    .enumerate()
                    .map(|(n, (id, label))| {
                        let id2 = id.clone();
                        Checkbox::new(("jump", n))
                            .label(label.clone())
                            .checked(chain.contains(id))
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.values.jump_chain.retain(|x| x != &id2);
                                if *checked {
                                    this.values.jump_chain.push(id2.clone());
                                }
                                cx.notify();
                            }))
                    })
                    .collect();
                v_flex()
                    .gap_3()
                    .child(field(
                        t!("assets.jumpChain"),
                        v_flex().gap_1().when(boxes.is_empty(), |d| d.child(div().text_size(px(12.)).child(t!("assets.directNoJump")))).children(boxes),
                        None,
                        cx,
                    ))
                    .child(self.proxy_block(cx))
                    .into_any_element()
            }
            OsKind::Windows => v_flex()
                .gap_3()
                .child(section(t!("assets.rdpAdvanced"), t!("assets.rdpAdvancedExtra"), cx))
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(field(t!("assets.rdpDomain"), Input::new(&i.rdp_domain), None, cx).flex_1())
                        .child(field(t!("assets.rdpDesktopWidth"), crate::widgets::form::spin(Input::new(&i.rdp_width), "host-rdp-width", &i.rdp_width, 640, Some(3840), crate::theme::ui(cx)), None, cx).w(px(110.)))
                        .child(field(t!("assets.rdpDesktopHeight"), crate::widgets::form::spin(Input::new(&i.rdp_height), "host-rdp-height", &i.rdp_height, 480, Some(2160), crate::theme::ui(cx)), None, cx).w(px(110.))),
                )
                .child(switch_row(
                    "rdp-clipboard",
                    self.values.rdp_clipboard,
                    t!("assets.rdpEnableClipboard"),
                    t!("assets.rdpEnableClipboardExtra"),
                    cx.listener_for_bool(|this, v| this.values.rdp_clipboard = v),
                    cx,
                ))
                .child(switch_row(
                    "rdp-audio",
                    self.values.rdp_audio,
                    t!("assets.rdpEnableAudio"),
                    t!("assets.rdpEnableAudioExtra"),
                    cx.listener_for_bool(|this, v| this.values.rdp_audio = v),
                    cx,
                ))
                .child(switch_row(
                    "rdp-map-disk",
                    self.values.rdp_map_disk,
                    t!("assets.rdpMapDisk"),
                    t!("assets.rdpMapDiskExtra"),
                    cx.listener_for_bool(|this, v| this.values.rdp_map_disk = v),
                    cx,
                ))
                .when(self.values.rdp_map_disk, |d| d.child(field(t!("assets.rdpDiskPath"), Input::new(&i.rdp_disk_path), None, cx)))
                .child(self.proxy_block(cx))
                .into_any_element(),
            OsKind::Vnc => self.proxy_block(cx).into_any_element(),
        }
    }
}

/// `cx.listener`-style adapter for boolean controls: updates `this` then repaints.
trait ListenerForBool {
    fn listener_for_bool(&self, f: impl Fn(&mut HostDialog, bool) + 'static) -> Box<dyn Fn(bool, &mut Window, &mut App)>;
}

impl ListenerForBool for Context<'_, HostDialog> {
    fn listener_for_bool(&self, f: impl Fn(&mut HostDialog, bool) + 'static) -> Box<dyn Fn(bool, &mut Window, &mut App)> {
        let weak = self.entity().downgrade();
        Box::new(move |value, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                f(this, value);
                cx.notify();
            });
        })
    }
}

impl Render for HostDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let advanced_label = match self.values.os {
            OsKind::Linux => t!("assets.linuxAdvancedTab"),
            OsKind::Windows => t!("assets.rdpAdvancedTab"),
            OsKind::Vnc => t!("assets.vncAdvancedTab"),
        };
        let body = match self.tab {
            Tab::Basic => self.basic_tab(cx).into_any_element(),
            Tab::Advanced => self.advanced_tab(cx),
        };
        let saving = self.saving;
        v_flex()
            .id("host-dialog")
            .test_support()
            .gap_3()
            .child(
                h_flex()
                    .gap(px(4.))
                    .items_end()
                    .border_b_1()
                    .border_color(crate::theme::ui(cx).border)
                    .child(self.tab_button("host-tab-basic", t!("assets.hostBasicInfo"), Tab::Basic, cx))
                    .child(self.tab_button("host-tab-advanced", advanced_label, Tab::Advanced, cx)),
            )
            .child(div().id("host-dialog-body").max_h(px(460.)).overflow_y_scroll().pr_1().child(body))
            .when_some(self.error.clone(), |d, e| d.child(div().text_size(px(12.)).text_color(theme.danger).child(e)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("host-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("host-save")
                            .primary()
                            .label(t!("common.ok"))
                            .loading(saving)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}

#[allow(dead_code)]
fn _unused(_: HashMap<(), ()>) {}
