//! Touch/mouse and control-bar input: the `App` state transitions each event
//! drives, plus the DOM event wiring that feeds them.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Event, HtmlCanvasElement, HtmlElement, PointerEvent};

use crate::config::*;
use crate::state::{App, Drag, Finger, Mode, Phase};
use crate::util::{add_listener, document, inv_smoothstep, window, with_app};

impl App {
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
        // A touch landing on a staged virtual dot grabs it to drag rather than
        // starting a finger of its own, so dots can be arranged after they are
        // added. Checked after the cancel above, so touching a dot while a
        // result is up still clears the result first (and `reset_idle` leaves
        // dot positions alone, so the hit test above still applies).
        if let Some(vid) = self.dot_at(x, y) {
            self.drags.insert(
                id,
                Drag {
                    dot: vid,
                    anchor: (x, y),
                },
            );
            // Restart the countdown, as adding or removing a dot does: moving a
            // participant is still arranging, and the round shouldn't fire from
            // under the hand doing it. `on_move` keeps restarting it for as
            // long as the dot is really being moved.
            self.phase = Phase::Gather { changed: self.now };
            return;
        }
        let color = self.pick_color();
        let base = self.base_r();
        self.fingers
            .insert(id, Finger::spawn(x, y, color, base, self.now, false));
        self.phase = Phase::Gather { changed: self.now };
    }

    /// Clear all selection state back to a fresh idle screen. Fingers still
    /// pressed and staged virtual dots are kept — both are participants of the
    /// next round (so a touch that cancels a result doesn't silently drop the
    /// fingers still on the glass) — and reset to a clean, fully-grown resting
    /// state. Everything lifted or mid-removal goes.
    pub(crate) fn reset_idle(&mut self) {
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
        self.regather();
        self.clear_result();
        self.update_dcount();
    }

    /// Settle into the pre-pick phase that matches who is on screen: a fresh
    /// `Gather` countdown if any finger or dot is still pressed, else `Idle`.
    ///
    /// Keys off `alive`, not whether the map is empty: a just-lifted finger
    /// stays in the map while it shrinks out, and landing in `Gather` because
    /// of it would strand the screen there once it is removed — and `Idle` is
    /// the only phase that draws the "place a finger" hint.
    pub(crate) fn regather(&mut self) {
        self.phase = if self.fingers.values().any(|f| f.alive) {
            Phase::Gather { changed: self.now }
        } else {
            Phase::Idle
        };
    }

    /// The page is going into the background. Drop every real touch and return
    /// to a clean screen.
    ///
    /// This is the one transition where a pressed finger never gets its
    /// `pointerup`: iOS claims the home / app-switcher swipe part-way through
    /// the gesture and freezes the PWA, so the touch that began the swipe is
    /// still `alive` on return and sits there as a dot nothing can clear (a
    /// lone finger can't reach a pick, and only a real release on that exact
    /// spot would lift it). Releasing implicit pointer capture for multi-touch
    /// (see `install_pointer_handlers`) makes the missing up-event likelier
    /// still, since the pointer isn't bound to the canvas.
    ///
    /// By the time we're hidden the fingers are physically off the glass, so
    /// clearing them is the truth, not a guess. Anything mid-shrink goes too —
    /// on return `now` has jumped far past its `depart`, so it would be culled
    /// on the first frame anyway. Staged virtual dots are kept and reset in
    /// place by `reset_idle`: they aren't touches, and they already persist
    /// across rounds.
    pub(crate) fn release_all_touches(&mut self) {
        self.fingers.retain(|_, f| f.virt);
        // Any in-flight dot drag ends here too: its pointer is gone, so the
        // entry would otherwise linger and re-attach to a recycled id. The dot
        // itself stays staged where it was dropped.
        self.drags.clear();
        // Drop the cancel overlay as well — with no frames running it would
        // otherwise still be mid-whoosh and flash on the way back in.
        self.cancel = None;
        self.reset_idle();
    }

    fn on_move(&mut self, id: i32, x: f64, y: f64) {
        // Positions lock in once a pick is running or shown — the winner (and
        // everyone else) stops tracking the finger.
        if !matches!(self.phase, Phase::Idle | Phase::Gather { .. }) {
            return;
        }
        // A grabbed dot follows the pointer, kept clear of the control bars.
        // Removing the dot with `-` mid-drag drops the entry (see
        // `remove_virtual`), so this stops matching and the pointer goes inert
        // for the rest of the gesture rather than towing a vanishing dot.
        if let Some(drag) = self.drags.get(&id) {
            let (vid, (ax, ay)) = (drag.dot, drag.anchor);
            let (cx, cy) = self.clamp_dot(x, y);
            if let Some(f) = self.fingers.get_mut(&vid) {
                f.x = cx;
                f.y = cy;
            }
            // A real move (beyond jitter) is still arranging, so it restarts
            // the countdown; a finger merely resting on the dot lets it run,
            // just like a held finger.
            if (x - ax).hypot(y - ay) > DRAG_SLOP * self.base_r() {
                if let Some(drag) = self.drags.get_mut(&id) {
                    drag.anchor = (x, y);
                }
                self.phase = Phase::Gather { changed: self.now };
            }
            return;
        }
        if let Some(f) = self.fingers.get_mut(&id) {
            f.x = x;
            f.y = y;
        }
    }

    fn on_up(&mut self, id: i32) {
        // Releasing a dragged dot only ends the drag. The dot stays staged
        // exactly where it was dropped, and since the grab never created a
        // finger there is no round state to unwind.
        if self.drags.remove(&id).is_some() {
            return;
        }

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
            // Order / Teams caught mid-reveal: same deal. The team colour is a
            // lerp driven by `reveal` (see `draw_finger`), and `reveal` only
            // advances while in Animate — so cutting straight to Hold here would
            // freeze the blend part-way and the rings would never reach their
            // final team colour. Let the reveal play out; `frame` picks the hold
            // up from Result.
            let revealing = showing && matches!(self.phase, Phase::Animate { .. });
            if decided || revealing {
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
                self.regather();
                self.clear_result();
            }
        } else if matches!(self.phase, Phase::Idle | Phase::Gather { .. }) {
            // Removing a finger resets the countdown (unless a pick is locked in).
            self.phase = Phase::Gather { changed: self.now };
        }
    }

    fn set_mode(&mut self, m: Mode) {
        self.mode = m;
        self.clear_result();
        for f in self.fingers.values_mut() {
            f.rank = 0;
            f.group = 0;
        }
        self.regather();
    }

    fn change_groups(&mut self, d: i32) {
        self.groups = (self.groups as i32 + d).clamp(2, 8) as usize;
        self.gcount_el.set_inner_text(&self.groups.to_string());
        self.clear_result();
        self.regather();
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
        self.fingers
            .insert(id, Finger::spawn(x, y, color, base, self.now, true));
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
        // If this dot was being dragged (a second touch can reach `-` while the
        // first holds it), end that drag — otherwise it would keep towing the
        // shrinking dot around by its corpse.
        self.drags.retain(|_, d| d.dot != id);
        self.regather();
        self.update_dcount();
    }
}

