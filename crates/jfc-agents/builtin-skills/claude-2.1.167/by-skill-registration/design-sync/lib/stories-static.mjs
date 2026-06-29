// Storybook-static built-chunk extraction — reads storybook-static/assets/
// *.stories-*.js (already fully resolved by the storybook build) to pull
// args/argTypes without a source transpile.

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { ls } from './common.mjs';
import { installBrowserStubs } from './stories.mjs';

// ── Built-chunk extraction (storybook-static/assets/*.stories-*.js) ────────
// Storybook's build has already resolved everything the Node-side transpile
// below fights against: deps bundled, docgen applied, args/argTypes composed
// onto the exported story objects. We rewrite the chunk's relative imports to
// a self-returning stub and import it — args/argTypes/parameters are plain
// data literals that survive; only `.component`/`.render` see the stub.
const STUB_DEF =
  // Vite emits lazy-init thunks `helper((()=>{...C=...}))` — call 0-arity fn
  // args so the exported identifiers are assigned. `then→undefined` so a
  // top-level `await stub` doesn't deadlock.
  'var __s=new Proxy(function(a){' +
  'if(typeof a==="function"&&a.length===0)try{a()}catch(e){}return __s},{' +
  'get:function(t,k){return k==="then"?void 0:__s},' +
  'set:function(){return true}});' +
  // Storybook-injected runtime globals some chunks reference bare.
  'for(var __g of["ACTIONS","PREVIEW_API","GLOBAL","CHANNELS","CLIENT_LOGGER","CORE_EVENTS","TYPES","ADDONS","TEST"])' +
  'globalThis["__STORYBOOK_MODULE_"+__g+"__"]??=__s;';
function stubBuiltChunkImports(src) {
  return STUB_DEF + src
    // Side-effect: `import"./x.css";` → drop. No leading anchor so
    // consecutive side-effect imports don't alternate-match.
    .replace(/\bimport\s*["']\.[^"']*["'];?/g, '')
    .replace(/import\s+(\w+)\s*,\s*(\{[^}]*\})\s*from\s*["']\.[^"']*["'];?/g,
      (_, d, n) => `const ${d}=__s;const${n.replace(/\s+as\s+/g, ':')}=__s;`)
    .replace(/import\s*(?:(\{[^}]*\})|\*\s*as\s+(\w+)|(\w+))\s*from\s*["']\.[^"']*["'];?/g,
      (_, n, s, d) => n ? `const${n.replace(/\s+as\s+/g, ':')}=__s;` : `const ${s ?? d}=__s;`);
}
export async function extractFromBuiltChunks(sbStatic, components, tmpDir) {
  const assets = join(sbStatic, 'assets');
  if (!existsSync(assets)) return new Map();
  const chunks = ls(assets).filter((f) => /\.stories-[\w-]+\.js$/.test(f));
  // importPath base ("./Button.stories.tsx" → "Button") → chunk files.
  // One component can have several *.stories-*.js chunks (Playground + each
  // example split separately) — collect all per base.
  const byBase = new Map();
  for (const f of chunks) {
    const b = f.replace(/\.stories-[\w-]+\.js$/, '');
    (byBase.get(b) ?? byBase.set(b, []).get(b)).push(f);
  }
  const argsById = new Map();
  installBrowserStubs();
  let hits = 0;
  for (const ip of new Set(components.flatMap((c) => [...(c.importPaths ?? [])]))) {
    const base = ip.split('/').pop().replace(/\.stories\.\w+$/, '');
    for (const chunk of byBase.get(base) ?? []) {
    const src = readFileSync(join(assets, chunk), 'utf8');
    const out = join(tmpDir, createHash('sha256').update(chunk).digest('hex').slice(0, 12) + '.mjs');
    writeFileSync(out, stubBuiltChunkImports(src));
    try {
      const mod = await import(pathToFileURL(out).href);
      const meta = mod.default ?? {};
      const order = Array.isArray(mod.__namedExportsOrder) ? mod.__namedExportsOrder : null;
      for (const [name, S] of Object.entries(mod)) {
        if (name === 'default' || name === '__namedExportsOrder' || S == null) continue;
        if (order && !order.includes(name)) continue;
        // composeStories' core behavior: meta.args merged under story.args.
        const args = { ...(meta.args ?? {}), ...(S.args ?? {}) };
        const argTypes = { ...(meta.argTypes ?? {}), ...(S.argTypes ?? {}) };
        if (!Object.keys(args).length && !Object.keys(argTypes).length && typeof S !== 'function') continue;
        const renderSource = (S.parameters ?? meta.parameters)?.docs?.source?.originalSource;
        // Title part uses only the LAST title segment so emit's fuzzy lookup
        // (which matches against the component name, not the full path) can
        // find it: "Components/Button" → "button", not "components-button".
        const titlePart = String(meta.title ?? base).split('/').pop();
        argsById.set(S.id ?? `${titlePart.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}--${name.replace(/([a-z])([A-Z])/g, '$1-$2').toLowerCase()}`, {
          args: Object.keys(args).length ? args : argsFromArgTypes(argTypes),
          argTypes, storyName: S.storyName ?? name,
          renderSource: typeof renderSource === 'string' ? renderSource : null,
        });
      }
      hits++;
    } catch (e) {
      console.error(`  ! chunk import failed: ${chunk}: ${String(e).split('\n')[0]}`);
    }
    }
  }
  if (hits) console.error(`  built-chunk extraction: ${hits}/${chunks.length} chunks → ${argsById.size} stories`);
  return argsById;
}

// Synthesize preview args from a Storybook argTypes map. Reads, in order:
// defaultValue, table.defaultValue.summary, first of control.options /
// options / mapping / type.value[]. Covers the shapes SB7 docgen and manual
// CSF both emit. Function/object-typed controls are skipped.
export function argsFromArgTypes(at) {
  const out = {};
  for (const [k, v] of Object.entries(at ?? {})) {
    if (!v || typeof v !== 'object') continue;
    const t = v.type?.name ?? v.type;
    if (t === 'function' || v.action) continue;
    let val = v.defaultValue;
    if (val === undefined) {
      const s = v.table?.defaultValue?.summary;
      if (s && s !== 'undefined') {
        try { val = JSON.parse(s); } catch { if (!/[(){}[\]]/.test(s)) val = s; }
      }
    }
    if (val === undefined) {
      const opts = v.control?.options ?? v.options
        ?? (v.mapping && Object.keys(v.mapping))
        ?? (Array.isArray(v.type?.value) && v.type.value.map((x) => x?.value ?? x));
      if (Array.isArray(opts) && opts.length) val = opts[0];
    }
    if (val === undefined && t === 'boolean') val = false;
    if (val !== undefined) out[k] = val;
  }
  return out;
}
