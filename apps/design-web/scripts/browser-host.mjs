import crypto from 'node:crypto';
import { readFileSync } from 'node:fs';
import fs from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

import pptxgen from 'pptxgenjs';
import { chromium } from 'playwright';

const DEFAULT_VIEWPORT = { width: 1440, height: 900 };
const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_SLIDE_SELECTOR = '[data-slide], .slide, section';
const PPTX_WIDE = { w: 13.333, h: 7.5 };
const PREVIEW_RUNTIME_SOURCE = readFileSync(
  new URL('../../../crates/jfc-design/assets/preview-runtime.js', import.meta.url),
  'utf8'
).replace('__JFC_DESIGN_CONFIG__', '{}');

async function main() {
  const command = process.argv[2];
  const request = JSON.parse(await readStdin());

  if (command === 'eval-js') {
    await writeJson(await evalJs(request));
    return;
  }
  if (command === 'screenshot') {
    await writeJson(await screenshot(request));
    return;
  }
  if (command === 'multi-screenshot') {
    await writeJson(await multiScreenshot(request));
    return;
  }
  if (command === 'gen-pptx') {
    await writeJson(await genPptx(request));
    return;
  }
  if (command === 'direct-edit-inspect') {
    await writeJson(await directEditInspect(request));
    return;
  }
  if (command === 'verify') {
    await writeJson(await verifyArtifact(request));
    return;
  }
  if (command === 'print-pdf') {
    await writeJson(await printPdf(request));
    return;
  }

  throw new Error(`unknown browser-host command: ${command ?? '<missing>'}`);
}

async function evalJs(request) {
  const started = Date.now();
  const { browser, page, logs, errors } = await openPage(request);
  try {
    const result = await page.evaluate(async (source) => {
      const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
      const serialize = (value) => {
        if (value === undefined) return { kind: 'undefined' };
        if (value === null) return { kind: 'json', value: null };
        if (typeof Node !== 'undefined' && value instanceof Node) {
          return {
            kind: 'node',
            value: {
              nodeType: value.nodeType,
              nodeName: value.nodeName,
              text: value.textContent?.slice(0, 2000) ?? ''
            }
          };
        }
        try {
          return { kind: 'json', value: JSON.parse(JSON.stringify(value)) };
        } catch {
          return { kind: 'string', value: String(value) };
        }
      };

      try {
        return serialize(await (0, eval)(source));
      } catch (expressionError) {
        try {
          return serialize(await new AsyncFunction(source)());
        } catch (statementError) {
          return {
            kind: 'error',
            value: {
              expression: String(expressionError?.message ?? expressionError),
              statement: String(statementError?.message ?? statementError)
            }
          };
        }
      }
    }, request.script ?? '');

    return {
      ok: result.kind !== 'error',
      url: page.url(),
      title: await page.title(),
      result,
      logs,
      errors,
      duration_ms: Date.now() - started
    };
  } finally {
    await browser.close();
  }
}

async function screenshot(request) {
  const started = Date.now();
  const { browser, page, logs, errors } = await openPage(request);
  try {
    const target = request.selector ? page.locator(request.selector).first() : page;
    const buffer = await target.screenshot({
      type: 'png',
      fullPage: request.selector ? undefined : request.full_page !== false
    });
    if (request.output) await writeBuffer(request.output, buffer);
    return {
      ok: true,
      output: request.output ?? null,
      bytes: buffer.length,
      data_base64: buffer.toString('base64'),
      logs,
      errors,
      duration_ms: Date.now() - started
    };
  } finally {
    await browser.close();
  }
}

