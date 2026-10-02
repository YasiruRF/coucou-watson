// Island window: placement on the chosen display, the two window sizes
// (full panel / invisible wake strip), click-through and the cursor poll.
//
// There is no notch on a PC, so the island is a black shape drawn at the top
// centre of the main display inside a borderless, transparent, always-on-top
// window that never takes focus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::platform::{self, cursor_physical, left_button_down};

/// Logical size of the full window — the largest island view, like the macOS panel.
pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 320.0;
/// Logical size of the invisible strip that wakes the island when it is hidden.
pub const STRIP_W: f64 = 240.0;
pub const STRIP_H: f64 = 6.0;
/// Logical size of the window while Mochi is a floating ball: the ball itself
/// plus room for the entry margin around it.
pub const BALL_W: f64 = 104.0;
pub const BALL_H: f64 = 104.0;
/// Where the ball first appears, measured down from the top of the screen.
const BALL_TOP: f64 = 56.0;
/// Pointer travel, in physical pixels, below which a press on the ball is a tap.
const TAP_SLOP: f64 = 4.0;

pub const WINDOW_LABEL: &str = "island";

/// Margin around the island that still counts as "on the island", in logical px.
/// Wider than the macOS 6 pt because a click must never be swallowed.
const HIT_MARGIN: f64 = 14.0;

#[derive(Serialize, Clone)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// The island shape in window-logical coordinates, pushed by the front end.
/// The poll thread owns the click-through decision so it lands in the same 16 ms
/// tick as the cursor read — an IPC round trip here loses clicks.
#[derive(Clone, Copy, Default)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Sent when the button comes up after a press on the floating ball.
#[derive(Serialize, Clone)]
pub struct BallReleased {
    /// False for a tap: the pointer never really travelled.
    pub moved: bool,
}

/// A press on the ball that the poll thread is turning into a window drag.
/// Dragging is done here rather than by the page: the window is the thing that
/// moves, so the page would lose the pointer the moment it did.
#[derive(Clone, Copy)]
struct BallDrag {
    /// Cursor minus the window's top-left corner, physical pixels, at press time.
    grab: (f64, f64),
    /// Cursor at press time, physical pixels.
    start: (f64, f64),
    moved: bool,
}

/// Wakes / parks the cursor poll thread so a hidden island costs literally nothing.
pub struct PollGate {
    active: Mutex<bool>,
    cv: Condvar,
    pub collapsed: AtomicBool,
    /// True while the window is the small floating-ball window.
    pub ball: AtomicBool,
    /// Where the ball was last left (physical, top-left); None until it is first moved.
    ball_pos: Mutex<Option<(i32, i32)>>,
    drag: Mutex<Option<BallDrag>>,
    pub rect: Mutex<IslandRect>,
    /// Mirrors the window flag so we only call into the OS when it changes.
    ignoring: AtomicBool,
}

