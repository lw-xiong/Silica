// The UI and rendering rules the backlog left inert because nobody had
// observed Obsidian: the property pills' remaining gestures, what clearing
// every property leaves in the file, the callout fold chevron in Live
// Preview, the setext underline row, the lucide icon per built-in callout
// type, the done-task decoration, the empty "No file is open" tab, the
// view-header title's click, `graph:open-local` with no file open, the local
// graph's camera, a half-typed search query, the search collation tie, Ctrl+E
// inside a text input, and the explorer's context menu and the modal
// backdrop.
//
// The attachment-location dropdown is NOT here: it needs the Settings modal,
// which has hung this harness's CDP endpoint before, so it lives in
// `attachment-dropdown.mjs` on its own.
//
// Phases: fixtures | run   (node ui-rules.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 420000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.uiRules = all.uiRules || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.uiRules[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
const ONLY = process.env.ONLY_STEPS ? process.env.ONLY_STEPS.split(',') : null;
async function step(name, fn) {
  if (ONLY && !ONLY.some((p) => name.startsWith(p))) return null;
  out.order.push(name);
  if (Date.now() - T0 > BUDGET_MS) { out.steps[name] = { ok: false, error: 'skipped: time budget exhausted' }; save(); return null; }
  const t = Date.now(); const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save(); return out.steps[name];
}
const vp = (rel) => path.join(VAULT, rel);

// The canonical callout types from Obsidian's public documentation, one note
// section each, so the rendered icon of every one can be read.
const CALLOUT_TYPES = ['note', 'abstract', 'summary', 'tldr', 'info', 'todo', 'tip', 'hint',
  'important', 'success', 'check', 'done', 'question', 'help', 'faq', 'warning', 'caution',
  'attention', 'failure', 'fail', 'missing', 'danger', 'error', 'bug', 'example', 'quote', 'cite',
  'nosuchtype'];

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.mkdirSync(vp('notes'), { recursive: true });
  fs.writeFileSync(vp('Props.md'),
    '---\ntags:\n  - one\n  - two\ntitle: a text value\ncount: 3\ndone: false\ndue: 2026-01-02\n---\n\nBody of the note.\n');
  fs.writeFileSync(vp('Callouts.md'),
    CALLOUT_TYPES.map((t) => `> [!${t}] ${t} title\n> Body of ${t}.`).join('\n\n') + '\n');
  fs.writeFileSync(vp('Fold.md'),
    '> [!tip] plain\n> body\n\n> [!tip]- collapsed\n> body\n\n> [!tip]+ expanded\n> body\n');
  fs.writeFileSync(vp('Setext.md'), 'Setext One\n==========\n\nbody text\n\nSetext Two\n----------\n\nmore body\n\n# ATX One\n\ntail\n');
  fs.writeFileSync(vp('Tasks.md'), '- [ ] open task\n- [x] done task\n- [X] caps done\n- plain bullet\n');
  fs.writeFileSync(vp('apple.md'), 'apple at the root, with foo in it\n');
  fs.writeFileSync(vp('notes/apple.md'), 'apple in a folder, with foo in it\n');
  fs.writeFileSync(vp('notes/Linked.md'), 'A note linking [[apple]] and [[Props]].\n');
  fs.writeFileSync(vp('Hub.md'), 'Links to [[apple]], [[notes/apple|folder apple]] and [[Props]].\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/graph.json'), JSON.stringify({
    collapse_filter: false, search: '', showTags: true, showAttachments: false, hideUnresolved: false,
    showOrphans: true, collapse_color_groups: true, colorGroups: [], collapse_display: true,
    showArrow: false, textFadeMultiplier: 0, nodeSizeMultiplier: 1, lineSizeMultiplier: 1,
    collapse_forces: true, centerStrength: 0.5, repelStrength: 10, linkStrength: 1, linkDistance: 250,
    scale: 8, close: false,
  }, null, 2));
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({
    'file-explorer': true, 'global-search': true, graph: true, backlink: true, 'outgoing-link': true,
    'tag-pane': true, properties: true, 'page-preview': true, 'command-palette': true, switcher: true,
    outline: true, bookmarks: true,
  }, null, 2));
  save(); process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
save();

await c.evaluate(`
window.__u = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  leaf() {
    const a = app.workspace.activeLeaf;
    if (a && a.view && a.view.getViewType() === 'markdown') return a;
    const ls = app.workspace.getLeavesOfType('markdown');
    return ls.length ? ls[0] : app.workspace.getLeaf(false);
  },
  view() { return this.leaf().view; },
  async open(file, mode) {
    const leaf = this.leaf();
    await leaf.openFile(app.vault.getAbstractFileByPath(file));
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await this.sleep(500);
    if (mode) await this.setMode(mode);
    await this.sleep(300);
    return { file: this.view().file.path, mode: this.mode() };
  },
  async setMode(mode) {
    const leaf = this.leaf(); const st = leaf.getViewState();
    st.state = Object.assign({}, st.state, { mode: mode === 'reading' ? 'preview' : 'source', source: mode === 'source' });
    await leaf.setViewState(st);
    await this.sleep(400);
    return this.mode();
  },
  mode() { const st = this.leaf().getViewState().state; return { mode: st.mode, source: st.source }; },
  cs(el, props) {
    if (!el) return null;
    const s = getComputedStyle(el); const o = {};
    for (const p of props) o[p] = s.getPropertyValue(p);
    return o;
  },
  rect(el) { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; },
  calloutInfo(co) {
    const icon = co.querySelector('.callout-icon');
    const svg = icon && icon.querySelector('svg');
    return {
      dataCallout: co.getAttribute('data-callout'),
      metadata: co.getAttribute('data-callout-metadata'),
      fold: co.getAttribute('data-callout-fold'),
      titleText: (co.querySelector('.callout-title-inner') || {}).textContent,
      svgClass: svg ? svg.getAttribute('class') : null,
      svgViewBox: svg ? svg.getAttribute('viewBox') : null,
      svgChildren: svg ? Array.from(svg.children).map((ch) => ch.tagName) : null,
      calloutColourVar: getComputedStyle(co).getPropertyValue('--callout-color'),
      iconColour: this.cs(icon, ['color']),
    };
  },
  // The rendered container of the active markdown view, whichever mode.
  rendered() {
    const el = this.view().containerEl;
    return el.querySelector('.markdown-preview-view') || el.querySelector('.cm-content') || el;
  },
};
return true;`);

const ev = (body) => c.evaluate(body);
const open = (f, m) => c.evaluate(`return await window.__u.open(${JSON.stringify(f)}${m ? ', ' + JSON.stringify(m) : ''});`);
const read = (rel) => fs.readFileSync(vp(rel), 'utf8');