async function multiScreenshot(request) {
  const started = Date.now();
  const { browser, page, logs, errors } = await openPage(request);
  try {
    const selector = request.selector || DEFAULT_SLIDE_SELECTOR;
    const handles = await slideHandles(page, selector, request.max_items ?? request.max_slides ?? 40);
    const screenshots = [];

    for (let index = 0; index < handles.length; index += 1) {
      const handle = handles[index];
      await handle.scrollIntoViewIfNeeded().catch(() => undefined);
      await page.waitForTimeout(request.item_wait_ms ?? 50);
      const buffer = await handle.screenshot({ type: 'png' });
      const output = request.output_dir
        ? path.join(request.output_dir, `capture-${String(index + 1).padStart(2, '0')}.png`)
        : null;
      if (output) await writeBuffer(output, buffer);
      screenshots.push({
        index,
        output,
        bytes: buffer.length,
        hash: sha1(buffer),
        data_base64: request.include_data === false ? null : buffer.toString('base64')
      });
    }

    return {
      ok: screenshots.length > 0,
      output_dir: request.output_dir ?? null,
      selector,
      screenshots,
      logs,
      errors,
      duration_ms: Date.now() - started
    };
  } finally {
    await browser.close();
  }
}

async function genPptx(request) {
  const started = Date.now();
  const { browser, page, logs, errors } = await openPage(request);
  try {
    const mode = request.mode || 'editable';
    if (mode === 'screenshots') {
      return await genPptxScreenshots(request, page, logs, errors, started, []);
    }
    try {
      return await genPptxEditable(request, page, logs, errors, started);
    } catch (error) {
      if (request.fallback === false) throw error;
      return await genPptxScreenshots(request, page, logs, errors, started, [
        `editable_export_failed: ${String(error?.message ?? error)}`
      ]);
    }
  } finally {
    await browser.close();
  }
}

async function genPptxScreenshots(request, page, logs, errors, started, warnings) {
  const pptx = await makePptx(request, page);
  const selector = request.selector || DEFAULT_SLIDE_SELECTOR;
  const handles = await slideHandles(page, selector, request.max_slides ?? 40);
  const images = [];

  for (const handle of handles) {
    await handle.scrollIntoViewIfNeeded().catch(() => undefined);
    await page.waitForTimeout(request.slide_wait_ms ?? 50);
    images.push(await handle.screenshot({ type: 'png' }));
  }
  if (images.length === 0) {
    images.push(await page.screenshot({ type: 'png', fullPage: false }));
  }

  const duplicateWarnings = duplicateHashWarnings(images.map((image) => sha1(image)));
  for (const image of images) {
    const slide = pptx.addSlide();
    slide.background = { color: 'FFFFFF' };
    slide.addImage({
      data: `data:image/png;base64,${image.toString('base64')}`,
      x: 0,
      y: 0,
      w: PPTX_WIDE.w,
      h: PPTX_WIDE.h
    });
  }

  const stat = await writePptx(request, pptx);
  return {
    ok: true,
    output: request.output,
    bytes: stat.size,
    slides: images.length,
    mode: 'screenshots',
    warnings: [...warnings, ...duplicateWarnings],
    validation: {
      selector,
      editable_elements: 0,
      duplicate_hashes: duplicateWarnings.length
    },
    logs,
    errors,
    duration_ms: Date.now() - started
  };
}

async function genPptxEditable(request, page, logs, errors, started) {
  const pptx = await makePptx(request, page);
  const selector = request.selector || DEFAULT_SLIDE_SELECTOR;
  const warnings = [];
  const fontsReady = await waitForFonts(page, 8000);
  if (!fontsReady) warnings.push('fonts_timeout');

  const handles = await slideHandles(page, selector, request.max_slides ?? 40);
  const models = [];
  for (const handle of handles) {
    await handle.scrollIntoViewIfNeeded().catch(() => undefined);
    await page.waitForTimeout(request.slide_wait_ms ?? 75);
    const model = await handle.evaluate(captureEditableSlide);
    models.push(model);
    warnings.push(...model.warnings);
  }
  if (models.length === 0) {
    const body = await page.locator('body').elementHandle();
    if (body) models.push(await body.evaluate(captureEditableSlide));
  }
  if (models.length === 0) throw new Error('no visible slide roots found');

  const notes = await readSpeakerNotes(page);
  let editableElements = 0;
  const modelHashes = [];
  for (let index = 0; index < models.length; index += 1) {
    const model = models[index];
    editableElements += model.items.length;
    modelHashes.push(sha1(JSON.stringify(model.items)));
    renderEditableSlide(pptx, model, notes[index]);
  }
  warnings.push(...duplicateHashWarnings(modelHashes));
  if (notes.length && notes.length !== models.length) warnings.push('notes_count_mismatch');

  const stat = await writePptx(request, pptx);
  return {
    ok: true,
    output: request.output,
    bytes: stat.size,
    slides: models.length,
    mode: 'editable',
    warnings: uniqueStrings(warnings),
    validation: {
      selector,
      editable_elements: editableElements,
      notes: notes.length,
      duplicate_hashes: duplicateHashWarnings(modelHashes).length
    },
    logs,
    errors,
    duration_ms: Date.now() - started
  };
}

