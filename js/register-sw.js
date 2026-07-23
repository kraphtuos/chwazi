// Optional: registers a tiny service worker for offline use *if* sw.js
// is deployed alongside this file. The page works standalone without it.
//
// Skip it during local development (`trunk serve`), where a stale SW
// would keep serving old WASM/HTML across rebuilds. Detected by
// hostname, so it stays off whether or not `--release` is passed. Any
// SW/caches left over from a previous dev session are torn down too.
if ('serviceWorker' in navigator) {
    // Treat localhost AND private LAN addresses / *.local as dev, so a
    // stale service worker never caches old builds while testing on a
    // phone over the LAN (where the host is e.g. 192.168.x.x, not
    // localhost).
    const host = location.hostname;
    const isDev = ['localhost', '127.0.0.1', '[::1]', ''].includes(host)
        || /^(10\.|127\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host)
        || host.endsWith('.local');
    window.addEventListener('load', () => {
        if (isDev) {
            navigator.serviceWorker.getRegistrations()
                .then((rs) => rs.forEach((r) => r.unregister())).catch(() => { });
            if (window.caches) {
                caches.keys().then((ks) => ks.forEach((k) => caches.delete(k))).catch(() => { });
            }
        } else {
            navigator.serviceWorker.register('./sw.js').catch(() => { });
        }
    });
}
