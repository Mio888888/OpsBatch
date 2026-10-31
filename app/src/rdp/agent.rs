//! Turns AI operations into RDP input (port of `utils/rdpAgentExecutor.ts`) and plays them with human-ish pacing.

use std::time::Duration;

use futures::future::BoxFuture;
use ops_ai::parser::RdpOperation;
use ops_rdp::keys::{scancode_for_key, Scancode};
use ops_rdp::RdpInputEvent;

/// How to decide that the remote screen has finished reacting to an operation. The executor does not guess with a
/// fixed pause: it watches the picture (see [`wait_for_settle`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settle {
    /// never continue earlier than this
    pub min: Duration,
    /// the picture must have been still for this long
    pub quiet: Duration,
    /// if nothing changed at all after this long, the operation has no visible effect: continue
    pub grace: Duration,
    /// never wait longer than this
    pub max: Duration,
}

/// The final wait after a plan, before the next step (or the next screenshot for the model) starts.
pub const PLAN_SETTLE: Settle = Settle {
    min: Duration::from_millis(800),
    quiet: Duration::from_millis(800),
    grace: Duration::from_millis(1500),
    max: Duration::from_secs(10),
};

const fn settle(min: u64, quiet: u64, grace: u64, max: u64) -> Settle {
    Settle {
        min: Duration::from_millis(min),
        quiet: Duration::from_millis(quiet),
        grace: Duration::from_millis(grace),
        max: Duration::from_millis(max),
    }
}

/// What to wait for after `op` has been sent completely.
pub fn settle_for(op: &RdpOperation) -> Option<Settle> {
    match op {
        // menus open, dialogs appear
        RdpOperation::Click { .. } => Some(settle(350, 500, 1200, 6000)),
        RdpOperation::Drag { .. } => Some(settle(300, 500, 1200, 6000)),
        // Enter / Win / Alt+F4 / Ctrl+Shift+Esc: programs start, windows close
        RdpOperation::Key { .. } => Some(settle(500, 700, 1500, 10_000)),
        RdpOperation::Type { .. } => Some(settle(150, 300, 600, 3000)),
        RdpOperation::Scroll { .. } => Some(settle(150, 350, 800, 3000)),
        RdpOperation::Move { .. } | RdpOperation::Wait { .. } => None,
    }
}

/// Wait until the picture has reacted to the last input and then stopped changing.
///
/// `now_ms` is a monotonic clock and `last_change_ms` the time of the last meaningful picture update, both in
/// milliseconds. It returns when
/// * the picture changed after the wait began and has then been still for `quiet` (and `min` has passed), or
/// * nothing changed within `grace` (the operation had no visible effect), or
/// * `max` has passed.
pub async fn wait_for_settle<N, L, W>(settle: Settle, now_ms: N, last_change_ms: L, sleep: W)
where
    N: Fn() -> u64,
    L: Fn() -> u64,
    W: Fn(Duration) -> BoxFuture<'static, ()>,
{
    let start = now_ms();
    loop {
        sleep(Duration::from_millis(40)).await;
        let now = now_ms();
        let elapsed = Duration::from_millis(now.saturating_sub(start));
        if elapsed >= settle.max {
            return;
        }
        let last = last_change_ms();
        if last > start {
            let still_for = Duration::from_millis(now.saturating_sub(last));
            if elapsed >= settle.min && still_for >= settle.quiet {
                return;
            }
        } else if elapsed >= settle.grace && elapsed >= settle.min {
            return;
        }
    }
}

fn modifier_scancode(key: &str) -> Option<Scancode> {
    let s = |code: u8, extended: bool| Some(Scancode { code, extended });
    match key {
        "Meta" | "MetaLeft" | "Win" | "Windows" => s(0x5b, true),
        "MetaRight" => s(0x5c, true),
        "Control" | "ControlLeft" | "Ctrl" => s(0x1d, false),
        "ControlRight" => s(0x1d, true),
        "Shift" | "ShiftLeft" => s(0x2a, false),
        "ShiftRight" => s(0x36, false),
        "Alt" | "AltLeft" => s(0x38, false),
        "AltRight" | "AltGr" => s(0x38, true),
        _ => None,
    }
}

