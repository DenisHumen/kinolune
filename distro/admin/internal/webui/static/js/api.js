// SPDX-License-Identifier: MPL-2.0
// REST client of alpha-admin (internal/server/openapi.yaml). Only documented endpoints.

let csrf = '';
const listeners = new Set();

/** Error of an API call: `code` is the API error code (or "network"), `status` the HTTP status. */
export class ApiError extends Error {
  constructor(status, code, message, body = {}, retryAfter = 0) {
    super(message || code);
    this.status = status;
    this.code = code;
    this.body = body;
    this.retryAfter = retryAfter;
  }
}

/** Called with no arguments when the session is gone (401). */
export function onLoggedOut(fn) { listeners.add(fn); }

export function setCsrf(token) { csrf = token || ''; }

/** Calls the API. `body` is sent as JSON. Returns the parsed body (or null). */
export async function api(method, path, body, { quiet401 = false } = {}) {
  const headers = { Accept: 'application/json' };
  const opts = { method, headers, credentials: 'same-origin', cache: 'no-store' };
  if (method !== 'GET' && method !== 'HEAD') headers['X-CSRF-Token'] = csrf;
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  let res;
  try {
    res = await fetch(path, opts);
  } catch (e) {
    throw new ApiError(0, 'network', String(e && e.message || e));
  }
  const text = await res.text();
  let data = null;
  if (text) {
    try { data = JSON.parse(text); } catch { data = { raw: text }; }
  }
  if (!res.ok) {
    const code = (data && data.error) || `http_${res.status}`;
    const err = new ApiError(res.status, code, data && data.message, data || {}, Number(res.headers.get('Retry-After')) || 0);
    if (res.status === 401 && path !== '/api/v1/session' && !quiet401) for (const fn of listeners) fn();
    throw err;
  }
  return data;
}

export const get = (p, o) => api('GET', p, undefined, o);
export const post = (p, b) => api('POST', p, b);
export const put = (p, b) => api('PUT', p, b);
export const del = (p, b) => api('DELETE', p, b);

/** Logs in; returns the session ({user, role, csrf_token}). */
export async function login(username, password) {
  const s = await post('/api/v1/session', { username, password });
  setCsrf(s.csrf_token);
  return s;
}

/** The current session or null. */
export async function session() {
  try {
    const s = await get('/api/v1/session', { quiet401: true });
    setCsrf(s.csrf_token);
    return s;
  } catch (e) {
    if (e.status === 401) return null;
    throw e;
  }
}

export async function logout() {
  try { await del('/api/v1/session'); } catch { /* the session may be gone already */ }
  setCsrf('');
}

let features = null;
/**
 * Optional parts of the API, detected from /api/v1/openapi.yaml: users, sharing and SSH
 * come with DIST-018. Returns {users, sharing, ssh}.
 */
export async function detectFeatures() {
  if (features) return features;
  let doc = '';
  try {
    const res = await fetch('/api/v1/openapi.yaml', { cache: 'no-store' });
    if (res.ok) doc = await res.text();
  } catch { /* offline: no optional pages */ }
  const has = (p) => doc.includes(`\n  ${p}:\n`);
  features = { users: has('/api/v1/users'), sharing: has('/api/v1/sharing'), ssh: has('/api/v1/ssh') };
  return features;
}

/** Resolves when the admin service answers again (after a reboot), or after `timeoutMs`. */
export async function waitForService({ timeoutMs = 15 * 60 * 1000, onTick } = {}) {
  const start = Date.now();
  let wentDown = false;
  while (Date.now() - start < timeoutMs) {
    await new Promise((r) => setTimeout(r, 3000));
    let up = false;
    // A request to a machine that is going down can hang for minutes: give up after 5 s.
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), 5000);
    try {
      const res = await fetch('/healthz', { cache: 'no-store', signal: ctl.signal });
      up = res.ok;
    } catch { up = false; } finally { clearTimeout(timer); }
    if (!up) wentDown = true;
    if (onTick) onTick({ up, wentDown, elapsed: Date.now() - start });
    if (up && wentDown) return true;
    // A fast reboot may slip between two polls: sessions live in memory, so a lost
    // session after the first minute also means the machine has restarted.
    if (up && Date.now() - start > 60 * 1000 && (await session().catch(() => 'error')) === null) return true;
  }
  return false;
}
