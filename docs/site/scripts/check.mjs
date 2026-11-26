// SPDX-License-Identifier: MPL-2.0
// Website checks (SITE-001), run in CI after `npm run build`:
//   - every locale has the same string keys;
//   - no colour literals in the site's own styles (colours = design tokens);
//   - the build has the pages it must have and nothing at the paths that the
//     APT/DNF repositories own in the same Pages artifact (/apt, /rpm,
//     /kinolune.asc — linux-repo.yml, PKG-023);
//   - every internal link and #anchor resolves;
//   - no third-party scripts, styles or fonts (the site promises no trackers).
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { locales, strings } from '../src/i18n/strings.mjs';

const site = join(dirname(fileURLToPath(import.meta.url)), '..');
const dist = join(site, 'dist');
const base = (process.env.SITE_BASE ?? '/kinolune').replace(/\/?$/, '/');
const errors = [];

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

// 1. Same keys in every locale.
const reference = Object.keys(strings[locales[0]]).sort();
for (const l of locales) {
  const keys = Object.keys(strings[l]).sort();
  for (const k of reference) if (!keys.includes(k)) errors.push(`i18n: "${k}" missing in ${l}`);
  for (const k of keys) if (!reference.includes(k)) errors.push(`i18n: "${k}" only in ${l}`);
  for (const k of keys) if (!String(strings[l][k]).trim()) errors.push(`i18n: "${k}" empty in ${l}`);
}

// 2. Colour literals in own styles.
const colour = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(|(?<![-\w])(?:white|black)(?![-\w])/;
for (const file of walk(join(site, 'src'))) {
  if (file.includes(`${join('src', 'generated')}`)) continue;
  const text = readFileSync(file, 'utf8');
  let css = '';
  if (file.endsWith('.css')) css = text;
  else if (file.endsWith('.astro')) {
    css = [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>|style="([^"]*)"/g)]
      .map((m) => m[1] ?? m[2])
      .join('\n');
  }
  css = css.replace(/\/\*[\s\S]*?\*\//g, '');
  css.split('\n').forEach((line, i) => {
    if (colour.test(line)) errors.push(`colour literal in ${relative(site, file)}: ${line.trim()} (use a --pa-* token)`);
  });
}

// 3–5. The built site.
if (!existsSync(dist)) {
  errors.push('dist/ is missing: run `npm run build` first');
} else {
  const required = ['index.html', '404.html', 'favicon.svg', ...locales.map((l) => `${l}/index.html`)];
  for (const f of required) if (!existsSync(join(dist, f))) errors.push(`build: ${f} is missing`);
  for (const f of ['apt', 'rpm', 'kinolune.asc']) {
    if (existsSync(join(dist, f))) errors.push(`build: /${f} belongs to the Linux repositories (linux-repo.yml)`);
  }

  const pages = walk(dist).filter((f) => f.endsWith('.html'));
  const idsOf = new Map();
  const ids = (file) => {
    if (!idsOf.has(file)) {
      idsOf.set(file, new Set([...readFileSync(file, 'utf8').matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
    }
    return idsOf.get(file);
  };
  const resolve = (path) => {
    const p = join(dist, path);
    if (existsSync(p) && statSync(p).isFile()) return p;
    if (existsSync(join(p, 'index.html'))) return join(p, 'index.html');
    return null;
  };

  for (const page of pages) {
    const html = readFileSync(page, 'utf8');
    const name = relative(dist, page);
    for (const m of html.matchAll(/\s(?:href|src)="([^"]+)"/g)) {
      const url = m[1];
      if (/^(https?:)?\/\//.test(url) || url.startsWith('mailto:')) continue;
      const [path, anchor] = url.split('#');
      let target = page;
      if (path) {
        if (!path.startsWith(base)) {
          errors.push(`${name}: link outside the site base ${base}: ${url}`);
          continue;
        }
        target = resolve(path.slice(base.length));
        if (!target) {
          errors.push(`${name}: broken link ${url}`);
          continue;
        }
      }
      if (anchor && target.endsWith('.html') && !ids(target).has(anchor)) {
        errors.push(`${name}: missing anchor ${url}`);
      }
    }
    for (const m of html.matchAll(/<(script|link)\b[^>]*>/g)) {
      const tag = m[0];
      const ext = /\s(?:src|href)="(?:https?:)?\/\//.test(tag);
      if (ext && (m[1] === 'script' || /rel="(?:stylesheet|preload|modulepreload|preconnect|dns-prefetch)"/.test(tag))) {
        errors.push(`${name}: third-party resource ${tag}`);
      }
    }
    if (/@import\s+url\(\s*["']?https?:|fonts\.googleapis/.test(html)) errors.push(`${name}: third-party font or style`);
  }
  for (const css of walk(dist).filter((f) => f.endsWith('.css'))) {
    if (/url\(\s*["']?(?:https?:)?\/\//.test(readFileSync(css, 'utf8'))) {
      errors.push(`${relative(dist, css)}: third-party url() in CSS`);
    }
  }
  console.log(`site check: ${pages.length} pages`);
}

if (errors.length) {
  for (const e of errors) console.error(`✗ ${e}`);
  process.exit(1);
}
console.log('site check: OK');
