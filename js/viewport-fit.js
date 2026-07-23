// Size the canvas from JS rather than CSS viewport units.
//
// The canvas is sized and positioned to the VISUAL VIEWPORT — the actual
// visible area — rather than CSS viewport units or screen.height. On this
// device screen.height is 874 but only 812 is visible in standalone, and
// 100dvh / safe-area insets resolve against a viewport that is still
// mid-settle on a reload (leaving an uncovered strip, with no event when
// it finally settles). Tracking the visual viewport sidesteps all of
// that; a per-load running max plus a persisted best guard the reload
// under-report, and we re-fit on every signal plus a short settle schedule.
(function () {
    const c = document.getElementById('c');
    const isStandalone = () =>
        window.navigator.standalone === true ||
        (window.matchMedia && window.matchMedia('(display-mode: standalone)').matches);

    // In-memory running max: within a single page load the canvas can
    // only ever grow, no matter what a mid-settle measurement reports.
    // (localStorage carries this across reloads; this variable guards
    // against a shrink even if storage is unavailable.)
    let runW = 0, runH = 0;

    // Orientation key: portrait vs landscape are cached separately.
    // v2 key: v1 persisted screen.height (874), which overshot the real
    // visible area (the visual viewport, 812) and cut off the bottom.
    const orient = () => (window.innerHeight >= window.innerWidth ? 'p' : 'l');
    const KEY = () => 'cnv-fit2-' + orient();

    // Remember the largest coverage ever achieved for this orientation.
    // This is the crux of the refresh bug: a reload measures a viewport
    // that is still settling and reports dimensions that are too small,
    // and WebKit fires no event when it finally settles. By persisting
    // the known-good size and seeding from it on load, a refresh starts
    // already full-screen instead of climbing up from too-small — and
    // the live measurements below can still grow it further, never shrink.
    function loadBest() {
        try {
            const v = JSON.parse(localStorage.getItem(KEY()));
            if (v && v.w > 0 && v.h > 0) return v;
        } catch (e) { }
        return null;
    }
    function saveBest(w, h) {
        try { localStorage.setItem(KEY(), JSON.stringify({ w, h })); } catch (e) { }
    }

    function fitCanvas() {
        let w, h, top = 0, left = 0;
        const vv = window.visualViewport;
        if (isStandalone()) {
            // Installed PWA: the *visual viewport* is the authoritative
            // visible area — NOT screen.height, which on this device is
            // 874 while only 812 is actually visible (the extra 62px ran
            // the canvas off the bottom). Position at its offset and size
            // to it, so the canvas exactly overlays what the user sees.
            w = vv ? vv.width : window.innerWidth;
            h = vv ? vv.height : window.innerHeight;
            top = vv ? vv.offsetTop : 0;
            left = vv ? vv.offsetLeft : 0;
            // Defend against iOS reporting a too-small viewport mid-settle
            // on reload: never shrink within a load, and remember the best
            // across reloads. Capped by the visible area, so it can't
            // overshoot the way the old screen floor did.
            const best = loadBest();
            if (best) { w = Math.max(w, best.w); h = Math.max(h, best.h); }
            runW = w = Math.max(w, runW);
            runH = h = Math.max(h, runH);
            saveBest(w, h);
        } else {
            // Browser tab (e.g. `trunk serve` in Safari): match the
            // visible viewport exactly, chrome and all.
            w = vv ? vv.width : window.innerWidth;
            h = vv ? vv.height : window.innerHeight;
            top = vv ? vv.offsetTop : 0;
            left = vv ? vv.offsetLeft : 0;
        }

        c.style.width = Math.ceil(w) + 'px';
        c.style.height = Math.ceil(h) + 'px';
        c.style.top = Math.round(top) + 'px';
        c.style.left = Math.round(left) + 'px';
        // A reload can restore a stray scroll offset, which shifts the
        // fixed canvas off the top edge on iOS.
        if (window.scrollX || window.scrollY) window.scrollTo(0, 0);
    }

    fitCanvas();
    ['resize', 'orientationchange', 'pageshow', 'focus'].forEach((ev) =>
        window.addEventListener(ev, fitCanvas));
    document.addEventListener('visibilitychange', fitCanvas);
    if (window.visualViewport) {
        window.visualViewport.addEventListener('resize', fitCanvas);
        window.visualViewport.addEventListener('scroll', fitCanvas);
    }
    // Catch the silent post-reload settle, which lands somewhere in the
    // first second with no event of its own.
    [0, 50, 150, 300, 600, 1000].forEach((t) => setTimeout(fitCanvas, t));
    window.addEventListener('load', fitCanvas);
})();
