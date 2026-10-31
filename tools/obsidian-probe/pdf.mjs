// Obsidian's PDF view, driven over CDP: the view object and where pdf.js's
// PDFViewer hangs off it, the chrome's markup and computed styles in both
// themes, the default zoom and the zoom <select>, every toolbar control
// clicked from a fresh default, the sidebar and the find bar, keys inside the
// viewer, wheel and programmatic scrolling against `getState()` and the leaf
// `workspace.json` records, `#page=`/`#height=` on links and on embeds, the
// embeds of Embeds.md in reading view, Live Preview and source mode, an embed
// below the fold, interaction inside an embed, the canvas file node, the
// context menus, text selection and the copy rows' clipboard bytes, the
// Outline/Backlinks/Outgoing links panes, small.pdf and broken.pdf, ctrl-hover,
// the settings and plugins that mention PDF, and restart fidelity.
//
// The first pass (s00-s18) left pdf.js's own view history behind: s06 sets a
// 90 degree rotation, pdf.js stores it in localStorage['pdfjs.history'] under
// the file's fingerprint, and every later open of doc.pdf came back rotated,
// in the leaf, the embeds and the canvas alike. The `b` steps (s02b-s17b, and
// s18 re-recorded) strip the probe's own entries first and supersede the
// spoiled ones; the first pass's records are kept as evidence of the history
// itself. Run them with
//   ONLY_STEPS=s02b,s03b,s03c,s04b,s05b,s06b,s07b,s08b,s09b,s10b,s11b,s12b,s14b,s16b,s17b,s18
//
// Phases: fixtures | run | after-restart   (node pdf.mjs <phase>)
// ONLY_STEPS=s03,s04 re-runs a step or two (it never filters the fixtures
// phase: the runner makes a fresh vault every time). SHOTS names the folder
// the PNGs go to (default: a `shots` folder beside RESULTS). `typst` must be
// on PATH for the fixtures phase.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execSync } from 'node:child_process';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }
const SHOTS = process.env.SHOTS || path.join(path.dirname(RESULTS), 'shots');

const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.pdf = all.pdf || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
if (phase !== 'fixtures' && process.env.ONLY_STEPS && all.pdf[phase]) {
  // A re-run of a few steps keeps the earlier run's other steps.
  const prev = all.pdf[phase];
  Object.assign(out, { steps: prev.steps || {}, order: prev.order || [], version: prev.version, reruns: (prev.reruns || []).concat([{ at: out.startedAt, steps: process.env.ONLY_STEPS }]) });
}
all.pdf[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

// ONLY_STEPS names run steps; it must never filter the fixtures phase, or the
// re-run starts on an empty vault and every open misses.
const ONLY = process.env.ONLY_STEPS && phase !== 'fixtures' ? process.env.ONLY_STEPS.split(',') : null;
const STEP_MS = Number(process.env.STEP_MS || 240000);
async function step(name, fn) {
  if (ONLY && !ONLY.some((p) => name.startsWith(p))) return null;
  if (!out.order.includes(name)) out.order.push(name);
  const t = Date.now();
  const partial = {};
  let timer;
  const timeout = new Promise((_, rej) => { timer = setTimeout(() => rej(new Error('step timeout after ' + STEP_MS + ' ms')), STEP_MS); });
  try { const v = await Promise.race([fn(partial), timeout]); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  finally { clearTimeout(timer); }
  save();
  const s = out.steps[name];
  console.error(`[pdf] ${name}: ${s.ok ? 'ok' : 'FAILED ' + s.error.split('\n')[0]} (${s.ms} ms)`);
  return s;
}

// ---- fixtures --------------------------------------------------------------
const DOC_TYP = `#set document(title: "Probe document", author: "probe")
#set page(paper: "a4")
#set heading(numbering: "1.1")
#set text(size: 14pt)

= One
== One point one
#text(size: 40pt)[This is page one.]
#lorem(60)

#pagebreak()
= Two
== Two point one
#text(size: 40pt)[This is page two.]
#lorem(60)

#pagebreak()
#set page(paper: "a4", flipped: true)
= Three
== Three point one
#text(size: 40pt)[This is page three.]
#lorem(60)

#pagebreak()
#set page(paper: "a4", flipped: false)
= Four
== Four point one
#text(size: 40pt)[This is page four.]
#lorem(60)

#pagebreak()
= Five
== Five point one
#text(size: 40pt)[This is page five.]
#lorem(60)
`;
const SMALL_TYP = `#set document(title: "Small", author: "probe")
#set page(paper: "us-letter")
#text(size: 30pt)[A small single page.]
`;
const EMBEDS = [
  ['wiki plain', '![[doc.pdf]]'],
  ['wiki page 3', '![[doc.pdf#page=3]]'],
  ['wiki height 300', '![[doc.pdf#height=300]]'],
  ['wiki page 2 height 200', '![[doc.pdf#page=2&height=200]]'],
  ['wiki alias', '![[doc.pdf|alias text]]'],
  ['wiki page 3 alias', '![[doc.pdf#page=3|alias]]'],
  ['wiki missing', '![[missing.pdf]]'],
  ['wiki broken', '![[broken.pdf]]'],
  ['wiki small', '![[small.pdf]]'],
  ['md plain', '![](doc.pdf)'],
  ['md alt page 2', '![alt](doc.pdf#page=2)'],
];
const LINKS = [
  ['link plain', '[[doc.pdf]]'],
  ['link page 4', '[[doc.pdf#page=4]]'],
  ['link alias', '[[doc.pdf|aliased link]]'],
  ['md link page 2', '[text](doc.pdf#page=2)'],
];
if (phase === 'fixtures') {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'pdf-probe-typst-'));
  fs.writeFileSync(path.join(tmp, 'doc.typ'), DOC_TYP);
  fs.writeFileSync(path.join(tmp, 'small.typ'), SMALL_TYP);
  execSync('typst compile doc.typ doc.pdf', { cwd: tmp, stdio: 'inherit' });
  execSync('typst compile small.typ small.pdf', { cwd: tmp, stdio: 'inherit' });
  fs.copyFileSync(path.join(tmp, 'doc.pdf'), path.join(VAULT, 'doc.pdf'));
  fs.copyFileSync(path.join(tmp, 'small.pdf'), path.join(VAULT, 'small.pdf'));
  // 200 bytes: a real header, then deterministic garbage (an LCG, so the bytes are reproducible).
  const head = Buffer.from('%PDF-1.4\n');
  const junk = Buffer.alloc(200 - head.length);
  let x = 12345; for (let i = 0; i < junk.length; i++) { x = (x * 1103515245 + 12345) & 0x7fffffff; junk[i] = (x >> 16) & 0xff; }
  fs.writeFileSync(path.join(VAULT, 'broken.pdf'), Buffer.concat([head, junk]));
  const embeds = EMBEDS.map(([label, src]) => `Embed: ${label}\n\n${src}\n`).join('\n');
  const links = LINKS.map(([label, src]) => `Link: ${label} ${src}\n`).join('\n');
  fs.writeFileSync(path.join(VAULT, 'Embeds.md'), `# Embeds\n\n${embeds}\n# Links\n\n${links}`);
  const canvas = { nodes: [
    { id: 'f1', type: 'file', file: 'doc.pdf', x: 0, y: 0, width: 600, height: 500 },
    { id: 'f2', type: 'file', file: 'doc.pdf', subpath: '#page=2', x: 700, y: 0, width: 600, height: 500 },
    { id: 't1', type: 'text', text: '![[doc.pdf#page=4]]', x: 0, y: 600, width: 600, height: 500 },
  ], edges: [] };
  fs.writeFileSync(path.join(VAULT, 'Board.canvas'), JSON.stringify(canvas, null, '\t') + '\n');
  const para = (i) => `Paragraph ${i}. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\n`;
  const tall = [];
  for (let i = 1; i <= 40; i++) tall.push(para(i));
  tall.push('![[doc.pdf]]\n');
  for (let i = 41; i <= 80; i++) tall.push(para(i));
  fs.writeFileSync(path.join(VAULT, 'Tall.md'), '# Tall\n\n' + tall.join('\n'));
  const doc = fs.readFileSync(path.join(VAULT, 'doc.pdf'));
  out.fixtures = {
    files: fs.readdirSync(VAULT).filter((f) => f !== '.obsidian'),
    sizes: Object.fromEntries(['doc.pdf', 'small.pdf', 'broken.pdf'].map((f) => [f, fs.statSync(path.join(VAULT, f)).size])),
    docHasOutline: /\/Outlines/.test(doc.toString('latin1')),
    docMediaBoxes: Array.from(new Set(doc.toString('latin1').match(/\/MediaBox \[[^\]]*\]/g) || [])),
    typst: execSync('typst --version').toString().trim(),
  };
  save();
  process.exit(0);
}

// ---- run / after-restart ---------------------------------------------------
const c = await connect();
out.vaultPath = await c.vaultPath();
out.version = await c.evaluate(`return { title: document.title, ua: navigator.userAgent, theme: app.vault.getConfig('theme'), body: document.body.className, win: [window.innerWidth, window.innerHeight], dpr: window.devicePixelRatio };`);
save();
const ev = (b) => c.evaluate(b);
async function shot(name) {
  fs.mkdirSync(SHOTS, { recursive: true });
  const file = path.join(SHOTS, `${name}.png`);
  const r = await c.send('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(file, Buffer.from(r.data, 'base64'));
  return file;
}
const wsJson = () => { try { return JSON.parse(fs.readFileSync(path.join(VAULT, '.obsidian', 'workspace.json'), 'utf8')); } catch (e) { return { error: e.message }; } };
/** Every leaf object in workspace.json whose state.type matches, with the path it sits at. */
const wsLeaves = (ws, type) => {
  const found = [];
  const walk = (n, p) => {
    if (!n || typeof n !== 'object') return;
    if (Array.isArray(n)) { n.forEach((x, i) => walk(x, p + '[' + i + ']')); return; }
    if (n.type === 'leaf' && n.state && (!type || n.state.type === type)) found.push({ path: p, leaf: n });
    for (const k of Object.keys(n)) if (k !== 'state' || n.type !== 'leaf') walk(n[k], p + '.' + k);
  };
  walk(ws, '$');
  return found;
};
/** Custom key dispatch for keys cdp.mjs does not name ('=' '-' '0' ' '). */
const pressRaw = async (key, code, vk, mods = {}) => {
  const m = (mods.alt ? 1 : 0) | (mods.ctrl ? 2 : 0) | (mods.meta ? 4 : 0) | (mods.shift ? 8 : 0);
  const p = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: m };
  if (!(m & ~8) && key.length === 1) { p.text = key; p.unmodifiedText = key; }
  await c.send('Input.dispatchKeyEvent', { type: p.text ? 'keyDown' : 'rawKeyDown', ...p });
  await c.send('Input.dispatchKeyEvent', { type: 'keyUp', ...p });
};
const wheel = async (x, y, dy, dx = 0, modifiers = 0) => c.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: Math.round(x), y: Math.round(y), deltaX: dx, deltaY: dy, modifiers });

// Renderer-side helpers, installed once per connection.
await ev(`
window.__pdf = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  async file(p) { for (let i = 0; i < 40; i++) { const f = app.vault.getAbstractFileByPath(p); if (f) return f; await this.sleep(500); } throw new Error('missing ' + p); },
  pdfLeaf() { return app.workspace.getLeavesOfType('pdf')[0] || null; },
  async openPdf(p, opts) {
    opts = opts || {};
    // Open first, detach after: detaching every root leaf takes the tab group with it and getLeaf('tab') then fails.
    const old = opts.detach !== false ? app.workspace.getLeavesOfType('pdf') : [];
    const f = await this.file(p);
    const leaf = app.workspace.getLeaf('tab');
    await leaf.openFile(f);
    app.workspace.setActiveLeaf(leaf, { focus: true });
    for (const l of old) if (l !== leaf) l.detach();
    return leaf;
  },
  async openMd(p, mode, source) {
    const old = app.workspace.getLeavesOfType('markdown');
    const f = await this.file(p);
    const leaf = app.workspace.getLeaf('tab');
    await leaf.setViewState({ type: 'markdown', state: { file: f.path, mode: mode || 'preview', source: !!source } });
    app.workspace.setActiveLeaf(leaf, { focus: true });
    for (const l of old) if (l !== leaf) l.detach();
    return leaf;
  },
  cls(el) { if (!el) return ''; return typeof el.className === 'string' ? el.className : (el.getAttribute && el.getAttribute('class')) || ''; },
  isViewer(v) { try { return !!v && typeof v === 'object' && !(v instanceof Node) && ('currentPageNumber' in v) && ('pagesCount' in v) && ('currentScaleValue' in v); } catch (e) { return false; } },
  skip(o, k) {
    if (['app', 'leaf', 'parent', 'workspace', 'vault', 'metadataCache', 'fileManager', 'containerEl', 'contentEl', 'scope', 'keymap', 'plugins', 'internalPlugins', 'commands', 'win', 'doc', 'ownerDocument'].includes(k)) return true;
    if (o === app || o === app.workspace || o === app.vault || o === app.metadataCache || o === app.fileManager || o === app.keymap || o === app.scope || o === app.commands || o === app.plugins || o === app.internalPlugins) return true;
    if (o && o.vault && typeof o.path === 'string' && o.stat) return true;
    return false;
  },
  findViewers(root, maxDepth, budgetMs) {
    maxDepth = maxDepth || 6; budgetMs = budgetMs || 3000;
    const found = []; const seen = new Set(); const q = [[root, '', 0]]; const end = Date.now() + budgetMs;
    while (q.length && Date.now() < end && seen.size < 60000) {
      const [o, p, d] = q.shift();
      if (!o || typeof o !== 'object' || seen.has(o)) continue; seen.add(o);
      if (o instanceof Node || o instanceof Window || this.skip(o, '')) continue;
      if (this.isViewer(o)) { found.push({ path: p, viewer: o }); continue; }
      if (d >= maxDepth) continue;
      let keys; try { keys = Object.keys(o); } catch (e) { continue; }
      if (keys.length > 600) continue;
      for (const k of keys) { if (this.skip(o, k)) continue; let v; try { v = o[k]; } catch (e) { continue; } if (v && typeof v === 'object') q.push([v, p ? p + '.' + k : k, d + 1]); }
    }
    return found;
  },
  viewerOf(view) { const f = this.findViewers(view); return f.length ? f[0].viewer : null; },
  vstate(v) {
    if (!v) return null;
    const cont = v.container;
    const loc = v._location;
    return { currentPageNumber: v.currentPageNumber, currentScale: v.currentScale, currentScaleValue: v.currentScaleValue, pagesCount: v.pagesCount, spreadMode: v.spreadMode, scrollMode: v.scrollMode, pagesRotation: v.pagesRotation,
      scrollTop: cont ? cont.scrollTop : null, scrollLeft: cont ? cont.scrollLeft : null, scrollHeight: cont ? cont.scrollHeight : null, clientHeight: cont ? cont.clientHeight : null, clientWidth: cont ? cont.clientWidth : null, containerCls: cont ? this.cls(cont) : null,
      location: loc ? { pageNumber: loc.pageNumber, scale: loc.scale, top: loc.top, left: loc.left, rotation: loc.rotation, pdfOpenParams: loc.pdfOpenParams } : null,
      firstPage: this.pageInfo(cont, 1), currentPage: this.pageInfo(cont, v.currentPageNumber) };
  },
  pageInfo(cont, n) { if (!cont) return null; const el = cont.querySelector('.page[data-page-number="' + n + '"]'); if (!el) return null; const cv = el.querySelector('canvas'); return { rect: this.rect(el), loaded: el.getAttribute('data-loaded'), canvas: cv ? { w: cv.width, h: cv.height, css: [cv.style.width, cv.style.height] } : null, style: [el.style.width, el.style.height] }; },
  leafState(leaf) { if (!leaf) return null; const view = leaf.view; let st = null; try { st = view.getState ? view.getState() : null; } catch (e) { st = 'error: ' + e.message; } return { viewType: view.getViewType(), viewState: leaf.getViewState(), state: st, displayText: leaf.getDisplayText(), icon: view.getIcon ? view.getIcon() : null, viewer: this.vstate(this.viewerOf(view)) }; },
  pdfState() { return this.leafState(this.pdfLeaf()); },
  rect(el) { if (!el) return null; const b = el.getBoundingClientRect(); const r = (n) => Math.round(n * 100) / 100; return { x: r(b.x), y: r(b.y), w: r(b.width), h: r(b.height) }; },
  PROPS: ['width', 'height', 'padding', 'margin', 'gap', 'border', 'border-radius', 'box-shadow', 'background-color', 'color', 'font-size', 'font-weight', 'display', 'position', 'overflow'],
  style(el, props) { if (!el) return null; const cs = getComputedStyle(el); const o = { tag: el.tagName.toLowerCase(), cls: this.cls(el), aria: el.getAttribute('aria-label'), rect: this.rect(el), text: (el.textContent || '').trim().slice(0, 60) }; for (const p of (props || this.PROPS)) o[p] = cs.getPropertyValue(p); return o; },
  styles(sels, root) { const res = {}; const base = root || document; for (const s of sels) { try { res[s] = Array.from(base.querySelectorAll(s)).slice(0, 3).map((e) => this.style(e)); } catch (e) { res[s] = 'error: ' + e.message; } } return res; },
  styleTree(root, depth, max) {
    max = max || 220; const res = [];
    const walk = (el, p, d) => {
      if (res.length >= max || el.nodeType !== 1) return;
      const cls = this.cls(el);
      if (el.tagName === 'CANVAS' || /\\b(textLayer|annotationLayer|annotationEditorLayer)\\b/.test(cls)) return;
      const label = el.tagName.toLowerCase() + (cls ? '.' + cls.trim().split(/\\s+/).join('.') : '');
      res.push({ path: p ? p + ' > ' + label : label, ...this.style(el) });
      if (d <= 0) return;
      let i = 0; for (const ch of el.children) { if (i++ >= 12) break; walk(ch, p ? p + ' > ' + label : label, d - 1); }
    };
    walk(root, '', depth || 4);
    return res;
  },
  dump(el, depth, maxChildren) {
    depth = depth === undefined ? 8 : depth; maxChildren = maxChildren || 40;
    if (!el) return null;
    if (el.nodeType === 3) { const t = el.textContent.trim(); return t ? { text: t.slice(0, 200) } : null; }
    if (el.nodeType !== 1) return null;
    const o = { tag: el.tagName.toLowerCase() };
    const cls = this.cls(el); if (cls) o.cls = cls;
    const attrs = {}; for (const a of el.attributes) { if (a.name === 'class') continue; attrs[a.name] = a.value.length > 200 ? a.value.slice(0, 200) + '...' : a.value; } if (Object.keys(attrs).length) o.attrs = attrs;
    try { if (getComputedStyle(el).display === 'none') o.hidden = true; } catch (e) {}
    if (el.tagName === 'SELECT') o.options = Array.from(el.options).map((op) => ({ value: op.value, label: op.textContent.trim(), selected: op.selected }));
    if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') { o.value = el.value; if (el.placeholder) o.placeholder = el.placeholder; if (el.type === 'checkbox') o.checked = el.checked; }
    if (el.tagName === 'CANVAS') { o.canvas = { w: el.width, h: el.height, cssW: el.style.width, cssH: el.style.height }; return o; }
    if (el.tagName.toLowerCase() === 'svg') return o;
    if (/\\b(textLayer|annotationLayer|annotationEditorLayer)\\b/.test(cls)) { o.childCount = el.childNodes.length; o.text = el.textContent.trim().slice(0, 120); return o; }
    if (depth > 0) { const kids = []; let n = 0; for (const ch of el.childNodes) { if (n >= maxChildren) { kids.push({ more: el.childNodes.length - maxChildren }); break; } n++; const d = this.dump(ch, depth - 1, maxChildren); if (d) kids.push(d); } if (kids.length) o.children = kids; }
    else if (el.childNodes.length) { o.childCount = el.childNodes.length; o.text = el.textContent.trim().slice(0, 120); }
    return o;
  },
  sig(root) {
    const set = []; if (!root) return set;
    for (const e of root.querySelectorAll('*')) {
      if (e.closest('.textLayer, .annotationLayer, canvas')) continue;
      let hidden = false; try { hidden = getComputedStyle(e).display === 'none'; } catch (x) {}
      set.push(e.tagName.toLowerCase() + (this.cls(e) ? '.' + this.cls(e).trim().split(/\\s+/).join('.') : '') + (hidden ? '[hidden]' : ''));
    }
    return set;
  },
  delta(a, b) {
    const count = (xs) => { const m = new Map(); for (const x of xs) m.set(x, (m.get(x) || 0) + 1); return m; };
    const ma = count(a), mb = count(b); const added = [], removed = [];
    for (const [k, n] of mb) { const d = n - (ma.get(k) || 0); if (d > 0) added.push(d > 1 ? k + ' x' + d : k); }
    for (const [k, n] of ma) { const d = n - (mb.get(k) || 0); if (d > 0) removed.push(d > 1 ? k + ' x' + d : k); }
    return { added: added.slice(0, 80), removed: removed.slice(0, 80) };
  },
  ui() { return { menus: Array.from(document.querySelectorAll('.menu')).map((m) => this.cls(m)), popovers: Array.from(document.querySelectorAll('.popover, .hover-popover')).map((m) => this.cls(m)), modals: Array.from(document.querySelectorAll('.modal-container')).map((m) => this.cls(m)), notices: Array.from(document.querySelectorAll('.notice')).map((n) => n.textContent.trim()), focus: document.activeElement ? document.activeElement.tagName.toLowerCase() + '.' + this.cls(document.activeElement) : null, activeLeafType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType() }; },
  menus() {
    return Array.from(document.querySelectorAll('.menu')).map((m) => {
      const groups = Array.from(m.querySelectorAll('.menu-group'));
      return { cls: this.cls(m), rect: this.rect(m), groups: groups.length,
        items: Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map((e) => ({
          cls: this.cls(e), separator: e.classList.contains('menu-separator'), section: e.getAttribute('data-section'), group: groups.indexOf(e.closest('.menu-group')),
          title: e.querySelector('.menu-item-title') ? e.querySelector('.menu-item-title').textContent.trim() : null,
          text: (e.textContent || '').trim() || null,
          icon: e.querySelector('.menu-item-icon svg') ? e.querySelector('.menu-item-icon svg').getAttribute('class') : null,
          aria: e.getAttribute('aria-label'),
          checked: e.classList.contains('mod-checked') || e.getAttribute('aria-checked') === 'true' || !!e.querySelector('.menu-item-icon.mod-checked'),
          disabled: e.classList.contains('is-disabled') || e.classList.contains('mod-disabled'),
          submenu: e.classList.contains('mod-submenu') || /chevron-right/.test(e.innerHTML),
          h: e.offsetHeight, visible: !!e.offsetParent && e.offsetHeight > 0,
        })),
        html: m.outerHTML.slice(0, 30000) };
    });
  },
  dismiss() { for (const m of document.querySelectorAll('.menu')) m.remove(); return true; },
  leaves() { const r = []; app.workspace.iterateAllLeaves((l) => { if (!l.view) { r.push({ id: l.id, type: '(no view yet)' }); return; } r.push({ id: l.id, type: l.view.getViewType(), file: l.view.file && l.view.file.path, root: l.getRoot() === app.workspace.rootSplit, active: l === app.workspace.activeLeaf, display: l.getDisplayText() }); }); return r; },
  viewContent(leaf) { return (leaf || this.pdfLeaf()).view.containerEl.querySelector('.view-content'); },
  // The embeds in a markdown preview or editor, with the pdf viewer each one owns (found through the leaf's object graph, matched by container).
  embedsOf(leaf, root) {
    root = root || leaf.view.containerEl;
    const viewers = this.findViewers(leaf.view, 9, 4000);
    const els = Array.from(root.querySelectorAll('.internal-embed, .cm-embed-block, img[src], span.image-embed, .pdf-embed'));
    const seen = new Set(); const res = [];
    for (const el of els) {
      if (el.matches('.cm-embed-block') && el.querySelector('.internal-embed')) continue; // the inner one is the embed
      if (el.closest('.internal-embed') && el.closest('.internal-embed') !== el) continue;
      if (seen.has(el)) continue; seen.add(el);
      const v = viewers.find((x) => x.viewer.container && el.contains(x.viewer.container));
      res.push({ el, viewer: v ? v.viewer : null, viewerPath: v ? v.path : null, wrapper: el.closest('.cm-embed-block') ? this.cls(el.closest('.cm-embed-block')) : null, wrapperRect: el.closest('.cm-embed-block') ? this.rect(el.closest('.cm-embed-block')) : null });
    }
    return res;
  },
  embedInfo(e, depth) {
    const el = e.el; const inner = el.querySelector('[class*=toolbar]');
    const first = el.querySelector('.page');
    return { cls: this.cls(el), attrs: Object.fromEntries(Array.from(el.attributes).filter((a) => a.name !== 'class').map((a) => [a.name, a.value.slice(0, 200)])), rect: this.rect(el), wrapper: e.wrapper, wrapperRect: e.wrapperRect,
      viewerPath: e.viewerPath, viewer: this.vstate(e.viewer),
      toolbar: inner ? this.dump(inner, 4) : null, hasSidebar: !!el.querySelector('[class*=sidebar]'), hasFindbar: !!el.querySelector('[class*=find]'),
      pages: Array.from(el.querySelectorAll('.page')).map((p) => ({ n: p.getAttribute('data-page-number'), loaded: p.getAttribute('data-loaded'), rect: this.rect(p), canvas: !!p.querySelector('canvas') })).slice(0, 8),
      firstPageRect: first ? this.rect(first) : null,
      box: this.style(el), text: el.textContent.trim().slice(0, 200), dump: this.dump(el, depth || 6, 12) };
  },
};
return Object.keys(window.__pdf).length;`);

