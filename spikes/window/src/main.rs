//! M0-04 spike (v2, official `TitleBar`): window chrome from gpui-component's `TitleBar` (native traffic lights on
//! macOS, drawn caption buttons on Windows/Linux, built-in drag + double-click), singleton/keyed secondary windows,
//! tray icon with menu, and "close to tray" implemented as close-window + keep-app-alive + reopen.
//!
//! Findings so far: gpui has no per-window hide; `App::hide()` hides the whole app.
//! So "hide to tray" == drop the window (QuitMode::Explicit) and rebuild it from
//! app-level Entities when the tray asks for it.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

gpui_kit::actions!(spike_window, [Quit]);

/// App-level state that must survive the main window being closed to tray.
struct Shared {
    clicks: usize,
}

struct AppState(Entity<Shared>);
impl Global for AppState {}

#[derive(Default)]
struct WindowRegistry {
    windows: HashMap<String, AnyWindowHandle>,
}
impl Global for WindowRegistry {}

struct TrayHolder {
    _tray: TrayIcon,
    show_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
}
impl Global for TrayHolder {}

/// Open a window under `label`, or focus it if it is still alive.
fn open_or_focus<V: 'static + Render>(
    label: &str,
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) {
    let existing = cx.global::<WindowRegistry>().windows.get(label).copied();
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            cx.activate(true);
            return;
        }
    }
    if let Ok((handle, _view)) = gpui_kit::open_window(options, cx, build) {
        cx.global_mut::<WindowRegistry>()
            .windows
            .insert(label.to_string(), handle);
        cx.activate(true);
    }
}

fn window_options(cx: &App, w: f32, h: f32, title: &str) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(w), px(h)),
            cx,
        ))),
        window_min_size: Some(size(px(480.), px(320.))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    }
}

fn open_main(cx: &mut App) {
    let opts = window_options(cx, 900., 560., "OpsBatch window spike");
    open_or_focus("main", opts, cx, |_, cx| {
        let shared = cx.global::<AppState>().0.clone();
        cx.new(|_| MainView { shared })
    });
}

struct MainView {
    shared: Entity<Shared>,
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.shared.read(cx).clicks;
        let shared = self.shared.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .child(
                TitleBar::new().child(
                    div()
                        .text_size(px(12.))
                        .child("OpsBatch window spike (drag the title bar, double-click to zoom)"),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .gap_3()
                    .p_4()
                    .child(format!(
                        "Shared counter (lives outside the window): {clicks}"
                    ))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(Button::new("inc").primary().label("Increment").on_click({
                                let shared = shared.clone();
                                move |_, _, cx| {
                                    shared.update(cx, |s, cx| {
                                        s.clicks += 1;
                                        cx.notify();
                                    })
                                }
                            }))
                            .child(
                                Button::new("settings")
                                    .label("Open Settings (singleton)")
                                    .on_click(|_, _, cx| {
                                        let opts = window_options(cx, 640., 420., "Settings");
                                        open_or_focus("settings", opts, cx, |_, cx| {
                                            cx.new(|_| LabelView("Settings window".into()))
                                        });
                                    }),
                            )
                            .child(Button::new("ed-a").label("Editor A").on_click(|_, _, cx| {
                                let opts = window_options(cx, 560., 380., "Editor A");
                                open_or_focus("editor-a", opts, cx, |_, cx| {
                                    cx.new(|_| LabelView("Editor A".into()))
                                });
                            }))
                            .child(Button::new("ed-b").label("Editor B").on_click(|_, _, cx| {
                                let opts = window_options(cx, 560., 380., "Editor B");
                                open_or_focus("editor-b", opts, cx, |_, cx| {
                                    cx.new(|_| LabelView("Editor B".into()))
                                });
                            })),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("totray")
                                    .label("Close to tray (drop window, keep app)")
                                    .on_click(|_, window, _| window.remove_window()),
                            )
                            .child(
                                Button::new("hideapp")
                                    .label("cx.hide() (hide whole app)")
                                    .on_click(|_, _, cx| cx.hide()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "window active: {} | controls: {:?}",
                                window.is_window_active(),
                                window.window_controls()
                            )),
                    ),
            )
    }
}

struct LabelView(SharedString);
impl Render for LabelView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .child(TitleBar::new().child(div().text_size(px(12.)).child(self.0.clone())))
            .child(div().p_4().child(self.0.clone()))
    }
}

fn make_icon() -> Icon {
    // 32x32 blue disc, no image-decoding dependency needed for the spike.
    let n = 32usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            if dx * dx + dy * dy <= 14.0 * 14.0 {
                let i = (y * n + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[0x7a, 0xa2, 0xf7, 0xff]);
            }
        }
    }
    Icon::from_rgba(rgba, n as u32, n as u32).expect("icon")
}

fn main() {
    gpui_kit::application()
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);
            cx.set_global(WindowRegistry::default());
            let shared = cx.new(|_| Shared { clicks: 0 });
            cx.set_global(AppState(shared));

            // Tray (must be created on the main thread, after the event loop exists).
            let menu = Menu::new();
            let show = MenuItem::new("Show main window", true, None);
            let quit = MenuItem::new("Quit", true, None);
            menu.append_items(&[&show, &quit]).unwrap();
            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("OpsBatch spike")
                .with_icon(make_icon())
                .build()
                .expect("tray");
            cx.set_global(TrayHolder {
                _tray: tray,
                show_id: show.id().clone(),
                quit_id: quit.id().clone(),
            });

            // Poll tray events on the GPUI foreground executor.
            cx.spawn(async move |cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                cx.update(|cx| {
                    while let Ok(ev) = MenuEvent::receiver().try_recv() {
                        let (show_id, quit_id) = {
                            let t = cx.global::<TrayHolder>();
                            (t.show_id.clone(), t.quit_id.clone())
                        };
                        if ev.id == show_id {
                            open_main(cx);
                        } else if ev.id == quit_id {
                            cx.quit();
                        }
                    }
                    while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                        if let TrayIconEvent::Click {
                            button: tray_icon::MouseButton::Left,
                            button_state: tray_icon::MouseButtonState::Up,
                            ..
                        }
                        | TrayIconEvent::DoubleClick {
                            button: tray_icon::MouseButton::Left,
                            ..
                        } = ev
                        {
                            open_main(cx);
                        }
                    }
                });
            })
            .detach();

            open_main(cx);
        });
}
