#!/usr/bin/env node
// Storybook-shape orchestrator. Previews are iframe grids of the repo's own
// storybook-static/iframe.html — no story re-bundling. The SKILL.md flow has
// the user run `storybook build -o ds-bundle/_sb` directly; this script
// strips _sb/ in-place, bundles dist/ → window.<Global>, and emits
// .d.ts/.prompt.md/<Name>.html/README.
//
// Usage:
//   node storybook/build.mjs --config design-sync.config.json \
//     --node-modules ./node_modules --pkg-dir . --out ./ds-bundle

import { build as esbuild } from 'esbuild';
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, dirname, extname, isAbsolute, join, relative, resolve } from 'node:path';
import { bundleToIife, resolveDistEntry, stampHeader } from '../lib/bundle.mjs';
import { titleParts } from '../lib/common.mjs';
import { extractFonts } from '../lib/css.mjs';
import { discoverDocs, emitGuidelines, ingestDoc } from '../lib/docs.mjs';
import { exportedNames, findTypesRoot, loadDts, propsBodyFor } from '../lib/dts.mjs';
import { emitBuildMeta, emitReadme } from '../lib/emit.mjs';
import { emitIframeHtml } from './emit.mjs';
import { probe } from './probe.mjs';

// ── flags + config ───────────────────────────────────────────────────────
const argv = process.argv.slice(2);
const flag = (n, d) => { const i = argv.indexOf(`--${n}`); return i < 0 ? d : argv[i + 1]; };
const CONFIG_PATH = flag('config');
let cfg = {};
if (CONFIG_PATH) {
  try { cfg = JSON.parse(readFileSync(CONFIG_PATH, 'utf8')); }
  catch (e) { console.error(`[CONFIG] ${CONFIG_PATH}: ${e.message}`); process.exit(1); }
}
const NODE_MODULES = flag('node-modules') && resolve(flag('node-modules'));
const PKG = flag('pkg', cfg.pkg);
const OUT = resolve(flag('out') ?? '');
let GLOBAL = flag('global', cfg.globalName);
const LIMIT = Number(flag('limit', '0'));
const TITLE_MAP = cfg.titleMap ?? {};
const SB_STATIC = flag('storybook-static');
const SB = join(OUT, '_sb');
if (!NODE_MODULES || !PKG || !OUT) {
  console.error('required: --node-modules --pkg (or --config) --out');
  process.exit(1);
}
if (!existsSync(join(SB, 'iframe.html'))) {
  if (SB_STATIC && existsSync(join(SB_STATIC, 'iframe.html'))) {
    console.error(`  copying ${SB_STATIC}/ → _sb/`);
    cpSync(SB_STATIC, SB, { recursive: true, dereference: true });
  } else if (existsSync(join('storybook-static', 'iframe.html'))) {
    // `npm run build-storybook` default output dir.
    console.error(`[SB_COPIED] found ./storybook-static/ → copying to _sb/ (next time build directly with \`-o ${OUT}/_sb\` or pass --storybook-static)`);
    cpSync('storybook-static', SB, { recursive: true, dereference: true });
  }
}
if (!existsSync(join(SB, 'iframe.html')) || !existsSync(join(SB, 'index.json'))) {
  console.error(`[SB_MISSING] ${SB}/iframe.html or index.json not found. Run \`npx storybook build -o ${OUT}/_sb\`, or if you already built to ./storybook-static/, re-run with --storybook-static ./storybook-static.`);
  process.exit(1);
}

