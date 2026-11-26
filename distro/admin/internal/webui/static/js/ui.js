// SPDX-License-Identifier: MPL-2.0
// Small DOM helpers. Text from the API always goes through textContent, never innerHTML.

import { ApiError } from './api.js';
import { t } from './i18n.js';

/** Error text for the user: by API code, else the server's message. */
export function errText(e) {
  if (e instanceof ApiError) {
    const k = `err.${e.code}`;
    const s = t(k);
    if (s !== k) return e.code === 'locked' && e.retryAfter ? `${s} ${t('err.retry_in', { s: e.retryAfter })}` : s;
    return e.message || e.code;
  }
  return String((e && e.message) || e);
}

/** Shows an error toast; on 401 the shell goes back to the login page by itself. */
export function fail(e) {
  if (e instanceof ApiError && e.status === 401) return;
  toast(errText(e), 'bad', 8000);
}

let timer = null;
export function stopTimer() {
  if (timer) clearInterval(timer);
  timer = null;
}

/** Refreshes the current page every `ms` while it is shown (one timer for the whole app). */
export function every(ms, fn) {
  stopTimer();
  timer = setInterval(() => { if (!document.hidden) fn(); }, ms);
}

/**
 * h('div.panel#id', {onclick, attrs…}, children…) — creates an element.
 * Children: strings (text), nodes, arrays, null/false (skipped).
 */
