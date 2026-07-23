//! Core state types shared across the app modules.
//!
//! `App` and `Finger` fields are `pub(crate)` because the logic that reads and
//! mutates them is split across the sibling modules (`input`, `sim`, `render`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::TAU;

use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlElement};

use crate::config::Rgb;
use crate::util::rnd;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    One,
    Order,
    Groups,
}

#[derive(Clone, Copy)]
pub(crate) enum Phase {
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

pub(crate) struct Finger {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) color: Rgb,
    /// Animated ring radius (px), lerped toward a phase-driven target.
    pub(crate) ring: f64,
    /// Opacity 0..1, lerped down as losing rings fall away.
    pub(crate) alpha: f64,
    /// 1-based rank (order mode).
    pub(crate) rank: usize,
    /// 0-based team index (groups mode).
    pub(crate) group: usize,
    /// Time this finger touched down: drives the ring build-in and desyncs bob.
    pub(crate) birth: f64,
    /// Whether the finger is still pressed. When it lifts this flips to false
    /// and the whole object shrinks out (`scale` → 0) before being removed.
    pub(crate) alive: bool,
    /// Time the object began shrinking out. `None` while it should stay on
    /// screen at full size (e.g. a ranked ring frozen during the result hold).
    /// Drives the shrink off absolute time so it stays smooth (`SHRINK_TIME`).
    pub(crate) depart: Option<f64>,
    /// The `scale` at the instant the finger lifted, so the shrink-out eases
    /// from the size it actually had (usually 1, but less if lifted mid-grow).
    pub(crate) depart_scale: f64,
    /// 0..1 overall object scale — grows in on add (snappy exponential ease),
    /// shrinks out analytically from `depart`/`depart_scale` on remove.
    pub(crate) scale: f64,
    /// A "virtual" dot added with the +/- control rather than a real touch, to
    /// work around iOS capping simultaneous touches at 5. It behaves exactly
    /// like a pressed finger for rendering and selection, but never lifts on its
    /// own and does not count toward the round-completion / reset triggers.
    pub(crate) virt: bool,
    /// Angle (rad) the ring build-in grows out from, expanding both ways to a
    /// full circle. Randomised per finger/dot so they don't all unfurl from the
    /// same point.
    pub(crate) birth_angle: f64,
}

impl Finger {
    /// A freshly-added finger or virtual dot: full opacity, resting ring radius,
    /// zero `scale` so it grows in, a random unfurl angle, and not yet departing.
    /// `virt` distinguishes a +/- dot from a real touch (see the field docs).
    pub(crate) fn spawn(x: f64, y: f64, color: Rgb, base: f64, now: f64, virt: bool) -> Self {
        Finger {
            x,
            y,
            color,
            ring: base,
            alpha: 1.0,
            rank: 0,
            group: 0,
            birth: now,
            alive: true,
            depart: None,
            depart_scale: 1.0,
            scale: 0.0,
            virt,
            birth_angle: rnd() * TAU,
        }
    }
}

pub(crate) struct App {
    pub(crate) canvas: HtmlCanvasElement,
    pub(crate) ctx: CanvasRenderingContext2d,
    pub(crate) ui: HtmlElement,
    pub(crate) gcount_el: HtmlElement,
    pub(crate) dots_el: HtmlElement,
    pub(crate) dcount_el: HtmlElement,

    pub(crate) fingers: HashMap<i32, Finger>,
    pub(crate) phase: Phase,
    pub(crate) mode: Mode,
    pub(crate) groups: usize,
    /// Next id handed to a virtual dot. Counts *down* from -1 so virtual dots
    /// never collide with real pointer ids (which the spec keeps non-negative),
    /// and a more-negative id means a more-recently-added dot.
    pub(crate) next_virt: i32,

    pub(crate) now: f64,    // seconds, updated each frame
    pub(crate) last_t: f64, // seconds

    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) dpr: f64,

    // selection / animation state
    pub(crate) winner: Option<i32>,
    pub(crate) reveal: f64,
    /// Continuously accumulated angle offsetting the countdown arc so it spins.
    pub(crate) spin: f64,
    /// Winner's colour + position, kept so the spotlight-flood can hold on
    /// screen after the winning finger has lifted.
    pub(crate) flood_col: Option<Rgb>,
    pub(crate) flood_pos: Option<(f64, f64)>,
    /// A result's flood being whooshed away because a touch cancelled it, drawn
    /// as an overlay over the fresh round. (start time, colour, centre).
    pub(crate) cancel: Option<(f64, Rgb, (f64, f64))>,
}

thread_local! {
    pub(crate) static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}