const P = 'const P = window.__pdf;';

// ---- after-restart ---------------------------------------------------------
if (phase === 'after-restart') {
  await step('s18-after-restart', async () => {
    await sleep(3500);
    const r = await ev(`${P} const leaf = P.pdfLeaf(); return { leaves: P.leaves(), pdf: P.leafState(leaf), activeType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType(), tabTitle: document.querySelector('.mod-root .workspace-tab-header.is-active') && document.querySelector('.mod-root .workspace-tab-header.is-active').getAttribute('aria-label') };`);
    await sleep(1500);
    const again = await ev(`${P} return P.pdfState();`);
    const shotFile = await shot('18-after-restart');
    const history = await ev(`${P} const o = (P.pdfLeaf() && P.pdfLeaf().view.viewer && P.pdfLeaf().view.viewer.child && P.pdfLeaf().view.viewer.child.pdfViewer); const fp = o && o.pdfDocument && o.pdfDocument.fingerprints && o.pdfDocument.fingerprints[0]; let h = null; try { h = JSON.parse(localStorage.getItem('pdfjs.history') || '{"files":[]}'); } catch (e) {} return { fingerprint: fp, entry: fp && h ? (h.files || []).find((f) => f.fingerprint === fp) || null : null, sidebarView: localStorage.getItem('pdfjs.sidebarView') };`);
    const lastOpen = wsJson().lastOpenFiles;
    // Leave the shared localStorage as it was found: drop the probe's own entries.
    const stripped = await ev(`${P} const fp = ${JSON.stringify('__FP__')}; let h; try { h = JSON.parse(localStorage.getItem('pdfjs.history') || '{"files":[]}'); } catch (e) { return null; } const o = (P.pdfLeaf() && P.pdfLeaf().view.viewer && P.pdfLeaf().view.viewer.child && P.pdfLeaf().view.viewer.child.pdfViewer); const mine = o && o.pdfDocument && o.pdfDocument.fingerprints ? o.pdfDocument.fingerprints[0] : null; const before = (h.files || []).length; h.files = (h.files || []).filter((f) => f.fingerprint !== mine); localStorage.setItem('pdfjs.history', JSON.stringify(h)); return { before, after: h.files.length, mine };`);
    return { ...r, afterSettle: again, history, lastOpenFiles: lastOpen, workspaceJsonPdfLeaves: wsLeaves(wsJson(), 'pdf'), shot: shotFile, strippedOnExit: stripped };
  });
  out.finishedAt = new Date().toISOString();
  save();
  c.close();
  process.exit(0);
}

// ---- run -------------------------------------------------------------------
await ev(`app.workspace.rightSplit.collapse(); app.workspace.leftSplit.expand(); return true;`);
await sleep(300);

const fresh = async (p = 'doc.pdf', wait = 2500) => { await ev(`${P} app.workspace.leftSplit.expand(); app.workspace.rightSplit.collapse(); await P.openPdf(${JSON.stringify(p)}); return true;`); await sleep(wait); };

await step('s00-explore', async (partial) => {
  await fresh();
  const r = await ev(`${P}
    const leaf = P.pdfLeaf(); if (!leaf) throw new Error('no pdf leaf');
    const view = leaf.view;
    const header = leaf.tabHeaderEl || document.querySelector('.mod-root .workspace-tab-header.is-active');
    const one = (o) => { try { return o && typeof o === 'object' ? Object.keys(o).slice(0, 80) : typeof o; } catch (e) { return 'error'; } };
    const viewers = P.findViewers(view, 6, 4000).map((x) => ({ path: x.path, ctor: x.viewer.constructor && x.viewer.constructor.name, keys: Object.keys(x.viewer).slice(0, 60) }));
    const proto = (o) => { const names = []; let p = Object.getPrototypeOf(o); let d = 0; while (p && p !== Object.prototype && d++ < 4) { names.push({ ctor: p.constructor && p.constructor.name, methods: Object.getOwnPropertyNames(p).slice(0, 80) }); p = Object.getPrototypeOf(p); } return names; };
    const viewer = P.viewerOf(view);
    // The object that owns the viewer (pdf.js's application-like object): its keys name the sidebar, find bar and toolbar.
    const vpath = viewers.length ? viewers[0].path : null;
    let owner = null, ownerPath = null;
    if (vpath && vpath.includes('.')) { ownerPath = vpath.slice(0, vpath.lastIndexOf('.')); owner = ownerPath.split('.').reduce((o, k) => (o ? o[k] : null), view); }
    return {
      title: document.title, viewType: view.getViewType(), viewState: leaf.getViewState(), state: view.getState(), displayText: leaf.getDisplayText(), icon: view.getIcon(),
      ctor: view.constructor && view.constructor.name, viewProto: proto(view),
      tabHeader: header ? { aria: header.getAttribute('aria-label'), cls: P.cls(header), iconSvg: header.querySelector('svg') && header.querySelector('svg').getAttribute('class'), title: header.querySelector('.workspace-tab-header-inner-title') && header.querySelector('.workspace-tab-header-inner-title').textContent } : null,
      viewKeys: Object.keys(view),
      oneDown: Object.fromEntries(['viewer', 'pdfViewer', 'child', 'renderer', 'pdfApp', 'app', 'component'].filter((k) => k in view).map((k) => [k, one(view[k])])),
      twoDown: Object.fromEntries(['viewer', 'pdfViewer', 'child', 'renderer'].filter((k) => view[k] && typeof view[k] === 'object').flatMap((k) => ['viewer', 'pdfViewer', 'child', 'renderer', 'pdfApp', 'toolbar', 'sidebar', 'findBar', 'pdfSidebar', 'pdfFindBar'].filter((j) => view[k][j] && typeof view[k][j] === 'object').map((j) => [k + '.' + j, one(view[k][j])]))),
      viewers, viewerPath: vpath, ownerPath, ownerKeys: one(owner), ownerProto: owner ? proto(owner) : null,
      viewer: P.vstate(viewer),
      viewerProto: viewer ? proto(viewer) : null,
      viewerMisc: viewer ? { scrollModeNames: ['VERTICAL', 'HORIZONTAL', 'WRAPPED', 'PAGE'], spreadModeNames: ['NONE', 'ODD', 'EVEN'], isInPresentationMode: viewer.isInPresentationMode, hasTextLayer: !!viewer.textLayerMode, textLayerMode: viewer.textLayerMode, annotationMode: viewer.annotationMode, maxCanvasPixels: viewer.maxCanvasPixels, viewerEl: viewer.viewer && P.cls(viewer.viewer) } : null,
      leaves: P.leaves(), ui: P.ui(),
      docTitle: (viewer && viewer.pdfDocument) ? 'pdfDocument present' : 'no pdfDocument',
    };`);
  partial.r = r;
  r.shot = await shot('00-viewer-default');
  return r;
});

await step('s01-markup', async (partial) => {
  await fresh();
  const dark = await ev(`${P}
    const leaf = P.pdfLeaf(); const root = leaf.view.containerEl;
    const header = root.querySelector('.view-header'); const content = root.querySelector('.view-content');
    const toolbar = content.querySelector('[class*=toolbar]');
    const pages = Array.from(content.querySelectorAll('.page')).slice(0, 2);
    const gap = pages.length === 2 ? Math.round((pages[1].getBoundingClientRect().top - pages[0].getBoundingClientRect().bottom) * 100) / 100 : null;
    return {
      theme: { config: app.vault.getConfig('theme'), body: document.body.className },
      header: P.dump(header, 6), headerTitle: header.querySelector('.view-header-title') && header.querySelector('.view-header-title').textContent, headerActions: Array.from(header.querySelectorAll('.view-actions .clickable-icon, .view-actions button')).map((b) => ({ aria: b.getAttribute('aria-label'), icon: b.querySelector('svg') && b.querySelector('svg').getAttribute('class'), cls: P.cls(b) })),
      content: P.dump(content, 9, 30),
      styleTree: P.styleTree(content, 5),
      toolbar: toolbar ? { rect: P.rect(toolbar), style: P.style(toolbar), controls: Array.from(toolbar.querySelectorAll('*')).filter((e) => e.matches('button, .clickable-icon, input, select, [role=button]') && !e.parentElement.closest('button, .clickable-icon')).map((e) => ({ ...P.style(e), icon: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), value: e.value, placeholder: e.placeholder })) } : null,
      pages: pages.map((p) => ({ ...P.style(p), n: p.getAttribute('data-page-number'), canvas: p.querySelector('canvas') ? { ...P.style(p.querySelector('canvas')), w: p.querySelector('canvas').width, h: p.querySelector('canvas').height } : null, wrapper: p.querySelector('.canvasWrapper') ? P.style(p.querySelector('.canvasWrapper')) : null })),
      pageGap: gap,
      container: P.styles(['[class*=viewer-container]', '[class*=viewerContainer]', '.pdfViewer', '[class*=sidebar]', '[class*=sidebar] [role=tab], [class*=sidebar] button, [class*=sidebar] .clickable-icon', '[class*=find]'], content),
      viewContent: P.style(content), viewHeader: P.style(header),
      vars: Object.fromEntries(['--pdf-background', '--pdf-page-margin', '--pdf-sidebar-background', '--pdf-toolbar-background', '--pdf-spread-color', '--pdf-thumbnail-shadow', '--pdf-shadow', '--pdf-page-background'].map((v) => [v, getComputedStyle(content).getPropertyValue(v).trim()])),
    };`);
  partial.dark = dark;
  await ev(`app.changeTheme('moonstone'); return true;`); await sleep(1200);
  const light = await ev(`${P}
    const leaf = P.pdfLeaf(); const root = leaf.view.containerEl; const content = root.querySelector('.view-content');
    const toolbar = content.querySelector('[class*=toolbar]');
    return { theme: { config: app.vault.getConfig('theme'), body: document.body.className }, styleTree: P.styleTree(content, 5),
      toolbar: toolbar ? { style: P.style(toolbar), controls: Array.from(toolbar.querySelectorAll('*')).filter((e) => e.matches('button, .clickable-icon, input, select, [role=button]') && !e.parentElement.closest('button, .clickable-icon')).map((e) => P.style(e)) } : null,
      pages: Array.from(content.querySelectorAll('.page')).slice(0, 2).map((p) => P.style(p)), viewContent: P.style(content), viewHeader: P.style(root.querySelector('.view-header')),
      vars: Object.fromEntries(['--pdf-background', '--pdf-page-margin', '--pdf-sidebar-background', '--pdf-toolbar-background', '--pdf-spread-color', '--pdf-thumbnail-shadow', '--pdf-shadow', '--pdf-page-background'].map((v) => [v, getComputedStyle(content).getPropertyValue(v).trim()])) };`);
  const lightShot = await shot('01-light-theme');
  await ev(`app.changeTheme('obsidian'); return true;`); await sleep(800);
  return { dark, light, lightShot };
});

await step('s02-zoom-default', async () => {
  await fresh();
  const read = () => ev(`${P} const s = P.pdfState(); const leafEl = P.pdfLeaf().view.containerEl; return { viewer: s.viewer, state: s.state, leafWidth: P.rect(leafEl).w, viewContentWidth: P.rect(leafEl.querySelector('.view-content')).w, leftCollapsed: app.workspace.leftSplit.collapsed, win: [window.innerWidth, window.innerHeight] };`);
  const initial = await read();
  await ev(`app.workspace.leftSplit.collapse(); return true;`); await sleep(500);
  await ev(`app.workspace.onResize && app.workspace.onResize(); return true;`); await sleep(1200);
  const collapsed = await read();
  await ev(`app.workspace.leftSplit.expand(); return true;`); await sleep(500);
  await ev(`app.workspace.onResize && app.workspace.onResize(); return true;`); await sleep(1200);
  const expanded = await read();
  // The same on a second fresh open: is the default stable?
  await fresh();
  const second = await read();
  return { initial, collapsed, expanded, second };
});

await step('s03-toolbar', async (partial) => {
  await fresh();
  const controls = await ev(`${P}
    const content = P.viewContent(); const toolbar = content.querySelector('[class*=toolbar]');
    if (!toolbar) return { toolbarCls: null, controls: [] };
    const els = Array.from(toolbar.querySelectorAll('*')).filter((e) => e.matches('button, .clickable-icon, input, select, [role=button]') && !(e.parentElement && e.parentElement.closest('button, .clickable-icon')));
    window.__pdfControls = els;
    return { toolbarCls: P.cls(toolbar), controls: els.map((e, i) => ({ i, tag: e.tagName.toLowerCase(), cls: P.cls(e), aria: e.getAttribute('aria-label'), title: e.getAttribute('title'), icon: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), text: e.textContent.trim().slice(0, 40), value: e.value, placeholder: e.placeholder, rect: P.rect(e), hidden: getComputedStyle(e).display === 'none' })) };`);
  partial.controls = controls;
  const clicks = [];
  for (const ctl of controls.controls) {
    if (ctl.hidden || ctl.rect.w === 0) { clicks.push({ ...ctl, skipped: 'hidden' }); continue; }
    await fresh('doc.pdf', 2200);
    const before = await ev(`${P} const content = P.viewContent(); window.__sigBefore = P.sig(content); return { state: P.pdfState(), ui: P.ui(), sidebarOpen: !!content.querySelector('[class*=sidebar]:not([style*="display: none"])') };`);
    // Re-find the control in the fresh view by index (the toolbar is rebuilt per open).
    const r = await ev(`${P} const content = P.viewContent(); const toolbar = content.querySelector('[class*=toolbar]');
      const els = Array.from(toolbar.querySelectorAll('*')).filter((e) => e.matches('button, .clickable-icon, input, select, [role=button]') && !(e.parentElement && e.parentElement.closest('button, .clickable-icon')));
      const e = els[${ctl.i}]; if (!e) return null; e.scrollIntoView({ block: 'nearest' }); return P.rect(e);`);
    if (!r) { clicks.push({ ...ctl, skipped: 'not found on re-open' }); continue; }
    const one = { ...ctl };
    if (ctl.tag === 'select') {
      one.select = await ev(`${P} const toolbar = P.viewContent().querySelector('[class*=toolbar]'); const el = toolbar.querySelectorAll('select')[0]; return { value: el.value, options: Array.from(el.options).map((o) => ({ value: o.value, label: o.textContent.trim(), selected: o.selected })) };`);
      clicks.push(one); continue;
    }
    await c.clickAt(r.x + r.w / 2, r.y + r.h / 2); await sleep(900);
    const after = await ev(`${P} const content = P.viewContent(); const sig = P.sig(content); const menus = P.menus();
      return { state: P.pdfState(), ui: P.ui(), delta: P.delta(window.__sigBefore, sig), menus, popovers: Array.from(document.querySelectorAll('.popover, .hover-popover')).map((p) => P.dump(p, 5)), focus: document.activeElement && (document.activeElement.tagName + '.' + P.cls(document.activeElement)), focusValue: document.activeElement && document.activeElement.value };`);
    one.before = { state: before.state.state, viewer: before.state.viewer, ui: before.ui };
    one.after = after;
    if (ctl.tag === 'input') {
      // The page input: select all, type 3, Enter.
      await c.pressKey('a', { ctrl: true }); await c.typeText('3'); await c.pressKey('Enter'); await sleep(900);
      one.afterTyping3Enter = await ev(`${P} return { state: P.pdfState(), focus: document.activeElement && (document.activeElement.tagName + '.' + P.cls(document.activeElement)), value: document.activeElement && document.activeElement.value, inputs: Array.from(P.viewContent().querySelectorAll('[class*=toolbar] input')).map((i) => i.value) };`);
    }
    if (after.menus.length) { one.menuItems = await c.openMenuItems(); await ev(`${P} P.dismiss(); return true;`); await sleep(200); }
    clicks.push(one);
    partial.clicks = clicks;
  }
  // The sidebar: open through its control, read the tabs and the outline.
  const sidebarCtl = controls.controls.find((x) => /sidebar|outline|thumbnail|panel/i.test((x.aria || '') + ' ' + (x.icon || '') + ' ' + x.cls));
  const sidebar = {};
  await fresh('doc.pdf', 2200);
  if (sidebarCtl) {
    const r = await ev(`${P} const toolbar = P.viewContent().querySelector('[class*=toolbar]'); const els = Array.from(toolbar.querySelectorAll('*')).filter((e) => e.matches('button, .clickable-icon, input, select, [role=button]') && !(e.parentElement && e.parentElement.closest('button, .clickable-icon'))); return P.rect(els[${sidebarCtl.i}]);`);
    await c.clickAt(r.x + r.w / 2, r.y + r.h / 2); await sleep(1200);
    sidebar.viaControl = sidebarCtl.aria || sidebarCtl.icon;
  }
  sidebar.open = await ev(`${P} const content = P.viewContent(); const sb = Array.from(content.querySelectorAll('[class*=sidebar]')).find((e) => getComputedStyle(e).display !== 'none');
    if (!sb) return { found: false, candidates: Array.from(content.querySelectorAll('[class*=sidebar]')).map((e) => ({ cls: P.cls(e), display: getComputedStyle(e).display, rect: P.rect(e) })) };
    window.__sb = sb;
    const tabs = Array.from(sb.querySelectorAll('button, .clickable-icon, [role=tab]')).filter((e) => !e.closest('[class*=outline], [class*=thumbnail], [class*=attachment]') || e.matches('[role=tab]'));
    return { found: true, cls: P.cls(sb), rect: P.rect(sb), style: P.style(sb), dump: P.dump(sb, 7, 20), tabs: tabs.map((t) => ({ cls: P.cls(t), aria: t.getAttribute('aria-label'), icon: t.querySelector('svg') && t.querySelector('svg').getAttribute('class'), text: t.textContent.trim().slice(0, 40), rect: P.rect(t), style: P.style(t) })), state: P.pdfState().state, viewer: P.pdfState().viewer };`);
  if (sidebar.open && sidebar.open.found) {
    sidebar.tabs = [];
    for (let i = 0; i < Math.min(sidebar.open.tabs.length, 5); i++) {
      const t = sidebar.open.tabs[i];
      if (!t.rect || t.rect.w === 0) continue;
      await c.clickAt(t.rect.x + t.rect.w / 2, t.rect.y + t.rect.h / 2); await sleep(1200);
      const r = await ev(`${P} const sb = window.__sb; const body = Array.from(sb.children).map((ch) => ({ cls: P.cls(ch), display: getComputedStyle(ch).display, rect: P.rect(ch), text: ch.textContent.trim().slice(0, 300) }));
        const rows = Array.from(sb.querySelectorAll('.tree-item, .treeItem, .outlineItem, [class*=outline-item], [class*=thumbnail]')).filter((e) => e.offsetParent).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 80), rect: P.rect(e), aria: e.getAttribute('aria-label'), data: Object.fromEntries(Array.from(e.attributes).filter((a) => a.name.startsWith('data-')).map((a) => [a.name, a.value])) }));
        return { tab: ${JSON.stringify(t.aria || t.icon || t.text)}, active: Array.from(sb.querySelectorAll('.is-active, [aria-selected=true], .toggled')).map((e) => P.cls(e)), body, rows: rows.slice(0, 40), dump: P.dump(sb, 8, 24), state: P.pdfState().viewer };`);
      sidebar.tabs.push(r);
    }
    // The outline: click the row for page three.
    const outlineRow = await ev(`${P} const sb = window.__sb;
      const tabs = Array.from(sb.querySelectorAll('button, .clickable-icon, [role=tab]'));
      const ot = tabs.find((t) => /outline/i.test((t.getAttribute('aria-label') || '') + (t.querySelector('svg') ? t.querySelector('svg').getAttribute('class') : '')));
      if (ot) ot.click();
      await P.sleep(800);
      const rows = Array.from(sb.querySelectorAll('*')).filter((e) => e.offsetParent && e.children.length <= 3 && /^3 Three$|^Three$|Three/.test(e.textContent.trim()) && e.textContent.trim().length < 20);
      const row = rows[rows.length - 1];
      return row ? { cls: P.cls(row), text: row.textContent.trim(), rect: P.rect(row), parentCls: P.cls(row.parentElement), html: row.parentElement.outerHTML.slice(0, 600) } : null;`);
    sidebar.outlineRowThree = outlineRow;
    if (outlineRow) {
      await c.clickAt(outlineRow.rect.x + Math.min(20, outlineRow.rect.w / 2), outlineRow.rect.y + outlineRow.rect.h / 2); await sleep(1200);
      sidebar.afterOutlineClick = await ev(`${P} const s = P.pdfState(); return { state: s.state, viewer: s.viewer, sidebarStillOpen: window.__sb && getComputedStyle(window.__sb).display !== 'none' && !!window.__sb.offsetParent };`);
      sidebar.shot = await shot('03-sidebar-outline');
    }
  }
  partial.sidebar = sidebar;
  // The find bar.
  const find = {};
  await fresh('doc.pdf', 2200);
  const findCtl = controls.controls.find((x) => /find|search/i.test((x.aria || '') + ' ' + (x.icon || '') + ' ' + x.cls));
  if (findCtl) {
    const r = await ev(`${P} const toolbar = P.viewContent().querySelector('[class*=toolbar]'); const els = Array.from(toolbar.querySelectorAll('*')).filter((e) => e.matches('button, .clickable-icon, input, select, [role=button]') && !(e.parentElement && e.parentElement.closest('button, .clickable-icon'))); return P.rect(els[${findCtl.i}]);`);
    await c.clickAt(r.x + r.w / 2, r.y + r.h / 2); await sleep(900);
    find.via = 'control ' + (findCtl.aria || findCtl.icon);
  } else {
    await c.clickElement('.workspace-leaf.mod-active .page'); await sleep(200);
    await c.pressKey('f', { ctrl: true }); await sleep(900);
    find.via = 'ctrl+f';
  }
  find.open = await ev(`${P} const content = P.viewContent(); const fb = Array.from(content.querySelectorAll('[class*=find]')).find((e) => getComputedStyle(e).display !== 'none' && e.offsetParent);
    if (!fb) return { found: false, candidates: Array.from(content.querySelectorAll('[class*=find], [class*=search]')).map((e) => ({ cls: P.cls(e), display: getComputedStyle(e).display })), focus: document.activeElement && (document.activeElement.tagName + '.' + P.cls(document.activeElement)) };
    window.__fb = fb;
    return { found: true, cls: P.cls(fb), rect: P.rect(fb), style: P.style(fb), dump: P.dump(fb, 7, 30), styleTree: P.styleTree(fb, 4, 60), focus: document.activeElement && (document.activeElement.tagName + '.' + P.cls(document.activeElement)), focusInside: fb.contains(document.activeElement) };`);
  if (find.open.found) {
    if (!find.open.focusInside) { const ir = await ev(`${P} const i = window.__fb.querySelector('input'); return P.rect(i);`); if (ir) await c.clickAt(ir.x + ir.w / 2, ir.y + ir.h / 2); await sleep(150); }
    await c.typeText('three'); await sleep(800);
    find.afterTyping = await ev(`${P} const fb = window.__fb; return { text: fb.textContent.trim().slice(0, 300), labels: Array.from(fb.querySelectorAll('span, label, .pdf-find-result, [class*=result], [class*=count]')).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 80) })).filter((x) => x.text), state: P.pdfState().viewer, highlighted: fb.closest('.view-content').querySelectorAll('.highlight, .textLayer .highlight').length };`);
    await c.pressKey('Enter'); await sleep(1200);
    find.afterEnter = await ev(`${P} const fb = window.__fb; return { text: fb.textContent.trim().slice(0, 300), labels: Array.from(fb.querySelectorAll('span, label, [class*=result], [class*=count], [class*=message]')).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 80) })).filter((x) => x.text), inputs: Array.from(fb.querySelectorAll('input')).map((i) => ({ type: i.type, value: i.value, checked: i.checked, cls: P.cls(i), id: i.id, placeholder: i.placeholder })), state: P.pdfState().viewer, highlighted: Array.from(fb.closest('.view-content').querySelectorAll('.textLayer .highlight')).map((h) => ({ cls: P.cls(h), text: h.textContent, page: h.closest('.page') && h.closest('.page').getAttribute('data-page-number'), style: P.style(h) })).slice(0, 10) };`);
    find.shot = await shot('03-find-match');
    await c.pressKey('Enter'); await sleep(900);
    find.afterSecondEnter = await ev(`${P} const fb = window.__fb; return { text: fb.textContent.trim().slice(0, 200), state: P.pdfState().viewer };`);
    await c.pressKey('Escape'); await sleep(600);
    find.afterEscape = await ev(`${P} const fb = window.__fb; return { display: getComputedStyle(fb).display, hidden: !fb.offsetParent, inDom: document.contains(fb), focus: document.activeElement && (document.activeElement.tagName + '.' + P.cls(document.activeElement)), highlightsLeft: fb.closest('.view-content') ? fb.closest('.view-content').querySelectorAll('.textLayer .highlight').length : null };`);
  }
  return { controls, clicks, sidebar, find };
});