// ---------------------------------------------------------------------------
// Event wiring
// ---------------------------------------------------------------------------

/// `preventDefault` an event only when it is actually over the canvas. The move
/// and release listeners live on the window (see `install_pointer_handlers`), so
/// they also see events over the control bars — and cancelling the default there
/// would risk the buttons' own tap handling. There is only one canvas in the
/// page, so a successful cast is an exact test.
fn prevent_on_canvas(e: &PointerEvent) {
    if e.target()
        .and_then(|t| t.dyn_into::<HtmlCanvasElement>().ok())
        .is_some()
    {
        e.prevent_default();
    }
}

pub(crate) fn install_pointer_handlers(canvas: &HtmlCanvasElement) {
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

    // Move and release listen on the WINDOW, not the canvas. The control bars
    // sit above the canvas with `pointer-events: auto` and are *siblings* of it,
    // so a pointer drifting onto one retargets there and its events bubble
    // straight past the canvas to the window — a canvas listener never sees
    // them. A finger released over the dots stepper was therefore never lifted
    // and stayed on screen as a stuck dot (and stopped tracking the moment it
    // touched the pill). Mouse/pen escape this through the explicit capture
    // above; touch cannot, since capture has to be released for multi-touch.
    //
    // `pointerdown` deliberately stays on the canvas: a touch must only spawn a
    // finger when it *starts* on the canvas, never when it starts on a button.
    // A release whose pointer never spawned one is a harmless no-op — `on_up`
    // ignores ids it doesn't know.
    let c = canvas.clone();
    add_listener(&window(), "pointermove", move |e: PointerEvent| {
        prevent_on_canvas(&e);
        let (id, x, y) = point(&c, &e);
        with_app(|app| app.on_move(id, x, y));
    });

    // Only genuine releases lift a finger. `pointerleave` is a hover event —
    // touch pointers fire it spuriously (notably on iOS Safari), which would
    // remove fingers the instant they touch down, so it must NOT be here.
    for ev in ["pointerup", "pointercancel"] {
        add_listener(&window(), ev, move |e: PointerEvent| {
            prevent_on_canvas(&e);
            let id = e.pointer_id();
            with_app(|app| app.on_up(id));
        });
    }
}

pub(crate) fn install_ui_handlers() -> Result<(), JsValue> {
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

pub(crate) fn install_window_handlers() {
    add_listener(&window(), "resize", move |_e: Event| {
        with_app(|app| app.resize());
    });
    add_listener(&window(), "contextmenu", move |e: Event| {
        e.prevent_default();
    });

    // Backgrounding the app strands any pressed finger (see
    // `release_all_touches`). Both events are wired, not one: `visibilitychange`
    // is the precise signal — it means "no longer foreground" and nothing else —
    // while `pagehide` is the backstop for the iOS cases where it has been known
    // not to fire. Whichever lands first does the work; the second is then a
    // no-op on already-cleared state.
    //
    // Deliberately NOT `blur`/`focusout`: those also fire for a Control Centre
    // pull-down or a notification banner, where the fingers really are still
    // down, so clearing on them would drop live touches mid-round.
    add_listener(&document(), "visibilitychange", move |_e: Event| {
        if document().hidden() {
            with_app(|app| app.release_all_touches());
        }
    });
    add_listener(&window(), "pagehide", move |_e: Event| {
        with_app(|app| app.release_all_touches());
    });
}
