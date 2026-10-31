//! Command / script / Docker panels shown under a terminal. Port of CommandsInlinePanel, ScriptsInlinePanel and
//! DockerInlinePanel: pick an entry, fill its parameters, and put the command on the terminal's prompt.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use crate::scale::px;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Icon, Selectable as _, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ops_i18n::t;
use ops_library::params::{effective_command, fetch_remote_script, RemoteScriptParam};

use super::model::{CommandEntry, Kind, Param, Risk, ScriptEntry};
use super::shell_hl::{tokenize, Tok};
use super::store::{library_store, LibraryStore};
use super::ui::*;
use crate::widgets::form::field;
use crate::widgets::toast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    Commands,
    Scripts,
    Docker,
}

/// Called with the final command text to put on the terminal prompt.
pub type InsertFn = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// One list row, independent of whether it came from a command or a script.
#[derive(Clone)]
struct Row {
    id: String,
    name: String,
    description: String,
    code: String,
    risk: Risk,
    remote: bool,
    has_params: bool,
    starred: bool,
    language: Option<String>,
    category: String,
}

pub struct InlinePanel {
    kind: InlineKind,
    store: Entity<LibraryStore>,
    search: Entity<InputState>,
    category: Option<String>,
    insert: InsertFn,
    _subs: Vec<Subscription>,
}

impl InlinePanel {
    pub fn new(
        kind: InlineKind,
        insert: InsertFn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = library_store(cx);
        let placeholder = if kind == InlineKind::Scripts {
            t!("inlinePanel.searchScripts")
        } else {
            t!("inlinePanel.searchCommands")
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&search, |_, _, cx| cx.notify()),
        ];
        store.update(cx, |s, cx| {
            if kind == InlineKind::Scripts {
                s.load_scripts(cx)
            } else {
                s.load_commands(cx)
            }
        });
        Self {
            kind,
            store,
            search,
            category: None,
            insert,
            _subs: subs,
        }
    }

    pub fn kind(&self) -> InlineKind {
        self.kind
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let q = self.search.read(cx).value().trim().to_lowercase();
        let has = |hay: &str| hay.to_lowercase().contains(&q);
        let s = self.store.read(cx);
        match self.kind {
            InlineKind::Commands | InlineKind::Docker => s
                .commands
                .iter()
                .filter(|c| self.kind == InlineKind::Commands || c.kind == Kind::Docker)
                .filter(|c| {
                    q.is_empty()
                        || has(&c.name)
                        || has(&c.command)
                        || has(&c.description)
                        || (self.kind == InlineKind::Docker && c.tags.iter().any(|t| has(t)))
                })
                .map(|c| Row {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    description: c.description.clone(),
                    code: c.display_command(),
                    risk: c.risk,
                    remote: !c.url.is_empty(),
                    has_params: !c.parameters.is_empty(),
                    starred: c.starred,
                    language: None,
                    category: c.category.clone(),
                })
                .collect(),
            InlineKind::Scripts => s
                .scripts
                .iter()
                .filter(|c| q.is_empty() || has(&c.name) || has(&c.description) || has(&c.content))
                .map(|c| {
                    let text = c.display_text();
                    Row {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        description: c.description.clone(),
                        code: if c.url.is_empty() && text.chars().count() > 120 {
                            text.chars().take(120).collect::<String>() + "…"
                        } else {
                            text
                        },
                        risk: c.risk,
                        remote: !c.url.is_empty(),
                        has_params: !c.parameters.is_empty(),
                        starred: c.starred,
                        language: Some(c.language.clone()),
                        category: c.category.clone(),
                    }
                })
                .collect(),
        }
    }

    fn categories(&self, cx: &App) -> Vec<String> {
        let s = self.store.read(cx);
        let set: BTreeSet<String> = match self.kind {
            InlineKind::Commands => s.commands.iter().map(|c| c.category.clone()).collect(),
            InlineKind::Docker => s
                .commands
                .iter()
                .filter(|c| c.kind == Kind::Docker)
                .map(|c| c.category.clone())
                .filter(|c| !c.trim().is_empty())
                .collect(),
            InlineKind::Scripts => s.scripts.iter().map(|c| c.category.clone()).collect(),
        };
        set.into_iter().collect()
    }

    pub fn set_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.search
            .update(cx, |s, cx| s.set_value(text, window, cx));
    }

    pub fn run(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<RunDialog>> {
        let (title, risk, description, url, params, base, language) = {
            let s = self.store.read(cx);
            match self.kind {
                InlineKind::Scripts => {
                    let c: &ScriptEntry = s.scripts.iter().find(|c| c.id == id)?;
                    (
                        c.name.clone(),
                        c.risk,
                        c.description.clone(),
                        c.url.clone(),
                        c.parameters.clone(),
                        c.content.clone(),
                        true,
                    )
                }
                _ => {
                    let c: &CommandEntry = s.commands.iter().find(|c| c.id == id)?;
                    // Docker entries without parameters go straight to the prompt.
                    if self.kind == InlineKind::Docker && c.parameters.is_empty() {
                        let text = c.display_command();
                        (self.insert)(text, window, cx);
                        toast::success(window, cx, t!("docker.commandInserted"));
                        return None;
                    }
                    (
                        c.name.clone(),
                        c.risk,
                        c.description.clone(),
                        c.url.clone(),
                        c.parameters.clone(),
                        c.command.clone(),
                        false,
                    )
                }
            }
        };
        // Scripts keep their content verbatim (no ${PARAM} substitution), commands substitute.
        let substitute = !language;
        let insert = self.insert.clone();
        let docker = self.kind == InlineKind::Docker;
        let on_confirm: InsertFn = Rc::new(move |text, window, cx| {
            insert(text, window, cx);
            if docker {
                toast::success(window, cx, t!("docker.commandInserted"));
            }
        });
        let view = cx.new(|cx| {
            RunDialog::new(
                RunSpec {
                    title: title.clone(),
                    risk,
                    description,
                    url,
                    params,
                    base,
                    substitute,
                },
                on_confirm,
                window,
                cx,
            )
        });
        let shown = view.clone();
        window.open_dialog(cx, move |d, _, _| {
            d.title(title.clone()).w(px(560.)).child(shown.clone())
        });
        Some(view)
    }

    fn toggle_star(&mut self, id: String, cx: &mut Context<Self>) {
        let kind = self.kind;
        let task = self.store.update(cx, |s, cx| {
            if kind == InlineKind::Scripts {
                s.toggle_star_script(id, cx)
            } else {
                s.toggle_star_command(id, cx)
            }
        });
        task.detach();
    }
}

