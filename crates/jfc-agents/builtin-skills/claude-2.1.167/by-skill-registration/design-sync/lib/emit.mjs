// Output emitters: vendor React, per-component files (.jsx / .d.ts /
// .prompt.md / <Name>.html), README.md, demo.html, .ds-build-meta.json.
// Previews are self-contained (render from window.<GLOBAL>) — a story-args
// grid when the component has storyIds, else a scaffold the agent edits.

import { build } from 'esbuild';
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { join, resolve } from 'node:path';
import { escapeHtml, lookupArgsById, readText } from './common.mjs';
import { previewExamples } from './docs.mjs';

// React ≤18 ships UMD; React 19 dropped it, so we bundle our own IIFE.
export async function vendorReact({ nodeModules, out }) {
  const reactPkg = JSON.parse(readFileSync(join(nodeModules, 'react/package.json'), 'utf8'));
  // Both branches assign under a temp global then `||=`-merge so a host
  // page's existing React isn't clobbered.
  const noClobber =
    ';window.React=window.React||window.__dsReact;' +
    'window.ReactDOM=window.ReactDOM||window.__dsReactDOM;' +
    'try{delete window.__dsReact;delete window.__dsReactDOM;}catch(e){}';
  const reactUmd = join(nodeModules, 'react/umd/react.development.js');
  if (existsSync(reactUmd)) {
    writeFileSync(
      join(out, '_vendor', 'react.js'),
      ';(function(){var __r=window.React,__rd=window.ReactDOM;' +
      readFileSync(reactUmd, 'utf8') + '\n' +
      readFileSync(join(nodeModules, 'react-dom/umd/react-dom.development.js'), 'utf8') + '\n' +
      ';window.__dsReact=window.React;window.__dsReactDOM=window.ReactDOM;' +
      'if(__r)window.React=__r;if(__rd)window.ReactDOM=__rd;})();' + noClobber,
    );
  } else {
    console.error(`  react@${reactPkg.version} has no UMD — bundling via esbuild`);
    await build({
      stdin: {
        contents:
          'window.__dsReact=require("react");' +
          'window.__dsReactDOM=require("react-dom");' +
          'try{Object.assign(window.__dsReactDOM,require("react-dom/client"))}catch(e){}',
        resolveDir: nodeModules,
      },
      bundle: true, format: 'iife', outfile: join(out, '_vendor', 'react.js'),
      platform: 'browser', define: { 'process.env.NODE_ENV': '"development"' },
      logLevel: 'error', footer: { js: noClobber },
    });
  }
  writeFileSync(join(out, '_vendor', 'react-dom.js'), '/* merged into react.js */');
}