// PKG_DIR: --pkg-dir → node_modules/<pkg> → walk up for package.json name===PKG.
let PKG_DIR = flag('pkg-dir');
if (!PKG_DIR) {
  const inNm = join(NODE_MODULES, PKG);
  if (existsSync(join(inNm, 'package.json'))) PKG_DIR = inNm;
  else for (let d = dirname(resolve(NODE_MODULES)); d !== dirname(d); d = dirname(d)) {
    const pj = join(d, 'package.json');
    if (!existsSync(pj)) continue;
    try { if (JSON.parse(readFileSync(pj, 'utf8')).name === PKG) { PKG_DIR = d; break; } } catch {}
  }
}
if (!PKG_DIR || !existsSync(join(PKG_DIR, 'package.json'))) {
  console.error(`[CONFIG] can't find ${PKG}/package.json — pass --pkg-dir`);
  process.exit(1);
}
// dts.mjs's pkgDir walk-up stops at dirname(x)===x (fs root); with a
// relative PKG_DIR, dirname('.') === '.' and it short-circuits.
PKG_DIR = resolve(PKG_DIR);
const pkgJson = JSON.parse(readFileSync(join(PKG_DIR, 'package.json'), 'utf8'));
GLOBAL ||= PKG.replace(/^@/, '').replace(/[^A-Za-z0-9]+(.)?/g, (_, c) => (c ?? '').toUpperCase());
if (!/^[A-Za-z_$][\w$]*$/.test(GLOBAL)) {
  console.error(`[CONFIG] globalName must be a valid JS identifier, got ${JSON.stringify(GLOBAL)}`);
  process.exit(1);
}
console.error(`${PKG}@${pkgJson.version} → window.${GLOBAL}`);

const workspaceRoot = realpathSync(dirname(resolve(NODE_MODULES)));
function cfgPath(rel, field) {
  if (rel == null) return undefined;
  const p = resolve(PKG_DIR, rel);
  if (!existsSync(p)) { console.error(`  ! ${field}: ${rel} not found — skipped`); return undefined; }
  const r = relative(workspaceRoot, realpathSync(p));
  if (r.startsWith('..') || isAbsolute(r)) {
    console.error(`  ! ${field}: ${rel} resolves outside the workspace root — skipped`);
    return undefined;
  }
  return p;
}

for (const d of ['components', '_vendor', 'guidelines']) rmSync(join(OUT, d), { recursive: true, force: true });
mkdirSync(join(OUT, '_vendor'), { recursive: true });
writeFileSync(join(OUT, '.ds-bundle'), '');

// ── strip _sb/ in-place + inject storage shim ────────────────────────────
// sb-preview/ carries runtime.js that iframe.html loads — not droppable.
const SB_ADDON_RX = /\bsb-(?:addons?|common-assets|manager)\b/;
const DROP_EXT = new Set(['.map', '.txt', '.md', '.ts', '.mp4', '.webm', '.mov', '.avi', '.mkv', '.ogv']);
const KEEP_LARGE = new Set(['.woff', '.woff2', '.ttf', '.otf', '.js', '.mjs', '.css', '.json', '.html']);
let sbBytes = 0;
const css = [];
(function walk(d, rel) {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const p = join(d, e.name), r = rel ? `${rel}/${e.name}` : e.name;
    if (e.isDirectory()) {
      if (SB_ADDON_RX.test(e.name)) rmSync(p, { recursive: true, force: true });
      else walk(p, r);
      continue;
    }
    const ext = extname(e.name).toLowerCase();
    if (DROP_EXT.has(ext) || SB_ADDON_RX.test(e.name)) { rmSync(p, { force: true }); continue; }
    const sz = statSync(p).size;
    if (sz > 5 * 1024 * 1024 && !KEEP_LARGE.has(ext)) {
      console.error(`[SB_DROPPED] large: ${r} (${(sz / 1024 / 1024).toFixed(1)}MB)`);
      rmSync(p, { force: true });
      continue;
    }
    sbBytes += sz;
    if (ext === '.css') css.push({ r, txt: readFileSync(p, 'utf8') });
  }
})(SB, '');
const iframePath = join(SB, 'iframe.html');
const shim =
  '<script>try{localStorage.setItem("__ds","1");localStorage.removeItem("__ds")}catch(e){' +
  'var __m={};Object.defineProperty(window,"localStorage",{value:{getItem:function(k){return __m[k]||null},' +
  'setItem:function(k,v){__m[k]=String(v)},removeItem:function(k){delete __m[k]},clear:function(){__m={}},key:function(){return null},length:0}})}' +
  ';(function p(){if(parent!==window){var sid=new URLSearchParams(location.search).get("id");' +
  'parent.postMessage({kind:"ds-height",sid:sid,h:document.documentElement.scrollHeight},location.origin)}setTimeout(p,500)})();</script>';
