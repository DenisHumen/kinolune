// SPDX-License-Identifier: MPL-2.0
// Strings: static/i18n/{ru,en,uk}.json. Russian is the default and the fallback.

export const LANGS = ['ru', 'en', 'uk'];
const KEY = 'kinolune-admin-lang';
let dict = {};
let fallback = {};
let lang = 'ru';

async function load(l) {
  const res = await fetch(`/static/i18n/${l}.json`);
  if (!res.ok) throw new Error(`i18n ${l}: ${res.status}`);
  return res.json();
}

function pick(installLang) {
  const saved = localStorage.getItem(KEY);
  if (LANGS.includes(saved)) return saved;
  const cands = [installLang, ...(navigator.languages || [navigator.language])];
  for (const c of cands) {
    const base = String(c || '').toLowerCase().split(/[-_]/)[0];
    if (LANGS.includes(base)) return base;
  }
  return 'ru';
}

/** Loads the language: saved choice, then the OS install language, then the browser's. */
export async function initI18n(installLang) {
  fallback = await load('ru');
  await setLang(pick(installLang), false);
}

export async function setLang(l, save = true) {
  lang = LANGS.includes(l) ? l : 'ru';
  dict = lang === 'ru' ? fallback : await load(lang);
  document.documentElement.lang = lang;
  if (save) localStorage.setItem(KEY, lang);
}

export const currentLang = () => lang;

/** Translates `key`, replacing {name} placeholders from `vars`. */
export function t(key, vars) {
  let s = dict[key] ?? fallback[key] ?? key;
  if (vars) s = s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m));
  return s;
}

/** Bytes as KB/MB/GB/TB (decimal, like disk vendors). */
export function bytes(n) {
  if (!Number.isFinite(n) || n <= 0) return '—';
  const u = [t('unit.b'), t('unit.kb'), t('unit.mb'), t('unit.gb'), t('unit.tb')];
  let i = 0;
  while (n >= 1000 && i < u.length - 1) { n /= 1000; i++; }
  return `${new Intl.NumberFormat(lang, { maximumFractionDigits: n < 10 && i ? 1 : 0 }).format(n)} ${u[i]}`;
}

export function duration(sec) {
  sec = Math.max(0, Math.floor(sec || 0));
  const d = Math.floor(sec / 86400), h = Math.floor((sec % 86400) / 3600), m = Math.floor((sec % 3600) / 60);
  if (d) return t('dur.dh', { d, h });
  if (h) return t('dur.hm', { h, m });
  return t('dur.m', { m });
}

export function dateTime(s) {
  if (!s) return '—';
  const d = new Date(s);
  if (Number.isNaN(d.getTime()) || d.getFullYear() < 2000) return '—';
  return new Intl.DateTimeFormat(lang, { dateStyle: 'medium', timeStyle: 'short' }).format(d);
}

export const num = (n, digits = 0) => new Intl.NumberFormat(lang, { maximumFractionDigits: digits }).format(n);
