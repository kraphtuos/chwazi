//! Chwazi-style random finger picker.
//!
//! Everyone puts a finger on the screen; after a short still moment the app
//! randomly chooses one finger, an order, or splits everyone into teams.
//!
//! Rendered on a full-screen `<canvas>` driven by pointer events, so it works
//! with real multi-touch on phones/tablets and with a mouse on desktop.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::TAU;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    CanvasRenderingContext2d, Document, Event, HtmlCanvasElement, HtmlElement, PointerEvent, Window,
};

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Seconds the set of fingers must stay unchanged before a pick fires. The
/// countdown arc sweeps a full circle over exactly this time.
const SELECT_DELAY: f64 = 2.0;
/// Duration of the "pick one" flood flooding in around the winner.
const ANIM_ONE: f64 = 0.6;
/// Duration of the order/teams reveal animation.
const ANIM_OTHER: f64 = 1.4;
/// Solid centre disc radius as a fraction of the ring radius (leaves a gap).
const INNER_FRAC: f64 = 0.68;
/// Ring stroke width as a fraction of the base radius.
const RING_W: f64 = 0.16;
/// Radius of the clear "spotlight" window kept around the winning ring while
/// the rest of the screen floods, as a multiple of the base radius.
const HOLE_FRAC: f64 = 1.85;
/// Rotation speed (rad/s) of the shaded wedge as it grows around each ring.
const SPIN_SPEED: f64 = 4.5;
/// Buffer after each add before the countdown may start. During it the newly
/// added ring gradually grows in from its disc, so a still-settling touch never
/// triggers an instant pick.
const RING_BUILD: f64 = 0.5;
/// Beat the flooded winner is held after release, before the flood exits.
const HOLD_TIME: f64 = 0.8;
/// Duration of the flood receding and the winner dissolving away on release.
const EXIT_ONE: f64 = 0.7;
/// Duration of the quick flood whoosh-out when a result is cancelled by touch.
const EXIT_FAST: f64 = 0.32;
/// Time for a lifted dot to shrink out of the canvas. Driven off an absolute
/// timestamp (not per-frame integration) so it stays smooth even when frames
/// are dropped during the busy lift transition.
const SHRINK_TIME: f64 = 0.38;

/// A single bob drives each finger's disc and ring together, forever. Fingers
/// are desynced from one another by a per-id phase. (amplitude, rad·s⁻¹)
const BOB_AMP: f64 = 0.075;
const BOB_FREQ: f64 = 4.0;

type Rgb = (u8, u8, u8);

/// Distinct, vivid colours assigned to fingers as they touch down. Spread
/// around the hue wheel so any two picked at random stay easy to tell apart.
const FINGER_COLORS: &[Rgb] = &[
    (255, 59, 48),  // red
    (255, 130, 0),  // orange
    (255, 204, 0),  // yellow
    (180, 210, 0),  // chartreuse
    (52, 199, 89),  // green
    (0, 190, 140),  // teal
    (0, 200, 210),  // cyan
    (90, 170, 255), // sky blue
    (0, 110, 255),  // blue
    (90, 80, 230),  // indigo
    (150, 90, 240), // violet
    (200, 70, 230), // purple
    (255, 60, 180), // magenta
    (255, 60, 110), // pink
    (170, 120, 90), // brown
];

/// Colours used to tint each team in "groups" mode.
const GROUP_COLORS: &[Rgb] = &[
    (255, 69, 58),
    (10, 132, 255),
    (48, 209, 88),
    (255, 214, 10),
    (191, 90, 242),
    (255, 159, 10),
    (50, 215, 190),
    (255, 55, 95),
];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    One,
    Order,
    Groups,
}

#[derive(Clone, Copy)]
enum Phase {
    /// No fingers on screen.
    Idle,
    /// Fingers present, counting down to a pick. `changed` = time of last add/remove.
    Gather { changed: f64 },
    /// Playing the selection animation. `start` = time it began.
    Animate { start: f64 },
    /// Selection finished; result on display until fingers lift.
    Result,
    /// Fingers have lifted but the winner's screen lingers. `start` = lift time.
    Hold { start: f64 },
}

struct Finger {
    x: f64,
    y: f64,
    color: Rgb,
    /// Animated ring radius (px), lerped toward a phase-driven target.
    ring: f64,
    /// Opacity 0..1, lerped down as losing rings fall away.
    alpha: f64,
    /// 1-based rank (order mode).
    rank: usize,
    /// 0-based team index (groups mode).
    group: usize,
    /// Time this finger touched down: drives the ring build-in and desyncs bob.
    birth: f64,
    /// Whether the finger is still pressed. When it lifts this flips to false
    /// and the whole object shrinks out (`scale` → 0) before being removed.
    alive: bool,
    /// Time the object began shrinking out. `None` while it should stay on
    /// screen at full size (e.g. a ranked ring frozen during the result hold).
    /// Drives the shrink off absolute time so it stays smooth (`SHRINK_TIME`).
    depart: Option<f64>,
    /// The `scale` at the instant the finger lifted, so the shrink-out eases
    /// from the size it actually had (usually 1, but less if lifted mid-grow).
    depart_scale: f64,
    /// 0..1 overall object scale — grows in on add (snappy exponential ease),
    /// shrinks out analytically from `depart`/`depart_scale` on remove.
    scale: f64,
    /// A "virtual" dot added with the +/- control rather than a real touch, to
    /// work around iOS capping simultaneous touches at 5. It behaves exactly
    /// like a pressed finger for rendering and selection, but never lifts on its
    /// own and does not count toward the round-completion / reset triggers.
    virt: bool,
    /// Angle (rad) the ring build-in grows out from, expanding both ways to a
    /// full circle. Randomised per finger/dot so they don't all unfurl from the
    /// same point.
    birth_angle: f64,
}

struct App {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    ui: HtmlElement,
    gcount_el: HtmlElement,
    dots_el: HtmlElement,
    dcount_el: HtmlElement,

    fingers: HashMap<i32, Finger>,
    phase: Phase,
    mode: Mode,
    groups: usize,
    /// Next id handed to a virtual dot. Counts *down* from -1 so virtual dots
    /// never collide with real pointer ids (which the spec keeps non-negative),
    /// and a more-negative id means a more-recently-added dot.
    next_virt: i32,

    now: f64,    // seconds, updated each frame
    last_t: f64, // seconds

    width: f64,
    height: f64,
    dpr: f64,