const iframeHtml = readFileSync(iframePath, 'utf8');
if (!iframeHtml.includes('kind:"ds-height"')) {
  let patched = iframeHtml.replace('</head>', `${shim}</head>`);
  if (patched === iframeHtml) patched = iframeHtml.replace(/<body\b/i, `${shim}<body`);
  writeFileSync(iframePath, patched);
}
// url() rewrite: root-absolute → _sb/<u>, relative → _sb/<dir-of-sheet>/<u>
// (posix-normalized so `assets/./x.woff2` → `assets/x.woff2`). Fonts are
// additionally copied to a top-level fonts/ and pointed there — the design
// app's @font-face resolver whitelists fonts/, not _sb/.
const FONT_EXT = /\.(?:woff2?|ttf|otf|eot)(?:[?#].*)?$/i;
mkdirSync(join(OUT, 'fonts'), { recursive: true });
const fontSeen = new Set();
const sbCss = css.map(({ r, txt }) =>
  `/* ${r} */\n` + txt.replace(/url\((['"]?)(?!data:|https?:|\/\/)([^'")]+)\1\)/g,
    (_, q, u) => {
      const sbRel = (u.startsWith('/') ? u.slice(1) : `${dirname(r)}/${u}`)
        .split('/').reduce((a, s) => (s === '.' || s === '' ? a : s === '..' ? (a.pop(), a) : (a.push(s), a)), []).join('/');
      if (FONT_EXT.test(sbRel)) {
        const base = sbRel.split('/').pop().replace(/[?#].*$/, '');
        if (!fontSeen.has(base)) {
          const src = join(SB, sbRel.replace(/[?#].*$/, ''));
          if (existsSync(src)) { cpSync(src, join(OUT, 'fonts', base)); fontSeen.add(base); }
        }
        if (fontSeen.has(base)) return `url(${q}fonts/${base}${q})`;
      }
      return `url(${q}_sb/${sbRel}${q})`;
    }),
).join('\n\n');
console.error(`  _sb/: ${(sbBytes / 1024 / 1024).toFixed(1)} MB after strip; ${css.length} CSS sheet(s) collected; ${fontSeen.size} font(s) → fonts/`);
if (sbBytes > 50 * 1024 * 1024) console.error(`[SB_SIZE] _sb/ is ${(sbBytes / 1024 / 1024).toFixed(0)} MB — consider excluding dev/playground stories from the storybook config`);

// ── vendor react + bundle dist ───────────────────────────────────────────
// The host page (omelette) may already have window.React/ReactDOM set via
// its support.js — don't clobber. The IIFE lands on a temp name; the
// footer assigns to window.<Name> only if unset.
await esbuild({
  stdin: { contents: `module.exports=require('react');`, resolveDir: NODE_MODULES },
  bundle: true, format: 'iife', globalName: '__dsReact', outfile: join(OUT, '_vendor', 'react.js'),
  platform: 'browser', define: { 'process.env.NODE_ENV': '"development"' }, logLevel: 'error',
  footer: { js: 'window.React=window.React||__dsReact;' },
});
// react-dom merges main + /client (React 19's main has no createRoot). Shim
// react to window.React (reads at load time — either the host's or the one
// react.js just set). scheduler bundles into react-dom.js naturally.
await esbuild({
  stdin: {
    contents: `module.exports=Object.assign({},require('react-dom'),require('react-dom/client'));`,
    resolveDir: NODE_MODULES,
  },
  bundle: true, format: 'iife', globalName: '__dsReactDOM', outfile: join(OUT, '_vendor', 'react-dom.js'),
  platform: 'browser', define: { 'process.env.NODE_ENV': '"development"' }, logLevel: 'error',
  footer: { js: 'window.ReactDOM=window.ReactDOM||__dsReactDOM;' },
  plugins: [{
    name: 'react-to-window',
    setup(b) {
      b.onResolve({ filter: /^react$/ }, () => ({ path: 'r', namespace: 'rw' }));
      b.onLoad({ filter: /^r$/, namespace: 'rw' }, () => ({ contents: 'module.exports=window.React;', loader: 'js' }));
    },
  }],
});
const distEntry = resolveDistEntry({ pkgDir: PKG_DIR, pkgJson, pkgName: PKG, override: flag('entry') });
let bundleEntry = distEntry;
if (cfg.extraEntries?.length) {
  const mainAbs = JSON.stringify(resolve(distEntry));
  bundleEntry = join(OUT, '.bundle-entry.mjs');
  writeFileSync(bundleEntry,
    cfg.extraEntries.map((p) => `export * from ${JSON.stringify(p)};`).join('\n') + '\n' +
    `export * from ${mainAbs};\nexport * as __dsMainNs from ${mainAbs};\n`);
}
const { bundleJs, inlinedExternals } = await bundleToIife({
  entry: bundleEntry, globalName: GLOBAL, nodePaths: NODE_MODULES, out: OUT,
});
// _ds_bundle.css (esbuild's CSS sidecar from dist/) has the CSS-module
// class hashes that match what the agent renders via window.<Global>.
// Always emit — README says "link both", so no 404 when esbuild had none.
if (!existsSync(join(OUT, '_ds_bundle.css'))) {
  writeFileSync(join(OUT, '_ds_bundle.css'),
    '/* @ds-css-runtime: no extracted CSS — styles are runtime-generated */\n');
}
// cfg.extraFonts: explicit @font-face .css or bare font files for brand
// families that storybook-static doesn't itself ship (e.g. a <link> in
// preview-head.html). Same cfgPath/extractFonts helpers as package-build.
const extraFontRules = [];
for (const rel of cfg.extraFonts ?? []) {
  const p = cfgPath(rel, 'extraFonts');
  if (!p) continue;
  const pReal = realpathSync(p);
  if (/\.css$/i.test(p)) {
    extraFontRules.push(...extractFonts(pReal, dirname(pReal), { fontsOut: join(OUT, 'fonts'), roots: workspaceRoot }));
  } else if (/\.(woff2?|ttf|otf)$/i.test(p)) {
    cpSync(pReal, join(OUT, 'fonts', basename(p)));
    console.error(`  extraFonts: copied ${basename(p)} — add a matching @font-face .css to use it`);
  } else console.error(`  ! extraFonts: ${rel} isn't .css or a font file — skipped`);
}
writeFileSync(join(OUT, 'styles.css'),
  `@import './_ds_bundle.css';\n\n${sbCss || '/* storybook-static had no CSS */\n'}` +
  (extraFontRules.length ? `\n\n/* cfg.extraFonts */\n${extraFontRules.join('\n')}\n` : ''));

// Lightweight story-source extraction for .prompt.md ## Examples. Read the
// CSF file at importPath and pull each `export const <Name> = <initializer>`
// (brace/paren/bracket balanced, string-quote aware; stops at a top-level
// newline or ; with all delimiters closed). Doesn't strip comments — an
// unbalanced brace in `//` or `/* */` just drops that story.
const storySrcCache = new Map();
function storiesFromImportPath(ip) {
  if (!ip || storySrcCache.has(ip)) return storySrcCache.get(ip) ?? [];
  let src;
  // cfg.storybookConfigDir is relative to the config file (same base as
  // package-build.mjs and the SKILL.md config table).
  const cfgDir = CONFIG_PATH ? dirname(CONFIG_PATH) : workspaceRoot;
  const sbBase = cfg.storybookConfigDir && resolve(cfgDir, cfg.storybookConfigDir, '..');
  for (const base of [workspaceRoot, PKG_DIR, process.cwd(), sbBase].filter(Boolean)) {
    const p = resolve(base, ip.replace(/^\.\//, ''));
    if (existsSync(p)) { src = readFileSync(p, 'utf8'); break; }
  }
  if (!src) { storySrcCache.set(ip, null); return []; }
  const out = [];
  const rx = /export\s+const\s+([A-Z][A-Za-z0-9_]*)\s*(?::(?:[^=]|=>)+)?=\s*/g;
  let m;
  while ((m = rx.exec(src))) {
    const name = m[1];
    let i = rx.lastIndex, depth = 0, q = null;
    for (; i < src.length; i++) {
      const ch = src[i];
      if (q) { if (ch === q && src[i - 1] !== '\\') q = null; else if (ch === '\n' && q !== '`') q = null; continue; }
      if (ch === '"' || ch === "'" || ch === '`') { q = ch; continue; }
      if ('({['.includes(ch)) depth++;
      else if (')}]'.includes(ch)) depth--;
      else if (depth === 0 && (ch === ';' || ch === '\n')) break;
    }
    const value = src.slice(rx.lastIndex, i).trim();
    if (value && value.length < 2000) out.push({ name, value });
  }
  storySrcCache.set(ip, out);
  return out;
}

// ── components from index.json ───────────────────────────────────────────
const exported = exportedNames(PKG_DIR, pkgJson);
const index = JSON.parse(readFileSync(join(SB, 'index.json'), 'utf8'));
const byName = new Map();
let firstStoryId = null;
const unmapped = new Set();
for (const e of Object.values(index.entries ?? index.stories ?? {})) {
  if (e.type && e.type !== 'story') continue;
  // Precedence: explicit titleMap hit > importPath segment > title segment.
  // Nested titles like Controls/DropButton/Calendar/Simple mis-derive
  // name=Calendar from a DropButton demo; importPath is unambiguous there.
  // An explicit titleMap entry is the user's override and always wins.
  let { group, name } = titleParts(e.title, TITLE_MAP, exported);
  const viaMap = e.title.split('/').some((s) => s.replace(/\s+/g, '') in TITLE_MAP);
  if (!viaMap) {
    const ipSegs = (e.importPath ?? '').replace(/\.(stories|story)\.[jt]sx?$/i, '').split(/[\\/]+/);
    const ipName = [...ipSegs].reverse().find((s) => exported.has(s));
    if (ipName) name = ipName;
  }
  if (!exported.has(name)) { unmapped.add(e.title.split('/').pop()); continue; }
  // Probe story: first one that resolved to an exported component — not a
  // gallery/all/docs page whose layout (Box/Grid/…) would pollute the
  // fiber-walk provider chain.
  firstStoryId ??= e.id;
  const flatGroup = group.replace(/\//g, '-');
  if (!byName.has(name)) byName.set(name, { name, group: flatGroup, storyIds: [], title: e.title, importPaths: new Set() });
  const c = byName.get(name);
  c.storyIds.push({ id: e.id, name: e.name });
  if (e.importPath) c.importPaths.add(e.importPath);
}
let components = [...byName.values()];
if (LIMIT) components = components.slice(0, LIMIT);
if (!components.length) { console.error('[ZERO_MATCH] no story titles matched a package export'); process.exit(1); }
if (unmapped.size) console.error(`[TITLE_UNMAPPED] ${unmapped.size} title(s) didn't match an export: ${[...unmapped].slice(0, 10).join(', ')}${unmapped.size > 10 ? ', …' : ''} — add cfg.titleMap`);
console.error(`  components: ${components.length}`);

// ── probe: extract() argTypes + fiber-walk provider ──────────────────────
const { argTypesByName, provider } = await probe({ out: OUT, globalName: GLOBAL, firstStoryId, exportedNames: [...exported] });
let PROVIDER = cfg.provider ?? provider;
if (!cfg.provider && provider && CONFIG_PATH) {
  cfg.provider = provider;
  writeFileSync(CONFIG_PATH, JSON.stringify(cfg, null, 2) + '\n');
  console.error(`  wrote inferred provider → ${CONFIG_PATH}`);
}

// ── .d.ts / .prompt.md / .jsx / .html per component ──────────────────────
const typesRoot = findTypesRoot(PKG_DIR, pkgJson);
const dtsCtx = { ...loadDts(typesRoot), dtsPropsFor: cfg.dtsPropsFor };
discoverDocs({ components, PKG_DIR, cfg, cfgPath });
const escTbl = (s) => String(s ?? '').replace(/\n+/g, ' ').replace(/\|/g, '\\|');
for (const c of components) {
  const dir = join(OUT, 'components', c.group, c.name);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${c.name}.jsx`), `export const ${c.name} = window.${GLOBAL}.${c.name};\n`);
  const pb = propsBodyFor(c.name, dtsCtx);
  writeFileSync(join(dir, `${c.name}.d.ts`),
    `import * as React from 'react';\n\n${pb?.prelude ?? ''}` +
    `export interface ${c.name}Props${pb?.generics ?? ''}${pb?.extendsClause ?? ''} {\n` +
    `${pb?.body ?? '  [key: string]: unknown;'}\n}\n`);
  const at = argTypesByName[c.title?.split('/').pop()] ?? argTypesByName[c.name] ?? {};
  const rows = Object.entries(at).filter(([, v]) => v.description || v.type)
    .map(([k, v]) => `| \`${escTbl(k)}\` | \`${escTbl(v.type) || '—'}\` | ${escTbl(v.description)} |`);
  const propsTbl = rows.length ? `## Props\n\n| Prop | Type | Description |\n|---|---|---|\n${rows.join('\n')}\n\n` : '';
  const docMd = c.docPath ? ingestDoc(c.docPath)?.body : null;
  const head = docMd ? `${docMd}\n\n${propsTbl}` : `# ${c.name}\n\n\`\`\`tsx\nimport { ${c.name} } from '${PKG}';\n\`\`\`\n\n${propsTbl}`;
  const seen = new Set();
  const stories = [...(c.importPaths ?? [])].flatMap((ip) => storiesFromImportPath(ip))
    .filter((s) => !seen.has(s.name) && seen.add(s.name))
    .slice(0, 3);
  const examples = stories.length
    ? (/^#{1,2}\s+Examples\b/m.test(head) ? '' : '## Examples\n\n') +
      stories.map((s) => `### ${s.name}\n\n\`\`\`\`tsx\n${s.value.replace(/````+/g, '```')}\n\`\`\`\`\n`).join('\n')
    : '';
  writeFileSync(join(dir, `${c.name}.prompt.md`),
    `${c.name} from ${PKG}. Use via \`window.${GLOBAL}.${c.name}\` (loaded from /_ds_bundle.js).\n\n${head}${examples}`);
  emitIframeHtml({ c, out: OUT });
}
const ipAll = new Set(components.flatMap((c) => [...(c.importPaths ?? [])]));
const ipMisses = [...ipAll].filter((ip) => storySrcCache.get(ip) === null).length;
if (ipMisses) console.error(`[EXAMPLES] ${ipMisses}/${ipAll.size} importPath(s) didn't resolve under workspaceRoot/PKG_DIR/cwd — those components' .prompt.md have no ## Examples. Try running from the dir \`storybook build\` was invoked in.`);

emitGuidelines({ cfg, PKG_DIR, OUT, cfgPath, workspaceRoot });
emitReadme({
  OUT, GLOBAL, PKG, VERSION: pkgJson.version, components, tokenFiles: [],
  demoNames: [], hasProvider: !!PROVIDER, PROVIDER, jsdocFor: () => '',
});
emitBuildMeta({ OUT, GLOBAL, PKG, VERSION: pkgJson.version, PROVIDER, OVERRIDES: {}, components, shape: 'storybook', cfg });
stampHeader(bundleJs, { namespace: GLOBAL, components, inlinedExternals });
writeFileSync(join(OUT, '.stories.json'), JSON.stringify({
  stories: Object.fromEntries(components.map((c) => [c.name, c.storyIds[0]?.id])),
  provider: PROVIDER,
}));

console.error(`✓ ${components.length} components → ${OUT}`);
