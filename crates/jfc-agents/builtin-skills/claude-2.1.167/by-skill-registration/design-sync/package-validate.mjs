#!/usr/bin/env node
// Validation for a package-build.mjs output dir. File-shape checks ensure
// the bundle is complete and well-formed; an optional render check (when
// playwright is importable) additionally samples a few stories via
// <Name>.html and asserts its first root is non-empty.
//
// Usage: node package-validate.mjs <out-dir>

import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, relative, resolve, sep } from 'node:path';

const OUT = process.argv[2];
if (!OUT || !existsSync(OUT)) {
  console.error('usage: node package-validate.mjs <out-dir> [--render-sample N]');
  process.exit(1);
}
const rsFlag = process.argv.indexOf('--render-sample');
const RENDER_SAMPLE = rsFlag > 0 ? Number(process.argv[rsFlag + 1]) || 0 : 0;

// Bundle-relative path for reporting and render-check URLs. relative() (not a
// length-based slice) so OUT spelled as `./out`, with a trailing slash, or
// backslashed still yields `components/...` — a prefix-length mismatch used to
// shear leading characters off every rel and 404 every render-check URL.
const relOut = (p) => relative(OUT, p).replaceAll('\\', '/');

let errors = 0;
let warnings = 0;
const fail = (msg) => { errors++; console.error(`✗ ${msg}`); };
const warn = (msg) => { warnings++; console.error(`! ${msg}`); };
const ok = (msg) => console.error(`  ${msg}`);

// .ds-build-meta.json well-formed (local-only build metadata; not uploaded).
let ver;
try {
  ver = JSON.parse(readFileSync(join(OUT, '.ds-build-meta.json'), 'utf8'));
  ok(`.ds-build-meta.json: ${ver.componentCount} components (${ver.shape})`);
} catch (e) { fail(`.ds-build-meta.json: ${e.message}`); }