    // selection / animation state
    winner: Option<i32>,
    reveal: f64,
    /// Continuously accumulated angle offsetting the countdown arc so it spins.
    spin: f64,
    /// Winner's colour + position, kept so the spotlight-flood can hold on
    /// screen after the winning finger has lifted.
    flood_col: Option<Rgb>,
    flood_pos: Option<(f64, f64)>,
    /// A result's flood being whooshed away because a touch cancelled it, drawn
    /// as an overlay over the fresh round. (start time, colour, centre).
    cancel: Option<(f64, Rgb, (f64, f64))>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------------------
// Entry point (called by the wasm-bindgen init generated by Trunk)
// ---------------------------------------------------------------------------

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let document = document();
    let canvas = document
        .get_element_by_id("c")
        .unwrap()
        .dyn_into::<HtmlCanvasElement>()?;
    let ctx = canvas
        .get_context("2d")?
        .unwrap()
        .dyn_into::<CanvasRenderingContext2d>()?;
    let ui = document
        .get_element_by_id("ui")
        .unwrap()
        .dyn_into::<HtmlElement>()?;
    let gcount_el = document
        .get_element_by_id("gcount")
        .unwrap()
        .dyn_into::<HtmlElement>()?;
    let dots_el = document
        .get_element_by_id("dots")
        .unwrap()
        .dyn_into::<HtmlElement>()?;
    let dcount_el = document
        .get_element_by_id("dcount")
        .unwrap()
        .dyn_into::<HtmlElement>()?;

    let mut app = App {
        canvas: canvas.clone(),
        ctx,
        ui,
        gcount_el,
        dots_el,
        dcount_el,
        fingers: HashMap::new(),
        phase: Phase::Idle,
        mode: Mode::One,
        groups: 2,
        next_virt: -1,
        now: 0.0,
        last_t: 0.0,
        width: 0.0,
        height: 0.0,
        dpr: 1.0,
        winner: None,
        reveal: 0.0,
        spin: 0.0,
        flood_col: None,
        flood_pos: None,
        cancel: None,
    };
    app.resize();
    APP.with(|a| *a.borrow_mut() = Some(app));

    install_pointer_handlers(&canvas);
    install_ui_handlers()?;
    install_window_handlers();
    start_raf();
    Ok(())
}

// ---------------------------------------------------------------------------
// App logic
// ---------------------------------------------------------------------------

impl App {
    fn base_r(&self) -> f64 {
        (self.width.min(self.height) * 0.11).clamp(46.0, 70.0)
    }

    /// Fingers still pressed (ignores ones currently shrinking out). Counts both
    /// real touches and virtual dots, since both are participants in a pick.
    fn active_count(&self) -> usize {
        self.fingers.values().filter(|f| f.alive).count()
    }

    /// Real (non-virtual) fingers still pressed. The round-completion and reset
    /// triggers key off this, not `active_count`: virtual dots never lift on
    /// their own, so counting them would stop a round ever finishing.
    fn real_alive(&self) -> usize {
        self.fingers.values().filter(|f| f.alive && !f.virt).count()
    }

    /// Alive virtual dots currently staged.
    fn virt_count(&self) -> usize {
        self.fingers.values().filter(|f| f.alive && f.virt).count()
    }

    fn update_dcount(&self) {
        self.dcount_el
            .set_inner_text(&self.virt_count().to_string());
    }

    /// A spread-out spot for a new virtual dot: away from the screen edges and
    /// the top/bottom control bars, and as far as possible from existing dots so
    /// the added rings don't pile up.
    fn free_spot(&self) -> (f64, f64) {
        let base = self.base_r();
        let m = base * 2.2; // clear of the side edges
        let top = base * 3.0 + 60.0; // clear of the mode control bar
        let bot = base * 3.0 + 60.0; // clear of the +/- dots control bar
        let x0 = m;
        let x1 = (self.width - m).max(x0 + 1.0);
        let y0 = top;
        let y1 = (self.height - bot).max(y0 + 1.0);
        let mut best = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let mut best_d = -1.0;
        for _ in 0..40 {
            let x = x0 + rnd() * (x1 - x0);
            let y = y0 + rnd() * (y1 - y0);
            let d = self
                .fingers
                .values()
                .map(|f| (f.x - x).hypot(f.y - y))
                .fold(f64::INFINITY, f64::min);
            if d > best_d {
                best_d = d;
                best = (x, y);
            }
            if d > base * 3.0 {
                break; // comfortably spaced — good enough
            }
        }
        best
    }

    /// Add a virtual dot (the iOS >5-touch workaround). It behaves like a pressed
    /// finger from here on, participating in every pick until removed.
    fn add_virtual(&mut self) {
        // Cap so colours stay distinct and the screen doesn't fill with dots.
        if self.virt_count() >= 8 {
            return;
        }
        let (x, y) = self.free_spot();
        let color = self.pick_color();
        let base = self.base_r();
        let id = self.next_virt;
        self.next_virt -= 1;
        self.fingers.insert(
            id,
            Finger {
                x,
                y,
                color,
                ring: base,
                alpha: 1.0,
                rank: 0,
                group: 0,
                birth: self.now,
                alive: true,
                depart: None,
                depart_scale: 1.0,
                scale: 0.0,
                virt: true,
                birth_angle: rnd() * TAU,
            },
        );
        // Restart the countdown so adding a dot never triggers an instant pick.
        self.phase = Phase::Gather { changed: self.now };
        self.update_dcount();
    }

    /// Remove the most recently added virtual dot, shrinking it out.
    fn remove_virtual(&mut self) {
        // Most-negative id == most recently added.
        let target = self
            .fingers
            .iter()
            .filter(|(_, f)| f.virt && f.alive)
            .map(|(id, _)| *id)
            .min();
        let Some(id) = target else {
            return; // nothing staged — leave a running countdown alone
        };
        if let Some(f) = self.fingers.get_mut(&id) {
            f.alive = false;
            f.depart = Some(self.now);
            f.depart_scale = f.scale;
        }
        self.phase = if self.fingers.values().any(|f| f.alive) {
            Phase::Gather { changed: self.now }
        } else {
            Phase::Idle
        };
        self.update_dcount();
    }

    /// The current spotlight-hole radius of the Pick One flood, mirroring the
    /// geometry in `draw_flood`. Used to hand the flood off to a cancel overlay
    /// at exactly its current size so a touch mid-exit doesn't make it jump.
    fn flood_hole(&self) -> f64 {
        let base = self.base_r();
        let cover = self.width.hypot(self.height);
        let wr = base * (1.0 + 0.25 * self.reveal.clamp(0.0, 1.0));
        let spot_r = wr * HOLE_FRAC;
        match self.phase {
            Phase::Hold { start } => {
                let t = self.now - start;
                if t < HOLD_TIME {
                    spot_r
                } else {
                    let e = ((t - HOLD_TIME) / EXIT_ONE).clamp(0.0, 1.0);
                    spot_r + (cover - spot_r) * smoothstep(e)
                }
            }
            _ => {
                let r = self.reveal.clamp(0.0, 1.0);
                let fp = 1.0 - (1.0 - r) * (1.0 - r);
                cover + (spot_r - cover) * fp
            }
        }
    }

