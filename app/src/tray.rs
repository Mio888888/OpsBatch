//! System tray icon: "show main window" / "quit"; left click restores the main window (port of lib.rs tray setup).

use std::time::Duration;

use gpui_kit::{App, Global};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

struct TrayHolder {
    _tray: TrayIcon,
    show_id: MenuId,
    quit_id: MenuId,
}

impl Global for TrayHolder {}

fn icon() -> Option<Icon> {
    // 32x32 PNG shipped with the app (same artwork as the Tauri build's default window icon).
    let (rgba, w, h) = decode_png_rgba(include_bytes!("../../assets/32x32.png"))?;
    Icon::from_rgba(rgba, w, h).ok()
}

/// The shipped artwork as straight RGBA (`None` when the bytes are not a decodable image).
pub fn decode_png_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    Some((image.into_raw(), w, h))
}

/// Must run on the main thread after GPUI's event loop exists.
pub fn init(cx: &mut App, show_main: fn(&mut App)) {
    let menu = Menu::new();
    let show = MenuItem::new(ops_i18n::t!("tray.show"), true, None);
    let quit = MenuItem::new(ops_i18n::t!("tray.quit"), true, None);
    if menu.append_items(&[&show, &quit]).is_err() {
        return;
    }
    let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_tooltip("OpsBatch");
    if let Some(icon) = icon() {
        builder = builder.with_icon(icon);
    }
    let Ok(tray) = builder.build() else {
        log::warn!("system tray unavailable");
        return;
    };
    cx.set_global(TrayHolder { _tray: tray, show_id: show.id().clone(), quit_id: quit.id().clone() });

    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_millis(120)).await;
        let _ = cx.update(|cx| {
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                let (show_id, quit_id) = {
                    let t = cx.global::<TrayHolder>();
                    (t.show_id.clone(), t.quit_id.clone())
                };
                if ev.id == show_id {
                    show_main(cx);
                } else if ev.id == quit_id {
                    cx.quit();
                }
            }
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                let left_up = matches!(ev, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. });
                let double = matches!(ev, TrayIconEvent::DoubleClick { button: MouseButton::Left, .. });
                if left_up || double {
                    show_main(cx);
                }
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{decode_png_rgba, icon};

    #[test]
    fn the_tray_icon_is_the_real_artwork() {
        let (rgba, w, h) = decode_png_rgba(include_bytes!("../../assets/32x32.png")).expect("decodes");
        assert_eq!((w, h), (32, 32));
        assert_eq!(rgba.len(), 32 * 32 * 4);
        // not the old flat blue disc: the artwork has more than two distinct pixels
        let distinct: std::collections::HashSet<[u8; 4]> = rgba.chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        assert!(distinct.len() > 8, "{} distinct colours", distinct.len());
        assert!(icon().is_some());
        assert!(decode_png_rgba(b"not an image").is_none());
    }
}