// ---------------------------------------------------------------------------
// The empty tab has to be read before anything is opened.
// ---------------------------------------------------------------------------
await step('s01-empty-tab', async (r) => {
  r.atLaunch = await ev(`
    const u = window.__u;
    const leaves = app.workspace.getLeavesOfType('empty');
    const leaf = leaves[0] || app.workspace.getLeaf(false);
    const el = leaf.view.containerEl;
    const header = el.closest('.workspace-leaf') ? el.closest('.workspace-leaf').querySelector('.view-header') : null;
    return {
      viewType: leaf.view.getViewType(),
      displayText: leaf.view.getDisplayText ? leaf.view.getDisplayText() : null,
      emptyLeaves: app.workspace.getLeavesOfType('empty').length,
      markdownLeaves: app.workspace.getLeavesOfType('markdown').length,
      html: el.innerHTML,
      text: el.textContent,
      actions: Array.from(el.querySelectorAll('.empty-state-action, a, button')).map((a) => ({
        cls: a.className, text: a.textContent.trim(), tag: a.tagName,
      })),
      containerClasses: el.className,
      headerTitle: header ? header.querySelector('.view-header-title') && header.querySelector('.view-header-title').textContent : null,
      headerActions: header ? Array.from(header.querySelectorAll('.view-action')).map((a) => a.getAttribute('aria-label')) : null,
      tabTitle: (() => { const t = document.querySelector('.workspace-tab-header.is-active .workspace-tab-header-inner-title'); return t ? t.textContent : null; })(),
      statusBar: (() => { const s = document.querySelector('.status-bar'); return s ? s.textContent : null; })(),
      windowTitle: document.title,
    };`);
  // And the same after closing every tab, which is how a user reaches it.
  r.afterClosingAll = await ev(`
    const u = window.__u;
    await u.open('Props.md');
    await u.sleep(300);
    app.workspace.detachLeavesOfType('markdown');
    await u.sleep(600);
    const leaf = app.workspace.getLeavesOfType('empty')[0] || app.workspace.getLeaf(false);
    const el = leaf.view.containerEl;
    return {
      viewType: leaf.view.getViewType(),
      text: el.textContent,
      html: el.innerHTML.slice(0, 1200),
      actions: Array.from(el.querySelectorAll('.empty-state-action, a, button')).map((a) => ({ cls: a.className, text: a.textContent.trim() })),
      statusBar: (() => { const s = document.querySelector('.status-bar'); return s ? s.textContent : null; })(),
      modeSwitch: !!document.querySelector('.view-action[aria-label*="reading"], .view-action[aria-label*="edit"]'),
      windowTitle: document.title,
    };`);
  return r;
});

await step('s02-callout-icons', async (r) => {
  await open('Callouts.md', 'reading');
  await sleep(1200);
  // The reading view only builds the sections near the viewport, so the
  // first pass saw 13 of 28 callouts: scroll the whole note and merge.
  r.reading = await ev(`
    const u = window.__u;
    const scroller = u.view().containerEl.querySelector('.markdown-preview-view');
    const seen = new Map();
    const collect = () => {
      for (const co of u.rendered().querySelectorAll('.callout')) {
        const key = (co.querySelector('.callout-title-inner') || {}).textContent || co.getAttribute('data-callout');
        if (!seen.has(key)) seen.set(key, u.calloutInfo(co));
      }
    };
    collect();
    for (let y = 0; y < scroller.scrollHeight + 400; y += 300) {
      scroller.scrollTop = y;
      await u.sleep(160);
      collect();
    }
    scroller.scrollTop = 0;
    await u.sleep(200);
    collect();
    return Array.from(seen.values());`);
  r.unusedShape = await ev(`
    const u = window.__u;
    const el = u.rendered();
    return Array.from(el.querySelectorAll('.callout')).map((co) => {
      const icon = co.querySelector('.callout-icon');
      const svg = icon && icon.querySelector('svg');
      return {
        dataCallout: co.getAttribute('data-callout'),
        metadata: co.getAttribute('data-callout-metadata'),
        fold: co.getAttribute('data-callout-fold'),
        calloutClass: co.className,
        titleText: (co.querySelector('.callout-title-inner') || {}).textContent,
        iconHtml: icon ? icon.innerHTML : null,
        svgClass: svg ? svg.getAttribute('class') : null,
        svgTag: svg ? svg.tagName : null,
        svgChildren: svg ? Array.from(svg.children).map((ch) => ch.tagName + (ch.getAttribute('d') ? ' d=' + ch.getAttribute('d').slice(0, 40) : '')) : null,
        iconColour: u.cs(icon, ['color']),
        calloutColourVar: getComputedStyle(co).getPropertyValue('--callout-color'),
      };
    });`);
  r.live = await ev(`
    const u = window.__u;
    await u.setMode('live');
    await u.sleep(900);
    const scroller = u.view().containerEl.querySelector('.cm-scroller');
    const seen = new Map();
    const collect = () => {
      for (const co of u.view().containerEl.querySelectorAll('.callout')) {
        const key = (co.querySelector('.callout-title-inner') || {}).textContent || co.getAttribute('data-callout');
        const svg = co.querySelector('.callout-icon svg');
        if (!seen.has(key)) seen.set(key, { dataCallout: co.getAttribute('data-callout'), title: key, svgClass: svg ? svg.getAttribute('class') : null });
      }
    };
    collect();
    for (let y = 0; y < scroller.scrollHeight + 400; y += 300) { scroller.scrollTop = y; await u.sleep(160); collect(); }
    return Array.from(seen.values());`);
  return r;
});