    fn resize(&mut self) {
        // Backing store = the element's actual displayed size × dpr, so the
        // canvas is always pixel-crisp. The element is stretched to the screen
        // by CSS; we just match its real client size here.
        let w = self.canvas.client_width() as f64;
        let h = self.canvas.client_height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let dpr = window().device_pixel_ratio().max(1.0);
        self.width = w;
        self.height = h;
        self.dpr = dpr;
        self.canvas.set_width((w * dpr).round() as u32);
        self.canvas.set_height((h * dpr).round() as u32);
    }

    fn pick_color(&self) -> Rgb {
        // Choose at random among the colours not currently in use (including by
        // fingers still shrinking out), so touches never collide while any are
        // free and successive rounds vary.
        let free: Vec<Rgb> = FINGER_COLORS
            .iter()
            .filter(|c| !self.fingers.values().any(|f| f.color == **c))
            .copied()
            .collect();
        if !free.is_empty() {
            free[rnd_idx(free.len())]
        } else {
            FINGER_COLORS[rnd_idx(FINGER_COLORS.len())]
        }
    }

    fn on_down(&mut self, id: i32, x: f64, y: f64) {
        // A touch during the flood (running, shown, or holding) cancels the
        // result: the flood whooshes away quickly as an overlay while this touch
        // begins a fresh round — instead of the pick vanishing instantly.
        if matches!(
            self.phase,
            Phase::Animate { .. } | Phase::Result | Phase::Hold { .. }
        ) {
            if let (Some(col), Some(pos)) = (self.flood_col, self.flood_pos) {
                // Hand the flood off to the cancel overlay at its *current* hole
                // size, so a touch while the flood is already receding (e.g. the
                // Hold-timeout exit) continues opening outward instead of jumping
                // back to the small spotlight and re-expanding. Back-date the
                // overlay's start so its progress matches the flood right now.
                let cover = self.width.hypot(self.height);
                let spot_c = self.base_r() * 1.25 * HOLE_FRAC;
                let s_val = ((self.flood_hole() - spot_c) / (cover - spot_c)).clamp(0.0, 1.0);
                let t0 = self.now - inv_smoothstep(s_val) * EXIT_FAST;
                self.cancel = Some((t0, col, pos));
            }
            self.reset_idle();
        }
        let color = self.pick_color();
        let base = self.base_r();
        self.fingers.insert(
            id,
            Finger {
                x,
                y,
                color,
                ring: base,
                alpha: 1.0,
                rank: 0,
                group: 0,
                birth: self.now,
                alive: true,
                depart: None,
                depart_scale: 1.0,
                scale: 0.0,
                virt: false,
                birth_angle: rnd() * TAU,
            },
        );
        self.phase = Phase::Gather { changed: self.now };
    }

    /// Clear all selection state back to a fresh idle screen. Fingers still
    /// pressed and staged virtual dots are kept — both are participants of the
    /// next round (so a touch that cancels a result doesn't silently drop the
    /// fingers still on the glass) — and reset to a clean, fully-grown resting
    /// state. Everything lifted or mid-removal goes.
    fn reset_idle(&mut self) {
        let base = self.base_r();
        self.fingers.retain(|_, f| f.alive);
        for f in self.fingers.values_mut() {
            f.alive = true;
            f.depart = None;
            f.depart_scale = 1.0;
            f.scale = 1.0;
            f.alpha = 1.0;
            f.ring = base;
            f.rank = 0;
            f.group = 0;
        }
        // With dots still staged the screen is really "waiting for fingers", so
        // stay in Gather (keeps their rings in the same light-track look as
        // during a gather, and the countdown stays gated on a real finger).
        self.phase = if self.fingers.is_empty() {
            Phase::Idle
        } else {
            Phase::Gather { changed: self.now }
        };
        self.winner = None;
        self.reveal = 0.0;
        self.flood_col = None;
        self.flood_pos = None;
        self.update_dcount();
    }

    fn on_move(&mut self, id: i32, x: f64, y: f64) {
        // Positions lock in once a pick is running or shown — the winner (and
        // everyone else) stops tracking the finger.
        if !matches!(self.phase, Phase::Idle | Phase::Gather { .. }) {
            return;
        }
        if let Some(f) = self.fingers.get_mut(&id) {
            f.x = x;
            f.y = y;
        }
    }

    fn on_up(&mut self, id: i32) {
        // Order / Teams with a result on screen: the ring must stay put as the
        // finger lifts, so its rank/team number remains readable. The shrink-out
        // is played later, after the hold (see `frame`). Otherwise the object
        // starts shrinking away immediately.
        let showing = matches!(self.mode, Mode::Order | Mode::Groups)
            && matches!(self.phase, Phase::Animate { .. } | Phase::Result);

        // Mark the finger as departing rather than removing it outright, so its
        // object can shrink out of the canvas. It is cleaned up once `scale`
        // reaches ~0 (see `update_targets`).
        let existed = match self.fingers.get_mut(&id) {
            Some(f) if f.alive => {
                f.alive = false;
                if !showing {
                    f.depart = Some(self.now);
                    f.depart_scale = f.scale;
                }
                true
            }
            _ => false,
        };
        if !existed {
            return;
        }
        if self.real_alive() == 0 {
            // If a winner has been chosen, leave the pick playing: frame()
            // carries Animate → Result → Hold once no fingers remain, so a flood
            // caught mid-way still finishes filling in, holds, then exits.
            let decided = self.mode == Mode::One
                && self.flood_col.is_some()
                && matches!(self.phase, Phase::Animate { .. } | Phase::Result);
            if decided {
                // nothing to do — the animation continues on its own.
            } else if showing {
                // Everyone has lifted with the order/teams result up: hold it on
                // screen so it can be read, then play the exit.
                self.phase = Phase::Hold { start: self.now };
            } else {
                // Back to idle, but leave the departing fingers in place to
                // finish shrinking out. If dots are still staged, stay in Gather
                // (not Idle): Idle draws rings in the solid saturated style, so
                // the remaining dots would visibly shade over on the lift.
                self.phase = if self.fingers.values().any(|f| f.alive) {
                    Phase::Gather { changed: self.now }
                } else {
                    Phase::Idle
                };
                self.winner = None;
                self.reveal = 0.0;
                self.flood_col = None;
                self.flood_pos = None;
            }
        } else if matches!(self.phase, Phase::Idle | Phase::Gather { .. }) {
            // Removing a finger resets the countdown (unless a pick is locked in).
            self.phase = Phase::Gather { changed: self.now };
        }
    }