export function h(tag, props, ...children) {
  const m = /^([a-z0-9]+)((?:[.#][\w-]+)*)$/i.exec(tag);
  const el = document.createElement(m ? m[1] : tag);
  if (m && m[2]) {
    for (const part of m[2].match(/[.#][\w-]+/g)) {
      if (part[0] === '.') el.classList.add(part.slice(1));
      else el.id = part.slice(1);
    }
  }
  if (props && (typeof props !== 'object' || props instanceof Node || Array.isArray(props))) {
    children.unshift(props);
    props = null;
  }
  for (const [k, v] of Object.entries(props || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (k === 'class') el.className = v;
    else if (k === 'value') el.value = v;
    else if (k === 'checked' || k === 'disabled' || k === 'selected') el[k] = Boolean(v);
    else el.setAttribute(k, v === true ? '' : String(v));
  }
  append(el, children);
  return el;
}

function append(el, kids) {
  for (const c of kids) {
    if (c === null || c === undefined || c === false) continue;
    if (Array.isArray(c)) append(el, c);
    else el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

/** An inline SVG icon from our own constant set (never from data). */
export function icon(name) {
  const span = document.createElement('span');
  span.className = 'icon';
  span.setAttribute('aria-hidden', 'true');
  span.innerHTML = ICONS[name] || '';
  const el = span.firstElementChild;
  if (!el) return span;
  el.classList.add('icon');
  return el;
}

const svg = (d) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`;
const ICONS = {
  // The Kinolune mark (design/brand/src/mark.svg), in the accent colour.
  mark: '<svg viewBox="0 0 1024 1024"><g transform="translate(-28 -28) scale(1.055)"><path fill="currentColor" d="M531.74 192.15A320 320 0 1 0 817.76 634.17A264 264 0 1 1 531.74 192.15Z"/><path fill="currentColor" stroke="currentColor" stroke-width="52" stroke-linejoin="round" d="M598 322V538L782 430Z"/></g></svg>',
  home: svg('<path d="M3 11l9-7 9 7"/><path d="M5 10v10h14V10"/>'),
  update: svg('<path d="M20 12a8 8 0 1 1-2.3-5.6"/><path d="M20 4v5h-5"/>'),
  network: svg('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>'),
  users: svg('<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><path d="M16 4.5a3.5 3.5 0 0 1 0 7M18 14a6.5 6.5 0 0 1 3.5 6"/>'),
  share: svg('<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>'),
  disk: svg('<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 15h.01M11 15h6"/>'),
  system: svg('<rect x="4" y="4" width="16" height="16" rx="2"/><path d="M9 9h6v6H9zM9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2"/>'),
  advanced: svg('<path d="M4 17l6-6-6-6"/><path d="M12 19h8"/>'),
  logout: svg('<path d="M15 4h4v16h-4"/><path d="M10 8l-4 4 4 4M6 12h10"/>'),
  power: svg('<path d="M12 3v9"/><path d="M6.3 6.3a8 8 0 1 0 11.4 0"/>'),
};

/** Shows a short message in the corner. kind: ok | bad | info. */
export function toast(text, kind = 'info', ms = 5000) {
  const box = document.getElementById('toasts');
  const el = h(`div.toast.${kind}`, { role: kind === 'bad' ? 'alert' : 'status' }, text);
  box.append(el);
  setTimeout(() => el.remove(), ms);
}

/**
 * A modal dialog. `body` is a node; `buttons` = [{label, kind, value, onClick}] — onClick may
 * return a promise; a falsy result keeps the dialog open (validation), anything else closes it.
 * Resolves with the value of the button (or null on Escape / cancel).
 */
export function dialog({ title, body, buttons, testid }) {
  return new Promise((resolve) => {
    const prev = document.activeElement;
    const close = (v) => {
      modal.remove();
      document.removeEventListener('keydown', onKey);
      if (prev && prev.focus) prev.focus();
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === 'Escape') close(null);
      if (e.key === 'Enter' && e.target instanceof HTMLInputElement) { e.preventDefault(); btns[btns.length - 1].click(); }
    };
    const btns = buttons.map((b) => {
      const el = h(`button${b.kind ? '.' + b.kind : ''}`, {
        type: 'button',
        'data-testid': b.testid,
        onclick: async () => {
          if (!b.onClick) return close(b.value ?? null);
          el.disabled = true;
          try {
            const r = await b.onClick();
            if (r) close(r === true ? (b.value ?? true) : r);
          } finally { el.disabled = false; }
        },
      }, b.label);
      return el;
    });
    const box = h('div.dialog', { role: 'dialog', 'aria-modal': 'true', 'aria-label': title, 'data-testid': testid }, h('h2', title), body, h('div.buttons', btns));
    const modal = h('div.modal', { onclick: (e) => { if (e.target === modal) close(null); } }, box);
    document.body.append(modal);
    document.addEventListener('keydown', onKey);
    const first = box.querySelector('input, select') || btns[btns.length - 1];
    if (first) first.focus();
  });
}

/** Confirmation dialog; resolves true/false. */
export async function confirmDialog({ title, text, ok, danger = false, extra, testid }) {
  const r = await dialog({
    title,
    testid,
    body: h('div.stack', text ? h('p', text) : null, extra || null),
    buttons: [
      { label: t('common.cancel'), kind: 'ghost', value: false },
      { label: ok, kind: danger ? 'danger' : 'primary', value: true, testid: 'confirm' },
    ],
  });
  return r === true;
}

/** Asks for the administrator's own password (DIST-018 re-authentication). */
export async function askAdminPassword(intro) {
  const input = h('input', { type: 'password', autocomplete: 'current-password', 'data-testid': 'admin-password' });
  const r = await dialog({
    title: t('users.confirm_title'),
    body: h('div.stack', intro ? h('p', intro) : null, h('label.field', t('users.your_password'), input)),
    buttons: [
      { label: t('common.cancel'), kind: 'ghost', value: null },
      { label: t('common.continue'), kind: 'primary', testid: 'confirm', onClick: () => { if (input.value) return true; input.focus(); return false; } },
    ],
  });
  return r ? input.value : null;
}

/** Switch (checkbox) with a label. */
export function switchEl({ label, checked, onChange, testid, disabled }) {
  const input = h('input', { type: 'checkbox', role: 'switch', checked, disabled, 'data-testid': testid });
  input.addEventListener('change', async () => {
    input.disabled = true;
    try { await onChange(input.checked); } catch { input.checked = !input.checked; } finally { input.disabled = Boolean(disabled); }
  });
  return h('label.switch', input, h('span.track'), label ? h('span', label) : null);
}

export function chip(text, kind = '') { return h(`span.chip${kind ? '.' + kind : ''}`, text); }

export function meter(fraction, warnAt = 0.8, badAt = 0.92) {
  const f = Math.max(0, Math.min(1, fraction || 0));
  const bar = h('i');
  bar.style.width = `${(f * 100).toFixed(1)}%`;
  return h(`div.meter${f >= badAt ? '.bad' : f >= warnAt ? '.warn' : ''}`, { role: 'meter', 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': Math.round(f * 100) }, bar);
}

export function kv(pairs) {
  return h('dl.kv', pairs.filter(Boolean).map(([k, v]) => [h('dt', k), h('dd', v ?? '—')]));
}