await step('s03-callout-fold-chevron', async (r) => {
  await open('Fold.md', 'live');
  await sleep(900);
  r.before = { bytes: read('Fold.md') };
  r.markup = await ev(`
    const u = window.__u;
    const el = u.view().containerEl;
    return Array.from(el.querySelectorAll('.callout')).map((co) => ({
      dataCallout: co.getAttribute('data-callout'),
      metadata: co.getAttribute('data-callout-metadata'),
      fold: co.getAttribute('data-callout-fold'),
      classes: co.className,
      collapsed: co.classList.contains('is-collapsed'),
      foldEl: (() => { const f = co.querySelector('.callout-fold'); return f ? { cls: f.className, html: f.innerHTML.slice(0, 200), rect: u.rect(f), svgClass: f.querySelector('svg') ? f.querySelector('svg').getAttribute('class') : null } : null; })(),
      titleRect: u.rect(co.querySelector('.callout-title')),
    }));`);
  // Click the plain callout's fold affordance with a real mouse.
  const foldRect = await c.rect('.callout[data-callout="tip"] .callout-fold', 0);
  r.foldRect = foldRect;
  if (foldRect) {
    await c.clickAt(foldRect.cx, foldRect.cy);
    await sleep(900);
  }
  r.afterClickPlain = {
    dom: await ev(`
      const el = window.__u.view().containerEl;
      return Array.from(el.querySelectorAll('.callout')).map((co) => ({
        metadata: co.getAttribute('data-callout-metadata'), fold: co.getAttribute('data-callout-fold'),
        collapsed: co.classList.contains('is-collapsed'), classes: co.className }));`),
    editor: await ev(`return window.__u.view().editor.getValue();`),
  };
  await ev(`await window.__u.view().save(); return true;`);
  await sleep(700);
  r.afterClickPlain.bytes = read('Fold.md');
  // Then the already-collapsed one.
  const r2 = await c.rect('.callout.is-collapsed .callout-fold', 0);
  if (r2) { await c.clickAt(r2.cx, r2.cy); await sleep(900); }
  await ev(`await window.__u.view().save(); return true;`);
  await sleep(700);
  r.afterClickCollapsed = { bytes: read('Fold.md'), editor: await ev(`return window.__u.view().editor.getValue();`) };
  // Does the fold survive a mode switch and a reopen?
  r.survives = await ev(`
    const u = window.__u;
    await u.setMode('reading'); await u.sleep(600);
    const inReading = Array.from(u.rendered().querySelectorAll('.callout')).map((co) => co.classList.contains('is-collapsed'));
    await u.setMode('live'); await u.sleep(600);
    const backInLive = Array.from(u.view().containerEl.querySelectorAll('.callout')).map((co) => co.classList.contains('is-collapsed'));
    await u.open('Props.md'); await u.sleep(400);
    await u.open('Fold.md', 'live'); await u.sleep(800);
    const afterReopen = Array.from(u.view().containerEl.querySelectorAll('.callout')).map((co) => co.classList.contains('is-collapsed'));
    return { inReading, backInLive, afterReopen };`);
  r.finalBytes = read('Fold.md');
  return r;
});

await step('s04-setext-underline', async (r) => {
  await open('Setext.md', 'reading');
  await sleep(900);
  r.reading = await ev(`
    const u = window.__u;
    const el = u.rendered();
    return {
      html: el.innerHTML.slice(0, 1500),
      headings: Array.from(el.querySelectorAll('h1,h2,h3')).map((h) => ({ tag: h.tagName, text: h.textContent, style: u.cs(h, ['font-size', 'font-weight', 'line-height']), rect: u.rect(h) })),
      text: el.textContent,
    };`);
  r.live = await ev(`
    const u = window.__u;
    await u.setMode('live'); await u.sleep(900);
    const cm = u.view().containerEl.querySelector('.cm-content');
    const lines = Array.from(cm.querySelectorAll('.cm-line')).map((l) => ({
      text: l.textContent, cls: l.className, rect: u.rect(l),
      style: u.cs(l, ['font-size', 'font-weight', 'height', 'display', 'visibility']),
      childClasses: Array.from(l.children).map((ch) => ch.className || ch.tagName),
    }));
    return { lines };`);
  r.source = await ev(`
    const u = window.__u;
    await u.setMode('source'); await u.sleep(700);
    const cm = u.view().containerEl.querySelector('.cm-content');
    return Array.from(cm.querySelectorAll('.cm-line')).map((l) => ({ text: l.textContent, cls: l.className, style: u.cs(l, ['font-size', 'height']) }));`);
  await ev(`return await window.__u.setMode('live');`);
  return r;
});

await step('s05-done-task-decoration', async (r) => {
  await open('Tasks.md', 'reading');
  await sleep(900);
  r.reading = await ev(`
    const u = window.__u;
    const el = u.rendered();
    const P = ['text-decoration-line', 'text-decoration', 'color', 'opacity', 'font-style', 'text-decoration-color'];
    return Array.from(el.querySelectorAll('li')).map((li) => {
      const input = li.querySelector('input.task-list-item-checkbox');
      return {
        text: li.textContent, cls: li.className, dataTask: li.getAttribute('data-task'),
        liStyle: u.cs(li, P),
        inputStyle: u.cs(input, ['width', 'height', 'accent-color', 'appearance']),
        childSpans: Array.from(li.children).filter((ch) => ch.tagName !== 'INPUT').map((ch) => ({ tag: ch.tagName, cls: ch.className, text: ch.textContent.slice(0, 20), style: u.cs(ch, P) })),
        html: li.innerHTML.slice(0, 300),
      };
    });`);
  r.live = await ev(`
    const u = window.__u;
    await u.setMode('live'); await u.sleep(900);
    const cm = u.view().containerEl.querySelector('.cm-content');
    const P = ['text-decoration-line', 'color', 'opacity'];
    return Array.from(cm.querySelectorAll('.cm-line')).map((l) => ({
      text: l.textContent, cls: l.className, style: u.cs(l, P),
      spans: Array.from(l.querySelectorAll('span')).map((s) => ({ cls: s.className, text: s.textContent.slice(0, 16), style: u.cs(s, P) })),
    }));`);
  return r;
});

await step('s06-view-header-title', async (r) => {
  await open('Props.md', 'live');
  await sleep(600);
  r.before = await ev(`
    const u = window.__u;
    const t = document.querySelector('.workspace-leaf.mod-active .view-header-title') || document.querySelector('.view-header-title');
    return t ? { text: t.textContent, cls: t.className, contentEditable: t.contentEditable, tabindex: t.getAttribute('tabindex'), rect: u.rect(t), parentCls: t.parentElement.className } : null;`);
  const tr = await c.rect('.workspace-leaf.mod-active .view-header-title', 0) || await c.rect('.view-header-title', 0);
  r.rect = tr;
  if (tr) {
    await c.clickAt(tr.cx, tr.cy);
    await sleep(600);
    r.afterSingleClick = await ev(`
      const t = document.querySelector('.view-header-title');
      return { contentEditable: t && t.contentEditable, activeElement: document.activeElement && { tag: document.activeElement.tagName, cls: typeof document.activeElement.className === 'string' ? document.activeElement.className : '' }, inputs: Array.from(document.querySelectorAll('.view-header input')).map((i) => ({ cls: i.className, value: i.value })), modal: !!document.querySelector('.modal'), menu: !!document.querySelector('.menu') };`);
    await c.clickAt(tr.cx, tr.cy, { clickCount: 2 });
    await sleep(700);
    r.afterDoubleClick = await ev(`
      const t = document.querySelector('.view-header-title');
      return { contentEditable: t && t.contentEditable, text: t && t.textContent, activeElement: document.activeElement && { tag: document.activeElement.tagName, cls: typeof document.activeElement.className === 'string' ? document.activeElement.className : '' }, selectionText: String(window.getSelection()), inputs: Array.from(document.querySelectorAll('.view-header input')).map((i) => ({ cls: i.className, value: i.value })), modal: !!document.querySelector('.modal') };`);
    // If something became editable, what does typing then Enter write?
    r.typedAfterDouble = await (async () => {
      const editable = r.afterDoubleClick && (r.afterDoubleClick.contentEditable === 'true' || r.afterDoubleClick.inputs.length);
      if (!editable) return 'nothing became editable';
      await c.typeText('X');
      await c.pressKey('Enter');
      await sleep(900);
      return { files: fs.readdirSync(VAULT).filter((f) => f.endsWith('.md')) };
    })();
    await c.pressKey('Escape');
  }
  r.inlineTitle = await ev(`
    const u = window.__u;
    const it = u.view().containerEl.querySelector('.inline-title');
    return it ? { text: it.textContent, cls: it.className, contentEditable: it.contentEditable, style: u.cs(it, ['font-size', 'font-weight']) } : null;`);
  return r;
});