    fn set_mode(&mut self, m: Mode) {
        self.mode = m;
        self.winner = None;
        self.reveal = 0.0;
        self.flood_col = None;
        self.flood_pos = None;
        for f in self.fingers.values_mut() {
            f.rank = 0;
            f.group = 0;
        }
        self.phase = if self.fingers.is_empty() {
            Phase::Idle
        } else {
            Phase::Gather { changed: self.now }
        };
    }

    fn change_groups(&mut self, d: i32) {
        self.groups = (self.groups as i32 + d).clamp(2, 8) as usize;
        self.gcount_el.set_inner_text(&self.groups.to_string());
        self.winner = None;
        self.reveal = 0.0;
        self.flood_col = None;
        self.flood_pos = None;
        self.phase = if self.fingers.is_empty() {
            Phase::Idle
        } else {
            Phase::Gather { changed: self.now }
        };
    }

    fn begin_selection(&mut self, now: f64) {
        let mut ids: Vec<i32> = self
            .fingers
            .iter()
            .filter(|(_, f)| f.alive)
            .map(|(id, _)| *id)
            .collect();
        if ids.len() < 2 {
            return;
        }
        self.flood_col = None;
        self.flood_pos = None;
        match self.mode {
            Mode::One => {
                let w = ids[rnd_idx(ids.len())];
                self.winner = Some(w);
                if let Some(f) = self.fingers.get(&w) {
                    self.flood_col = Some(f.color);
                    self.flood_pos = Some((f.x, f.y));
                }
            }
            Mode::Order => {
                shuffle(&mut ids);
                for (i, id) in ids.iter().enumerate() {
                    if let Some(f) = self.fingers.get_mut(id) {
                        f.rank = i + 1;
                    }
                }
            }
            Mode::Groups => {
                shuffle(&mut ids);
                let g = self.groups.max(1);
                for (i, id) in ids.iter().enumerate() {
                    if let Some(f) = self.fingers.get_mut(id) {
                        f.group = i % g;
                    }
                }
            }
        }
        self.reveal = 0.0;
        self.phase = Phase::Animate { start: now };
    }

    fn frame(&mut self, now_ms: f64) {
        let now = now_ms * 0.001;
        let dt = if self.last_t == 0.0 {
            0.016
        } else {
            (now - self.last_t).clamp(0.0, 0.05)
        };
        self.last_t = now;
        self.now = now;

        // iOS standalone can settle to its final size a moment after launch (and
        // report it via no reliable event), so poll the element size cheaply
        // each frame and re-fit whenever it changes.
        let cw = self.canvas.client_width() as f64;
        let ch = self.canvas.client_height() as f64;
        if (cw - self.width).abs() > 0.5 || (ch - self.height).abs() > 0.5 {
            self.resize();
        }

        // Drop the cancel-flood overlay once it has finished whooshing out.
        if let Some((t0, _, _)) = self.cancel {
            if now - t0 >= EXIT_FAST {
                self.cancel = None;
            }
        }

        match self.phase {
            Phase::Gather { changed } => {
                // The ring build-in buffer must elapse before the countdown, so
                // the effective wait is `RING_BUILD + SELECT_DELAY`. At least one
                // real finger must be down: staged virtual dots on their own must
                // not run a pick (nobody is there to see or start it).
                if self.active_count() >= 2
                    && self.real_alive() >= 1
                    && now - changed >= RING_BUILD + SELECT_DELAY
                {
                    self.begin_selection(now);
                }
            }
            Phase::Animate { start } => {
                let dur = if self.mode == Mode::One {
                    ANIM_ONE
                } else {
                    ANIM_OTHER
                };
                if now - start >= dur {
                    self.phase = Phase::Result;
                    self.reveal = 1.0;
                }
            }
            Phase::Result => {
                // Everyone already lifted (the flood finished on its own): begin
                // the post-win hold, which then plays the exit.
                if self.mode == Mode::One && self.flood_col.is_some() && self.real_alive() == 0 {
                    self.phase = Phase::Hold { start: now };
                }
            }
            Phase::Hold { start } => {
                if self.mode == Mode::One {
                    // The flood recedes and the winner dissolves (see draw_flood).
                    if now - start >= HOLD_TIME + EXIT_ONE {
                        self.reset_idle();
                    }
                } else {
                    // Order / Teams: the ranked rings sat readable through the
                    // hold; now shrink them all out together. Dating the exit
                    // from `start + HOLD_TIME` (not `now`) keeps the timeline
                    // exact regardless of which frame this lands on.
                    if now - start >= HOLD_TIME {
                        for f in self.fingers.values_mut() {
                            // Virtual dots persist across rounds, so don't shrink
                            // them out here — reset_idle resets them in place.
                            if f.depart.is_none() && !f.virt {
                                f.depart = Some(start + HOLD_TIME);
                                f.depart_scale = f.scale;
                            }
                        }
                    }
                    if now - start >= HOLD_TIME + SHRINK_TIME {
                        self.reset_idle();
                    }
                }
            }
            _ => {}
        }

        self.update_targets(dt);
        self.update_ui();
        self.draw();

        // Dev builds only (off under `--release`): show when this WASM was built
        // so a stale cache is obvious at a glance on-device.
        #[cfg(debug_assertions)]
        self.draw_build_stamp();
    }

