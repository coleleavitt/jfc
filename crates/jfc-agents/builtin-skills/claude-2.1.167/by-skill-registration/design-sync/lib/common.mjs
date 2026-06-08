// Shared filesystem + string helpers used across the converter modules.
// Pure functions only — no process globals, no CLI parsing.

import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join, sep } from 'node:path';

// Normalize to `/` so downstream regexes and split('/') are platform-agnostic.
// Node fs functions accept `/` on Windows, so the normalized form is usable
// everywhere.
export const slash = (p) => (sep === '/' ? p : p.split(sep).join('/'));

// readdirSync order is filesystem-dependent; sort for reproducible output.
export const ls = (d, o) =>
  readdirSync(d, o).sort((a, b) => (a.name ?? a).localeCompare(b.name ?? b));

export const readText = (p) => (existsSync(p) ? readFileSync(p, 'utf8') : '');

export const escapeHtml = (s) =>
  String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

// Storybook title → {name, group}. titleMap remaps a derived name to the
// real export name (e.g. {"Toast": "ToastNotification"}). With `exportedSet`,
// scan segments right-to-left for the first that's a known export — handles
// 3-level titles like `Media/Carousel/Simple` where the last segment is the
// story variant, not the component.
export function titleParts(title, titleMap = {}, exportedSet = null) {
  const parts = title.split('/');
  const segs = parts.map((s) => s.replace(/\s+/g, ''));
  let idx = segs.length - 1;
  if (exportedSet) {
    for (let i = segs.length - 1; i >= 0; i--) {
      if (exportedSet.has(titleMap[segs[i]] ?? segs[i])) { idx = i; break; }
    }
  }
  let name = segs[idx];
  name = titleMap[name] ?? name;
  const group =
    (parts[idx - 1] || 'misc').trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'misc';
  return { name, group };
}

// Fuzzy lookup for an argsById entry — exact id first, else match on the
// title segment (alnum-squashed) equalling the component's name and the
// story-name segment matching. Tolerates index.json ids vs composeStories ids
// drifting in their title prefix (the Components/Button vs button case).
export function lookupArgsById(argsById, id, componentName) {
  let e = argsById?.get(id);
  if (e) return e;
  if (!argsById?.size) return undefined;
  const squash = (s) => s.replace(/[^a-z0-9]/gi, '').toLowerCase();
  const ownName = squash(componentName);
  const story = id.slice(id.lastIndexOf('--') + 2);
  for (const [k, v] of argsById) {
    const i = k.lastIndexOf('--');
    if (k.slice(i + 2) === story && squash(k.slice(0, i)) === ownName) return v;
  }
  return undefined;
}

// JSDoc `/** … */` block immediately preceding `name`'s own declaration,
// `* ` gutters stripped, empty string when no match. Walks backward from the
// decl so a multi-export file picks the nearest doc, not the first-in-file.
export function leadingJsdoc(text, name) {
  const declRx = name
    ? new RegExp(`(?:export\\s+)?(?:declare\\s+)?(?:const|let|function|class|interface|type)\\s+${name}\\b`)
    : /(?:export|declare|const|function|class|interface)/;
  const dm = declRx.exec(text);
  if (!dm) return '';
  const before = text.slice(0, dm.index);
  const end = before.lastIndexOf('*/');
  if (end < 0 || before.slice(end + 2).trim() !== '') return '';
  const start = before.lastIndexOf('/**', end);
  if (start < 0) return '';
  return before.slice(start + 3, end).split('\n').map((l) => l.replace(/^\s*\*\s?/, '')).join('\n').trim();
}

// Recursive directory walk, skipping node_modules. `accept(name)` filters
// which file basenames to collect; default keeps everything.
export function walk(dir, accept = () => true, out = []) {
  if (!existsSync(dir)) return out;
  for (const e of ls(dir, { withFileTypes: true })) {
    if (e.name === 'node_modules') continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, accept, out);
    else if (accept(e.name)) out.push(slash(p));
  }
  return out;
}