await step('s07-property-pills', async (r) => {
  await open('Props.md', 'reading');
  await sleep(1000);
  r.markup = await ev(`
    const u = window.__u;
    const el = u.rendered();
    const rows = Array.from(el.querySelectorAll('.metadata-property')).map((p) => ({
      key: p.getAttribute('data-property-key'), type: p.getAttribute('data-property-type'),
      keyText: (p.querySelector('.metadata-property-key-input') || {}).textContent,
      valueHtml: (p.querySelector('.metadata-property-value') || {}).innerHTML,
    }));
    const tagRow = el.querySelector('.metadata-property[data-property-key="tags"]');
    const pills = tagRow ? Array.from(tagRow.querySelectorAll('.multi-select-pill')).map((pl) => ({
      text: pl.textContent, cls: pl.className,
      remove: (() => { const x = pl.querySelector('.multi-select-pill-remove-button'); return x ? { cls: x.className, style: u.cs(x, ['display', 'visibility', 'opacity', 'width']), rect: u.rect(x), html: x.innerHTML.slice(0, 120) } : null; })(),
    })) : null;
    const input = tagRow ? tagRow.querySelector('.multi-select-input') : null;
    return { rows, pills, input: input ? { cls: input.className, contentEditable: input.contentEditable, rect: u.rect(input), text: input.textContent } : null };`);
  // Hover a pill and read the remove button again: hover-only, or always drawn?
  const pillRect = await c.rect('.metadata-property[data-property-key="tags"] .multi-select-pill', 0);
  r.pillRect = pillRect;
  if (pillRect) {
    await c.mouseMove(pillRect.cx, pillRect.cy);
    await sleep(400);
    r.removeWhileHovered = await ev(`
      const u = window.__u;
      const x = document.querySelector('.metadata-property[data-property-key="tags"] .multi-select-pill .multi-select-pill-remove-button');
      return x ? { style: u.cs(x, ['display', 'visibility', 'opacity', 'width']), rect: u.rect(x) } : null;`);
    await c.mouseMove(5, 5);
    await sleep(400);
    r.removeWhileAway = await ev(`
      const u = window.__u;
      const x = document.querySelector('.metadata-property[data-property-key="tags"] .multi-select-pill .multi-select-pill-remove-button');
      return x ? { style: u.cs(x, ['display', 'visibility', 'opacity', 'width']), rect: u.rect(x) } : null;`);
  }
  // Enter in the trailing input: does it add an item?
  const inputRect = await c.rect('.metadata-property[data-property-key="tags"] .multi-select-input', 0);
  r.inputRect = inputRect;
  if (inputRect) {
    await c.clickAt(inputRect.cx, inputRect.cy);
    await sleep(500);
    await c.typeText('three');
    await sleep(400);
    r.afterTyping = { dom: await ev(`return { pills: Array.from(document.querySelectorAll('.metadata-property[data-property-key="tags"] .multi-select-pill')).map((p) => p.textContent), suggester: !!document.querySelector('.suggestion-container') };`) };
    await c.pressKey('Enter');
    await sleep(1200);
    r.afterEnter = {
      pills: await ev(`return Array.from(document.querySelectorAll('.metadata-property[data-property-key="tags"] .multi-select-pill')).map((p) => p.textContent);`),
      bytes: read('Props.md'),
    };
    // Backspace with the input empty: does it remove the previous pill?
    await c.pressKey('Backspace');
    await sleep(900);
    r.afterBackspaceOnce = {
      pills: await ev(`return Array.from(document.querySelectorAll('.metadata-property[data-property-key="tags"] .multi-select-pill')).map((p) => p.textContent);`),
      bytes: read('Props.md'),
    };
    await c.pressKey('Backspace');
    await sleep(900);
    r.afterBackspaceTwice = {
      pills: await ev(`return Array.from(document.querySelectorAll('.metadata-property[data-property-key="tags"] .multi-select-pill')).map((p) => p.textContent);`),
      bytes: read('Props.md'),
    };
  }
  // Remove every remaining item with the × and read what the key becomes.
  for (let i = 0; i < 4; i++) {
    const x = await c.rect('.metadata-property[data-property-key="tags"] .multi-select-pill-remove-button', 0);
    if (!x) break;
    await c.clickAt(x.cx, x.cy);
    await sleep(900);
    r['afterRemove' + (i + 1)] = {
      pills: await ev(`return Array.from(document.querySelectorAll('.metadata-property[data-property-key="tags"] .multi-select-pill')).map((p) => p.textContent);`),
      bytes: read('Props.md'),
    };
  }
  r.finalBytes = read('Props.md');
  return r;
});