    fn update_targets(&mut self, dt: f64) {
        let base = self.base_r();
        let now = self.now;
        let phase = self.phase;
        let mode = self.mode;
        let winner = self.winner;

        // Progress of the flood expanding out from the winner (0..1).
        if let Phase::Animate { start } = phase {
            let dur = if mode == Mode::One {
                ANIM_ONE
            } else {
                ANIM_OTHER
            };
            self.reveal = ((now - start) / dur).clamp(0.0, 1.0);
        }
        let reveal = self.reveal;

        // Steady rotation offsetting each ring's countdown arc so it clearly
        // spins while filling.
        self.spin += SPIN_SPEED * dt;

        let s = (dt * 12.0).min(1.0);
        let mut dead: Vec<i32> = Vec::new();
        for (id, f) in self.fingers.iter_mut() {
            // A departing (lifted) finger just holds its last look and lets the
            // shrinking `scale` carry it off screen.
            let (tr, ta) = if !f.alive {
                (f.ring, f.alpha)
            } else {
                match phase {
                    Phase::Idle | Phase::Gather { .. } => (base, 1.0),
                    Phase::Animate { .. } => match mode {
                        Mode::One => {
                            if Some(*id) == winner {
                                (base, 1.0)
                            } else {
                                // losers drop away quickly (gone by ~1/3 in)
                                (base * (1.0 - 0.5 * reveal), (1.0 - 3.0 * reveal).max(0.0))
                            }
                        }
                        Mode::Order => (
                            if reveal > 0.0 && f.rank == 1 {
                                base * 1.28
                            } else {
                                base
                            },
                            1.0,
                        ),
                        Mode::Groups => (base, 1.0),
                    },
                    Phase::Result => match mode {
                        Mode::One => (base, if Some(*id) == winner { 1.0 } else { 0.0 }),
                        Mode::Order => (if f.rank == 1 { base * 1.28 } else { base }, 1.0),
                        Mode::Groups => (base, 1.0),
                    },
                    Phase::Hold { .. } => (f.ring, f.alpha),
                }
            };
            f.ring += (tr - f.ring) * s;
            f.alpha += (ta - f.alpha) * s;

            // Grow in on add (unchanged: snappy exponential ease). Shrink out on
            // remove is driven off an absolute timestamp instead — recomputed
            // from a fixed timeline each frame, so a dropped frame during the
            // busy lift transition can't accumulate into visible stutter (as the
            // old per-frame `scale += (0 - scale) * dt·12` decay did). Mirrors
            // the winner's dissolve in `draw_flood`.
            match f.depart {
                Some(dep) => {
                    let out = smoothstep(((now - dep) / SHRINK_TIME).clamp(0.0, 1.0));
                    f.scale = f.depart_scale * (1.0 - out);
                    if now - dep >= SHRINK_TIME {
                        dead.push(*id);
                    }
                }
                None => f.scale += (1.0 - f.scale) * s,
            }
        }
        for id in dead {
            self.fingers.remove(&id);
        }

        // Keep the spotlight hole tracking the winning finger until it lifts.
        if mode == Mode::One {
            if let Some(f) = winner.and_then(|w| self.fingers.get(&w)) {
                self.flood_pos = Some((f.x, f.y));
            }
        }
    }

    fn update_ui(&self) {
        // A pick is animating or on display: hide every control.
        let busy = matches!(
            self.phase,
            Phase::Animate { .. } | Phase::Result | Phase::Hold { .. }
        );
        // The mode bar hides as soon as a real finger is down (staged virtual
        // dots alone keep it up, so you can still add dots / switch mode). The
        // dots stepper stays available right through the gather countdown, so an
        // extra dot can be added even after fingers are down — the whole point
        // of the workaround — and only hides while a pick is showing.
        let real_down = self.fingers.values().any(|f| f.alive && !f.virt);
        set_hidden(&self.ui, busy || real_down);
        set_hidden(&self.dots_el, busy);
    }

    // ---- rendering --------------------------------------------------------

