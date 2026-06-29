#!/usr/bin/env node
// Storybook-shape validate. Previews are iframes of the repo's own storybook,
// so the render check is just "does _sb/iframe.html load a story". The
// substantive checks are on the importable bundle: every component is a
// function on window.<Global>, and styles.css carries the DS tokens.
//
// Usage: node storybook/validate.mjs ./ds-bundle

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { serveDir } from './http-serve.mjs';

const OUT = resolve(process.argv[2] ?? '.');
if (!existsSync(join(OUT, '_ds_bundle.js'))) {
  console.error('usage: node storybook/validate.mjs <out-dir>');
  process.exit(2);
}
const firstLine = readFileSync(join(OUT, '_ds_bundle.js'), 'utf8').split('\n', 1)[0];
const meta = JSON.parse(firstLine.match(/^\/\* @ds-bundle: (.+?) \*\/$/)?.[1] ?? '{}');
const GLOBAL = meta.namespace;
const names = (meta.components ?? []).map((c) => c.name);
const sideband = existsSync(join(OUT, '.stories.json'))
  ? JSON.parse(readFileSync(join(OUT, '.stories.json'), 'utf8')) : {};
const storiesByName = sideband.stories ?? {};
const PROVIDER = sideband.provider ?? null;

let failed = 0;
const fail = (m) => { console.error(`✗ ${m}`); failed++; };
const ok = (m) => console.error(`✓ ${m}`);