await step('s04-zoom-select', async (partial) => {
  await fresh();
  const sel = await ev(`${P} const s = P.viewContent().querySelector('[class*=toolbar] select, .view-content select'); if (!s) return null; s.setAttribute('data-probe', 'zoom'); return { cls: P.cls(s), value: s.value, options: Array.from(s.options).map((o) => ({ value: o.value, label: o.textContent.trim(), selected: o.selected })), style: P.style(s), rect: P.rect(s) };`);
  partial.select = sel;
  if (!sel) return { select: null, note: 'no <select> in the view' };
  const read = () => ev(`${P} const s = P.pdfState(); const sel = document.querySelector('select[data-probe=zoom]'); return { selectValue: sel && sel.value, selectLabel: sel && sel.options[sel.selectedIndex] && sel.options[sel.selectedIndex].textContent.trim(), state: s.state, viewer: s.viewer };`);
  const picks = {};
  const want = [];
  const fixed = sel.options.find((o) => /^1(\.0+)?$|100/.test(o.value) || /100/.test(o.label)) || sel.options.find((o) => /^[0-9.]+$/.test(o.value) && o.value !== sel.value);
  const fit = sel.options.find((o) => /fit/i.test(o.value) || /fit/i.test(o.label));
  const width = sel.options.find((o) => /width/i.test(o.value) || /width/i.test(o.label));
  const auto = sel.options.find((o) => /auto/i.test(o.value) || /auto/i.test(o.label));
  for (const o of [fixed, fit, width, auto]) if (o && !want.includes(o)) want.push(o);
  for (const o of want) {
    await c.selectSet('select[data-probe=zoom]', o.value); await sleep(1000);
    picks[o.value] = { label: o.label, ...(await read()) };
    if (o === fit) picks[o.value].shot = await shot('04-page-fit');
  }
  return { select: sel, picks };
});

await step('s05-keyboard', async (partial) => {
  await fresh();
  const scope = await ev(`${P} const leaf = P.pdfLeaf(); const v = leaf.view;
    const ser = (s) => s && s.keys ? s.keys.map((k) => ({ modifiers: k.modifiers, key: k.key })) : null;
    const cmds = Object.keys(app.commands.commands).filter((id) => /pdf/i.test(id)).map((id) => ({ id, name: app.commands.commands[id].name, defaultHotkeys: app.hotkeyManager.getDefaultHotkeys(id), hotkeys: app.hotkeyManager.getHotkeys(id) }));
    return { viewScope: ser(v.scope), hasScope: !!v.scope, leafScope: ser(leaf.scope), commands: cmds, keymapPushed: app.keymap && app.keymap.getRootScope ? !!app.keymap.getRootScope() : null, modalScope: null };`);
  partial.scope = scope;
  const pageRect = await c.rect('.workspace-leaf.mod-active .page[data-page-number="1"]');
  if (!pageRect) throw new Error('no page 1');
  await c.clickAt(pageRect.x + pageRect.w / 2, pageRect.y + 40); await sleep(300);
  const focus0 = await ev(`${P} return P.ui();`);
  const reset = async () => { await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentScaleValue = window.__defaultScaleValue; v.pagesRotation = 0; v.currentPageNumber = 1; v.container.scrollTop = 0; P.dismiss(); for (const m of document.querySelectorAll('.modal-container')) m.remove(); return true;`); await sleep(400); };
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); window.__defaultScaleValue = v.currentScaleValue; return true;`);
  const read = () => ev(`${P} const s = P.pdfState(); const content = P.viewContent(); return { page: s.viewer && s.viewer.currentPageNumber, scrollTop: s.viewer && s.viewer.scrollTop, scrollLeft: s.viewer && s.viewer.scrollLeft, scale: s.viewer && s.viewer.currentScale, scaleValue: s.viewer && s.viewer.currentScaleValue, rotation: s.viewer && s.viewer.pagesRotation, spread: s.viewer && s.viewer.spreadMode, scrollMode: s.viewer && s.viewer.scrollMode, state: s.state, ui: P.ui(), findbarOpen: !!Array.from(content.querySelectorAll('[class*=find]')).find((e) => e.offsetParent && getComputedStyle(e).display !== 'none'), presentation: !!document.fullscreenElement };`);
  const keys = [
    ['Down', () => c.pressKey('ArrowDown')], ['Up', () => c.pressKey('ArrowUp')],
    ['PageDown', () => c.pressKey('PageDown')], ['PageUp', () => c.pressKey('PageUp')],
    ['End', () => c.pressKey('End')], ['Home', () => c.pressKey('Home')],
    ['Space', () => pressRaw(' ', 'Space', 32)], ['Shift+Space', () => pressRaw(' ', 'Space', 32, { shift: true })],
    ['j', () => c.pressKey('j')], ['k', () => c.pressKey('k')], ['n', () => c.pressKey('n')], ['p', () => c.pressKey('p')],
    ['Right', () => c.pressKey('ArrowRight')], ['Left', () => c.pressKey('ArrowLeft')],
    ['Ctrl+=', () => pressRaw('=', 'Equal', 187, { ctrl: true })], ['Ctrl+-', () => pressRaw('-', 'Minus', 189, { ctrl: true })], ['Ctrl+0', () => pressRaw('0', 'Digit0', 48, { ctrl: true })],
    ['Ctrl+Shift+=', () => pressRaw('+', 'Equal', 187, { ctrl: true, shift: true })],
    ['r', () => c.pressKey('r')], ['Shift+r', () => pressRaw('R', 'KeyR', 82, { shift: true })],
    ['Ctrl+f', () => c.pressKey('f', { ctrl: true })], ['Escape', () => c.pressKey('Escape')],
    ['Ctrl+p', () => c.pressKey('p', { ctrl: true })], ['Ctrl+o', () => c.pressKey('o', { ctrl: true })], ['Ctrl+e', () => c.pressKey('e', { ctrl: true })],
    ['Ctrl+a', () => c.pressKey('a', { ctrl: true })],
  ];
  const rows = [];
  for (const [name, press] of keys) {
    if (name !== 'Escape') await reset();
    if (['Up', 'PageUp', 'Shift+Space', 'k', 'p', 'Left'].includes(name)) { await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentPageNumber = 3; return true;`); await sleep(300); }
    // Re-focus the page before every key: a modal or the find bar may have taken focus.
    if (!['Escape'].includes(name)) { await c.clickAt(pageRect.x + pageRect.w / 2, pageRect.y + 40); await sleep(200); }
    const before = await read();
    await press(); await sleep(700);
    const after = await read();
    const row = { key: name, before: { page: before.page, scrollTop: before.scrollTop, scale: before.scale, scaleValue: before.scaleValue, rotation: before.rotation, focus: before.ui.focus }, after };
    if (name === 'Ctrl+a') row.selectionLength = await ev(`return window.getSelection().toString().length;`);
    rows.push(row);
    partial.rows = rows;
    // Close whatever the key opened.
    if (after.ui.modals.length || after.ui.menus.length) { await c.pressKey('Escape'); await sleep(300); }
    if (name === 'Ctrl+f' && after.findbarOpen) { await c.pressKey('Escape'); await sleep(300); }
    if (after.presentation) { await c.pressKey('Escape'); await sleep(500); }
  }
  return { scope, focusAfterClick: focus0, rows };
});

await step('s06-scroll-state', async (partial) => {
  await fresh();
  const pageRect = await c.rect('.workspace-leaf.mod-active .page[data-page-number="1"]');
  const read = () => ev(`${P} const s = P.pdfState(); return { state: s.state, viewState: s.viewState, viewer: s.viewer };`);
  const start = await read();
  await wheel(pageRect.x + pageRect.w / 2, pageRect.y + 100, 900); await sleep(900);
  const afterWheel = await read();
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.container.scrollTop = v.container.scrollTop + 900; return true;`); await sleep(900);
  const afterScrollTop = await read();
  // A page boundary: scroll so page 3's top is 10 px above the container's top, then 10 px below.
  const boundary = await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); const c3 = v.container.querySelector('.page[data-page-number="3"]'); const c2 = v.container.querySelector('.page[data-page-number="2"]');
    const top3 = c3.offsetTop; const res = {};
    for (const [name, y] of [['page3TopMinus10', top3 - 10], ['page3TopPlus10', top3 + 10], ['page3Middle', top3 + c3.offsetHeight / 2], ['page2Bottom-5', c2.offsetTop + c2.offsetHeight - 5]]) { v.container.scrollTop = y; await P.sleep(400); res[name] = { scrollTop: v.container.scrollTop, page: v.currentPageNumber, state: P.pdfLeaf().view.getState(), location: v._location ? { top: v._location.top, left: v._location.left, pageNumber: v._location.pageNumber } : null }; }
    res.offsets = { page1: v.container.querySelector('.page[data-page-number="1"]').offsetTop, page2: c2.offsetTop, page3: top3, page3Height: c3.offsetHeight, page2Height: c2.offsetHeight };
    return res;`);
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.container.scrollTop = 0; await P.sleep(300); v.container.scrollTop = 1800; return true;`); await sleep(600);
  const beforeSave = await read();
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  const ws = wsJson();
  const pdfLeaves = wsLeaves(ws, 'pdf');
  // Then the same leaf's record after zoom and rotation change.
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentScaleValue = '1.5'; v.pagesRotation = 90; return true;`); await sleep(800);
  const zoomed = await read();
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  const ws2 = wsJson();
  return { start, afterWheel, afterScrollTop, boundary, beforeSave, workspaceJsonPdfLeaves: pdfLeaves, workspaceTopLevelKeys: Object.keys(ws), active: ws.active, lastOpenFiles: ws.lastOpenFiles, zoomedRotated: zoomed, workspaceJsonPdfLeavesAfterZoomRotate: wsLeaves(ws2, 'pdf') };
});

await step('s07-links', async (partial) => {
  await fresh();
  const read = () => ev(`${P} const s = P.pdfState(); return { leaves: P.leaves(), state: s.state, viewer: s.viewer && { page: s.viewer.currentPageNumber, scrollTop: s.viewer.scrollTop, scale: s.viewer.currentScale, scaleValue: s.viewer.currentScaleValue, location: s.viewer.location }, active: app.workspace.activeLeaf.id, notices: P.ui().notices };`);
  const viaApi = [];
  for (const link of ['doc.pdf#page=4', 'doc.pdf#page=2', 'doc.pdf#page=99', 'doc.pdf#page=0', 'doc.pdf#page=abc', 'doc.pdf#foo', 'doc.pdf#height=300', 'doc.pdf#page=3&height=200', 'doc.pdf', 'doc.pdf#page=5', 'doc.pdf#page=2', 'doc.pdf#page=4']) {
    const before = await read();
    let err = null;
    try { await ev(`await app.workspace.openLinkText(${JSON.stringify(link)}, '', false); return true;`); } catch (e) { err = e.message; }
    await sleep(1300);
    const after = await read();
    viaApi.push({ link, error: err, before: { page: before.viewer && before.viewer.page, scrollTop: before.viewer && before.viewer.scrollTop, leaves: before.leaves.length, active: before.active }, after });
    partial.viaApi = viaApi;
  }
  // The same links with openLinkText(..., '', true) (new leaf).
  const newLeaf = {};
  {
    const before = await read();
    await ev(`await app.workspace.openLinkText('doc.pdf#page=3', '', true); return true;`); await sleep(1300);
    newLeaf.page3 = { before: { leaves: before.leaves.length }, after: await read() };
  }
  // Real clicks on the links in Embeds.md, reading view (the pdf leaf is detached after the note opens).
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(1500);
  await ev(`
    window.__clicks = [];
    if (!window.__linkGuard) { window.__linkGuard = (ev) => { const a = ev.target.closest ? ev.target.closest('a') : null; window.__clicks.push({ href: a ? a.getAttribute('href') : null, dataHref: a ? a.getAttribute('data-href') : null, cls: a ? a.className : null, defaultPrevented: ev.defaultPrevented, tag: ev.target.tagName }); ev.preventDefault(); }; document.addEventListener('click', window.__linkGuard, false); }
    return true;`);
  const links = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view');
    for (let y = 0; y < pv.scrollHeight + 600; y += 300) { pv.scrollTop = y; await P.sleep(120); }
    const links = Array.from(pv.querySelectorAll('a')).filter((a) => !a.closest('.internal-embed')).map((a, i) => { a.setAttribute('data-probe-link', String(i)); return { i, text: a.textContent, href: a.getAttribute('href'), dataHref: a.getAttribute('data-href'), cls: a.className, aria: a.getAttribute('aria-label'), target: a.getAttribute('target'), rel: a.getAttribute('rel') }; });
    return links;`);
  const clicks = [];
  for (const l of links) {
    const before = await ev(`${P} return { leaves: P.leaves(), pdf: P.pdfState() && { state: P.pdfState().state, page: P.pdfState().viewer && P.pdfState().viewer.currentPageNumber } };`);
    const r = await ev(`${P} const a = document.querySelector('a[data-probe-link="${l.i}"]'); a.scrollIntoView({ block: 'center' }); await P.sleep(300); return P.rect(a);`);
    await c.clickAt(r.x + Math.min(12, r.w / 2), r.y + r.h / 2); await sleep(1500);
    const after = await ev(`${P} const s = P.pdfState(); return { leaves: P.leaves(), activeType: app.workspace.activeLeaf.view.getViewType(), activeFile: app.workspace.activeLeaf.view.file && app.workspace.activeLeaf.view.file.path, pdf: s && { state: s.state, viewState: s.viewState, page: s.viewer && s.viewer.currentPageNumber, scrollTop: s.viewer && s.viewer.scrollTop, scaleValue: s.viewer && s.viewer.currentScaleValue }, guard: window.__clicks.splice(0), ui: P.ui(), url: location.href.slice(0, 80) };`);
    clicks.push({ link: l, before: { leaves: before.leaves.length, pdfPage: before.pdf && before.pdf.page }, after });
    partial.clicks = clicks;
    // Back to the note for the next link; keep the pdf leaf (to see reuse) but activate the markdown leaf.
    await ev(`${P} const ml = app.workspace.getLeavesOfType('markdown')[0]; app.workspace.setActiveLeaf(ml, { focus: true }); return true;`); await sleep(300);
  }
  // Ctrl+click on the first link: a new tab?
  let ctrlClick = null;
  if (links.length) {
    const before = await ev(`${P} return P.leaves().length;`);
    const r = await ev(`${P} const a = document.querySelector('a[data-probe-link="0"]'); a.scrollIntoView({ block: 'center' }); await P.sleep(300); return P.rect(a);`);
    await c.clickAt(r.x + Math.min(12, r.w / 2), r.y + r.h / 2, { mods: { ctrl: true } }); await sleep(1500);
    ctrlClick = { leavesBefore: before, after: await ev(`${P} return { leaves: P.leaves(), guard: window.__clicks.splice(0) };`) };
  }
  await ev(`document.removeEventListener('click', window.__linkGuard, false); window.__linkGuard = null; return true;`);
  return { viaApi, newLeaf, links, clicks, ctrlClick };
});

await step('s08-embeds', async (partial) => {
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(2000);
  const readAll = async (mode) => {
    // Scroll the whole note so every section builds, then read each embed scrolled into view.
    const n = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.markdown-preview-view, .cm-scroller');
      for (let y = 0; y < sc.scrollHeight + 600; y += 250) { sc.scrollTop = y; await P.sleep(150); }
      sc.scrollTop = 0; await P.sleep(500);
      const es = P.embedsOf(leaf); es.forEach((e, i) => e.el.setAttribute('data-probe-embed', String(i))); return es.length;`);
    const items = [];
    for (let i = 0; i < n; i++) {
      const info = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const el = leaf.view.containerEl.querySelector('[data-probe-embed="${i}"]'); if (!el) return { missing: ${i} };
        el.scrollIntoView({ block: 'center' }); await P.sleep(900);
        const es = P.embedsOf(leaf); const e = es.find((x) => x.el === el) || { el };
        const label = (() => { let p = el.closest('.cm-embed-block') || el; let prev = p.previousElementSibling; let hops = 0; while (prev && hops++ < 6) { const t = prev.textContent.trim(); if (/^Embed:/.test(t)) return t; prev = prev.previousElementSibling; } return null; })();
        const info = P.embedInfo(e, 7); info.label = label; info.i = ${i};
        info.innerStyles = P.styles(['[class*=toolbar]', '[class*=toolbar] *', '.page', '[class*=viewer-container], [class*=viewerContainer]', '.pdfViewer'], el);
        info.sourceLine = null;
        return info;`);
      items.push(info);
      partial[mode] = items;
    }
    return items;
  };
  const preview = await readAll('preview');
  // Screenshot the first two embeds.
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const el = leaf.view.containerEl.querySelector('[data-probe-embed="0"]'); if (el) el.scrollIntoView({ block: 'start' }); await P.sleep(600); return true;`);
  const shotPreview = await shot('08-embeds-reading');
  // Live Preview, caret away from the embeds (line 0).
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; await leaf.setViewState({ type: 'markdown', state: { file: 'Embeds.md', mode: 'source', source: false } }); await P.sleep(800); leaf.view.editor.setCursor({ line: 0, ch: 0 }); return true;`); await sleep(1500);
  const livePreview = await readAll('livePreview');
  const lpShot = await shot('08-embeds-live-preview');
  // Source mode: the embeds are text.
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; await leaf.setViewState({ type: 'markdown', state: { file: 'Embeds.md', mode: 'source', source: true } }); await P.sleep(800); leaf.view.editor.setCursor({ line: 0, ch: 0 }); return true;`); await sleep(1200);
  const source = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.cm-scroller'); for (let y = 0; y < sc.scrollHeight + 600; y += 250) { sc.scrollTop = y; await P.sleep(100); } sc.scrollTop = 0; await P.sleep(300);
    return { embedBlocks: leaf.view.containerEl.querySelectorAll('.cm-embed-block, .internal-embed').length, pages: leaf.view.containerEl.querySelectorAll('.page').length, lines: Array.from(leaf.view.containerEl.querySelectorAll('.cm-line')).filter((l) => /pdf/.test(l.textContent)).map((l) => ({ text: l.textContent, cls: Array.from(l.querySelectorAll('span')).map((s) => s.className).slice(0, 8) })).slice(0, 20), viewState: leaf.getViewState() };`);
  // Tall.md: is the embed below the fold built before scrolling?
  await ev(`${P} await P.openMd('Tall.md', 'preview'); return true;`); await sleep(2500);
  const tall = {};
  tall.beforeScroll = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const sizer = pv.querySelector('.markdown-preview-sizer');
    const emb = pv.querySelector('.internal-embed'); return { scrollTop: pv.scrollTop, scrollHeight: pv.scrollHeight, clientHeight: pv.clientHeight, sections: pv.querySelectorAll('.markdown-preview-section > div').length, embedExists: !!emb, embedRect: emb && P.rect(emb), embedPages: emb ? emb.querySelectorAll('.page').length : 0, embedCanvases: emb ? emb.querySelectorAll('canvas').length : 0, embedCls: emb && P.cls(emb), embedChildren: emb ? emb.children.length : 0, lastParagraph: (pv.querySelector('.markdown-preview-section > div:last-child') || {}).textContent, sizerMinHeight: sizer && sizer.style.minHeight };`);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); pv.scrollTop = pv.scrollHeight / 2 - 200; return true;`); await sleep(1500);
  tall.afterScrollToMiddle = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const emb = pv.querySelector('.internal-embed'); return { scrollTop: pv.scrollTop, scrollHeight: pv.scrollHeight, embedExists: !!emb, embedRect: emb && P.rect(emb), embedPages: emb ? emb.querySelectorAll('.page').length : 0, embedCanvases: emb ? emb.querySelectorAll('canvas').length : 0, pagesLoaded: emb ? Array.from(emb.querySelectorAll('.page')).map((p) => p.getAttribute('data-loaded')) : null, viewers: P.findViewers(leaf.view, 9, 3000).length };`);
  // Tall.md in Live Preview: the same question.
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; await leaf.setViewState({ type: 'markdown', state: { file: 'Tall.md', mode: 'source', source: false } }); await P.sleep(600); leaf.view.editor.setCursor({ line: 0, ch: 0 }); return true;`); await sleep(2000);
  tall.livePreviewBeforeScroll = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.cm-scroller'); const emb = sc.querySelector('.internal-embed, .cm-embed-block'); return { scrollTop: sc.scrollTop, scrollHeight: sc.scrollHeight, lines: sc.querySelectorAll('.cm-line').length, embedExists: !!emb, embedPages: emb ? emb.querySelectorAll('.page').length : 0 };`);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.editor.scrollIntoView({ from: { line: 82, ch: 0 }, to: { line: 82, ch: 0 } }, true); return true;`); await sleep(1500);
  tall.livePreviewAfterScroll = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.cm-scroller'); const emb = sc.querySelector('.internal-embed, .cm-embed-block'); return { scrollTop: sc.scrollTop, scrollHeight: sc.scrollHeight, embedExists: !!emb, embedRect: emb && P.rect(emb), embedPages: emb ? emb.querySelectorAll('.page').length : 0, embedCanvases: emb ? emb.querySelectorAll('canvas').length : 0, embedCls: emb && P.cls(emb) };`);
  return { preview, shotPreview, livePreview, lpShot, source, tall };
});