async function directEditInspect(request) {
  const started = Date.now();
  const { browser, page, logs, errors } = await openPage(request);
  try {
    const inspected = await page.evaluate(({ selector, x, y }) => {
      const target = selector
        ? document.querySelector(selector)
        : document.elementFromPoint(Number(x ?? 0), Number(y ?? 0));
      if (!target) {
        return { ok: false, error: 'element not found' };
      }
      if (window.__om?.describe) {
        const described = window.__om.describe(target);
        return {
          ok: true,
          ...described,
          selectors: Array.isArray(described?.selectors)
            ? described.selectors.map((item) => typeof item === 'string' ? item : item.selector).filter(Boolean)
            : []
        };
      }

      const style = getComputedStyle(target);
      const rect = target.getBoundingClientRect();
      const source = sourceHints(target);
      const selectors = stableSelectors(target);
      return {
        ok: true,
        selector: selectors[0] ?? selector ?? null,
        selectors,
        tag: target.tagName.toLowerCase(),
        text: target.textContent?.slice(0, 4000) ?? '',
        rect: {
          x: rect.x,
          y: rect.y,
          width: rect.width,
          height: rect.height
        },
        styles: {
          color: style.color,
          backgroundColor: style.backgroundColor,
          fontFamily: style.fontFamily,
          fontSize: style.fontSize,
          fontWeight: style.fontWeight,
          lineHeight: style.lineHeight,
          display: style.display,
          position: style.position
        },
        attributes: Object.fromEntries(
          Array.from(target.attributes ?? []).map((attr) => [attr.name, attr.value])
        ),
        source
      };

      function stableSelectors(element) {
        const out = [];
        const escaped = (value) => {
          if (window.CSS?.escape) return CSS.escape(value);
          return String(value).replace(/[^a-zA-Z0-9_-]/g, '\\$&');
        };
        if (element.id) out.push(`#${escaped(element.id)}`);
        for (const attr of ['data-om-id', 'data-testid', 'data-test-id', 'data-component', 'aria-label']) {
          const value = element.getAttribute(attr);
          if (value) out.push(`${element.tagName.toLowerCase()}[${attr}="${value.replace(/"/g, '\\"')}"]`);
        }
        const parts = [];
        let current = element;
        while (current && current.nodeType === Node.ELEMENT_NODE && current !== document.body) {
          let part = current.tagName.toLowerCase();
          const siblings = Array.from(current.parentElement?.children ?? []).filter(
            (sibling) => sibling.tagName === current.tagName
          );
          if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(current) + 1})`;
          parts.unshift(part);
          current = current.parentElement;
        }
        if (parts.length) out.push(parts.join(' > '));
        return Array.from(new Set(out)).slice(0, 8);
      }

      function sourceHints(element) {
        const attrs = {};
        for (const name of [
          'data-source',
          'data-src',
          'data-src-file',
          'data-src-line',
          'data-src-column',
          'data-om-source',
          'data-om-path',
          'data-om-line',
          'data-om-column',
          'data-om-start',
          'data-om-end',
          'data-source-start',
          'data-source-end',
          'data-loc'
        ]) {
          const value = element.getAttribute(name);
          if (value) attrs[name] = value;
        }

        let react = null;
        const reactKey = Object.keys(element).find(
          (key) => key.startsWith('__reactFiber$') || key.startsWith('__reactInternalInstance$')
        );
        let fiber = reactKey ? element[reactKey] : null;
        for (let depth = 0; fiber && depth < 12; depth += 1, fiber = fiber.return) {
          if (fiber?._debugSource) {
            react = fiber._debugSource;
            break;
          }
        }
        return { attributes: attrs, react };
      }
    }, request);

    return {
      ...inspected,
      url: page.url(),
      title: await page.title(),
      logs,
      errors,
      duration_ms: Date.now() - started
    };
  } finally {
    await browser.close();
  }
}

async function verifyArtifact(request) {
  const started = Date.now();
  const { browser, page, logs, errors } = await openPage(request);
  try {
    const selector = request.selector || DEFAULT_SLIDE_SELECTOR;
    const fontsReady = await waitForFonts(page, request.font_timeout_ms ?? 5000);
    const stats = await page.evaluate((slideSelector) => {
      const visibleElements = Array.from(document.querySelectorAll('body *')).filter((element) => {
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return (
          style.display !== 'none' &&
          style.visibility !== 'hidden' &&
          Number(style.opacity || 1) > 0 &&
          rect.width > 1 &&
          rect.height > 1
        );
      });
      const images = Array.from(document.images).map((img) => ({
        src: img.currentSrc || img.src,
        complete: img.complete,
        naturalWidth: img.naturalWidth,
        naturalHeight: img.naturalHeight
      }));
      const root = document.scrollingElement || document.documentElement;
      return {
        title: document.title,
        textLength: (document.body?.innerText ?? '').trim().length,
        visibleElements: visibleElements.length,
        slideCount: document.querySelectorAll(slideSelector).length,
        imageCount: images.length,
        failedImages: images.filter((img) => !img.complete || img.naturalWidth === 0).slice(0, 20),
        scrollWidth: root.scrollWidth,
        scrollHeight: root.scrollHeight,
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight,
        bodyBackground: getComputedStyle(document.body).backgroundColor
      };
    }, selector);

    const screenshotBuffer = await page.screenshot({ type: 'png', fullPage: false });
    if (request.output) await writeBuffer(request.output, screenshotBuffer);

    const slideImages = [];
    const handles = await slideHandles(page, selector, request.max_screenshots ?? 8);
    for (const handle of handles) {
      await handle.scrollIntoViewIfNeeded().catch(() => undefined);
      slideImages.push(await handle.screenshot({ type: 'png' }));
    }
    const duplicateWarnings = duplicateHashWarnings(slideImages.map((buffer) => sha1(buffer)));
    const checks = [
      check('page_loaded', true, page.url()),
      check('fonts_ready', fontsReady, fontsReady ? 'document fonts settled' : 'font loading timed out', 'warn'),
      check('visible_dom', stats.visibleElements > 0, `${stats.visibleElements} visible elements`),
      check('body_text', stats.textLength > 0, `${stats.textLength} text characters`, 'warn'),
      check('images_loaded', stats.failedImages.length === 0, `${stats.failedImages.length} failed images`, 'fail'),
      check(
        'viewport_overflow',
        stats.scrollWidth <= stats.viewportWidth + 2,
        `${stats.scrollWidth}px content in ${stats.viewportWidth}px viewport`,
        'warn'
      ),
      check(
        'duplicate_slides',
        duplicateWarnings.length === 0,
        duplicateWarnings.join(', ') || 'slide captures are distinct',
        'warn'
      ),
      check('console_errors', errors.length === 0, `${errors.length} browser errors`, 'warn')
    ];

    return {
      ok: checks.every((item) => item.status !== 'fail'),
      output: request.output ?? null,
      screenshot: {
        bytes: screenshotBuffer.length,
        hash: sha1(screenshotBuffer),
        data_base64: request.include_data === false ? null : screenshotBuffer.toString('base64')
      },
      stats,
      checks,
      warnings: checks.filter((item) => item.status === 'warn').map((item) => item.name),
      logs,
      errors,
      duration_ms: Date.now() - started
    };
  } finally {
    await browser.close();
  }
}

async function printPdf(request) {
  const started = Date.now();
  const { browser, page, logs, errors } = await openPage(request);
  try {
    if (!request.output) throw new Error('output is required for print-pdf');
    await fs.mkdir(path.dirname(request.output), { recursive: true });
    const buffer = await page.pdf({
      path: request.output,
      format: request.format || 'A4',
      landscape: !!request.landscape,
      printBackground: request.print_background !== false,
      preferCSSPageSize: request.prefer_css_page_size !== false,
      margin: request.margin || { top: '0.4in', right: '0.4in', bottom: '0.4in', left: '0.4in' }
    });
    const stat = await fs.stat(request.output);
    return {
      ok: true,
      output: request.output,
      bytes: stat.size || buffer.length,
      logs,
      errors,
      duration_ms: Date.now() - started
    };
  } finally {
    await browser.close();
  }
}

async function openPage(request) {
  const logs = [];
  const errors = [];
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({
      viewport: request.viewport || DEFAULT_VIEWPORT,
      deviceScaleFactor: request.device_scale_factor || 1
    });
    page.setDefaultTimeout(request.timeout_ms || DEFAULT_TIMEOUT_MS);
    await page.addInitScript({ content: PREVIEW_RUNTIME_SOURCE });
    page.on('console', (message) => {
      logs.push({
        type: message.type(),
        text: message.text(),
        location: message.location()
      });
    });
    page.on('pageerror', (error) => {
      errors.push(String(error?.stack || error?.message || error));
    });
    page.on('requestfailed', (failedRequest) => {
      errors.push(
        `${failedRequest.method()} ${failedRequest.url()} failed: ${failedRequest.failure()?.errorText}`
      );
    });

    await page.goto(request.url, {
      waitUntil: request.wait_until || 'load',
      timeout: request.timeout_ms || DEFAULT_TIMEOUT_MS
    });
    if (request.wait_ms) await page.waitForTimeout(request.wait_ms);
    return { browser, page, logs, errors };
  } catch (error) {
    await browser.close();
    throw error;
  }
}

async function makePptx(request, page) {
  const pptx = new pptxgen();
  pptx.layout = 'LAYOUT_WIDE';
  pptx.author = 'JFC Design';
  pptx.subject = 'Generated from JFC Design browser host';
  pptx.title = request.title || (await page.title()) || 'JFC Design';
  pptx.company = 'JFC';
  pptx.lang = 'en-US';
  return pptx;
}

async function writePptx(request, pptx) {
  if (!request.output) throw new Error('output is required for gen-pptx');
  await fs.mkdir(path.dirname(request.output), { recursive: true });
  await pptx.writeFile({ fileName: request.output });
  return await fs.stat(request.output);
}

async function slideHandles(page, selector, maxItems) {
  let handles = [];
  try {
    handles = await page.locator(selector || DEFAULT_SLIDE_SELECTOR).elementHandles();
  } catch {
    handles = [];
  }
  const visible = [];
  const limit = Math.max(1, Math.min(maxItems ?? 40, 120));
  for (const handle of handles) {
    if (visible.length >= limit) break;
    if (await handle.isVisible().catch(() => false)) visible.push(handle);
  }
  if (visible.length === 0) {
    const body = await page.locator('body').elementHandle().catch(() => null);
    if (body) visible.push(body);
  }
  return visible;
}

async function waitForFonts(page, timeoutMs) {
  return await page.evaluate((timeout) => {
    if (!document.fonts?.ready) return true;
    return Promise.race([
      document.fonts.ready.then(() => true).catch(() => false),
      new Promise((resolve) => window.setTimeout(() => resolve(false), timeout))
    ]);
  }, timeoutMs);
}

async function readSpeakerNotes(page) {
  return await page.evaluate(() => {
    const raw = document.querySelector('#speaker-notes')?.textContent?.trim();
    if (!raw) return [];
    try {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.map((note) => String(note));
      if (Array.isArray(parsed.notes)) return parsed.notes.map((note) => String(note));
      return [];
    } catch {
      return raw
        .split(/\n-{3,}\n/g)
        .map((note) => note.trim())
        .filter(Boolean);
    }
  });
}

async function captureEditableSlide(root) {
  const warnings = [];
  const rootRect = root.getBoundingClientRect();
  const rootStyle = getComputedStyle(root);
  const items = [];
  const textTags = new Set([
    'a',
    'button',
    'caption',
    'dd',
    'dt',
    'figcaption',
    'h1',
    'h2',
    'h3',
    'h4',
    'h5',
    'h6',
    'label',
    'li',
    'p',
    'small',
    'span',
    'strong',
    'td',
    'th'
  ]);

  const nodes = [root, ...Array.from(root.querySelectorAll('*'))];
  for (let index = 0; index < nodes.length; index += 1) {
    const element = nodes[index];
    const style = getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity || 1) <= 0) continue;

    const rect = element.getBoundingClientRect();
    if (rect.width <= 1 || rect.height <= 1) continue;
    const box = {
      x: rect.left - rootRect.left,
      y: rect.top - rootRect.top,
      w: rect.width,
      h: rect.height
    };
    const tag = element.tagName.toLowerCase();
    const base = {
      index,
      tag,
      depth: depthOf(element, root),
      box,
      style: pickStyle(style)
    };

    const background = solidColor(style.backgroundColor);
    const border = borderInfo(style);
    if ((background && tag !== 'img') || border) {
      items.push({ ...base, kind: 'shape', background, border });
    }

    if (tag === 'img') {
      const data = await dataUrlForImage(element).catch((error) => {
        warnings.push(`image_read_failed: ${String(error?.message ?? error)}`);
        return null;
      });
      if (data) items.push({ ...base, kind: 'image', data });
      continue;
    }

    if (tag === 'canvas') {
      try {
        const data = element.toDataURL('image/png');
        items.push({ ...base, kind: 'image', data });
      } catch (error) {
        warnings.push(`canvas_read_failed: ${String(error?.message ?? error)}`);
      }
      continue;
    }

    if (tag === 'svg') {
      const text = new XMLSerializer().serializeToString(element);
      const data = `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(text)))}`;
      items.push({ ...base, kind: 'image', data });
      continue;
    }

    const backgroundImage = await backgroundImageData(style).catch((error) => {
      warnings.push(`background_read_failed: ${String(error?.message ?? error)}`);
      return null;
    });
    if (backgroundImage) items.push({ ...base, kind: 'image', data: backgroundImage });

    const text = readableText(element, tag, textTags);
    if (text) items.push({ ...base, kind: 'text', text });
  }

  return {
    width: rootRect.width || window.innerWidth,
    height: rootRect.height || window.innerHeight,
    backgroundColor: rootStyle.backgroundColor || getComputedStyle(document.body).backgroundColor,
    items,
    warnings
  };

  function readableText(element, tag, textTags) {
    const text = (element.innerText || element.textContent || '').replace(/\s+/g, ' ').trim();
    if (!text) return '';
    const visibleChildren = Array.from(element.children).filter((child) => {
      const childStyle = getComputedStyle(child);
      return childStyle.display !== 'none' && childStyle.visibility !== 'hidden';
    });
    if (visibleChildren.length === 0 || textTags.has(tag)) return text.slice(0, 4000);
    return '';
  }

  function depthOf(element, stop) {
    let depth = 0;
    let current = element;
    while (current && current !== stop) {
      depth += 1;
      current = current.parentElement;
    }
    return depth;
  }

  function pickStyle(style) {
    return {
      color: style.color,
      backgroundColor: style.backgroundColor,
      borderColor: style.borderColor,
      borderRadius: style.borderRadius,
      borderWidth: style.borderWidth,
      fontFamily: style.fontFamily,
      fontSize: style.fontSize,
      fontStyle: style.fontStyle,
      fontWeight: style.fontWeight,
      lineHeight: style.lineHeight,
      opacity: style.opacity,
      textAlign: style.textAlign,
      textTransform: style.textTransform
    };
  }

  function solidColor(value) {
    if (!value || value === 'transparent' || value === 'rgba(0, 0, 0, 0)') return null;
    return value;
  }

  function borderInfo(style) {
    const width = parseFloat(style.borderTopWidth || '0');
    if (!Number.isFinite(width) || width <= 0) return null;
    if (style.borderStyle === 'none' || style.borderColor === 'rgba(0, 0, 0, 0)') return null;
    return { width, color: style.borderColor };
  }

  async function dataUrlForImage(img) {
    const url = img.currentSrc || img.src;
    if (!url) return null;
    if (url.startsWith('data:')) return url;
    return await fetchAsDataUrl(url);
  }

  async function backgroundImageData(style) {
    const value = style.backgroundImage;
    if (!value || value === 'none') return null;
    const match = value.match(/url\(["']?(.+?)["']?\)/);
    if (!match) return null;
    const url = match[1];
    if (url.startsWith('data:')) return url;
    return await fetchAsDataUrl(url);
  }

  async function fetchAsDataUrl(url) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
    const blob = await response.blob();
    return await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });
  }
}

function renderEditableSlide(pptx, model, notes) {
  const slide = pptx.addSlide();
  const rootColor = rgbaToHex(model.backgroundColor) ?? 'FFFFFF';
  slide.background = { color: rootColor };
  const scaleX = PPTX_WIDE.w / Math.max(1, model.width);
  const scaleY = PPTX_WIDE.h / Math.max(1, model.height);

  const items = [...model.items].sort((a, b) => a.depth - b.depth || a.index - b.index);
  for (const item of items.filter((entry) => entry.kind === 'shape')) {
    addRect(slide, pptx, item, scaleX, scaleY);
  }
  for (const item of items.filter((entry) => entry.kind === 'image')) {
    const box = pptxBox(item.box, scaleX, scaleY);
    if (box.w <= 0 || box.h <= 0) continue;
    slide.addImage({ data: item.data, ...box });
  }
  for (const item of items.filter((entry) => entry.kind === 'text')) {
    addText(slide, item, scaleX, scaleY);
  }
  if (notes && typeof slide.addNotes === 'function') {
    slide.addNotes(String(notes));
  }
}

function addRect(slide, pptx, item, scaleX, scaleY) {
  const box = pptxBox(item.box, scaleX, scaleY);
  if (box.w <= 0 || box.h <= 0) return;
  const fillColor = rgbaToHex(item.background);
  const borderColor = rgbaToHex(item.border?.color);
  const rect = pptx.ShapeType?.rect ?? 'rect';
  slide.addShape(rect, {
    ...box,
    fill: fillColor
      ? { color: fillColor, transparency: rgbaTransparency(item.background) }
      : { color: 'FFFFFF', transparency: 100 },
    line: borderColor
      ? { color: borderColor, transparency: rgbaTransparency(item.border?.color), width: Math.max(0.25, item.border.width * 0.4) }
      : { color: fillColor ?? 'FFFFFF', transparency: 100 }
  });
}

function addText(slide, item, scaleX, scaleY) {
  const box = pptxBox(item.box, scaleX, scaleY);
  if (box.w <= 0 || box.h <= 0) return;
  const style = item.style;
  const fillColor = rgbaToHex(style.backgroundColor);
  slide.addText(item.text, {
    ...box,
    margin: 0.03,
    breakLine: false,
    fit: 'shrink',
    valign: 'mid',
    align: normalizeAlign(style.textAlign),
    color: rgbaToHex(style.color) ?? '111111',
    transparency: rgbaTransparency(style.color),
    fontFace: normalizeFontFace(style.fontFamily),
    fontSize: clamp(px(style.fontSize) * 0.72, 4, 60),
    bold: isBold(style.fontWeight),
    italic: style.fontStyle === 'italic' || style.fontStyle === 'oblique',
    fill: fillColor ? { color: fillColor, transparency: rgbaTransparency(style.backgroundColor) } : undefined
  });
}

function pptxBox(box, scaleX, scaleY) {
  return {
    x: clamp(box.x * scaleX, -1, PPTX_WIDE.w + 1),
    y: clamp(box.y * scaleY, -1, PPTX_WIDE.h + 1),
    w: clamp(box.w * scaleX, 0, PPTX_WIDE.w + 2),
    h: clamp(box.h * scaleY, 0, PPTX_WIDE.h + 2)
  };
}

function normalizeFontFace(value) {
  return String(value || 'Arial')
    .split(',')[0]
    .replace(/^["']|["']$/g, '')
    .trim() || 'Arial';
}

function normalizeAlign(value) {
  if (['center', 'right', 'justify'].includes(value)) return value;
  return 'left';
}

function isBold(value) {
  if (!value) return false;
  if (value === 'bold' || value === 'bolder') return true;
  const n = Number(value);
  return Number.isFinite(n) && n >= 600;
}

function px(value) {
  const n = parseFloat(String(value || '0'));
  return Number.isFinite(n) ? n : 0;
}

function rgbaToHex(value) {
  const parsed = parseColor(value);
  if (!parsed || parsed.a <= 0) return null;
  return [parsed.r, parsed.g, parsed.b].map((n) => n.toString(16).padStart(2, '0')).join('').toUpperCase();
}

function rgbaTransparency(value) {
  const parsed = parseColor(value);
  if (!parsed) return 0;
  return clamp(Math.round((1 - parsed.a) * 100), 0, 100);
}

function parseColor(value) {
  if (!value) return null;
  const text = String(value).trim();
  if (text.startsWith('#')) {
    const hex = text.slice(1);
    if (hex.length === 3) {
      return {
        r: parseInt(hex[0] + hex[0], 16),
        g: parseInt(hex[1] + hex[1], 16),
        b: parseInt(hex[2] + hex[2], 16),
        a: 1
      };
    }
    if (hex.length >= 6) {
      return {
        r: parseInt(hex.slice(0, 2), 16),
        g: parseInt(hex.slice(2, 4), 16),
        b: parseInt(hex.slice(4, 6), 16),
        a: 1
      };
    }
  }
  const match = text.match(/rgba?\(([^)]+)\)/);
  if (!match) return null;
  const parts = match[1].split(',').map((part) => part.trim());
  return {
    r: clamp(Math.round(Number(parts[0])), 0, 255),
    g: clamp(Math.round(Number(parts[1])), 0, 255),
    b: clamp(Math.round(Number(parts[2])), 0, 255),
    a: parts[3] === undefined ? 1 : clamp(Number(parts[3]), 0, 1)
  };
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function check(name, passed, detail, warnOrFail = 'fail') {
  return {
    name,
    status: passed ? 'pass' : warnOrFail,
    detail
  };
}

function duplicateHashWarnings(hashes) {
  const warnings = [];
  for (let i = 1; i < hashes.length; i += 1) {
    if (hashes[i] === hashes[i - 1]) warnings.push(`duplicate_adjacent:${i}`);
  }
  const counts = new Map();
  for (const hash of hashes) counts.set(hash, (counts.get(hash) ?? 0) + 1);
  for (const [hash, count] of counts) {
    if (hashes.length > 2 && count > Math.ceil(hashes.length * 0.6)) {
      warnings.push(`duplicate_majority:${hash.slice(0, 10)}:${count}`);
    }
  }
  return warnings;
}

function uniqueStrings(values) {
  return Array.from(new Set(values.filter(Boolean)));
}

function sha1(value) {
  return crypto.createHash('sha1').update(value).digest('hex');
}

async function writeBuffer(output, buffer) {
  await fs.mkdir(path.dirname(output), { recursive: true });
  await fs.writeFile(output, buffer);
}

function readStdin() {
  return new Promise((resolve, reject) => {
    let data = '';
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (chunk) => {
      data += chunk;
    });
    process.stdin.on('end', () => resolve(data));
    process.stdin.on('error', reject);
  });
}

async function writeJson(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error?.stack || error?.message || error}\n`);
  process.exit(1);
});
