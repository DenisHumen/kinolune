// SPDX-License-Identifier: MPL-2.0
// Copies the shared design assets into the site before a build (SITE-001):
// generated design tokens (colours come only from here, so the palette v2
// swap in design/tokens needs no site change), the logo, favicons and the
// release screenshots. The copies are git-ignored.
import { copyFileSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const site = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(site, '..', '..');

const files = [
  ['design/tokens/generated/web/tokens.css', 'src/generated/tokens.css'],
  ['design/brand/export/svg/lockup-dark.svg', 'public/assets/lockup-dark.svg'],
  ['design/brand/export/web/favicon.svg', 'public/favicon.svg'],
  ['design/brand/export/web/favicon.ico', 'public/favicon.ico'],
  ['design/brand/export/web/apple-touch-icon.png', 'public/apple-touch-icon.png'],
  ['docs/screenshots/0.1/desktop-home.png', 'public/assets/desktop-home.png'],
  ['docs/screenshots/0.1/phone-home.png', 'public/assets/phone-home.png'],
  ['docs/screenshots/0.1/tv-home.png', 'public/assets/tv-home.png'],
];

rmSync(join(site, 'src/generated'), { recursive: true, force: true });
rmSync(join(site, 'public'), { recursive: true, force: true });
for (const [from, to] of files) {
  mkdirSync(dirname(join(site, to)), { recursive: true });
  copyFileSync(join(repo, from), join(site, to));
}
console.log(`sync-assets: ${files.length} files`);