    fn draw(&self) {
        let ctx = &self.ctx;
        let _ = ctx.set_transform(self.dpr, 0.0, 0.0, self.dpr, 0.0, 0.0);
        ctx.clear_rect(0.0, 0.0, self.width, self.height);

        // Pick One, once decided: the winner's colour floods the screen but
        // leaves a clear round "spotlight" so the winning ring stays visible.
        // This holds after the fingers lift, then fades out.
        if self.mode == Mode::One
            && matches!(
                self.phase,
                Phase::Animate { .. } | Phase::Result | Phase::Hold { .. }
            )
        {
            if let (Some(col), Some((wx, wy))) = (self.flood_col, self.flood_pos) {
                self.draw_flood(wx, wy, col);
                return;
            }
        }

        if matches!(self.phase, Phase::Idle) && self.fingers.is_empty() {
            self.draw_hint();
        }

        // While gathering, each ring starts fully in the lighter colour and the
        // shaded (saturated) area grows to a full circle as the countdown runs —
        // but only once at least two fingers are down. The instant it completes,
        // the pick fires. `Some(fraction)` means "draw the lighter track"; the
        // fraction is 0 until there are ≥2 fingers.
        let count = match self.phase {
            Phase::Gather { changed } => {
                // The wedge stays empty through the ring build-in buffer, then
                // sweeps to full over `SELECT_DELAY`.
                let cd = if self.active_count() >= 2 && self.real_alive() >= 1 {
                    ((self.now - changed - RING_BUILD) / SELECT_DELAY).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                Some(cd)
            }
            _ => None,
        };

        for id in self.fingers.keys().copied() {
            self.draw_finger(id, count);
        }

        // A cancelled result whooshes away on top of the fresh round.
        if let Some((t0, col, (cx, cy))) = self.cancel {
            self.draw_cancel(t0, col, cx, cy);
        }
    }

    /// A cancelled result's flood, whooshing back out to the edges. Full-opacity
    /// fill receding purely by the growing hole — identical to the Hold-timeout
    /// exit in `draw_flood`, so a touch that hands the flood off mid-exit is
    /// seamless (no brightness pop) as well as continuous in hole size.
    fn draw_cancel(&self, t0: f64, col: Rgb, cx: f64, cy: f64) {
        let e = ((self.now - t0) / EXIT_FAST).clamp(0.0, 1.0);
        let cover = self.width.hypot(self.height);
        let spot_r = self.base_r() * 1.25 * HOLE_FRAC;
        let hole = spot_r + (cover - spot_r) * smoothstep(e);
        if hole >= cover {
            return;
        }
        let ctx = &self.ctx;
        ctx.save();
        ctx.set_fill_style_str(&rgba(col, 1.0));
        ctx.begin_path();
        ctx.rect(0.0, 0.0, self.width, self.height);
        if hole > 1.0 {
            ctx.move_to(cx + hole, cy);
            let _ = ctx.arc_with_anticlockwise(cx, cy, hole, 0.0, TAU, true);
        }
        ctx.fill();
        ctx.restore();
    }

    /// Draw one finger's disc + gap + ring. `count` is the countdown fraction
    /// (`Some` while gathering) that grows the shaded wedge toward a full circle.
    fn draw_finger(&self, id: i32, count: Option<f64>) {
        let f = &self.fingers[&id];
        if f.alpha * f.scale <= 0.01 {
            return;
        }
        let base = self.base_r();
        let col = if self.mode == Mode::Groups && self.reveal > 0.0 {
            lerp_col(
                f.color,
                GROUP_COLORS[f.group % GROUP_COLORS.len()],
                self.reveal,
            )
        } else {
            f.color
        };

        // How far the ring has grown in from its disc since this finger landed.
        let ring_in = smoothstep(((self.now - f.birth) / RING_BUILD).clamp(0.0, 1.0));

        // (wedge start angle, countdown fraction). The start angle spins, so the
        // growing shaded wedge also rotates. Desynced per finger.
        //
        // The angle MUST be reduced modulo a full turn before it reaches
        // ctx.arc(): iOS reports enormous pointer ids (~1.7e9), so `id * 2.3`
        // is an astronomically large angle that Safari's canvas fails to draw —
        // the wedge then never renders, grows, or spins. `rem_euclid(TAU)` keeps
        // it small while preserving both the spin and the per-finger offset.
        // A finger lifted *before* a pick keeps the normal light-track ring —
        // just without the countdown wedge and a touch faded — as it shrinks
        // out. Forcing the light track (`Some` with a zero wedge) matters once
        // the last finger lifts and the phase flips to Idle: then `count` is
        // None, which would otherwise draw the solid *saturated* ring, making
        // the ring suddenly match the central disc's colour on the way out.
        // Once a result is on screen the ring instead keeps its result look all
        // the way out, so the ranked/team rings don't go pale as they leave.
        let (arc, alpha) = if f.depart.is_some() && self.reveal <= 0.0 {
            (Some((0.0, 0.0)), f.alpha * 0.7)
        } else {
            (
                count.map(|c| ((self.spin + id as f64 * 2.3).rem_euclid(TAU), c)),
                f.alpha,
            )
        };
        self.draw_ring(
            f.x,
            f.y,
            f.ring,
            col,
            alpha,
            f.scale,
            ring_in,
            id as f64,
            f.birth_angle,
            arc,
        );

        // Rank number for order mode, centred in the disc and big enough to read
        // at a glance. It sits under the player's own fingertip while they press,
        // which is fine — the post-release hold is when everyone reads it.
        // Teams needs no number: the team colour already says it.
        if self.reveal > 0.0 && self.mode == Mode::Order && f.rank > 0 {
            let sz = (base * 0.9 * f.scale) as i32;
            let ctx = &self.ctx;
            ctx.save();
            ctx.set_global_alpha(f.alpha * self.reveal * f.scale);
            ctx.set_text_align("center");
            ctx.set_text_baseline("middle");
            ctx.set_fill_style_str("rgba(255,255,255,0.98)");
            ctx.set_font(&format!("700 {sz}px system-ui, sans-serif"));
            let _ = ctx.fill_text(&f.rank.to_string(), f.x, f.y);
            ctx.restore();
        }
    }

    /// Solid centre disc, a gap, then the ring. `arc` is `(start_angle,
    /// countdown_fraction)`: the ring starts fully in the lighter tint and a
    /// shaded (saturated) wedge grows from the start angle to a full circle as
    /// the countdown runs, the start angle spinning so the wedge also rotates.
    #[allow(clippy::too_many_arguments)]
    fn draw_ring(
        &self,
        x: f64,
        y: f64,
        r: f64,
        col: Rgb,
        alpha: f64,
        scale: f64,
        ring_in: f64,
        seed: f64,
        mid: f64,
        arc: Option<(f64, f64)>,
    ) {
        if alpha * scale <= 0.01 {
            return;
        }
        let ctx = &self.ctx;
        let base = self.base_r();

        // One bob drives the disc and the ring together, seeded per pointer so
        // fingers stay out of step with each other. (The seed is wrapped so
        // iOS's ~1.7e9 ids keep the sine argument well-conditioned.) The
        // amplitude scales with the object's own `scale`, so the bob ramps in as
        // it grows and — crucially — damps to nothing as it shrinks out, keeping
        // the exit a smooth, monotonic shrink instead of a wobble that reads as
        // stutter.
        let bob =
            1.0 + BOB_AMP * scale * (self.now * BOB_FREQ + (seed * 2.3).rem_euclid(TAU)).sin();

        // Floors at 0 (not a positive minimum) so a shrinking object glides all
        // the way to nothing instead of stalling at a few pixels — the object is
        // culled by the alpha·scale check above once it's effectively gone.
        let r = (r * scale).max(0.0);
        let lw = ((base * RING_W).max(4.0) * scale).max(0.0);
        let disc_r = (r * INNER_FRAC * bob).max(0.0);
        let ring_r = r * bob;

        // The ring draws itself on from a single point (`mid`), extending both
        // ways until it closes into a full circle (`ring_in`: 0 → 1).
        let span = ring_in * TAU;
        let (a0, a1) = (mid - span / 2.0, mid + span / 2.0);

        ctx.save();
        ctx.set_global_alpha(alpha * scale);

        // Solid filled centre disc (always the saturated colour).
        ctx.set_fill_style_str(&rgba(col, 1.0));
        ctx.begin_path();
        let _ = ctx.arc(x, y, disc_r, 0.0, TAU);
        ctx.fill();

        // Round caps only while the ring is a partial arc (building in). A full
        // circle uses butt caps so the two ends meet cleanly instead of stacking
        // a round cap that reads as a bump — especially as the ring shrinks out.
        let cap = if ring_in < 0.999 { "round" } else { "butt" };
        match arc {
            Some((start, cd)) => {
                // Lighter "track" — sweeps on from the point as the ring builds.
                let light = lerp_col(col, (255, 255, 255), 0.55);
                ctx.set_line_cap(cap);
                ctx.set_line_width(lw);
                ctx.set_stroke_style_str(&rgba(light, 1.0));
                ctx.begin_path();
                let _ = ctx.arc(x, y, ring_r, a0, a1);
                ctx.stroke();

                // The shaded (saturated) wedge grows 0 → full over the countdown
                // while its whole span rotates. Rounded ends look best here.
                if cd > 0.0015 {
                    let len = TAU * cd.min(1.0);
                    ctx.set_line_cap("round");
                    ctx.set_line_width(lw);
                    ctx.set_stroke_style_str(&rgba(col, 1.0));
                    ctx.begin_path();
                    let _ = ctx.arc(x, y, ring_r, start, start + len);
                    ctx.stroke();
                }
                ctx.set_line_cap("butt");
            }
            None => {
                // Solid saturated ring (settled) — also draws on both ways while
                // building in.
                ctx.set_line_cap(cap);
                ctx.set_line_width(lw);
                ctx.set_stroke_style_str(&rgba(col, 1.0));
                ctx.begin_path();
                let _ = ctx.arc(x, y, ring_r, a0, a1);
                ctx.stroke();
                ctx.set_line_cap("butt");
            }
        }

        ctx.restore();
    }

    /// The Pick One spotlight-flood. The winner's colour *zooms* out from the
    /// finger to fill the screen (accelerating), then a clear round spotlight
    /// irises open around the winning ring. Holds, then fades, after lift.
    fn draw_flood(&self, wx: f64, wy: f64, col: Rgb) {
        let ctx = &self.ctx;
        let base = self.base_r();
        let cover = self.width.hypot(self.height);
        let seed = self.winner.unwrap_or(0) as f64;
        let wr = base * (1.0 + 0.25 * self.reveal.clamp(0.0, 1.0));
        let spot_r = wr * HOLE_FRAC;

        // The whole thing is one clear "spotlight" circle punched out of a
        // full-screen colour fill. As it shrinks from covering everything down
        // to just the ring, the colour appears to flood IN from the edges. On
        // release it opens back out to the edges and the colour recedes. A
        // single smooth interpolation for both, so neither stutters.
        let (hole, ws) = match self.phase {
            Phase::Hold { start } => {
                let t = self.now - start;
                if t < HOLD_TIME {
                    // Hold the full flood: winner sits visible and bobbing.
                    (spot_r, 1.0)
                } else {
                    // Flood and winner recede together, one smooth curve. The
                    // bob is damped by `scale` (see draw_ring), so this shrink
                    // stays monotonic.
                    let e = ((t - HOLD_TIME) / EXIT_ONE).clamp(0.0, 1.0);
                    let k = smoothstep(e);
                    (spot_r + (cover - spot_r) * k, 1.0 - k)
                }
            }
            _ => {
                // Flood in from the edges; ease-out so it starts sooner.
                let r = self.reveal.clamp(0.0, 1.0);
                let fp = 1.0 - (1.0 - r) * (1.0 - r);
                (cover + (spot_r - cover) * fp, 1.0)
            }
        };

        // Losing rings sit under the incoming colour and get swallowed edge-in.
        if !matches!(self.phase, Phase::Hold { .. }) {
            for (id, f) in self.fingers.iter() {
                if Some(*id) != self.winner {
                    self.draw_ring(
                        f.x,
                        f.y,
                        f.ring,
                        f.color,
                        f.alpha,
                        f.scale,
                        1.0,
                        *id as f64,
                        f.birth_angle,
                        None,
                    );
                }
            }
        }

        // Full-screen colour with the clear spotlight punched out (rect wound
        // one way, hole the other ⇒ nonzero-rule cut-out).
        ctx.save();
        ctx.set_fill_style_str(&rgba(col, 1.0));
        ctx.begin_path();
        ctx.rect(0.0, 0.0, self.width, self.height);
        if hole > 1.0 {
            ctx.move_to(wx + hole, wy);
            let _ = ctx.arc_with_anticlockwise(wx, wy, hole, 0.0, TAU, true);
        }
        ctx.fill();
        ctx.restore();

        // The winning ring sits ON TOP, in the clear spotlight, still bobbing.
        if ws > 0.01 {
            self.draw_ring(
                wx,
                wy,
                wr,
                col,
                1.0,
                ws,
                1.0,
                seed,
                -std::f64::consts::FRAC_PI_2,
                None,
            );
        }
    }

    fn draw_hint(&self) {
        let ctx = &self.ctx;
        let (cx, cy) = (self.width / 2.0, self.height / 2.0);
        ctx.save();
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.set_fill_style_str("rgba(255,255,255,0.85)");
        ctx.set_font("600 26px system-ui, sans-serif");
        let _ = ctx.fill_text("Everyone, place a finger", cx, cy - 14.0);
        ctx.set_fill_style_str("rgba(255,255,255,0.5)");
        ctx.set_font("400 17px system-ui, sans-serif");
        let _ = ctx.fill_text("Hold still \u{2014} it picks after a moment", cx, cy + 20.0);
        ctx.restore();
    }

    /// Small dev-only overlay showing when this build was produced (local time),
    /// so you can tell at a glance whether the device is running fresh WASM.
    #[cfg(debug_assertions)]
    fn draw_build_stamp(&self) {
        let secs: f64 = env!("BUILD_UNIX").parse().unwrap_or(0.0);
        let d = js_sys::Date::new(&JsValue::from_f64(secs * 1000.0));
        let ih = window()
            .inner_height()
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        // Where the canvas actually sits on screen, so the stamp can be pinned
        // to the visible viewport edges even though the canvas is overscanned.
        let rect = self.canvas.get_bounding_client_rect();
        let s = format!(
            "dev {:02}:{:02}:{:02}",
            d.get_hours(),
            d.get_minutes(),
            d.get_seconds(),
        );
        let ctx = &self.ctx;
        ctx.save();
        ctx.set_text_align("left");
        ctx.set_text_baseline("bottom");
        ctx.set_fill_style_str("rgba(255,255,255,0.6)");
        ctx.set_font("500 11px ui-monospace, Menlo, monospace");
        // Placed against the VISIBLE viewport edges (the canvas is overscanned,
        // so its own edges are off-screen). `-rect.*` converts viewport 0 into
        // canvas coordinates; extra offset clears the home indicator.
        let x = 10.0 - rect.left();
        let y = ih - rect.top() - 44.0;
        let _ = ctx.fill_text(&s, x, y);
        ctx.restore();
    }
}

// ---------------------------------------------------------------------------
// Event wiring
// ---------------------------------------------------------------------------

fn install_pointer_handlers(canvas: &HtmlCanvasElement) {
    let point = |canvas: &HtmlCanvasElement, e: &PointerEvent| -> (i32, f64, f64) {
        let rect = canvas.get_bounding_client_rect();
        (
            e.pointer_id(),
            e.client_x() as f64 - rect.left(),
            e.client_y() as f64 - rect.top(),
        )
    };

    let c = canvas.clone();
    add_listener(canvas, "pointerdown", move |e: PointerEvent| {
        e.prevent_default();
        let (id, x, y) = point(&c, &e);
        if e.pointer_type() == "touch" {
            // The browser applies *implicit* pointer capture to the target on
            // touchdown. On iOS Safari (and others) that implicit capture
            // serialises pointers to a single stream and blocks simultaneous
            // multi-touch: the second finger onward is never delivered, so the
            // two-finger countdown (and its spinning ring) never starts.
            // Explicitly *release* it so each touch is reported independently.
            // The full-screen canvas still receives every touch's move/up
            // without capture, since the finger stays over it.
            if c.has_pointer_capture(id) {
                let _ = c.release_pointer_capture(id);
            }
        } else {
            // Mouse/pen get no implicit capture, so request it explicitly to
            // keep move/up flowing even if the pointer drifts over the control
            // bar or off the canvas.
            let _ = c.set_pointer_capture(id);
        }
        with_app(|app| app.on_down(id, x, y));
    });

    let c = canvas.clone();
    add_listener(canvas, "pointermove", move |e: PointerEvent| {
        e.prevent_default();
        let (id, x, y) = point(&c, &e);
        with_app(|app| app.on_move(id, x, y));
    });

    // Only genuine releases lift a finger. `pointerleave` is a hover event —
    // touch pointers fire it spuriously (notably on iOS Safari), which would
    // remove fingers the instant they touch down, so it must NOT be here.
    for ev in ["pointerup", "pointercancel"] {
        add_listener(canvas, ev, move |e: PointerEvent| {
            e.prevent_default();
            let id = e.pointer_id();
            with_app(|app| app.on_up(id));
        });
    }
}

fn install_ui_handlers() -> Result<(), JsValue> {
    let doc = document();
    let buttons = doc.query_selector_all("#ui .seg button")?;
    for i in 0..buttons.length() {
        let el = buttons.item(i).unwrap().dyn_into::<HtmlElement>()?;
        let mode = el.get_attribute("data-mode").unwrap_or_default();
        let clicked = el.clone();
        add_listener(&el, "click", move |_e: Event| {
            let m = match mode.as_str() {
                "order" => Mode::Order,
                "groups" => Mode::Groups,
                _ => Mode::One,
            };
            with_app(|app| app.set_mode(m));
            set_active_button(&clicked, m);
        });
    }

    let gminus = doc
        .get_element_by_id("gminus")
        .unwrap()
        .dyn_into::<HtmlElement>()?;
    add_listener(&gminus, "click", move |_e: Event| {
        with_app(|app| app.change_groups(-1));
    });

    let gplus = doc
        .get_element_by_id("gplus")
        .unwrap()
        .dyn_into::<HtmlElement>()?;
    add_listener(&gplus, "click", move |_e: Event| {
        with_app(|app| app.change_groups(1));
    });

    let dminus = doc
        .get_element_by_id("dminus")
        .unwrap()
        .dyn_into::<HtmlElement>()?;
    add_listener(&dminus, "click", move |_e: Event| {
        with_app(|app| app.remove_virtual());
    });

    let dplus = doc
        .get_element_by_id("dplus")
        .unwrap()
        .dyn_into::<HtmlElement>()?;
    add_listener(&dplus, "click", move |_e: Event| {
        with_app(|app| app.add_virtual());
    });

    Ok(())
}

/// Toggle a `hidden` class on an element without disturbing its other classes.
fn set_hidden(el: &HtmlElement, hidden: bool) {
    let cls = el.class_name();
    let has = cls.split_whitespace().any(|c| c == "hidden");
    if hidden && !has {
        el.set_class_name(&format!("{cls} hidden"));
    } else if !hidden && has {
        let n: Vec<&str> = cls.split_whitespace().filter(|c| *c != "hidden").collect();
        el.set_class_name(&n.join(" "));
    }
}

fn set_active_button(clicked: &HtmlElement, m: Mode) {
    let doc = document();
    if let Ok(list) = doc.query_selector_all("#ui .seg button") {
        for i in 0..list.length() {
            if let Some(el) = list.item(i).and_then(|n| n.dyn_into::<HtmlElement>().ok()) {
                el.set_class_name("");
            }
        }
    }
    clicked.set_class_name("active");
    if let Some(g) = doc
        .get_element_by_id("grp")
        .and_then(|e| e.dyn_into::<HtmlElement>().ok())
    {
        g.set_class_name(if m == Mode::Groups {
            "grp"
        } else {
            "grp hidden"
        });
    }
}

fn install_window_handlers() {
    add_listener(&window(), "resize", move |_e: Event| {
        with_app(|app| app.resize());
    });
    add_listener(&window(), "contextmenu", move |e: Event| {
        e.prevent_default();
    });
}

fn start_raf() {
    // The standard wasm-bindgen self-referencing rAF loop.
    #[allow(clippy::type_complexity)]
    let f: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let g = f.clone();
    *g.borrow_mut() = Some(Closure::wrap(Box::new(move |ts: f64| {
        with_app(|app| app.frame(ts));
        request_frame(f.borrow().as_ref().unwrap());
    }) as Box<dyn FnMut(f64)>));
    request_frame(g.borrow().as_ref().unwrap());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn window() -> Window {
    web_sys::window().expect("no window")
}

fn document() -> Document {
    window().document().expect("no document")
}

fn with_app<F: FnOnce(&mut App)>(f: F) {
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            f(app);
        }
    });
}