await step('s09-embed-interaction', async (partial) => {
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(2000);
  const locate = () => ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view');
    const es = P.embedsOf(leaf); const e = es.find((x) => x.viewer) || es[0]; if (!e) return null; e.el.scrollIntoView({ block: 'center' }); await P.sleep(700);
    window.__emb = e.el;
    const inner = e.el.querySelector('[class*=viewer-container], [class*=viewerContainer], [class*=container]');
    return { rect: P.rect(e.el), noteScrollTop: pv.scrollTop, innerScrollTop: e.viewer ? e.viewer.container.scrollTop : (inner ? inner.scrollTop : null), innerScrollHeight: e.viewer ? e.viewer.container.scrollHeight : null, innerClientHeight: e.viewer ? e.viewer.container.clientHeight : null, page: e.viewer ? e.viewer.currentPageNumber : null, scaleValue: e.viewer ? e.viewer.currentScaleValue : null, viewerPath: e.viewerPath, inputs: Array.from(e.el.querySelectorAll('input')).map((i) => ({ cls: P.cls(i), value: i.value, type: i.type, rect: P.rect(i) })), pages: e.el.querySelectorAll('.page').length };`);
  const before = await locate();
  if (!before) throw new Error('no embed');
  partial.before = before;
  await wheel(before.rect.x + before.rect.w / 2, before.rect.y + before.rect.h / 2, 300); await sleep(800);
  const afterWheel = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const es = P.embedsOf(leaf); const e = es.find((x) => x.el === window.__emb); return { noteScrollTop: pv.scrollTop, innerScrollTop: e && e.viewer ? e.viewer.container.scrollTop : null, page: e && e.viewer ? e.viewer.currentPageNumber : null, rect: P.rect(window.__emb) };`);
  // A second wheel, and one from the bottom of the embed's inner scroll (does the note take over when the embed is at its end?).
  await wheel(afterWheel.rect.x + afterWheel.rect.w / 2, afterWheel.rect.y + afterWheel.rect.h / 2, 300); await sleep(800);
  const afterWheel2 = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const es = P.embedsOf(leaf); const e = es.find((x) => x.el === window.__emb); return { noteScrollTop: pv.scrollTop, innerScrollTop: e && e.viewer ? e.viewer.container.scrollTop : null, page: e && e.viewer ? e.viewer.currentPageNumber : null, rect: P.rect(window.__emb) };`);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const es = P.embedsOf(leaf); const e = es.find((x) => x.el === window.__emb); if (e && e.viewer) e.viewer.container.scrollTop = e.viewer.container.scrollHeight; return true;`); await sleep(500);
  const atEnd = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const es = P.embedsOf(leaf); const e = es.find((x) => x.el === window.__emb); return { noteScrollTop: pv.scrollTop, innerScrollTop: e && e.viewer ? e.viewer.container.scrollTop : null, page: e && e.viewer ? e.viewer.currentPageNumber : null, rect: P.rect(window.__emb) };`);
  await wheel(atEnd.rect.x + atEnd.rect.w / 2, atEnd.rect.y + atEnd.rect.h / 2, 300); await sleep(800);
  const afterWheelAtEnd = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const es = P.embedsOf(leaf); const e = es.find((x) => x.el === window.__emb); return { noteScrollTop: pv.scrollTop, innerScrollTop: e && e.viewer ? e.viewer.container.scrollTop : null, page: e && e.viewer ? e.viewer.currentPageNumber : null };`);
  // The page input inside the embed, if there is one.
  let pageInput = null;
  const inp = before.inputs.find((i) => i.type !== 'checkbox' && /^[0-9]+$/.test(i.value || '')) || before.inputs[0];
  if (inp) {
    const r = await ev(`${P} window.__emb.scrollIntoView({ block: 'center' }); await P.sleep(500); const i = window.__emb.querySelector('input'); return P.rect(i);`);
    await c.clickAt(r.x + r.w / 2, r.y + r.h / 2); await sleep(200);
    await c.pressKey('a', { ctrl: true }); await c.typeText('3'); await c.pressKey('Enter'); await sleep(1000);
    pageInput = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const es = P.embedsOf(leaf); const e = es.find((x) => x.el === window.__emb); return { input: window.__emb.querySelector('input') ? window.__emb.querySelector('input').value : null, page: e && e.viewer ? e.viewer.currentPageNumber : null, innerScrollTop: e && e.viewer ? e.viewer.container.scrollTop : null, focus: document.activeElement && (document.activeElement.tagName + '.' + P.cls(document.activeElement)) };`);
  } else {
    // No input: set the page through the viewer and see whether the DOM follows.
    pageInput = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const es = P.embedsOf(leaf); const e = es.find((x) => x.el === window.__emb); if (!e || !e.viewer) return { noViewer: true }; e.viewer.currentPageNumber = 3; await P.sleep(800); return { set: 3, page: e.viewer.currentPageNumber, innerScrollTop: e.viewer.container.scrollTop, toolbarText: window.__emb.querySelector('[class*=toolbar]') ? window.__emb.querySelector('[class*=toolbar]').textContent.trim() : null };`);
  }
  const noteState = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; return { viewState: leaf.getViewState(), state: leaf.view.getState() };`);
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  const ws = wsJson();
  return { before, afterWheel, afterWheel2, atEnd, afterWheelAtEnd, pageInput, noteState, workspaceMarkdownLeaves: wsLeaves(ws, 'markdown'), workspaceMentionsPdf: JSON.stringify(ws).includes('pdf') };
});

await step('s10-canvas', async (partial) => {
  await ev(`${P} const f = await P.file('Board.canvas'); const leaf = app.workspace.getLeaf('tab'); await leaf.openFile(f); app.workspace.setActiveLeaf(leaf, { focus: true }); for (const t of ['pdf', 'markdown']) app.workspace.detachLeavesOfType(t); for (const l of app.workspace.getLeavesOfType('canvas')) if (l !== leaf) l.detach(); return true;`); await sleep(2500);
  const nodes = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; cv.zoomToFit(); await P.sleep(1200);
    const res = {}; for (const [id, n] of cv.nodes) {
      const el = n.nodeEl; const viewers = P.findViewers(n, 7, 2500);
      res[id] = { type: n.getData ? n.getData().type : null, data: n.getData ? n.getData() : null, rect: P.rect(el), cls: P.cls(el), dump: P.dump(el, 9, 16), hasToolbar: !!el.querySelector('[class*=toolbar]'), toolbarText: el.querySelector('[class*=toolbar]') ? el.querySelector('[class*=toolbar]').textContent.trim() : null, pages: Array.from(el.querySelectorAll('.page')).map((p) => ({ n: p.getAttribute('data-page-number'), loaded: p.getAttribute('data-loaded'), rect: P.rect(p), canvas: !!p.querySelector('canvas') })).slice(0, 6), viewers: viewers.map((v) => ({ path: v.path, state: P.vstate(v.viewer) })), nodeKeys: Object.keys(n).slice(0, 60), childKeys: n.child ? Object.keys(n.child).slice(0, 60) : null, label: el.querySelector('.canvas-node-label') && el.querySelector('.canvas-node-label').textContent, styles: P.styles(['.canvas-node-content', '.canvas-node-container', '[class*=embed]', '[class*=pdf]'], el) };
    }
    return { zoom: cv.zoom, x: cv.x, y: cv.y, nodes: res, viewState: leaf.getViewState() };`);
  partial.nodes = nodes;
  const shotFile = await shot('10-canvas-nodes');
  // Zoom in on f1 and screenshot it alone.
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, 0); return true;`); await sleep(1200);
  const f1 = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); const el = n.nodeEl; const viewers = P.findViewers(n, 7, 2500); return { rect: P.rect(el), pages: Array.from(el.querySelectorAll('.page')).map((p) => ({ n: p.getAttribute('data-page-number'), rect: P.rect(p) })).slice(0, 6), viewers: viewers.map((v) => ({ path: v.path, state: P.vstate(v.viewer) })), contentCls: P.cls(el.querySelector('.canvas-node-content')), dump: P.dump(el, 10, 16), styles: P.styles(['.canvas-node-content', '.canvas-node-content > *', '[class*=toolbar]', '[class*=toolbar] *'], el) };`);
  const shotF1 = await shot('10-canvas-node-f1');
  // Wheel inside the node: the canvas pans/zooms or the pdf scrolls?
  const wheelBefore = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); const v = P.findViewers(n, 7, 2000)[0]; return { x: cv.x, y: cv.y, zoom: cv.zoom, inner: v ? v.viewer.container.scrollTop : null, page: v ? v.viewer.currentPageNumber : null };`);
  await wheel(f1.rect.x + f1.rect.w / 2, f1.rect.y + f1.rect.h / 2, 300); await sleep(800);
  const wheelAfter = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); const v = P.findViewers(n, 7, 2000)[0]; return { x: cv.x, y: cv.y, zoom: cv.zoom, inner: v ? v.viewer.container.scrollTop : null, page: v ? v.viewer.currentPageNumber : null };`);
  // Single click then double click.
  await c.clickAt(f1.rect.x + f1.rect.w / 2, f1.rect.y + f1.rect.h / 2); await sleep(600);
  const afterClick = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; return { selection: Array.from(cv.selection).map((o) => o.id), nodeCls: P.cls(cv.nodes.get('f1').nodeEl), focus: P.ui().focus, leaves: P.leaves().length };`);
  await c.clickAt(f1.rect.x + f1.rect.w / 2, f1.rect.y + f1.rect.h / 2, { clickCount: 2 }); await sleep(1500);
  const afterDbl = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; return { selection: Array.from(cv.selection).map((o) => o.id), nodeCls: P.cls(cv.nodes.get('f1').nodeEl), leaves: P.leaves(), activeType: app.workspace.activeLeaf.view.getViewType(), activeFile: app.workspace.activeLeaf.view.file && app.workspace.activeLeaf.view.file.path, pdf: P.pdfState() && { state: P.pdfState().state, viewState: P.pdfState().viewState }, ui: P.ui() };`);
  // Double-click f2 (the #page=2 node): which page does the opened leaf show?
  await ev(`${P} app.workspace.detachLeavesOfType('pdf'); const leaf = app.workspace.getLeavesOfType('canvas')[0]; app.workspace.setActiveLeaf(leaf, { focus: true }); const cv = leaf.view.canvas; const n = cv.nodes.get('f2'); cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, 0); return true;`); await sleep(1200);
  const f2rect = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; return P.rect(leaf.view.canvas.nodes.get('f2').nodeEl);`);
  await c.clickAt(f2rect.x + f2rect.w / 2, f2rect.y + f2rect.h / 2, { clickCount: 2 }); await sleep(1500);
  const afterDblF2 = await ev(`${P} return { leaves: P.leaves(), activeType: app.workspace.activeLeaf.view.getViewType(), pdf: P.pdfState() && { state: P.pdfState().state, page: P.pdfState().viewer && P.pdfState().viewer.currentPageNumber } };`);
  return { nodes, shot: shotFile, f1, shotF1, wheel: { before: wheelBefore, after: wheelAfter }, afterClick, afterDbl, afterDblF2 };
});

await step('s11-menus', async (partial) => {
  await fresh();
  await ev(`${P} app.workspace.detachLeavesOfType('canvas'); app.workspace.detachLeavesOfType('markdown'); return true;`);
  const pageRect = await c.rect('.workspace-leaf.mod-active .page[data-page-number="1"]');
  await ev(`window.getSelection().removeAllRanges(); return true;`);
  await c.clickAt(pageRect.x + pageRect.w / 2, pageRect.y + pageRect.h - 60, { button: 'right' }); await sleep(700);
  const page = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`), selection: await ev(`return window.getSelection().toString();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  const tabRect = await c.rect('.mod-root .workspace-tab-header.is-active');
  await c.clickAt(tabRect.cx, tabRect.cy, { button: 'right' }); await sleep(700);
  const tab = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  const more = await c.clickElement('.workspace-leaf.mod-active .view-actions .clickable-icon[aria-label="More options"], .workspace-leaf.mod-active .view-actions .clickable-icon:last-child'); await sleep(700);
  const moreMenu = { clicked: more, items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  // Right-click on the toolbar and on the gap between pages, for completeness.
  const toolbarRect = await c.rect('.workspace-leaf.mod-active .view-content [class*=toolbar]');
  let toolbar = null;
  if (toolbarRect) { await c.clickAt(toolbarRect.x + toolbarRect.w - 10, toolbarRect.cy, { button: 'right' }); await sleep(600); toolbar = { items: await c.openMenuItems() }; await ev(`${P} P.dismiss(); return true;`); }
  const gapY = await ev(`${P} const c = P.viewContent(); const p1 = c.querySelector('.page[data-page-number="1"]'); const p2 = c.querySelector('.page[data-page-number="2"]'); if (!p1 || !p2) return null; const a = p1.getBoundingClientRect(), b = p2.getBoundingClientRect(); return (a.bottom + b.top) / 2;`);
  let gap = null;
  if (gapY && gapY < out.version.win[1]) { await c.clickAt(pageRect.x + pageRect.w / 2, gapY, { button: 'right' }); await sleep(600); gap = { y: gapY, items: await c.openMenuItems() }; await ev(`${P} P.dismiss(); return true;`); }
  return { page, tab, moreMenu, toolbar, gap };
});

await step('s12-selection', async (partial) => {
  await fresh();
  const span = await ev(`${P} const c = P.viewContent(); const p1 = c.querySelector('.page[data-page-number="1"]'); const spans = Array.from(p1.querySelectorAll('.textLayer span, .textLayer div')).filter((s) => /This is page one/.test(s.textContent));
    const s = spans[spans.length - 1]; if (!s) return { spans: p1.querySelectorAll('.textLayer *').length, texts: Array.from(p1.querySelectorAll('.textLayer span')).map((x) => x.textContent).slice(0, 20) };
    return { text: s.textContent, rect: P.rect(s), tag: s.tagName, cls: P.cls(s), textLayerCls: P.cls(p1.querySelector('.textLayer')), spanCount: p1.querySelectorAll('.textLayer span').length };`);
  partial.span = span;
  if (!span.rect) throw new Error('big sentence not found in the text layer: ' + JSON.stringify(span));
  const select = async () => { await ev(`window.getSelection().removeAllRanges(); return true;`); await c.drag(span.rect.x + 2, span.rect.y + span.rect.h / 2, span.rect.x + span.rect.w - 2, span.rect.y + span.rect.h / 2, { steps: 10, pauseMs: 30 }); await sleep(400); return ev(`return window.getSelection().toString();`); };
  const selected = await select();
  const selectionStyle = await ev(`${P} const c = P.viewContent(); const sel = window.getSelection(); const node = sel.anchorNode && sel.anchorNode.parentElement; return { anchorCls: node && P.cls(node), rangeCount: sel.rangeCount, selectionBg: getComputedStyle(c.querySelector('.textLayer'), '::selection').backgroundColor, selectionColor: getComputedStyle(c.querySelector('.textLayer'), '::selection').color, textLayerStyle: P.style(c.querySelector('.textLayer')), spanStyle: P.style(node) };`);
  const shotSel = await shot('12-selection');
  await c.clickAt(span.rect.x + span.rect.w / 2, span.rect.y + span.rect.h / 2, { button: 'right' }); await sleep(700);
  const menu = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`), selectionAfterRightClick: await ev(`return window.getSelection().toString();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  const copies = [];
  const rows = (menu.items[0] || []).filter((t) => /copy/i.test(t));
  for (const title of rows) {
    const sel = await select();
    await c.writeClipboard('PROBE-SENTINEL-' + Date.now());
    await c.clickAt(span.rect.x + span.rect.w / 2, span.rect.y + span.rect.h / 2, { button: 'right' }); await sleep(600);
    const r = await ev(`${P} const items = Array.from(document.querySelectorAll('.menu .menu-item')); const it = items.find((e) => (e.querySelector('.menu-item-title') || e).textContent.trim() === ${JSON.stringify(title)}); if (!it) return null; return P.rect(it);`);
    if (!r) { copies.push({ title, error: 'row not found' }); await ev(`${P} P.dismiss(); return true;`); continue; }
    await c.clickAt(r.x + r.w / 2, r.y + r.h / 2); await sleep(900);
    const clip = await c.readClipboard();
    const ui = await ev(`${P} return P.ui();`);
    copies.push({ title, selectionBefore: sel, clipboard: /^PROBE-SENTINEL-/.test(clip) ? '(no clipboard write)' : clip, clipboardJson: JSON.stringify(clip), ui });
    if (ui.modals.length) await c.pressKey('Escape');
    await ev(`${P} P.dismiss(); return true;`); await sleep(200);
    partial.copies = copies;
  }
  // Ctrl+C on the selection.
  await select(); await c.writeClipboard('PROBE-SENTINEL-' + Date.now());
  await c.pressKey('c', { ctrl: true }); await sleep(600);
  const ctrlC = await c.readClipboard();
  return { span, selected, selectionStyle, shot: shotSel, menu, copies, ctrlC: /^PROBE-SENTINEL-/.test(ctrlC) ? '(no clipboard write)' : ctrlC };
});

await step('s13-panes', async (partial) => {
  await fresh();
  const panes = {};
  for (const type of ['outline', 'backlink', 'outgoing-link']) {
    const r = await ev(`${P} const leaf = app.workspace.getRightLeaf(false); await leaf.setViewState({ type: ${JSON.stringify(type)}, state: {} }); app.workspace.rightSplit.expand(); app.workspace.setActiveLeaf(P.pdfLeaf(), { focus: true }); await P.sleep(1500);
      const el = leaf.view.containerEl; const content = el.querySelector('.view-content');
      return { viewType: leaf.view.getViewType(), displayText: leaf.getDisplayText(), state: leaf.view.getState(), headerTitle: el.querySelector('.view-header-title') && el.querySelector('.view-header-title').textContent, text: content.textContent.trim().slice(0, 600), rows: Array.from(content.querySelectorAll('.tree-item-self, .tree-item-inner, .search-result-file-title, .outline-item')).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 100) })).slice(0, 40), empty: content.querySelector('.pane-empty, .empty-state') && content.querySelector('.pane-empty, .empty-state').textContent.trim(), dump: P.dump(content, 7, 20), file: leaf.view.file && leaf.view.file.path };`);
    panes[type] = r;
    partial[type] = r;
  }
  const shotFile = await shot('13-panes-backlinks');
  await ev(`app.workspace.rightSplit.collapse(); return true;`);
  return { panes, shot: shotFile };
});

await step('s14-small-broken', async (partial) => {
  const res = {};
  for (const f of ['small.pdf', 'broken.pdf']) {
    await fresh(f, 3000);
    res[f] = await ev(`${P} const leaf = P.pdfLeaf(); const s = P.leafState(leaf); const content = P.viewContent(leaf); const header = leaf.view.containerEl.querySelector('.view-header');
      return { ...s, tabTitle: document.querySelector('.mod-root .workspace-tab-header.is-active') && document.querySelector('.mod-root .workspace-tab-header.is-active').getAttribute('aria-label'), headerTitle: header.querySelector('.view-header-title') && header.querySelector('.view-header-title').textContent, contentText: content.textContent.trim().slice(0, 400), contentDump: P.dump(content, 6, 20), ui: P.ui(), pages: content.querySelectorAll('.page').length, errorEls: Array.from(content.querySelectorAll('[class*=error], [class*=empty], [class*=message]')).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 200), style: P.style(e) })), toolbarVisible: !!(content.querySelector('[class*=toolbar]') && content.querySelector('[class*=toolbar]').offsetParent) };`);
    res[f].shot = await shot('14-' + f.replace('.', '-'));
    partial[f] = res[f];
  }
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  res.workspaceJsonPdfLeaves = wsLeaves(wsJson(), 'pdf');
  return res;
});

await step('s15-hover', async (partial) => {
  const plugin = await ev(`${P} const p = app.internalPlugins.plugins['page-preview']; if (!p) return { exists: false }; if (!p.enabled) await p.enable(); return { exists: true, enabled: p.enabled, options: p.instance && p.instance.options, overrides: p.instance && p.instance.overrides };`);
  partial.plugin = plugin;
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(1800);
  const link = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); for (let y = 0; y < pv.scrollHeight + 600; y += 300) { pv.scrollTop = y; await P.sleep(100); }
    const a = Array.from(pv.querySelectorAll('a.internal-link')).find((x) => !x.closest('.internal-embed') && x.getAttribute('data-href') === 'doc.pdf'); if (!a) return null; a.scrollIntoView({ block: 'center' }); await P.sleep(400); return { rect: P.rect(a), text: a.textContent, href: a.getAttribute('href'), dataHref: a.getAttribute('data-href') };`);
  if (!link) throw new Error('no [[doc.pdf]] link in the preview');
  const popoverRead = () => ev(`${P} const ps = Array.from(document.querySelectorAll('.popover, .hover-popover')); return ps.map((p) => ({ cls: P.cls(p), rect: P.rect(p), style: P.style(p), inner: Array.from(p.children).map((ch) => ({ cls: P.cls(ch), rect: P.rect(ch) })), contentCls: Array.from(p.querySelectorAll('*')).slice(0, 12).map((e) => P.cls(e)).filter(Boolean), pages: p.querySelectorAll('.page').length, hasToolbar: !!p.querySelector('[class*=toolbar]'), text: p.textContent.trim().slice(0, 200), dump: P.dump(p, 7, 12) }));`);
  // Plain hover first.
  await c.mouseMove(link.rect.x + 8, link.rect.y + link.rect.h / 2); await sleep(1500);
  const plain = await popoverRead();
  // Then with Ctrl held: a key down, then moves with the modifier.
  await c.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: 'Control', code: 'ControlLeft', windowsVirtualKeyCode: 17, nativeVirtualKeyCode: 17, modifiers: 2 });
  for (let i = 0; i < 4; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: Math.round(link.rect.x + 8 + i), y: Math.round(link.rect.y + link.rect.h / 2), modifiers: 2 }); await sleep(120); }
  await sleep(1600);
  const ctrl = await popoverRead();
  const shotFile = await shot('15-hover-popover');
  await sleep(1200);
  const ctrlLater = await popoverRead();
  await c.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Control', code: 'ControlLeft', windowsVirtualKeyCode: 17, nativeVirtualKeyCode: 17 });
  await c.mouseMove(5, out.version.win[1] - 5); await sleep(800);
  const after = await popoverRead();
  return { plugin, link, plain, ctrl, ctrlLater, after, shot: shotFile };
});

