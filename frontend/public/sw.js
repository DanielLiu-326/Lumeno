const SHELL_CACHE = "lumeno-shell-v8";

const SHELL_URLS = ["/", "/index.html", "/manifest.webmanifest"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(SHELL_CACHE)
      .then((cache) => cache.addAll(SHELL_URLS))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((key) => key !== SHELL_CACHE).map((key) => caches.delete(key))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  if (event.request.method !== "GET") {
    return;
  }

  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin) {
    return;
  }

  // Photos, APIs, WebDAV, and the worker itself stay network-only.
  if (shouldBypass(url.pathname)) {
    return;
  }

  if (event.request.mode === "navigate") {
    event.respondWith(networkFirst(event.request));
    return;
  }

  if (url.pathname.startsWith("/assets/")) {
    event.respondWith(cacheFirst(event.request));
    return;
  }

  if (url.pathname.startsWith("/icons/") || url.pathname.endsWith(".webmanifest")) {
    event.respondWith(staleWhileRevalidate(event.request));
  }
});

function shouldBypass(pathname) {
  return (
    pathname.startsWith("/api/") ||
    pathname.startsWith("/dav/") ||
    pathname.startsWith("/egui") ||
    pathname === "/sw.js"
  );
}

async function networkFirst(request) {
  const cache = await caches.open(SHELL_CACHE);
  try {
    const fresh = await fetch(request);
    if (fresh.ok) {
      cache.put(request, fresh.clone());
      return fresh;
    }
  } catch {
    /* use cache / shell */
  }
  const cached = await cache.match(request);
  if (cached) {
    return cached;
  }
  const shell = (await cache.match("/")) || (await cache.match("/index.html"));
  if (shell) {
    return shell;
  }
  throw new Error("offline");
}

async function cacheFirst(request) {
  const cache = await caches.open(SHELL_CACHE);
  const cached = await cache.match(request);
  if (cached) {
    return cached;
  }
  const fresh = await fetch(request);
  if (fresh.ok) {
    cache.put(request, fresh.clone());
  }
  return fresh;
}

async function staleWhileRevalidate(request) {
  const cache = await caches.open(SHELL_CACHE);
  const cached = await cache.match(request);
  const network = fetch(request)
    .then((fresh) => {
      if (fresh.ok) {
        cache.put(request, fresh.clone());
      }
      return fresh;
    })
    .catch(() => cached);
  return cached || network;
}