impl Render for InlinePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows = self.rows(cx);
        let categories = self.categories(cx);
        let kind = self.kind;
        let id_prefix = match kind {
            InlineKind::Commands => "inline-cmd",
            InlineKind::Scripts => "inline-script",
            InlineKind::Docker => "inline-docker",
        };
        let empty = if kind == InlineKind::Scripts {
            t!("inlinePanel.noScripts")
        } else {
            t!("inlinePanel.noCommands")
        };
        let weak = cx.entity().downgrade();

        v_flex()
            .id(id_prefix)
            .test_support()
            .size_full()
            .child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Input::new(&self.search)
                            .prefix(Icon::new(IconName::Search).size_3p5())
                            .cleanable(true)
                            .small(),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                Button::new((id_prefix, 0usize))
                                    .ghost()
                                    .xsmall()
                                    .label(t!("inlinePanel.all"))
                                    .selected(self.category.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.category = None;
                                        cx.notify();
                                    })),
                            )
                            .children(categories.into_iter().enumerate().map(|(i, cat)| {
                                let c2 = cat.clone();
                                Button::new((id_prefix, i + 1))
                                    .ghost()
                                    .xsmall()
                                    .label(cat.clone())
                                    .selected(self.category.as_deref() == Some(cat.as_str()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.category =
                                            if this.category.as_deref() == Some(c2.as_str()) {
                                                None
                                            } else {
                                                Some(c2.clone())
                                            };
                                        cx.notify();
                                    }))
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id((id_prefix, 9000usize))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(rows.is_empty(), |d| d.child(empty_state(empty, cx)))
                    .children(
                        rows.into_iter()
                            .filter(|r| self.category.as_deref().map_or(true, |c| r.category == c))
                            .enumerate()
                            .map(|(ix, r)| {
                                let (w_star, w_run) = (weak.clone(), weak.clone());
                                let (id_star, id_run) = (r.id.clone(), r.id.clone());
                                v_flex()
                                    .id((id_prefix, 1000 + ix))
                                    .test_support()
                                    .px_3()
                                    .py_2()
                                    .gap_1()
                                    .border_b_1()
                                    .border_color(theme.border)
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_size(px(13.))
                                                    .child(r.name.clone()),
                                            )
                                            .when_some(r.language.clone(), |d, l| {
                                                d.child(tag_chip(l, cx))
                                            })
                                            .when(kind == InlineKind::Docker, |d| {
                                                d.child(tag_chip(
                                                    if r.category.is_empty() {
                                                        "Docker".to_string()
                                                    } else {
                                                        r.category.clone()
                                                    },
                                                    cx,
                                                ))
                                            })
                                            .child(risk_chip(r.risk, cx))
                                            .when(r.remote, |d| {
                                                d.child(tag_chip(t!("library.remote"), cx))
                                            })
                                            .when(r.has_params, |d| {
                                                d.child(tag_chip(t!("inlinePanel.params"), cx))
                                            })
                                            .when(kind != InlineKind::Docker, |d| {
                                                d.child(act_button(
                                                    (id_prefix, 5000 + ix),
                                                    if r.starred {
                                                        IconName::StarFill
                                                    } else {
                                                        IconName::Star
                                                    },
                                                    if r.starred {
                                                        t!("inlinePanel.unstar")
                                                    } else {
                                                        t!("inlinePanel.star")
                                                    },
                                                    move |_, _, cx| {
                                                        let id = id_star.clone();
                                                        let _ = w_star.update(cx, |this, cx| {
                                                            this.toggle_star(id, cx)
                                                        });
                                                    },
                                                ))
                                            })
                                            .child(act_button(
                                                (id_prefix, 7000 + ix),
                                                IconName::CirclePlay,
                                                if kind == InlineKind::Docker {
                                                    t!("docker.tooltip.insert")
                                                } else {
                                                    t!("inlinePanel.insertTerminal")
                                                },
                                                move |_, window, cx| {
                                                    let id = id_run.clone();
                                                    let _ = w_run.update(cx, |this, cx| {
                                                        drop(this.run(&id, window, cx))
                                                    });
                                                },
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme.muted_foreground)
                                            .child(r.description.clone()),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .font_family("Menlo")
                                            .text_size(px(11.))
                                            .child(r.code.clone()),
                                    )
                            }),
                    ),
            )
    }
}