// Self-contained preview — one cell per story, rendered from
// window.<GLOBAL>.<Name> with the story's args. No _sb/ dependency;
// works identically for storybook- and package-sourced stories.
function previewHtmlBundle(group, name, GLOBAL, wrap, stories, argsById, decoratorScript, bundleCssLink, missCounter, fallbackProps) {
  // index.json's `id` and composeStories' `S.id` can differ in the title
  // prefix when the story module's default.title doesn't match what
  // storybook's indexer derived (e.g. indexer adds a 'Components/' segment).
  // Fall back: match when the argsById key's title segment (before `--`,
  // alnum-squashed) EQUALS this component's name and story-name segments
  // match. Exact title equality avoids ActionButton's `actionbutton--default`
  // satisfying Button's lookup; alnum-squash tolerates `action-button` vs
  // `actionbutton` (storybook toId() version differences).
  const squash = (s) => s.replace(/[^a-z0-9]/gi, '').toLowerCase();
  const titleStory = (s) => { const i = s.lastIndexOf('--'); return i < 0 ? [squash(s), ''] : [squash(s.slice(0, i)), s.slice(i + 2)]; };
  const ownName = squash(name);
  const argExpr = (id) => {
    let entry = argsById?.get(id);
    if (!entry && argsById?.size) {
      const [, story] = titleStory(id);
      for (const [k, v] of argsById) {
        const [kTitle, kStory] = titleStory(k);
        if (kStory === story && kTitle === ownName) { entry = v; break; }
      }
      if (!entry && missCounter) missCounter.n++;
    }
    const a = entry?.args;
    // CSF3 render-fn (empty args) — pull plain-text children from the JSX
    // source so `<Button>Click me</Button>` previews with its label, merged
    // over the fallback (Playground args / scaffold) so the cell still has
    // size/variant/etc. Whitespace-only children skip the merge.
    if ((!a || !Object.keys(a).length) && entry?.renderSource) {
      const m = />([^<>{}]{1,80})</.exec(entry.renderSource);
      const kids = m && m[1].trim();
      // Require a letter or digit (Unicode-aware so CJK labels pass):
      // multi-line parenthesized render bodies (`render: () => (\n  <X … />\n)`)
      // match the arrow's own `=> (` against the next `<`, which would pass a
      // literal "(" as the children.
      const hasText = kids && /\p{L}|\p{N}/u.test(kids);
      return scaffoldPropsExpr(hasText ? { ...fallbackProps, children: kids } : fallbackProps, 'C');
    }
    // No args, no renderSource → use the .d.ts-derived scaffold props so the
    // cell doesn't render blank next to siblings that do have args.
    if (!a || !Object.keys(a).length) return scaffoldPropsExpr(fallbackProps, 'C');
    // Replacer drops functions + React elements at any depth, keeps siblings.
    // `<` → < so a `</script>` in an arg (Code/Markdown stories) can't
    // terminate the inline script block.
    let json;
    try {
      json = JSON.stringify(a, (_, v) => (typeof v === 'function' || v?.$$typeof ? undefined : v));
    } catch { return '{}'; }
    return (json ?? '{}').replace(/</g, '\\u003c');
  };
  const cells = stories
    .map((s, i) => `<section class="ds-cell"><h4>${escapeHtml(s.name)}</h4><div id="r${i}"></div></section>`)
    .join('\n');
  // Per-mount try/catch so one bad story shows ⚠ in its cell and the rest render.
  const mounts = stories
    .map((s, i) => `try{ReactDOM.createRoot(document.getElementById('r${i}')).render(${wrap(`h(C,${argExpr(s.id)})`)})}catch(e){document.getElementById('r${i}').textContent='⚠ '+(e&&e.message||e)}`)
    .join('\n    ');
  return `<!-- @dsCard group="${escapeHtml(group)}" -->
<!doctype html>
<html><head><meta charset="utf-8">
  <link rel="stylesheet" href="../../../styles.css">${bundleCssLink}
  <style>
    body{margin:0;padding:24px;font-family:system-ui;background:#fff}
    .ds-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(320px,1fr));gap:20px;align-items:start}
    .ds-cell{border:1px solid #e5e7eb;border-radius:8px;padding:12px;min-width:0}
    .ds-cell>h4{margin:0 0 8px;font:600 12px system-ui;color:#6b7280;text-transform:uppercase;letter-spacing:.04em}
  </style>
</head><body>
  <div class="ds-grid">
${cells}
  </div>
  <script src="../../../_vendor/react.js"></script>
  <script src="../../../_vendor/react-dom.js"></script>
  <script src="../../../_ds_bundle.js"></script>${decoratorScript}
  <script>
    var h=React.createElement, C=window.${GLOBAL}.${name};
    ${mounts}
  </script>
</body></html>
`;
}

// Scaffold preview for non-storybook shapes — mounts the component with empty
// props. The agent edits this later to pass representative props (the SKILL
// step says: "open each preview.html; for any that render blank, edit it").
// Serialize smart-scaffold props to a JS expression. {$jsx: 'Item', text}
// becomes `h(C.Item,{},text)`; everything else JSON-stringifies (with `<`
// escaped — this lands in a <script> block).
function scaffoldPropsExpr(props, mount) {
  const esc = (s) => (JSON.stringify(s) ?? 'null').replace(/</g, '\\u003c');
  // $raw values from smartDefaultProps are a small closed set of literal
  // expressions — whitelist-gate them so config-supplied previewArgs can't
  // inject arbitrary JS into the emitted <script> block.
  const RAW_OK = /^(?:\(\)=>null|new Date\(\))$/;
  const pairs = Object.entries(props).map(([k, v]) => {
    const key = JSON.stringify(k);
    if (v && typeof v === 'object' && v.$jsx && /^[A-Za-z_$][\w$.]*$/.test(v.$jsx)) {
      return `${key}:h(${mount}.${v.$jsx},{},${esc(v.text ?? '')})`;
    }
    if (v && typeof v === 'object' && typeof v.$raw === 'string' && RAW_OK.test(v.$raw)) {
      return `${key}:${v.$raw}`;
    }
    return `${key}:${esc(v)}`;
  });
  return `{${pairs.join(',')}}`;
}

