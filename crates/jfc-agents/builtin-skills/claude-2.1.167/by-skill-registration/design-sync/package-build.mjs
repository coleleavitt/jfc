#!/usr/bin/env node
// Convert a React design system into the claude.ai/design DS-project layout.
// Two source shapes feed the same Source seam (see lib/source-*.mjs):
// storybook (.storybook/ + storybook-static) and package (dist + .d.ts,
// enriched from src/ when present). The output is identical regardless: root
// _ds_bundle.js (IIFE → window.<Namespace> with a first-line `/* @ds-bundle:
// {...} */` header), root styles.css, per-component .d.ts/.prompt.md/<Name>.html.
// The claude.ai/design app's self-check regenerates the adherence config and
// ds_manifest.
//
// lib/emit.mjs + lib/bundle.mjs are the app contract surface — agent never
// edits. Discovery (lib/source-*.mjs) is heuristic; each heuristic has a
// cfg override (grep ASSUMPTION) so non-matching repos write config, not code.
//
// Usage:
//   node package-build.mjs --config design-sync.config.json \
//     --node-modules ./node_modules \
//     --entry ./dist/index.js \
//     --storybook-static ./storybook-static \
//     --out ./ds-bundle

import {
  appendFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { basename, dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

// Repo-local script overrides: a repo can commit `.design-sync/lib/<name>.mjs`
// to fork a single adapter for its own quirks. Resolved relative to this
// script's own ./lib/ so cwd doesn't matter.
const BUNDLED_LIB = new URL('./lib/', import.meta.url);
const REPO_LIB = resolve('.design-sync', 'lib');
// Scanned up front (not accumulated via loadLib) so the [OVERRIDE*]
// cross-check below sees forks whose loadLib runs after it.
const forkedLibs = new Set(
  existsSync(REPO_LIB) ? readdirSync(REPO_LIB).filter((f) => f.endsWith('.mjs')) : [],
);
// `names` are tried in order: repo-local forks first (any name), then
// bundled. Lets a user `.design-sync/lib/<name>.mjs` override an adapter
// the bundled layout ships only as `<name>-<shape>.mjs`.
async function loadLib(...names) {
  for (const name of names) {
    if (forkedLibs.has(`${name}.mjs`)) {
      return import(pathToFileURL(join(REPO_LIB, `${name}.mjs`)).href);
    }
  }
  let lastErr;
  for (const name of names) {
    try { return await import(new URL(`${name}.mjs`, BUNDLED_LIB).href); }
    catch (e) { if (e?.code !== 'ERR_MODULE_NOT_FOUND') throw e; lastErr = e; }
  }
  throw lastErr;
}
const { bundleToIife, reactShim, resolveDistEntry, stampHeader, tsconfigPathsPlugin } = await loadLib('bundle');
const { copyTokens, extractFonts, writeStylesCss } = await loadLib('css');
const { exportedNames, findTypesRoot, isComponentName, jsdocFor, loadDts, partitionSubcomponents, propsBodyFor, smartDefaultProps } = await loadLib('dts');
const { emitBuildMeta, emitDemo, emitPerComponent, emitReadme, vendorReact } = await loadLib('emit');
const { buildPreviews, writePreviewFiles } = await loadLib('previews');
const { discoverDocs, emitGuidelines, ingestDoc } = await loadLib('docs');
const { detectShape } = await loadLib('detect');
const { resolvePackage } = await loadLib('source-kit');
const { bundlePreviewDecorators, resolveStorybook } = await loadLib('source-storybook');

// ── flags + config ───────────────────────────────────────────────────────
const argv = process.argv.slice(2);
function flag(name, dflt) {
  const i = argv.indexOf(`--${name}`);
  if (i < 0) return dflt;
  return argv[i + 1];
}
const CONFIG_PATH = flag('config');
let cfg = {};
if (CONFIG_PATH) {
  try { cfg = JSON.parse(readFileSync(CONFIG_PATH, 'utf8')); }
  catch (e) { console.error(`[CONFIG] ${CONFIG_PATH}: ${e.message}`); process.exit(1); }
}
// CLI flags override config values.
const NODE_MODULES = flag('node-modules') && resolve(flag('node-modules'));
const INPUTS = flag('inputs', NODE_MODULES ? dirname(NODE_MODULES) : '.');
const PKG = flag('pkg', cfg.pkg);
const TOKENS_PKG = flag('tokens-pkg', cfg.tokensPkg);
let GLOBAL = flag('global', cfg.globalName); // normalized to a valid id below, derived from pkg name if unset
const OUT = flag('out');
const LIMIT = Number(flag('limit', '0'));
const PROVIDER = cfg.provider ?? null; // {component, props, inner?}
const DEMO_SPEC = cfg.demo ?? [];
const TOKENS_GLOB = cfg.tokensGlob ?? null;
const STORIES_ROOT = flag('stories-root', INPUTS);
const SB_CONFIG_DIR = flag('storybook-config', null)
  ?? (cfg.storybookConfigDir ? resolve(dirname(CONFIG_PATH ?? '.'), cfg.storybookConfigDir) : null);
const SB_STATIC = flag('storybook-static', cfg.storybookStatic);
// (--components-dir dropped — package shape reads src/ directly; set cfg.srcDir to override)
let OVERRIDES = cfg.overrides ?? {};
if (typeof cfg.overrides === 'string') {
  try { OVERRIDES = JSON.parse(readFileSync(join(dirname(CONFIG_PATH), cfg.overrides), 'utf8')); }
  catch (e) { console.error(`[CONFIG] overrides ${cfg.overrides}: ${e.message}`); process.exit(1); }
}
const TITLE_MAP = cfg.titleMap ?? {};
// cfg.libOverrides declares which .design-sync/lib/ forks exist and why.
// Cross-check so an undocumented fork (or a declared-but-missing one) is loud.
const LIB_OVERRIDES = cfg.libOverrides ?? {};
for (const f of forkedLibs) {
  console.error(LIB_OVERRIDES[f]
    ? `[OVERRIDE] using .design-sync/lib/${f} — ${LIB_OVERRIDES[f]}`
    : `[OVERRIDE_UNDECLARED] .design-sync/lib/${f} is forked but not in cfg.libOverrides — add it with a one-line reason`);
}
for (const f of Object.keys(LIB_OVERRIDES)) {
  if (!forkedLibs.has(f)) console.error(`[OVERRIDE_MISSING] cfg.libOverrides declares "${f}" but .design-sync/lib/${f} doesn't exist`);
}

if (!NODE_MODULES || !PKG || !OUT) {
  console.error('required: --config (or --pkg) --node-modules --out');
  process.exit(1);
}

// Derive window.<Namespace> from a DS/package name — mirrors the
// claude.ai/design app's namespace derivation so a CLI-built bundle and an
// app-rebuilt one land on the same global. PascalCase the alnum runs; prefix
// `Ds` if it would start with a digit; fall back to `Ds`. globalName
// (config/--global) overrides the source string but is still normalized, so
// the header and the IIFE global agree.
function toNamespace(name) {
  const ns = String(name ?? '')
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((w) => w[0].toUpperCase() + w.slice(1))
    .join('');
  return !ns ? 'Ds' : /^[0-9]/.test(ns) ? 'Ds' + ns : ns;
}

// In the DS's own source repo, node_modules/<pkg> doesn't exist (npm won't
// self-install). --entry points at the built dist directly; we then walk up
// to find its package.json.
const ENTRY_OVERRIDE = flag('entry', cfg.entry);
let PKG_DIR;
if (ENTRY_OVERRIDE) {
  let d = dirname(resolve(ENTRY_OVERRIDE));
  while (!existsSync(join(d, 'package.json')) && d !== dirname(d)) d = dirname(d);
  PKG_DIR = d;
} else {
  PKG_DIR = join(NODE_MODULES, PKG);
}
const pkgJson = existsSync(join(PKG_DIR, 'package.json'))
  ? JSON.parse(readFileSync(join(PKG_DIR, 'package.json'), 'utf8'))
  : { name: PKG };
// VERSION goes into README.md which reaches the design agent — semver-only.
const VERSION = /^\d+\.\d+\.\d+[\w.+-]*$/.test(pkgJson.version ?? '') ? pkgJson.version : '0.0.0';
// Generic pkg names (e.g. "app") → prefer the DS dir's own name.
const GENERIC_PKG = new Set(['app', 'root', 'frontend', 'web', 'www', 'monorepo', '']);
const pkgNameForNs = GENERIC_PKG.has((pkgJson.name ?? '').toLowerCase()) ? basename(PKG_DIR) : pkgJson.name;
GLOBAL = toNamespace(GLOBAL || pkgNameForNs || PKG);
console.error(`» ${PKG}@${VERSION} → ${OUT} (window.${GLOBAL})`);

// ── reset out dir ────────────────────────────────────────────────────────
// Guard: refuse to rm -rf cwd, $HOME, /, or a non-empty dir that isn't a
// prior bundle (no _ds_bundle.js and no .ds-bundle marker). --out is user-supplied.
{
  const outAbs = resolve(OUT);
  const unsafe = [resolve('/'), resolve(process.env.HOME ?? '/nonexistent'), process.cwd()].includes(outAbs)
    || (existsSync(outAbs) && statSync(outAbs).isDirectory() && !existsSync(join(outAbs, '_ds_bundle.js')) && !existsSync(join(outAbs, '.ds-bundle')) && readdirSync(outAbs).length > 0)
    || (existsSync(outAbs) && !statSync(outAbs).isDirectory());
  if (unsafe) { console.error(`[OUT_UNSAFE] refusing to rm ${outAbs} — point --out at an empty dir or a prior bundle`); process.exit(1); }
}
rmSync(OUT, { recursive: true, force: true });
mkdirSync(join(OUT, '_vendor'), { recursive: true });
mkdirSync(join(OUT, 'components'), { recursive: true });
// Marker written early so a mid-run failure (which leaves OUT populated
// before _ds_bundle.js exists) doesn't trip [OUT_UNSAFE] on the next self-heal
// iteration. The guard above treats either file as "prior bundle output".
writeFileSync(join(OUT, '.ds-bundle'), '');
mkdirSync(join(OUT, 'tokens'), { recursive: true });
mkdirSync(join(OUT, 'guidelines'), { recursive: true });

// ── shape detect → adapter → Source ──────────────────────────────────────
await vendorReact({ nodeModules: NODE_MODULES, out: OUT });

const autodetected = detectShape({ INPUTS, SB_STATIC, SB_CONFIG_DIR });
const shape = cfg.shape ?? autodetected;
if (shape !== 'storybook' && shape !== 'package') {
  console.error(`[CONFIG] cfg.shape must be 'storybook' or 'package', got ${JSON.stringify(cfg.shape)}`);
  process.exit(1);
}
console.error(`  source shape: ${shape}${cfg.shape ? ' (from cfg.shape)' : ''}`);
if (cfg.shape && cfg.shape !== autodetected)
  console.error(`[CONFIG] cfg.shape=${cfg.shape} overrides auto-detected '${autodetected}'`);

const { generatePreviewSource } = await loadLib(`preview-gen-${shape}`);

// Storybook bundles the package's real dist entry; package shape resolves its
// own (dist if present, else synth from src/).
const distEntry =
  shape === 'storybook'
    ? resolveDistEntry({ pkgDir: PKG_DIR, pkgJson, override: ENTRY_OVERRIDE, pkgName: PKG })
    : null;
if (distEntry) console.error(`  entry: ${relative(NODE_MODULES, distEntry)}`);

// Compute the package's export set up front so the storybook adapter's
// titleParts can match 3-level titles (Category/Component/Story) against it.
const exportedSet = exportedNames(PKG_DIR, pkgJson);

const adapters = { storybook: resolveStorybook, package: resolvePackage };
const src = await adapters[shape]({
  INPUTS, STORIES_ROOT, SB_CONFIG_DIR, SB_STATIC, NODE_MODULES, OUT,
  PKG, PKG_DIR, pkgJson, ENTRY_OVERRIDE, entry: distEntry,
  titleMap: TITLE_MAP, exportedSet, cfg,
});

// Extra packages to merge into window.<GLOBAL> alongside the DS entry.
// Auto-detect icon sibling packages (same scope, name ends in /icons or
// /icons-react, installed) — otherwise icon components the design agent
// reaches for aren't on the global. cfg.extraEntries is the manual override.
// Match any dep whose name ends in `icons`/`icon`/`icons-react` AND whose
// scope either matches the DS scope OR squash-matches the DS name (covers
// unscoped DSes with scoped icon siblings, e.g. `<pkg>` → `@<pkg>/icons`).
const scope = PKG.startsWith('@') ? PKG.split('/')[0] : null;
const pkgSquash = PKG.replace(/^@/, '').replace(/[^a-z0-9]/gi, '').toLowerCase();
const depNames = Object.keys({ ...pkgJson.dependencies, ...pkgJson.peerDependencies });
const iconSiblings = depNames.filter((d) => {
  if (d === PKG || !/(?:^|[\/-])icons?(?:-react)?$/i.test(d)) return false;
  if (!existsSync(join(NODE_MODULES, d, 'package.json'))) return false;
  if (scope && d.startsWith(scope + '/')) return true;
  if (pkgSquash.length < 3) return false;  // too broad to squash-match safely
  const dScope = d.startsWith('@') ? d.split('/')[0] : d;
  return dScope.replace(/^@/, '').replace(/[^a-z0-9]/gi, '').toLowerCase().startsWith(pkgSquash);
});
const extraEntries = [...new Set([...(cfg.extraEntries ?? []), ...iconSiblings])];
let bundleEntry = src.entry;
if (extraEntries.length) {
  for (const p of iconSiblings) console.error(`  [ICON_PKG] auto-including sibling icon package ${p}`);
  // ESM drops ambiguous star re-exports to undefined, so an icon named `Tag`
  // would clobber the `Tag` component. Export main's full namespace as a
  // marker (`__dsMainNs`) and let bundleToIife's footer Object.assign it over
  // the IIFE global at runtime — types are already erased by then.
  const mainAbs = JSON.stringify(resolve(src.entry));
  bundleEntry = join(OUT, '.bundle-entry.mjs');
  writeFileSync(bundleEntry,
    extraEntries.map((p) => `export * from ${JSON.stringify(p)};`).join('\n') + '\n' +
    `export * from ${mainAbs};\n` +
    `export * as __dsMainNs from ${mainAbs};\n`);
}

// cfg.* path fields (cssEntry, tsconfig, extraFonts) come from
// design-sync.config.json, which is part of the synced repo and so
// untrusted when syncing a third-party DS. Each resolved path must land
// inside a fixed root: absolute paths, ../ escapes past the root, and
// symlinks pointing outside it are rejected rather than read/copied.
// workspaceRoot = dirname(NODE_MODULES) (not INPUTS — --inputs can point
// at a source subtree that doesn't contain PKG_DIR, whereas NODE_MODULES'
// parent is where npm/yarn/pnpm put the project root). realpath +
// path.relative so Windows case-insensitivity and symlink targets are
// handled by node. cssEntry is bounded to PKG_DIR (its content is
// uploaded verbatim, so a path anywhere under workspaceRoot would let a
// malicious dep's config exfiltrate project-root files); tsconfig and
// extraFonts are bounded to workspaceRoot (workspace-root tsconfig and
// sibling typography packages are legitimate, and their content is
// filtered rather than uploaded whole).
const workspaceRoot = realpathSync(dirname(NODE_MODULES));
const pkgRoot = realpathSync(PKG_DIR);
const outside = (real, root) => {
  const r = relative(root, real);
  return r.startsWith('..') || isAbsolute(r);
};
function cfgPath(rel, field, root) {
  if (rel == null) return undefined;
  const p = resolve(PKG_DIR, rel);
  if (!existsSync(p)) { console.error(`  ! ${field}: ${rel} not found — skipped`); return undefined; }
  if (outside(realpathSync(p), root)) {
    console.error(`  ! ${field}: ${rel} resolves outside ${root === pkgRoot ? 'the package' : 'the workspace root'} — skipped`);
    return undefined;
  }
  // Return the resolved path, not the realpath: downstream dirname-relative
  // resolution (tsconfig baseUrl, extractFonts srcDir) must match the
  // non-canonical paths the rest of the build uses, or e.g. `@/lib/utils`
  // aliases break on macOS where /var is a symlink to /private/var.
  return p;
}

// ── bundle → IIFE at window.<GLOBAL> ─────────────────────────────────────
const { bundleJs, bundleCss, inlinedExternals } = await bundleToIife({
  entry: bundleEntry,
  globalName: GLOBAL,
  nodePaths: NODE_MODULES,
  out: OUT,
  tsconfig: cfgPath(cfg.tsconfig, 'tsconfig', workspaceRoot),
});

// Auto-apply .storybook/preview decorators as the preview wrapper when no
// cfg.provider is set. Best-effort; cfg.provider remains the override.
let hasDecorators = false;
if (PROVIDER) console.error('  (decorator auto-detect skipped — cfg.provider is set)');
else if (!src.sbDir) console.error('  (decorator auto-detect skipped — no .storybook/ dir found)');
else hasDecorators = await bundlePreviewDecorators({ sbDir: src.sbDir, OUT, NODE_MODULES, PKG, PKG_DIR, GLOBAL });

// ── css / fonts / tokens ─────────────────────────────────────────────────
// Many DSes ship CSS as a separate import rather than
// importing it from the JS entry. cfg.cssEntry overrides; else the shape
// default; else common dist layouts.
let bundleCssSrcDir = PKG_DIR;
const explicitCss = cfgPath(flag('css', cfg.cssEntry), 'cssEntry', pkgRoot);
if (explicitCss && existsSync(bundleCss)) {
  // The esbuild bundle already emitted some CSS (often just an icon @font-face
  // that rode in via the JS module graph) — don't silently drop the explicitly
  // configured stylesheet on top of it; append it so the DS's real component
  // styles still ship in _ds_bundle.css.
  appendFileSync(bundleCss, `\n/* appended from cfg.cssEntry */\n${readFileSync(explicitCss, 'utf8')}`);
  bundleCssSrcDir = dirname(explicitCss);
  console.error(`  css: ${relative(INPUTS, explicitCss)} (${(statSync(explicitCss).size / 1024).toFixed(0)} KB, appended — bundle already had CSS)`);
} else if (!existsSync(bundleCss)) {
  // explicitCss (cfg.cssEntry/--css, contained); else src.cssEntry (shape
  // default, already absolute); else common dist layouts under PKG_DIR.
  const cand = explicitCss
    ? [explicitCss]
    : src.cssEntry
      ? [src.cssEntry]
      : ['build/esm/styles.css', 'dist/styles.css', 'dist/style.css', 'styles.css'].map((c) => join(PKG_DIR, c));
  for (const p of cand) {
    if (existsSync(p)) {
      cpSync(p, bundleCss);
      bundleCssSrcDir = dirname(p);
      console.error(`  css: ${relative(INPUTS, p)} (${(statSync(p).size / 1024).toFixed(0)} KB, copied)`);
      break;
    }
  }
}
let sbFallback = null, remoteStyleImports = [];
if (src.sbStatic) {
  const { fallbackCssFromStorybook, scrapeRemoteImports } = await loadLib('css-fallback');
  sbFallback = fallbackCssFromStorybook({ bundleCss, sbStatic: src.sbStatic, out: OUT });
  remoteStyleImports = scrapeRemoteImports(src.sbStatic);
}
if (sbFallback) bundleCssSrcDir = sbFallback;
// README says "link _ds_bundle.css" — always emit so that's never a 404.
// Marker lets package-validate.mjs report [CSS_RUNTIME] not [CSS_PLACEHOLDER].
if (!existsSync(bundleCss)) {
  writeFileSync(bundleCss,
    '/* @ds-css-runtime: no extracted CSS — styles are runtime-generated */\n');
}
// Containment roots for extractFonts: PKG_DIR always; sbStatic too when the
// fallback fired (fonts live under storybook-static/, not under the package).
const fontRoots = sbFallback ? [PKG_DIR, src.sbStatic] : [PKG_DIR];

const fontsOut = join(OUT, 'fonts');
const fontRules = [
  ...extractFonts(bundleCss, bundleCssSrcDir, { fontsOut, roots: fontRoots }),
  ...(explicitCss ? extractFonts(explicitCss, dirname(explicitCss), { fontsOut, roots: PKG_DIR }) : []),
];
// cfg.extraFonts: explicit paths (package-relative; may point outside the
// package, e.g. a sibling typography package) to @font-face .css files or
// bare font files for brand families the DS's CSS references but doesn't
// itself ship. CSS entries reuse extractFonts; url() refs resolve from the
// CSS file's directory and are copied when they land anywhere under
// workspaceRoot (a typography package's sibling fonts dir is a common
// layout). Containment: see cfgPath above.
for (const rel of cfg.extraFonts ?? []) {
  const p = cfgPath(rel, 'extraFonts', workspaceRoot);
  if (!p) continue;
  // extractFonts' startsWith roots-check isn't realpath-aware; workspaceRoot
  // is realpath'd, so srcDir must be too or macOS /var → /private/var
  // rejects every url().
  const pReal = realpathSync(p);
  if (/\.css$/i.test(p)) {
    fontRules.push(...extractFonts(pReal, dirname(pReal), { fontsOut, roots: workspaceRoot }));
  } else if (/\.(woff2?|ttf|otf)$/i.test(p)) {
    mkdirSync(fontsOut, { recursive: true });
    cpSync(pReal, join(fontsOut, basename(p)));
    console.error(`  extraFonts: copied ${basename(p)} — add a matching @font-face (e.g. an extraFonts .css) to use it`);
  } else {
    console.error(`  ! extraFonts: ${rel} isn't a .css or font file — skipped`);
  }
}
if (fontRules.length) {
  mkdirSync(fontsOut, { recursive: true });
  writeFileSync(join(fontsOut, 'fonts.css'), [...new Set(fontRules)].join('\n') + '\n');
  console.error(`  fonts: ${fontRules.length} @font-face rule(s) → fonts/`);
}

// ASSUMPTION: when cfg.tokensPkg is unset, a same-scope (or squash-matched, for
// unscoped DSes) dependency whose name contains "tokens" or "theme" is the
// tokens package. Override with cfg.tokensPkg.
let tokensPkg = TOKENS_PKG;
if (!tokensPkg) {
  const tokenSibling = depNames.find((d) => {
    if (d === PKG || !/(?:^|[\/-])(?:tokens?|themes?)(?:$|[\/-])/i.test(d)) return false;
    if (!existsSync(join(NODE_MODULES, d, 'package.json'))) return false;
    if (scope && d.startsWith(scope + '/')) return true;
    if (pkgSquash.length < 3) return false;
    const dScope = d.startsWith('@') ? d.split('/')[0] : d;
    return dScope.replace(/^@/, '').replace(/[^a-z0-9]/gi, '').toLowerCase().startsWith(pkgSquash);
  });
  if (tokenSibling) {
    tokensPkg = tokenSibling;
    console.error(`  [TOKENS_PKG] auto-detected sibling tokens package ${tokenSibling} (override with cfg.tokensPkg)`);
  }
}
let tokenFiles = copyTokens({ tokensPkg, tokensGlob: TOKENS_GLOB, nodeModules: NODE_MODULES, out: OUT });
// Adapter-supplied token CSS when no tokens-pkg given.
if (!tokenFiles.length && src.tokensCss?.length) {
  for (const p of src.tokensCss) {
    if (!existsSync(p)) continue;
    const name = basename(p);
    cpSync(p, join(OUT, 'tokens', name));
    tokenFiles.push(name);
  }
  if (tokenFiles.length) console.error(`  tokens: ${tokenFiles.length} file(s) from source shape default`);
}


// ── component list filtering (storybook: must be public exports) ─────────
const exported = src.exported ?? exportedSet;
// Synth-entry has no .d.ts — the entry IS the export list.
if (src.synthEntry) for (const c of src.components) exported.add(c.name);
// extraEntries exports are merged onto window.<GLOBAL>, so treat them as
// exported — unknownRefs and referencedExports both check against this set,
// so e.g. `<CheckIcon />` in a story renderSource isn't over-skipped.
for (const ep of extraEntries) {
  try {
    const dir = join(NODE_MODULES, ep);
    const pj = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
    for (const n of exportedNames(dir, pj)) exported.add(n);
  } catch { /* not installed or no .d.ts — dsShim still resolves it at runtime */ }
}
console.error(`  exported PascalCase symbols: ${exported.size}`);

// _adherence.oxlintrc.json rules: map raw HTML elements to the DS component
// that should replace them. One rule per raw element — the first name the DS
// actually exports wins. Weak-semantic elements (p/span/h1-h6) are excluded.
const REPLACES_BY_ELEMENT = {
  button: ['Button'],
  a: ['Link', 'Anchor'],
  input: ['TextField', 'TextInput', 'Input'],
  textarea: ['Textarea', 'TextArea'],
  select: ['Select', 'Picker', 'Dropdown'],
  'input[type=checkbox]': ['Checkbox'],
  'input[type=radio]': ['RadioButton', 'Radio'],
  'input[type=range]': ['Slider'],
  img: ['Image'],
  ul: ['List'],
  form: ['Form'],
  table: ['Table', 'DataTable'],
  dialog: ['Modal', 'Dialog'],
  ...(cfg.replaces ?? {}),
};
const REPLACES = {};
for (const [el, names] of Object.entries(REPLACES_BY_ELEMENT)) {
  const n = (Array.isArray(names) ? names : [names]).find((c) => exported.has(c));
  if (n) REPLACES[n] = el;
}

if (!src.components.length && !src.tokensOnly) {
  console.error(`[ZERO_MATCH] ${shape === 'storybook' ? 'no story-type entries in storybook-static/index.json (only docs, or empty) — check the storybook config stories glob' : 'no components discovered'}.`);
  process.exit(1);
}
let components = src.shape === 'storybook'
  ? src.components.filter((c) => exported.has(c.name))
  : src.components;
if (src.shape === 'storybook') {
  const unmapped = src.components.filter((c) => !exported.has(c.name)).map((c) => c.name);
  if (unmapped.length) {
    console.error(
      `[TITLE_UNMAPPED] ${unmapped.length} storybook title(s) don't match a package export — dropped: ` +
        `${unmapped.slice(0, 10).join(', ')}${unmapped.length > 10 ? ', …' : ''}. ` +
        `Add cfg.titleMap {<title-name>: <export-name>} if these are real components under different names.`,
    );
  }
  console.error(`  ${components.length}/${src.components.length} storybook components are public exports`);
}
// Dedup by name + sort.
const seen = new Set();
components = components.filter((c) => !seen.has(c.name) && seen.add(c.name));
components.sort((a, b) => a.name.localeCompare(b.name));
if (LIMIT > 0) components = components.slice(0, LIMIT);
console.error(`  components: ${components.length}`);

// ── per-component types from shipped .d.ts ───────────────────────────────
const dts = loadDts(findTypesRoot(PKG_DIR, pkgJson));
for (const n of dts.nonComponents) exported.delete(n);
{
  const before = components.length;
  components = components.filter((c) => !dts.nonComponents.has(c.name) && isComponentName(c.name));
  console.error(
    `  (excluded ${before - components.length} enum/type/context/hook exports; ${components.length} components)`,
  );
}
// Subcomponents (TableRow when Table exists) don't get a standalone preview
// — they typically need the parent to render. Still in `exported` (importable)
// and listed under the parent. cfg.componentSrcMap pins (non-null) force a
// name to be treated as a root.
{
  const pinned = new Set(Object.entries(cfg.componentSrcMap ?? {}).filter(([, v]) => v !== null).map(([k]) => k));
  const { parentOf, hints } = partitionSubcomponents(components.map((c) => c.name), dts.compounds);
  for (const k of pinned) { parentOf.delete(k); hints.delete(k); }
  // Tier-2 hints → attach parentHint (generatePreviewSource emits an extra
  // InParent cell; never skips).
  for (const c of components) if (hints.has(c.name)) c.parentHint = hints.get(c.name);
  if (parentOf.size) {
    const byParent = new Map();
    for (const [sub, parent] of parentOf) (byParent.get(parent) ?? byParent.set(parent, []).get(parent)).push(sub);
    for (const c of components) if (byParent.has(c.name)) c.subcomponents = byParent.get(c.name).sort();
    components = components.filter((c) => !parentOf.has(c.name));
    const sample = [...byParent].slice(0, 3).map(([p, s]) => `${p}←${s.slice(0, 3).join(',')}${s.length > 3 ? ',…' : ''}`).join('; ');
    console.error(`  (grouped ${parentOf.size} subcomponents under ${byParent.size} parents; ${components.length} roots: ${sample}${byParent.size > 3 ? '; …' : ''})`);
  }
  if (hints.size) console.error(`  (${hints.size} components have a parentHint annotation)`);
}

// ── per-component docs + guidelines ──────────────────────────────────────
// Probe for a doc file per component (sibling .md → docsDir → stories.mdx, with
// cfg.docsMap overrides). Ingest the matched ones; frontmatter `category` sets
// c.group when the component doesn't already have a non-generic one. cfg paths
// (docsDir / docsMap / guidelinesGlob) route through the same cfgPath/outside
// validation as tsconfig/cssEntry/extraFonts above, bounded to workspaceRoot.
// Runs AFTER the .d.ts non-component filter so the docs:N/M count and
// [DOCS_UNMAPPED] lines reflect the components actually emitted.
const wsCfgPath = (rel, field) => cfgPath(rel, field, workspaceRoot);
emitGuidelines({ cfg, PKG_DIR, OUT, cfgPath: wsCfgPath, workspaceRoot });
discoverDocs({ components, PKG_DIR, cfg, cfgPath: wsCfgPath });
for (const c of components) {
  if (!c.docPath) continue;
  const d = ingestDoc(c.docPath);
  c.docBody = d.body;
  c.docKeywords = d.keywords;
  if (d.category && (!c.group || c.group === 'general' || c.group === 'misc')) {
    c.group = d.category.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || c.group;
  }
}

// ── .design-sync/previews/ — per-component JSX preview files ────────────
// Auto-generated each run while the marker line is present; user deletes the
// marker to take ownership. Compiled to OUT/_preview/<Name>.js for the html
// to load; build failures fall back to the existing scaffold/story-grid path.
const previewDir = resolve('.design-sync', 'previews');
writePreviewFiles({
  components, previewDir,
  gen: (c) => generatePreviewSource(c, {
    smart: smartDefaultProps(c.name, propsBodyFor(c.name, { ...dts, dtsPropsFor: cfg.dtsPropsFor })),
    members: dts.compounds.get(c.name),
    exported, pkg: PKG,
    skip: OVERRIDES[c.name]?.skip,
    previewArgs: cfg.previewArgs?.[c.name],
  }),
});
const tsconfigForPreviews = cfgPath(cfg.tsconfig, 'tsconfig', workspaceRoot);
const builtPreviews = await buildPreviews({
  components, previewDir, OUT, GLOBAL, PKG, reactShim, NODE_MODULES, extraEntries,
  pathsPlugin: tsconfigForPreviews ? tsconfigPathsPlugin(tsconfigForPreviews) : null,
});

// ── emit ─────────────────────────────────────────────────────────────────
emitPerComponent({
  src, components, OUT, GLOBAL, PKG, VERSION, OVERRIDES, REPLACES, PROVIDER, hasDecorators, exported, builtPreviews,
  propsBodyFor: (n) => propsBodyFor(n, { ...dts, dtsPropsFor: cfg.dtsPropsFor }),
  compoundsFor: (n) => dts.compounds.get(n),
  smartDefaultProps,
  previewArgs: cfg.previewArgs,
});

const demoNames = [...new Set(DEMO_SPEC.flat(Infinity).filter((x) => typeof x === 'string' && exported.has(x)))];
emitDemo({ DEMO_SPEC, demoWrap: cfg.demoWrap, exported, OUT, GLOBAL, PROVIDER });

writeStylesCss({ out: OUT, tokenFiles, bundleCss, fontRules, remoteImports: remoteStyleImports });

stampHeader(bundleJs, { namespace: GLOBAL, components, inlinedExternals });

emitReadme({
  OUT, GLOBAL, PKG, VERSION, TOKENS_PKG, components, tokenFiles, demoNames,
  hasProvider: PROVIDER && exported.has(PROVIDER.component.split('.')[0]),
  PROVIDER,
  jsdocFor: (n) => jsdocFor(n, dts),
});

const count = emitBuildMeta({ OUT, GLOBAL, PKG, VERSION, PROVIDER, OVERRIDES, components, shape: src.shape, cfg });

console.error(`✓ wrote ${OUT}: _ds_bundle.js + styles.css + ${count} component previews`);