// ---- run dialog ----

pub struct RunSpec {
    pub title: String,
    pub risk: Risk,
    pub description: String,
    pub url: String,
    pub params: Vec<Param>,
    pub base: String,
    /// Commands substitute `${NAME}`; scripts are inserted verbatim.
    pub substitute: bool,
}

#[derive(Clone, PartialEq)]
enum Source {
    Idle,
    Loading,
    Loaded(String),
    Failed(String),
}

pub struct RunDialog {
    spec: RunSpec,
    pub inputs: Vec<Entity<InputState>>,
    error: Option<String>,
    show_source: bool,
    source: Source,
    on_confirm: InsertFn,
    _subs: Vec<Subscription>,
    _task: Option<Task<()>>,
}

impl RunDialog {
    fn new(
        spec: RunSpec,
        on_confirm: InsertFn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs: Vec<Entity<InputState>> = spec
            .params
            .iter()
            .map(|p| {
                let ph = if p.default_value.is_empty() {
                    t!("inlinePanel.paramPlaceholder", name = p.name.clone())
                } else {
                    p.default_value.clone()
                };
                let dv = p.default_value.clone();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(ph)
                        .default_value(dv)
                })
            })
            .collect();
        let subs = inputs
            .iter()
            .map(|i| {
                cx.observe(i, |this, _, cx| {
                    this.error = None;
                    cx.notify();
                })
            })
            .collect();
        let mut this = Self {
            spec,
            inputs,
            error: None,
            show_source: false,
            source: Source::Idle,
            on_confirm,
            _subs: subs,
            _task: None,
        };
        this.load_source(cx);
        this
    }

    fn load_source(&mut self, cx: &mut Context<Self>) {
        if self.spec.url.is_empty() {
            return;
        }
        self.source = Source::Loading;
        let url = self.spec.url.clone();
        self._task = Some(cx.spawn(async move |this, cx| {
            let r = ops_core::rt::run(async move { fetch_remote_script(&url).await })
                .await
                .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.source = match r {
                    Ok(t) => Source::Loaded(t),
                    Err(e) => Source::Failed(e),
                };
                cx.notify();
            });
        }));
    }

    fn values(&self, cx: &App) -> HashMap<String, String> {
        self.spec
            .params
            .iter()
            .zip(&self.inputs)
            .map(|(p, i)| (p.name.clone(), i.read(cx).value().to_string()))
            .collect()
    }

    /// The command that will be inserted with the current parameter values.
    pub fn resolved(&self, cx: &App) -> String {
        let params: Vec<RemoteScriptParam> = self
            .spec
            .params
            .iter()
            .map(|p| RemoteScriptParam {
                name: p.name.clone(),
                default_value: p.default_value.clone(),
            })
            .collect();
        if self.spec.substitute || !self.spec.url.is_empty() {
            effective_command(&self.spec.base, &self.spec.url, &params, &self.values(cx))
        } else {
            self.spec.base.clone()
        }
    }

    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = self.values(cx);
        for p in &self.spec.params {
            if p.required && values.get(&p.name).map_or(true, |v| v.trim().is_empty()) {
                self.error = Some(t!("inlinePanel.paramRequired", name = p.name.clone()));
                cx.notify();
                return;
            }
        }
        let text = self.resolved(cx);
        window.close_dialog(cx);
        (self.on_confirm)(text, window, cx);
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn source_loaded(&self) -> Option<&str> {
        match &self.source {
            Source::Loaded(t) => Some(t),
            _ => None,
        }
    }
}

