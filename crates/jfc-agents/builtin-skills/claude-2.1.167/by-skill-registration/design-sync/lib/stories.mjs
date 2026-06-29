// Node-side story-args extraction — each story file is esbuild-transpiled
// (packages:external), dynamically imported, and run through composeStories.
// Best-effort, per-file try/catch so one broken story (CSS import, ?raw
// loader, path alias) doesn't kill the rest.

import { build } from 'esbuild';
import { createRequire, isBuiltin } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { Project, SyntaxKind, ts } from 'ts-morph';

// Resolve and import @storybook/react from the DS repo's own dependency tree.
// pnpm/yarn-berry monorepos don't hoist, so a bare specifier or a direct
// `node_modules/@storybook/react` join both fail — use createRequire.resolve
// from (a) near the .storybook/ dir (where it's actually a dep) and (b) the
// --node-modules root. Returns the imported module or null.
export async function importStorybookReact({ nodeModules, sbDir }) {
  const bases = [sbDir, nodeModules].filter(Boolean).map((d) => resolve(d, '_'));
  for (const base of bases) {
    try {
      const entry = createRequire(pathToFileURL(base)).resolve('@storybook/react');
      return await import(pathToFileURL(entry).href);
    } catch {}
  }
  // Last resort: bare specifier (works when scripts are under the repo tree).
  try { return await import('@storybook/react'); } catch {}
  return null;
}


// CSF3 stories that use `render: () => <JSX>` instead of `.args` have empty
// args at composeStories time. Parse the story file and pull the first
// `<ComponentName …>` JSX element from the export's `render` initializer.
const storyProject = new Project({
  useInMemoryFileSystem: true,
  compilerOptions: { jsx: ts.JsxEmit.Preserve, allowJs: true },
});

export function extractRenderSource(raw, exportName, componentName) {
  const sf = storyProject.createSourceFile('s.tsx', raw, { overwrite: true });
  const decl = sf.getVariableDeclaration(exportName);
  const init = decl?.getInitializer();
  // An object-literal story with no `render` uses the default render — there's
  // no JSX source to extract (don't search decorators/parameters/play).
  const obj = init?.asKind?.(SyntaxKind.ObjectLiteralExpression);
  const render = obj ? obj.getProperty('render') : init;
  if (!render) return null;
  for (const jsx of render.getDescendantsOfKind(SyntaxKind.JsxElement)
    .concat(render.getDescendantsOfKind(SyntaxKind.JsxSelfClosingElement))) {
    const tag = (jsx.getOpeningElement?.() ?? jsx).getTagNameNode().getText();
    if (tag === componentName || tag.startsWith(`${componentName}.`)) {
      const txt = jsx.getText();
      return txt.length > 1500 ? null : txt;
    }
  }
  return null;
}

// Stories often touch browser globals at import time — stub enough DOM so
// node-side composeStories doesn't crash on window.matchMedia / document.body.
export function installBrowserStubs() {
  globalThis.window ??= globalThis;
  const noop = () => {};
  const stubEl = () => ({
    style: {}, dataset: {}, childNodes: [], children: [],
    setAttribute: noop, getAttribute: () => null, removeAttribute: noop, hasAttribute: () => false,
    appendChild: (c) => c, removeChild: (c) => c, insertBefore: (c) => c, append: noop, remove: noop,
    addEventListener: noop, removeEventListener: noop, dispatchEvent: () => true,
    querySelector: () => null, querySelectorAll: () => [], closest: () => null, contains: () => false,
    getBoundingClientRect: () => ({ top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0 }),
    focus: noop, blur: noop, cloneNode: stubEl, attachShadow: stubEl,
    classList: { add: noop, remove: noop, toggle: noop, contains: () => false },
    innerHTML: '', textContent: '',
  });
  globalThis.document ??= Object.assign(stubEl(), {
    createElement: stubEl, createElementNS: stubEl, createTextNode: () => ({ textContent: '' }),
    createDocumentFragment: stubEl, createComment: () => ({}),
    body: stubEl(), head: stubEl(), documentElement: stubEl(),
    getElementById: () => null, getElementsByTagName: () => [],
    readyState: 'complete', visibilityState: 'visible', hidden: false, currentScript: null,
  });
  globalThis.matchMedia ??= () => ({ matches: false, addListener: noop, removeListener: noop, addEventListener: noop, removeEventListener: noop });
  globalThis.navigator ??= { userAgent: 'node' };
  globalThis.addEventListener ??= noop;
  globalThis.removeEventListener ??= noop;
  globalThis.dispatchEvent ??= () => true;
  globalThis.getComputedStyle ??= () => ({ getPropertyValue: () => '' });
  globalThis.requestAnimationFrame ??= (cb) => setTimeout(cb, 0);
  globalThis.cancelAnimationFrame ??= clearTimeout;
  globalThis.requestIdleCallback ??= (cb) => setTimeout(cb, 0);
  globalThis.localStorage ??= { getItem: () => null, setItem: noop, removeItem: noop };
  globalThis.HTMLElement ??= class {};
  globalThis.Element ??= globalThis.HTMLElement;
  globalThis.Node ??= class {};
  globalThis.Event ??= class { constructor(t, o) { this.type = t; Object.assign(this, o); } };
  globalThis.CustomEvent ??= globalThis.Event;
  globalThis.customElements ??= { define: noop, get: () => undefined, upgrade: noop, whenDefined: () => Promise.resolve() };
  globalThis.MutationObserver ??= class { observe() {} disconnect() {} takeRecords() { return []; } };
  globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
  globalThis.IntersectionObserver ??= globalThis.ResizeObserver;
  globalThis.CSS ??= { supports: () => false };
  // Build-time constants that packages:external deps (not transpiled) may
  // reference bare.
  globalThis.__DEV__ ??= false;
}