fn resolve_scancode(key: &str) -> Option<Scancode> {
    if let Some(m) = modifier_scancode(key) {
        return Some(m);
    }
    let code = if key.chars().count() == 1 {
        format!("Key{}", key.to_uppercase())
    } else {
        key.to_string()
    };
    scancode_for_key(key, &code)
}

fn key_event(s: Scancode, down: bool) -> RdpInputEvent {
    RdpInputEvent::KeyScancode {
        code: s.code,
        extended: s.extended,
        down,
    }
}

fn unicode_events(c: &str) -> Vec<RdpInputEvent> {
    vec![
        RdpInputEvent::Unicode {
            character: c.to_string(),
            down: true,
        },
        RdpInputEvent::Unicode {
            character: c.to_string(),
            down: false,
        },
    ]
}

fn combo_events(keys: &[String]) -> Vec<RdpInputEvent> {
    let mapped: Vec<(&String, Option<Scancode>)> =
        keys.iter().map(|k| (k, resolve_scancode(k))).collect();
    let mut events = Vec::new();
    for (key, sc) in &mapped {
        match sc {
            Some(s) => events.push(key_event(*s, true)),
            None if key.chars().count() == 1 => events.push(RdpInputEvent::Unicode {
                character: (*key).clone(),
                down: true,
            }),
            None => {}
        }
    }
    for (key, sc) in mapped.iter().rev() {
        match sc {
            Some(s) => events.push(key_event(*s, false)),
            None if key.chars().count() == 1 => events.push(RdpInputEvent::Unicode {
                character: (*key).clone(),
                down: false,
            }),
            None => {}
        }
    }
    events
}

fn mouse_move(x: u16, y: u16) -> Vec<RdpInputEvent> {
    vec![RdpInputEvent::MouseMove { x, y }]
}

fn mouse_button(x: u16, y: u16, button: u8, down: bool) -> Vec<RdpInputEvent> {
    vec![RdpInputEvent::MouseButton { x, y, button, down }]
}

/// One inner vector is sent atomically; the executor waits between vectors.
pub fn operations_to_input_events(ops: &[RdpOperation]) -> Vec<Vec<RdpInputEvent>> {
    let mut batches = Vec::new();
    for op in ops {
        match op {
            RdpOperation::Move { x, y } => batches.push(mouse_move(*x, *y)),
            RdpOperation::Click {
                x,
                y,
                button,
                double_click,
            } => {
                batches.push(mouse_move(*x, *y));
                // Remote Windows does not reliably register an instantaneous down/up pair.
                batches.push(mouse_button(*x, *y, *button, true));
                batches.push(mouse_button(*x, *y, *button, false));
                if *double_click {
                    batches.push(mouse_button(*x, *y, *button, true));
                    batches.push(mouse_button(*x, *y, *button, false));
                }
            }
            RdpOperation::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
            } => {
                batches.push(mouse_move(*from_x, *from_y));
                batches.push(mouse_button(*from_x, *from_y, *button, true));
                let steps = 4i32;
                for step in 1..steps {
                    let px = (f64::from(*from_x)
                        + f64::from(i32::from(*to_x) - i32::from(*from_x)) * f64::from(step)
                            / f64::from(steps))
                    .round() as u16;
                    let py = (f64::from(*from_y)
                        + f64::from(i32::from(*to_y) - i32::from(*from_y)) * f64::from(step)
                            / f64::from(steps))
                    .round() as u16;
                    batches.push(mouse_move(px, py));
                }
                batches.push(mouse_move(*to_x, *to_y));
                batches.push(mouse_button(*to_x, *to_y, *button, false));
            }
            RdpOperation::Scroll {
                x,
                y,
                delta,
                vertical,
            } => {
                batches.push(mouse_move(*x, *y));
                // WHEEL_DELTA is 120 per notch; positive scrolls up.
                let units = (delta * 120).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                batches.push(vec![RdpInputEvent::Wheel {
                    x: *x,
                    y: *y,
                    delta: units,
                    vertical: *vertical,
                }]);
            }
            RdpOperation::Type { text } => {
                for ch in text.chars() {
                    batches.push(unicode_events(&ch.to_string()));
                }
            }
            RdpOperation::Key { keys } => match keys.as_slice() {
                [single] => {
                    if let Some(s) = resolve_scancode(single) {
                        batches.push(vec![key_event(s, true), key_event(s, false)]);
                    }
                }
                [] => {}
                many => batches.push(combo_events(many)),
            },
            RdpOperation::Wait { .. } => {}
        }
    }
    batches
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchKind {
    Key,
    Click,
    Scroll,
    Type,
    Move,
}

