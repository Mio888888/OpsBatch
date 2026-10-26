//! Cloud import: pick provider + region, list instances, import the selected ones as hosts
//! (port of CloudImport.tsx).

use std::collections::BTreeSet;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use crate::scale::px;
use ops_i18n::t;
use ops_library::cloud::{CloudInstance, CloudProvider};

use crate::stores::assets::AssetsStore;
use crate::widgets::choice::{confirmed_value, new_select, Choice, ChoiceSelect};
use crate::widgets::toast;

pub struct ProviderInfo {
    pub value: &'static str,
    pub default_region: &'static str,
    pub regions: &'static [&'static str],
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo { value: "aliyun", default_region: "cn-hangzhou", regions: &["cn-hangzhou", "cn-shanghai", "cn-beijing", "cn-shenzhen", "cn-chengdu"] },
    ProviderInfo { value: "aws", default_region: "us-east-1", regions: &["us-east-1", "us-west-2", "ap-southeast-1", "ap-northeast-1", "eu-west-1"] },
    ProviderInfo { value: "tencent", default_region: "ap-guangzhou", regions: &["ap-guangzhou", "ap-shanghai", "ap-beijing", "ap-chengdu", "ap-hongkong"] },
];

pub fn provider_label(value: &str) -> String {
    match value {
        "aliyun" => t!("cloudImport.provider.aliyun"),
        "tencent" => t!("cloudImport.provider.tencent"),
        _ => "AWS".to_string(),
    }
}

fn provider_info(value: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.value == value)
}

fn provider_choices() -> Vec<Choice> {
    PROVIDERS.iter().map(|p| Choice::new(p.value, provider_label(p.value))).collect()
}

fn region_choices(provider: &str) -> Vec<Choice> {
    provider_info(provider).map(|p| p.regions.iter().map(|r| Choice::new(*r, *r)).collect()).unwrap_or_default()
}

pub struct CloudImportDialog {
    store: Entity<AssetsStore>,
    providers: Vec<CloudProvider>,
    provider_select: Entity<ChoiceSelect>,
    region_select: Entity<ChoiceSelect>,
    provider: Option<String>,
    region: String,
    instances: Vec<CloudInstance>,
    selected: BTreeSet<String>,
    fetching: bool,
    importing: bool,
    _subs: Vec<Subscription>,
}

impl CloudImportDialog {
    pub fn open(store: Entity<AssetsStore>, window: &mut Window, cx: &mut App) {
        let _ = Self::open_with_handle(store, window, cx);
    }

    pub fn open_with_handle(store: Entity<AssetsStore>, window: &mut Window, cx: &mut App) -> Entity<CloudImportDialog> {
        let view = cx.new(|cx| CloudImportDialog::new(store, window, cx));
        let title: SharedString = t!("cloudImport.title").into();
        let shown = view.clone();
        window.open_dialog(cx, move |dialog, _, _| dialog.title(h_flex().gap(px(6.)).items_center().child(Icon::new(IconName::Server).size_4()).child(title.clone())).w(px(900.)).child(shown.clone()));
        // like Radix, the first control (the provider select) has the focus when the dialog opens
        let first = view.read(cx).provider_select.clone();
        first.update(cx, |s, cx| s.focus(window, cx));
        view
    }