await step('s16-settings', async (partial) => {
  return ev(`${P}
    const tabs = (xs) => (xs || []).map((t) => ({ id: t.id, name: t.name }));
    const settingTabs = tabs(app.setting.settingTabs), pluginTabs = tabs(app.setting.pluginTabs);
    // Render each core tab offscreen and grep its labels for PDF.
    const hits = []; const errors = [];
    for (const t of app.setting.settingTabs) {
      try { if (!t.containerEl) continue; t.display(); const items = Array.from(t.containerEl.querySelectorAll('.setting-item')).map((e) => ({ name: (e.querySelector('.setting-item-name') || {}).textContent, desc: (e.querySelector('.setting-item-description') || {}).textContent, heading: e.classList.contains('setting-item-heading') })); for (const it of items) if (/pdf/i.test((it.name || '') + ' ' + (it.desc || ''))) hits.push({ tab: t.id, ...it }); t.hide && t.hide(); } catch (e) { errors.push({ tab: t.id, error: e.message }); }
    }
    const cfg = app.vault.config || {};
    const cfgKeys = Object.keys(cfg).filter((k) => /pdf/i.test(k));
    const internal = Object.keys(app.internalPlugins.plugins).map((k) => ({ id: k, enabled: app.internalPlugins.plugins[k].enabled })).filter((x) => /pdf/i.test(x.id));
    const internalAll = Object.keys(app.internalPlugins.plugins);
    const community = Object.keys(app.plugins.plugins);
    const reg = app.viewRegistry;
    const pdfView = reg.viewByType && reg.viewByType['pdf'];
    const embedReg = app.embedRegistry;
    return { settingTabs, pluginTabs, pdfSettingHits: hits, errors, cfgKeysWithPdf: cfgKeys, cfgValues: Object.fromEntries(cfgKeys.map((k) => [k, cfg[k]])), internalPluginsWithPdf: internal, internalPluginIds: internalAll, communityPlugins: community,
      viewRegistry: { typeByExtensionPdf: reg.typeByExtension && reg.typeByExtension['pdf'], viewByTypePdf: pdfView ? (pdfView.name || String(pdfView).slice(0, 80)) : null, allExtensions: reg.typeByExtension },
      embedRegistry: embedReg ? { pdf: !!(embedReg.embedByExtension && embedReg.embedByExtension['pdf']), extensions: embedReg.embedByExtension ? Object.keys(embedReg.embedByExtension) : null } : null,
      pdfCommands: Object.keys(app.commands.commands).filter((id) => /pdf/i.test(id)),
      localStorageKeysWithPdf: Object.keys(localStorage).filter((k) => /pdf/i.test(k)).map((k) => ({ k, v: String(localStorage.getItem(k)).slice(0, 200) })),
      idbNames: (await indexedDB.databases()).map((d) => d.name).filter((n) => /pdf/i.test(n || '')) };`);
});


// ---- the second pass: pdf.js's view history stripped first ------------------
// pdf.js keeps a per-file record (page, zoom, scroll, rotation, sidebar) in
// localStorage['pdfjs.history'] keyed by the document's fingerprint, shared by
// every vault this Obsidian opens. Only the probe's own files are touched.
const owner = `(P.pdfLeaf() && P.pdfLeaf().view.viewer && P.pdfLeaf().view.viewer.child && P.pdfLeaf().view.viewer.child.pdfViewer)`;
const historyEntry = () => ev(`${P} const o = ${owner}; const fp = o && o.pdfDocument && o.pdfDocument.fingerprints && o.pdfDocument.fingerprints[0];
  let h = null; try { h = JSON.parse(localStorage.getItem('pdfjs.history') || '{"files":[]}'); } catch (e) { return { error: e.message }; }
  return { fingerprint: fp, entry: fp ? (h.files || []).find((f) => f.fingerprint === fp) || null : null, sidebarView: localStorage.getItem('pdfjs.sidebarView'), files: (h.files || []).length };`);
const stripHistory = () => ev(`${P} const fps = window.__fps || []; let h; try { h = JSON.parse(localStorage.getItem('pdfjs.history') || '{"files":[]}'); } catch (e) { return { error: e.message }; }
  const before = (h.files || []).length; h.files = (h.files || []).filter((f) => !fps.includes(f.fingerprint)); localStorage.setItem('pdfjs.history', JSON.stringify(h)); return { fingerprints: fps, before, after: h.files.length };`);
const collectFingerprints = async () => {
  const fps = [];
  for (const f of ['doc.pdf', 'small.pdf', 'broken.pdf']) {
    await fresh(f, 2000);
    const fp = await ev(`${P} const o = ${owner}; return o && o.pdfDocument && o.pdfDocument.fingerprints ? o.pdfDocument.fingerprints[0] : null;`);
    if (fp) fps.push(fp);
  }
  await ev(`window.__fps = ${JSON.stringify(fps)}; return true;`);
  return fps;
};
// Enter through a keyDown that carries text, so the input's `change` fires (rawKeyDown makes no keypress).
const pressEnterReal = async () => { const p = { key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13, text: '\r', unmodifiedText: '\r' }; await c.send('Input.dispatchKeyEvent', { type: 'keyDown', ...p }); await c.send('Input.dispatchKeyEvent', { type: 'keyUp', ...p }); };
const toolbarCtl = (aria) => ev(`${P} const t = P.viewContent().querySelector('.pdf-toolbar'); const e = t && t.querySelector('[aria-label=${JSON.stringify(aria)}]'); return e ? P.rect(e) : null;`);
const clickCtl = async (aria, wait = 900) => { const r = await toolbarCtl(aria); if (!r) throw new Error('no toolbar control ' + aria); await c.clickAt(r.x + r.w / 2, r.y + r.h / 2); await sleep(wait); return r; };
const clickMenuRow = async (title, wait = 1000) => { const r = await ev(`${P} const items = Array.from(document.querySelectorAll('.menu .menu-item')); const it = items.find((e) => (e.querySelector('.menu-item-title') || e).textContent.trim() === ${JSON.stringify(title)}); return it ? P.rect(it) : null;`); if (!r) throw new Error('no menu row ' + title); await c.clickAt(r.x + r.w / 2, r.y + r.h / 2); await sleep(wait); return r; };
const vs = () => ev(`${P} const s = P.pdfState(); return { state: s.state, viewer: s.viewer, ui: P.ui(), sidebarOpen: !!P.viewContent().querySelector('.pdf-content-container.sidebarOpen'), contentCls: P.cls(P.viewContent().querySelector('.pdf-content-container')), pageInvert: ${owner} ? ${owner}.pageInvert : null, pageBackground: ${owner} ? ${owner}.pageBackground : null, zoomFactor: window.require('electron').webFrame.getZoomFactor() };`);
const brief = (v) => v && v.viewer ? { page: v.viewer.currentPageNumber, scale: v.viewer.currentScale, scaleValue: v.viewer.currentScaleValue, rotation: v.viewer.pagesRotation, spread: v.viewer.spreadMode, scrollMode: v.viewer.scrollMode, scrollTop: v.viewer.scrollTop, clientWidth: v.viewer.clientWidth, page1: v.viewer.firstPage && v.viewer.firstPage.rect, sidebarOpen: v.sidebarOpen, pageInvert: v.pageInvert, pageBackground: v.pageBackground, zoomFactor: v.zoomFactor, focus: v.ui.focus, menus: v.ui.menus, modals: v.ui.modals } : v;

await step('s02b-hygiene', async () => {
  const fps = await collectFingerprints();
  const before = await ev(`${P} let h = null; try { h = JSON.parse(localStorage.getItem('pdfjs.history') || '{"files":[]}'); } catch (e) {} const fps = window.__fps || []; return { ours: (h && h.files || []).filter((f) => fps.includes(f.fingerprint)), sidebarView: localStorage.getItem('pdfjs.sidebarView') };`);
  const stripped = await stripHistory();
  await fresh();
  const after = await vs();
  return { fingerprints: fps, ourEntriesBefore: before.ours, sidebarViewBefore: before.sidebarView, stripped, freshAfterStrip: brief(after) };
});

