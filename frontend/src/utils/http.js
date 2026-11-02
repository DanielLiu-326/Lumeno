// The single HTTP entry point for the app.
//
// Every request carries the language header and the session cookie, and an
// expired session is handled in one place instead of per component. Callers
// that want the `{ success, message, code, data }` envelope use `httpJson`;
// callers that stream a blob or read a partial response use `httpFetch`.
import { locale } from "../i18n/index.js";

let unauthorizedHandler = null;

/**
 * Register what happens when the session is gone (HTTP 401). The app composable
 * installs this once; without it a 401 just reaches the caller as usual.
 */
export function setUnauthorizedHandler(handler) {
  unauthorizedHandler = typeof handler === "function" ? handler : null;
}

export function authHeaders(extra = {}) {
  return { "Accept-Language": locale.value, ...extra };
}

/** Fetch with session cookie + language header; a 401 notifies the app once. */
export async function httpFetch(url, options = {}) {
  const response = await fetch(url, {
    credentials: "include",
    ...options,
    headers: authHeaders(options.headers),
  });
  if (response.status === 401) {
    unauthorizedHandler?.();
  }
  return response;
}

/**
 * Request a JSON envelope. Throws an `Error` carrying `status` when the server
 * rejects the request, so callers only handle the happy path.
 */
export async function httpJson(url, options = {}, fallbackMessage = "") {
  const response = await httpFetch(url, options);
  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    const payload = await response.json();
    if (!payload.success) {
      const error = new Error(payload.message || fallbackMessage || `HTTP ${response.status}`);
      error.status = response.status;
      error.code = payload.code;
      throw error;
    }
    return payload;
  }
  if (!response.ok) {
    const error = new Error(fallbackMessage || `HTTP ${response.status}`);
    error.status = response.status;
    throw error;
  }
  return response;
}

/** POST a JSON body and return the parsed envelope. */
export function postJson(url, body, options = {}) {
  return httpJson(url, {
    method: "POST",
    ...options,
    headers: { "Content-Type": "application/json", ...(options.headers || {}) },
    body: JSON.stringify(body),
  });
}