impl PollGate {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(false),
            cv: Condvar::new(),
            collapsed: AtomicBool::new(true),
            ball: AtomicBool::new(false),
            ball_pos: Mutex::new(None),
            drag: Mutex::new(None),
            rect: Mutex::new(IslandRect::default()),
            ignoring: AtomicBool::new(false),
        }
    }

    /// The page saw a press on the ball: from here the poll thread drags the window.
    pub fn begin_ball_drag(&self, app: &AppHandle) {
        let Some(win) = window(app) else { return };
        let (Ok(origin), Some((cx, cy))) = (win.outer_position(), cursor_physical()) else { return };
        *self.drag.lock().unwrap() = Some(BallDrag {
            grab: (cx - origin.x as f64, cy - origin.y as f64),
            start: (cx, cy),
            moved: false,
        });
    }

    pub fn set_rect(&self, rect: IslandRect) {
        *self.rect.lock().unwrap() = rect;
    }

    /// Forces the next poll tick to re-apply the flag (after a window resize).
    pub fn forget_ignore_state(&self) {
        self.ignoring.store(false, Ordering::Relaxed);
    }

    pub fn set_active(&self, on: bool) {
        let mut guard = self.active.lock().unwrap();
        *guard = on;
        self.cv.notify_all();
    }

    fn wait_until_active(&self) {
        let mut guard = self.active.lock().unwrap();
        while !*guard {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn is_active(&self) -> bool {
        *self.active.lock().unwrap()
    }
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

fn monitor_contains(m: &Monitor, x: f64, y: f64) -> bool {
    let p = m.position();
    let s = m.size();
    x >= p.x as f64
        && x < (p.x + s.width as i32) as f64
        && y >= p.y as f64
        && y < (p.y + s.height as i32) as f64
}

/// The display the island lives on: the primary one, or the one under the cursor.
fn target_monitor(app: &AppHandle, pref: &str) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    if pref == "cursor" {
        if let Some((cx, cy)) = cursor_physical() {
            if let Some(m) = monitors.iter().find(|m| monitor_contains(m, cx, cy)) {
                return Some(m.clone());
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next())
}

pub fn screen_info(app: &AppHandle, pref: &str) -> ScreenInfo {
    match target_monitor(app, pref) {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    }
}

/// Places and sizes the window. `collapsed` picks the wake strip instead of the panel.
pub fn apply_geometry(app: &AppHandle, pref: &str, collapsed: bool) {
    let Some(win) = window(app) else {
        crate::log::line("apply_geometry: window(app) returned None");
        return;
    };
    let Some(m) = target_monitor(app, pref) else {
        crate::log::line(format!("apply_geometry: target_monitor(pref={pref}) returned None"));
        return;
    };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    let (lw, lh) = if collapsed { (STRIP_W, STRIP_H) } else { (PANEL_W, PANEL_H) };
    let pw = (lw * scale).round().max(1.0) as u32;
    let ph = (lh * scale).round().max(1.0) as u32;
    let x = mp.x + (ms.width as i32 - pw as i32) / 2;
    let y = mp.y;

    let s1 = win.set_size(PhysicalSize::new(pw, ph));
    let p1 = win.set_position(PhysicalPosition::new(x, y));
    let s2 = win.set_size(PhysicalSize::new(pw, ph));
    let top = win.set_always_on_top(true);
    let vis = win.is_visible();
    let actual_pos = win.outer_position();
    let actual_size = win.inner_size();
    crate::log::line(format!(
        "apply_geometry: collapsed={collapsed} target=({x},{y} {pw}x{ph}) scale={scale} mp={mp:?} ms={ms:?} s1={s1:?} p1={p1:?} s2={s2:?} top={top:?} vis={vis:?} actual_pos={actual_pos:?} actual_size={actual_size:?}"
    ));
}

/// Shrinks the window to the floating ball and puts it where it was last left
/// (or just under the top edge, centred, the first time). Always kept on screen:
/// a display change since the last drag must not strand the ball off it.
pub fn apply_ball_geometry(app: &AppHandle, pref: &str, gate: &PollGate) {
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, pref) else { return };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    let pw = (BALL_W * scale).round().max(1.0) as u32;
    let ph = (BALL_H * scale).round().max(1.0) as u32;
    let max_x = mp.x + (ms.width as i32 - pw as i32).max(0);
    let max_y = mp.y + (ms.height as i32 - ph as i32).max(0);
    let (x, y) = match *gate.ball_pos.lock().unwrap() {
        Some((x, y)) => (x.clamp(mp.x, max_x), y.clamp(mp.y, max_y)),
        None => (
            mp.x + (ms.width as i32 - pw as i32) / 2,
            mp.y + (BALL_TOP * scale).round() as i32,
        ),
    };

    let s1 = win.set_size(PhysicalSize::new(pw, ph));
    let p1 = win.set_position(PhysicalPosition::new(x, y));
    let s2 = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_always_on_top(true);
    crate::log::line(format!(
        "apply_ball_geometry: target=({x},{y} {pw}x{ph}) s1={s1:?} p1={p1:?} s2={s2:?}"
    ));
}

/// Position, size and scale of the monitor the island lives on. Any change here
/// means the island has to be placed again.
fn current_screen_key(app: &AppHandle) -> Option<(i32, i32, u32, u32, u64)> {
    let pref = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().screen.clone())
        .unwrap_or_else(|| "primary".into());
    let m = target_monitor(app, &pref)?;
    let p = m.position();
    let size = m.size();
    Some((p.x, p.y, size.width, size.height, m.scale_factor().to_bits()))
}

/// One tick of a ball drag. While the button is held the window follows the
/// cursor; once it comes up the drag ends and says whether it was only a tap.
/// Returns `Some` exactly once, on release.
fn drive_ball_drag(
    win: &WebviewWindow,
    gate: &PollGate,
    down: bool,
) -> Option<BallReleased> {
    let mut slot = gate.drag.lock().unwrap();
    let mut drag = (*slot)?;

    if !down {
        *slot = None;
        return Some(BallReleased { moved: drag.moved });
    }

    let (cx, cy) = cursor_physical()?;
    if !drag.moved && (cx - drag.start.0).hypot(cy - drag.start.1) > TAP_SLOP {
        drag.moved = true;
    }
    if drag.moved {
        let pos = ((cx - drag.grab.0).round() as i32, (cy - drag.grab.1).round() as i32);
        let _ = win.set_position(PhysicalPosition::new(pos.0, pos.1));
        *gate.ball_pos.lock().unwrap() = Some(pos);
    }
    *slot = Some(drag);
    None
}

/// Emits `cursor` (window-logical coordinates) at ~60 Hz while the island is
/// visible. Parked on a condvar the rest of the time.
pub fn spawn_cursor_poll(app: AppHandle, gate: Arc<PollGate>) {
    std::thread::spawn(move || {
        let mut was_down = false;
        // Remembered across wakes so a display change while hidden is noticed the
        // moment the island comes back.
        let mut last_screen: Option<(i32, i32, u32, u32, u64)> = None;
        loop {
            gate.wait_until_active();
            let mut last = (f64::MIN, f64::MIN);
            let mut ticks: u32 = 0;
            while gate.is_active() {
                std::thread::sleep(Duration::from_millis(16));

                // Monitors get plugged in, unplugged, rearranged and rescaled, and
                // an island pinned to coordinates that no longer exist is an island
                // nobody can reach. Checked about twice a second — the cursor poll
                // is already running, so this costs one monitor query.
                ticks = ticks.wrapping_add(1);
                if ticks % 30 == 0 {
                    let now = current_screen_key(&app);
                    if now.is_some() && now != last_screen {
                        let first = last_screen.is_none();
                        last_screen = now;
                        if !first {
                            crate::log::line("display layout changed — repositioning".to_string());
                            let _ = app.emit_to(WINDOW_LABEL, "screen-changed", ());
                        }
                    }
                }

                let Some(win) = window(&app) else { continue };
                let Ok(origin) = win.outer_position() else { continue };
                let scale = win.scale_factor().unwrap_or(1.0);
                let Some((cx, cy)) = cursor_physical() else { continue };
                let x = (cx - origin.x as f64) / scale;
                let y = (cy - origin.y as f64) / scale;
                let size = match win.inner_size() {
                    Ok(s) => (s.width as f64 / scale, s.height as f64 / scale),
                    Err(_) => (PANEL_W, PANEL_H),
                };
                // Click-through: the window only takes the mouse over the island
                // shape. A small entry margin means the flag is already off by the
                // time a moving cursor reaches a button.
                let r = *gate.rect.lock().unwrap();
                let on_island = r.w > 0.0
                    && x >= r.x - HIT_MARGIN
                    && x <= r.x + r.w + HIT_MARGIN
                    && y >= r.y - HIT_MARGIN
                    && y <= r.y + r.h + HIT_MARGIN;

                // The button is read every tick, before the "did the cursor move"
                // shortcut below: a click that never moves the mouse still counts.
                let down = left_button_down();
                let pressed = down && !was_down;
                was_down = down;

                // A file being dragged has to be able to find us. WS_EX_TRANSPARENT
                // — what click-through is on Windows — hides the window from
                // WindowFromPoint, so OLE finds no drop target and shows the "no
                // drop" cursor. macOS has no such problem: AppKit delivers drags to
                // registered destinations whatever ignoresMouseEvents says. So while
                // a button is held anywhere over the panel, the whole panel takes
                // the mouse, which also makes the drop zone as forgiving as the Mac's.
                // A press may be the start of a drag: make sure the drop target is
                // ours before the file arrives.
                if pressed {
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || platform::unblock_webview_drops(&handle));
                    // A press that lands off the island is the page's cue to fold
                    // back to the notch. The window is click-through there, so the
                    // page itself never sees this click.
                    if !on_island {
                        let _ = win.emit("outside-click", ());
                    }
                }

                // Dragging the floating ball: the window follows the cursor until
                // the button comes up, then the page is told whether it was a tap.
                if let Some(released) = drive_ball_drag(&win, &gate, down) {
                    let _ = win.emit("ball-released", released);
                }

                if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 && !down {
                    continue;
                }
                last = (x, y);

                let dragging = down
                    && x >= 0.0
                    && x <= size.0
                    && y >= 0.0
                    && y <= size.1;

                let accept = on_island || dragging;
                if gate.ignoring.load(Ordering::Relaxed) == accept {
                    gate.ignoring.store(!accept, Ordering::Relaxed);
                    let _ = win.set_ignore_cursor_events(!accept);
                }

                let _ = win.emit("cursor", CursorPayload { x, y });
            }
        }
    });
}

