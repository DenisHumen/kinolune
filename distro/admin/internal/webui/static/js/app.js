// SPDX-License-Identifier: MPL-2.0
// Kinolune OS admin (DIST-014): shell, login and the hash router. Pages are in pages.js.

import { ApiError, detectFeatures, get, login, logout, onLoggedOut, session } from './api.js';
import { LANGS, currentLang, initI18n, setLang, t } from './i18n.js';
import { errText, h, icon, stopTimer } from './ui.js';
import { PAGES } from './pages.js';

const app = document.getElementById('app');
const state = { session: null, features: null, system: null, page: null };

// --- login -------------------------------------------------------------------------------

function renderLogin(message) {
  stopTimer();
  app.className = 'app bare';
  const user = h('input', { name: 'username', autocomplete: 'username', required: true, autocapitalize: 'none', spellcheck: 'false', 'data-testid': 'login-user' });
  const pass = h('input', { name: 'password', type: 'password', autocomplete: 'current-password', required: true, 'data-testid': 'login-password' });
  const err = h('div.alert.bad', { hidden: !message, role: 'alert', 'data-testid': 'login-error' }, message || '');
  const btn = h('button.primary', { type: 'submit', 'data-testid': 'login-submit' }, t('login.submit'));
  const form = h('form.login.glass', {
    onsubmit: async (ev) => {
      ev.preventDefault();
      btn.disabled = true;
      err.hidden = true;
      try {
        state.session = await login(user.value.trim(), pass.value);
        await start();
      } catch (e) {
        err.textContent = errText(e);
        err.hidden = false;
        pass.select();
      } finally {
        btn.disabled = false;
      }
    },
  },
  h('div.brand', icon('mark'), h('div', 'Kinolune OS', h('small', t('login.subtitle')))),
  h('label.field', t('login.username'), user),
  h('label.field', t('login.password'), pass),
  err,
  btn,
  h('p.caption', t('login.hint')),
  langPicker());
  app.replaceChildren(form);
  user.focus();
}

function langPicker() {
  const names = { ru: 'Русский', en: 'English', uk: 'Українська' };
  const sel = h('select', { 'aria-label': t('common.language'), 'data-testid': 'lang', onchange: async () => { await setLang(sel.value); route(); } },
    LANGS.map((l) => h('option', { value: l, selected: l === currentLang() }, names[l])));
  return sel;
}

// --- shell -------------------------------------------------------------------------------

function nav(active) {
  const links = Object.entries(PAGES)
    .filter(([, p]) => !p.feature || state.features[p.feature])
    .map(([id, p]) => h('a', { href: `#/${id}`, 'aria-current': id === active ? 'page' : null, 'data-testid': `nav-${id}` }, icon(p.icon), t(p.title)));
  return h('nav.nav', { 'aria-label': t('nav.label') },
    h('div.brand', icon('mark'), h('div', state.system?.hostname || 'Kinolune OS', h('small', state.system?.os_version ? `Kinolune OS ${state.system.os_version}` : 'Kinolune OS'))),
    links,
    h('div.spacer'),
    h('div.who',
      h('span', t('nav.signed_in', { user: state.session.user })),
      langPicker(),
      h('button.ghost.small', { type: 'button', 'data-testid': 'logout', onclick: async () => { await logout(); state.session = null; renderLogin(); } }, icon('logout'), t('nav.logout'))));
}

export async function route() {
  if (!state.session) return renderLogin();
  stopTimer();
  const id = (location.hash.replace(/^#\/?/, '') || 'dashboard').split('/')[0];
  const page = PAGES[id] && (!PAGES[id].feature || state.features[PAGES[id].feature]) ? id : 'dashboard';
  state.page = page;
  app.className = 'app';
  const main = h('main.main', { 'data-page': page });
  app.replaceChildren(nav(page), main);
  document.title = `${t(PAGES[page].title)} — Kinolune OS`;
  try {
    await PAGES[page].render(main, { state, route });
  } catch (e) {
    if (e instanceof ApiError && e.status === 401) return;
    main.append(h('div.alert.bad', errText(e)));
  }
}

async function start() {
  state.features = await detectFeatures();
  try { state.system = await get('/api/v1/system'); } catch { state.system = null; }
  route();
}

onLoggedOut(() => {
  if (!state.session) return;
  state.session = null;
  renderLogin(t('login.expired'));
});
window.addEventListener('hashchange', () => { if (state.session) route(); });

// Weak devices and "less transparency": no blur (palette v2, req. 2).
if (localStorage.getItem('kinolune-admin-no-blur') === '1' || (navigator.hardwareConcurrency || 4) <= 2) {
  document.documentElement.classList.add('no-blur');
}

(async () => {
  let installLang = '';
  state.session = await session().catch(() => null);
  if (state.session) {
    try { state.system = await get('/api/v1/system'); installLang = state.system?.install?.language || ''; } catch { /* shown later */ }
  }
  await initI18n(installLang);
  if (state.session) await start();
  else renderLogin();
})();