await step('s08-clear-all-properties', async (r) => {
  await ev(`
    const u = window.__u;
    await app.vault.modify(app.vault.getAbstractFileByPath('Props.md'), '---\\nalpha: one\\nbeta: two\\n---\\n\\nbody\\n');
    await u.sleep(800);
    await u.open('Props.md', 'reading');
    await u.sleep(700);
    return true;`);
  r.before = read('Props.md');
  r.commandPresent = await ev(`
    const cmd = app.commands.commands['markdown:clear-metadata-properties'];
    return cmd ? { name: cmd.name, kind: cmd.editorCallback ? 'editorCallback' : cmd.editorCheckCallback ? 'editorCheckCallback' : cmd.checkCallback ? 'checkCallback' : 'callback' } : null;`);
  r.ran = await ev(`
    const u = window.__u;
    const ran = app.commands.executeCommandById('markdown:clear-metadata-properties');
    await u.sleep(800);
    try { await u.view().save(); } catch (e) {}
    await u.sleep(700);
    return ran;`);
  r.afterReading = { bytes: read('Props.md'), firstBytes: Array.from(fs.readFileSync(vp('Props.md'))).slice(0, 12) };
  // And from Live Preview, in case the mode matters.
  await ev(`
    const u = window.__u;
    await app.vault.modify(app.vault.getAbstractFileByPath('Props.md'), '---\\nalpha: one\\nbeta: two\\n---\\n\\nbody\\n');
    await u.sleep(800);
    await u.setMode('live'); await u.sleep(700);
    app.commands.executeCommandById('markdown:clear-metadata-properties');
    await u.sleep(800);
    try { await u.view().save(); } catch (e) {}
    await u.sleep(700);
    return true;`);
  r.afterLive = { bytes: read('Props.md'), firstBytes: Array.from(fs.readFileSync(vp('Props.md'))).slice(0, 12) };
  // A note whose frontmatter is the whole file.
  await ev(`
    const u = window.__u;
    await app.vault.modify(app.vault.getAbstractFileByPath('Props.md'), '---\\nalpha: one\\n---\\n');
    await u.sleep(800);
    app.commands.executeCommandById('markdown:clear-metadata-properties');
    await u.sleep(800);
    try { await u.view().save(); } catch (e) {}
    await u.sleep(700);
    return true;`);
  r.afterFrontmatterOnly = { bytes: read('Props.md'), firstBytes: Array.from(fs.readFileSync(vp('Props.md'))).slice(0, 12) };
  return r;
});

await step('s09-graph-open-local-no-file', async (r) => {
  r.withFileOpen = await ev(`
    const u = window.__u;
    await u.open('Hub.md', 'live');
    await u.sleep(400);
    const cmd = app.commands.commands['graph:open-local'];
    return { present: !!cmd, name: cmd && cmd.name, kind: cmd && (cmd.checkCallback ? 'checkCallback' : cmd.callback ? 'callback' : 'other'), check: cmd && cmd.checkCallback ? cmd.checkCallback(true) : 'no checkCallback' };`);
  r.withNoFileOpen = await ev(`
    const u = window.__u;
    app.workspace.detachLeavesOfType('markdown');
    app.workspace.detachLeavesOfType('localgraph');
    app.workspace.detachLeavesOfType('graph');
    await u.sleep(700);
    const cmd = app.commands.commands['graph:open-local'];
    return {
      activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path,
      activeLeafType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType(),
      check: cmd && cmd.checkCallback ? cmd.checkCallback(true) : 'no checkCallback',
      listedInPalette: (() => {
        const cmds = app.commands.listCommands();
        return cmds.some((x) => x.id === 'graph:open-local');
      })(),
      ran: app.commands.executeCommandById('graph:open-local'),
    };`);
  await sleep(900);
  r.afterRunningWithNoFile = await ev(`
    return { localgraph: app.workspace.getLeavesOfType('localgraph').length, graph: app.workspace.getLeavesOfType('graph').length, activeLeafType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType() };`);
  return r;
});

await step('s10-local-graph-camera', async (r) => {
  r.graphJsonOnDisk = JSON.parse(read('.obsidian/graph.json'));
  r.openGlobal = await ev(`
    const u = window.__u;
    app.workspace.detachLeavesOfType('graph');
    app.workspace.detachLeavesOfType('localgraph');
    await u.sleep(400);
    await u.open('Hub.md', 'live');
    await u.sleep(400);
    app.commands.executeCommandById('graph:open');
    await u.sleep(1500);
    const leaf = app.workspace.getLeavesOfType('graph')[0];
    const v = leaf && leaf.view;
    return v ? { scale: v.renderer && v.renderer.scale, options: v.options ? JSON.parse(JSON.stringify(v.options)) : null, state: JSON.parse(JSON.stringify(leaf.getViewState())) } : null;`);
  r.openLocal = await ev(`
    const u = window.__u;
    app.workspace.detachLeavesOfType('graph');
    await u.sleep(300);
    await u.open('Hub.md', 'live');
    await u.sleep(300);
    app.commands.executeCommandById('graph:open-local');
    await u.sleep(1800);
    const leaf = app.workspace.getLeavesOfType('localgraph')[0];
    const v = leaf && leaf.view;
    return v ? { scale: v.renderer && v.renderer.scale, options: v.options ? JSON.parse(JSON.stringify(v.options)) : null, state: JSON.parse(JSON.stringify(leaf.getViewState())) } : null;`);
  return r;
});

await step('s11-search-half-typed-and-collation', async (r) => {
  r.queries = {};
  for (const q of ['foo', 'foo OR', 'foo (', 'foo -', 'OR', '(', 'apple', 'file:apple', '']) {
    r.queries[q || '(empty)'] = await ev(`
      const u = window.__u;
      const leaf = app.workspace.getLeavesOfType('search')[0] || (app.commands.executeCommandById('global-search:open'), null);
      await u.sleep(500);
      const l = app.workspace.getLeavesOfType('search')[0];
      const v = l.view;
      v.setQuery(${JSON.stringify(q)});
      await u.sleep(1400);
      const el = v.containerEl;
      return {
        resultCount: el.querySelectorAll('.tree-item.search-result').length,
        files: Array.from(el.querySelectorAll('.tree-item.search-result .tree-item-inner')).map((x) => x.textContent),
        summary: (el.querySelector('.search-result-count, .search-results-info, .search-info-more-matches') || {}).textContent,
        empty: !!el.querySelector('.search-empty-state'),
        emptyText: (el.querySelector('.search-empty-state') || {}).textContent,
      };`);
  }
  return r;
});