fn highlighted(code: &str, theme: &gpui_kit::component::Theme) -> StyledText {
    let color = |t: Tok| -> Option<Hsla> {
        Some(match t {
            Tok::Keyword => theme.primary,
            Tok::Builtin => theme.info,
            Tok::Str => theme.success,
            Tok::Variable | Tok::Number => theme.warning,
            Tok::Operator | Tok::Comment => theme.muted_foreground,
            Tok::Text => return None,
        })
    };
    let runs: Vec<(std::ops::Range<usize>, HighlightStyle)> = tokenize(code)
        .into_iter()
        .filter_map(|(r, t)| {
            color(t).map(|c| {
                (
                    r,
                    HighlightStyle {
                        color: Some(c),
                        ..Default::default()
                    },
                )
            })
        })
        .collect();
    StyledText::new(code.to_string()).with_highlights(runs)
}

impl Render for RunDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let high = self.spec.risk.is_high();
        let resolved = self.resolved(cx);
        let has_url = !self.spec.url.is_empty();
        let preview: AnyElement = if self.show_source {
            match &self.source {
                Source::Loading => div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(t!("inlinePanel.loadingScript"))
                    .into_any_element(),
                Source::Failed(e) => div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(theme.danger)
                    .child(t!("inlinePanel.loadFailed", error = e.clone()))
                    .into_any_element(),
                Source::Loaded(text) => div()
                    .id("run-source")
                    .p_3()
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .child(highlighted(text, &theme))
                    .into_any_element(),
                Source::Idle => div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(t!("inlinePanel.noSource"))
                    .into_any_element(),
            }
        } else {
            div()
                .id("run-preview")
                .p_3()
                .max_h(px(240.))
                .overflow_y_scroll()
                .font_family("Menlo")
                .text_size(px(12.))
                .child(highlighted(&resolved, &theme))
                .into_any_element()
        };
        v_flex()
            .id("inline-run-dialog")
            .test_support()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(risk_chip(self.spec.risk, cx))
                    .when(has_url, |d| {
                        d.child(tag_chip(t!("inlinePanel.remoteScript"), cx))
                    }),
            )
            .when(!self.spec.description.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(self.spec.description.clone()),
                )
            })
            .when(!self.spec.params.is_empty(), |d| {
                d.child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(12.))
                                .child(t!("inlinePanel.params")),
                        )
                        .children(self.spec.params.iter().enumerate().map(|(i, p)| {
                            let label = format!(
                                "{}{}{}",
                                p.name,
                                if p.required { " *" } else { "" },
                                if p.description.is_empty() {
                                    String::new()
                                } else {
                                    format!(" — {}", p.description)
                                }
                            );
                            field(label, Input::new(&self.inputs[i]).small(), None, cx)
                        }))
                        .when_some(self.error.clone(), |d, e| {
                            d.child(div().text_size(px(12.)).text_color(theme.danger).child(e))
                        }),
                )
            })
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("run-tab-command")
                                    .ghost()
                                    .xsmall()
                                    .label(t!("inlinePanel.commandPreview"))
                                    .selected(!self.show_source)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_source = false;
                                        cx.notify();
                                    })),
                            )
                            .when(has_url, |d| {
                                d.child(
                                    Button::new("run-tab-source")
                                        .ghost()
                                        .xsmall()
                                        .label(t!("inlinePanel.scriptSource"))
                                        .selected(self.show_source)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.show_source = true;
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.muted)
                            .child(preview),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("run-cancel")
                            .label(t!("common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child({
                        let b = Button::new("run-confirm")
                            .label(t!("inlinePanel.insertTerminal"))
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
                        if high {
                            b.danger()
                        } else {
                            b.primary()
                        }
                    }),
            )
    }
}
