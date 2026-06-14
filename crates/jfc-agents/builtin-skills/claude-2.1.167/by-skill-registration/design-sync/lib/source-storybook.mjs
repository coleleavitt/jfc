// Storybook source adapter. Builds (or copies) storybook-static, parses
// index.json into the component list, and runs node-side composeStories to
// extract story args for .prompt.md examples. Returns the Source shape the
// orchestrator and emitter consume.

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { titleParts } from './common.mjs';
import { findStorybookDirs } from './detect.mjs';
import { extractRenderSource, importStorybookReact, installBrowserStubs, nodeTranspile } from './stories.mjs';
import { argsFromArgTypes, extractFromBuiltChunks } from './stories-static.mjs';

function pickStorybookDir({ STORIES_ROOT, INPUTS, PKG, SB_CONFIG_DIR }) {
  if (SB_CONFIG_DIR) return SB_CONFIG_DIR;
  // Many repos name the config dir via `storybook dev -c <dir>` in
  // package.json scripts — that's authoritative when present.
  try {
    const scripts = JSON.parse(readFileSync(join(STORIES_ROOT, 'package.json'), 'utf8')).scripts ?? {};
    for (const s of Object.values(scripts)) {
      const m = typeof s === 'string' && s.match(/\bstorybook\s+(?:dev|build)\b[^;&|]*?(?:-c|--config-dir)[= ]+(\S+)/);
      if (m) return resolve(STORIES_ROOT, m[1]);
    }
  } catch {}
  const found = findStorybookDirs(STORIES_ROOT);
  if (found.length > 1) {
    const pkgTail = PKG.split('/').pop();
    const ranked = found
      .map((d) => {
        const sib = join(dirname(d), 'package.json');
        let name = '';
        try { name = JSON.parse(readFileSync(sib, 'utf8')).name ?? ''; } catch {}
        return { d, score: name === PKG ? 2 : d.includes(pkgTail) ? 1 : 0, depth: d.split(sep).length };
      })
      .sort((a, b) => b.score - a.score || a.depth - b.depth);
    console.error(
      `[MULTI_STORYBOOK] ${found.length} .storybook/ dirs under --stories-root; picked ${ranked[0].d}. ` +
        `Override with --storybook-config <dir> if wrong. Found: ${found.join(', ')}`,
    );
    return ranked[0].d;
  }
  return found[0] ?? [join(STORIES_ROOT, '.storybook'), join(INPUTS, '.storybook')].find(existsSync);
}