await step('s03b-toolbar-clean', async (partial) => {
  await stripHistory();
  await fresh();
  const r = { start: brief(await vs()) };
  // Sidebar: open, measure, the thumbnails, the options menu, the table of contents, a row click, reveal.
  r.sidebarViewKeyBefore = await ev(`return localStorage.getItem('pdfjs.sidebarView');`);
  await clickCtl('Toggle sidebar', 1500);
  r.afterToggle = brief(await vs());
  r.sidebarOpen = await ev(`${P} const content = P.viewContent(); const sb = content.querySelector('.pdf-sidebar-container'); const vc = content.querySelector('.pdf-viewer-container'); const cc = content.querySelector('.pdf-content-container');
    return { contentCls: P.cls(cc), sidebar: P.style(sb), viewerContainer: P.style(vc), resizer: P.style(content.querySelector('.pdf-sidebar-resizer')), thumbView: P.style(content.querySelector('.pdf-thumbnail-view')), outlineView: P.style(content.querySelector('.pdf-outline-view')),
      thumbs: Array.from(content.querySelectorAll('.thumbnail')).map((t) => ({ cls: P.cls(t), n: t.getAttribute('data-page-number'), style: t.getAttribute('style'), rect: P.rect(t), img: t.querySelector('img') && P.rect(t.querySelector('img')), css: P.style(t) })),
      thumbAnchors: Array.from(content.querySelectorAll('.pdf-thumbnail-view a')).slice(0, 2).map((a) => ({ href: a.getAttribute('href'), cls: P.cls(a), style: P.style(a) })),
      toggleBtn: P.style(content.querySelector('.pdf-toolbar [aria-label="Toggle sidebar"]')), sidebarViewKey: localStorage.getItem('pdfjs.sidebarView') };`);
  r.shotThumbs = await shot('03b-sidebar-thumbnails');
  // Click thumbnail 3.
  const t3 = r.sidebarOpen.thumbs.find((t) => t.n === '3');
  if (t3 && t3.rect.w) { await c.clickAt(t3.rect.x + t3.rect.w / 2, t3.rect.y + t3.rect.h / 2); await sleep(1000); r.afterThumb3 = brief(await vs()); r.thumbSelected = await ev(`${P} return Array.from(P.viewContent().querySelectorAll('.thumbnail')).map((t) => P.cls(t));`); }
  // Sidebar options -> Table of contents.
  await clickCtl('Sidebar options', 600);
  r.sidebarMenu = await ev(`${P} return P.menus();`);
  await clickMenuRow('Table of contents', 1200);
  r.afterToc = brief(await vs());
  r.outline = await ev(`${P} const content = P.viewContent(); const ov = content.querySelector('.pdf-outline-view'); const sb = content.querySelector('.pdf-sidebar-container');
    return { contentCls: P.cls(content.querySelector('.pdf-content-container')), dataView: sb && sb.getAttribute('data-view'), outlineView: P.style(ov), thumbView: P.style(content.querySelector('.pdf-thumbnail-view')), dump: P.dump(ov, 9, 30), styleTree: P.styleTree(ov, 5, 80), sidebarViewKey: localStorage.getItem('pdfjs.sidebarView'),
      rows: Array.from(ov ? ov.querySelectorAll('*') : []).filter((e) => e.children.length === 0 && e.textContent.trim()).map((e) => ({ text: e.textContent.trim(), cls: P.cls(e), parent: P.cls(e.parentElement), rect: P.rect(e), style: P.style(e) })) };`);
  r.shotOutline = await shot('03b-sidebar-outline');
  const row3 = r.outline.rows.find((x) => /^3 Three$/.test(x.text)) || r.outline.rows.find((x) => /Three/.test(x.text));
  if (row3 && row3.rect.w) {
    await c.clickAt(row3.rect.x + 6, row3.rect.y + row3.rect.h / 2); await sleep(1200);
    r.afterOutlineRow3 = brief(await vs());
    r.outlineAfterClick = await ev(`${P} const ov = P.viewContent().querySelector('.pdf-outline-view'); return Array.from(ov.querySelectorAll('*')).filter((e) => /is-active|selected|current/.test(P.cls(e))).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 40) }));`);
    const row31 = r.outline.rows.find((x) => /^3\.1 /.test(x.text));
    if (row31 && row31.rect.w) { await c.clickAt(row31.rect.x + 6, row31.rect.y + row31.rect.h / 2); await sleep(1200); r.afterOutlineRow31 = brief(await vs()); }
  }
  // Reveal page in table of contents, from page 5.
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentPageNumber = 5; return true;`); await sleep(600);
  await clickCtl('Sidebar options', 600); await clickMenuRow('Reveal page in table of contents', 1000);
  r.afterReveal = await ev(`${P} const ov = P.viewContent().querySelector('.pdf-outline-view'); return { active: Array.from(ov.querySelectorAll('*')).filter((e) => /is-active|selected|current/.test(P.cls(e))).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 40) })), scrollTop: ov.scrollTop, sidebarScroll: P.viewContent().querySelector('.pdf-sidebar-content').scrollTop };`);
  // Back to thumbnails, close the sidebar.
  await clickCtl('Sidebar options', 600); await clickMenuRow('Thumbnails', 800);
  await clickCtl('Toggle sidebar', 1200);
  r.afterClose = brief(await vs());
  r.sidebarViewKeyAfter = await ev(`return localStorage.getItem('pdfjs.sidebarView');`);
  r.historyAfterSidebar = await historyEntry();
  partial.r = r;
  // Does the sidebar state follow a re-open? Open, re-open, read.
  await clickCtl('Toggle sidebar', 1200);
  await fresh();
  r.reopenWithSidebarOpen = brief(await vs());
  r.historyReopen = await historyEntry();
  await ev(`${P} if (P.viewContent().querySelector('.pdf-content-container.sidebarOpen')) P.viewContent().querySelector('.pdf-toolbar [aria-label="Toggle sidebar"]').click(); return true;`); await sleep(600);
  // Display options: each row from a fresh default.
  r.display = {};
  for (const row of ['Fit width', 'Fit height', 'Single page', 'Two-page (odd)', 'Two-page (even)', 'Adapt to theme']) {
    await stripHistory(); await fresh();
    const before = brief(await vs());
    await ev(`${P} window.__sigBefore = P.sig(P.viewContent()); return true;`);
    await clickCtl('Display options', 600);
    const menu = await ev(`${P} return P.menus().map((m) => m.items.filter((i) => i.visible).map((i) => ({ title: i.title, icon: i.icon, checked: i.checked })));`);
    await clickMenuRow(row, 1300);
    const after = brief(await vs());
    const extra = await ev(`${P} const content = P.viewContent(); const vwr = content.querySelector('.pdfViewer'); const pages = Array.from(content.querySelectorAll('.page')).slice(0, 4).map((p) => ({ n: p.getAttribute('data-page-number'), rect: P.rect(p) }));
      return { delta: P.delta(window.__sigBefore, P.sig(content)), viewerCls: P.cls(vwr), viewerChildren: Array.from(vwr.children).map((ch) => P.cls(ch)).slice(0, 6), pages, pageCanvasFilter: getComputedStyle(content.querySelector('.page canvas')).filter, pageStyle: content.querySelector('.page') && content.querySelector('.page').getAttribute('style'), viewerStyle: vwr.getAttribute('style'), bodyVars: { pdfBackground: getComputedStyle(content).getPropertyValue('--pdf-background') }, history: null };`);
    await clickCtl('Display options', 600);
    const menuAfter = await ev(`${P} return P.menus().map((m) => m.items.filter((i) => i.visible).map((i) => ({ title: i.title, checked: i.checked })));`);
    await ev(`${P} P.dismiss(); return true;`);
    r.display[row] = { menuBefore: menu, before, after, ...extra, menuAfter, history: await historyEntry() };
    if (row === 'Two-page (odd)') r.display[row].shot = await shot('03b-two-page-odd');
    if (row === 'Adapt to theme') { r.display[row].shot = await shot('03b-adapt-to-theme'); r.display[row].canvasStyle = await ev(`${P} const cv = P.viewContent().querySelector('.page canvas'); const w = P.viewContent().querySelector('.page .canvasWrapper'); return { canvas: P.style(cv, ['filter', 'mix-blend-mode', 'background-color', 'opacity']), wrapper: P.style(w, ['filter', 'mix-blend-mode', 'background-color']), page: P.style(P.viewContent().querySelector('.page'), ['filter', 'background-color']), container: P.style(P.viewContent().querySelector('.pdf-viewer-container'), ['filter', 'background-color']) };`); }
    partial.r = r;
  }
  // Does "Adapt to theme" persist? It was clicked last; re-open and read.
  await fresh(); r.afterAdaptReopen = brief(await vs()); r.afterAdaptReopenFilter = await ev(`${P} return getComputedStyle(P.viewContent().querySelector('.page canvas')).filter;`);
  // Turn it off again if it stuck.
  await clickCtl('Display options', 600);
  const adapt = await ev(`${P} return P.menus()[0].items.find((i) => i.title === 'Adapt to theme');`);
  if (adapt && adapt.checked) await clickMenuRow('Adapt to theme', 800); else await ev(`${P} P.dismiss(); return true;`);
  r.adaptWasSticky = !!(adapt && adapt.checked);
  // The zoom ladder: in x6 then out x8 from the default.
  await stripHistory(); await fresh();
  r.zoomIn = [brief(await vs()).scale];
  for (let i = 0; i < 6; i++) { await clickCtl('Zoom in', 700); r.zoomIn.push((await vs()).viewer.currentScale); }
  await stripHistory(); await fresh();
  r.zoomOut = [brief(await vs()).scale];
  for (let i = 0; i < 8; i++) { await clickCtl('Zoom out', 700); r.zoomOut.push((await vs()).viewer.currentScale); }
  r.zoomOutState = brief(await vs());
  r.zoomScaleValueAfterOut = (await vs()).viewer.currentScaleValue;
  // Page input: type 3 and a real Enter; then 99, 0, abc.
  await stripHistory(); await fresh();
  r.pageInput = {};
  for (const v of ['3', '99', '0', 'abc', '2']) {
    const ir = await toolbarCtl('Toggle sidebar') && await ev(`${P} return P.rect(P.viewContent().querySelector('.pdf-page-input'));`);
    await c.clickAt(ir.x + ir.w / 2, ir.y + ir.h / 2); await sleep(150);
    await c.pressKey('a', { ctrl: true }); await c.typeText(v); await pressEnterReal(); await sleep(900);
    r.pageInput[v] = { ...brief(await vs()), inputValue: await ev(`${P} return P.viewContent().querySelector('.pdf-page-input').value;`), label: await ev(`${P} return P.viewContent().querySelector('.pdf-page-numbers').textContent;`) };
  }
  // The input's attributes.
  r.pageInputAttrs = await ev(`${P} const i = P.viewContent().querySelector('.pdf-page-input'); return Object.fromEntries(Array.from(i.attributes).map((a) => [a.name, a.value]));`);
  // Blur commits too?
  await ev(`${P} const i = P.viewContent().querySelector('.pdf-page-input'); i.focus(); i.value = '4'; i.blur(); return true;`); await sleep(800);
  r.pageInputBlur4 = brief(await vs());
  return r;
});

await step('s03c-find', async (partial) => {
  await stripHistory(); await fresh();
  const r = {};
  await ev(`${P} app.workspace.setActiveLeaf(P.pdfLeaf(), { focus: true }); return true;`); await sleep(200);
  r.focusBefore = (await vs()).ui.focus;
  await c.pressKey('f', { ctrl: true }); await sleep(900);
  r.open = await ev(`${P} const content = P.viewContent(); const fb = content.querySelector('.pdf-findbar'); const cc = content.querySelector('.pdf-content-container');
    return { contentCls: P.cls(cc), contentStyle: cc.getAttribute('style'), cls: P.cls(fb), rect: P.rect(fb), style: P.style(fb), dump: P.dump(fb, 8, 30), styleTree: P.styleTree(fb, 5, 80), viewerContainer: P.rect(content.querySelector('.pdf-viewer-container')), focus: P.ui().focus, focusInside: fb.contains(document.activeElement),
      controls: Array.from(fb.querySelectorAll('button, .clickable-icon, input, label, span')).map((e) => ({ tag: e.tagName.toLowerCase(), cls: P.cls(e), aria: e.getAttribute('aria-label'), title: e.getAttribute('title'), icon: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), text: e.textContent.trim().slice(0, 40), placeholder: e.placeholder, type: e.type, rect: P.rect(e) })) };`);
  partial.open = r.open;
  await c.typeText('three'); await sleep(1000);
  const readFind = () => ev(`${P} const content = P.viewContent(); const fb = content.querySelector('.pdf-findbar'); const v = P.viewerOf(P.pdfLeaf().view);
    return { count: fb.querySelector('.pdf-find-results-count') && fb.querySelector('.pdf-find-results-count').textContent, message: fb.querySelector('.pdf-findbar-message') && fb.querySelector('.pdf-findbar-message').textContent.trim(), inputValue: fb.querySelector('input').value, inputCls: P.cls(fb.querySelector('input')), page: v.currentPageNumber, scrollTop: v.container.scrollTop,
      highlights: Array.from(content.querySelectorAll('.textLayer .highlight')).map((h) => ({ cls: P.cls(h), text: h.textContent, page: h.closest('.page') && h.closest('.page').getAttribute('data-page-number'), bg: getComputedStyle(h).backgroundColor, color: getComputedStyle(h).color, rect: P.rect(h) })).slice(0, 12), focus: P.ui().focus };`);
  r.afterTyping = await readFind();
  await pressEnterReal(); await sleep(1200);
  r.afterEnter = await readFind();
  r.shot = await shot('03c-find-match');
  await pressEnterReal(); await sleep(900);
  r.afterEnter2 = await readFind();
  await c.pressKey('Enter', { shift: true }); await sleep(900);
  r.afterShiftEnter = await readFind();
  // The next/previous buttons and the settings menu.
  const btns = r.open.controls.filter((x) => x.tag === 'button' && x.rect.w);
  r.buttons = [];
  for (const b of btns) { await c.clickAt(b.rect.x + b.rect.w / 2, b.rect.y + b.rect.h / 2); await sleep(800); r.buttons.push({ aria: b.aria, icon: b.icon, after: await readFind() }); }
  const settings = r.open.controls.find((x) => /settings/.test(x.cls));
  if (settings) { await c.clickAt(settings.rect.x + settings.rect.w / 2, settings.rect.y + settings.rect.h / 2); await sleep(600); r.settingsMenu = await ev(`${P} return P.menus();`); r.settingsMenuItems = await c.openMenuItems(); await ev(`${P} P.dismiss(); return true;`); }
  const deco = r.open.controls.find((x) => /input-right-decorator/.test(x.cls));
  if (deco) { await c.clickAt(deco.rect.x + deco.rect.w / 2, deco.rect.y + deco.rect.h / 2); await sleep(800); r.afterDecorator = { ...(await readFind()), decoratorCls: await ev(`${P} return P.cls(P.viewContent().querySelector('.pdf-findbar .input-right-decorator'));`), aria: deco.aria }; await c.clickAt(deco.rect.x + deco.rect.w / 2, deco.rect.y + deco.rect.h / 2); await sleep(300); }
  // A query with no match.
  const inp = r.open.controls.find((x) => x.tag === 'input' && x.rect.w);
  await c.clickAt(inp.rect.x + inp.rect.w / 2, inp.rect.y + inp.rect.h / 2); await c.pressKey('a', { ctrl: true }); await c.typeText('zzqx'); await sleep(900);
  r.noMatch = await readFind();
  r.noMatchClasses = await ev(`${P} const fb = P.viewContent().querySelector('.pdf-findbar'); return { fb: P.cls(fb), input: P.cls(fb.querySelector('input')), inputStyle: P.style(fb.querySelector('input')), wrapper: P.cls(fb.querySelector('.pdf-search-wrapper')) };`);
  await c.pressKey('Escape'); await sleep(600);
  r.afterEscape = await ev(`${P} const content = P.viewContent(); const fb = content.querySelector('.pdf-findbar'); return { cls: P.cls(fb), contentCls: P.cls(content.querySelector('.pdf-content-container')), display: getComputedStyle(fb).display, focus: P.ui().focus, highlightsLeft: content.querySelectorAll('.textLayer .highlight').length, viewerContainer: P.rect(content.querySelector('.pdf-viewer-container')) };`);
  // The close button, if any: reopen and click the last clickable-icon in the bar.
  await c.pressKey('f', { ctrl: true }); await sleep(600);
  const closeBtn = await ev(`${P} const fb = P.viewContent().querySelector('.pdf-findbar'); const els = Array.from(fb.querySelectorAll('.clickable-icon')); const e = els[els.length - 1]; return e ? { cls: P.cls(e), aria: e.getAttribute('aria-label'), icon: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), rect: P.rect(e) } : null;`);
  if (closeBtn) { await c.clickAt(closeBtn.rect.x + closeBtn.rect.w / 2, closeBtn.rect.y + closeBtn.rect.h / 2); await sleep(500); r.closeButton = { ...closeBtn, after: await ev(`${P} return { cls: P.cls(P.viewContent().querySelector('.pdf-findbar')), focus: P.ui().focus };`) }; }
  return r;
});

await step('s04b-zoom-options', async () => {
  // There is no zoom <select>: the zoom options are the Display options menu's
  // Fit width / Fit height plus the two step buttons. Three explicit scales
  // through the viewer, for the page width each gives.
  await stripHistory(); await fresh();
  const read = () => ev(`${P} const s = P.pdfState(); const v = s.viewer; return { scale: v.currentScale, scaleValue: v.currentScaleValue, page1: v.firstPage && v.firstPage.rect, canvas: v.firstPage && v.firstPage.canvas, clientWidth: v.clientWidth, clientHeight: v.clientHeight, scaleFactor: P.viewContent().querySelector('.pdfViewer').style.getPropertyValue('--scale-factor'), state: s.state };`);
  const r = { default: await read() };
  for (const v of ['1', '1.5', 'page-fit', 'page-width', 'auto', 'page-actual', '0.25', '10']) {
    await ev(`${P} const vw = P.viewerOf(P.pdfLeaf().view); vw.currentScaleValue = ${JSON.stringify(v)}; return true;`); await sleep(900);
    r[v] = await read();
  }
  r.shotPageFit = null;
  await ev(`${P} const vw = P.viewerOf(P.pdfLeaf().view); vw.currentScaleValue = 'page-fit'; return true;`); await sleep(900);
  r.shotPageFit = await shot('04b-page-fit');
  r.history = await historyEntry();
  return r;
});

await step('s05b-keyboard', async (partial) => {
  await stripHistory(); await fresh();
  const pageTop = async () => { const r = await c.rect('.workspace-leaf.mod-active .pdf-viewer-container'); return r; };
  const cont = await pageTop();
  const read = () => ev(`${P} const s = P.pdfState(); const v = s.viewer; const content = P.viewContent(); return { page: v.currentPageNumber, scrollTop: v.scrollTop, scrollLeft: v.scrollLeft, scale: v.currentScale, scaleValue: v.currentScaleValue, rotation: v.pagesRotation, spread: v.spreadMode, ui: P.ui(), findbar: P.cls(content.querySelector('.pdf-findbar')), zoomFactor: window.require('electron').webFrame.getZoomFactor(), fullscreen: !!document.fullscreenElement, sidebarOpen: !!content.querySelector('.pdf-content-container.sidebarOpen') };`);
  const reset = async () => { await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); P.dismiss(); for (const m of document.querySelectorAll('.modal-container')) m.remove(); v.pagesRotation = 0; v.currentPageNumber = 1; v.container.scrollTop = 0; v.currentScaleValue = 'page-width'; return true;`); await sleep(400); };
  const keys = [
    ['Down', () => c.pressKey('ArrowDown')], ['Up', () => c.pressKey('ArrowUp'), 3],
    ['PageDown', () => c.pressKey('PageDown')], ['PageUp', () => c.pressKey('PageUp'), 3],
    ['End', () => c.pressKey('End')], ['Home', () => c.pressKey('Home'), 3],
    ['Space', () => pressRaw(' ', 'Space', 32)], ['Shift+Space', () => pressRaw(' ', 'Space', 32, { shift: true }), 3],
    ['j', () => c.pressKey('j')], ['k', () => c.pressKey('k'), 3], ['n', () => c.pressKey('n')], ['p', () => c.pressKey('p'), 3],
    ['Right', () => c.pressKey('ArrowRight')], ['Left', () => c.pressKey('ArrowLeft'), 3],
    ['Ctrl+=', () => pressRaw('=', 'Equal', 187, { ctrl: true })], ['Ctrl+-', () => pressRaw('-', 'Minus', 189, { ctrl: true })], ['Ctrl+0', () => pressRaw('0', 'Digit0', 48, { ctrl: true })],
    ['r', () => c.pressKey('r')], ['Shift+r', () => pressRaw('R', 'KeyR', 82, { shift: true })],
    ['Ctrl+f', () => c.pressKey('f', { ctrl: true })], ['Escape', () => c.pressKey('Escape')],
    ['Ctrl+p', () => c.pressKey('p', { ctrl: true })], ['Ctrl+a', () => c.pressKey('a', { ctrl: true })],
    ['Ctrl+Wheel+100', () => wheel(cont.cx, cont.cy, 100, 0, 2)], ['Ctrl+Wheel-100', () => wheel(cont.cx, cont.cy, -100, 0, 2)],
  ];
  const rows = { apiFocus: [], clickFocus: [] };
  for (const mode of ['apiFocus', 'clickFocus']) {
    for (const [name, press, startPage] of keys) {
      await reset();
      if (startPage) { await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentPageNumber = ${startPage}; return true;`); await sleep(300); }
      if (mode === 'apiFocus') { await ev(`${P} app.workspace.setActiveLeaf(P.pdfLeaf(), { focus: true }); const v = P.viewerOf(P.pdfLeaf().view); v.container.focus(); return true;`); }
      else { await c.clickAt(cont.cx, cont.y + 60); }
      await sleep(250);
      const before = await read();
      await press(); await sleep(800);
      const after = await read();
      const row = { key: name, focusBefore: before.ui.focus, before: { page: before.page, scrollTop: before.scrollTop, scale: before.scale, scaleValue: before.scaleValue, rotation: before.rotation, zoomFactor: before.zoomFactor }, after: { page: after.page, scrollTop: after.scrollTop, scrollLeft: after.scrollLeft, scale: after.scale, scaleValue: after.scaleValue, rotation: after.rotation, spread: after.spread, zoomFactor: after.zoomFactor, focus: after.ui.focus, modals: after.ui.modals, menus: after.ui.menus, findbar: after.findbar, fullscreen: after.fullscreen, sidebarOpen: after.sidebarOpen } };
      if (name === 'Ctrl+a') row.selectionLength = await ev(`return window.getSelection().toString().length;`);
      rows[mode].push(row);
      partial.rows = rows;
      if (after.ui.modals.length || after.ui.menus.length) { await c.pressKey('Escape'); await sleep(300); }
      if (/findbarOpen/.test(after.findbar) || after.ui.focus.startsWith('input')) { await c.pressKey('Escape'); await sleep(300); }
      if (after.zoomFactor !== before.zoomFactor) { await pressRaw('0', 'Digit0', 48, { ctrl: true }); await sleep(600); }
      if (after.fullscreen) { await c.pressKey('Escape'); await sleep(500); }
    }
  }
  const scope = await ev(`${P} const leaf = P.pdfLeaf(); const v = leaf.view; const ser = (s) => s && s.keys ? s.keys.map((k) => ({ modifiers: k.modifiers, key: k.key })) : null; return { viewScope: ser(v.scope), childScope: v.viewer && v.viewer.scope ? ser(v.viewer.scope) : null, grandchildScope: v.viewer && v.viewer.child && v.viewer.child.scope ? ser(v.viewer.child.scope) : null, zoomCommands: ['window:zoom-in', 'window:zoom-out', 'window:reset-zoom'].map((id) => ({ id, exists: !!app.commands.commands[id], name: app.commands.commands[id] && app.commands.commands[id].name, hotkeys: app.hotkeyManager.getDefaultHotkeys(id) })) };`);
  await stripHistory();
  return { container: cont, scope, rows };
});

await step('s06b-scroll-history', async () => {
  await stripHistory(); await fresh();
  const read = () => ev(`${P} const s = P.pdfState(); const v = s.viewer; return { page: v.currentPageNumber, scrollTop: v.scrollTop, scale: v.currentScale, scaleValue: v.currentScaleValue, rotation: v.pagesRotation, location: v.location, state: s.state };`);
  const r = { start: await read(), historyStart: await historyEntry() };
  const cont = await c.rect('.workspace-leaf.mod-active .pdf-viewer-container');
  await wheel(cont.cx, cont.cy, 900); await sleep(900);
  r.afterWheel900 = await read(); r.historyAfterWheel = await historyEntry();
  await wheel(cont.cx, cont.cy, 100); await sleep(700);
  r.afterWheel100 = await read();
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.container.scrollTop = 1800; return true;`); await sleep(900);
  r.afterScrollTop1800 = await read(); r.historyAfterScrollTop = await historyEntry();
  // The page number at page boundaries, unrotated.
  r.boundary = await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); const c = v.container; const off = (n) => c.querySelector('.page[data-page-number="' + n + '"]').offsetTop; const h = (n) => c.querySelector('.page[data-page-number="' + n + '"]').offsetHeight;
    const res = { offsets: [1, 2, 3, 4, 5].map(off), heights: [1, 2, 3, 4, 5].map(h), clientHeight: c.clientHeight };
    for (const [name, y] of [['p2top-10', off(2) - 10], ['p2top', off(2)], ['p2top+10', off(2) + 10], ['p2mid', off(2) + h(2) / 2], ['p2bottom-10', off(2) + h(2) - 10], ['p3top-1', off(3) - 1], ['p3top+1', off(3) + 1], ['half-of-p1', h(1) / 2], ['p1-60pct', h(1) * 0.6]]) { c.scrollTop = y; await P.sleep(350); res[name] = { scrollTop: c.scrollTop, page: v.currentPageNumber, loc: v._location && { pageNumber: v._location.pageNumber, top: v._location.top } }; }
    return res;`);
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.container.scrollTop = 1800; return true;`); await sleep(500);
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  r.workspaceJsonPdfLeaf = wsLeaves(wsJson(), 'pdf');
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentScaleValue = '1.5'; return true;`); await sleep(800);
  r.afterScale15 = await read(); r.historyAfterScale = await historyEntry();
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.pagesRotation = 90; return true;`); await sleep(800);
  r.afterRotate90 = await read(); r.historyAfterRotate = await historyEntry();
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  r.workspaceJsonPdfLeafAfterZoomRotate = wsLeaves(wsJson(), 'pdf');
  // Re-open: what comes back from the history, and what the leaf's own state says.
  await fresh();
  r.reopen = await read(); r.historyReopen = await historyEntry();
  // Undo, strip.
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.pagesRotation = 0; v.currentScaleValue = 'page-width'; v.currentPageNumber = 1; return true;`); await sleep(600);
  r.historyAfterUndo = await historyEntry();
  r.stripped = await stripHistory();
  await fresh();
  r.reopenAfterStrip = await read();
  return r;
});

await step('s07b-links', async (partial) => {
  await stripHistory(); await fresh();
  const read = () => ev(`${P} const s = P.pdfState(); const v = s && s.viewer; const vw = P.pdfLeaf() ? P.viewerOf(P.pdfLeaf().view) : null; return { leaves: P.leaves(), active: app.workspace.activeLeaf.id, pdfLeafId: P.pdfLeaf() && P.pdfLeaf().id, state: s && s.state, page: v && v.currentPageNumber, scrollTop: v && v.scrollTop, scaleValue: v && v.currentScaleValue, scale: v && v.currentScale, location: v && v.location, notices: P.ui().notices, offsets: vw && vw.container ? [1, 2, 3, 4, 5].map((n) => { const p = vw.container.querySelector('.page[data-page-number="' + n + '"]'); return p ? p.offsetTop : null; }) : null };`);
  const r = { viaApi: [] };
  for (const link of ['doc.pdf#page=4', 'doc.pdf#page=2', 'doc.pdf#page=99', 'doc.pdf#page=0', 'doc.pdf#page=abc', 'doc.pdf#foo', 'doc.pdf#height=300', 'doc.pdf#page=3&height=200', 'doc.pdf', 'doc.pdf#page=5', 'doc.pdf#page=2', 'doc.pdf#page=-1', 'doc.pdf#page=2.7', 'doc.pdf#page=4', 'doc.pdf#page=1']) {
    const before = await read();
    let err = null;
    try { await ev(`await app.workspace.openLinkText(${JSON.stringify(link)}, '', false); return true;`); } catch (e) { err = e.message; }
    await sleep(1500);
    const after = await read();
    r.viaApi.push({ link, error: err, before: { page: before.page, scrollTop: before.scrollTop, scale: before.scale, leaves: before.leaves.length, pdfLeafId: before.pdfLeafId }, after: { page: after.page, scrollTop: after.scrollTop, scaleValue: after.scaleValue, scale: after.scale, leaves: after.leaves.length, pdfLeafId: after.pdfLeafId, active: after.active, location: after.location, notices: after.notices, offsets: after.offsets, state: after.state } });
    partial.viaApi = r.viaApi;
  }
  {
    const before = await read();
    await ev(`await app.workspace.openLinkText('doc.pdf#page=3', '', true); return true;`); await sleep(1500);
    const after = await read();
    r.newLeafPage3 = { leavesBefore: before.leaves.length, leavesAfter: after.leaves.length, page: after.page, pdfLeaves: after.leaves.filter((l) => l.type === 'pdf') };
  }
  // Real clicks in the reading view: the links are re-found each time, the preview re-renders when the leaf changes.
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(1500);
  await stripHistory();
  await ev(`window.__clicks = []; if (!window.__linkGuard) { window.__linkGuard = (ev) => { const a = ev.target.closest ? ev.target.closest('a') : null; window.__clicks.push({ href: a ? a.getAttribute('href') : null, dataHref: a ? a.getAttribute('data-href') : null, cls: a ? a.className : null, defaultPrevented: ev.defaultPrevented, tag: ev.target.tagName }); ev.preventDefault(); }; document.addEventListener('click', window.__linkGuard, false); } return true;`);
  const findLinks = () => ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view');
    pv.scrollTop = pv.scrollHeight; await P.sleep(600);
    return Array.from(pv.querySelectorAll('a')).filter((a) => !a.closest('.internal-embed') && a.offsetParent).map((a, i) => ({ i, text: a.textContent, href: a.getAttribute('href'), dataHref: a.getAttribute('data-href'), cls: a.className, target: a.getAttribute('target'), rel: a.getAttribute('rel'), rect: P.rect(a) }));`);
  r.links = await findLinks();
  r.clicks = [];
  for (let i = 0; i < r.links.length; i++) {
    // A click navigates the note's own leaf to the pdf, so the note has to be re-opened for the next link.
    await ev(`${P} let ml = app.workspace.getLeavesOfType('markdown')[0]; if (!ml) { app.workspace.detachLeavesOfType('pdf'); ml = await P.openMd('Embeds.md', 'preview'); await P.sleep(1200); } app.workspace.setActiveLeaf(ml, { focus: true }); return true;`); await sleep(400);
    const links = await findLinks(); const l = links[i]; if (!l) { r.clicks.push({ i, error: 'link not found' }); continue; }
    await stripHistory();
    const before = await read();
    await c.clickAt(l.rect.x + Math.min(10, l.rect.w / 2), l.rect.y + l.rect.h / 2); await sleep(1600);
    const after = await read();
    r.clicks.push({ link: { text: l.text, href: l.href, dataHref: l.dataHref, cls: l.cls, target: l.target, rel: l.rel }, before: { leaves: before.leaves.length, pdfLeafId: before.pdfLeafId, pdfPage: before.page }, after: { leaves: after.leaves, active: after.active, activeType: after.leaves.find((x) => x.active) && after.leaves.find((x) => x.active).type, pdfLeafId: after.pdfLeafId, page: after.page, scrollTop: after.scrollTop, scaleValue: after.scaleValue, location: after.location, state: after.state }, guard: await ev(`return window.__clicks.splice(0);`) });
    partial.clicks = r.clicks;
  }
  // Ctrl+click and middle click on the first link: a new tab?
  for (const [name, opts] of [['ctrlClick', { mods: { ctrl: true } }], ['middleClick', { button: 'middle' }]]) {
    await ev(`${P} let ml = app.workspace.getLeavesOfType('markdown')[0]; if (!ml) { app.workspace.detachLeavesOfType('pdf'); ml = await P.openMd('Embeds.md', 'preview'); await P.sleep(1200); } app.workspace.setActiveLeaf(ml, { focus: true }); return true;`); await sleep(400);
    const links = await findLinks(); const l = links[1]; if (!l) continue;
    const before = await read();
    await c.clickAt(l.rect.x + 10, l.rect.y + l.rect.h / 2, opts); await sleep(1500);
    const after = await read();
    r[name] = { link: l.href, leavesBefore: before.leaves.length, leavesAfter: after.leaves.length, pdfLeaves: after.leaves.filter((x) => x.type === 'pdf'), active: after.leaves.find((x) => x.active), page: after.page };
  }
  await ev(`document.removeEventListener('click', window.__linkGuard, false); window.__linkGuard = null; return true;`);
  await stripHistory();
  return r;
});

// The embeds of Embeds.md by the line each one sits on, scrolled to in turn.
const EMBED_LINES = (() => { const lines = []; let n = 2; for (const [label, src] of EMBEDS) { n += 1; lines.push({ label, src, line: n + 1 }); n += 3; } return lines; })();
const embedInfoAt = (mode, line, src) => ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0];
  if (${JSON.stringify(mode)} === 'preview') { leaf.view.previewMode.applyScroll(${line}); } else { leaf.view.editor.scrollIntoView({ from: { line: ${line}, ch: 0 }, to: { line: ${line}, ch: 0 } }, true); }
  await P.sleep(1100);
  const root = leaf.view.containerEl.querySelector(${JSON.stringify(mode)} === 'preview' ? '.markdown-preview-view' : '.cm-scroller');
  const H = window.innerHeight;
  const cands = Array.from(root.querySelectorAll('.internal-embed, .file-embed')).filter((e) => e.offsetParent && e.getAttribute('src') === ${JSON.stringify(src)});
  const onScreen = cands.filter((e) => { const b = e.getBoundingClientRect(); return b.bottom > 0 && b.top < H; });
  const target = ${JSON.stringify(mode)} === 'preview' ? 0 : H / 2;
  onScreen.sort((a, b) => Math.abs(a.getBoundingClientRect().top - target) - Math.abs(b.getBoundingClientRect().top - target));
  const el = onScreen[0] || cands[0]; if (!el) return { missing: true, src: ${JSON.stringify(src)}, cands: cands.length };
  const viewers = P.findViewers(leaf.view, 9, 4000); const v = viewers.find((x) => x.viewer.container && el.contains(x.viewer.container));
  const info = P.embedInfo({ el, viewer: v ? v.viewer : null, viewerPath: v ? v.path : null, wrapper: el.closest('.cm-embed-block') ? P.cls(el.closest('.cm-embed-block')) : null, wrapperRect: el.closest('.cm-embed-block') ? P.rect(el.closest('.cm-embed-block')) : null }, 5);
  info.inlineStyle = el.getAttribute('style'); info.onScreen = onScreen.length; info.candidates = cands.length;
  info.parts = { toolbar: P.style(el.querySelector('.pdf-toolbar')), container: P.style(el.querySelector('.pdf-container')), viewerContainer: P.style(el.querySelector('.pdf-viewer-container')), pdfViewer: P.style(el.querySelector('.pdfViewer')), firstPage: P.style(el.querySelector('.page')), canvasWrapper: P.style(el.querySelector('.canvasWrapper')), toolbarControls: Array.from(el.querySelectorAll('.pdf-toolbar .clickable-icon, .pdf-toolbar input, .pdf-toolbar span')).map((e) => ({ cls: P.cls(e), aria: e.getAttribute('aria-label'), text: e.textContent.trim(), value: e.value, hidden: !e.offsetParent })) };
  info.sidebarEl = el.querySelector('.pdf-sidebar-container') ? P.style(el.querySelector('.pdf-sidebar-container')) : null; info.findbarEl = el.querySelector('.pdf-findbar') ? P.cls(el.querySelector('.pdf-findbar')) : null;
  info.parentChain = (() => { const out = []; let p = el.parentElement; let n = 0; while (p && n++ < 4) { out.push(P.cls(p) || p.tagName.toLowerCase()); p = p.parentElement; } return out; })();
  info.noteScrollTop = root.scrollTop;
  return info;`);

await step('s08b-embeds', async (partial) => {
  await stripHistory();
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(2000);
  const r = { embedLines: EMBED_LINES, preview: [], livePreview: [] };
  for (const e of EMBED_LINES) { const info = await embedInfoAt('preview', e.line, e.src.replace(/^!\[\[|\]\]$/g, '').replace(/\|.*$/, '').replace(/^!\[[^\]]*\]\(|\)$/g, '')); info.label = e.label; r.preview.push(info); partial.preview = r.preview; }
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.previewMode.applyScroll(3); return true;`); await sleep(1000);
  r.shotPreview = await shot('08b-embeds-reading');
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.previewMode.applyScroll(${EMBED_LINES[8].line - 3}); return true;`); await sleep(1000);
  r.shotPreviewBrokenSmall = await shot('08b-embeds-reading-broken-small');
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; await leaf.setViewState({ type: 'markdown', state: { file: 'Embeds.md', mode: 'source', source: false } }); await P.sleep(800); leaf.view.editor.setCursor({ line: 0, ch: 0 }); return true;`); await sleep(1500);
  for (const e of EMBED_LINES) { const info = await embedInfoAt('live', e.line, e.src.replace(/^!\[\[|\]\]$/g, '').replace(/\|.*$/, '').replace(/^!\[[^\]]*\]\(|\)$/g, '')); info.label = e.label; r.livePreview.push(info); partial.livePreview = r.livePreview; }
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.editor.scrollIntoView({ from: { line: 3, ch: 0 }, to: { line: 3, ch: 0 } }, true); return true;`); await sleep(1000);
  r.shotLivePreview = await shot('08b-embeds-live-preview');
  // Caret on the embed line in Live Preview: does the widget give way to the source?
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.editor.setCursor({ line: ${EMBED_LINES[0].line}, ch: 3 }); return true;`); await sleep(900);
  r.livePreviewCaretOnEmbed = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.cm-scroller'); const line = sc.querySelector('.cm-active'); return { activeLineText: line && line.textContent, activeLineCls: line && P.cls(line), embedsVisible: Array.from(sc.querySelectorAll('.internal-embed')).filter((e) => e.offsetParent).length, activeLineHasEmbed: !!(line && line.querySelector('.internal-embed')), nextEl: line && line.nextElementSibling && P.cls(line.nextElementSibling) };`);
  // Tall.md: lazy build in both modes.
  await ev(`${P} await P.openMd('Tall.md', 'preview'); return true;`); await sleep(2500);
  r.tall = {};
  r.tall.previewBeforeScroll = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const emb = pv.querySelector('.internal-embed'); const secs = (leaf.view.previewMode.renderer && leaf.view.previewMode.renderer.sections) || []; const s82 = secs.find((s) => s.lineStart === 82); return { scrollTop: pv.scrollTop, scrollHeight: pv.scrollHeight, clientHeight: pv.clientHeight, sectionsTotal: secs.length, sectionsRendered: secs.filter((s) => s.rendered).length, sectionKeys: secs[0] ? Object.keys(secs[0]).slice(0, 20) : null, embedSection: s82 ? { rendered: s82.rendered, lineStart: s82.lineStart, lineEnd: s82.lineEnd, hasEl: !!s82.el, elChildren: s82.el ? s82.el.children.length : null } : null, embedExists: !!emb, embedPages: emb ? emb.querySelectorAll('.page').length : 0, embedCanvases: emb ? emb.querySelectorAll('canvas').length : 0, viewers: P.findViewers(leaf.view, 9, 3000).length };`);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.previewMode.applyScroll(82); return true;`); await sleep(1500);
  r.tall.previewAfterScroll = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view'); const emb = pv.querySelector('.internal-embed'); return { scrollTop: pv.scrollTop, embedExists: !!emb, embedRect: emb && P.rect(emb), inlineStyle: emb && emb.getAttribute('style'), embedPages: emb ? emb.querySelectorAll('.page').length : 0, embedCanvases: emb ? emb.querySelectorAll('canvas').length : 0, pagesLoaded: emb ? Array.from(emb.querySelectorAll('.page')).map((p) => p.getAttribute('data-loaded')) : null, viewers: P.findViewers(leaf.view, 9, 3000).length };`);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; await leaf.setViewState({ type: 'markdown', state: { file: 'Tall.md', mode: 'source', source: false } }); await P.sleep(600); leaf.view.editor.setCursor({ line: 0, ch: 0 }); return true;`); await sleep(2000);
  r.tall.livePreviewBeforeScroll = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.cm-scroller'); const emb = sc.querySelector('.internal-embed'); return { scrollTop: sc.scrollTop, scrollHeight: sc.scrollHeight, lines: sc.querySelectorAll('.cm-line').length, embedExists: !!emb, embedPages: emb ? emb.querySelectorAll('.page').length : 0, viewers: P.findViewers(leaf.view, 9, 3000).length };`);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.editor.scrollIntoView({ from: { line: 82, ch: 0 }, to: { line: 82, ch: 0 } }, true); return true;`); await sleep(1500);
  r.tall.livePreviewAfterScroll = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.cm-scroller'); const emb = sc.querySelector('.internal-embed'); return { scrollTop: sc.scrollTop, embedExists: !!emb, embedRect: emb && P.rect(emb), inlineStyle: emb && emb.getAttribute('style'), embedPages: emb ? emb.querySelectorAll('.page').length : 0, embedCanvases: emb ? emb.querySelectorAll('canvas').length : 0 };`);
  return r;
});