    fn new(store: Entity<AssetsStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let provider_select = cx.new(|cx| new_select(provider_choices(), "", window, cx));
        let region_select = cx.new(|cx| new_select(Vec::new(), "", window, cx));
        let subs = vec![
            cx.subscribe_in(&provider_select, window, |this, _, ev, window, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.choose_provider(&v, window, cx);
                }
            }),
            cx.subscribe_in(&region_select, window, |this, _, ev, _, cx| {
                if let Some(v) = confirmed_value(ev) {
                    this.region = v;
                    cx.notify();
                }
            }),
        ];
        let mut this = Self {
            store,
            providers: Vec::new(),
            provider_select,
            region_select,
            provider: None,
            region: String::new(),
            instances: Vec::new(),
            selected: BTreeSet::new(),
            fetching: false,
            importing: false,
            _subs: subs,
        };
        this.load_providers(cx);
        this
    }

    fn load_providers(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        cx.spawn(async move |this, cx| {
            let list = ops_core::rt::run(async move { ops_library::cloud::list_cloud_providers(&ctx.db) }).await.ok().and_then(|r| r.ok());
            let _ = this.update(cx, |this, cx| {
                this.providers = list.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    /// Show a fixed list (offline preview / tests) instead of fetching from a cloud API.
    pub fn set_instances(&mut self, list: Vec<CloudInstance>, cx: &mut Context<Self>) {
        self.instances = list;
        self.selected.clear();
        cx.notify();
    }

    pub fn providers(&self) -> &[CloudProvider] {
        &self.providers
    }

    pub fn selected_ids(&self) -> &BTreeSet<String> {
        &self.selected
    }

    pub fn instances(&self) -> &[CloudInstance] {
        &self.instances
    }

    fn choose_provider(&mut self, provider: &str, window: &mut Window, cx: &mut Context<Self>) {
        let info = provider_info(provider);
        self.provider = Some(provider.to_string());
        self.region = info.map(|p| p.default_region.to_string()).unwrap_or_default();
        self.instances.clear();
        self.selected.clear();
        let region = self.region.clone();
        self.region_select.update(cx, |s, cx| {
            s.set_items(region_choices(provider), window, cx);
            s.set_selected_value(&region, window, cx);
        });
        cx.notify();
    }

    fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(provider), false) = (self.provider.clone(), self.region.is_empty()) else {
            toast::warning(window, cx, t!("cloudImport.selectProviderAndRegion"));
            return;
        };
        let region = self.region.clone();
        let ctx = crate::services::ctx(cx);
        self.fetching = true;
        self.instances.clear();
        self.selected.clear();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move {
                ops_core::rt::spawn_blocking(move || ops_library::cloud::fetch_cloud_instances(&ctx.db, provider, region))
                    .await
                    .map_err(|e| e.to_string())?
            })
            .await
            .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.fetching = false;
                match result {
                    Ok(list) if list.is_empty() => toast::warning(window, cx, t!("cloudImport.noInstances")),
                    Ok(list) => {
                        toast::success(window, cx, t!("cloudImport.foundInstances", count = list.len()));
                        this.instances = list;
                    }
                    Err(e) => toast::error(window, cx, t!("cloudImport.fetchFailed", error = e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            toast::warning(window, cx, t!("cloudImport.selectInstances"));
            return;
        }
        let chosen: Vec<CloudInstance> = self.instances.iter().filter(|i| self.selected.contains(&i.instance_id)).cloned().collect();
        let ctx = crate::services::ctx(cx);
        let store = self.store.clone();
        self.importing = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = ops_core::rt::run(async move { ops_library::cloud::import_cloud_instances(&ctx.db, chosen, None) })
                .await
                .and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.importing = false;
                match result {
                    Ok(count) => {
                        toast::success(window, cx, t!("cloudImport.importSuccess", count = count));
                        store.update(cx, |s, cx| s.reload(cx));
                        window.close_dialog(cx);
                    }
                    Err(e) => {
                        toast::error(window, cx, t!("cloudImport.importFailed", error = e));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn open_credentials(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.provider.as_ref().and_then(|p| self.providers.iter().find(|x| &x.provider == p)).cloned();
        let initial_provider = self.provider.clone().unwrap_or_else(|| "aliyun".to_string());
        let parent = cx.entity();
        let dialog = cx.new(|cx| CredentialsDialog::new(parent, initial_provider, existing, window, cx));
        let title: SharedString = t!("cloudImport.configTitle").into();
        window.open_dialog(cx, move |d, _, _| d.title(title.clone()).w(px(450.)).child(dialog.clone()));
    }

    fn credentials_saved(&mut self, providers: Vec<CloudProvider>, cx: &mut Context<Self>) {
        self.providers = providers;
        cx.notify();
    }
}

impl Render for CloudImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let configured = self.provider.as_ref().map(|p| self.providers.iter().any(|x| &x.provider == p)).unwrap_or(true);
        let all_selected = !self.instances.is_empty() && self.selected.len() == self.instances.len();
        let count = self.selected.len();
        let rows: Vec<_> = self
            .instances
            .iter()
            .enumerate()
            .map(|(n, inst)| {
                let id = inst.instance_id.clone();
                let running = inst.status.eq_ignore_ascii_case("running");
                h_flex()
                    .id(("cloud-row", n))
                    .test_support()
                    .h(px(32.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Checkbox::new(("cloud-check", n)).checked(self.selected.contains(&inst.instance_id)).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            if *checked {
                                this.selected.insert(id.clone());
                            } else {
                                this.selected.remove(&id);
                            }
                            cx.notify();
                        })),
                    )
                    .child(div().w(px(160.)).truncate().text_size(px(12.)).child(inst.instance_id.clone()))
                    .child(div().w(px(140.)).truncate().text_size(px(12.)).child(inst.name.clone()))
                    .child(div().w(px(140.)).text_size(px(12.)).child(inst.ip.clone()))
                    .child(div().w(px(140.)).text_size(px(12.)).child(inst.inner_ip.clone()))
                    .child(div().w(px(100.)).truncate().text_size(px(12.)).child(inst.instance_type.clone()))
                    .child(
                        div()
                            .px_1p5()
                            .rounded(px(4.))
                            .text_size(px(11.))
                            .bg(if running { theme.success.opacity(0.18) } else { theme.muted })
                            .text_color(if running { theme.success } else { theme.muted_foreground })
                            .child(inst.status.clone()),
                    )
            })
            .collect();

        v_flex()
            .id("cloud-dialog")
            .test_support()
            .gap_3()
            .child({
                // `<Card size="small">`: 12px padding, 10px corners; two 140/160px selects, the fetch button and, on the right, the credentials button
                let ui = crate::theme::ui(cx);
                v_flex()
                    .p(px(12.))
                    .bg(ui.card)
                    .border_1()
                    .border_color(ui.border)
                    .rounded(px(10.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .items_center()
                                    .child(div().w(px(140.)).child(Select::new(&self.provider_select).placeholder(t!("cloudImport.selectProvider"))))
                                    .child(div().w(px(160.)).child(Select::new(&self.region_select).placeholder(t!("cloudImport.selectRegion"))))
                                    .child(
                                        Button::new("cloud-fetch")
                                            .primary()
                                            .icon(Icon::new(IconName::Server))
                                            .label(t!("cloudImport.fetchInstances"))
                                            .loading(self.fetching)
                                            .disabled(self.provider.is_none() || self.region.is_empty())
                                            .on_click(cx.listener(|this, _, window, cx| this.fetch(window, cx))),
                                    ),
                            )
                            .child(
                                Button::new("cloud-credentials")
                                    .label(t!("cloudImport.configureCredentials"))
                                    .on_click(cx.listener(|this, _, window, cx| this.open_credentials(window, cx))),
                            ),
                    )
            })
            .when(!configured, |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.warning)
                        .child(t!("cloudImport.configureProviderFirst", name = provider_label(self.provider.as_deref().unwrap_or("")))),
                )
            })
            .child(
                v_flex()
                    .child(
                        h_flex()
                            .h(px(32.))
                            .px_2()
                            .gap_2()
                            .items_center()
                            .bg(theme.table_head)
                            .child(Checkbox::new("cloud-select-all").checked(all_selected).on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.selected = if *checked { this.instances.iter().map(|i| i.instance_id.clone()).collect() } else { BTreeSet::new() };
                                cx.notify();
                            })))
                            .child(div().w(px(160.)).text_size(px(12.)).child(t!("cloudImport.instanceId")))
                            .child(div().w(px(140.)).text_size(px(12.)).child(t!("cloudImport.instanceName")))
                            .child(div().w(px(140.)).text_size(px(12.)).child(t!("cloudImport.publicIp")))
                            .child(div().w(px(140.)).text_size(px(12.)).child(t!("cloudImport.innerIp")))
                            .child(div().w(px(100.)).text_size(px(12.)).child(t!("cloudImport.instanceType")))
                            .child(div().text_size(px(12.)).child(t!("cloudImport.instanceStatus"))),
                    )
                    .child(
                        div()
                            .id("cloud-rows")
                            .h(px(if rows.is_empty() { 140. } else { 320. }))
                            .overflow_y_scroll()
                            .when(rows.is_empty(), |d| {
                                d.child(
                                    v_flex()
                                        .h_full()
                                        .items_center()
                                        .justify_center()
                                        .gap(px(8.))
                                        .text_size(px(12.))
                                        .text_color(theme.muted_foreground)
                                        .when(!self.fetching, |d| d.child(div().text_size(px(28.)).child("∅")))
                                        .child(if self.fetching { t!("common.loadingShort") } else { t!("cloudImport.clickToFetch") }),
                                )
                            })
                            .children(rows),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("cloud-cancel").label(t!("common.cancel")).on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(
                        Button::new("cloud-import")
                            .primary()
                            .label(t!("cloudImport.importCount", count = count))
                            .loading(self.importing)
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, window, cx| this.import(window, cx))),
                    ),
            )
    }
}