// Node-side args extraction for .prompt.md — transpile each story file with
// packages:external (node's resolver dedupes), import in this process,
// composeStories, read .args. We never render in node, so context dual-
// instance doesn't matter. Best-effort: a story that throws just doesn't
// contribute example args.
async function extractArgs({ sbDir, sbStatic, csfComponents, nodeModules, skip }) {
  const argsById = new Map();
  const sbPreview = ['tsx', 'ts', 'jsx', 'js'].map((e) => join(sbDir, `preview.${e}`)).find(existsSync);
  const tmpDir = join(sbStatic, '.node-extract');
  mkdirSync(tmpDir, { recursive: true });
  installBrowserStubs();
  // A story module that throws from a deferred callback (setTimeout/
  // microtask at import time) must not take down the converter.
  const swallow = (e) => console.error(`  ! node-side args (async): ${String(e).split('\n')[0]}`);
  process.on('uncaughtException', swallow);
  process.on('unhandledRejection', swallow);
  const sb = await importStorybookReact({ nodeModules, sbDir });
  let composeStories = sb?.composeStories, setProjectAnnotations = sb?.setProjectAnnotations;
  try {
    if (sbPreview && setProjectAnnotations) {
      const pv = join(tmpDir, 'preview.mjs');
      await nodeTranspile(sbPreview, pv);
      const pmod = await import(pathToFileURL(pv).href);
      setProjectAnnotations(pmod.default ?? pmod);
    }
  } catch (e) {
    console.error(`  ! node-side setup: ${String(e).split('\n')[0]} — .prompt.md will have story names only`);
  }
  if (composeStories) {
    const shared = {};
    for (const ip of new Set(csfComponents.flatMap((c) => [...c.importPaths]))) {
      const abs = resolve(dirname(sbDir), ip);
      if (!existsSync(abs)) continue;
      const out = join(tmpDir, createHash('sha256').update(ip).digest('hex').slice(0, 12) + '.mjs');
      try {
        await nodeTranspile(abs, out, shared);
        const mod = await import(pathToFileURL(out).href);
        if (!mod.default) continue;
        const raw = readFileSync(abs, 'utf8');
        const compName = mod.default?.component?.displayName ?? mod.default?.component?.name
          ?? mod.default?.title?.split('/').pop();
        for (const [exportName, S] of Object.entries(composeStories(mod))) {
          if (skip?.has(S.id)) continue;
          let args = S.args;
          // Manual argTypes (rare, but free when present).
          if ((!args || !Object.keys(args).length) && S.argTypes) {
            args = { ...argsFromArgTypes(S.argTypes), ...(args ?? {}) };
          }
          const hasArgs = args && Object.keys(args).length > 0;
          // addon-docs injects originalSource at build time; node-side this is
          // usually absent, but cheap to prefer when it's there.
          const sbSource = S.parameters?.docs?.source?.originalSource;
          const renderSource = !hasArgs && compName
            ? (typeof sbSource === 'string' ? sbSource : null)
              ?? extractRenderSource(raw, exportName, compName)
            : null;
          argsById.set(S.id, { args, storyName: S.storyName, renderSource });
        }
      } catch (e) {
        // Best-effort — a story file that throws at import just doesn't
        // contribute example args. Log so the aggregate miss count is
        // explainable. esbuild wraps the real message under e.errors[0].text.
        const msg = e?.errors?.[0]?.text ?? e?.message ?? String(e);
        console.error(`  ! story import failed: ${ip}: ${String(msg).split('\n')[0]}`);
      }
    }
    console.error(`  node-side args: ${argsById.size} stories extracted${argsById.size === 0 ? ' — all previews will fall through to the .d.ts scaffold' : ''}`);
  }
  // Let deferred callbacks fire while swallow is still installed, then
  // restore the default handlers so later real errors aren't masked.
  await new Promise((r) => setTimeout(r, 250));
  process.off('uncaughtException', swallow);
  process.off('unhandledRejection', swallow);
  try { rmSync(tmpDir, { recursive: true, force: true }); } catch {}
  return argsById;
}