// Preview rendered from the compiled .design-sync/previews/<Name>.tsx — its
// IIFE assigns named exports to window.__dsPreview; mount each as a labeled
// cell under the existing provider wrap. Per-mount try/catch so one bad export
// shows ⚠ in its cell and the rest render.
function previewHtmlModule(group, name, GLOBAL, providerWrap, decoratorScript, bundleCssLink) {
  return `<!-- @dsCard group="${escapeHtml(group)}" -->
<!doctype html>
<html><head><meta charset="utf-8">
  <link rel="stylesheet" href="../../../styles.css">${bundleCssLink}
  <style>
    body{margin:0;padding:24px;font-family:system-ui;background:#fff}
    .ds-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(320px,1fr));gap:20px;align-items:start}
    .ds-cell{border:1px solid #e5e7eb;border-radius:8px;padding:12px;min-width:0}
    .ds-cell>h4{margin:0 0 8px;font:600 12px system-ui;color:#6b7280;text-transform:uppercase;letter-spacing:.04em}
  </style>
</head><body>
  <div class="ds-grid" id="g"></div>
  <script src="../../../_vendor/react.js"></script>
  <script src="../../../_vendor/react-dom.js"></script>
  <script src="../../../_ds_bundle.js"></script>${decoratorScript}
  <script src="../../../_preview/${name}.js"></script>
  <script>
    var h=React.createElement, g=document.getElementById('g'), i=0;
    for (var k in (window.__dsPreview||{})) {
      var fn=window.__dsPreview[k];
      if (typeof fn!=='function' || !/^[A-Z]/.test(k)) continue;
      var cell=document.createElement('section'); cell.className='ds-cell';
      cell.innerHTML='<h4>'+k+'</h4><div id="r'+i+'"></div>'; g.appendChild(cell);
      (function(fn,id){try{ReactDOM.createRoot(document.getElementById(id)).render(${providerWrap('h(fn)')})}catch(e){document.getElementById(id).textContent='⚠ '+(e&&e.message||e)}})(fn,'r'+i);
      i++;
    }
    if(i===0){g.textContent='⚠ no PascalCase exports in _preview/${name}.js'}
  </script>
</body></html>
`;
}

function previewHtmlScaffold(group, name, GLOBAL, providerWrap, rootMember, decoratorScript, bundleCssLink, smart) {
  // Namespace export (e.g. Dialog) — `h(C,{})` on a namespace object throws;
  // mount the Root sub-component instead.
  const mount = rootMember ? `C.${rootMember}` : 'C';
  const base = smart?.props ?? {};
  // One card per variant value (up to 4) — else a single mount with smart props.
  const cards = smart?.variants
    ? smart.variants.values.map((v, i) => ({ id: `r${i}`, label: `${smart.variants.prop}="${v}"`, props: { ...base, [smart.variants.prop]: v } }))
    : [{ id: 'root', label: null, props: base }];
  const cells = cards.map((c) => `${c.label ? `<h4 style="font:600 12px system-ui;color:#6b7280">${escapeHtml(c.label)}</h4>` : ''}<div id="${c.id}"></div>`).join('\n  ');
  const mounts = cards
    .map((c) => `try{ReactDOM.createRoot(document.getElementById('${c.id}')).render(${providerWrap(`h(${mount},${scaffoldPropsExpr(c.props, mount)})`)})}catch(e){document.getElementById('${c.id}').textContent='⚠ '+(e&&e.message||e)}`)
    .join('\n    ');
  return `<!-- @dsCard group="${escapeHtml(group)}" -->
<!doctype html>
<html><head><meta charset="utf-8">
  <link rel="stylesheet" href="../../../styles.css">${bundleCssLink}
  <style>body{margin:0;padding:24px;font-family:system-ui;background:#fff;display:grid;gap:20px}</style>
</head><body>
  ${cells}
  <script src="../../../_vendor/react.js"></script>
  <script src="../../../_vendor/react-dom.js"></script>
  <script src="../../../_ds_bundle.js"></script>${decoratorScript}
  <script>
    // Scaffold: edit the props below if this renders blank/broken — see <Name>.d.ts.
    var h=React.createElement, C=window.${GLOBAL}.${name};
    ${mounts}
  </script>
</body></html>
`;
}

