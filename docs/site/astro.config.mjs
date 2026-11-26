// SPDX-License-Identifier: MPL-2.0
// Kinolune website (SITE-001, ADR-NNNN). Static build for GitHub Pages.
//
// The site lives at the root of the project's Pages artifact, next to the
// APT/DNF repositories (/apt, /rpm, /kinolune.asc) that linux-repo.yml adds
// to the same artifact. SITE_URL and SITE_BASE switch it to a custom domain
// later (SITE-006) without touching the pages.
import { defineConfig } from 'astro/config';
import { fileURLToPath } from 'node:url';

const repoRoot = fileURLToPath(new URL('../..', import.meta.url));

export default defineConfig({
  site: process.env.SITE_URL ?? 'https://denishumen.github.io',
  base: process.env.SITE_BASE ?? '/kinolune',
  trailingSlash: 'always',
  build: { format: 'directory' },
  // No telemetry, no external requests at build time.
  devToolbar: { enabled: false },
  vite: { server: { fs: { allow: [repoRoot] } } },
});