// ── static checks ────────────────────────────────────────────────────────
const stylesCss = existsSync(join(OUT, 'styles.css')) ? readFileSync(join(OUT, 'styles.css'), 'utf8') : '';
const tokens = [...stylesCss.matchAll(/--([a-z][\w-]*)\s*:/gi)].map((m) => m[1]);
if (tokens.length) ok(`styles.css: ${tokens.length} CSS custom properties`);
else console.error('[TOKENS_MISSING] styles.css has no --custom-properties — check storybook-static CSS was concatenated');
const fontFaces = (stylesCss.match(/@font-face\b/g) ?? []).length;
if (fontFaces) ok(`styles.css: ${fontFaces} @font-face rule(s)`);
// [FONT_MISSING]: families referenced by styles.css but no @font-face ships them.
{
  const SYSTEM_FONTS = new Set([
    '-apple-system', 'blinkmacsystemfont', 'system-ui', 'ui-sans-serif', 'ui-serif',
    'ui-monospace', 'sans-serif', 'serif', 'monospace', 'arial', 'helvetica',
    'helvetica neue', 'roboto', 'segoe ui', 'times', 'times new roman', 'courier',
    'courier new', 'menlo', 'consolas', 'monaco', 'inherit', 'initial', 'unset',
  ]);
  const norm = (s) => s.replace(/^['"]|['"]$/g, '').trim().toLowerCase();
  const provided = new Set(
    [...stylesCss.matchAll(/@font-face\s*\{[^}]*?font-family\s*:\s*([^;}]+)/gi)]
      .map((m) => norm(m[1])),
  );
  const referenced = new Set(
    [...stylesCss.matchAll(/(?<!@font-face\s*\{[^}]*?)font-family\s*:\s*([^;}]+)/gi)]
      .flatMap((m) => m[1].split(',').map(norm)),
  );
  const missing = [...referenced].filter(
    (f) => f && !provided.has(f) && !SYSTEM_FONTS.has(f) && !/^var\(/.test(f),
  );
  if (missing.length) {
    console.error(`[FONT_MISSING] ${missing.slice(0, 6).join(', ')}${missing.length > 6 ? `, +${missing.length - 6}` : ''} — referenced in styles.css but no @font-face. Check .storybook/preview-head.html for a host-provided <link>, or add via cfg.extraFonts.`);
  }
}
const sbBytes = (function du(d) { let n = 0; for (const e of readdirSync(d, { withFileTypes: true })) { const p = join(d, e.name); n += e.isDirectory() ? du(p) : statSync(p).size; } return n; })(join(OUT, '_sb'));
if (sbBytes > 50 * 1024 * 1024) console.error(`[SB_SIZE] _sb/ is ${(sbBytes / 1024 / 1024).toFixed(0)} MB`);

const readme = existsSync(join(OUT, 'README.md')) ? readFileSync(join(OUT, 'README.md'), 'utf8') : '';
if (/Provider/i.test(readme) || meta.provider) ok('README: provider documented');

// ── chromium checks ──────────────────────────────────────────────────────
let pw;
try { pw = await import('playwright'); }
catch {
  console.error('[NO_CHROMIUM] playwright not installed — bundle-smoke + iframe check skipped');
  console.log(JSON.stringify({ components: names.length, exported: null, iframeLoads: null, skipped: 'NO_CHROMIUM' }));
  process.exit(failed ? 1 : 0);
}

const { srv, port } = await serveDir(OUT);
let browser, exportedCount = null, badExports = [], iframeLoads = null, iframeChecked = 0, iframeTotal = 0;
try {
  for (let i = 0; ; i++) {
    try { browser = await pw.chromium.launch(process.env.DS_CHROMIUM_PATH ? { executablePath: process.env.DS_CHROMIUM_PATH } : {}); break; }
    catch (e) { if (i === 2) throw e; await new Promise((r) => setTimeout(r, 1000)); }
  }
  const page = await browser.newPage();

  // [BUNDLE_EXPORT] + [BUNDLE_STYLE]
  await page.goto(`http://127.0.0.1:${port}/`).catch(() => {});
  await page.setContent(
    `<!doctype html><link rel="stylesheet" href="/styles.css">` +
    `<script src="/_vendor/react.js"></script><script src="/_vendor/react-dom.js"></script>` +
    `<script src="/_ds_bundle.js"></script><div id="r"></div>`,
  );
  await page.waitForFunction((g) => window[g], GLOBAL, { timeout: 10_000 }).catch(() => {});
  const { exp, compound, bad } = await page.evaluate(({ g, ns }) => {
    const NS = window[g] ?? {};
    const isFn = (v) => typeof v === 'function' || (v && v.$$typeof);
    const isCompound = (v) => v && typeof v === 'object' && Object.values(v).some(isFn);
    const compound = [], bad = [];
    for (const n of ns) {
      if (isFn(NS[n])) continue;
      if (isCompound(NS[n])) compound.push(n);
      else bad.push(n);
    }
    return { exp: Object.keys(NS).length, compound, bad };
  }, { g: GLOBAL, ns: names });
  exportedCount = exp; badExports = bad;
  if (compound.length) console.error(`[BUNDLE_EXPORT] ${compound.length} compound namespace(s) (usable via .Sub): ${compound.slice(0, 8).join(', ')}${compound.length > 8 ? ', …' : ''}`);
  if (bad.length) fail(`[BUNDLE_EXPORT] ${bad.length}/${names.length} not a component on window.${GLOBAL}: ${bad.slice(0, 8).join(', ')}${bad.length > 8 ? ', …' : ''}`);
  else ok(`window.${GLOBAL}: ${exp} exports (${names.length - compound.length} fn + ${compound.length} compound)`);

  // [BUNDLE_STYLE]: render one component (in provider if known), check the
  // rendered element got SOME styling (token resolves / non-default font /
  // has class|style attr / non-default color). Covers CSS-in-JS DSes where
  // tokens[] is empty.
  // Compound namespaces won't mount on h(NS[n], {}) — pick a callable export.
  const noncompound = names.filter((n) => !compound.includes(n) && !bad.includes(n));
  const firstName = noncompound.find((n) => /Button|Card|Text|Badge|Box|Alert|Input/i.test(n)) ?? noncompound[0];
  if (firstName) {
    const r = await page.evaluate(({ g, n, prov, t }) => {
      try {
        const React = window.React, ReactDOM = window.ReactDOM, NS = window[g];
        const h = React.createElement;
        const deref = (o) => { const r = {}; for (const k in o) { const v = o[k]; r[k] = v && v.$hint !== undefined ? undefined : v; } return r; };
        const wrap = [];
        for (let p = prov; p; p = p.inner) wrap.push(p);
        let el = h(NS[n], {});
        for (let i = wrap.length - 1; i >= 0; i--) el = h(NS[wrap[i].component] ?? React.Fragment, deref(wrap[i].props ?? {}), el);
        const host = document.getElementById('r');
        (ReactDOM.createRoot ? ReactDOM.createRoot(host) : { render: (e) => ReactDOM.render(e, host) }).render(el);
        return new Promise((res) => setTimeout(() => {
          let child = host; while (child.firstElementChild) child = child.firstElementChild;
          if (child === host) return res({ mounted: false, err: 'host empty after render' });
          const cs = getComputedStyle(child);
          const tokenVal = t ? getComputedStyle(document.documentElement).getPropertyValue(`--${t}`).trim() : '';
          res({
            mounted: true, tokenVal,
            hasClass: !!child.className || child.hasAttribute('style'),
            font: cs.fontFamily, color: cs.color, bg: cs.backgroundColor,
          });
        }, 100));
      } catch (e) { return { mounted: false, err: String(e).slice(0, 200) }; }
    }, { g: GLOBAL, n: firstName, prov: PROVIDER, t: tokens[0] });
    if (!r.mounted) {
      fail(`[BUNDLE_MOUNT] rendering ${firstName} threw: ${r.err}`);
    } else {
      const styled = r.tokenVal || r.hasClass || !/Times|serif|-apple-system|system-ui/i.test(r.font) || r.color !== 'rgb(0, 0, 0)';
      if (styled) ok(`[BUNDLE_STYLE] ${firstName} rendered with styling${r.tokenVal ? ` (--${tokens[0]}=${r.tokenVal})` : r.hasClass ? ' (class/style attr)' : ` (font=${r.font})`}`);
      else fail(`[BUNDLE_STYLE] ${firstName} rendered but got zero styling — check styles.css / _ds_bundle.css + cfg.provider props (theme)`);
    }
  }

  // [IFRAME_LOAD] — each component's first story. Also sample the root's
  // computed style so an all-unstyled storybook (CSS stripped from _sb/)
  // surfaces as [IFRAME_STYLE], not just a rendered count.
  const iframeBad = [];
  let iframeDefaultFont = 0;
  const capMs = (Number(process.env.DS_VALIDATE_CAP_SECONDS) || 600) * 1000;
  const started = Date.now();
  const entries = Object.entries(storiesByName);
  // CSS-in-JS runtimes (mantine, chakra) inject <style>/<script> as the first root
  // child; waitForSelector locks onto the first match and times out waiting for it.
  const CONTENT = '#storybook-root > :not(style,script,link,meta,template)';
  let checked = 0;
  for (const [name, sid] of entries) {
    if (!sid) { checked++; continue; }
    if (Date.now() - started > capMs) {
      console.error(`[VALIDATE_PARTIAL] wall-clock cap (${capMs / 1000}s) hit at ${name}; ${entries.length - checked} unchecked`);
      break;
    }
    checked++;
    try {
      await page.goto(`http://127.0.0.1:${port}/_sb/iframe.html?id=${encodeURIComponent(sid)}&viewMode=story`, { waitUntil: 'domcontentloaded', timeout: 15_000 });
    } catch { iframeBad.push({ name, err: 'goto timeout' }); continue; }
    const loaded = await page.waitForSelector(CONTENT, { timeout: 5_000 }).then(() => true).catch(() => false);
    if (!loaded) {
      const err = await page.evaluate(() => {
        const e = document.querySelector('.sb-errordisplay');
        return e && getComputedStyle(e).display !== 'none' ? e.textContent?.slice(0, 120) : 'no #storybook-root content';
      }).catch(() => '?');
      iframeBad.push({ name, err });
      continue;
    }
    const font = await page.evaluate((s) => getComputedStyle(document.querySelector(s)).fontFamily, CONTENT).catch(() => '');
    if (/^"?Times|^serif$/i.test(font)) iframeDefaultFont++;
  }
  const total = iframeTotal = Object.keys(storiesByName).length;
  iframeLoads = checked - iframeBad.length;
  const partial = checked < total;
  if (checked && iframeDefaultFont > checked * 0.5) {
    fail(`[IFRAME_STYLE] ${iframeDefaultFont}/${checked} iframe stories render with browser-default font — storybook-static CSS likely stripped or mis-pathed`);
  } else if (iframeDefaultFont) {
    console.error(`[IFRAME_STYLE] ${iframeDefaultFont}/${checked} stories have default font (may be intentional)`);
  }
  if (iframeBad.length) {
    fail(`[IFRAME_LOAD] ${iframeBad.length}/${checked} failed${partial ? ` (${total - checked} unchecked)` : ''}`);
    const byErr = new Map();
    for (const b of iframeBad) byErr.set(b.err, [...(byErr.get(b.err) ?? []), b.name]);
    for (const [err, ns] of byErr) console.error(`    ${ns.slice(0, 5).join(', ')}${ns.length > 5 ? `, +${ns.length - 5}` : ''}: ${err}`);
  } else ok(`[IFRAME_LOAD] all ${checked}${partial ? `/${total} checked` : ''} first-stories rendered`);
  iframeChecked = checked;
} catch (e) {
  fail(`validate: ${String(e).split('\n')[0]}`);
} finally {
  await browser?.close().catch(() => {});
  srv.close();
}

console.log(JSON.stringify({ components: names.length, exported: exportedCount, badExports: badExports.length, iframeLoads, iframeChecked, iframeTotal, tokens: tokens.length }));
process.exit(failed ? 1 : 0);