// Like `packages: 'external'` but any bare specifier that node can't resolve
// from the output .mjs location is stubbed to an empty module instead of left
// as a throwing import — covers side-effect-only polyfills and unhoisted
// pnpm transitive deps the story eval pulls in. Logged once per pkg.
// `shared` carries resolve-cache + stubbed-log-once across a batch of builds
// (one nodeTranspile per story file).
const stubUnresolvable = (outFile, shared) => ({
  name: 'stub-unresolvable',
  setup(b) {
    const req = shared.req ??= createRequire(pathToFileURL(outFile));
    const cache = shared.resolveCache ??= new Map();
    b.onResolve({ filter: /^[^./]/ }, (a) => {
      // Windows absolute entry paths (`C:\…`) match this filter and their
      // drive letter parses like a URI scheme below — let esbuild resolve
      // entry points itself.
      if (a.kind === 'entry-point') return;
      // URI-scheme paths (node:, data:, blob:, http:) → external. Most are
      // CSS url() refs (kind==='url-token') where the stub namespace's JS
      // loader is rejected; JS-side data: imports are rare and externalizing
      // leaves them for node (which may or may not load them — best-effort).
      // `@scope/pkg` isn't a scheme (`@` excluded by ^[a-z]).
      if (/^[a-z][a-z0-9+.-]*:/i.test(a.path)) return { path: a.path, external: true };
      // Package-name import: external if node can resolve it from the output
      // file's location, else stub (logged — these are the informative ones).
      let ok = cache.get(a.path);
      if (ok === undefined) {
        try { req.resolve(a.path); ok = true; } catch { ok = false; }
        cache.set(a.path, ok);
        if (!ok) console.error(`  ! stubbed unresolvable: ${a.path}`);
      }
      return ok ? { path: a.path, external: true } : { path: a.path, namespace: 'stub-unresolvable' };
    });
    // Plain null-prototype CJS object — no Proxy. Side-effect imports work;
    // default/named imports destructure to undefined; any call/iterate on
    // them throws a normal TypeError that the per-file catch logs. The
    // Proxy-based stub hit 4 successive edge cases (ownKeys, descriptor,
    // Symbol.iterator, …) — not worth it for args-extraction.
    b.onLoad({ filter: /.*/, namespace: 'stub-unresolvable' }, () => ({
      contents: 'module.exports=Object.create(null);', loader: 'js',
    }));
  },
});

// Retry mode for stories whose externalized deps Node can't import natively
// (.json without an import attribute, raw .css, .ts/.tsx workspace source):
// keep react/react-dom/@storybook external (their identity must match the
// importing process), let esbuild's own node-style resolver handle every
// other bare specifier, and stub only what esbuild itself can't find.
const bundleDepsResolve = {
  name: 'bundle-deps',
  setup(b) {
    b.onResolve({ filter: /^[^./]/ }, async (a) => {
      // Same Windows entry-point caveat as stubUnresolvable above. The
      // pluginData guard breaks the b.resolve() re-entry below.
      if (a.kind === 'entry-point' || a.pluginData?.viaBundleDeps) return;
      if (isBuiltin(a.path)) return { path: a.path, external: true };
      if (/^[a-z][a-z0-9+.-]*:/i.test(a.path)) return { path: a.path, external: true };
      if (/^react(-dom)?(\/|$)|^react-is$|^scheduler$|^@storybook\//.test(a.path)) return { path: a.path, external: true };
      // Delegate to esbuild's own resolver: it resolves from the importer's
      // dir (so non-hoisted pnpm/yarn-berry layouts work) and honors the
      // `import` export-condition (so ESM-only deps work) — neither of which
      // createRequire(...).resolve handles. Stub only if esbuild can't either.
      const r = await b.resolve(a.path, { resolveDir: a.resolveDir, kind: a.kind, pluginData: { viaBundleDeps: true } });
      if (!r.errors.length) return r;
      console.error(`  ! stubbed unresolvable (bundleDeps): ${a.path}`);
      return { path: a.path, namespace: 'stub-unresolvable' };
    });
    b.onLoad({ filter: /.*/, namespace: 'stub-unresolvable' }, () => ({
      contents: 'module.exports=Object.create(null);', loader: 'js',
    }));
  },
};

export const nodeTranspile = (entry, out, shared = {}, { bundleDeps = false } = {}) =>
  build({
    entryPoints: [entry], outfile: out, bundle: true, format: 'esm',
    platform: 'node', jsx: 'automatic',
    plugins: [bundleDeps ? bundleDepsResolve : stubUnresolvable(out, shared)],
    // The bundleDeps output is ESM but bundles CJS deps that may
    // `require('react')` (kept external above) — esbuild rewrites those to a
    // `__require` shim that throws unless a real `require` is in scope.
    banner: bundleDeps ? { js: "import { createRequire as __cR } from 'node:module'; const require = __cR(import.meta.url);" } : undefined,
    loader: {
      '.css': 'empty', '.scss': 'empty', '.less': 'empty', '.js': 'jsx',
      '.svg': 'text', '.png': 'text', '.jpg': 'empty', '.jpeg': 'empty', '.gif': 'empty', '.webp': 'empty',
      '.woff': 'empty', '.woff2': 'empty', '.ttf': 'empty', '.otf': 'empty', '.eot': 'empty',
    },
    // Build-time constants some DS source trees reference bare.
    define: { __DEV__: 'false', 'process.env.NODE_ENV': '"production"' },
    logLevel: 'silent',
  });
