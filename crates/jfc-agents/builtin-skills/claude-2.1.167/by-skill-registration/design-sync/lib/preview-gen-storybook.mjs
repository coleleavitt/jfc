// generatePreviewSource — emits the .design-sync/previews/<Name>.tsx body
// for one component. Helpers here are local to this function; the shared
// previews.mjs holds the marker/ownership machinery and buildPreviews.

import { lookupArgsById } from './common.mjs';
import { unknownRefs } from './previews.mjs';

// Export name from a story name: PascalCase the alnum runs; prefix S if it
// would start with a digit. Dedup with a counter.
function exportName(storyName, used) {
  let n = String(storyName ?? 'Default').split(/[^A-Za-z0-9]+/).filter(Boolean)
    .map((w) => w[0].toUpperCase() + w.slice(1)).join('') || 'Default';
  if (/^[0-9]/.test(n)) n = 'S' + n;
  let out = n, i = 2;
  while (used.has(out)) out = `${n}${i++}`;
  used.add(out);
  return out;
}

// Body for one export. renderSource is a JSX element string (extractRenderSource
// returns the first <Name>/<Name.X> element); the captured CSF3 render-fn may
// also be a full arrow/function expression.
function asFn(body) {
  const t = body.trim();
  if (/^(?:function\b|\([^)]*\)\s*=>|[\w$]+\s*=>)/.test(t)) return t;
  return `() => (\n  ${t}\n)`;
}

// smartDefaultProps $raw values — a small closed set of literal expressions.
// Whitelist-gated so config-sourced previewArgs can't inject arbitrary JS.
const RAW_OK = /^(?:\(\)\s*=>\s*(?:null|undefined|\{\})|new Date\(\))$/;

// JSON props → JSX attribute string. Functions / React elements drop out.
// `$raw` values (smartDefaultProps' crash-prevention stubs) emit as bare
// expression containers; everything else as `{JSON.stringify(v)}`.
function propsToJsx(args) {
  const out = [];
  for (const [k, v] of Object.entries(args)) {
    if (typeof v === 'function' || (v && typeof v === 'object' && v.$$typeof)) continue;
    // Dotted argType keys (`Title.as`) are sub-component addressing — not a
    // valid JSX attr name on the root.
    if (k === 'children' || k.includes('.')) continue;
    if (v && typeof v === 'object' && typeof v.$raw === 'string') {
      if (RAW_OK.test(v.$raw)) out.push(` ${k}={${v.$raw}}`);
    } else if (v && typeof v === 'object' && v.$jsx) {
      // scaffold-only marker — not expressible as a JSX attr here
    } else if (v === true) out.push(` ${k}`);
    else {
      try { out.push(` ${k}={${JSON.stringify(v)}}`); } catch { /* skip uncloneable */ }
    }
  }
  return out.join('');
}

// Children between `>`/`<` — wrap in `{JSON.stringify(...)}` so a value
// containing `{ } < >` doesn't reopen the parser. `{"plain"}` renders the
// same as `plain`, so always-wrap is correct.
const jsxChildren = (s) => `{${JSON.stringify(s)}}`;

// Placeholder child for the precedence-3/4/5 paths where smartDefaultProps
// would have set children to the bare component name. Renders a visible
// dashed box so the screenshot makes placeholder-ness obvious, and carries
// data-ds-placeholder so validate can flag it as needs-attention.
const placeholderChild = (name) =>
  `<div data-ds-placeholder="" style={{padding:8,minHeight:40,maxWidth:'100%',overflow:'hidden',boxSizing:'border-box',border:'1px dashed #999',color:'#999',fontSize:12}}>` +
  `{${JSON.stringify(name + ' content')}}</div>`;
const childJsx = (kids, name) => (kids === name ? placeholderChild(name) : jsxChildren(kids));

// PascalCase identifiers referenced as JSX tags, namespace heads, or bare
// attribute values (`icon={PlusIcon}`) in the renderSource bodies — best-
// effort regex, intersected with the DS export set.
function referencedExports(bodies, exported, own) {
  const refs = new Set([own]);
  const rx = /<([A-Z][A-Za-z0-9_]*)|(?:^|[^.\w])([A-Z][A-Za-z0-9_]*)\.|[{,[(]\s*([A-Z][A-Za-z0-9_]*)\s*[},\])]/g;
  for (const b of bodies) for (const m of b.matchAll(rx)) {
    const n = m[1] || m[2] || m[3];
    if (exported.has(n)) refs.add(n);
  }
  return [...refs].sort();
}