await step('s12-ctrl-e-in-text-input', async (r) => {
  r.setup = await ev(`
    const u = window.__u;
    await u.open('Props.md', 'live');
    await u.sleep(400);
    app.commands.executeCommandById('global-search:open');
    await u.sleep(800);
    const input = document.querySelector('.workspace-leaf-content[data-type="search"] input');
    if (input) { input.focus(); input.value = 'foobar'; input.setSelectionRange(3, 3); }
    return input ? { focused: document.activeElement === input, value: input.value, start: input.selectionStart } : null;`);
  await c.pressKey('e', { ctrl: true });
  await sleep(700);
  r.afterCtrlE = await ev(`
    const input = document.querySelector('.workspace-leaf-content[data-type="search"] input');
    const leaf = app.workspace.getLeavesOfType('markdown')[0];
    return {
      inputValue: input && input.value,
      selectionStart: input && input.selectionStart,
      stillFocused: document.activeElement === input,
      activeElement: document.activeElement && document.activeElement.tagName,
      markdownMode: leaf ? leaf.getViewState().state.mode : null,
      markdownSource: leaf ? leaf.getViewState().state.source : null,
    };`);
  // The same in a modal's text field, and in the quick switcher.
  r.inSwitcher = await ev(`
    const u = window.__u;
    app.commands.executeCommandById('switcher:open');
    await u.sleep(700);
    const input = document.querySelector('.prompt-input');
    if (input) { input.focus(); input.value = 'appl'; input.setSelectionRange(2, 2); }
    return input ? { value: input.value, start: input.selectionStart } : null;`);
  await c.pressKey('e', { ctrl: true });
  await sleep(600);
  r.switcherAfterCtrlE = await ev(`
    const input = document.querySelector('.prompt-input');
    const leaf = app.workspace.getLeavesOfType('markdown')[0];
    return { value: input && input.value, start: input && input.selectionStart, open: !!document.querySelector('.prompt'), markdownMode: leaf ? leaf.getViewState().state.mode : null };`);
  await c.pressKey('Escape');
  await sleep(300);
  // And the editor, for the control: Ctrl+E is "Toggle reading view".
  r.hotkeyForToggle = await ev(`
    const hm = app.hotkeyManager;
    const fmt = (hk) => hk ? hk.map((h) => (h.modifiers || []).join('+') + (h.modifiers && h.modifiers.length ? '+' : '') + h.key) : null;
    return { 'markdown:toggle-preview': fmt(hm.defaultKeys['markdown:toggle-preview']), names: app.commands.commands['markdown:toggle-preview'] && app.commands.commands['markdown:toggle-preview'].name };`);
  r.inEditor = await ev(`
    const u = window.__u;
    const leaf = app.workspace.getLeavesOfType('markdown')[0];
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await u.sleep(300);
    leaf.view.editor.focus();
    await u.sleep(200);
    return leaf.getViewState().state.mode;`);
  await c.pressKey('e', { ctrl: true });
  await sleep(800);
  r.editorAfterCtrlE = await ev(`
    const leaf = app.workspace.getLeavesOfType('markdown')[0];
    return leaf.getViewState().state.mode;`);
  return r;
});

await step('s13-explorer-menu-and-backdrop', async (r) => {
  r.setup = await ev(`
    const u = window.__u;
    app.workspace.leftSplit.expand();
    app.workspace.revealLeaf(app.workspace.getLeavesOfType('file-explorer')[0]);
    await u.sleep(700);
    return true;`);
  const fileRow = await c.rect('.workspace-leaf-content[data-type="file-explorer"] .nav-file-title[data-path="apple.md"]', 0);
  r.fileRow = fileRow;
  if (fileRow) {
    await c.clickAt(fileRow.cx, fileRow.cy, { button: 'right' });
    await sleep(700);
    r.fileMenu = await c.openMenuItems();
    r.fileMenuDetail = await ev(`
      return Array.from(document.querySelectorAll('.menu')).map((m) => Array.from(m.children).map((ch) => ({
        cls: ch.className,
        title: (ch.querySelector('.menu-item-title') || {}).textContent,
        icon: (() => { const s = ch.querySelector('svg'); return s ? s.getAttribute('class') : null; })(),
        section: ch.getAttribute('data-section'),
      })));`);
    await c.pressKey('Escape'); await sleep(300);
  }
  const folderRow = await c.rect('.workspace-leaf-content[data-type="file-explorer"] .nav-folder-title[data-path="notes"]', 0);
  if (folderRow) {
    await c.clickAt(folderRow.cx, folderRow.cy, { button: 'right' });
    await sleep(700);
    r.folderMenu = await c.openMenuItems();
    await c.pressKey('Escape'); await sleep(300);
  }
  // Empty space below the last row, and the pane's own header.
  r.emptySpaceMenu = await (async () => {
    const pane = await c.rect('.workspace-leaf-content[data-type="file-explorer"] .nav-files-container', 0);
    if (!pane) return null;
    await c.clickAt(pane.x + pane.w / 2, pane.y + pane.h - 6, { button: 'right' });
    await sleep(600);
    const items = await c.openMenuItems();
    await c.pressKey('Escape'); await sleep(250);
    return items;
  })();
  // The modal backdrop.
  r.backdrop = await ev(`
    const u = window.__u;
    app.commands.executeCommandById('app:open-settings');
    await u.sleep(1200);
    const bg = document.querySelector('.modal-bg');
    const container = document.querySelector('.modal-container');
    const modal = document.querySelector('.modal');
    const out = {
      bg: bg ? { cls: bg.className, style: u.cs(bg, ['background-color', 'opacity', 'backdrop-filter']), rect: u.rect(bg) } : null,
      container: container ? { cls: container.className, style: u.cs(container, ['background-color', 'z-index']) } : null,
      modal: modal ? { cls: modal.className, style: u.cs(modal, ['width', 'height', 'max-width', 'max-height', 'border-radius', 'background-color']), rect: u.rect(modal) } : null,
      windowSize: { w: window.innerWidth, h: window.innerHeight },
    };
    app.setting.close();
    await u.sleep(500);
    return out;`);
  return r;
});

// ---------------------------------------------------------------------------
// Second pass: the rows the first run read through a wrong selector, a count
// that had not settled, or a modal that turned out to be a popout window.
// ---------------------------------------------------------------------------

await step('s14-menu-icons', async (r) => {
  await ev(`
    const u = window.__u;
    app.workspace.leftSplit.expand();
    app.workspace.revealLeaf(app.workspace.getLeavesOfType('file-explorer')[0]);
    await u.sleep(700);
    return true;`);
  const menuRows = () => ev(`
    return Array.from(document.querySelectorAll('.menu')).map((m) => Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map((it) => {
      if (it.classList.contains('menu-separator')) return { separator: true };
      const svg = it.querySelector('svg');
      return {
        title: (it.querySelector('.menu-item-title') || {}).textContent,
        icon: svg ? svg.getAttribute('class') : null,
        section: it.getAttribute('data-section'),
        disabled: it.classList.contains('is-disabled'),
        cls: it.className,
      };
    }));`);
  for (const [label, sel] of [
    ['file', '.workspace-leaf-content[data-type="file-explorer"] .nav-file-title[data-path="apple.md"]'],
    ['folder', '.workspace-leaf-content[data-type="file-explorer"] .nav-folder-title[data-path="notes"]'],
  ]) {
    const rect = await c.rect(sel, 0);
    if (!rect) { r[label] = 'row not found'; continue; }
    await c.clickAt(rect.cx, rect.cy, { button: 'right' });
    await sleep(700);
    r[label] = await menuRows();
    await c.pressKey('Escape');
    await sleep(300);
  }
  return r;
});