// JS expression that wraps `expr` in the config's provider chain (if any).
// `{"$ref": "X"}` in a prop value emits `G.X` instead of a JSON literal —
// for providers that need a bundle export (e.g. `theme={LIGHT_THEME}`).
// `hasDecorators` → auto-detected .storybook/preview decorators were bundled
// to _vendor/preview-decorators.js which defines window.__dsDecorate; an
// explicit PROVIDER still wins so cfg.provider remains the manual override.
function providerWrapper(PROVIDER, GLOBAL, hasDecorators) {
  if (!PROVIDER && hasDecorators) {
    return (expr) => `(window.__dsDecorate?window.__dsDecorate(${expr}):${expr})`;
  }
  // p.component and props reach a `<script>` block — validate as identifier
  // paths and escape `<` in stringified values.
  for (let p = PROVIDER; p; p = p.inner) {
    if (!/^[A-Za-z_$][\w$.]*$/.test(p.component)) {
      console.error(`[PROVIDER_INVALID] cfg.provider component "${p.component}" isn't a valid identifier path`);
      return (e) => e;
    }
  }
  const providerProps = (props, G) => {
    const pairs = Object.entries(props ?? {}).map(([k, v]) => {
      // $hint reaches a /* */ comment inside a <script> block — strip */ and
      // < so it can neither terminate the comment nor open a tag.
      const san = (s) => String(s).replace(/\*\//g, '* /').replace(/</g, '\\u003c');
      const val = v && typeof v.$hint === 'string'
        ? `undefined /* your ${san(k)} — storybook applies an object with keys: ${san(v.$hint)} */`
        : JSON.stringify(v).replace(/</g, '\\u003c');
      return `${JSON.stringify(k)}:${val}`;
    });
    return `{${pairs.join(',')}}`;
  };
  return (expr, G = `window.${GLOBAL}`) => {
    // Collect the chain so we can wrap innermost-first (N-deep, matches
    // providerJsx's walk).
    const chain = [];
    for (let p = PROVIDER; p; p = p.inner) chain.push(p);
    let out = expr;
    for (let i = chain.length - 1; i >= 0; i--) {
      const p = chain[i];
      out = `h(${G}.${p.component},${providerProps(p.props, G)},${out})`;
    }
    return out;
  };
}

// Story-args → example JSX for .prompt.md. Sanitize so backticks/newlines
// can't break the ```jsx fence (args come from dynamically-imported stories).
function jsxExamples(c, visibleStoryIds) {
  const examples = visibleStoryIds.slice(0, 4).map((s) => {
    const entry = c.argsById?.get(s.id);
    // CSF3 render-fn stories have empty args — use the extracted JSX source
    // verbatim as the example.
    if (entry?.renderSource) {
      return `// ${String(s.name ?? '').replace(/[`\r\n]/g, ' ')}\n` +
        entry.renderSource.replace(/```/g, '');
    }
    const a = entry?.args;
    if (!a) return null;
    const fsan = (s) => String(s ?? '').replace(/[`\r\n]/g, ' ');
    const propStr = Object.entries(a)
      .filter(([k, v]) => k !== 'children' && v !== undefined && typeof v !== 'function')
      .map(([k, v]) => {
        const sk = fsan(k);
        if (v === true) return ` ${sk}`;
        let jv;
        try { jv = fsan(JSON.stringify(v)); } catch { jv = undefined; }
        if (jv === undefined) return ` ${sk}={/* … */}`;
        return typeof v === 'string' ? ` ${sk}=${jv}` : ` ${sk}={${jv.length > 60 ? '/* … */' : jv}}`;
      })
      .join('');
    const kids = typeof a.children === 'string' ? fsan(a.children).slice(0, 80) : '';
    return `// ${fsan(s.name)}\n<${c.name}${propStr}${kids ? `>${kids}</${c.name}>` : ' />'}`;
  }).filter(Boolean);
  return examples;
}

