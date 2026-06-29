// One chromium page visit against the repo's own _sb/iframe.html: extract()
// for argTypes (→ .prompt.md prop tables) + fiber-walk provider detection.
// Provider match is name-based (displayName/name against the package's
// exported names) — the storybook page's React is a separate bundled copy,
// so identity-matching against our _ds_bundle.js wouldn't work.

import { serveDir } from './http-serve.mjs';

const EXTRACT = `(async () => {
  const api = globalThis.__STORYBOOK_PREVIEW__;
  if (!api) return {};
  if (api.storyStore?.cacheAllCSFFiles) await api.storyStore.cacheAllCSFFiles();
  const index = await api.extract();
  const out = {};
  for (const s of Object.values(index)) {
    const key = (s.title || '').split('/').pop();
    const tgt = out[key] ??= {};
    for (const [k, v] of Object.entries(s.argTypes || {})) tgt[k] ??= ({
      description: v.description || '',
      type: (v.table && v.table.type && v.table.type.summary) || (v.type && v.type.name) || '',
      required: !!(v.type && v.type.required),
      defaultValue: (v.table && v.table.defaultValue && v.table.defaultValue.summary) || '',
    });
  }
  return out;
})()`;

// Pass 1: walk fibers, store chain with LIVE prop object refs on window so
// pass 2 (after _ds_bundle.js injected) can identity-match prop values
// against DS exports. Component match is name-based (storybook's React is
// a separate copy); prop-value match is identity-based (the theme object
// the decorator passes IS the DS export, same module instance).
const FIBER_WALK_STORE = `((names) => {
  const set = new Set(names);
  const root = document.querySelector('#storybook-root');
  let n = root; while (n && n.firstElementChild) n = n.firstElementChild;
  if (!n || n === root) return (window.__dsChain = []).length;
  const fkey = Object.keys(n).find((k) => k.startsWith('__reactFiber$'));
  let fiber = fkey ? n[fkey] : null;
  const out = [];
  while (fiber) {
    const t = fiber.type || fiber.elementType;
    const nm = t && (t.displayName || t.name);
    if (nm && set.has(nm)) out.push({ component: nm, liveProps: fiber.memoizedProps || {} });
    fiber = fiber.return;
  }
  return (window.__dsChain = out.reverse()).length;
})`;

const RESOLVE_PROPS = `(() => {
  // Primitives pass through; objects become a $hint of their top-level keys
  // (the user sets the real value via cfg.provider).
  const ser = (v) => {
    if (v == null || typeof v !== 'object') return typeof v === 'function' ? undefined : v;
    if (v.$$typeof) return undefined;
    return { $hint: Object.keys(v).slice(0, 5).map((k) => String(k).replace(/[^\\w]/g, '_')).join(',') };
  };
  // The chain runs outermost-first. Keep only the outermost DS-exported
  // component plus any immediately-nested one whose name suggests it's
  // part of the provider shell (Provider/Theme/Root/App) — layout
  // components like Box/Grid deeper in are story-specific, not provider.
  const chain = window.__dsChain || [];
  const shell = chain.slice(0, 1).concat(
    chain.slice(1).filter((c, i) => i < 1 && /Provider|Theme|Root|App|Config|Styles|Reset|Base/i.test(c.component)),
  );
  return shell.map((c) => {
    const props = {};
    for (const [k, v] of Object.entries(c.liveProps)) {
      if (k === 'children') continue;
      const s = ser(v);
      if (s !== undefined) props[k] = s;
    }
    return { component: c.component, props };
  });
})`;

export async function probe({ out, globalName, firstStoryId, exportedNames }) {
  let pw;
  try { pw = await import('playwright'); }
  catch {
    console.error('[NO_CHROMIUM] argTypes + provider detection skipped — set cfg.provider manually if the DS needs one');
    return { argTypesByName: {}, provider: null };
  }
  const { srv, port } = await serveDir(out);
  let browser;
  try {
    browser = await pw.chromium.launch(
      process.env.DS_CHROMIUM_PATH ? { executablePath: process.env.DS_CHROMIUM_PATH } : {},
    );
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${port}/_sb/iframe.html?id=${encodeURIComponent(firstStoryId)}&viewMode=story`, { waitUntil: 'networkidle', timeout: 30_000 });
    await page.waitForSelector('#storybook-root > :not(style,script,link,meta,template)', { timeout: 10_000 }).catch(() => {});
    let extractTimer;
    const argTypesByName = await Promise.race([
      page.evaluate(EXTRACT),
      new Promise((r) => { extractTimer = setTimeout(() => r({}), 30_000); }),
    ]).catch(() => ({})).finally(() => clearTimeout(extractTimer));
    await page.evaluate(`(${FIBER_WALK_STORE})(${JSON.stringify(exportedNames)})`).catch(() => 0);
    const chain = await page.evaluate(`(${RESOLVE_PROPS})()`).catch(() => []);
    const provider = chain.length
      ? chain.reduceRight((inner, p) => ({ component: p.component, props: p.props, ...(inner ? { inner } : {}) }), null)
      : null;
    if (provider) console.error(`[PROVIDER_DETECTED] ${chain.map((p) => p.component).join(' > ')}`);
    console.error(`  extract(): argTypes for ${Object.keys(argTypesByName).length} component(s)`);
    return { argTypesByName, provider };
  } catch (e) {
    console.error(`  ! probe: ${String(e).split('\n')[0]}`);
    return { argTypesByName: {}, provider: null };
  } finally {
    await browser?.close().catch(() => {});
    srv.close();
  }
}
