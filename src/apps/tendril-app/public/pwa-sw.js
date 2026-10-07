/* Forge's service worker: installability, a fast shell, an offline page, and push notifications.
 *
 * Deliberately conservative about what it caches. The app sits behind a login-gated proxy, so an expired
 * session turns every URL into a login page; caching one of those as "the app" would break the app until
 * the cache is cleared. Only same-origin GETs that came back OK and are not HTML are ever stored, and
 * nothing the daemon answers (/api, /ivy, websockets, the web viewer's own worker) is touched.
 */
const VERSION = "forge-pwa-v1";
const ASSETS = `${VERSION}-assets`;
const STATIC = `${VERSION}-static`;
const OFFLINE_URL = "/offline.html";

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(STATIC)
      .then((cache) => cache.addAll([OFFLINE_URL, "/icons/icon-192.png"]))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => !k.startsWith(VERSION)).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") self.skipWaiting();
});

const isDaemon = (url) => url.pathname.startsWith("/api") || url.pathname.startsWith("/ivy/") || url.pathname.startsWith("/__") || url.pathname === "/sw.js";

const cacheable = (response) =>
  response && response.ok && response.type === "basic" && !(response.headers.get("content-type") || "").includes("text/html");

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin || isDaemon(url) || url.pathname === "/pwa-sw.js") return;

  // A page: always the network (so a deploy shows up at once), the offline page when there is none.
  if (request.mode === "navigate") {
    event.respondWith(fetch(request).catch(() => caches.match(OFFLINE_URL)));
    return;
  }

  // Hashed build output never changes under its name: cache first.
  if (url.pathname.startsWith("/assets/")) {
    event.respondWith(
      caches.open(ASSETS).then(async (cache) => {
        const hit = await cache.match(request);
        if (hit) return hit;
        const response = await fetch(request);
        if (cacheable(response)) cache.put(request, response.clone());
        return response;
      }),
    );
    return;
  }

  // Icons, the manifest, fonts: show what we have and refresh it behind.
  event.respondWith(
    caches.open(STATIC).then(async (cache) => {
      const hit = await cache.match(request);
      const refresh = fetch(request)
        .then((response) => {
          if (cacheable(response)) cache.put(request, response.clone());
          return response;
        })
        .catch(() => hit);
      return hit || refresh;
    }),
  );
});

/* ---- push ------------------------------------------------------------------------------------- */

self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = { title: "Forge", body: event.data ? event.data.text() : "" };
  }
  const title = data.title || "Forge";
  event.waitUntil(
    self.registration.showNotification(title, {
      body: data.body || "",
      icon: "/icons/icon-192.png",
      badge: "/icons/favicon-32.png",
      tag: data.tag || undefined,
      renotify: Boolean(data.tag),
      requireInteraction: data.urgent === true,
      data: { url: data.url || "/" },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL((event.notification.data && event.notification.data.url) || "/", self.location.origin).href;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      // Reuse a window that is already open: take it to the right page and bring it forward.
      for (const w of windows) {
        if (new URL(w.url).origin === self.location.origin && "focus" in w) {
          return w.focus().then((focused) => ("navigate" in focused ? focused.navigate(target) : focused));
        }
      }
      return self.clients.openWindow(target);
    }),
  );
});
