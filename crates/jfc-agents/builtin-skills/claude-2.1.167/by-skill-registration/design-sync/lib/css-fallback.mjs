// Storybook-only CSS fallbacks — storybook-static's iframe.html is the source
// for both the compiled-stylesheet fallback (when _ds_bundle.css is a
// bundler-resolve-only stub) and remote webfont <link> scraping.

import { existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, sep } from 'node:path';

// Utility-CSS / CSS-in-JS DSes often ship a dist/styles.css
// that's a stub `@import "@scope/styles"` meant for a bundler to resolve.
export function isPlaceholderCss(p) {
  if (!existsSync(p)) return false;
  const sz = statSync(p).size;
  if (sz > 500) return false;
  const txt = readFileSync(p, 'utf8');
  // Only @import/@charset/comments/whitespace → no real rules.
  const stripped = txt.replace(/\/\*[\s\S]*?\*\//g, '').replace(/@(import|charset)\b[^;]*;/g, '').trim();
  return stripped.length === 0;
}

// If bundleCss is a placeholder stub, replace it with storybook-static's own
// compiled CSS (the largest local <link rel=stylesheet> in iframe.html).
// Relative url()s are NOT rewritten — sbStatic isn't uploaded, so pointing
// into it would break post-upload. They'll 404 in the preview (images missing)
// but class rules still apply. Returns the new srcDir for extractFonts, which
// DOES copy font files into the bundle.
export function fallbackCssFromStorybook({ bundleCss, sbStatic, out }) {
  if (!isPlaceholderCss(bundleCss) || !sbStatic || !existsSync(join(sbStatic, 'iframe.html'))) return null;
  const iframeHtml = readFileSync(join(sbStatic, 'iframe.html'), 'utf8');
  const links = [...iframeHtml.matchAll(/<link\b[^>]*>/gi)]
    .map((m) => m[0])
    .filter((t) => /\brel\s*=\s*["']stylesheet["']/i.test(t))
    .map((t) => t.match(/\bhref\s*=\s*["']([^"']+)["']/i)?.[1])
    .filter((h) => h && !/^(https?:|\/\/)/.test(h))
    .map((h) => join(sbStatic, h.replace(/^\.\//, '')))
    .filter((p) => p.startsWith(sbStatic + sep) && existsSync(p))
    .sort((a, b) => statSync(b).size - statSync(a).size);
  if (links[0]) {
    const oldSz = statSync(bundleCss).size;
    const kb = (statSync(links[0]).size / 1024).toFixed(0);
    const srcDir = dirname(links[0]);
    const css = readFileSync(links[0], 'utf8');
    const assets = [...new Set([...css.matchAll(/url\(\s*(['"]?)(?!data:|https?:|\/\/|\/)([^'")]+)\1\s*\)/gi)].map((m) => m[2]))];
    writeFileSync(bundleCss, css);
    console.error(`[CSS_FROM_STORYBOOK] _ds_bundle.css was a ${oldSz}B placeholder — replaced with ${relative(out, links[0])} (${kb} KB).`);
    if (assets.length) {
      console.error(`[CSS_ASSETS] ${assets.length} relative url() ref(s) in the fallback CSS won't resolve post-upload (fonts are copied separately via extractFonts; images will 404): ${assets.slice(0, 5).join(', ')}${assets.length > 5 ? ', …' : ''}`);
    }
    return srcDir;
  }
  console.error(`[CSS_PLACEHOLDER] _ds_bundle.css is a stub (@import-only, <500B) and no storybook CSS found to fall back to — set cfg.cssEntry to the compiled stylesheet.`);
  return null;
}

// Remote stylesheet links (webfonts, etc.) from the storybook iframe. CSS-in-JS
// DSes emit no static stylesheet, but commonly inject a remote webfont <link>
// via .storybook/preview-head.html — that link
// is then the ONLY static style source. Returns absolute URLs to @import url().
export function scrapeRemoteImports(sbStatic) {
  if (!sbStatic || !existsSync(join(sbStatic, 'iframe.html'))) return [];
  const iframeHtml = readFileSync(join(sbStatic, 'iframe.html'), 'utf8');
  const out = [...new Set(
    [...iframeHtml.matchAll(/<link\b[^>]*>/gi)]
      .map((m) => m[0])
      .filter((t) => /\brel\s*=\s*["']stylesheet["']/i.test(t))
      .map((t) => t.match(/\bhref\s*=\s*["']([^"']+)["']/i)?.[1])
      .filter((h) => h && /^(https?:|\/\/)/.test(h))
      .map((h) => (h.startsWith('//') ? 'https:' + h : h)),
  )];
  if (out.length) {
    console.error(`  remote stylesheet(s) from storybook: ${out.length} → styles.css @import url(...)`);
  }
  return out;
}