await step('s09b-embed-interaction', async (partial) => {
  await stripHistory();
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(2000);
  const first = EMBED_LINES[0];
  const locate = () => ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const pv = leaf.view.containerEl.querySelector('.markdown-preview-view');
    const el = Array.from(pv.querySelectorAll('.internal-embed')).find((e) => e.offsetParent && e.getAttribute('src') === 'doc.pdf'); if (!el) return null; window.__emb = el;
    const v = P.findViewers(leaf.view, 9, 4000).find((x) => x.viewer.container && el.contains(x.viewer.container));
    return { rect: P.rect(el), noteScrollTop: pv.scrollTop, inner: v ? { scrollTop: v.viewer.container.scrollTop, scrollHeight: v.viewer.container.scrollHeight, clientHeight: v.viewer.container.clientHeight, page: v.viewer.currentPageNumber, scaleValue: v.viewer.currentScaleValue } : null, input: el.querySelector('.pdf-page-input') && { value: el.querySelector('.pdf-page-input').value, rect: P.rect(el.querySelector('.pdf-page-input')) }, label: el.querySelector('.pdf-page-numbers') && el.querySelector('.pdf-page-numbers').textContent };`);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.previewMode.applyScroll(${first.line - 1}); return true;`); await sleep(1200);
  const r = { before: await locate() };
  if (!r.before) throw new Error('first embed not on screen');
  partial.r = r;
  const mid = () => ({ x: r.before.rect.x + r.before.rect.w / 2, y: Math.min(out.version.win[1] - 40, r.before.rect.y + r.before.rect.h / 2) });
  await wheel(mid().x, mid().y, 300); await sleep(800); r.afterWheel300 = await locate();
  await wheel(mid().x, mid().y, 300); await sleep(800); r.afterWheel600 = await locate();
  // Scroll the inner viewer to its end, then wheel again: does the note take over?
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const el = window.__emb; const v = P.findViewers(leaf.view, 9, 4000).find((x) => x.viewer.container && el.contains(x.viewer.container)); v.viewer.container.scrollTop = v.viewer.container.scrollHeight; return true;`); await sleep(500);
  r.atEnd = await locate();
  const atEndMid = { x: r.atEnd.rect.x + r.atEnd.rect.w / 2, y: Math.max(40, Math.min(out.version.win[1] - 40, r.atEnd.rect.y + r.atEnd.rect.h / 2)) };
  await wheel(atEndMid.x, atEndMid.y, 300); await sleep(800); r.afterWheelAtEnd = await locate();
  // Back up, then the page input.
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.previewMode.applyScroll(${first.line - 1}); return true;`); await sleep(1000);
  const loc = await locate();
  if (loc && loc.input) {
    await c.clickAt(loc.input.rect.x + loc.input.rect.w / 2, loc.input.rect.y + loc.input.rect.h / 2); await sleep(200);
    r.focusAfterInputClick = (await ev(`${P} return P.ui();`)).focus;
    await c.pressKey('a', { ctrl: true }); await c.typeText('3'); await pressEnterReal(); await sleep(1000);
    r.afterPageInput3 = await locate();
  }
  // Click on a page inside the embed: focus and the note's state.
  const loc2 = await locate();
  await c.clickAt(loc2.rect.x + loc2.rect.w / 2, Math.min(out.version.win[1] - 40, loc2.rect.y + 120)); await sleep(400);
  r.afterPageClick = { focus: (await ev(`${P} return P.ui();`)).focus, ...(await locate()) };
  r.noteState = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; return { viewState: leaf.getViewState(), state: leaf.view.getState(), ephemeral: leaf.view.getEphemeralState ? leaf.view.getEphemeralState() : null };`);
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  const ws = wsJson();
  r.workspaceMarkdownLeaves = wsLeaves(ws, 'markdown'); r.workspaceMentionsPdf = JSON.stringify(ws).includes('pdf');
  r.history = await historyEntry();
  await stripHistory();
  return r;
});

await step('s10b-canvas', async (partial) => {
  await stripHistory();
  await ev(`${P} const f = await P.file('Board.canvas'); const leaf = app.workspace.getLeaf('tab'); await leaf.openFile(f); app.workspace.setActiveLeaf(leaf, { focus: true }); for (const t of ['pdf', 'markdown']) app.workspace.detachLeavesOfType(t); for (const l of app.workspace.getLeavesOfType('canvas')) if (l !== leaf) l.detach(); return true;`); await sleep(2500);
  const nodeInfo = (id) => ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get(${JSON.stringify(id)}); const el = n.nodeEl; const viewers = P.findViewers(n, 7, 2500);
    const emb = el.querySelector('.pdf-embed'); return { data: n.getData(), rect: P.rect(el), cls: P.cls(el), label: el.querySelector('.canvas-node-label') && { text: el.querySelector('.canvas-node-label').textContent, style: P.style(el.querySelector('.canvas-node-label')) }, contentCls: P.cls(el.querySelector('.canvas-node-content')), embedCls: emb && P.cls(emb), embedStyle: emb && emb.getAttribute('style'), embedRect: emb && P.rect(emb), toolbar: el.querySelector('.pdf-toolbar') && { rect: P.rect(el.querySelector('.pdf-toolbar')), text: el.querySelector('.pdf-toolbar').textContent.trim(), controls: Array.from(el.querySelectorAll('.pdf-toolbar .clickable-icon')).map((e) => e.getAttribute('aria-label')) }, container: el.querySelector('.pdf-container') && P.style(el.querySelector('.pdf-container')), pages: Array.from(el.querySelectorAll('.page')).map((p) => ({ n: p.getAttribute('data-page-number'), loaded: p.getAttribute('data-loaded'), rect: P.rect(p), canvas: !!p.querySelector('canvas') })).slice(0, 6), viewers: viewers.map((v) => ({ path: v.path, page: v.viewer.currentPageNumber, scaleValue: v.viewer.currentScaleValue, scale: v.viewer.currentScale, rotation: v.viewer.pagesRotation, clientWidth: v.viewer.container.clientWidth, clientHeight: v.viewer.container.clientHeight, scrollTop: v.viewer.container.scrollTop })), childSubpath: n.child && n.child.subpath, isEditing: n.isEditing, dump: P.dump(el, 7, 10) };`);
  const r = { nodes: {} };
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; leaf.view.canvas.zoomToFit(); return true;`); await sleep(1200);
  r.fit = { viewState: await ev(`${P} return app.workspace.getLeavesOfType('canvas')[0].getViewState();`), shot: await shot('10b-canvas-fit') };
  for (const id of ['f1', 'f2', 't1']) {
    await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get(${JSON.stringify(id)}); cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, 0); return true;`); await sleep(1500);
    r.nodes[id] = await nodeInfo(id);
    r.nodes[id].shot = await shot('10b-canvas-node-' + id);
    partial.nodes = r.nodes;
  }
  // Wheel inside f1, then click, then double click.
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, 0); return true;`); await sleep(1000);
  const f1 = await nodeInfo('f1');
  const cvState = () => ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); const v = P.findViewers(n, 7, 2000)[0]; return { x: cv.x, y: cv.y, zoom: cv.zoom, inner: v ? v.viewer.container.scrollTop : null, page: v ? v.viewer.currentPageNumber : null, selection: Array.from(cv.selection).map((o) => o.id), nodeCls: P.cls(n.nodeEl), isEditing: n.isEditing, focus: P.ui().focus, leaves: P.leaves().map((l) => l.type + ':' + (l.file || '')) };`);
  r.wheel = { before: await cvState() };
  await wheel(f1.rect.x + f1.rect.w / 2, f1.rect.y + f1.rect.h / 2, 300); await sleep(800);
  r.wheel.after = await cvState();
  await wheel(f1.rect.x + f1.rect.w / 2, f1.rect.y + 10, 300); await sleep(800);
  r.wheel.afterOnToolbar = await cvState();
  await c.clickAt(f1.rect.x + f1.rect.w / 2, f1.rect.y + f1.rect.h / 2); await sleep(700);
  r.afterClick = await cvState();
  await c.clickAt(f1.rect.x + f1.rect.w / 2, f1.rect.y + f1.rect.h / 2, { clickCount: 2 }); await sleep(1500);
  r.afterDbl = { ...(await cvState()), pdf: await ev(`${P} const s = P.pdfState(); return s && { state: s.state, viewState: s.viewState, page: s.viewer && s.viewer.currentPageNumber };`), ui: await ev(`${P} return P.ui();`) };
  r.afterDbl.shot = await shot('10b-canvas-after-dblclick');
  // Double-click f2 (#page=2) and the text node t1.
  for (const id of ['f2', 't1']) {
    await ev(`${P} app.workspace.detachLeavesOfType('pdf'); const leaf = app.workspace.getLeavesOfType('canvas')[0]; app.workspace.setActiveLeaf(leaf, { focus: true }); const cv = leaf.view.canvas; cv.deselectAll && cv.deselectAll(); const n = cv.nodes.get(${JSON.stringify(id)}); cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, 0); return true;`); await sleep(1200);
    const rr = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; return P.rect(leaf.view.canvas.nodes.get(${JSON.stringify(id)}).nodeEl);`);
    await c.clickAt(rr.x + rr.w / 2, rr.y + rr.h / 2, { clickCount: 2 }); await sleep(1500);
    r['afterDbl_' + id] = { leaves: await ev(`${P} return P.leaves();`), pdf: await ev(`${P} const s = P.pdfState(); return s && { state: s.state, page: s.viewer && s.viewer.currentPageNumber };`), node: await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const n = leaf.view.canvas.nodes.get(${JSON.stringify(id)}); return { isEditing: n.isEditing, cls: P.cls(n.nodeEl), focus: P.ui().focus };`) };
  }
  r.canvasFileAfter = fs.readFileSync(path.join(VAULT, 'Board.canvas'), 'utf8');
  await ev(`${P} app.workspace.detachLeavesOfType('pdf'); return true;`);
  await stripHistory();
  return r;
});

const sentenceRect = () => ev(`${P} const content = P.viewContent(); const p1 = content.querySelector('.page[data-page-number="1"]'); const spans = Array.from(p1.querySelectorAll('.textLayer span')).filter((s) => /This is page one/.test(s.textContent) && s.offsetParent);
  const s = spans[spans.length - 1]; if (!s) return null; s.scrollIntoView({ block: 'center' }); await P.sleep(300); const r = P.rect(s); const b = s.getBoundingClientRect(); return { text: s.textContent, rect: r, cls: P.cls(s), hit: document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2) && P.cls(document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2)), pageRect: P.rect(p1), spanCount: p1.querySelectorAll('.textLayer span').length, spans: Array.from(p1.querySelectorAll('.textLayer span')).slice(0, 6).map((x) => ({ text: x.textContent, style: x.getAttribute('style'), role: x.getAttribute('role'), cls: P.cls(x) })) };`);

await step('s11b-menus', async () => {
  await stripHistory(); await fresh();
  const r = {};
  const sp = await sentenceRect(); if (!sp) throw new Error('sentence not found');
  r.sentence = sp;
  const blank = { x: sp.rect.x + sp.rect.w / 2, y: sp.rect.y + sp.rect.h + 260 };
  r.blankHit = await ev(`${P} const e = document.elementFromPoint(${blank.x}, ${blank.y}); return e && (e.tagName.toLowerCase() + '.' + P.cls(e));`);
  await ev(`window.getSelection().removeAllRanges(); return true;`);
  await c.clickAt(blank.x, blank.y, { button: 'right' }); await sleep(700);
  r.page = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`), ui: await ev(`${P} return P.ui();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  // On the sentence itself without a selection.
  await c.clickAt(sp.rect.x + sp.rect.w / 2, sp.rect.y + sp.rect.h / 2, { button: 'right' }); await sleep(700);
  r.onTextNoSelection = { items: await c.openMenuItems(), selection: await ev(`return window.getSelection().toString();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  const tabRect = await c.rect('.mod-root .workspace-tab-header.is-active');
  await c.clickAt(tabRect.cx, tabRect.cy, { button: 'right' }); await sleep(700);
  r.tab = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  await c.clickElement('.workspace-leaf.mod-active .view-actions .clickable-icon[aria-label="More options"]'); await sleep(700);
  r.moreMenu = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`) };
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  const tb = await c.rect('.workspace-leaf.mod-active .pdf-toolbar');
  await c.clickAt(tb.x + tb.w - 30, tb.cy, { button: 'right' }); await sleep(600);
  r.toolbar = { items: await c.openMenuItems() }; await ev(`${P} P.dismiss(); return true;`);
  const gapY = await ev(`${P} const c = P.viewContent(); const p1 = c.querySelector('.page[data-page-number="1"]'); p1.scrollIntoView({ block: 'end' }); await P.sleep(300); const p2 = c.querySelector('.page[data-page-number="2"]'); const a = p1.getBoundingClientRect(), b = p2.getBoundingClientRect(); return { y: (a.bottom + b.top) / 2, x: a.x + a.width / 2, hit: document.elementFromPoint(a.x + a.width / 2, (a.bottom + b.top) / 2) && P.cls(document.elementFromPoint(a.x + a.width / 2, (a.bottom + b.top) / 2)) };`);
  if (gapY && gapY.y > 0 && gapY.y < out.version.win[1]) { await c.clickAt(gapY.x, gapY.y, { button: 'right' }); await sleep(600); r.gap = { ...gapY, items: await c.openMenuItems() }; await ev(`${P} P.dismiss(); return true;`); }
  // Beside the page (the dark margin inside the viewer container).
  const side = await ev(`${P} const c = P.viewContent(); const vc = c.querySelector('.pdf-viewer-container'); const b = vc.getBoundingClientRect(); return { x: b.x + 6, y: b.y + b.height / 2, hit: document.elementFromPoint(b.x + 6, b.y + b.height / 2) && P.cls(document.elementFromPoint(b.x + 6, b.y + b.height / 2)) };`);
  await c.clickAt(side.x, side.y, { button: 'right' }); await sleep(600); r.margin = { ...side, items: await c.openMenuItems() }; await ev(`${P} P.dismiss(); return true;`);
  return r;
});

await step('s12b-selection', async (partial) => {
  await stripHistory(); await fresh();
  const r = {};
  const sp = await sentenceRect(); if (!sp) throw new Error('sentence not found');
  r.sentence = sp;
  const select = async () => { await ev(`window.getSelection().removeAllRanges(); return true;`); await c.drag(sp.rect.x + 2, sp.rect.y + sp.rect.h / 2, sp.rect.x + sp.rect.w - 2, sp.rect.y + sp.rect.h / 2, { steps: 10, pauseMs: 30 }); await sleep(400); return ev(`return window.getSelection().toString();`); };
  r.selected = await select();
  r.selectionStyle = await ev(`${P} const c = P.viewContent(); const sel = window.getSelection(); const node = sel.anchorNode && sel.anchorNode.parentElement; const tl = c.querySelector('.page[data-page-number="1"] .textLayer'); return { anchorCls: node && P.cls(node), rangeCount: sel.rangeCount, selectionBg: getComputedStyle(tl, '::selection').backgroundColor, selectionColor: getComputedStyle(tl, '::selection').color, textLayer: P.style(tl, ['opacity', 'mix-blend-mode', 'display', 'position', 'color', 'width', 'height']), span: node && P.style(node, ['color', 'font-size', 'font-family', 'transform', 'position', 'white-space', 'opacity']), ui: P.ui() };`);
  r.shot = await shot('12b-selection');
  await c.clickAt(sp.rect.x + sp.rect.w / 2, sp.rect.y + sp.rect.h / 2, { button: 'right' }); await sleep(700);
  r.menu = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`), selectionAfterRightClick: await ev(`return window.getSelection().toString();`) };
  r.shotMenu = await shot('12b-selection-menu');
  await ev(`${P} P.dismiss(); return true;`); await sleep(200);
  r.copies = [];
  const rows = (r.menu.items[0] || []).filter((t) => t !== '---');
  for (const title of rows) {
    const sel = await select();
    await c.writeClipboard('PROBE-SENTINEL-' + Date.now());
    await c.clickAt(sp.rect.x + sp.rect.w / 2, sp.rect.y + sp.rect.h / 2, { button: 'right' }); await sleep(600);
    let rr = null; try { rr = await clickMenuRow(title, 900); } catch (e) { r.copies.push({ title, error: e.message }); await ev(`${P} P.dismiss(); return true;`); continue; }
    const clip = await c.readClipboard();
    const ui = await ev(`${P} return { ...P.ui(), leaves: P.leaves().map((l) => l.type + ':' + (l.file || '')), url: location.href.slice(0, 60) };`);
    r.copies.push({ title, selectionBefore: sel, clipboard: /^PROBE-SENTINEL-/.test(clip) ? '(no clipboard write)' : clip, clipboardJson: JSON.stringify(clip), ui });
    if (ui.modals.length) { r.copies[r.copies.length - 1].modal = await ev(`${P} const m = document.querySelector('.modal-container'); return m && { text: m.textContent.trim().slice(0, 300), dump: P.dump(m, 6, 12) };`); await c.pressKey('Escape'); await sleep(300); }
    if (ui.leaves.some((l) => l.startsWith('markdown'))) { await ev(`${P} app.workspace.detachLeavesOfType('markdown'); app.workspace.setActiveLeaf(P.pdfLeaf(), { focus: true }); return true;`); await sleep(500); }
    await ev(`${P} P.dismiss(); return true;`); await sleep(200);
    partial.copies = r.copies;
  }
  await select(); await c.writeClipboard('PROBE-SENTINEL-' + Date.now());
  await c.pressKey('c', { ctrl: true }); await sleep(600);
  const ctrlC = await c.readClipboard();
  r.ctrlC = /^PROBE-SENTINEL-/.test(ctrlC) ? '(no clipboard write)' : ctrlC;
  // A multi-line selection: from the sentence down two lines.
  await ev(`window.getSelection().removeAllRanges(); return true;`);
  await c.drag(sp.rect.x + 2, sp.rect.y + sp.rect.h / 2, sp.rect.x + 200, sp.rect.y + sp.rect.h + 60, { steps: 10, pauseMs: 30 }); await sleep(400);
  r.multiLine = await ev(`return window.getSelection().toString();`);
  await c.writeClipboard('PROBE-SENTINEL-' + Date.now()); await c.pressKey('c', { ctrl: true }); await sleep(500);
  const mc = await c.readClipboard(); r.multiLineCtrlC = /^PROBE-SENTINEL-/.test(mc) ? '(no clipboard write)' : mc;
  await ev(`window.getSelection().removeAllRanges(); return true;`);
  return r;
});

await step('s14b-small-broken', async (partial) => {
  await stripHistory();
  const res = {};
  for (const f of ['small.pdf', 'broken.pdf']) {
    await fresh(f, 3000);
    res[f] = await ev(`${P} const leaf = P.pdfLeaf(); const s = P.leafState(leaf); const content = P.viewContent(leaf); const header = leaf.view.containerEl.querySelector('.view-header'); const o = ${owner};
      return { ...s, tabTitle: document.querySelector('.mod-root .workspace-tab-header.is-active') && document.querySelector('.mod-root .workspace-tab-header.is-active').getAttribute('aria-label'), headerTitle: header.querySelector('.view-header-title') && header.querySelector('.view-header-title').textContent, contentText: content.textContent.trim().slice(0, 400), ui: P.ui(), pages: content.querySelectorAll('.page').length, pageInput: content.querySelector('.pdf-page-input') && content.querySelector('.pdf-page-input').value, pageLabel: content.querySelector('.pdf-page-numbers') && content.querySelector('.pdf-page-numbers').textContent, toolbarControls: Array.from(content.querySelectorAll('.pdf-toolbar .clickable-icon')).map((e) => ({ aria: e.getAttribute('aria-label'), disabled: e.classList.contains('is-disabled') || e.hasAttribute('disabled') })), loadingEl: leaf.view.viewer && leaf.view.viewer.child && leaf.view.viewer.child.loadingEl ? { cls: P.cls(leaf.view.viewer.child.loadingEl), text: leaf.view.viewer.child.loadingEl.textContent, display: getComputedStyle(leaf.view.viewer.child.loadingEl).display, inDom: document.contains(leaf.view.viewer.child.loadingEl) } : null, pdfDocument: !!(o && o.pdfDocument), viewerContainerChildren: Array.from(content.querySelector('.pdf-viewer-container').children).map((e) => P.cls(e)), contentContainerChildren: Array.from(content.querySelector('.pdf-content-container').children).map((e) => P.cls(e) + (e.offsetParent ? '' : '[hidden]')), errorEls: Array.from(content.querySelectorAll('[class*=error], [class*=empty], [class*=message], [class*=loading]')).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 200), display: getComputedStyle(e).display })), consoleHint: null };`);
    res[f].shot = await shot('14b-' + f.replace('.', '-'));
    partial[f] = res[f];
  }
  // Wait longer for broken.pdf: does anything appear later?
  await sleep(3000);
  res['broken.pdf'].later = await ev(`${P} const content = P.viewContent(); return { text: content.textContent.trim().slice(0, 200), notices: P.ui().notices, children: Array.from(content.querySelector('.pdf-content-container').children).map((e) => P.cls(e) + (e.offsetParent ? '' : '[hidden]')), errorEls: Array.from(content.querySelectorAll('[class*=error], [class*=empty], [class*=message]')).map((e) => ({ cls: P.cls(e), text: e.textContent.trim().slice(0, 200) })) };`);
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  res.workspaceJsonPdfLeaves = wsLeaves(wsJson(), 'pdf');
  return res;
});

await step('s16b-storage', async () => {
  // localStorage is shared across every vault: only the probe's own entries are recorded.
  return ev(`${P} const fps = window.__fps || []; let h = null; try { h = JSON.parse(localStorage.getItem('pdfjs.history') || '{"files":[]}'); } catch (e) {}
    return { keysWithPdf: Object.keys(localStorage).filter((k) => /pdf/i.test(k)), sidebarView: localStorage.getItem('pdfjs.sidebarView'), historyShape: h ? { topLevelKeys: Object.keys(h), files: (h.files || []).length, entryKeys: h.files && h.files[0] ? Object.keys(h.files[0]) : null } : null, ourEntries: (h && h.files || []).filter((f) => fps.includes(f.fingerprint)), fingerprints: fps, vaultConfigKeysWithPdf: Object.keys(app.vault.config || {}).filter((k) => /pdf/i.test(k)), appJson: Object.keys(app.vault.config || {}) };`);
});

await step('s17b-quit', async () => {
  await stripHistory();
  await ev(`${P} app.workspace.detachLeavesOfType('markdown'); app.workspace.detachLeavesOfType('canvas'); return true;`);
  await fresh();
  await ev(`${P} const ml = app.workspace.getLeaf('tab'); await ml.openFile(await P.file('Embeds.md')); app.workspace.setActiveLeaf(P.pdfLeaf(), { focus: true }); return true;`); await sleep(1200);
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentScaleValue = '1.5'; await P.sleep(500); v.currentPageNumber = 3; v.container.scrollTop += 120; return true;`); await sleep(1500);
  const before = await ev(`${P} const s = P.pdfState(); return { state: s.state, viewState: s.viewState, page: s.viewer.currentPageNumber, scaleValue: s.viewer.currentScaleValue, scale: s.viewer.currentScale, scrollTop: s.viewer.scrollTop, rotation: s.viewer.pagesRotation, location: s.viewer.location, leaves: P.leaves() };`);
  const history = await historyEntry();
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  const ws = wsLeaves(wsJson(), 'pdf');
  await c.quit();
  return { before, historyBeforeQuit: history, workspaceJsonPdfLeavesBeforeQuit: ws, workspaceJsonPdfLeavesAfterQuit: wsLeaves(wsJson(), 'pdf'), workspaceActive: wsJson().active };
});


