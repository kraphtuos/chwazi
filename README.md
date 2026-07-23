# Chwazi (Rust + WASM PWA)

An installable clone of Chwazi — the "everyone puts a finger on the screen and
it picks one" app. Written in **Rust**, compiled to
**WebAssembly**, and built by **Trunk** into a static `dist/` directory served
with `index.html` as the entrypoint.

## Modes

| Mode         | What it does                                                        |
| ------------ | ------------------------------------------------------------------- |
| **Pick One** | A countdown arc fills each ring; when it completes, one winner's colour floods the screen around a spotlight on its ring. |
| **Order**    | Assigns everyone a random rank (1, 2, 3 …).                          |
| **Teams**    | Splits everyone into 2–8 randomly balanced, colour-coded teams.      |

Put ≥2 fingers on the screen, hold still for ~2.5 s, and the pick fires. Lift all
fingers to reset. Works with real multi-touch on phones/tablets and with a mouse
on desktop (single pointer). The control bar auto-hides while fingers are down.

## Tech

- **Rust + `web-sys`** driving a full-screen `<canvas>` via Pointer Events and
  `requestAnimationFrame`. No JS framework, no runtime dependencies.
- Randomness via `Math.random()` (no `getrandom`/`rand` needed).
- **PWA**: web-app manifest, icon, and a `sw.js` service worker for offline use.
  Installable to home screen / desktop.

## Prerequisites

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk           # or: cargo binstall trunk
```

## Build

```sh
trunk build --release
```

Everything lands in `dist/` — the compiled `.wasm`, the wasm-bindgen JS glue,
the manifest, the icon, `sw.js`, and `index.html` (the entrypoint). Deploy the
whole directory as-is.

## Develop

```sh
trunk serve --release
# open http://localhost:8080
```

## Deploy

Upload the `dist/` directory to any static host — GitHub Pages, Netlify, S3,
Cloudflare Pages, etc. — with `index.html` as the entrypoint. Serve over HTTPS
for PWA installability.

## Project layout

```
Cargo.toml        crate config (cdylib) + web-sys features
index.html        Trunk template: canvas + control bar markup, asset links
styles.css        all UI styling (inlined into index.html at build)
src/
  lib.rs          entry point (start): grabs DOM handles, wires events, starts rAF
  config.rs       tunable constants + colour palette
  state.rs        the App / Finger state types shared across modules
  input.rs        pointer + control-bar events and the state transitions they drive
  sim.rs          geometry helpers, the per-frame update, and the random pick
  render.rs       all canvas drawing (flood, rings, overlays)
  util.rs         DOM/rAF glue, easing, RNG, colour helpers
js/               page bootstrap scripts (inlined into index.html at build)
  viewport-fit.js   sizes the canvas to the visual viewport
  no-double-tap.js  suppresses iOS double-tap zoom / loupe
  register-sw.js    registers sw.js in production, tears it down in dev
assets/
  manifest.json   PWA manifest
  icon.svg        app icon
  sw.js           offline service worker  (copied to dist root for SW scope)
```