await step('s15-modal-backdrop', async (r) => {
  const P = ['background-color', 'opacity', 'backdrop-filter', 'z-index', 'position'];
  r.switcher = await ev(`
    const u = window.__u;
    app.commands.executeCommandById('switcher:open');
    await u.sleep(900);
    const P = ${JSON.stringify(P)};
    const bg = document.querySelector('.modal-bg');
    const container = document.querySelector('.modal-container');
    const prompt = document.querySelector('.prompt');
    const out = {
      bg: bg ? { cls: bg.className, style: u.cs(bg, P), rect: u.rect(bg) } : null,
      container: container ? { cls: container.className, style: u.cs(container, P) } : null,
      prompt: prompt ? { cls: prompt.className, style: u.cs(prompt, ['width', 'max-width', 'border-radius', 'background-color', 'box-shadow']), rect: u.rect(prompt) } : null,
      windowSize: { w: window.innerWidth, h: window.innerHeight },
    };
    return out;`);
  await c.pressKey('Escape');
  await sleep(400);
  // The delete confirmation, which is a real .modal rather than a prompt.
  r.confirm = await ev(`
    const u = window.__u;
    const P = ${JSON.stringify(P)};
    app.workspace.leftSplit.expand();
    await u.sleep(300);
    const f = app.vault.getAbstractFileByPath('notes/Linked.md');
    app.fileManager.promptForDeletion(f);
    await u.sleep(900);
    const bg = document.querySelector('.modal-bg');
    const modal = document.querySelector('.modal');
    const out = {
      bg: bg ? { cls: bg.className, style: u.cs(bg, P), rect: u.rect(bg) } : null,
      modal: modal ? { cls: modal.className, title: (modal.querySelector('.modal-title') || {}).textContent, text: (modal.querySelector('.modal-content') || {}).textContent, buttons: Array.from(modal.querySelectorAll('button')).map((b) => ({ text: b.textContent, cls: b.className })), style: u.cs(modal, ['width', 'max-width', 'border-radius', 'background-color', 'box-shadow']), rect: u.rect(modal) } : null,
      closeButton: (() => { const x = document.querySelector('.modal-close-button'); return x ? { cls: x.className, rect: u.rect(x) } : null; })(),
      windowSize: { w: window.innerWidth, h: window.innerHeight },
    };
    return out;`);
  await c.pressKey('Escape');
  await sleep(400);
  r.settingsIsPopout = await ev(`
    const u = window.__u;
    app.commands.executeCommandById('app:open-settings');
    await u.sleep(1200);
    const out = {
      modalBgInMainWindow: !!document.querySelector('.modal-bg'),
      settingOpen: !!app.setting && !!app.setting.activeTab,
      activeTabName: app.setting && app.setting.activeTab ? app.setting.activeTab.name : null,
      containerOwnerDocumentIsThis: app.setting && app.setting.containerEl ? app.setting.containerEl.ownerDocument === document : null,
    };
    app.setting.close();
    await u.sleep(500);
    return out;`);
  return r;
});

await step('s16-search-counts-and-collation', async (r) => {
  r.queries = {};
  const QUERIES = ['foo', 'foo OR', 'foo AND', 'foo -', 'foo (', 'foo )', '(foo', 'foo OR bar',
    'OR', '(', 'apple', 'file:apple', '"foo', 'foo "'];
  await ev(`
    const u = window.__u;
    app.commands.executeCommandById('global-search:open');
    await u.sleep(1000);
    return true;`);
  for (const q of QUERIES) {
    r.queries[q] = await ev(`
      const u = window.__u;
      const leaf = app.workspace.getLeavesOfType('search')[0];
      const v = leaf.view;
      v.setQuery(${JSON.stringify(q)});
      await u.sleep(2000);
      const el = v.containerEl;
      const rows = Array.from(el.querySelectorAll('.search-result-file-title')).map((x) => x.textContent.trim());
      return {
        query: v.searchQuery ? String(v.searchQuery.query || '') : null,
        resultCount: v.dom && v.dom.resultDomLookup ? v.dom.resultDomLookup.size : null,
        rows,
        countLabel: (el.querySelector('.search-results-info, .tree-item-flair, .search-result-count') || {}).textContent,
        emptyState: (el.querySelector('.search-empty-state') || {}).textContent || null,
      };`);
  }
  return r;
});

await step('s17-pill-backspace-detail', async (r) => {
  await ev(`
    const u = window.__u;
    await app.vault.modify(app.vault.getAbstractFileByPath('Props.md'), '---\\ntags:\\n  - one\\n  - two\\n---\\n\\nbody\\n');
    await u.sleep(800);
    await u.open('Props.md', 'reading');
    await u.sleep(900);
    return true;`);
  const pillState = () => ev(`
    return Array.from(document.querySelectorAll('.metadata-property[data-property-key="tags"] .multi-select-pill')).map((p) => ({ text: p.textContent, cls: p.className, tabindex: p.getAttribute('tabindex') }));`);
  const inputRect = await c.rect('.metadata-property[data-property-key="tags"] .multi-select-input', 0);
  r.inputRect = inputRect;
  if (!inputRect) return r;
  await c.clickAt(inputRect.cx, inputRect.cy);
  await sleep(600);
  r.afterFocus = { pills: await pillState(), active: await ev(`return document.activeElement && document.activeElement.className;`) };
  for (let i = 1; i <= 4; i++) {
    await c.pressKey('Backspace');
    await sleep(800);
    r['afterBackspace' + i] = {
      pills: await pillState(),
      active: await ev(`return document.activeElement && document.activeElement.className;`),
      bytes: read('Props.md'),
    };
  }
  return r;
});

