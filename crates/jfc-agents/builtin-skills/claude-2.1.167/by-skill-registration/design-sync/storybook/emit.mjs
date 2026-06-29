// Per-component <Name>.html — iframe grid of _sb/iframe.html?id=<storyId>.
// Whatever renders in the repo's own Storybook renders here verbatim.

import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { escapeHtml } from '../lib/common.mjs';

const CARD_CSS =
  'body{margin:0;padding:16px;font-family:system-ui;background:#fff}' +
  '.ds-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(340px,1fr));gap:16px}' +
  '.ds-cell{border:1px solid #e5e7eb;border-radius:8px;overflow:hidden}' +
  '.ds-cell>h4{margin:0;padding:8px 12px;font:600 11px system-ui;color:#6b7280;text-transform:uppercase;border-bottom:1px solid #f3f4f6}' +
  '.ds-cell>iframe{border:0;width:100%;min-height:180px;display:block}';

// Resize each iframe to its rendered content via postMessage from the inner
// page (stripStorybook injects a small height-poll script into _sb/iframe.html).
const RESIZE = `
(function(){
  addEventListener('message', function(ev){
    if (ev.origin !== location.origin || !ev.data || ev.data.kind !== 'ds-height') return;
    var fs = document.querySelectorAll('iframe[data-sid]');
    for (var i = 0; i < fs.length; i++) {
      if (fs[i].dataset.sid === ev.data.sid) {
        fs[i].style.height = Math.min(1200, Math.max(120, Number(ev.data.h) || 120)) + 'px';
        break;
      }
    }
  });
})();`;

export function emitIframeHtml({ c, out, depth = 3 }) {
  const dir = join(out, 'components', c.group, c.name);
  mkdirSync(dir, { recursive: true });
  const rel = '../'.repeat(depth);
  const cells = (c.storyIds ?? []).slice(0, 8).map((s) =>
    `  <section class="ds-cell">` +
    `<h4>${escapeHtml(s.name)}</h4>` +
    `<iframe loading="lazy" data-sid="${escapeHtml(s.id)}" src="${rel}_sb/iframe.html?id=${encodeURIComponent(s.id)}&viewMode=story"></iframe>` +
    `</section>`,
  ).join('\n');
  const html = `<!-- @dsCard group="${escapeHtml(c.group)}" -->
<!doctype html><html><head><meta charset="utf-8">
  <style>${CARD_CSS}</style>
</head><body>
  <div class="ds-grid">
${cells}
  </div>
  <script>${RESIZE}</script>
</body></html>
`;
  writeFileSync(join(dir, `${c.name}.html`), html);
}