export async function resolveStorybook(ctx) {
  const { INPUTS, STORIES_ROOT, SB_CONFIG_DIR, SB_STATIC, PKG, PKG_DIR, OUT, entry, titleMap, exportedSet } = ctx;
  const sbDir = pickStorybookDir({ STORIES_ROOT, INPUTS, PKG, SB_CONFIG_DIR });
  let sbStatic = SB_STATIC ? resolve(SB_STATIC) : null;
  if (sbStatic && !existsSync(join(sbStatic, 'index.json'))) {
    console.error(`--storybook-static ${sbStatic} has no index.json`);
    sbStatic = null;
  }
  // storybook-static is parsed for index.json + story-args extraction and the
  // CSS fallback, then discarded — previews render self-contained from the
  // bundle. Built into a dot-prefixed dir so it's never uploaded.
  if (!sbStatic && sbDir) {
    sbStatic = resolve(OUT, '.sb-static');
    console.error(`  running: npx storybook build -c ${sbDir} -o ${sbStatic}`);
    const { spawnSync } = await import('node:child_process');
    const r = spawnSync(
      'npx', ['storybook', 'build', '-c', sbDir, '-o', sbStatic, '--quiet'],
      { cwd: dirname(sbDir), stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, timeout: 600_000, shell: process.platform === 'win32' },
    );
    if (r.error || r.signal || r.status !== 0 || !existsSync(join(sbStatic, 'index.json'))) {
      console.error(`[SB_BUILD_FAIL] storybook build exited ${r.status ?? r.signal ?? r.error?.code}:\n${(r.stderr || r.stdout || '').slice(-2000)}`);
      sbStatic = null;
    }
  }
  const csfComponents = [];
  let argsById = new Map();
  if (sbStatic) {
    const idx = JSON.parse(readFileSync(join(sbStatic, 'index.json'), 'utf8'));
    // Multi-package Storybooks can have a 'TextField' from each sibling
    // package. Prefer stories whose importPath is under the target
    // package's own directory.
    const sbRoot = sbDir ? resolve(dirname(sbDir)) : null;
    const isOwn = (e) =>
      !!sbRoot && !!e.importPath && resolve(sbRoot, e.importPath).startsWith(resolve(PKG_DIR) + sep);
    const idxEntries = Object.values(idx.entries ?? {}).sort((a, b) => isOwn(b) - isOwn(a));
    const byComp = new Map();
    for (const e of idxEntries) {
      if (e.type === 'docs') continue;
      // Skip stories the DS marks deprecated/hidden so v1-API stories don't
      // render the v2 export with wrong props.
      if ((e.tags ?? []).includes('!dev') || (e.tags ?? []).includes('deprecated')) continue;
      if (/deprecated/i.test(e.importPath ?? '')) continue;
      const { name: compName, group } = titleParts(e.title, titleMap, exportedSet);
      if (!byComp.has(compName)) byComp.set(compName, { name: compName, group, own: isOwn(e), storyIds: [], importPaths: new Set() });
      const comp = byComp.get(compName);
      if (comp.own && !isOwn(e)) continue; // own-package stories win the name
      comp.storyIds.push({ id: e.id, name: e.name });
      if (e.importPath) comp.importPaths.add(e.importPath);
    }
    for (const c of byComp.values()) csfComponents.push(c);
    console.error(
      `  storybook-static: ${Object.keys(idx.entries ?? {}).length} entries → ${csfComponents.length} components`,
    );
    // Built chunks first — storybook's own build has args/argTypes composed
    // onto the story objects. Whatever that fills, the node-side transpile
    // path below skips.
    const tmpDir = join(sbStatic, '.node-extract');
    mkdirSync(tmpDir, { recursive: true });
    argsById = await extractFromBuiltChunks(sbStatic, csfComponents, tmpDir);
    if (sbDir) {
      const fallback = await extractArgs({ sbDir, sbStatic, csfComponents, nodeModules: ctx.NODE_MODULES, skip: argsById });
      for (const [k, v] of fallback) if (!argsById.has(k)) argsById.set(k, v);
    }
  } else {
    console.error(`[SB_BUILD_FAIL] no storybook-static and no .storybook/ dir found — pass --storybook-static <dir> or run from a repo with .storybook/.`);
  }
  for (const c of csfComponents) c.argsById = argsById;
  return { shape: 'storybook', entry, components: csfComponents, sbStatic, sbDir };
}