// Generate the preview .tsx body for one component (marker is prepended by
// writePreviewFiles so its hash covers this body only).
export function generatePreviewSource(c, { smart, members, exported, pkg, skip, previewArgs }) {
  // smart.props carries crash-prevention stubs from the .d.ts (required
  // callbacks → {$raw:'()=>null'}, arrays → [], open/visible → true). Spread
  // under explicit args so stubs fill gaps without overriding real values.
  const stubs = smart?.props ?? {};
  const stubKids = typeof stubs.children === 'string' ? stubs.children : null;
  // cfg.overrides.<Name>.skip mirrors emit.mjs's visibleStoryIds.
  const skipSet = new Set(skip ?? []);
  const stories = (c.storyIds ?? []).filter((s) => !skipSet.has(s.id));
  // Precedence 1/2: stories with renderSource or real args. Same fuzzy
  // argsById lookup emit.mjs uses (index.json ids vs composeStories ids may
  // differ in their title prefix). Seed `used` with the DS export set so a
  // story named like a DS export (`export const Form = () => <Form/>`)
  // doesn't shadow the import.
  const used = new Set(exported);
  const exports = [];
  const bodies = [];
  const skipped = [];
  for (const s of stories) {
    const e = lookupArgsById(c.argsById, s.id, c.name);
    let rs = e?.renderSource && /<[A-Z]/.test(e.renderSource) ? e.renderSource : null;
    if (rs) {
      const unknown = unknownRefs(rs, exported);
      if (unknown.length) { skipped.push([s.name, unknown]); rs = null; }
    }
    if (rs) {
      const body = asFn(rs);
      exports.push(`export const ${exportName(s.name, used)} = ${body};`);
      bodies.push(body);
    } else if (e?.args && Object.keys(e.args).length) {
      const kids = (typeof e.args.children === 'string' ? e.args.children : null) ?? stubKids;
      const attrs = propsToJsx({ ...stubs, ...e.args });
      const jsx = kids
        ? `<${c.name}${attrs}>${childJsx(kids, c.name)}</${c.name}>`
        : `<${c.name}${attrs} />`;
      exports.push(`export const ${exportName(s.name, used)} = () => ${jsx};`);
    }
  }
  // Precedence 2.5: cfg.previewArgs — accumulated config fixes from the §4
  // verify loop. Slot between story-derived exports and the variant grid so a
  // user-supplied previewArgs survives into the generated file.
  if (previewArgs) {
    const kids = (typeof previewArgs.children === 'string' ? previewArgs.children : null) ?? stubKids;
    const attrs = propsToJsx({ ...stubs, ...previewArgs });
    const jsx = kids
      ? `<${c.name}${attrs}>${childJsx(kids, c.name)}</${c.name}>`
      : `<${c.name}${attrs} />`;
    exports.push(`export const ${exportName('Preview', used)} = () => ${jsx};`);
  }
  if (skipped.length) {
    const refs = [...new Set(skipped.flatMap(([, u]) => u))].slice(0, 5);
    console.error(`  (preview: ${c.name} — ${skipped.length} renderSource(s) skipped, undeclared refs: ${refs.join(', ')}${refs.length < skipped.flatMap(([, u]) => u).length ? ', …' : ''})`);
  }
  if (exports.length) {
    const imports = referencedExports(bodies, exported, c.name);
    return `import { ${imports.join(', ')} } from '${pkg}';\n\n${exports.join('\n\n')}\n`;
  }
  // parentHint (tier-2 prefix-match) → emit an EXTRA cell wrapping this
  // component in its likely parent. For true subparts (TableRow) the wrapped
  // cell renders while Default is blank; for standalone-ok (FormItem) it's a
  // redundant extra cell. Never replaces the primary export.
  const parent = c.parentHint && exported.has(c.parentHint) ? c.parentHint : null;
  const selfAttrs = propsToJsx(stubs);
  const selfJsx = stubKids ? `<${c.name}${selfAttrs}>${childJsx(stubKids, c.name)}</${c.name}>` : `<${c.name}${selfAttrs} />`;
  const imps = parent ? [c.name, parent].sort().join(', ') : c.name;
  const wrap = (body) => parent
    ? `${body}\nexport const In${parent} = () => (\n  <${parent}>\n    ${selfJsx}\n  </${parent}>\n);\n`
    : body;
  // Precedence 3: variant grid from the .d.ts literal-union.
  if (smart?.variants) {
    const { prop, values } = smart.variants;
    const cells = values.slice(0, 4)
      .map((v) => `    <${c.name}${propsToJsx({ ...stubs, [prop]: v })}${stubKids ? `>${childJsx(stubKids, c.name)}</${c.name}>` : ' />'}`)
      .join('\n');
    return wrap(`import { ${imps} } from '${pkg}';\n\nexport const Variants = () => (\n  <>\n${cells}\n  </>\n);\n`);
  }
  // Precedence 4: namespace stub from the members list. Likely renders blank —
  // the value is that the user has the right file to edit.
  if (members?.length) {
    const root = members.includes('Root') ? `${c.name}.Root` : c.name;
    const item = members.find((m) => /^(Item|Panel|Tab|Trigger|Content|Body)$/.test(m)) ?? members[0];
    return wrap(`import { ${imps} } from '${pkg}';\n\nexport const Default = () => (\n  <${root}>\n    <${c.name}.${item}>${placeholderChild(c.name)}</${c.name}.${item}>\n  </${root}>\n);\n`);
  }
  // Precedence 5: smart-default fallback.
  return wrap(`import { ${imps} } from '${pkg}';\n\nexport const Default = () => ${selfJsx};\n`);
}