// _ds_bundle.js exists at root + loadable (syntax-valid IIFE) + a well-formed
// first-line `/* @ds-bundle: {…} */` header the claude.ai/design app's
// self-check parses.
const bundleJs = join(OUT, '_ds_bundle.js');
if (!existsSync(bundleJs)) fail('_ds_bundle.js missing — [NO_DIST] the package build failed');
else {
  const src = readFileSync(bundleJs, 'utf8');
  const kb = (statSync(bundleJs).size / 1024).toFixed(0);
  try { new Function(src); ok(`_ds_bundle.js: ${kb} KB, syntax OK`); }
  catch (e) { fail(`_ds_bundle.js: syntax error — ${e.message}`); }
  // Header: first line only, un-escape `*\/`.
  const m = /^\/\* @ds-bundle: (.*) \*\//.exec(src.split('\n', 1)[0]);
  if (!m) fail('_ds_bundle.js: missing first-line `/* @ds-bundle: {…} */` header');
  else {
    try {
      const meta = JSON.parse(m[1].replace(/\*\\\//g, '*/'));
      const missing = ['namespace', 'components', 'sourceHashes', 'inlinedExternals'].filter(
        (k) => meta[k] === undefined,
      );
      if (missing.length) fail(`_ds_bundle.js header missing field(s): ${missing.join(', ')}`);
      else if (typeof meta.namespace !== 'string' || !Array.isArray(meta.components)) {
        fail('_ds_bundle.js header: namespace must be a string and components an array');
      } else ok(`_ds_bundle.js header: window.${meta.namespace}, ${meta.components.length} components, ${meta.inlinedExternals.length} inlined externals`);
    } catch (e) { fail(`_ds_bundle.js header: invalid JSON — ${e.message}`); }
  }
}

// styles.css — the styles entry point. Normally @imports ≥1 file. A CSS-in-JS
// DS legitimately has nothing to import; the build marks that case with a
// `@ds-styles: runtime` comment, which downgrades the empty file to a warning.
const stylesCss = join(OUT, 'styles.css');
if (!existsSync(stylesCss)) fail('styles.css missing — the styles entry point the app reads');
else {
  const txt = readFileSync(stylesCss, 'utf8');
  // Each @import target must exist on disk — a broken relative path means
  // everything is unstyled post-upload.
  let n = 0, missing = 0;
  for (const m of txt.matchAll(/@import\s+(?:url\()?["']([^"']+)["']/g)) {
    n++;
    if (/^https?:|^data:/.test(m[1])) continue;
    if (!existsSync(join(OUT, m[1]))) { missing++; fail(`[CSS_IMPORT_MISSING] styles.css @imports "${m[1]}" which doesn't exist under ${OUT}`); }
  }
  if (n > 0) { if (!missing) ok(`styles.css: ${n} @import(s), all resolve`); }
  else if (/@ds-styles:\s*runtime/.test(txt)) {
    warn('[CSS_RUNTIME] styles.css has no @imports — DS styles itself at runtime (CSS-in-JS). OK; verify the render check passes. If the DS does ship a stylesheet, set cfg.cssEntry.');
  } else {
    fail('styles.css has no @import lines — no tokens/component/font CSS was scraped');
  }
}

// _ds_bundle.css — if present, must be real CSS (not a stub @import).
const bundleCss = join(OUT, '_ds_bundle.css');
if (existsSync(bundleCss)) {
  const sz = statSync(bundleCss).size;
  const txt = readFileSync(bundleCss, 'utf8');
  const stripped = txt.replace(/\/\*[\s\S]*?\*\//g, '').replace(/@(import|charset)\b[^;]*;/g, '').trim();
  if (txt.includes('@ds-css-runtime')) {
    console.error('[CSS_RUNTIME] _ds_bundle.css is the runtime-styles stub — expected for CSS-in-JS DSes');
  } else if (sz < 500 && stripped.length === 0) {
    fail(`[CSS_PLACEHOLDER] _ds_bundle.css is ${sz}B of @import-only stub — set cfg.cssEntry to the compiled stylesheet (storybook repos: the build-time CSS fallback should have caught this — check for [CSS_FROM_STORYBOOK] in the build log)`);
  } else ok(`_ds_bundle.css: ${(sz / 1024).toFixed(0)} KB`);
}

// Token coverage — CSS custom properties referenced by the shipped stylesheets
// but defined by none of them. Fires when the DS keeps its tokens in a sibling
// package that wasn't picked up. Skips var(--x, fallback) forms (they degrade
// gracefully) and degrades to no warning on any parse hiccup. Non-blocking —
// the §4 screenshot review is where colorless previews are caught.
try {
  const cssFiles = [bundleCss, stylesCss];
  if (existsSync(stylesCss)) {
    for (const m of readFileSync(stylesCss, 'utf8').matchAll(/@import\s+(?:url\()?["']([^"')]+)["']/g)) {
      if (!/^https?:|^data:/.test(m[1])) cssFiles.push(join(OUT, m[1]));
    }
  }
  let allCss = cssFiles.filter(p => existsSync(p)).map(p => readFileSync(p, 'utf8')).join('\n');
  // Vars the bundle's own JS sets at runtime (via setProperty / inline style)
  // count as defined — they're in what ships, just not in a .css file.
  if (existsSync(bundleJs)) {
    const js = readFileSync(bundleJs, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    for (const m of js.matchAll(/setProperty\(\s*['"`](--[\w-]+)/g)) allCss += `\n${m[1]}:;`;
    for (const m of js.matchAll(/['"`](--[\w-]+)['"`]\s*:/g)) allCss += `\n${m[1]}:;`;
  }
  // Component-local vars are often defined in inline <style> blocks the
  // preview HTML itself emits — those ship and are part of the closure.
  (function scanStyles(d) {
    if (!existsSync(d)) return;
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) scanStyles(p);
      else if (e.name.endsWith('.html')) {
        for (const m of readFileSync(p, 'utf8').matchAll(/<style\b[^>]*>([\s\S]*?)<\/style>/gi)) allCss += '\n' + m[1];
      }
    }
  })(join(OUT, 'components'));
  const defined = new Set([...allCss.matchAll(/(--[\w-]+)\s*:/g)].map(m => m[1]));
  const referenced = [...new Set([...allCss.matchAll(/var\(\s*(--[\w-]+)\s*\)/g)].map(m => m[1]))];
  const missing = referenced.filter(v => !defined.has(v));
  if (missing.length > 3) {
    warn(`[TOKENS_MISSING] ${missing.length} CSS custom ${missing.length === 1 ? 'property' : 'properties'} referenced but not defined in shipped stylesheets: ${missing.slice(0, 8).join(', ')}${missing.length > 8 ? ', …' : ''}. Set cfg.tokensPkg (or cfg.tokensGlob) to the package that defines them, or cfg.provider if they're injected at runtime by a theme provider.`);
  } else if (referenced.length) {
    ok(`tokens: ${defined.size} defined, ${referenced.length} referenced${missing.length ? ` (${missing.length} missing, below threshold)` : ''}`);
  }
} catch {}

// Brand-font coverage — families the shipped CSS references but no shipped
// @font-face declares. Common for corporate DSes whose host app provides the
// brand font; the DS pane then renders with system substitutes. Heuristic and
// strictly non-blocking: warn() only, and any parse hiccup degrades to no
// warning.
try {
  const GENERIC_FAMILIES = new Set([
    'sans-serif', 'serif', 'monospace', 'cursive', 'fantasy', 'math', 'emoji', 'fangsong',
    'system-ui', 'ui-sans-serif', 'ui-serif', 'ui-monospace', 'ui-rounded',
    '-apple-system', 'blinkmacsystemfont', 'roboto', 'arial', 'verdana', 'tahoma', 'georgia',
    'courier', 'courier new', 'times', 'times new roman', 'apple color emoji',
    'ubuntu', 'cantarell', 'oxygen', 'fira sans', 'droid sans',
    'sf mono', 'sfmono-regular', 'menlo', 'monaco', 'consolas', 'liberation mono',
    'san francisco', 'bitstream vera sans mono', 'dejavu sans', 'dejavu sans mono',
    'hiragino kaku gothic pron', 'hiragino sans', 'yu gothic', 'yugothic', 'meiryo',
    'ms pgothic', 'ms gothic', 'osaka', 'malgun gothic', 'apple gothic',
    'mingliu', 'pmingliu', 'microsoft jhenghei', 'microsoft jhenghei ui', 'simsun', 'simhei',
    'heiti sc', 'heiti sc light', 'heiti tc', 'heiti tc light', 'pingfang sc', 'pingfang tc',
    'inherit', 'initial', 'unset', 'revert', 'revert-layer', 'none', 'auto',
    'normal', 'italic', 'bold', 'bolder', 'lighter', 'oblique', 'small-caps',
  ]);
  // cfg.runtimeFontPrefixes — family-name prefixes for fonts served at
  // runtime (via a <script> or font service, not CSS @import), so the
  // FONT_MISSING check treats them as system-equivalent.
  const runtimePrefixes = (ver?.runtimeFontPrefixes ?? []).map((p) => p.toLowerCase()).filter(Boolean);
  const isGeneric = (f) =>
    GENERIC_FAMILIES.has(f) ||
    /^(segoe ui|noto|helvetica|ui-)/.test(f) ||
    runtimePrefixes.some((p) => f.startsWith(p));
  // CSS the bundle actually ships: _ds_bundle.css, fonts/fonts.css, and the
  // styles.css local @import chain.
  const cssPaths = [bundleCss, join(OUT, 'fonts', 'fonts.css')];
  if (existsSync(stylesCss)) {
    cssPaths.push(stylesCss);
    for (const m of readFileSync(stylesCss, 'utf8').matchAll(/@import\s+(?:url\()?["']([^"')]+)["']/g)) {
      if (!/^https?:|^data:/.test(m[1])) cssPaths.push(join(OUT, m[1]));
    }
  }
  // Per-file so @font-face url()s resolve against the file they live in. A
  // family with only dangling local url()s was emitted (so [FONT_MISSING]
  // won't fire — it's in `declared`) but the font file was never copied; the
  // DS pane falls back to system fonts with no other signal.
  const declared = new Set();
  const dangling = new Map(); // lowercased family → sample url that didn't resolve
  const cssChunks = [];
  for (const p of cssPaths) {
    if (!existsSync(p)) continue;
    const chunk = readFileSync(p, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    cssChunks.push(chunk);
    for (const m of chunk.matchAll(/@font-face\s*\{([^}]+)\}/g)) {
      const fam = m[1].match(/font-family\s*:\s*['"]?([^;'"\n}]+)['"]?/)?.[1]?.trim();
      if (!fam) continue;
      const key = fam.toLowerCase();
      let hasLocal = false, hasResolved = dangling.has(key) ? false : declared.has(key);
      for (const u of m[1].matchAll(/url\(\s*['"]?([^'")]+?\.(?:woff2?|ttf|otf|eot))(?:[?#][^'")]*)?['"]?\s*\)/gi)) {
        if (/^(https?|data):/i.test(u[1])) { hasResolved = true; continue; }
        hasLocal = true;
        if (existsSync(resolve(dirname(p), u[1]))) hasResolved = true;
        else if (!dangling.has(key)) dangling.set(key, u[1]);
      }
      if (hasResolved || !hasLocal) dangling.delete(key);
      declared.add(key);
    }
  }
  const css = cssChunks.join('\n');
  // Remote font-host @import present → families are served at runtime, not
  // shipped. Soften to info instead of warning.
  const hasRemoteFonts = /@import[^;]*(?:fonts\.googleapis|fonts\.gstatic|use\.typekit|fonts\.bunny)/i.test(css);
  // Custom properties: a lookup map for one-level-at-a-time var() resolution,
  // and the source of font tokens (--*font*) components consume via var().
  const customProps = new Map();
  for (const m of css.matchAll(/(--[\w-]+)\s*:\s*([^;}]+)/g)) customProps.set(m[1], m[2].trim());
  const resolveVars = (v, depth = 0) => (depth > 3 ? v : v.replace(/var\(\s*(--[\w-]+)[^)]*\)/g, (_, name) =>
    (customProps.has(name) ? resolveVars(customProps.get(name), depth + 1) : '')));
  const missing = new Map(); // lowercased family → { display, hint }
  const collect = (value, hint) => {
    for (let part of resolveVars(String(value)).split(',')) {
      part = part.trim().replace(/^['"]|['"]$/g, '').trim();
      if (!part || !/^\p{L}/u.test(part) || !/^[\p{L}\p{N}_ -]+$/u.test(part)) continue;
      const key = part.toLowerCase();
      if (isGeneric(key) || declared.has(key) || missing.has(key)) continue;
      missing.set(key, { display: part, hint });
    }
  };
  // font-family declarations outside @font-face blocks…
  const sansFontFace = css.replace(/@font-face\s*\{[^}]*\}/g, '');
  for (const m of sansFontFace.matchAll(/font-family\s*:\s*([^;}]+)/g)) collect(m[1], null);
  // …plus font-token custom properties (--ds-font-mono and friends), skipping
  // non-family font props (size/weight/feature-settings/…).
  for (const [name, value] of customProps) {
    if (/font/i.test(name) && !/font-(feature|variation|variant|kerning|stretch|optical|smooth(?:ing)?|size|weight|style|display|color|palette|leading|numeric|case|transform|synthesis)/i.test(name)) collect(value, name);
  }
  if (missing.size) {
    const list = [...missing.values()].map((m) => `"${m.display}"${m.hint ? ` (${m.hint})` : ''}`).join(', ');
    if (hasRemoteFonts) {
      ok(`[FONT_REMOTE] ${list} — a remote font-host @import is present; assuming it serves these at runtime`);
    } else {
      warn(`[FONT_MISSING] ${list} referenced by the shipped CSS but no @font-face ships them — add the woff2 + @font-face via cfg.extraFonts, or accept substitutes (the DS pane will render with system fonts)`);
    }
  }
  if (dangling.size) {
    const list = [...dangling.entries()].map(([fam, u]) => `"${fam}" (url: ${u})`).join(', ');
    warn(`[FONT_DANGLING] ${list} — @font-face is shipped but its url() target isn't (the rule emits but the font file wasn't copied; check cfg.extraFonts paths or the build log for a "resolves outside" skip)`);
  }
} catch { /* heuristic only — never block validation on a font-parse hiccup */ }

// README + per-component files. Parity with the app's self-check: each
// preview's first line must be the @dsCard comment (else the DS pane never
// registers the card), its <link href> targets must resolve (else previews
// render unstyled), and each .prompt.md's first line must be non-empty (it's
// the element-index summary).
if (!existsSync(join(OUT, 'README.md'))) fail('README.md missing');
let previews = 0, prompts = 0, badCard = 0, badLink = 0, badPrompt = 0;
(function walk(d) {
  if (!existsSync(d)) return;
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const p = join(d, e.name);
    if (e.isDirectory()) { walk(p); continue; }
    const rel = relOut(p);
    if (e.name.endsWith('.html')) {
      previews++;
      const txt = readFileSync(p, 'utf8');
      if (!/^<!--\s*@dsCard\s+group="[^"]*"\s*-->/.test(txt.split('\n', 1)[0])) {
        badCard++; fail(`[DSCARD_MISSING] ${rel}: first line isn't a \`<!-- @dsCard group="…" -->\` comment`);
      }
      for (const m of txt.matchAll(/<link\b[^>]*\bhref="([^"]+)"/g)) {
        if (/^https?:|^data:/.test(m[1])) continue;
        // _ds_bundle.css is optional (CSS-in-JS DSes have none) — a dangling
        // <link> to it is a harmless browser 404, not a validator error.
        if (m[1].endsWith('/_ds_bundle.css') && !existsSync(bundleCss)) continue;
        if (!existsSync(resolve(dirname(p), m[1]))) {
          badLink++; fail(`[LINK_HREF_MISSING] ${rel}: <link href="${m[1]}"> doesn't resolve`);
        }
      }
    } else if (e.name.endsWith('.prompt.md')) {
      prompts++;
      if (!readFileSync(p, 'utf8').split('\n', 1)[0].trim()) {
        badPrompt++; fail(`[PROMPT_EMPTY] ${rel}: first line is empty`);
      }
    }
  }
})(join(OUT, 'components'));
const tokensOnly = ver?.componentCount === 0;
if (previews === 0 && !tokensOnly) fail('no <Name>.html previews under components/');
else if (!badCard && !badLink && !badPrompt) ok(tokensOnly ? 'tokens-only DS — no component previews' : `components/: ${previews} previews, ${prompts} .prompt.md`);
if (ver && previews !== ver.componentCount) {
  fail(`count mismatch: ${previews} previews vs ${ver.componentCount} components`);
}

// TypeScript syntax check on every emitted .d.ts — catches malformed prelude/
// body debris before it reaches the app's parser. Best-effort (needs
// typescript in node_modules, usually present via the DS's own dev deps).
try {
  const ts = await import('typescript');
  let dtsErrs = 0;
  (function walkDts(d) {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) { walkDts(p); continue; }
      if (!e.name.endsWith('.d.ts')) continue;
      const sf = ts.createSourceFile(p, readFileSync(p, 'utf8'), ts.ScriptTarget.Latest, false);
      for (const diag of sf.parseDiagnostics ?? []) {
        const { line } = sf.getLineAndCharacterOfPosition(diag.start ?? 0);
        fail(`[DTS_PARSE] ${relOut(p)}:${line + 1}: ${ts.flattenDiagnosticMessageText(diag.messageText, ' ')}`);
        dtsErrs++;
      }
    }
  })(join(OUT, 'components'));
  if (!dtsErrs) ok(`all .d.ts parse cleanly`);
} catch {
  console.error('  (.d.ts parse check skipped — typescript not in node_modules)');
}

// Render check (optional — runs when playwright is importable). Opens EVERY
// <Name>.html, captures pageerror throws, and asserts the first root is
// non-empty — catches runtime-broken bundles the file-shape checks above miss.
let pw;
try { pw = await import('playwright'); } catch { /* not installed */ }
if (!pw) {
  console.error('  (render check skipped — `npm i -D playwright && npx playwright install chromium` to enable)');
} else {
  const htmls = [];
  (function collect(d) {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) collect(p);
      else if (e.name.endsWith('.html')) htmls.push(relOut(p));
    }
  })(join(OUT, 'components'));
  // Large DSes (>RENDER_SAMPLE components) render-check a deterministic
  // sample — full pass on 200+ previews can exceed the verify-loop budget.
  // Use `--render-sample 0` for the full set.
  const sample = RENDER_SAMPLE && htmls.length > RENDER_SAMPLE
    ? htmls.filter((_, i) => i % Math.ceil(htmls.length / RENDER_SAMPLE) === 0)
    : htmls;
  if (sample.length < htmls.length) {
    console.error(`  render check: sampling ${sample.length}/${htmls.length} previews (pass --render-sample 0 for all)`);
  }
  const { createServer } = await import('node:http');
  const mime = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.png': 'image/png' };
  const root = resolve(OUT) + sep;
  const srv = createServer((req, res) => {
    let p;
    try { p = resolve(OUT, '.' + decodeURIComponent(new URL(req.url, 'http://x').pathname)); }
    catch { res.statusCode = 400; return res.end(); }
    if (!p.startsWith(root) || !existsSync(p) || !statSync(p).isFile()) { res.statusCode = 404; return res.end(); }
    let body;
    try { body = readFileSync(p); } catch { res.statusCode = 404; return res.end(); }
    res.setHeader('Content-Type', mime[extname(p)] ?? 'application/octet-stream');
    res.end(body);
  }).listen(0, '127.0.0.1');
  await new Promise((r) => srv.on('listening', r));
  const port = srv.address().port;
  const shotDir = join(OUT, '_screenshots');
  mkdirSync(shotDir, { recursive: true });
  const results = [];
  let browser;
  try {
    browser = await pw.chromium.launch();
    const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
    let pageErrs = [];
    page.on('pageerror', (e) => pageErrs.push(String(e).split('\n')[0]));
    for (const rel of sample) {
      pageErrs = [];
      // components/<group>/<Name>/<Name>.html → <group>__<Name>.png
      const [, group, name] = rel.match(/^components\/([^/]+)\/([^/]+)\//) ?? [,'misc', rel.split('/').pop()];
      const shot = join(shotDir, `${group}__${name}.png`);
      let pngBytes = 0, rootEmpty = true, err = null, caught = 0, firstCaught = null, texts = [], nEls = 0, variantsIdentical = false, hollow = [], maxHeight = 0, nPlaceholder = 0;
      try {
        await page.goto(`http://127.0.0.1:${port}/${rel}`, { waitUntil: 'networkidle', timeout: 15000 });
        // Per-mount try/catch in the preview writes `⚠ <message>` into the
        // cell instead of throwing — count those as errors too. Also collect
        // each mount's textContent / element count / painted-ness and compare
        // innerHTMLs for the thin / variantsIdentical checks below. Portal
        // roots under document.body are included so a portalled Dialog isn't
        // read as empty.
        ({ rootEmpty, caught, firstCaught, texts, nEls, variantsIdentical, hollow, maxHeight, nPlaceholder } = await page.evaluate(() => {
          // A mount "paints something" when it (or any descendant) has a
          // visible replaced element, background, border, or shadow. This
          // discriminates a Divider (1px border, paints) from an empty
          // container (paints nothing) without screenshotting each mount.
          const stylePaints = (cs) => {
            if (cs.backgroundImage !== 'none') return true;
            if (!/^(rgba\(0, 0, 0, 0\)|transparent|)$/.test(cs.backgroundColor)) return true;
            if (cs.boxShadow !== 'none') return true;
            for (const s of ['Top', 'Right', 'Bottom', 'Left']) {
              if (parseFloat(cs[`border${s}Width`]) > 0 && !/transparent|rgba\(0, 0, 0, 0\)/.test(cs[`border${s}Color`])) return true;
            }
            return false;
          };
          const paints = (root) => {
            for (const el of [root, ...root.querySelectorAll('*')]) {
              if (el.hasAttribute?.('data-ds-placeholder')) continue;
              if (/^(IMG|SVG|CANVAS|VIDEO|IFRAME|PICTURE|HR)$/.test(el.tagName)) return true;
              if (stylePaints(getComputedStyle(el))) return true;
              // Pseudo-elements (Spinner-via-::before is common). content!=none
              // means the pseudo is generated; it paints if it has text
              // content or its own border/bg/shadow.
              for (const pe of ['::before', '::after']) {
                const ps = getComputedStyle(el, pe);
                if (ps.content === 'none' || ps.content === 'normal') continue;
                if ((ps.content !== '""' && ps.content !== "''") || stylePaints(ps)) return true;
              }
            }
            return false;
          };
          const roots = document.querySelectorAll('#root, [id^="r"]');
          const portals = [...document.body.children].filter((c) =>
            !c.matches('#root, [id^="r"], .ds-grid, .ds-cell, section, script, style, link'));
          let caught = 0, firstCaught = null, nEls = 0, maxHeight = 0;
          // Document-level so it's indifferent to where the placeholder
          // landed (mount root, portal descendant, or the portal root itself).
          const nPlaceholder = document.querySelectorAll('[data-ds-placeholder]').length;
          const texts = [], htmls = [], hollow = [];
          for (const r of roots) {
            const t = r.textContent ?? '';
            if (t.startsWith('⚠')) { caught++; firstCaught ??= t.slice(2, 150); continue; }
            texts.push(t.trim());
            htmls.push(r.innerHTML);
            hollow.push(!t.trim() && !paints(r));
            nEls = Math.max(nEls, r.querySelectorAll('*').length);
            // Measure the mount's children (the component's own root(s)),
            // not the mount div — the harness cell may have intrinsic height
            // even when the component collapsed to 0. Max over all children
            // so a 0-height VisuallyHidden-first sibling doesn't mask a
            // tall second child.
            for (const el of r.children.length ? r.children : [r]) {
              maxHeight = Math.max(maxHeight, el.getBoundingClientRect().height);
            }
          }
          // Portal content counts toward every mount's text/paint/height (we
          // can't attribute a portal to a specific cell).
          const pText = portals.map((p) => p.textContent ?? '').join(' ').trim();
          const pPaints = portals.some(paints);
          for (const p of portals) maxHeight = Math.max(maxHeight, p.getBoundingClientRect().height);
          if (pText || pPaints) for (let i = 0; i < texts.length; i++) {
            if (pText) texts[i] = (texts[i] + ' ' + pText).trim();
            if (pText || pPaints) hollow[i] = false;
          }
          const variantsIdentical = htmls.length > 1 && htmls.every((h) => h === htmls[0]);
          return { rootEmpty: !roots[0]?.innerHTML?.trim().length && !portals.length, caught, firstCaught, texts, nEls, variantsIdentical, hollow, maxHeight, nPlaceholder };
        }));
        const buf = await page.screenshot({ path: shot, fullPage: true });
        pngBytes = buf.length;
      } catch (e) { err = e.message.split('\n')[0]; }
      const blank = pngBytes > 0 && pngBytes < 5000;
      const errs = pageErrs.length + caught;
      // nameOnly: at least one mount's text is just the component-name
      // placeholder, and no mount has real text beyond that. Textless-by-
      // design components (Divider, Spinner) have no name-text so don't trip.
      const squash = (s) => s.replace(/[\s_-]+/g, '').toLowerCase();
      const nameS = squash(name);
      // Name-only = the squashed text is ≥2 repetitions of the name. React
      // concatenates adjacent text nodes, so 4× `{"Name"}` children becomes
      // `"NameNameNameName"` with no separators. A single occurrence (e.g.
      // FormLabel→"Form label", Loading→"loading") is likely the component's
      // legitimate rendered label, not a placeholder; `hasPlaceholder` covers
      // the generator-emitted case.
      const nameReps = (t) => {
        const s = squash(t);
        return (s.length > 0 && s.length % nameS.length === 0
          && s === nameS.repeat(s.length / nameS.length)) ? s.length / nameS.length : 0;
      };
      const hasNameText = texts.some((t) => nameReps(t) >= 2);
      const hasRealText = texts.some((t) => t && nameReps(t) === 0);
      const nameOnly = hasNameText && !hasRealText;
      // hasPlaceholder: a `data-ds-placeholder` element is in the mounted DOM
      // — the generator's intentional dashed-box. An edit-hint, not an error.
      const hasPlaceholder = nPlaceholder > 0;
      // allHollow: every mount has no text and paints nothing.
      const allHollow = hollow.length > 0 && hollow.every(Boolean);
      // collapsed: DOM content present but no mount laid out taller than ~0.
      // Gated on text-present so intentionally-thin textless components
      // (Divider 1-2px, Spacer) don't trip; those are allHollow's domain.
      const collapsed = maxHeight < 8 && texts.some((t) => t.trim());
      const thin = !err && (nameOnly || allHollow || collapsed);
      const bad = err || rootEmpty || errs || blank;
      results.push({ name, group, rel, errs, caught, firstErr: pageErrs[0] ?? firstCaught ?? err, pngBytes, blank, rootEmpty, thin, nameOnly, allHollow, collapsed, hasPlaceholder, nPlaceholder, maxHeight: Math.round(maxHeight), variantsIdentical, bad, texts });
      if (err) fail(`[RENDER] ${rel}: ${err}`);
      else if (rootEmpty) fail(`[RENDER] ${rel}: root empty`);
      else if (errs) warn(`[RENDER_ERRORS] ${rel}: ${pageErrs[0] ?? firstCaught} (${errs} total${caught ? `, ${caught} caught in-cell` : ''}) — set cfg.provider if these are context errors`);
      else if (blank) warn(`[RENDER_BLANK] ${rel}: renders but PNG is ${pngBytes}B (<5KB — likely blank, edit .design-sync/previews/${name}.tsx or add cfg.previewArgs.${name})`);
      else if (thin || variantsIdentical) warn(`[RENDER_THIN] ${rel}: ${variantsIdentical ? 'variants render identically' : nameOnly ? `mounted text is just "${name}"` : collapsed ? `DOM content present but rendered height is ${Math.round(maxHeight)}px` : 'mounts have no text and paint nothing'} — edit .design-sync/previews/${name}.tsx or add cfg.previewArgs.${name}`);
    }
    writeFileSync(join(OUT, '.render-check.json'), JSON.stringify(results, null, 2));
    const badOnes = results.filter((r) => r.bad);
    if (!badOnes.length) ok(`render check: ${results.length}/${results.length} previews render cleanly (screenshots in _screenshots/)`);
    else console.error(`  render check: ${results.length - badOnes.length}/${results.length} clean; ${badOnes.length} need attention (see .render-check.json, _screenshots/)`);
    // Contact sheets — tile every screenshot into labeled 4×4 grids so the
    // §4 review can sweep the whole set in a few image reads instead of
    // sampling. Best-effort and strictly additive: never fail()/warn(), never
    // changes the exit code, and only writes inside _screenshots/.
    try {
      // Make json presence = "this run completed all sheets": clear any prior
      // run's index up front so a mid-loop timeout below can't leave a stale
      // one for §4's sweep step to trust.
      rmSync(join(shotDir, 'contact-sheets.json'), { force: true });
      if (results.length) {
        const PER_SHEET = 16;
        const entries = [...results].sort((a, b) => a.name.localeCompare(b.name));
        const sheetCount = Math.ceil(entries.length / PER_SHEET);
        const statusOf = (r) => (r.rootEmpty ? '✗ empty' : r.blank ? '✗ blank' : r.bad ? '✗ error' : r.thin ? '⚠ thin' : r.variantsIdentical ? '⚠ variants identical' : '✓');
        const borderOf = (r) => (r.bad ? '#d33' : r.thin || r.variantsIdentical ? '#d90' : '#ddd');
        const index = [];
        let failedTiles = 0;
        await page.setViewportSize({ width: 1500, height: 900 });
        for (let s = 0; s < sheetCount; s++) {
          const slice = entries.slice(s * PER_SHEET, (s + 1) * PER_SHEET);
          const cells = slice.map((r) => {
            const shotName = `${r.group}__${r.name}.png`;
            let hasShot = false;
            try { hasShot = statSync(join(shotDir, shotName)).size > 0; } catch { hasShot = false; }
            const img = hasShot
              ? `<img src="./${shotName}" style="width:330px;height:300px;object-fit:cover;object-position:top left;display:block">`
              : `<div style="width:330px;height:300px;display:flex;align-items:center;justify-content:center;color:#999;font:14px system-ui">(no screenshot)</div>`;
            return `<div style="border:2px solid ${borderOf(r)};background:#fff;min-width:0">`
              + `<div style="font:600 18px system-ui;color:#222;padding:6px 8px;overflow-wrap:anywhere">${r.name} <span style="font-weight:400;color:#555">${statusOf(r)}</span></div>${img}</div>`;
          }).join('\n');
          const html = `<!doctype html><html><head><meta charset="utf-8"></head><body style="margin:0;background:#fff;width:1500px">`
            + `<div style="font:600 20px system-ui;color:#222;padding:12px 10px">render check — sheet ${s + 1}/${sheetCount} — components ${s * PER_SHEET + 1}–${s * PER_SHEET + slice.length} of ${entries.length} (alphabetical)</div>`
            + `<div style="display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;padding:0 10px 10px">${cells}</div></body></html>`;
          writeFileSync(join(shotDir, `.contact-sheet-${s + 1}.html`), html);
          await page.goto(`http://127.0.0.1:${port}/_screenshots/.contact-sheet-${s + 1}.html`, { waitUntil: 'networkidle', timeout: 15000 });
          await page.evaluate(() => Promise.all([...document.images].map((i) => i.decode().catch(() => {}))));
          // Tile fidelity: a broken/undecoded <img> must never silently stand
          // in for a real screenshot — swap it for an explicit label.
          failedTiles += await page.evaluate(() => {
            let n = 0;
            for (const img of [...document.images]) {
              if (img.complete && img.naturalWidth > 0) continue;
              const d = document.createElement('div');
              d.style.cssText = 'width:330px;height:300px;display:flex;align-items:center;justify-content:center;color:#c00;font:14px system-ui';
              d.textContent = '(screenshot failed to load)';
              img.replaceWith(d);
              n++;
            }
            return n;
          });
          await page.screenshot({ path: join(shotDir, `contact-sheet-${s + 1}.png`), fullPage: true });
          index.push({ sheet: s + 1, components: slice.map((r) => r.name) });
        }
        // Drop sheets a previous (larger) run left behind so the files on disk
        // always match contact-sheets.json.
        for (const f of readdirSync(shotDir)) {
          const m = /^\.?contact-sheet-(\d+)\.(?:png|html)$/.exec(f);
          if (m && Number(m[1]) > sheetCount) rmSync(join(shotDir, f), { force: true });
        }
        writeFileSync(join(shotDir, 'contact-sheets.json'), JSON.stringify(index, null, 2));
        console.error(`  contact sheets: ${sheetCount} sheet(s)${failedTiles ? `, ${failedTiles} tile(s) failed to load` : ''} → _screenshots/contact-sheet-*.png`);
      }
    } catch (e) {
      console.error(`  (contact sheets skipped — ${String(e).split('\n')[0]})`);
    }
  } catch (e) {
    console.error(`  (render check skipped — ${String(e).split('\n')[0]}; run \`npx playwright install chromium\`)`);
  } finally {
    await browser?.close();
    srv.close();
  }
}

const warnNote = warnings ? ` (${warnings} warning(s) — review above, non-blocking)` : '';
console.error(errors ? `\n${errors} error(s) — open a <Name>.html in a browser via \`npx serve ${OUT}\` to inspect.` : `\n✓ bundle is complete${warnNote}`);
process.exit(errors ? 1 : 0);