await step('s18-callout-fold-plus-minus', async (r) => {
  await ev(`
    const u = window.__u;
    await app.vault.modify(app.vault.getAbstractFileByPath('Fold.md'), '> [!tip] plain\\n> body\\n\\n> [!tip]- minus\\n> body\\n\\n> [!tip]+ plus\\n> body\\n');
    await u.sleep(800);
    await u.open('Fold.md', 'live');
    await u.sleep(1000);
    return true;`);
  r.start = read('Fold.md');
  const state = () => ev(`
    const u = window.__u;
    return Array.from(u.view().containerEl.querySelectorAll('.callout')).map((co) => ({
      title: (co.querySelector('.callout-title-inner') || {}).textContent,
      fold: co.getAttribute('data-callout-fold'),
      collapsible: co.classList.contains('is-collapsible'),
      collapsed: co.classList.contains('is-collapsed'),
      hasFoldEl: !!co.querySelector('.callout-fold'),
      chevron: (() => { const s = co.querySelector('.callout-fold svg'); return s ? s.getAttribute('class') : null; })(),
      chevronTransform: (() => { const f = co.querySelector('.callout-fold'); return f ? getComputedStyle(f).transform : null; })(),
    }));`);
  r.initial = await state();
  // Click each callout's title in turn (the whole title row is the hit area
  // for a collapsible callout), reading the bytes after each.
  for (const want of ['plain', 'minus', 'plus']) {
    const rect = await ev(`
      const u = window.__u;
      const co = Array.from(u.view().containerEl.querySelectorAll('.callout')).find((x) => ((x.querySelector('.callout-title-inner') || {}).textContent || '').trim() === ${JSON.stringify(want)});
      if (!co) return null;
      const f = co.querySelector('.callout-fold') || co.querySelector('.callout-title');
      const b = f.getBoundingClientRect();
      return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2, usedFold: !!co.querySelector('.callout-fold') };`);
    if (!rect) { r['click_' + want] = 'callout not rendered'; continue; }
    await c.clickAt(rect.cx, rect.cy);
    await sleep(900);
    await ev(`try { await window.__u.view().save(); } catch (e) {} return true;`);
    await sleep(600);
    r['click_' + want] = { rect, state: await state(), editor: await ev(`return window.__u.view().editor.getValue();`), bytes: read('Fold.md') };
    // Click again to put it back, and read again.
    await c.clickAt(rect.cx, rect.cy);
    await sleep(900);
    await ev(`try { await window.__u.view().save(); } catch (e) {} return true;`);
    await sleep(600);
    r['clickBack_' + want] = { state: await state(), bytes: read('Fold.md') };
  }
  r.readingView = await ev(`
    const u = window.__u;
    await u.setMode('reading');
    await u.sleep(900);
    return Array.from(u.rendered().querySelectorAll('.callout')).map((co) => ({
      title: (co.querySelector('.callout-title-inner') || {}).textContent,
      fold: co.getAttribute('data-callout-fold'),
      collapsible: co.classList.contains('is-collapsible'),
      collapsed: co.classList.contains('is-collapsed'),
      hasFoldEl: !!co.querySelector('.callout-fold'),
    }));`);
  r.finalBytes = read('Fold.md');
  return r;
});

await step('s19-header-title-rename', async (r) => {
  await ev(`
    const u = window.__u;
    await u.open('Props.md', 'live');
    await u.sleep(600);
    return true;`);
  const sel = '.workspace-leaf.mod-active .view-header-title';
  r.before = await ev(`
    const u = window.__u;
    const t = document.querySelector(${JSON.stringify(sel)});
    return t ? { text: t.textContent, contentEditable: t.contentEditable, tabindex: t.getAttribute('tabindex'), rect: u.rect(t) } : null;`);
  const tr = await c.rect(sel, 0);
  if (!tr) return r;
  await c.clickAt(tr.cx, tr.cy);
  await sleep(600);
  r.afterSingleClick = await ev(`
    const t = document.querySelector(${JSON.stringify(sel)});
    return { focused: document.activeElement === t, selectionText: String(window.getSelection()), caretOffset: (() => { const s = window.getSelection(); return s.rangeCount ? s.getRangeAt(0).startOffset : null; })() };`);
  await c.typeText('X');
  await sleep(500);
  r.afterTyping = await ev(`
    const t = document.querySelector(${JSON.stringify(sel)});
    return { text: t && t.textContent, file: window.__u.view().file.path };`);
  await c.pressKey('Enter');
  await sleep(1400);
  r.afterEnter = {
    dom: await ev(`
      const t = document.querySelector(${JSON.stringify(sel)});
      return { text: t && t.textContent, file: window.__u.view().file.path, focused: document.activeElement === t };`),
    files: fs.readdirSync(VAULT).filter((f) => f.endsWith('.md')).sort(),
  };
  // Double click, then Escape: does Escape abandon the edit?
  const tr2 = await c.rect(sel, 0);
  if (tr2) {
    await c.clickAt(tr2.cx, tr2.cy, { clickCount: 2 });
    await sleep(500);
    r.afterDoubleClick = await ev(`
      const t = document.querySelector(${JSON.stringify(sel)});
      return { selectionText: String(window.getSelection()), text: t && t.textContent };`);
    await c.typeText('ZZZ');
    await sleep(400);
    await c.pressKey('Escape');
    await sleep(1200);
    r.afterEscape = {
      dom: await ev(`const t = document.querySelector(${JSON.stringify(sel)}); return { text: t && t.textContent, file: window.__u.view().file.path };`),
      files: fs.readdirSync(VAULT).filter((f) => f.endsWith('.md')).sort(),
    };
  }
  return r;
});

await step('s20-search-collation-paths', async (r) => {
  r.files = await ev(`return app.vault.getMarkdownFiles().map((f) => f.path).sort();`);
  r.orders = {};
  await ev(`
    const u = window.__u;
    app.commands.executeCommandById('global-search:open');
    await u.sleep(1000);
    return true;`);
  for (const q of ['apple', 'file:apple', 'path:apple', 'foo']) {
    r.orders[q] = await ev(`
      const u = window.__u;
      const v = app.workspace.getLeavesOfType('search')[0].view;
      v.setQuery(${JSON.stringify(q)});
      await u.sleep(2200);
      const lookup = v.dom && v.dom.resultDomLookup;
      const vChildren = v.dom && v.dom.vChildren && v.dom.vChildren.children ? v.dom.vChildren.children : null;
      return {
        count: lookup ? lookup.size : null,
        lookupPaths: lookup ? Array.from(lookup.keys()).map((f) => (f && f.path) ? f.path : String(f)) : null,
        renderedOrder: vChildren ? vChildren.map((ch) => (ch.file ? ch.file.path : null)) : null,
        domOrder: Array.from(v.containerEl.querySelectorAll('.tree-item.search-result')).map((it) => {
          const t = it.querySelector('.tree-item-inner');
          return { title: t ? t.textContent.trim() : null, flair: (it.querySelector('.tree-item-flair') || {}).textContent };
        }),
        sortOrder: v.sortOrder || (v.dom && v.dom.sortOrder) || null,
      };`);
  }
  return r;
});

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