fn classify(batch: &[RdpInputEvent]) -> BatchKind {
    if batch
        .iter()
        .any(|e| matches!(e, RdpInputEvent::KeyScancode { .. }))
    {
        BatchKind::Key
    } else if batch
        .iter()
        .any(|e| matches!(e, RdpInputEvent::Unicode { .. }))
    {
        BatchKind::Type
    } else if batch
        .iter()
        .any(|e| matches!(e, RdpInputEvent::MouseButton { .. }))
    {
        BatchKind::Click
    } else if batch
        .iter()
        .any(|e| matches!(e, RdpInputEvent::Wheel { .. }))
    {
        BatchKind::Scroll
    } else {
        BatchKind::Move
    }
}

fn delay_for_kind(kind: BatchKind) -> Duration {
    Duration::from_millis(match kind {
        // Keys (Enter, Win) usually switch UI: menus open, programs start.
        BatchKind::Key => 500,
        BatchKind::Click => 350,
        // One character at a time so the remote start-menu search neither drops nor repeats letters.
        BatchKind::Type => 90,
        BatchKind::Scroll => 120,
        BatchKind::Move => 80,
    })
}

fn is_button_down(batch: &[RdpInputEvent]) -> bool {
    batch
        .iter()
        .any(|e| matches!(e, RdpInputEvent::MouseButton { down: true, .. }))
}

fn delay_after_batch(
    op: &RdpOperation,
    kind: BatchKind,
    index: usize,
    batch: &[RdpInputEvent],
) -> Duration {
    if let RdpOperation::Click { double_click, .. } = op {
        if kind == BatchKind::Click {
            if is_button_down(batch) {
                return Duration::from_millis(70);
            }
            if *double_click && index == 2 {
                return Duration::from_millis(140);
            }
        }
    }
    delay_for_kind(kind)
}

