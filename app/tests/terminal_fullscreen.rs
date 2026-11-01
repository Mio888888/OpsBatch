//! Full-screen programs and links in the terminal pane, driven through the real window with a local PTY:
//! vim (alternate screen, insert mode, application cursor keys, SGR mouse reporting), less (pager) and URL clicks.

mod common;

use std::time::Duration;

use alacritty_terminal::term::TermMode;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{point, AppContext as _, Entity, TestAppContext};
use opsbatch_app::terminal::pane::{pane_element_id, TerminalPane};
use opsbatch_app::terminal::workspace::{workspace, ConnState};

use common::{serial, wait_until, Fixture};

fn win<R>(
    cx: &mut TestAppContext,
    fx: &Fixture,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(fx.handle.into(), |_, window, cx| f(window, cx))
        .unwrap()
}

/// A connected local shell with the pane focused.
fn shell(cx: &mut TestAppContext) -> (Fixture, Entity<TerminalPane>, String) {
    let fx = common::setup(cx, |_| {});
    win(cx, &fx, |w, cx| w.click("terminal-reopen-local", cx));
    let wsp = cx.update(|cx| workspace(cx));
    wait_until(cx, "local tab connected", |cx| {
        cx.update(|cx| {
            wsp.read(cx)
                .active_tab()
                .is_some_and(|t| t.state == ConnState::Connected)
        })
    });
    std::thread::sleep(Duration::from_millis(600));
    cx.run_until_parked();
    win(cx, &fx, |w, cx| w.render_frame(cx));
    let pane = cx.update(|cx| {
        wsp.read(cx)
            .active_tab()
            .and_then(|t| t.pane.clone())
            .expect("pane")
    });
    let sid = pane.read_with(cx, |p, _| p.session_id.clone());
    win(cx, &fx, |w, cx| w.click(pane_element_id(&sid), cx));
    (fx, pane, sid)
}

fn has(cx: &mut TestAppContext, pane: &Entity<TerminalPane>, needle: &str) -> bool {
    cx.update(|cx| pane.read(cx).buffer_text().contains(needle))
}

fn alt_screen(cx: &mut TestAppContext, pane: &Entity<TerminalPane>) -> bool {
    pane.read_with(cx, |p, _| p.engine().mode().contains(TermMode::ALT_SCREEN))
}

fn run(cx: &mut TestAppContext, fx: &Fixture, line: &str) {
    win(cx, fx, |w, cx| {
        w.input(line, cx);
        w.press("enter", cx);
    });
}

fn temp_file(name: &str, content: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("opsbatch-{name}-{}", std::process::id()));
    std::fs::write(&p, content).unwrap();
    p
}

