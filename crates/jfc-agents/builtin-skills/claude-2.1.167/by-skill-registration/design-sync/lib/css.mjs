// CSS handling: token-file copy, @font-face extraction, and the final
// styles.css writer (the DS-v2 styles entry point — never flattened).
// Storybook-only fallbacks live in css-fallback.mjs.

import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve, sep } from 'node:path';
import { ls } from './common.mjs';

// Parse @font-face blocks from `cssPath` → resolve url() paths relative to
// `srcDir` → copy .woff2/.woff/.ttf/.otf to fonts/ → return rewritten rules.
// `roots` bounds the resolved path so a `url(../../etc/passwd)` can't escape —
// one or more directories the font file may legitimately be under.
export function extractFonts(cssPath, srcDir, { fontsOut, roots }) {
  const rootPrefixes = (Array.isArray(roots) ? roots : [roots]).map((r) => resolve(r) + sep);
  if (!existsSync(cssPath)) return [];
  const css = readFileSync(cssPath, 'utf8');
  const rules = [];
  for (const m of css.matchAll(/@font-face\s*\{([^}]+)\}/g)) {
    const body = m[1];
    const fam = body.match(/font-family\s*:\s*['"]?([^;'"\n]+)['"]?/)?.[1]?.trim();
    const urls = [...body.matchAll(/url\(\s*['"]?([^'")]+?\.(?:woff2?|ttf|otf))['"]?\s*\)/gi)].map((u) => u[1]);
    if (!fam || !urls.length) continue;
    let rewritten = body;
    for (const u of urls) {
      if (/^(https?:|data:)/.test(u)) continue; // CDN / inline — leave as-is
      const src = resolve(srcDir, u.replace(/^\.\//, ''));
      if (!rootPrefixes.some((p) => src.startsWith(p)) || !existsSync(src)) continue;
      const name = basename(src);
      mkdirSync(fontsOut, { recursive: true });
      cpSync(src, join(fontsOut, name));
      rewritten = rewritten.split(u).join(`./${name}`);
    }
    rules.push(`@font-face{${rewritten}}`);
  }
  return rules;
}

// Copy a tokens package's shipped CSS verbatim into OUT/tokens/. tokensGlob
// supports a single trailing `**` segment for deep recursion.
export function copyTokens({ tokensPkg, tokensGlob, nodeModules, out }) {
  const tokenFiles = [];
  if (!tokensPkg) return tokenFiles;
  const tdir = join(nodeModules, tokensPkg);
  const tjson = JSON.parse(readFileSync(join(tdir, 'package.json'), 'utf8'));
  if (tokensGlob) {
    const parts = tokensGlob.split('/');
    const pat = parts.pop();
    const rx = new RegExp('^' + pat.replace(/\./g, '\\.').replace(/\*/g, '.*') + '$');
    const deep = parts.includes('**');
    const base = join(tdir, ...parts.filter((p) => p !== '**'));
    (function collect(d, rel = '') {
      if (!existsSync(d)) return;
      for (const e of ls(d, { withFileTypes: true })) {
        const r = rel ? `${rel}/${e.name}` : e.name;
        if (e.isDirectory() && deep) collect(join(d, e.name), r);
        else if (e.isFile() && rx.test(e.name)) {
          // Preserve subdir structure so @import './sub/x.css' inside a
          // copied file keeps resolving.
          mkdirSync(dirname(join(out, 'tokens', r)), { recursive: true });
          cpSync(join(d, e.name), join(out, 'tokens', r));
          tokenFiles.push(r);
        }
      }
    })(base);
  } else {
    for (const sub of ['dist/css', 'css', 'dist', '.']) {
      const d = join(tdir, sub);
      if (!existsSync(d)) continue;
      for (const f of ls(d)) {
        if (f.endsWith('.css')) {
          cpSync(join(d, f), join(out, 'tokens', f));
          tokenFiles.push(f);
        }
      }
      if (tokenFiles.length) break;
    }
  }
  console.error(`  tokens: ${tokenFiles.length} files from ${tokensPkg}@${tjson.version}`);
  return tokenFiles;
}

// styles.css — the DS-v2 styles entry point. The claude.ai/design app's
// styles-closure walker reads this, follows @imports, and extracts tokens —
// so it should chain
// ONLY token/font CSS. `_ds_bundle.css` (component-local styles) is linked
// separately by each preview.html; importing it here would make every
// component-local `--*` var show up as an unscoped "system token" in the
// self-check.
export function writeStylesCss({ out, tokenFiles, bundleCss, fontRules, remoteImports }) {
  const styleImports = [
    ...tokenFiles.map((f) => `@import "./tokens/${f}";`),
    ...(fontRules.length ? ['@import "./fonts/fonts.css";'] : []),
    ...remoteImports.map((u) => `@import url("${u}");`),
  ];
  if (styleImports.length) {
    writeFileSync(join(out, 'styles.css'), styleImports.join('\n') + '\n');
    console.error(`  styles.css: ${styleImports.length} @import(s)`);
  } else {
    writeFileSync(
      join(out, 'styles.css'),
      '/* @ds-styles: runtime — this design system injects its styles at runtime (CSS-in-JS); no static stylesheet to import. */\n',
    );
    console.error('[CSS_RUNTIME] no static CSS found (tokens/component/fonts/remote all empty) — wrote a self-styling styles.css. Expected for CSS-in-JS DSes; if this DS does ship a stylesheet, set cfg.cssEntry to it.');
  }
}
