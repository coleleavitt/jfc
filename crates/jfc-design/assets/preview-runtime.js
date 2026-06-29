(function () {
  if (window.__om && window.__om.__jfc) return;

  const config = __JFC_DESIGN_CONFIG__;
  const OVERLAY_ATTR = 'data-designer-overlay';
  const CHROME_SELECTOR = '[data-omelette-chrome],[data-noncommentable],.twk-panel,[data-designer-overlay]';
  const EVENT_PREFIX = '__OM_EVT__';
  const refs = new Map();
  const cursorStack = [];
  let nextRef = 1;
  let hoverOverlay = null;
  let selectedOverlay = null;
  let dropOverlay = null;
  let cursorStyle = null;
  let selected = null;
  let textEditElement = null;
  let dragging = null;

  const STYLE_PROPS = [
    'position', 'top', 'left', 'right', 'bottom', 'zIndex',
    'width', 'height', 'minWidth', 'maxWidth', 'minHeight', 'maxHeight',
    'display', 'flexDirection', 'justifyContent', 'alignItems', 'flexWrap', 'gap',
    'gridTemplateColumns', 'gridTemplateRows',
    'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft',
    'marginTop', 'marginRight', 'marginBottom', 'marginLeft',
    'backgroundColor', 'backgroundImage',
    'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth',
    'borderStyle', 'borderColor', 'borderRadius',
    'boxShadow', 'textShadow',
    'color', 'fontSize', 'fontWeight', 'fontFamily', 'lineHeight', 'letterSpacing',
    'textAlign', 'textDecorationLine', 'textTransform',
    'opacity', 'overflow',
    'flexGrow', 'flexShrink', 'flexBasis', 'alignSelf',
    'fill', 'fillOpacity', 'fillRule',
    'stroke', 'strokeWidth', 'strokeOpacity', 'strokeDasharray',
    'strokeLinecap', 'strokeLinejoin'
  ];
  const SEMANTIC_TAGS = new Set(['a', 'button', 'input', 'select', 'textarea', 'label', 'form', 'nav', 'header', 'footer', 'main', 'section', 'article', 'aside', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6']);
  const TEXT_TAGS = new Set(['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'li', 'dt', 'dd', 'blockquote', 'figcaption', 'label', 'span', 'a', 'em', 'strong', 'small', 'td', 'th', 'caption']);
  const MEDIA_TAGS = new Set(['img', 'video']);
  const FORM_TAGS = new Set(['input', 'textarea', 'select', 'option', 'audio', 'video']);
  const VOID_TAGS = new Set(['img', 'input', 'br', 'hr', 'area', 'base', 'col', 'embed', 'link', 'meta', 'param', 'source', 'track', 'wbr']);
  const SVG_HIT_TAGS = 'hr,line,rect,path,polyline,polygon,circle,ellipse';

  function api(suffix) {
    return '/design/projects/' + encodeURIComponent(config.projectId || '') + suffix;
  }
  function trimText(value, limit) {
    const text = String(value || '').trim().replace(/\s+/g, ' ');
    return text.length > limit ? text.slice(0, Math.max(0, limit - 1)) + '...' : text;
  }
  function cssEscape(value) {
    return window.CSS && CSS.escape ? CSS.escape(String(value)) : String(value).replace(/[^a-zA-Z0-9_-]/g, '\\$&');
  }
  function attrValue(value) {
    return String(value).replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  }
  function queryCount(selector) {
    try { return document.querySelectorAll(selector).length; } catch { return 999999; }
  }
  function emit(type, detail) {
    const msg = { type, detail: detail || {}, projectId: config.projectId, path: config.path, ts: Date.now() };
    window.dispatchEvent(new CustomEvent(type, { detail: msg }));
    try { window.parent && window.parent.postMessage({ __OM_MSG__: msg }, '*'); } catch {}
    try { window.parent && window.parent.postMessage({ __DM_MSG__: msg }, '*'); } catch {}
    const legacy = legacyEvent(type, detail || {});
    if (legacy) {
      legacyConsoleEvent(legacy);
    }
    return msg;
  }
  function legacyConsoleEvent(event, detail) {
    const legacy = typeof event === 'string' ? { t: event, payload: detail || {} } : (event || {});
    try { console.log(EVENT_PREFIX + JSON.stringify(legacy)); } catch {}
    try { window.parent && window.parent.postMessage({ __OM_EVT__: legacy }, '*'); } catch {}
    try { window.parent && window.parent.postMessage({ __DM_EVT__: legacy }, '*'); } catch {}
    return legacy;
  }
  function legacyEvent(type, detail) {
    if (type === 'om:selected') return { t: 'selected', payload: detail };
    if (type === 'om:hovered') return { t: 'hovered', payload: detail || null };
    if (type === 'om:selection-changed') return { t: 'selectionChanged', sel: detail };
    if (type === 'om:range-style') return { t: 'rangeStyle', style: detail && detail.style };
    if (type === 'om:edit-action') return { t: 'editAction', action: detail && detail.action };
    if (type === 'om:drag-drop') return { t: 'dragDrop', drop: detail };
    if (type === 'om:anchor-moved') return { t: 'anchorMoved', rect: detail };
    return null;
  }

  function getRef(el) {
    if (!el || el.nodeType !== 1) return null;
    let ref = el.getAttribute('data-dm-ref') || el.getAttribute('data-om-ref') || el.getAttribute('data-cc-id');
    if (!ref) {
      ref = String(nextRef++);
      el.setAttribute('data-dm-ref', ref);
      el.setAttribute('data-om-ref', ref);
      el.setAttribute('data-cc-id', 'cc-' + ref);
    }
    refs.set(String(ref).replace(/^cc-/, ''), el);
    refs.set(String(ref), el);
    return Number(String(ref).replace(/^cc-/, '')) || ref;
  }
  function byRef(ref) {
    const key = String(ref).replace(/^dm-/, '').replace(/^cc-/, '');
    const known = refs.get(key) || refs.get(String(ref));
    if (known && known.isConnected) return known;
    const selector = '[data-dm-ref="' + attrValue(key) + '"],[data-om-ref="' + attrValue(key) + '"],[data-cc-id="' + attrValue(String(ref)) + '"]';
    const found = document.querySelector(selector);
    if (found) refs.set(key, found);
    return found || null;
  }

  function reactFiber(el) {
    if (!el) return null;
    const key = Object.keys(el).find((name) => name.startsWith('__reactFiber$') || name.startsWith('__reactInternalInstance$'));
    return key ? el[key] : null;
  }
  function reactName(el) {
    try {
      let fiber = reactFiber(el);
      for (let depth = 0; fiber && depth < 24; depth += 1, fiber = fiber.return || null) {
        const type = fiber.type || fiber.elementType;
        if (typeof type === 'function') {
          const name = type.displayName || type.name;
          if (name && name.length > 1) return name;
        } else if (type && typeof type === 'object' && type.displayName) {
          return type.displayName;
        }
      }
    } catch {}
    return null;
  }
  function nearestReactSource(el) {
    try {
      let fiber = reactFiber(el);
      for (let depth = 0; fiber && depth < 32; depth += 1, fiber = fiber.return || null) {
        if (fiber._debugSource) return fiber._debugSource;
        if (fiber.return && typeof fiber.return.type !== 'string' && fiber.return._debugSource) return fiber.return._debugSource;
      }
    } catch {}
    return null;
  }
  function markReactSourceElements() {
    const nodes = Array.from(document.querySelectorAll('body *')).slice(0, 5000);
    for (const el of nodes) {
      if (el.hasAttribute('data-om-id')) continue;
      const source = nearestReactSource(el);
      if (!source || !source.fileName || !source.lineNumber) continue;
      const file = String(source.fileName);
      const line = Number(source.lineNumber) || 0;
      const col = Number(source.columnNumber) || 0;
      const tag = el.tagName.toLowerCase();
      el.setAttribute('data-om-id', 'jsx:' + file + ':' + tag + ':' + line + ':' + col);
      el.setAttribute('data-om-path', file);
      el.setAttribute('data-om-line', String(line));
      el.setAttribute('data-om-column', String(col));
    }
  }

  function readStyles(el) {
    const computed = getComputedStyle(el);
    const out = {};
    for (const prop of STYLE_PROPS) out[prop] = computed[prop];
    return out;
  }
  function readInlineStyles(el) {
    const out = {};
    const style = el.style || {};
    for (const prop of STYLE_PROPS) if (style[prop]) out[prop] = style[prop];
    return out;
  }
  function rectOf(el) {
    if (!el) return null;
    const rect = el.classList && el.classList.contains('__om-t')
      ? (() => { const range = document.createRange(); range.selectNodeContents(el); return range.getBoundingClientRect(); })()
      : el.getBoundingClientRect();
    if ((rect.width || rect.height) || !el.children || !el.children.length) {
      return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
    }
    let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity;
    for (const child of el.children) {
      const childRect = child.getBoundingClientRect();
      if (!childRect.width && !childRect.height) continue;
      left = Math.min(left, childRect.left);
      top = Math.min(top, childRect.top);
      right = Math.max(right, childRect.right);
      bottom = Math.max(bottom, childRect.bottom);
    }
    if (left !== Infinity) return { x: left, y: top, width: right - left, height: bottom - top };
    return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
  }
  function makeOverlay(color, dashed) {
    const el = document.createElement('div');
    el.setAttribute(OVERLAY_ATTR, '1');
    el.style.cssText = 'position:fixed;pointer-events:none;border:2px solid ' + color + ';border-radius:4px;z-index:100000;display:none;';
    if (dashed) {
      el.style.borderStyle = 'dashed';
      el.style.background = 'rgba(37,99,235,0.06)';
    }
    document.body.appendChild(el);
    return el;
  }
  function updateOverlay(overlay, target) {
    const rect = rectOf(target);
    if (!overlay || !rect) return;
    overlay.style.left = (rect.x - 2) + 'px';
    overlay.style.top = (rect.y - 2) + 'px';
    overlay.style.width = (rect.width + 4) + 'px';
    overlay.style.height = (rect.height + 4) + 'px';
    overlay.style.display = 'block';
    overlay.__el = target;
  }
  function hideOverlays() {
    for (const el of document.querySelectorAll('[' + OVERLAY_ATTR + ']')) el.style.display = 'none';
  }
  function deepElementFromPoint(x, y) {
    let el = document.elementFromPoint(x, y);
    while (el && el.shadowRoot) {
      const inner = el.shadowRoot.elementFromPoint(x, y);
      if (!inner || inner === el) break;
      el = inner;
    }
    if (el && !el.closest(CHROME_SELECTOR)) {
      let root = el.getRootNode && el.getRootNode();
      while (root && root.host) {
        el = root.host;
        root = el.getRootNode && el.getRootNode();
      }
    }
    return el;
  }
  function preciseHit(x, y) {
    const base = deepElementFromPoint(x, y);
    if (!base || base.closest(CHROME_SELECTOR)) return null;
    if (!base.children || base.children.length === 0) return base;
    let best = null;
    let bestDist = Infinity;
    const shapes = base.querySelectorAll(SVG_HIT_TAGS);
    for (let index = 0; index < Math.min(shapes.length, 64); index += 1) {
      const shape = shapes[index];
      if (shape.closest(CHROME_SELECTOR)) continue;
      const rect = shape.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) continue;
      const dx = Math.max(rect.left - x, 0, x - rect.right);
      const dy = Math.max(rect.top - y, 0, y - rect.bottom);
      const dist = Math.max(dx, dy);
      if (dist <= 5 && dist < bestDist) {
        best = shape;
        bestDist = dist;
      }
    }
    return best || base;
  }
  function isChromeEvent(event) {
    const path = typeof event.composedPath === 'function' ? event.composedPath() : null;
    if (path) return path.some((el) => el && el.nodeType === 1 && el.matches && el.matches(CHROME_SELECTOR));
    return !!event.target && !!event.target.closest && !!event.target.closest(CHROME_SELECTOR);
  }

  function absoluteSelector(el) {
    if (!el || el.nodeType !== 1) return '';
    if (el === document.body) return 'body';
    if (el.id) {
      const id = '#' + cssEscape(el.id);
      if (queryCount(id) === 1) return id;
    }
    const parent = el.parentElement;
    if (!parent) return el.tagName.toLowerCase();
    const index = Array.from(parent.children).indexOf(el) + 1;
    return absoluteSelector(parent) + ' > ' + el.tagName.toLowerCase() + ':nth-child(' + index + ')';
  }
  function semanticClassScore(name) {
    if (!name || name.length < 2) return -2;
    if (/^(btn|button|nav|menu|header|footer|main|content|container|wrapper|row|col|column|title|heading|label|input|form|card|panel|section|item|list|link|icon|img|image|bg|background|text|font|color|size|flex|grid|layout|margin|padding|border|active|disabled|hidden|visible|selected|hover|focus)/.test(name)) return 4;
    if (/^[a-z]+(-[a-z]+)*$/.test(name) || /^[a-z][a-zA-Z0-9]*$/.test(name)) return 4;
    if (/^[a-zA-Z0-9]{5,}$/.test(name) || /^_[a-zA-Z0-9]+_[a-zA-Z0-9]+/.test(name) || /[0-9a-f]{4,}/.test(name)) return -2;
    return 0;
  }
  function simpleCandidates(el) {
    const out = [];
    const tag = el.tagName.toLowerCase();
    const add = (selector, score) => {
      if (!selector) return;
      out.push({ selector, score, matchCount: queryCount(selector) });
    };
    if (el === document.body) {
      add('body', 10);
      return out;
    }
    if (el.id) add('#' + cssEscape(el.id), 10);
    const anchor = el.getAttribute('data-comment-anchor');
    if (anchor) add('[data-comment-anchor="' + attrValue(anchor) + '"]', 10);
    const omId = el.getAttribute('data-om-id');
    if (omId) add('[data-om-id="' + attrValue(omId) + '"]', omId.startsWith('jsx:') ? 5 : 10);
    add(tag, SEMANTIC_TAGS.has(tag) ? 1 : -1);
    const siblings = el.parentElement ? Array.from(el.parentElement.children).filter((node) => node.tagName === el.tagName) : [];
    if (siblings.length > 1) add(tag + ':nth-of-type(' + (siblings.indexOf(el) + 1) + ')', 0);
    if (el.parentElement && Array.from(el.parentElement.children).indexOf(el) === el.parentElement.children.length - 1) add(tag + ':last-child', 0);
    for (const cls of Array.from(el.classList || [])) add('.' + cssEscape(cls), semanticClassScore(cls));
    for (const attr of Array.from(el.attributes || [])) {
      if (['id', 'class', 'style', 'contenteditable'].includes(attr.name)) continue;
      if (attr.name.startsWith('data-om-') || attr.name.startsWith('data-designer-') || attr.name === 'data-dm-ref' || attr.name === 'data-cc-id' || attr.name === 'data-omelette-injected') continue;
      const score = attr.name === 'role' || attr.name === 'aria-label' ? 6 : -1;
      add(tag + '[' + attr.name + ']', score);
      if (attr.value) add(tag + '[' + attr.name + '="' + attrValue(attr.value) + '"]', score + 1);
    }
    if (TEXT_TAGS.has(tag)) add(':is(h1,h2,h3,h4,h5,h6,p,li,dt,dd,blockquote,figcaption,label,span,a,em,strong,small,td,th,caption)', 3);
    if (MEDIA_TAGS.has(tag)) add(':is(img,video)', 3);
    return out;
  }
  function rankedSelectorList(el) {
    if (!el || el.nodeType !== 1) return [];
    const seen = new Set();
    let candidates = simpleCandidates(el);
    let parent = el.parentElement;
    for (let depth = 0; parent && depth < 3; depth += 1, parent = parent.parentElement) {
      const parents = simpleCandidates(parent).filter((item) => item.matchCount <= 1000).slice(0, 8);
      const current = candidates.slice(0, 24);
      for (const p of parents) {
        for (const c of current) {
          candidates.push({
            selector: p.selector + (depth === 0 ? ' > ' : ' ') + c.selector,
            score: p.score + c.score - 2,
            matchCount: queryCount(p.selector + (depth === 0 ? ' > ' : ' ') + c.selector)
          });
        }
      }
    }
    candidates = candidates
      .filter((item) => item.matchCount <= 4000)
      .sort((a, b) => (a.matchCount - b.matchCount) || (b.score - a.score));
    const out = [];
    const counts = new Set();
    for (const item of candidates) {
      if (seen.has(item.selector)) continue;
      seen.add(item.selector);
      if (!counts.has(item.matchCount)) {
        out.push({ selector: item.selector, matchCount: item.matchCount });
        counts.add(item.matchCount);
      }
      if (out.length >= 20) break;
    }
    const fallback = absoluteSelector(el);
    if (fallback && !seen.has(fallback)) out.push({ selector: fallback, matchCount: queryCount(fallback) });
    return out;
  }
  function selectorFor(el) {
    if (!el || el.nodeType !== 1) return null;
    if (el === document.body) return 'body';
    const omId = el.getAttribute('data-om-id');
    if (omId) {
      const selector = '[data-om-id="' + attrValue(omId) + '"]';
      if (!omId.startsWith('jsx:') || queryCount(selector) === 1) return selector;
    }
    const ranked = rankedSelectorList(el);
    const unique = ranked.find((item) => item.matchCount === 1);
    return (unique || ranked[0] || { selector: absoluteSelector(el) }).selector;
  }
  function selectionPayload(el, click) {
    const selector = selectorFor(el);
    const rect = rectOf(el);
    return {
      selector,
      selectors: rankedSelectorList(el),
      matchCount: selector ? queryCount(selector) : 0,
      omId: el.getAttribute('data-om-id') || undefined,
      leafTag: el.tagName.toLowerCase(),
      description: shortLabel(el),
      descriptor: richDescriptor(el, selector),
      rect,
      clickX: click && click.x,
      clickY: click && click.y,
      source: sourceHints(el)
    };
  }
  function sourceHints(el) {
    const attrs = {};
    for (const name of ['data-source', 'data-src', 'data-src-file', 'data-src-line', 'data-src-column', 'data-om-source', 'data-om-path', 'data-om-line', 'data-om-column', 'data-om-start', 'data-om-end', 'data-source-start', 'data-source-end', 'data-src-loc', 'data-loc']) {
      const value = el.getAttribute && el.getAttribute(name);
      if (value) attrs[name] = value;
    }
    const react = nearestReactSource(el);
    const loc = parseSrcLoc(attrs['data-src-loc'] || attrs['data-loc']);
    return { attributes: attrs, react, generated: loc };
  }
  function parseSrcLoc(value) {
    if (!value) return null;
    const match = String(value).match(/^(.*?)(?::(\d+))(?::(\d+))?$/);
    if (!match) return null;
    return { fileName: match[1], lineNumber: Number(match[2]), columnNumber: Number(match[3] || 0) };
  }
  function domHop(el, wantIndex) {
    let value = el.tagName.toLowerCase();
    if (el.id) value += '#' + el.id;
    const className = el.className;
    if (className && typeof className === 'string') {
      for (const cls of className.split(' ').filter(Boolean).slice(0, 2)) value += '.' + trimText(cls, 20);
    }
    const screen = el.getAttribute && el.getAttribute('data-screen-label');
    if (screen) value += '[screen="' + trimText(screen, 24) + '"]';
    if (wantIndex && el.parentElement && el.parentElement.children.length > 1) {
      value += '[' + (Array.from(el.parentElement.children).indexOf(el) + 1) + '/' + el.parentElement.children.length + ']';
    }
    return value;
  }
  function clampPath(text, sep, limit) {
    if (text.length <= limit) return text;
    const parts = text.split(sep);
    const head = [parts[0]];
    const tail = [parts[parts.length - 1]];
    let len = head[0].length + tail[0].length + sep.length + 3;
    let hi = 1;
    let ti = parts.length - 2;
    while (hi <= ti) {
      const nextTail = parts[ti];
      if (len + sep.length + nextTail.length <= limit) {
        tail.unshift(nextTail);
        len += sep.length + nextTail.length;
        ti -= 1;
        continue;
      }
      const nextHead = parts[hi];
      if (len + sep.length + nextHead.length <= limit) {
        head.push(nextHead);
        len += sep.length + nextHead.length;
        hi += 1;
        continue;
      }
      break;
    }
    return head.concat('...', tail).join(sep);
  }
  function richDescriptor(el, selector) {
    const sep = ' > ';
    const reactPath = [];
    for (let node = el; node && node.nodeType === 1 && node !== document.documentElement; node = node.parentElement) {
      const name = reactName(node);
      if (name && name !== reactPath[0]) reactPath.unshift(name);
    }
    const domPath = [];
    for (let node = el; node && node.nodeType === 1 && node !== document.documentElement; node = node.parentElement) domPath.unshift(domHop(node, node === el));
    const textBits = [];
    const text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
    if (text) textBits.push('"' + trimText(text, 60) + '"');
    const aria = el.getAttribute('aria-label');
    if (aria) textBits.push('aria-label: "' + trimText(aria, 40) + '"');
    const alt = [];
    if (el.getAttribute('alt')) alt.push(el.getAttribute('alt'));
    for (const img of Array.from(el.querySelectorAll('img[alt]')).slice(0, 3)) alt.push(img.getAttribute('alt'));
    if (alt.length) textBits.push('alt: "' + trimText(alt.join(' | '), 40) + '"');
    const children = [];
    for (const child of Array.from(el.childNodes)) {
      if (child.nodeType === 1) children.push(child.tagName.toLowerCase());
      else if (child.nodeType === 3 && child.textContent.trim()) children.push('text');
    }
    const lines = ['<mentioned-element>'];
    if (reactPath.length) lines.push('react:    ' + trimText(reactPath.join(sep), 100));
    lines.push('dom:      ' + clampPath(domPath.join(sep), sep, 100));
    if (textBits.length) lines.push('text:     ' + trimText(textBits.join(' | '), 100));
    if (children.length) lines.push('children: ' + trimText(children.join(', '), 100));
    if (selector) lines.push('selector: ' + trimText(selector, 100));
    lines.push('id:       dm-' + getRef(el));
    lines.push('</mentioned-element>');
    return lines.join('\n');
  }
  function shortLabel(el) {
    const tag = el.tagName.toLowerCase();
    if (el.id) return tag + '#' + el.id;
    const className = el.className;
    if (className && typeof className === 'string' && className.trim()) return tag + '.' + trimText(className.split(/\s+/)[0], 14);
    const text = (el.innerText || el.textContent || '').trim();
    if (text) return tag + ' "' + trimText(text, 12) + '"';
    return tag;
  }
  function describe(el) {
    if (!el || el.nodeType !== 1) return null;
    const selector = selectorFor(el);
    return {
      ref: getRef(el),
      selector,
      selectors: rankedSelectorList(el),
      tag: el.tagName.toLowerCase(),
      text: (el.innerText || el.textContent || '').slice(0, 4000),
      rect: rectOf(el),
      styles: readStyles(el),
      inlineStyles: readInlineStyles(el),
      attributes: Object.fromEntries(Array.from(el.attributes || []).map((attr) => [attr.name, attr.value])),
      descriptor: richDescriptor(el, selector),
      source: sourceHints(el)
    };
  }
  function inspect(target) {
    const el = typeof target === 'number' ? byRef(target) : typeof target === 'string' ? document.querySelector(target) : target;
    const detail = describe(el);
    emit('om:inspect', detail || { ok: false });
    return detail;
  }

  async function applyEdit(edit) {
    const selector = edit && (edit.selector || edit.sourceSelector);
    if (!selector) throw new Error('selector is required');
    const el = document.querySelector(selector);
    if (el) {
      if (edit.text !== undefined) el.textContent = String(edit.text);
      if (edit.html !== undefined) el.innerHTML = String(edit.html);
      if (edit.attributes) for (const [key, value] of Object.entries(edit.attributes)) value == null ? el.removeAttribute(key) : el.setAttribute(key, String(value));
      if (edit.styles && el.style) for (const [key, value] of Object.entries(edit.styles)) value == null ? el.style.removeProperty(key) : el.style.setProperty(key, String(value));
      if (selected && selected.el === el && selectedOverlay) updateOverlay(selectedOverlay, el);
    }
    if (!config.public && config.projectId) {
      await fetch(api('/tools/direct-edit-apply'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ path: config.path, fallback_overlay: true, ...edit, selector })
      }).catch(() => null);
    }
    emit('om:edit-applied', { selector, local: Boolean(el) });
  }
  async function getTweaks() {
    if (config.public || !config.projectId) return {};
    const response = await fetch(api('/tools/tweaks')).catch(() => null);
    return response && response.ok ? (await response.json()).values : {};
  }
  async function setTweaks(values) {
    if (!config.public && config.projectId) {
      await fetch(api('/tools/tweaks'), {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ values })
      }).catch(() => null);
    }
    emit('om:tweaks', { values });
  }
  async function dcUpdate(name, kind, content, streaming) {
    if (!config.public && config.projectId) {
      await fetch(api('/tools/dc-write'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ path: config.path, name, kind, content, streaming: streaming !== false })
      }).catch(() => null);
    }
    emit('om:dc-update', { name, kind, content, streaming: streaming !== false });
  }
  let dcStreamSource = null;
  let dcStreamBuffer = '';
  function dcHtmlStrReplace(target, search, replacement) {
    if (arguments.length === 1) {
      dcStreamBuffer = String(target ?? '');
      document.body.innerHTML = dcStreamBuffer;
      emit('om:dc-html-str-replace', { target: 'body', bytes: dcStreamBuffer.length });
      return dcStreamBuffer;
    }
    const el = document.querySelector(String(target || 'body')) || document.body;
    if (arguments.length === 2) {
      dcStreamBuffer = String(search ?? '');
      el.innerHTML = dcStreamBuffer;
      emit('om:dc-html-str-replace', { target, bytes: dcStreamBuffer.length });
      return dcStreamBuffer;
    }
    const current = el.innerHTML;
    const next = current.split(String(search ?? '')).join(String(replacement ?? ''));
    el.innerHTML = next;
    dcStreamBuffer = next;
    emit('om:dc-html-str-replace', { target, bytes: next.length });
    return next;
  }
  function dcAppend(content) {
    dcStreamBuffer += String(content ?? '');
    return dcHtmlStrReplace(dcStreamBuffer);
  }
  function dcReplace(content) {
    dcStreamBuffer = String(content ?? '');
    return dcHtmlStrReplace(dcStreamBuffer);
  }
  function startDcStream() {
    if (config.public || !config.projectId || !String(config.path || '').endsWith('.dc.html') || !window.EventSource) return null;
    if (dcStreamSource) return dcStreamSource;
    try {
      dcStreamSource = new EventSource(api('/tools/dc-stream?path=' + encodeURIComponent(config.path)));
      dcStreamSource.addEventListener('dc_html_str_replace', (event) => {
        try {
          const data = JSON.parse(event.data || '{}');
          if (!data.append) dcStreamBuffer = '';
          dcStreamBuffer += String(data.content || '');
          dcHtmlStrReplace(dcStreamBuffer);
        } catch {}
      });
      dcStreamSource.addEventListener('dc_done', () => {
        emit('om:dc-stream-done', { path: config.path, bytes: dcStreamBuffer.length });
      });
      dcStreamSource.onerror = () => emit('om:dc-stream-error', { path: config.path });
    } catch {
      dcStreamSource = null;
    }
    return dcStreamSource;
  }

  function setMode(next) {
    window.__omMode = Object.assign({ edit: false, comment: false, hoverTrack: false, structuralEdits: false }, window.__omMode || {}, next || {});
    window.__editModeActive = !!window.__omMode.edit;
    window.__commentModeActive = !!window.__omMode.comment;
    window.__hoverTrackActive = !!window.__omMode.hoverTrack;
    window.__structuralEditsEnabled = !!window.__omMode.structuralEdits;
    if (!window.__editModeActive && !window.__commentModeActive) {
      if (hoverOverlay) hoverOverlay.style.display = 'none';
      if (selectedOverlay) selectedOverlay.style.display = 'none';
      finishTextEdit(false);
    }
  }
  function pushCursor(cursor) {
    cursorStack.push(cursor);
    document.body.style.cursor = cursor;
  }
  function popCursor() {
    cursorStack.pop();
    document.body.style.cursor = cursorStack[cursorStack.length - 1] || '';
  }
  function setOverrideCursor(cursor) {
    if (!cursor) {
      if (cursorStyle) cursorStyle.textContent = '';
      return;
    }
    if (!cursorStyle) {
      cursorStyle = document.createElement('style');
      cursorStyle.setAttribute(OVERLAY_ATTR, '1');
      document.head.appendChild(cursorStyle);
    }
    cursorStyle.textContent = '*{cursor:' + cursor + ' !important;user-select:none !important;-webkit-user-select:none !important}';
  }
  function enterEditMode() {
    if (!window.__editModeActive) pushCursor('crosshair');
    setMode({ edit: true });
  }
  function exitEditMode() {
    if (window.__editModeActive) popCursor();
    setMode({ edit: false });
    selected = null;
    hideOverlays();
    setOverrideCursor(null);
  }
  function selectElement(el, click, asText) {
    if (!el || el.closest(CHROME_SELECTOR)) return null;
    const payload = selectionPayload(el, click);
    selected = { el, selector: payload.selector };
    if (el !== document.body) {
      if (!selectedOverlay) selectedOverlay = makeOverlay('#8B5CF6');
      updateOverlay(selectedOverlay, el);
    } else if (selectedOverlay) {
      selectedOverlay.style.display = 'none';
    }
    if (asText) startTextEdit(el, click);
    emit('om:selected', payload);
    emit('om:selection-changed', Object.assign({ isText: !!asText, originalText: asText ? (el.innerText || el.textContent || '') : null, originalInnerHTML: asText ? el.innerHTML : null }, payload));
    return payload;
  }
  function startTextEdit(el, click) {
    if (!isTextEditable(el)) return;
    if (textEditElement && textEditElement !== el) finishTextEdit(false);
    textEditElement = el;
    window.__textEditCurrentElement = el;
    el.__jfcOriginalHtml = el.innerHTML;
    el.contentEditable = 'true';
    el.style.outline = '2px solid #8B5CF6';
    el.style.outlineOffset = '2px';
    const selection = window.getSelection();
    if (selection && click && document.caretRangeFromPoint) {
      const range = document.caretRangeFromPoint(click.x, click.y);
      if (range && el.contains(range.startContainer)) {
        selection.removeAllRanges();
        selection.addRange(range);
      }
    }
  }
  function finishTextEdit(revert) {
    const el = textEditElement;
    if (!el) return;
    if (revert && el.__jfcOriginalHtml != null) el.innerHTML = el.__jfcOriginalHtml;
    el.removeAttribute('contenteditable');
    el.style.outline = '';
    el.style.outlineOffset = '';
    window.__textEditCurrentElement = null;
    textEditElement = null;
    if (!revert) emit('om:edit-action', { action: 'submit', selector: selectorFor(el), html: el.innerHTML, text: el.innerText || el.textContent || '' });
  }
  function isTextEditable(el) {
    if (!el || VOID_TAGS.has(el.tagName.toLowerCase())) return false;
    if (FORM_TAGS.has(el.tagName.toLowerCase())) return false;
    if (el.children.length === 0) return true;
    return TEXT_TAGS.has(el.tagName.toLowerCase());
  }
  function hoverMove(event) {
    if ((!window.__editModeActive && !window.__commentModeActive && !window.__hoverTrackActive) || dragging) return;
    let el = preciseHit(event.clientX, event.clientY);
    if (!el || el.closest(CHROME_SELECTOR) || el === document.body || el === document.documentElement) {
      if (hoverOverlay) hoverOverlay.style.display = 'none';
      return;
    }
    if (window.__structuralEditsEnabled) el = el.closest('[data-om-id]') || el;
    if (selected && selected.el === el) return;
    if (!hoverOverlay) hoverOverlay = makeOverlay(window.__commentModeActive ? '#D97757' : '#10B981');
    updateOverlay(hoverOverlay, el);
    emit('om:hovered', selectionPayload(el, { x: event.clientX, y: event.clientY }));
  }
  function clickSelect(event) {
    if (!window.__editModeActive && !window.__commentModeActive) return;
    if (isChromeEvent(event)) return;
    if (dragging && dragging.completed) {
      dragging = null;
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const el = preciseHit(event.clientX, event.clientY);
    if (!el) return;
    const same = selected && selected.el === el;
    selectElement(el, { x: event.clientX, y: event.clientY }, window.__editModeActive && same && isTextEditable(el));
  }
  function keyHandler(event) {
    if (event.key === 'Escape' && (window.__editModeActive || window.__commentModeActive)) {
      if (textEditElement) {
        event.preventDefault();
        finishTextEdit(true);
        if (selected) selectElement(selected.el, null, false);
      } else {
        try { window.parent && window.parent.postMessage({ type: 'escape-pressed' }, '*'); } catch {}
        exitEditMode();
      }
      return;
    }
    if (!window.__editModeActive) return;
    if (textEditElement) {
      if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        finishTextEdit(false);
      }
      return;
    }
    if (event.key === 'Enter' && selected && isTextEditable(selected.el)) {
      event.preventDefault();
      startTextEdit(selected.el, null);
    }
  }
  function selectionChanged() {
    const el = textEditElement;
    const selection = window.getSelection();
    if (!el || !selection || selection.isCollapsed || selection.rangeCount === 0 || !el.contains(selection.anchorNode) || !el.contains(selection.focusNode)) {
      window.__textEditLastRange = null;
      emit('om:range-style', { style: null });
      return;
    }
    const range = selection.getRangeAt(0);
    window.__textEditLastRange = range.cloneRange();
    let node = range.commonAncestorContainer;
    if (node.nodeType !== 1) node = node.parentElement;
    const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
    const colors = new Set();
    let textNode;
    while ((textNode = walker.nextNode())) {
      if (!textNode.data.trim() || !range.intersectsNode(textNode)) continue;
      colors.add(getComputedStyle(textNode.parentElement).color);
    }
    emit('om:range-style', { style: { color: colors.size === 1 ? Array.from(colors)[0] : null } });
  }

  function eligibleChildren(container, moving) {
    return Array.from(container.children || []).filter((child) => child !== moving && !child.hasAttribute(OVERLAY_ATTR) && !(child.classList && child.classList.contains('__om-t')));
  }
  function dropTargetAt(x, y, moving) {
    let container = preciseHit(x, y);
    if (!container) return null;
    container = container.closest('[data-om-id]') || container;
    if (container === moving || moving.contains(container)) container = moving.parentElement;
    if (!container) return null;
    const children = eligibleChildren(container, moving);
    let index = children.length;
    let before = null;
    for (let i = 0; i < children.length; i += 1) {
      const rect = children[i].getBoundingClientRect();
      const vertical = rect.height >= rect.width;
      const mid = vertical ? rect.top + rect.height / 2 : rect.left + rect.width / 2;
      const pos = vertical ? y : x;
      if (pos < mid) {
        index = i;
        before = children[i];
        break;
      }
    }
    return { container, point: { index, before } };
  }
  function startDrag(event) {
    if (event.button !== 0 || !window.__editModeActive || !window.__structuralEditsEnabled || textEditElement || isChromeEvent(event)) return;
    const hit = preciseHit(event.clientX, event.clientY);
    const el = hit && (hit.closest('[data-om-id]') || hit);
    if (!el || el === document.body || el === document.documentElement || FORM_TAGS.has(el.tagName.toLowerCase())) return;
    const rect = el.getBoundingClientRect();
    dragging = {
      el,
      startX: event.clientX,
      startY: event.clientY,
      grabX: event.clientX - rect.left,
      grabY: event.clientY - rect.top,
      active: false,
      completed: false,
      origParent: el.parentElement,
      origNext: el.nextSibling,
      origDisplay: el.style.display || '',
      origOpacity: el.style.opacity || '',
      origPointerEvents: el.style.pointerEvents || '',
      target: null,
      ghost: null
    };
  }
  function dragMove(event) {
    if (!dragging) return;
    if (event.buttons === 0) {
      cancelDrag(true);
      return;
    }
    const dx = event.clientX - dragging.startX;
    const dy = event.clientY - dragging.startY;
    if (!dragging.active && dx * dx + dy * dy < 64) return;
    event.preventDefault();
    if (!dragging.active) {
      dragging.active = true;
      if (hoverOverlay) hoverOverlay.style.display = 'none';
      if (selectedOverlay) selectedOverlay.style.display = 'none';
      const rect = dragging.el.getBoundingClientRect();
      const ghost = dragging.el.cloneNode(true);
      ghost.removeAttribute('data-om-id');
      for (const child of ghost.querySelectorAll('[data-om-id],[data-src-loc]')) {
        child.removeAttribute('data-om-id');
        child.removeAttribute('data-src-loc');
      }
      ghost.setAttribute(OVERLAY_ATTR, '1');
      ghost.style.cssText = 'position:fixed;left:0;top:0;margin:0;width:' + rect.width + 'px;pointer-events:none;transition:none;opacity:.65;z-index:100001;';
      document.body.appendChild(ghost);
      dragging.ghost = ghost;
      dragging.el.style.opacity = '0.35';
      dragging.el.style.pointerEvents = 'none';
      setOverrideCursor('grabbing');
    }
    if (dragging.ghost) dragging.ghost.style.transform = 'translate(' + (event.clientX - dragging.grabX) + 'px,' + (event.clientY - dragging.grabY) + 'px)';
    const oldDisplay = dragging.el.style.display;
    dragging.el.style.display = 'none';
    const target = dropTargetAt(event.clientX, event.clientY, dragging.el);
    dragging.el.style.display = oldDisplay;
    if (!target) return;
    dragging.target = target;
    if (!dropOverlay) dropOverlay = makeOverlay('#2563EB', true);
    updateOverlay(dropOverlay, target.container);
  }
  function finishDrag() {
    if (!dragging) return;
    const state = dragging;
    dragging = null;
    setOverrideCursor(null);
    if (state.ghost) state.ghost.remove();
    state.el.style.opacity = state.origOpacity;
    state.el.style.pointerEvents = state.origPointerEvents;
    state.el.style.display = state.origDisplay;
    if (dropOverlay) dropOverlay.style.display = 'none';
    if (!state.active || !state.target) return;
    const { container, point } = state.target;
    try {
      if (point.before && point.before.parentElement === container) container.insertBefore(state.el, point.before);
      else container.appendChild(state.el);
    } catch {}
    state.completed = true;
    if (selectedOverlay) updateOverlay(selectedOverlay, state.el);
    emit('om:drag-drop', {
      sourceSelector: selectorFor(state.el),
      sourceOmId: state.el.getAttribute('data-om-id'),
      sourceDescriptor: richDescriptor(state.el, selectorFor(state.el)),
      parentSelector: selectorFor(container),
      parentOmId: container.getAttribute('data-om-id'),
      parentDescriptor: richDescriptor(container, selectorFor(container)),
      index: point.index,
      siblingOmId: point.before ? point.before.getAttribute('data-om-id') : null,
      siblingSelector: point.before ? selectorFor(point.before) : null,
      sameParent: container === state.origParent
    });
  }
  function cancelDrag(revert) {
    if (!dragging) return;
    const state = dragging;
    dragging = null;
    setOverrideCursor(null);
    if (state.ghost) state.ghost.remove();
    state.el.style.opacity = state.origOpacity;
    state.el.style.pointerEvents = state.origPointerEvents;
    state.el.style.display = state.origDisplay;
    if (revert && state.origParent) {
      try {
        if (state.origNext && state.origNext.parentElement === state.origParent) state.origParent.insertBefore(state.el, state.origNext);
        else state.origParent.appendChild(state.el);
      } catch {}
    }
    if (dropOverlay) dropOverlay.style.display = 'none';
  }

  function serializeDocument() {
    const removeNodes = Array.from(document.querySelectorAll('[' + OVERLAY_ATTR + '],[data-designer-retranspiled],[data-omelette-injected],#__om_srcmap,#deck-stage-print-page'));
    const transientAttrs = ['data-om-id', 'data-om-path', 'data-om-line', 'data-om-column', 'data-src-ver', 'data-cc-id', 'data-dm-ref', 'data-om-ref', 'data-deck-active', 'data-deck-slide', 'data-screen-label', 'data-om-validate', 'data-editable', 'data-filled', 'data-over', 'data-panning', 'data-reframe'];
    const attrNodes = transientAttrs.length ? Array.from(document.querySelectorAll(transientAttrs.map((name) => '[' + name + ']').join(','))) : [];
    const savedAttrs = attrNodes.map((node) => transientAttrs.map((name) => {
      const value = node.getAttribute(name);
      if (value != null) node.removeAttribute(name);
      return value;
    }));
    const cursor = document.body.style.cursor;
    document.body.style.cursor = '';
    const savedLocations = removeNodes.map((node) => ({ node, parent: node.parentNode, next: node.nextSibling }));
    for (const node of removeNodes) node.remove();
    const html = '<!DOCTYPE html>' + document.documentElement.outerHTML;
    document.body.style.cursor = cursor;
    for (let i = savedLocations.length - 1; i >= 0; i -= 1) {
      const saved = savedLocations[i];
      if (saved.parent) saved.parent.insertBefore(saved.node, saved.next);
    }
    for (let i = 0; i < attrNodes.length; i += 1) {
      for (let j = 0; j < transientAttrs.length; j += 1) {
        if (savedAttrs[i][j] != null) attrNodes[i].setAttribute(transientAttrs[j], savedAttrs[i][j]);
      }
    }
    return html;
  }
  function serializeForSave() {
    const el = textEditElement;
    if (el) {
      el.removeAttribute('contenteditable');
      const outline = el.style.outline;
      const offset = el.style.outlineOffset;
      el.style.outline = '';
      el.style.outlineOffset = '';
      const html = serializeDocument();
      el.contentEditable = 'true';
      el.style.outline = outline;
      el.style.outlineOffset = offset;
      return html;
    }
    return serializeDocument();
  }
  function documentColors() {
    const seen = new Set();
    const out = [];
    for (const el of Array.from(document.body.querySelectorAll('*')).slice(0, 500)) {
      const style = getComputedStyle(el);
      for (const key of ['color', 'backgroundColor', 'borderColor', 'fill', 'stroke']) {
        const value = style[key];
        if (!value || value === 'none' || value.includes('rgba(0, 0, 0, 0)')) continue;
        if ((value.startsWith('rgb') || value.startsWith('#')) && !seen.has(value)) {
          seen.add(value);
          out.push(value);
          if (out.length >= 24) return out;
        }
      }
    }
    return out;
  }
  function pageFonts() {
    const fonts = new Set();
    for (const el of document.querySelectorAll('*')) {
      const value = getComputedStyle(el).fontFamily;
      if (!value) continue;
      for (const font of value.split(',')) {
        const clean = font.trim().replace(/['"]/g, '');
        if (clean && clean !== 'inherit' && clean !== 'initial') fonts.add(clean);
      }
      if (fonts.size >= 20) break;
    }
    return Array.from(fonts).slice(0, 20);
  }
  function readSrcMap() {
    const el = document.getElementById('__om_srcmap');
    return el ? el.textContent : null;
  }
  function probePage() {
    const hasBabel = !!(window.Babel && document.querySelector('script[type="text/babel"],script[type="text/jsx"]'));
    let hasGeneratorScript = false;
    for (const script of document.querySelectorAll('script:not([data-omelette-injected]):not([type="text/babel"]):not([type="text/jsx"])')) {
      const type = (script.getAttribute('type') || '').toLowerCase();
      if (type && type !== 'module' && !/javascript|ecmascript/.test(type)) continue;
      if (script.src && /^(https?:)?\/\//.test(script.getAttribute('src') || '')) continue;
      if (script.src || (script.textContent || '').trim()) {
        hasGeneratorScript = true;
        break;
      }
    }
    return {
      isBabel: hasBabel,
      hasGeneratorScript,
      starterKinds: ['deck-stage', 'image-slot'].filter((name) => document.querySelector(name))
    };
  }
  function dispatchKey(init) {
    document.dispatchEvent(new KeyboardEvent('keydown', init || {}));
  }
  function presentMode(active) {
    document.__omPresentActive = active ? 1 : 0;
  }
  function innerText(selector) {
    const el = document.querySelector(selector);
    return el ? (el.innerText || el.textContent || null) : null;
  }
  function removeElement(selector) {
    const el = document.querySelector(selector);
    if (el) el.remove();
    hideOverlays();
  }
  function moveLive(sourceSelector, parentSelector, index) {
    const el = document.querySelector(sourceSelector);
    const parent = document.querySelector(parentSelector);
    if (!el || !parent) return;
    const children = eligibleChildren(parent, el);
    const before = children[index] || null;
    if (before) parent.insertBefore(el, before);
    else parent.appendChild(el);
    hideOverlays();
  }
  function adjacentDrop(sourceSelector, direction) {
    const el = document.querySelector(sourceSelector);
    if (!el || !el.parentElement) return null;
    const siblings = eligibleChildren(el.parentElement, el);
    const current = siblings.filter((node) => {
      const pos = Array.from(el.parentElement.children).indexOf(node);
      return pos < Array.from(el.parentElement.children).indexOf(el);
    }).length;
    const index = direction === 'prev' ? current - 1 : current + 1;
    if (index < 0 || index > siblings.length) return null;
    const before = siblings[index] || null;
    return {
      sourceSelector: selectorFor(el),
      sourceOmId: el.getAttribute('data-om-id'),
      sourceDescriptor: richDescriptor(el, selectorFor(el)),
      parentSelector: selectorFor(el.parentElement),
      parentOmId: el.parentElement.getAttribute('data-om-id'),
      parentDescriptor: richDescriptor(el.parentElement, selectorFor(el.parentElement)),
      index,
      siblingOmId: before ? before.getAttribute('data-om-id') : null,
      siblingSelector: before ? selectorFor(before) : null,
      sameParent: true
    };
  }

  function initListeners() {
    markReactSourceElements();
    startDcStream();
    document.addEventListener('mousemove', hoverMove, true);
    document.addEventListener('click', clickSelect, true);
    document.addEventListener('keydown', keyHandler, true);
    document.addEventListener('selectionchange', selectionChanged);
    document.addEventListener('mousedown', startDrag, true);
    document.addEventListener('mousemove', dragMove, true);
    document.addEventListener('mouseup', finishDrag, true);
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && dragging) {
        event.preventDefault();
        event.stopPropagation();
        cancelDrag(true);
      }
    }, true);
    window.addEventListener('scroll', () => {
      if (selected && selectedOverlay) updateOverlay(selectedOverlay, selected.el);
      if (hoverOverlay && hoverOverlay.__el) updateOverlay(hoverOverlay, hoverOverlay.__el);
    }, true);
    window.addEventListener('resize', () => {
      markReactSourceElements();
      if (selected && selectedOverlay) updateOverlay(selectedOverlay, selected.el);
    });
  }

  window.__generateSelectorList = rankedSelectorList;
  window.__OM_EVT__ = legacyConsoleEvent;
  window.__DM_EVT__ = legacyConsoleEvent;
  window.__dcUpdate = dcUpdate;
  window.dc_html_str_replace = dcHtmlStrReplace;
  window.__dcAppend = dcAppend;
  window.__dcReplace = dcReplace;
  window.__dcStartStream = startDcStream;
  window.__om = {
    __jfc: true,
    config,
    emit,
    getRef,
    byRef,
    describe,
    describeEl: (selector) => {
      const el = document.querySelector(selector);
      return el ? richDescriptor(el, selector) : '';
    },
    inspect,
    applyEdit,
    directEdit: { inspect, apply: applyEdit },
    tweaks: { get: getTweaks, set: setTweaks },
    setMode,
    enterEditMode,
    exitEditMode,
    setStructuralEditsEnabled: (enabled) => setMode({ structuralEdits: !!enabled }),
    textEditFinish: () => finishTextEdit(false),
    textEditRevert: (html) => {
      if (textEditElement && typeof html === 'string') textEditElement.innerHTML = html;
      finishTextEdit(true);
    },
    serializeDocument,
    serializeForSave,
    getDocumentColors: documentColors,
    getPageFonts: pageFonts,
    readSrcMap,
    probePage,
    dispatchKey,
    setPresentMode: presentMode,
    setOverrideCursor,
    removeElement,
    innerText,
    moveLive,
    adjacentDrop,
    dcHtmlStrReplace,
    dcAppend,
    dcReplace,
    startDcStream
  };
  window.__DM = Object.assign(window.__DM || {}, window.__om, {
    previewSelector: (selector) => emit('dm:preview-selector', { selector, matches: Array.from(document.querySelectorAll(selector)).map(describe) }),
    selectSimilar: (selector) => emit('dm:select-similar', { selector, matches: Array.from(document.querySelectorAll(selector)).map(describe) })
  });
  window.addEventListener('message', (event) => {
    const msg = event.data && (event.data.__OM_MSG__ || event.data.__DM_MSG__ || event.data);
    if (!msg || typeof msg !== 'object') return;
    const type = msg.type || msg.kind;
    const detail = msg.detail || msg;
    if (type === 'om:ping' || type === 'dm:ping') emit('om:pong', { path: config.path });
    else if (type === 'om:inspect' || type === 'dm:inspect') inspect(detail.selector || detail.ref);
    else if (type === 'om:apply-edit' || type === 'dm:apply-edit') void applyEdit(detail);
    else if (type === 'om:set-mode') setMode(detail);
    else if (type === 'om:enter-edit-mode') enterEditMode();
    else if (type === 'om:exit-edit-mode') exitEditMode();
    else if (type === 'om:tweaks:set') void setTweaks(detail.values || detail);
    else if (type === 'om:dc-write') void dcUpdate(detail.name, detail.kind, detail.content, detail.streaming);
  });

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initListeners, { once: true });
  } else {
    initListeners();
  }
  emit('om:ready', { path: config.path, public: !!config.public, sourceMarked: true });
})();