/// Re-applies click-through after the window or the island changed shape.
///
/// With the cursor poll (Windows) the window takes the mouse again and the next
/// tick decides from the cursor. Without it (Linux) the input region is set to
/// the island itself, or to the whole wake strip while collapsed.
pub fn refresh_click_through(app: &AppHandle, gate: &PollGate) {
    if platform::CURSOR_POLL {
        set_ignore_cursor(app, false);
        gate.forget_ignore_state();
        return;
    }
    let Some(win) = window(app) else { return };
    let region = if gate.collapsed.load(Ordering::Relaxed) {
        None
    } else {
        let r = *gate.rect.lock().unwrap();
        if r.w <= 0.0 {
            // Nothing drawn yet: nothing takes the mouse.
            Some((0.0, 0.0, 0.0, 0.0))
        } else {
            let x0 = (r.x - HIT_MARGIN).max(0.0);
            let y0 = (r.y - HIT_MARGIN).max(0.0);
            let x1 = r.x + r.w + HIT_MARGIN;
            let y1 = r.y + r.h + HIT_MARGIN;
            Some((x0, y0, x1 - x0, y1 - y0))
        }
    };
    platform::set_input_region(&win, region);
}

pub fn set_ignore_cursor(app: &AppHandle, ignore: bool) {
    if let Some(win) = window(app) {
        let _ = win.set_ignore_cursor_events(ignore);
    }
}