// Bundle .storybook/preview.{tsx,ts,jsx,js} decorators into
// _vendor/preview-decorators.js so each preview can wrap its mount in the same
// provider chain Storybook does. Best-effort: bail (return false) if there's
// no decorator array or the bundle fails — cfg.provider remains the manual
// fallback. Imports of the DS package itself are shimmed to window.<GLOBAL>
// so the decorator's <AppProvider> is the same instance the previews use.
export async function bundlePreviewDecorators({ sbDir, OUT, NODE_MODULES, PKG, PKG_DIR, GLOBAL }) {
  if (!sbDir) return false;
  const sbPreview = ['tsx', 'ts', 'jsx', 'js'].map((e) => join(sbDir, `preview.${e}`)).find(existsSync);
  if (!sbPreview || !/\bdecorators\s*[:=]/.test(readFileSync(sbPreview, 'utf8'))) return false;
  const { build } = await import('esbuild');
  const entry = join(OUT, '.preview-decorators-entry.mjs');
  // The decorator receives (Story, ctx). We pass a Story fn that returns the
  // already-built inner element and an empty-shaped ctx; decorators that read
  // ctx.globals/args just get undefined-y defaults.
  writeFileSync(entry, `import * as pv from ${JSON.stringify(sbPreview)};
var ds = ((pv.default && pv.default.decorators) || pv.decorators || []).filter(function(d){return typeof d==="function"});
var ctx = {args:{},argTypes:{},globals:{},parameters:{},viewMode:"story",loaded:{}};
// reduce (not reduceRight): Storybook composes first-in-array = innermost.
// The chain runs inside a rendered component so decorator hooks have a
// dispatcher — calling decorators eagerly (outside render) would null it.
window.__dsDecorate = !ds.length ? null : function(el){
  return window.React.createElement(function(){
    return ds.reduce(function(inner,d){return d(function(){return inner},ctx)}, el);
  });
};`);
  // Shim the DS package (by name, or by a relative path that resolves under
  // PKG_DIR — e.g. `../src` from .storybook/) to window.<GLOBAL> so we don't
  // re-bundle the whole DS and the provider's Context matches the bundle's.
  const pkgRoot = resolve(PKG_DIR);
  const dsShim = {
    name: 'ds-global',
    setup(b) {
      const escPkg = PKG.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      // Exact match only — subpath imports (<pkg>/locales/en.json) must bundle
      // normally, not shim to a nonexistent window.<GLOBAL>.<subpath>.
      b.onResolve({ filter: new RegExp(`^${escPkg}$`) }, () => ({ path: 'ds', namespace: 'ds-shim' }));
      b.onResolve({ filter: /^\.\.?\// }, (a) => {
        const abs = resolve(a.resolveDir, a.path);
        if (abs === pkgRoot || abs === join(pkgRoot, 'src') || abs === join(pkgRoot, 'src', 'index')) {
          return { path: 'ds', namespace: 'ds-shim' };
        }
        return undefined;
      });
      b.onLoad({ filter: /^ds$/, namespace: 'ds-shim' }, () => ({
        contents: `module.exports=window.${GLOBAL};`, loader: 'js',
      }));
    },
  };
  // Storybook-runtime/addon/msw packages are preview-time only. Stub to {}
  // instead of externalizing — `external` in IIFE output leaves a bare
  // require() that throws in-browser, so the bundle fails to evaluate.
  const stubEmpty = {
    name: 'sb-stub',
    setup(b) {
      b.onResolve({ filter: /^(@storybook\/|storybook(\/|$)|msw(\/|$)|@mswjs\/)/ }, (a) => ({ path: a.path, namespace: 'stub' }));
      b.onLoad({ filter: /.*/, namespace: 'stub' }, () => ({ contents: 'module.exports={}', loader: 'js' }));
    },
  };
  // React shim for the decorator bundle: read window.React/ReactDOM at USE
  // time (getters), not via `var R = window.React` at thunk-define time —
  // esbuild can hoist the CJS thunk call before the page global is live.
  const reactGlobal = {
    name: 'react-global',
    setup(b) {
      // Catch every subpath (react/jsx-runtime, react-dom/client,
      // react-dom/server, …) so a transitive package's own `import React`
      // can't bundle a second copy alongside the page's window.React.
      b.onResolve({ filter: /^react(-dom)?($|\/)/ }, (a) =>
        ({ path: a.path.startsWith('react-dom') ? 'rd' : 'r', namespace: 'rg' }));
      // ownKeys + getOwnPropertyDescriptor so esbuild's __toESM/__copyProps
      // (which enumerate via getOwnPropertyNames) see every React export —
      // otherwise `import {useState} from 'react'` is undefined.
      const proxy = (g, extra) => `new Proxy(${extra},{
  get:function(o,k){return k in o?o[k]:(${g}||{})[k]},
  ownKeys:function(o){return Array.from(new Set(Object.keys(o).concat(Object.keys(${g}||{}))))},
  getOwnPropertyDescriptor:function(o,k){return{enumerable:true,configurable:true,get:function(){return k in o?o[k]:(${g}||{})[k]}}}
})`;
      b.onLoad({ filter: /^r$/, namespace: 'rg' }, () => ({
        loader: 'js',
        contents: `function jsx(t,p,k){return window.React.createElement(t,k===void 0?p:Object.assign({key:k},p))}
module.exports=${proxy('window.React', '{jsx:jsx,jsxs:jsx,jsxDEV:jsx,Fragment:undefined}')};`,
      }));
      b.onLoad({ filter: /^rd$/, namespace: 'rg' }, () => ({
        loader: 'js',
        contents: `module.exports=${proxy('window.ReactDOM', '{}')};`,
      }));
    },
  };
  try {
    await build({
      entryPoints: [entry], outfile: join(OUT, '_vendor', 'preview-decorators.js'),
      bundle: true, format: 'iife', platform: 'browser', target: 'es2020',
      jsx: 'automatic', loader: { '.js': 'jsx', '.json': 'json' },
      nodePaths: [NODE_MODULES], plugins: [reactGlobal, dsShim, stubEmpty],
      logLevel: 'silent',
    });
    console.error(`  preview-decorators.js: bundled from ${relative(pkgRoot, sbPreview)}`);
    return true;
  } catch (e) {
    console.error(`  ! preview decorator bundle failed: ${String(e).split('\n')[0]} — set cfg.provider manually`);
    return false;
  } finally {
    rmSync(entry, { force: true });
  }
}