export function emitPerComponent({ src, components, OUT, GLOBAL, PKG, VERSION, OVERRIDES, REPLACES, PROVIDER, hasDecorators, exported, builtPreviews, propsBodyFor, compoundsFor, smartDefaultProps, previewArgs }) {
  // Allow dotted paths (`FrameContext.Provider`) — exported-check is on the head.
  const wrap = providerWrapper(PROVIDER && exported.has(PROVIDER.component.split('.')[0]) ? PROVIDER : null, GLOBAL, hasDecorators);
  const decoratorScript = hasDecorators ? '\n  <script src="../../../_vendor/preview-decorators.js"></script>' : '';
  const missCounter = { n: 0, total: 0 };
  // _ds_bundle.css is optional (CSS-in-JS / headless DSes have none).
  const bundleCssLink = existsSync(join(OUT, '_ds_bundle.css'))
    ? '\n  <link rel="stylesheet" href="../../../_ds_bundle.css">' : '';
  let done = 0;
  for (const c of components) {
    if (++done % 20 === 0 || done === components.length) console.error(`  [DTS] ${done}/${components.length} components`);
    // One dir per component — the self-check's cardByDir stores the first
    // @dsCard .html per directory, so the .jsx and .html must be the only
    // pair in their dir.
    const dir = join(OUT, 'components', c.group, c.name);
    mkdirSync(dir, { recursive: true });
    // Apply cfg.overrides.<Component>.skip once so the preview grid,
    // .prompt.md variants, JSX examples, and asset subtitle all agree.
    const skip = new Set(OVERRIDES[c.name]?.skip ?? []);
    const visibleStoryIds = (c.storyIds ?? []).filter((s) => !skip.has(s.id));
    c.visibleStoryIds = visibleStoryIds;

    // .jsx — one-line re-export into window scope.
    writeFileSync(
      join(dir, `${c.name}.jsx`),
      `// Re-export of ${PKG}@${VERSION} ${c.name}. Implementation is in the root _ds_bundle.js (window.${GLOBAL}).\n` +
        `Object.assign(window, { ${c.name}: window.${GLOBAL}.${c.name} });\n`,
    );

    // .d.ts — props interface from shipped types + @replaces JSDoc.
    const pb = propsBodyFor(c.name);
    const members = compoundsFor?.(c.name);
    const replaces = REPLACES[c.name] ? ` * @replaces ${REPLACES[c.name]}\n` : '';
    // Prelude (inlined type refs) goes AFTER the Props interface — the app's
    // parser takes the first interface in the file, and TS hoists type decls.
    const dts =
      `import * as React from 'react';\n\n` +
      `/**\n * ${c.name} — from ${PKG}@${VERSION}${c.importPaths?.size ? ` (${[...c.importPaths][0]})` : ''}.\n${replaces} */\n` +
      `export interface ${c.name}Props${pb?.generics ?? ''}${pb?.extendsClause ?? ''} {\n${pb?.body ?? '  [key: string]: unknown;'}\n}\n\n` +
      (pb?.prelude ?? '') +
      // A namespace-only export (`export * as Dialog` — Root present,
      // no own Props) isn't itself callable — declare as just the member map.
      (members?.includes('Root') && !pb
        ? `export declare const ${c.name}: {\n${members.map((m) => `  ${m}: React.ComponentType<any>;`).join('\n')}\n};\n`
        : `export declare const ${c.name}: React.ComponentType<${c.name}Props>` +
          (members?.length ? ` & {\n${members.map((m) => `  ${m}: React.ComponentType<any>;`).join('\n')}\n}` : '') +
          `;\n`);
    // Strip structural hints — they're for smartDefaultProps, not the .d.ts reader.
    writeFileSync(join(dir, `${c.name}.d.ts`), dts.replace(/ \/\* @(?:fn|arr) \*\//g, ''));

    // .prompt.md — first line is the element-index summary the design agent
    // reads; the body is the matched doc (cfg.docsDir / sibling .md) when one
    // exists, else a synthesized doc (## Props / ## Examples / ## Related)
    // built from what the converter already knows.
    const kw = c.docKeywords?.length ? ` Keywords: ${c.docKeywords.join(', ')}.` : '';
    const head = `${c.name} from ${PKG}. Use via \`window.${GLOBAL}.${c.name}\` (bundle loaded from the root \`_ds_bundle.js\`).${kw}\n`;
    // Flat-sibling related components (DialogBody/MenuItem/TabPanel are
    // separate exports, not dotted) — surface the <Name>-prefixed siblings.
    const siblings = components
      .filter((s) => s !== c && s.name.startsWith(c.name) && s.name.length > c.name.length && /^[A-Z]/.test(s.name.slice(c.name.length)))
      .map((s) => `\`${s.name}\``);
    let prompt;
    if (c.docBody) {
      prompt = head + '\n' + c.docBody + '\n';
      // Append the synthesized ## Props when the doc body doesn't carry its
      // own props table/section — keeps .prompt.md format consistent.
      if (pb?.body && !/##\s*Props\b|\|\s*Prop\s*\|/i.test(c.docBody)) {
        const bodyClean = pb.body.replace(/ \/\* @(?:fn|arr) \*\//g, '');
        prompt += `\n## Props\n\n\`\`\`ts\ninterface ${c.name}Props {\n${bodyClean}\n}\n\`\`\`\n`;
      }
    } else {
      // Synthesized doc — strictly better than the previous 4-line stub.
      const parts = [head];
      if (c.doc) parts.push(c.doc + '\n');
      if (members?.length) {
        const subs = members.map((m) => `\`${c.name}.${m}\``).join(', ');
        parts.push(`Sub-components: ${subs}. See the DS docs for composition — e.g. items like \`${c.name}.Item\` go inside \`<${c.name}>\`; containers like \`${c.name}.Group\` wrap multiple \`<${c.name}>\`s.\n`);
      }
      if (visibleStoryIds.length) {
        const variantNames = visibleStoryIds.map((s) => s.name);
        parts.push(`Variants (see \`${c.name}.html\`): ${variantNames.join(', ')}.\n`);
      }
      // ## Props — always include the section.
      if (pb?.body) {
        const bodyClean = pb.body.replace(/ \/\* @(?:fn|arr) \*\//g, '');
        parts.push(`## Props\n\n\`\`\`ts\ninterface ${c.name}Props {\n${bodyClean}\n}\n\`\`\`\n`);
      }
      // ## Examples — story args/renderSource jsxExamples first; then any
      // .design-sync/previews/<Name>.tsx exports (gracefully empty when that
      // mechanism isn't present).
      const exParts = [];
      const argEx = jsxExamples(c, visibleStoryIds);
      if (argEx.length) exParts.push('```jsx\n' + argEx.join('\n') + '\n```');
      exParts.push(...previewExamples(resolve('.design-sync', 'previews', `${c.name}.tsx`)));
      if (exParts.length) parts.push(`## Examples\n\n${exParts.join('\n\n')}\n`);
      // ## Related.
      if (siblings.length || members?.length) {
        const rel = [...siblings, ...(members ?? []).map((m) => `\`${c.name}.${m}\``)];
        parts.push(`## Related\n\n${rel.join(', ')}\n`);
      }
      prompt = parts.join('\n');
    }
    writeFileSync(join(dir, `${c.name}.prompt.md`), prompt);

    // <Name>.html — self-contained (no _sb/); same rendering for both shapes.
    const rootMember = members?.includes('Root') && !pb ? 'Root' : null;
    // Scaffold props for the fallback paths (builtPreviews takes precedence
    // over all of this). cfg.previewArgs > richest story args merged over
    // .d.ts smart-defaults > .d.ts smart-defaults > bare {}. previewHtmlBundle
    // uses it per-cell when a grid story has neither args nor renderSource.
    // Same fuzzy lookup previewHtmlBundle and previews.mjs use — title part of
    // the argsById key may differ from index.json's id when the chunk's
    // meta.title has a path prefix.
    const lookup = (id) => lookupArgsById(c.argsById, id, c.name);
    // A story only buys a distinct preview cell when it carries args beyond
    // `children`. CSF3 render-fn stories (renderSource, no args) and
    // children-only args render identically via fallbackProps in argExpr —
    // prefer the .d.ts-derived variant grid when fewer than two stories
    // actually differentiate. The renderSource still feeds .prompt.md above
    // and the per-cell children scrape inside previewHtmlBundle.
    const realStoryIds = visibleStoryIds.filter((s) => {
      const e = lookup(s.id);
      return Object.keys(e?.args ?? {}).some((k) => k !== 'children');
    });
    // Prefer the richest real story's args (usually Playground) as the
    // fallback base — when storybook's own args are available, they beat
    // .d.ts-derived heuristics for display values. Merge over the .d.ts
    // defaults so crash-preventing stubs (required callbacks, arrays) stay.
    const rawBest = realStoryIds
      .map((s) => lookup(s.id)?.args)
      .filter((a) => a && Object.keys(a).length)
      .sort((a, b) => Object.keys(b).length - Object.keys(a).length)[0];
    // Drop function/React-element values so the .d.ts `$raw:'()=>null'`
    // stubs aren't overwritten by the chunk's stubbed action handlers.
    const bestArgs = rawBest && Object.fromEntries(
      Object.entries(rawBest).filter(([, v]) => typeof v !== 'function' && !v?.$$typeof));
    const dtsSmart = smartDefaultProps?.(c.name, pb);
    const smart = previewArgs?.[c.name]
      ? { props: previewArgs[c.name], variants: null }
      : bestArgs
        ? { props: { ...(dtsSmart?.props ?? {}), ...bestArgs }, variants: null }
        : dtsSmart;
    // Precedence: compiled .design-sync/previews/<Name>.tsx → fall back to the
    // existing story-grid / scaffold paths when the preview build was skipped
    // or failed.
    const html = builtPreviews?.has(c.name)
      ? previewHtmlModule(c.group, c.name, GLOBAL, wrap, decoratorScript, bundleCssLink)
      : realStoryIds.length >= 2
        ? (missCounter.total += visibleStoryIds.length,
          previewHtmlBundle(c.group, c.name, GLOBAL, wrap, visibleStoryIds, c.argsById, decoratorScript, bundleCssLink, missCounter, smart?.props ?? {}))
        : previewHtmlScaffold(c.group, c.name, GLOBAL, wrap, rootMember, decoratorScript, bundleCssLink, smart);
    writeFileSync(join(dir, `${c.name}.html`), html);
  }
  if (missCounter.n) {
    console.error(`[ARGS_MISMATCH] ${missCounter.n}/${missCounter.total} stories had no argsById entry — index.json ids differ from composeStories ids (previews render with {} props)`);
  }
}

// demo.html — several components composed on one page.
export function emitDemo({ DEMO_SPEC, demoWrap, exported, OUT, GLOBAL, PROVIDER }) {
  // Allow dotted paths (`FrameContext.Provider`) — exported-check is on the head.
  const wrap = providerWrapper(PROVIDER && exported.has(PROVIDER.component.split('.')[0]) ? PROVIDER : null, GLOBAL);
  function demoExpr(spec) {
    const [n, p, kids] = spec;
    if (!exported.has(n)) return null;
    const ch = (kids ?? [])
      .map(demoExpr)
      .filter(Boolean)
      .map((e) => ',' + e)
      .join('');
    return `h(G.${n},${JSON.stringify(p ?? {}).replace(/</g, '\\u003c')}${ch})`;
  }
  const demoRows = DEMO_SPEC.map(demoExpr).filter(Boolean);
  if (!demoRows.length) return;
  let body = `h('div',{style:{display:'grid',gap:'16px',justifyItems:'start',padding:'24px',maxWidth:'720px'}},${demoRows.join(',')})`;
  if (demoWrap && exported.has(demoWrap.component)) {
    body = `h(G.${demoWrap.component},${JSON.stringify(demoWrap.props ?? {}).replace(/</g, '\\u003c')},${body})`;
  }
  writeFileSync(
    join(OUT, 'demo.html'),
    `<!doctype html>
<html><head><meta charset="utf-8">
  <link rel="stylesheet" href="styles.css">
  <link rel="stylesheet" href="_ds_bundle.css">
  <style>body{margin:0}</style>
</head><body>
  <div id="root"></div>
  <script src="_vendor/react.js"></script>
  <script src="_vendor/react-dom.js"></script>
  <script src="_ds_bundle.js"></script>
  <script>
    var h=React.createElement, G=window.${GLOBAL};
    var body=${body};
    ReactDOM.createRoot(document.getElementById('root')).render(${wrap('body', 'G')});
  </script>
</body></html>
`,
  );
}

// Provider JSX line for README (from cfg.provider chain).
function providerJsx(PROVIDER) {
  if (!PROVIDER) return '';
  let open = '', close = '';
  for (let p = PROVIDER; p; p = p.inner) {
    const props = Object.entries(p.props ?? {})
      .map(([k, v]) =>
        v && typeof v.$hint === 'string' ? ` ${k}={/* your ${k} — keys: ${String(v.$hint).replace(/\*\//g, '* /')} */}`
        : ` ${k}={${JSON.stringify(v)}}`).join('');
    open += `<${p.component}${props}>`;
    close = `</${p.component}>` + close;
  }
  return `${open}{children}${close}`;
}

export function emitReadme({ OUT, GLOBAL, PKG, VERSION, TOKENS_PKG, components, tokenFiles, demoNames, hasProvider, PROVIDER, jsdocFor }) {
  const tokenNames = new Set();
  for (const f of tokenFiles) {
    const css = readText(join(OUT, 'tokens', f));
    for (const m of css.matchAll(/(?<![\w-])(--[A-Za-z][\w-]*)\s*:/g)) tokenNames.add(m[1]);
  }
  const tokenFamilies = { color: [], spacing: [], typography: [], radius: [], shadow: [], other: [] };
  for (const t of tokenNames) {
    const k = /color|bg-|fg-|text-|fill|border-(?!radius|width)|surface/i.test(t) ? 'color'
      : /space|gap|pad|margin|inset|-p-|-m-/i.test(t) ? 'spacing'
      : /font|line-height|letter|weight|tracking/i.test(t) ? 'typography'
      : /radius|rounded/i.test(t) ? 'radius'
      : /shadow|elevation/i.test(t) ? 'shadow'
      : 'other';
    tokenFamilies[k].push(t);
  }
  const tokenOverview = Object.entries(tokenFamilies)
    .filter(([, v]) => v.length)
    .map(([k, v]) => `- **${k}** (${v.length}): \`${v.slice(0, 3).join('`, `')}\`${v.length > 3 ? ', …' : ''}`)
    .join('\n');
  const byGroup = new Map();
  for (const c of components) {
    if (!byGroup.has(c.group)) byGroup.set(c.group, []);
    byGroup.get(c.group).push(c);
  }
  const componentIndex = [...byGroup.entries()]
    .map(([g, cs]) => `### ${g}\n${cs.map((c) => {
      const doc = jsdocFor(c.name);
      return `- \`${c.name}\`${doc ? ` — ${doc}` : ''}`;
    }).join('\n')}`)
    .join('\n\n');
  const readme = `# ${GLOBAL} (${PKG}@${VERSION})

This design system is the published ${PKG} React library, bundled as a single
browser global. All ${components.length} components are the real upstream code.

## Where things are

- \`_ds_bundle.js\` — the whole-DS bundle at the project root; loads every component to \`window.${GLOBAL}\`. First line is a \`/* @ds-bundle: … */\` metadata header.
- \`styles.css\` + \`_ds_bundle.css\` — link both. \`styles.css\` carries tokens and fonts; \`_ds_bundle.css\` carries component styles.
- \`components/<group>/<Name>/<Name>.prompt.md\` (example JSX + variants), \`<Name>.d.ts\` (types), \`<Name>.html\` (variant grid).
- \`tokens/*.css\` — CSS custom properties, names verbatim from upstream.
- \`fonts/\` — \`@font-face\` files + \`fonts.css\` (when the package ships fonts).

For a specific component, \`read_file("components/<group>/<Name>/<Name>.prompt.md")\`.

## Loading

Add these three lines to your page once (React must be on the page first):

\`\`\`html
<link rel="stylesheet" href="styles.css">
<link rel="stylesheet" href="_ds_bundle.css">
<script src="_ds_bundle.js"></script>
\`\`\`

Components are then available at \`window.${GLOBAL}.*\`. Mount into a dedicated child node (e.g. \`<div id="ds-root">\`), not the host page's own React root, so the two trees don't collide:

\`\`\`jsx
const { ${demoNames.slice(0, 3).join(', ') || (components[0]?.name ?? 'Component')} } = window.${GLOBAL};
ReactDOM.createRoot(document.getElementById('ds-root')).render(<${demoNames[0] ?? components[0]?.name ?? 'Component'} />);
\`\`\`
${hasProvider ? `
Wrap the tree in the provider — most components read theme/i18n from context:

\`\`\`jsx
${providerJsx(PROVIDER)}
\`\`\`
` : ''}
## Tokens

${tokenNames.size} CSS custom properties from ${TOKENS_PKG ?? PKG}. Names are
preserved verbatim from upstream. See \`tokens/\` for the full list.

${tokenOverview}

## Components

${componentIndex}
`;
  writeFileSync(join(OUT, 'README.md'), readme);
}

// .ds-build-meta.json — LOCAL build metadata only. The validator reads
// `componentCount` / `skippedStoryIds` / `runtimeFontPrefixes`; it is NOT
// uploaded.
export function emitBuildMeta({ OUT, GLOBAL, PKG, VERSION, PROVIDER, OVERRIDES, components, shape, cfg }) {
  const skippedStoryIds = [...new Set(Object.values(OVERRIDES).flatMap((o) => o?.skip ?? []))];
  // Fence so consumers don't read a half-uploaded tree (see §5 Upload).
  // The app's self-check reads `by` to set the manifest's `source`.
  writeFileSync(join(OUT, '_ds_needs_recompile'), JSON.stringify({ by: 'design-sync-cli' }));
  writeFileSync(
    join(OUT, '.ds-build-meta.json'),
    JSON.stringify(
      {
        namespace: GLOBAL,
        source: `${PKG}@${VERSION}`,
        shape,
        provider: PROVIDER?.component ?? null,
        componentCount: components.length,
        skippedStoryIds,
        runtimeFontPrefixes: cfg?.runtimeFontPrefixes ?? [],
      },
      null,
      2,
    ) + '\n',
  );
  return components.length;
}