/// Play `ops` through `send`: short pauses between the input events of one operation (`sleep`), and after each
/// operation a wait for the screen to settle (`settled`) before the next one starts, so a step never runs on top of
/// the previous one's still-loading window.
pub async fn execute_operations<S, W, T>(
    ops: &[RdpOperation],
    mut send: S,
    sleep: W,
    settled: T,
) -> Result<(), String>
where
    S: FnMut(RdpInputEvent) -> Result<(), String>,
    W: Fn(Duration) -> BoxFuture<'static, ()>,
    T: Fn(Settle) -> BoxFuture<'static, ()>,
{
    for op in ops {
        if let RdpOperation::Wait { ms } = op {
            sleep(Duration::from_millis(u64::from(*ms))).await;
            continue;
        }
        let batches = operations_to_input_events(std::slice::from_ref(op));
        for (index, batch) in batches.iter().enumerate() {
            for event in batch {
                send(event.clone())?;
            }
            // the last batch of an operation is followed by the settle wait instead of a fixed pause
            let last = index + 1 == batches.len();
            if !(last && settle_for(op).is_some()) {
                sleep(delay_after_batch(op, classify(batch), index, batch)).await;
            }
        }
        if let Some(policy) = settle_for(op) {
            settled(policy).await;
        }
    }
    settled(PLAN_SETTLE).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn click(x: u16, y: u16, double: bool) -> RdpOperation {
        RdpOperation::Click {
            x,
            y,
            button: 0,
            double_click: double,
        }
    }

    #[test]
    fn double_click_is_split_into_held_down_up_pairs() {
        let batches = operations_to_input_events(&[click(25, 225, true)]);
        assert_eq!(
            batches,
            vec![
                vec![RdpInputEvent::MouseMove { x: 25, y: 225 }],
                vec![RdpInputEvent::MouseButton {
                    x: 25,
                    y: 225,
                    button: 0,
                    down: true
                }],
                vec![RdpInputEvent::MouseButton {
                    x: 25,
                    y: 225,
                    button: 0,
                    down: false
                }],
                vec![RdpInputEvent::MouseButton {
                    x: 25,
                    y: 225,
                    button: 0,
                    down: true
                }],
                vec![RdpInputEvent::MouseButton {
                    x: 25,
                    y: 225,
                    button: 0,
                    down: false
                }],
            ]
        );
    }

    #[test]
    fn single_click_is_move_down_up() {
        let batches = operations_to_input_events(&[click(640, 400, false)]);
        assert_eq!(batches.len(), 3);
        assert_eq!(
            batches[2],
            vec![RdpInputEvent::MouseButton {
                x: 640,
                y: 400,
                button: 0,
                down: false
            }]
        );
    }

    #[test]
    fn drags_interpolate_and_scrolls_use_wheel_units() {
        let drag = operations_to_input_events(&[RdpOperation::Drag {
            from_x: 0,
            from_y: 100,
            to_x: 40,
            to_y: 0,
            button: 0,
        }]);
        let moves: Vec<_> = drag
            .iter()
            .filter_map(|b| {
                if let [RdpInputEvent::MouseMove { x, y }] = b.as_slice() {
                    Some((*x, *y))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(moves, vec![(0, 100), (10, 75), (20, 50), (30, 25), (40, 0)]);
        assert_eq!(drag.len(), 7);
        let scroll = operations_to_input_events(&[RdpOperation::Scroll {
            x: 5,
            y: 6,
            delta: -3,
            vertical: true,
        }]);
        assert_eq!(
            scroll[1],
            vec![RdpInputEvent::Wheel {
                x: 5,
                y: 6,
                delta: -360,
                vertical: true
            }]
        );
    }

    #[test]
    fn typing_is_one_unicode_pair_per_character() {
        let batches = operations_to_input_events(&[RdpOperation::Type {
            text: "a中".into()
        }]);
        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches[1][0],
            RdpInputEvent::Unicode {
                character: "中".into(),
                down: true
            }
        );
        assert_eq!(
            batches[1][1],
            RdpInputEvent::Unicode {
                character: "中".into(),
                down: false
            }
        );
    }

    #[test]
    fn keys_and_combos() {
        let keys = |names: &[&str]| RdpOperation::Key {
            keys: names.iter().map(|s| s.to_string()).collect(),
        };
        // Windows key alone.
        assert_eq!(
            operations_to_input_events(&[keys(&["Meta"])]),
            vec![vec![
                RdpInputEvent::KeyScancode {
                    code: 0x5b,
                    extended: true,
                    down: true
                },
                RdpInputEvent::KeyScancode {
                    code: 0x5b,
                    extended: true,
                    down: false
                }
            ]]
        );
        // Ctrl+C: both down in order, released in reverse.
        let combo = &operations_to_input_events(&[keys(&["Control", "c"])])[0];
        assert_eq!(combo.len(), 4);
        assert_eq!(
            combo[1],
            RdpInputEvent::KeyScancode {
                code: 0x2e,
                extended: false,
                down: true
            }
        );
        assert_eq!(
            combo[2],
            RdpInputEvent::KeyScancode {
                code: 0x2e,
                extended: false,
                down: false
            }
        );
        assert_eq!(
            combo[3],
            RdpInputEvent::KeyScancode {
                code: 0x1d,
                extended: false,
                down: false
            }
        );
        // Unknown single digit keys yield nothing (the original resolves single characters through `KeyX` codes).
        assert!(operations_to_input_events(&[keys(&["1"])]).is_empty());
        // ...but inside a combination an unmapped single character falls back to Unicode.
        let combo = &operations_to_input_events(&[keys(&["Control", "1"])])[0];
        assert!(combo.iter().any(
            |e| matches!(e, RdpInputEvent::Unicode { character, down: true } if character == "1")
        ));
        assert!(operations_to_input_events(&[RdpOperation::Wait { ms: 500 }]).is_empty());
    }

    #[test]
    fn pacing_depends_on_the_kind_of_batch() {
        let op = click(1, 1, true);
        let batches = operations_to_input_events(std::slice::from_ref(&op));
        let delays: Vec<u64> = batches
            .iter()
            .enumerate()
            .map(|(i, b)| delay_after_batch(&op, classify(b), i, b).as_millis() as u64)
            .collect();
        assert_eq!(delays, vec![80, 70, 140, 70, 350]);
        let key = RdpOperation::Key {
            keys: vec!["Enter".into()],
        };
        let b = &operations_to_input_events(std::slice::from_ref(&key))[0];
        assert_eq!(
            delay_after_batch(&key, classify(b), 0, b),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn the_executor_sends_everything_in_order_and_settles_after_each_operation() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (s2, l2, l3) = (sent.clone(), log.clone(), log.clone());
        let ops = [
            RdpOperation::Wait { ms: 1000 },
            click(3, 4, false),
            RdpOperation::Type { text: "x".into() },
        ];
        futures::executor::block_on(execute_operations(
            &ops,
            move |e| {
                s2.lock().unwrap().push(e);
                Ok(())
            },
            move |d| {
                l2.lock().unwrap().push(format!("sleep {}", d.as_millis()));
                Box::pin(async {})
            },
            move |s| {
                l3.lock().unwrap().push(format!(
                    "settle min={} quiet={}",
                    s.min.as_millis(),
                    s.quiet.as_millis()
                ));
                Box::pin(async {})
            },
        ))
        .unwrap();
        assert_eq!(sent.lock().unwrap().len(), 1 + 1 + 1 + 2);
        // explicit wait; the click's move and press are paced, its release is followed by the settle; the typed
        // character is one batch followed by its settle; and the plan ends with its own settle
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                "sleep 1000",
                "sleep 80",
                "sleep 70",
                "settle min=350 quiet=500",
                "settle min=150 quiet=300",
                "settle min=800 quiet=800"
            ]
        );
    }

    #[test]
    fn a_failing_send_stops_the_plan() {
        let result = futures::executor::block_on(execute_operations(
            &[click(1, 1, false)],
            |_| Err("session closed".into()),
            |_| Box::pin(async {}),
            |_| Box::pin(async {}),
        ));
        assert_eq!(result, Err("session closed".to_string()));
    }

    /// A fake clock: `sleep` advances it, `changes` are the times the picture changed.
    fn run_settle(policy: Settle, start: u64, changes: &[u64]) -> u64 {
        use std::cell::Cell;
        use std::rc::Rc;
        let clock = Rc::new(Cell::new(start));
        let (c1, c2, c3) = (clock.clone(), clock.clone(), clock.clone());
        let changes = changes.to_vec();
        futures::executor::block_on(wait_for_settle(
            policy,
            move || c1.get(),
            move || {
                changes
                    .iter()
                    .copied()
                    .filter(|t| *t <= c2.get())
                    .max()
                    .unwrap_or(0)
            },
            move |d| {
                c3.set(c3.get() + d.as_millis() as u64);
                Box::pin(async {})
            },
        ));
        clock.get() - start
    }

    #[test]
    fn the_wait_ends_when_the_picture_changed_and_then_stood_still() {
        let policy = settle(350, 500, 1200, 6000);
        // an app window appears at 700 ms and keeps painting until 2100 ms: done 500 ms after the last change
        let waited = run_settle(policy, 10_000, &[10_700, 10_900, 11_300, 11_700, 12_100]);
        // (a pause longer than `quiet` in the middle of the load already counts as settled)
        assert!(run_settle(policy, 10_000, &[10_700, 11_500]) < 2000);
        assert!((2600..=2700).contains(&waited), "waited {waited}");
        // an operation that changes nothing is released after the grace period
        let waited = run_settle(policy, 10_000, &[]);
        assert!((1200..=1280).contains(&waited), "waited {waited}");
        // a picture that never stops changing is cut off at the maximum
        let busy: Vec<u64> = (0..300).map(|i| 10_000 + 50 * i).collect();
        let waited = run_settle(policy, 10_000, &busy);
        assert!((6000..=6080).contains(&waited), "waited {waited}");
        // one quick change: still at least `min` and `quiet` after it
        let waited = run_settle(policy, 10_000, &[10_100]);
        assert!((600..=680).contains(&waited), "waited {waited}");
    }
}