#[gpui_kit::test]
fn vim_uses_the_alternate_screen_application_cursor_keys_and_returns_cleanly(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let (fx, pane, _sid) = shell(cx);
    let file = temp_file("vim-edit", "");
    run(cx, &fx, "echo before-vim");
    wait_until(cx, "echo", |cx| has(cx, &pane, "before-vim"));
    run(cx, &fx, &format!("vim -u NONE -N -n {}", file.display()));
    wait_until(cx, "alternate screen", |cx| alt_screen(cx, &pane));

    // insert text, move with the arrow keys (application cursor mode), insert in the middle, save and quit
    win(cx, &fx, |w, cx| {
        w.input("ihello", cx);
        w.press("left", cx);
        w.press("left", cx);
        w.input("X", cx);
        w.press("escape", cx);
        w.input(":wq", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "back on the normal screen", |cx| !alt_screen(cx, &pane));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "helXlo\n");
    // the shell screen is restored (what was there before vim is still visible) and the shell still works
    assert!(
        pane.read_with(cx, |p, _| p.screen_text())
            .contains("before-vim"),
        "screen restored after vim"
    );
    run(cx, &fx, "echo after-vim-ok");
    wait_until(cx, "shell usable again", |cx| {
        has(cx, &pane, "after-vim-ok")
    });
    let _ = std::fs::remove_file(&file);
}

#[gpui_kit::test]
fn vim_receives_sgr_mouse_clicks_and_selection_stays_local_without_mouse_mode(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let (fx, pane, sid) = shell(cx);
    let file = temp_file("vim-mouse", "abcdef\n");
    run(cx, &fx, &format!("vim -u NONE -N -n {}", file.display()));
    wait_until(cx, "alternate screen", |cx| alt_screen(cx, &pane));
    win(cx, &fx, |w, cx| {
        w.input(":set mouse=a ttymouse=sgr", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "mouse reporting on", |cx| {
        pane.read_with(cx, |p, _| {
            p.engine().mode().contains(TermMode::SGR_MOUSE)
                && p.engine().mode().intersects(TermMode::MOUSE_MODE)
        })
    });

    // click on the third character of the first row, then delete it
    let (cw, lh) = pane
        .read_with(cx, |p, _| p.cell_metrics())
        .expect("laid out");
    win(cx, &fx, |w, cx| {
        w.click_at(pane_element_id(&sid), point(cw * 2.5, lh * 0.5), cx);
        w.input("x", cx);
        w.input(":wq", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "vim quit", |cx| !alt_screen(cx, &pane));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "abdef\n",
        "the click moved vim's cursor to 'c'"
    );
    let _ = std::fs::remove_file(&file);
}

#[gpui_kit::test]
fn less_pages_on_the_alternate_screen_and_restores_the_shell(cx: &mut TestAppContext) {
    let _s = serial();
    let (fx, pane, _sid) = shell(cx);
    let file = temp_file(
        "less",
        &(1..=200)
            .map(|i| format!("LINE{i:03}\n"))
            .collect::<String>(),
    );
    run(cx, &fx, "echo marker-before-less");
    wait_until(cx, "marker", |cx| has(cx, &pane, "marker-before-less"));
    run(cx, &fx, &format!("less {}", file.display()));
    wait_until(cx, "pager shows the first page", |cx| {
        alt_screen(cx, &pane)
            && pane
                .read_with(cx, |p, _| p.screen_text())
                .contains("LINE001")
    });
    win(cx, &fx, |w, cx| w.input(" ", cx));
    let lines = pane.read_with(cx, |p, _| p.lines());
    wait_until(cx, "next page", |cx| {
        !pane
            .read_with(cx, |p, _| p.screen_text())
            .contains("LINE001")
            && pane
                .read_with(cx, |p, _| p.screen_text())
                .contains(&format!("LINE{:03}", lines))
    });
    win(cx, &fx, |w, cx| w.input("q", cx));
    wait_until(cx, "pager closed", |cx| !alt_screen(cx, &pane));
    assert!(pane
        .read_with(cx, |p, _| p.screen_text())
        .contains("marker-before-less"));
    let _ = std::fs::remove_file(&file);
}

#[gpui_kit::test]
fn clicking_a_url_in_the_output_opens_it_but_dragging_or_clicking_elsewhere_does_not(
    cx: &mut TestAppContext,
) {
    let _s = serial();
    let (fx, pane, sid) = shell(cx);
    run(
        cx,
        &fx,
        "printf 'docs at https://example.test/guide?x=1 now\\n'",
    );
    wait_until(cx, "output", |cx| {
        pane.read_with(cx, |p, _| p.screen_text())
            .lines()
            .any(|l| l.starts_with("docs at https://"))
    });
    let (row, col) = pane.read_with(cx, |p, _| {
        let text = p.screen_text();
        let (row, line) = text
            .lines()
            .enumerate()
            .find(|(_, l)| l.starts_with("docs at https://"))
            .unwrap();
        (row, line.find("https").unwrap())
    });
    assert_eq!(
        pane.read_with(cx, |p, _| p.url_at(col + 3, row)).as_deref(),
        Some("https://example.test/guide?x=1")
    );
    assert_eq!(pane.read_with(cx, |p, _| p.url_at(1, row)), None);

    let (cw, lh) = pane
        .read_with(cx, |p, _| p.cell_metrics())
        .expect("laid out");
    let at = |c: usize, r: usize| point(cw * (c as f32 + 0.5), lh * (r as f32 + 0.5));
    // a click on plain text opens nothing
    win(cx, &fx, |w, cx| {
        w.click_at(pane_element_id(&sid), at(1, row), cx)
    });
    assert_eq!(cx.opened_url(), None);
    // a drag that starts on the URL selects instead of opening
    let origin = win(cx, &fx, |w, _| {
        w.find(pane_element_id(&sid)).bounds().origin
    });
    win(cx, &fx, |w, cx| {
        w.drag(origin + at(col + 2, row), origin + at(col + 12, row), cx)
    });
    assert_eq!(cx.opened_url(), None, "dragging selects");
    // a plain click on the URL opens it
    win(cx, &fx, |w, cx| {
        w.click_at(pane_element_id(&sid), at(col + 5, row), cx)
    });
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.test/guide?x=1")
    );
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "types POSIX shell syntax into the local shell")]
fn a_terminal_multiplexer_draws_its_windows_and_detaches(cx: &mut TestAppContext) {
    // GNU screen stands in for tmux (not installed on this machine): same redraw, prefix key and window handling.
    let _s = serial();
    let (fx, pane, _sid) = shell(cx);
    let rc = temp_file(
        "screenrc",
        "startup_message off\nvbell off\ndefscrollback 1000\n",
    );
    let name = format!("opsbatch{}", std::process::id());
    run(cx, &fx, &format!("screen -c {} -S {name}", rc.display()));
    std::thread::sleep(Duration::from_millis(800));
    run(cx, &fx, "echo IN_SCREEN_WINDOW_ONE");
    wait_until(cx, "first window", |cx| {
        pane.read_with(cx, |p, _| p.screen_text())
            .matches("IN_SCREEN_WINDOW_ONE")
            .count()
            >= 2
    });
    // prefix + c opens a second window (a fresh, clean screen), prefix + p goes back to the first
    win(cx, &fx, |w, cx| {
        w.press("ctrl-a", cx);
        w.input("c", cx);
    });
    std::thread::sleep(Duration::from_millis(500));
    run(cx, &fx, "echo IN_SCREEN_WINDOW_TWO");
    wait_until(cx, "second window", |cx| {
        let t = pane.read_with(cx, |p, _| p.screen_text());
        t.matches("IN_SCREEN_WINDOW_TWO").count() >= 2 && !t.contains("IN_SCREEN_WINDOW_ONE")
    });
    win(cx, &fx, |w, cx| {
        w.press("ctrl-a", cx);
        w.input("p", cx);
    });
    wait_until(cx, "first window again", |cx| {
        pane.read_with(cx, |p, _| p.screen_text())
            .contains("IN_SCREEN_WINDOW_ONE")
    });
    // detach
    win(cx, &fx, |w, cx| {
        w.press("ctrl-a", cx);
        w.input("d", cx);
    });
    wait_until(cx, "detached", |cx| has(cx, &pane, "[detached"));
    run(
        cx,
        &fx,
        &format!("screen -S {name} -X quit; echo SCREEN_GONE"),
    );
    wait_until(cx, "session closed", |cx| {
        pane.read_with(cx, |p, _| p.screen_text())
            .matches("SCREEN_GONE")
            .count()
            >= 2
    });
    let _ = std::fs::remove_file(&rc);
}
