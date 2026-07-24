//! Geometry helpers, the per-frame update, and the random selection itself.

use crate::config::*;
use crate::state::{App, Mode, Phase};
use crate::util::{rnd, rnd_idx, set_hidden, shuffle, smoothstep, window};

impl App {
    pub(crate) fn base_r(&self) -> f64 {
        (self.width.min(self.height) * 0.11).clamp(46.0, 70.0)
    }

    /// Fingers still pressed (ignores ones currently shrinking out). Counts both
    /// real touches and virtual dots, since both are participants in a pick.
    pub(crate) fn active_count(&self) -> usize {
        self.fingers.values().filter(|f| f.alive).count()
    }

    /// Real (non-virtual) fingers still pressed. The round-completion and reset
    /// triggers key off this, not `active_count`: virtual dots never lift on
    /// their own, so counting them would stop a round ever finishing.
    pub(crate) fn real_alive(&self) -> usize {
        self.fingers.values().filter(|f| f.alive && !f.virt).count()
    }

    /// Alive virtual dots currently staged.
    pub(crate) fn virt_count(&self) -> usize {
        self.fingers.values().filter(|f| f.alive && f.virt).count()
    }

    pub(crate) fn update_dcount(&self) {
        self.dcount_el
            .set_inner_text(&self.virt_count().to_string());
    }

    /// A spread-out spot for a new virtual dot: away from the screen edges and
    /// the top/bottom control bars, and as far as possible from existing dots so
    /// the added rings don't pile up.
    pub(crate) fn free_spot(&self) -> (f64, f64) {
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

    /// The current spotlight-hole radius of the Pick One flood, mirroring the
    /// geometry in `draw_flood`. Used to hand the flood off to a cancel overlay
    /// at exactly its current size so a touch mid-exit doesn't make it jump.
    pub(crate) fn flood_hole(&self) -> f64 {
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

    pub(crate) fn resize(&mut self) {
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

    pub(crate) fn pick_color(&self) -> Rgb {
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

    /// Drop any decided-pick state — winner, reveal progress, and the held
    /// flood colour/position — back to "nothing chosen". Shared by the mode /
    /// team-count switches, the idle reset, and the last-finger-up path.
    pub(crate) fn clear_result(&mut self) {
        self.winner = None;
        self.reveal = 0.0;
        self.flood_col = None;
        self.flood_pos = None;
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

    pub(crate) fn frame(&mut self, now_ms: f64) {
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
                // Everyone already lifted and the reveal has played out: begin
                // the post-result hold, which then plays the exit. Pick One
                // needs a decided flood to hold on to; Order / Teams always
                // carry their result on the rings themselves, so simply
                // reaching Result is enough. This is also where a lift *during*
                // an Order / Teams reveal lands, once the animation it was left
                // to finish completes (see `on_up`).
                if self.real_alive() == 0 && (self.mode != Mode::One || self.flood_col.is_some()) {
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
}