/// Access-key editor for one provider (nested dialog).
pub struct CredentialsDialog {
    parent: Entity<CloudImportDialog>,
    provider_select: Entity<ChoiceSelect>,
    provider: String,
    key_id: Entity<InputState>,
    key_secret: Entity<InputState>,
    existing: Vec<CloudProvider>,
    _subs: Vec<Subscription>,
}

impl CredentialsDialog {
    fn new(parent: Entity<CloudImportDialog>, provider: String, existing_for_provider: Option<CloudProvider>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let existing = parent.read(cx).providers.clone();
        let provider_select = cx.new(|cx| new_select(provider_choices(), &provider, window, cx));
        let (id, secret) = existing_for_provider.map(|p| (p.access_key_id, p.access_key_secret)).unwrap_or_default();
        let key_id = cx.new(|cx| InputState::new(window, cx).placeholder("Access Key ID").masked(true).default_value(id));
        let key_secret = cx.new(|cx| InputState::new(window, cx).placeholder("Access Key Secret").masked(true).default_value(secret));
        let subs = vec![cx.subscribe_in(&provider_select, window, |this, _, ev, window, cx| {
            if let Some(v) = confirmed_value(ev) {
                // Switching provider shows that provider's stored keys (or blanks).
                let stored = this.existing.iter().find(|p| p.provider == v).cloned();
                let (id, secret) = stored.map(|p| (p.access_key_id, p.access_key_secret)).unwrap_or_default();
                this.key_id.update(cx, |s, cx| s.set_value(id, window, cx));
                this.key_secret.update(cx, |s, cx| s.set_value(secret, window, cx));
                this.provider = v;
            }
        })];
        Self { parent, provider_select, provider, key_id, key_secret, existing, _subs: subs }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.key_id.read(cx).value().to_string();
        let secret = self.key_secret.read(cx).value().to_string();
        if id.is_empty() || secret.is_empty() {
            toast::warning(window, cx, t!("cloudImport.fillAccessKey"));
            return;
        }
        let regions = provider_info(&self.provider).map(|p| p.regions.iter().map(|r| r.to_string()).collect()).unwrap_or_default();
        let entry = CloudProvider {
            provider: self.provider.clone(),
            name: provider_label(&self.provider),
            access_key_id: id,
            access_key_secret: secret,
            regions,
        };
        let mut updated: Vec<CloudProvider> = self.existing.iter().filter(|p| p.provider != self.provider).cloned().collect();
        updated.push(entry);
        let ctx = crate::services::ctx(cx);
        let to_save = updated.clone();
        let parent = self.parent.clone();
        cx.spawn_in(window, async move |_, cx| {
            let result = ops_core::rt::run(async move { ops_library::cloud::save_cloud_providers(&ctx.db, to_save) }).await.and_then(|r| r);
            let _ = cx.update(|window, cx| match result {
                Ok(()) => {
                    toast::success(window, cx, t!("cloudImport.credentialSaved"));
                    parent.update(cx, |p, cx| p.credentials_saved(updated.clone(), cx));
                    window.close_dialog(cx);
                }
                Err(e) => toast::error(window, cx, e),
            });
        })
        .detach();
    }
}

impl Render for CredentialsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .id("cloud-credentials-dialog")
            .test_support()
            .gap_3()
            .child(Select::new(&self.provider_select))
            .child(Input::new(&self.key_id).mask_toggle())
            .child(Input::new(&self.key_secret).mask_toggle())
            .child(div().text_size(px(12.)).text_color(theme.muted_foreground).child(t!("cloudImport.credentialHint")))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(Button::new("cred-cancel").label(t!("common.cancel")).on_click(|_, window, cx| window.close_dialog(cx)))
                    .child(Button::new("cred-save").primary().label(t!("common.ok")).on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `use super::*` pulls in GPUI's `test` macro (gpui_kit::*), so name the std one explicitly.
    #[::core::prelude::v1::test]
    fn provider_table_matches_the_tauri_build() {
        assert_eq!(PROVIDERS.len(), 3);
        for p in PROVIDERS {
            assert!(p.regions.contains(&p.default_region), "{}", p.value);
            assert_eq!(p.regions.len(), 5);
        }
        assert_eq!(provider_info("aliyun").unwrap().default_region, "cn-hangzhou");
        assert!(provider_info("gcp").is_none());
        assert_eq!(region_choices("aws").len(), 5);
        assert!(region_choices("nope").is_empty());
    }
}