fn request_frame(cb: &Closure<dyn FnMut(f64)>) {
    window()
        .request_animation_frame(cb.as_ref().unchecked_ref())
        .expect("rAF failed");
}

fn add_listener<T, E, F>(target: &T, ev: &str, f: F)
where
    T: AsRef<web_sys::EventTarget>,
    E: wasm_bindgen::convert::FromWasmAbi + 'static,
    F: FnMut(E) + 'static,
{
    let cb = Closure::wrap(Box::new(f) as Box<dyn FnMut(E)>);
    target
        .as_ref()
        .add_event_listener_with_callback(ev, cb.as_ref().unchecked_ref())
        .expect("addEventListener failed");
    cb.forget();
}

/// Classic Hermite ease (0→0, 1→1) with zero slope at both ends.
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Inverse of `smoothstep`: given `y` in 0..1, the `t` such that
/// `smoothstep(t) == y`. Used to hand an animation off at its current progress
/// so a follow-on (e.g. a cancel) continues from where it is instead of jumping.
fn inv_smoothstep(y: f64) -> f64 {
    let y = y.clamp(0.0, 1.0);
    0.5 - (((1.0 - 2.0 * y).asin()) / 3.0).sin()
}

fn rnd() -> f64 {
    js_sys::Math::random()
}

fn rnd_idx(n: usize) -> usize {
    ((rnd() * n as f64) as usize).min(n.saturating_sub(1))
}

fn shuffle<T>(v: &mut [T]) {
    for i in (1..v.len()).rev() {
        v.swap(i, rnd_idx(i + 1));
    }
}

fn rgba(c: Rgb, a: f64) -> String {
    format!("rgba({},{},{},{:.3})", c.0, c.1, c.2, a.clamp(0.0, 1.0))
}

fn lerp_col(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let l = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t) as u8;
    (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}