await step('s03d-find-settings', async () => {
  await stripHistory(); await fresh();
  await ev(`${P} app.workspace.setActiveLeaf(P.pdfLeaf(), { focus: true }); return true;`); await sleep(200);
  await c.pressKey('f', { ctrl: true }); await sleep(800);
  const sig = await ev(`${P} window.__bodySig = P.sig(document.body); return window.__bodySig.length;`);
  const btn = await ev(`${P} const e = P.viewContent().querySelector('.pdf-findbar-settings-btn'); return e ? P.rect(e) : null;`);
  if (!btn) throw new Error('no settings button');
  await c.clickAt(btn.x + btn.w / 2, btn.y + btn.h / 2); await sleep(800);
  const r = await ev(`${P} const delta = P.delta(window.__bodySig, P.sig(document.body)); const menus = P.menus(); const pops = Array.from(document.querySelectorAll('.popover, .hover-popover, .suggestion-container, .pdf-findbar-settings, [class*=settings]')).filter((e) => e.offsetParent && !e.closest('.workspace-leaf')).map((e) => ({ cls: P.cls(e), rect: P.rect(e), dump: P.dump(e, 8, 20), text: e.textContent.trim().slice(0, 300) }));
    const fb = P.viewContent().querySelector('.pdf-findbar'); return { delta, menus, pops, ui: P.ui(), findbarDump: P.dump(fb, 8, 30), btnCls: P.cls(P.viewContent().querySelector('.pdf-findbar-settings-btn')), storage: localStorage.getItem(app.appId + '-pdfjs-search-settings') };`);
  r.shot = await shot('03d-find-settings');
  // Toggle every checkbox/toggle inside whatever opened, reading the storage key after each.
  r.panel = await ev(`${P} const p = P.viewContent().querySelector('.pdf-findbar-settings'); return p ? { cls: P.cls(p), rect: P.rect(p), style: P.style(p), dump: P.dump(p, 8, 20), styleTree: P.styleTree(p, 4, 40), rows: Array.from(p.querySelectorAll('.setting-item')).map((e) => ({ name: (e.querySelector('.setting-item-name') || {}).textContent, desc: (e.querySelector('.setting-item-description') || {}).textContent, checked: !!(e.querySelector('.checkbox-container') && e.querySelector('.checkbox-container').classList.contains('is-enabled')), toggleRect: P.rect(e.querySelector('.checkbox-container')), toggleCls: P.cls(e.querySelector('.checkbox-container')) })) } : null;`);
  r.afterToggle = [];
  const inputRect = await ev(`${P} return P.rect(P.viewContent().querySelector('.pdf-findbar input'));`);
  for (const row of (r.panel ? r.panel.rows : [])) {
    if (!row.toggleRect || !row.toggleRect.w) continue;
    await c.clickAt(row.toggleRect.x + row.toggleRect.w / 2, row.toggleRect.y + row.toggleRect.h / 2); await sleep(500);
    const st = await ev(`${P} const p = P.viewContent().querySelector('.pdf-findbar-settings'); return { storage: localStorage.getItem(app.appId + '-pdfjs-search-settings'), panelOpen: !!(p && p.offsetParent), rows: p ? Array.from(p.querySelectorAll('.setting-item')).map((e) => ({ name: (e.querySelector('.setting-item-name') || {}).textContent, checked: !!(e.querySelector('.checkbox-container') && e.querySelector('.checkbox-container').classList.contains('is-enabled')) })) : null, count: (P.viewContent().querySelector('.pdf-find-results-count') || {}).textContent };`);
    // With the toggle on, search "THREE" to see what it changes.
    await c.clickAt(inputRect.x + inputRect.w / 2, inputRect.y + inputRect.h / 2); await c.pressKey('a', { ctrl: true }); await c.typeText('THREE'); await sleep(900);
    const found = await ev(`${P} return { count: (P.viewContent().querySelector('.pdf-find-results-count') || {}).textContent, inputBorder: getComputedStyle(P.viewContent().querySelector('.pdf-findbar input')).borderColor, decoratorCls: P.cls(P.viewContent().querySelector('.pdf-findbar .input-right-decorator')) };`);
    await c.pressKey('a', { ctrl: true }); await c.pressKey('Backspace'); await sleep(300);
    // Turn it back off.
    await c.clickAt(row.toggleRect.x + row.toggleRect.w / 2, row.toggleRect.y + row.toggleRect.h / 2); await sleep(400);
    r.afterToggle.push({ row: row.name, ...st, searchTHREE: found, storageAfterOff: await ev(`return localStorage.getItem(app.appId + '-pdfjs-search-settings');`) });
  }
  // Match case as a toggle, and the search with it on.
  const deco = await ev(`${P} return P.rect(P.viewContent().querySelector('.pdf-findbar .input-right-decorator'));`);
  await c.clickAt(inputRect.x + inputRect.w / 2, inputRect.y + inputRect.h / 2); await c.pressKey('a', { ctrl: true }); await c.typeText('THREE'); await sleep(800);
  r.matchCase = { beforeOn: await ev(`${P} return { count: (P.viewContent().querySelector('.pdf-find-results-count') || {}).textContent, decoratorCls: P.cls(P.viewContent().querySelector('.pdf-findbar .input-right-decorator')), storage: localStorage.getItem(app.appId + '-pdfjs-search-settings') };`) };
  await c.clickAt(deco.x + deco.w / 2, deco.y + deco.h / 2); await sleep(900);
  r.matchCase.afterOn = await ev(`${P} return { count: (P.viewContent().querySelector('.pdf-find-results-count') || {}).textContent, decoratorCls: P.cls(P.viewContent().querySelector('.pdf-findbar .input-right-decorator')), inputValue: P.viewContent().querySelector('.pdf-findbar input').value, inputBorder: getComputedStyle(P.viewContent().querySelector('.pdf-findbar input')).borderColor, storage: localStorage.getItem(app.appId + '-pdfjs-search-settings'), panelRows: Array.from(P.viewContent().querySelectorAll('.pdf-findbar-settings .setting-item')).map((e) => ({ name: (e.querySelector('.setting-item-name') || {}).textContent, checked: !!(e.querySelector('.checkbox-container') && e.querySelector('.checkbox-container').classList.contains('is-enabled')) })) };`);
  await c.clickAt(deco.x + deco.w / 2, deco.y + deco.h / 2); await sleep(400);
  await c.pressKey('Escape'); await sleep(300);
  // Put the settings back to what they were (click again whatever is now checked and was not).
  r.storageEnd = await ev(`return localStorage.getItem(app.appId + '-pdfjs-search-settings');`);
  return r;
});

await step('s08c-embed-page3', async () => {
  await stripHistory();
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('pdf'); return true;`); await sleep(3500);
  const e = EMBED_LINES[1];
  const first = await embedInfoAt('preview', e.line, 'doc.pdf#page=3');
  await sleep(3000);
  const again = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const el = Array.from(leaf.view.containerEl.querySelectorAll('.internal-embed')).find((x) => x.offsetParent && x.getAttribute('src') === 'doc.pdf#page=3'); const v = P.findViewers(leaf.view, 9, 4000).find((x) => x.viewer.container && el.contains(x.viewer.container)); const vw = v && v.viewer; const off = (n) => { const p = vw.container.querySelector('.page[data-page-number="' + n + '"]'); return p ? p.offsetTop : null; };
    return { page: vw && vw.currentPageNumber, scrollTop: vw && vw.container.scrollTop, scale: vw && vw.currentScale, scaleValue: vw && vw.currentScaleValue, offsets: vw ? [1, 2, 3, 4, 5].map(off) : null, heights: vw ? [1, 2, 3, 4, 5].map((n) => vw.container.querySelector('.page[data-page-number="' + n + '"]').offsetHeight) : null, location: vw && vw._location && { pageNumber: vw._location.pageNumber, top: vw._location.top }, input: el.querySelector('.pdf-page-input').value, clientHeight: vw && vw.container.clientHeight };`);
  // The same embed with a portrait target page for comparison: the md alt page 2 one.
  const p2 = await embedInfoAt('preview', EMBED_LINES[10].line, 'doc.pdf#page=2');
  const p2again = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const el = Array.from(leaf.view.containerEl.querySelectorAll('.internal-embed')).find((x) => x.offsetParent && x.getAttribute('src') === 'doc.pdf#page=2'); const v = P.findViewers(leaf.view, 9, 4000).find((x) => x.viewer.container && el.contains(x.viewer.container)); const vw = v && v.viewer; const off = (n) => { const p = vw.container.querySelector('.page[data-page-number="' + n + '"]'); return p ? p.offsetTop : null; }; return { page: vw && vw.currentPageNumber, scrollTop: vw && vw.container.scrollTop, offsets: vw ? [1, 2, 3, 4, 5].map(off) : null, input: el.querySelector('.pdf-page-input').value };`);
  // Live Preview: the embed's parent chain and the active line when the caret sits on the embed's line.
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; await leaf.setViewState({ type: 'markdown', state: { file: 'Embeds.md', mode: 'source', source: false } }); await P.sleep(800); leaf.view.editor.setCursor({ line: 0, ch: 0 }); return true;`); await sleep(1500);
  const lp = await embedInfoAt('live', EMBED_LINES[0].line, 'doc.pdf');
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.editor.setCursor({ line: ${EMBED_LINES[0].line}, ch: 3 }); return true;`); await sleep(900);
  const caret = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const sc = leaf.view.containerEl.querySelector('.cm-scroller'); const line = sc.querySelector('.cm-line.cm-active, .cm-activeLine'); const emb = Array.from(sc.querySelectorAll('.internal-embed')).filter((e) => e.offsetParent); return { activeLineText: line && line.textContent, activeLineCls: line && P.cls(line), embedsVisible: emb.length, firstEmbedParent: emb[0] && P.cls(emb[0].parentElement), lineHasEmbedWidget: !!(line && line.querySelector('.internal-embed')), prevSibling: line && line.previousElementSibling && P.cls(line.previousElementSibling), nextSibling: line && line.nextElementSibling && P.cls(line.nextElementSibling) };`);
  return { page3: { first: { page: first.viewer && first.viewer.currentPageNumber, scrollTop: first.viewer && first.viewer.scrollTop, inline: first.inlineStyle }, again }, page2: { first: { page: p2.viewer && p2.viewer.currentPageNumber, scrollTop: p2.viewer && p2.viewer.scrollTop }, again: p2again }, livePreview: { parentChain: lp.parentChain, wrapper: lp.wrapper, cls: lp.cls, attrs: lp.attrs, rect: lp.rect }, caret };
});

await step('s10c-canvas-toolbar', async () => {
  await stripHistory();
  await ev(`${P} const f = await P.file('Board.canvas'); const leaf = app.workspace.getLeaf('tab'); await leaf.openFile(f); app.workspace.setActiveLeaf(leaf, { focus: true }); for (const t of ['pdf', 'markdown']) app.workspace.detachLeavesOfType(t); for (const l of app.workspace.getLeavesOfType('canvas')) if (l !== leaf) l.detach(); return true;`); await sleep(2500);
  const r = { fileBefore: fs.readFileSync(path.join(VAULT, 'Board.canvas'), 'utf8') };
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, 0); return true;`); await sleep(1500);
  const ctl = (aria) => ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const n = leaf.view.canvas.nodes.get('f1'); const e = n.nodeEl.querySelector('.pdf-toolbar [aria-label=${JSON.stringify(aria)}]'); return e ? { rect: P.rect(e), icon: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), cls: P.cls(e), hidden: !e.offsetParent } : null;`);
  r.toolbar = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const n = leaf.view.canvas.nodes.get('f1'); const tb = n.nodeEl.querySelector('.pdf-toolbar'); return { dump: P.dump(tb, 6, 30), right: Array.from(tb.querySelectorAll('.pdf-toolbar-right > *')).map((e) => ({ cls: P.cls(e), aria: e.getAttribute('aria-label'), icon: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), rect: P.rect(e), hidden: !e.offsetParent })) };`);
  // Scroll the node's viewer to page 3, then "Save current position in document".
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const n = leaf.view.canvas.nodes.get('f1'); const v = P.findViewers(n, 7, 2500)[0]; v.viewer.currentPageNumber = 3; await P.sleep(400); v.viewer.container.scrollTop += 30; await P.sleep(400); v.viewer.container.scrollTop -= 30; return true;`); await sleep(1200);
  r.nodeStateBeforeSave = await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const n = leaf.view.canvas.nodes.get('f1'); const v = P.findViewers(n, 7, 2500)[0]; return { page: v.viewer.currentPageNumber, scrollTop: v.viewer.container.scrollTop, location: v.viewer._location && { pageNumber: v.viewer._location.pageNumber, top: v.viewer._location.top, pdfOpenParams: v.viewer._location.pdfOpenParams }, subpath: n.child && n.child.subpath, data: n.getData() };`);
  const save = await ctl('Save current position in document');
  r.saveControl = save;
  if (save && !save.hidden) {
    await c.clickAt(save.rect.x + save.rect.w / 2, save.rect.y + save.rect.h / 2); await sleep(1000);
    r.saveMenu = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`), ui: await ev(`${P} return P.ui();`) };
    await ev(`${P} P.dismiss(); return true;`); await sleep(4000);
    r.afterSave = { ui: await ev(`${P} return P.ui();`), node: await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const n = leaf.view.canvas.nodes.get('f1'); return { data: n.getData(), subpath: n.child && n.child.subpath, label: n.nodeEl.querySelector('.canvas-node-label') && n.nodeEl.querySelector('.canvas-node-label').textContent }; `), file: fs.readFileSync(path.join(VAULT, 'Board.canvas'), 'utf8') };
    r.afterSave.shot = await shot('10c-canvas-saved-position');
  }
  // "Open" in the node's toolbar.
  const open = await ctl('Open');
  r.openControl = open;
  if (open && !open.hidden) {
    const before = await ev(`${P} return P.leaves();`);
    await c.clickAt(open.rect.x + open.rect.w / 2, open.rect.y + open.rect.h / 2); await sleep(900);
    r.openMenu = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`), ui: await ev(`${P} return P.ui();`) };
    // Click each row of that menu in turn, re-opening it between rows.
    r.openRows = [];
    for (const title of (r.openMenu.items[0] || []).filter((t) => t !== '---')) {
      try { await clickMenuRow(title, 1800); } catch (e) { r.openRows.push({ title, error: e.message }); continue; }
      r.openRows.push({ title, leaves: await ev(`${P} return P.leaves();`), active: await ev(`return app.workspace.activeLeaf.view.getViewType();`), pdf: await ev(`${P} const s = P.pdfState(); return s && { state: s.state, page: s.viewer && s.viewer.currentPageNumber, scrollTop: s.viewer && s.viewer.scrollTop, location: s.viewer && s.viewer.location }; `), ui: await ev(`${P} return P.ui();`), windows: await ev(`return window.require('electron').ipcRenderer ? null : null;`) });
      await ev(`${P} P.dismiss(); app.workspace.detachLeavesOfType('pdf'); for (const l of app.workspace.getLeavesOfType('canvas').slice(1)) l.detach(); app.workspace.setActiveLeaf(app.workspace.getLeavesOfType('canvas')[0], { focus: true }); return true;`); await sleep(800);
      await ev(`${P} const leaf = app.workspace.getLeavesOfType('canvas')[0]; const cv = leaf.view.canvas; const n = cv.nodes.get('f1'); cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, 0); return true;`); await sleep(1000);
      const again = await ctl('Open'); if (!again) break;
      await c.clickAt(again.rect.x + again.rect.w / 2, again.rect.y + again.rect.h / 2); await sleep(800);
    }
    await ev(`${P} P.dismiss(); return true;`);
    r.afterOpen = { leavesBefore: before.length, leaves: await ev(`${P} return P.leaves();`) };
    await ev(`${P} app.workspace.detachLeavesOfType('pdf'); app.workspace.setActiveLeaf(app.workspace.getLeavesOfType('canvas')[0], { focus: true }); return true;`); await sleep(500);
  }
  // "Open" inside a note embed, for comparison.
  await ev(`${P} await P.openMd('Embeds.md', 'preview'); app.workspace.detachLeavesOfType('canvas'); return true;`); await sleep(2500);
  await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.previewMode.applyScroll(${EMBED_LINES[1].line - 1}); return true;`); await sleep(1200);
  const eo = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const el = Array.from(leaf.view.containerEl.querySelectorAll('.internal-embed')).find((x) => x.offsetParent && x.getAttribute('src') === 'doc.pdf#page=3'); const e = el && el.querySelector('.pdf-toolbar [aria-label="Open"]'); return e ? { rect: P.rect(e), icon: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), right: Array.from(el.querySelectorAll('.pdf-toolbar-right > *')).map((x) => ({ cls: P.cls(x), aria: x.getAttribute('aria-label'), hidden: !x.offsetParent })), embedPage: (P.findViewers(leaf.view, 9, 3000).find((v) => el.contains(v.viewer.container)) || {}).viewer && P.findViewers(leaf.view, 9, 3000).find((v) => el.contains(v.viewer.container)).viewer.currentPageNumber } : null;`);
  r.embedOpenControl = eo;
  if (eo) {
    const before = await ev(`${P} return P.leaves();`);
    await c.clickAt(eo.rect.x + eo.rect.w / 2, eo.rect.y + eo.rect.h / 2); await sleep(900);
    r.embedOpenMenu = { items: await c.openMenuItems(), menus: await ev(`${P} return P.menus();`) };
    const first = (r.embedOpenMenu.items[0] || []).find((t) => t !== '---');
    if (first) { await clickMenuRow(first, 1800); }
    r.embedAfterOpen = { clicked: first, leavesBefore: before.length, leaves: await ev(`${P} return P.leaves();`), pdf: await ev(`${P} const s = P.pdfState(); return s && { state: s.state, page: s.viewer && s.viewer.currentPageNumber, scrollTop: s.viewer && s.viewer.scrollTop, scaleValue: s.viewer && s.viewer.currentScaleValue, location: s.viewer && s.viewer.location }; `), active: await ev(`return app.workspace.activeLeaf.view.getViewType();`) };
    await ev(`${P} P.dismiss(); return true;`);
    // Ctrl+click on Open.
    await ev(`${P} app.workspace.detachLeavesOfType('pdf'); app.workspace.setActiveLeaf(app.workspace.getLeavesOfType('markdown')[0], { focus: true }); return true;`); await sleep(500);
    await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; leaf.view.previewMode.applyScroll(${EMBED_LINES[1].line - 1}); return true;`); await sleep(1000);
    const eo2 = await ev(`${P} const leaf = app.workspace.getLeavesOfType('markdown')[0]; const el = Array.from(leaf.view.containerEl.querySelectorAll('.internal-embed')).find((x) => x.offsetParent && x.getAttribute('src') === 'doc.pdf#page=3'); const e = el && el.querySelector('.pdf-toolbar [aria-label="Open"]'); return e ? P.rect(e) : null;`);
    if (eo2) { await c.clickAt(eo2.x + eo2.w / 2, eo2.y + eo2.h / 2, { mods: { ctrl: true } }); await sleep(1800); r.embedCtrlOpen = { leaves: await ev(`${P} return P.leaves();`), active: await ev(`return app.workspace.activeLeaf.view.getViewType();`) }; }
  }
  await ev(`${P} app.workspace.detachLeavesOfType('pdf'); return true;`);
  await stripHistory();
  return r;
});

await step('s17-quit', async () => {
  // Leave the pdf leaf active at page 3, 150%, with Embeds.md in another tab, then quit.
  await fresh();
  await ev(`${P} app.workspace.detachLeavesOfType('markdown'); app.workspace.detachLeavesOfType('canvas'); return true;`);
  await ev(`${P} const ml = app.workspace.getLeaf('tab'); await ml.openFile(await P.file('Embeds.md')); app.workspace.setActiveLeaf(P.pdfLeaf(), { focus: true }); return true;`); await sleep(1200);
  await ev(`${P} const v = P.viewerOf(P.pdfLeaf().view); v.currentScaleValue = '1.5'; await P.sleep(500); v.currentPageNumber = 3; return true;`); await sleep(1200);
  const before = await ev(`${P} return { pdf: P.pdfState(), leaves: P.leaves() };`);
  await ev(`app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  const ws = wsLeaves(wsJson(), 'pdf');
  await c.quit();
  return { before, workspaceJsonPdfLeavesBeforeQuit: ws, workspaceJsonPdfLeavesAfterQuit: wsLeaves(wsJson(), 'pdf') };
});

out.finishedAt = new Date().toISOString();
save();
try { c.close(); } catch { /* quit already closed it */ }
process.exit(0);
