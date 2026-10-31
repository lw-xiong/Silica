// Search pane block: sort options, copy results, menus, toggles, keyboard.
// Phases: fixtures | run | after-restart  (see cdp.mjs for the harness).
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep, waitForObsidian } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
const KEY = 'search';
if (!VAULT || !RESULTS) throw new Error('VAULT and RESULTS must be set');
const PHASE_KEY = { fixtures: 'fixtures', run: 'run', 'after-restart': 'after_restart' }[phase] || phase;

// ---------------------------------------------------------------------------
// results bookkeeping
const out = { phase, startedAt: new Date().toISOString(), steps: {}, order: [] };
function loadAll() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
function save() {
  const all = loadAll();
  all[KEY] = all[KEY] || {};
  all[KEY][PHASE_KEY] = out;
  fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1));
}
// run: every ordinary step must END by DEADLINE (its timeout is capped to the time left),
// then 14-quit (always run, <= 40 s) keeps the phase under 4 min of wall clock.
const DEADLINE = Date.now() + (phase === 'run' ? 185000 : 150000);
async function step(name, fn, stepMs = 30000, { always = false } = {}) {
  const t0 = Date.now();
  out.order.push(name);
  if (!always) {
    if (Date.now() > DEADLINE) { out.steps[name] = { ok: false, error: 'skipped: deadline' }; save(); return null; }
    stepMs = Math.min(stepMs, Math.max(5000, DEADLINE - Date.now()));
  }
  let timer;
  try {
    const r = await Promise.race([
      fn(),
      new Promise((_, rej) => { timer = setTimeout(() => rej(new Error('step timeout ' + stepMs + 'ms (the step body keeps running in the background)')), stepMs); }),
    ]);
    const rec = (r && typeof r === 'object' && !Array.isArray(r)) ? r : { value: r };
    out.steps[name] = { ok: true, ms: Date.now() - t0, ...rec };
    save();
    return r;
  } catch (e) {
    out.steps[name] = { ok: false, ms: Date.now() - t0, error: String((e && e.stack) || e).slice(0, 1500) };
    save();
    return null;
  } finally { clearTimeout(timer); }
}

// ---------------------------------------------------------------------------
// fixtures
const FIX = {
  'apple.md': 'apple needle one\n',
  'notes/banana.md': 'banana has a needle in a folder\n',
  'mango.md': 'line before first\nmango needle first\nline after first\n\nmiddle paragraph with no hit at all\n\nmango needle second\nmango needle third on this line\nline after third\nlast line\n',
  'zebra.md': 'zebra needle alpha\nfiller line\nzebra needle beta\n',
  'kiwi.md': 'kiwi has Needle with a capital N only\n',
  'Apple Pie.md': 'apple pie needle\n',
  'apple2.md': 'apple two needle\n',
  'apple10.md': 'apple ten needle\n',
  '_needle.md': 'underscore file needle\n',
  '9 needle.md': 'nine needle\n',
  'notes/apple.md': 'notes apple needle\n',
  'plain.txt': 'plain text needle\n',
};
// deliberate write order (creation times), not alphabetical
const WRITE_ORDER = ['zebra.md', 'apple.md', 'kiwi.md', 'mango.md', 'notes/banana.md', 'plain.txt', 'apple10.md', '_needle.md', 'Apple Pie.md', 'notes/apple.md', '9 needle.md', 'apple2.md'];
// mtimes: all old and distinct, except mango/zebra clearly newer (zebra newest)
const BASE = Date.UTC(2024, 0, 1, 0, 0, 0);
const MTIME_MIN = { 'apple.md': 50, 'notes/banana.md': 10, 'kiwi.md': 20, 'Apple Pie.md': 30, 'apple2.md': 40, 'apple10.md': 5, '_needle.md': 60, '9 needle.md': 15, 'notes/apple.md': 25, 'plain.txt': 35 };

function fixtures() {
  const written = {};
  for (const rel of WRITE_ORDER) {
    const p = path.join(VAULT, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, FIX[rel], 'utf8');
    written[rel] = { bytes: FIX[rel].length, content: FIX[rel] };
  }
  const now = Date.now();
  const mtimes = {};
  for (const rel of WRITE_ORDER) {
    let ms;
    if (rel === 'mango.md') ms = now - 120000;
    else if (rel === 'zebra.md') ms = now - 60000;
    else ms = BASE + MTIME_MIN[rel] * 60000;
    const t = new Date(ms);
    fs.utimesSync(path.join(VAULT, rel), t, t);
    mtimes[rel] = { ms, iso: t.toISOString() };
  }
  const stat = {};
  for (const rel of WRITE_ORDER) {
    const s = fs.statSync(path.join(VAULT, rel));
    stat[rel] = { mtimeMs: s.mtimeMs, ctimeMs: s.ctimeMs, birthtimeMs: s.birthtimeMs, size: s.size };
  }
  const expectedMtimeOrderNewestFirst = Object.entries(mtimes).sort((a, b) => b[1].ms - a[1].ms).map(([k]) => k);
  out.steps.fixtures = { ok: true, writeOrder: WRITE_ORDER, written, mtimesSet: mtimes, statAfter: stat, expectedMtimeOrderNewestFirst };
  let obsidianDir = null;
  try { obsidianDir = fs.readdirSync(path.join(VAULT, '.obsidian')); } catch (e) { obsidianDir = 'absent: ' + e.message; }
  out.steps.obsidianDirBefore = { ok: true, entries: obsidianDir };
  save();
}

// ---------------------------------------------------------------------------
// renderer snippets
const VIEW = `
  const leaf = app.workspace.getLeavesOfType('search')[0];
  if (leaf && leaf.isDeferred && typeof leaf.loadIfDeferred === 'function') { try { await leaf.loadIfDeferred(); } catch (e) {} }
  const v = leaf && leaf.view; if (!v) throw new Error('no search view');
  const dom = v.dom; if (!dom) throw new Error('search view has no dom (deferred? type=' + (v.getViewType ? v.getViewType() : '?') + ')');`;

// the sort control: Obsidian's DropdownComponent on the view, or any <select> / sort-labelled control in the params section
const SORT_EL = `
  const d = v.sortOrderDropdown; let sortEl = d && (d.selectEl || d.el || d.containerEl) || null; let sortHow = sortEl ? 'v.sortOrderDropdown' : null;
  if (!sortEl) { const p = v.searchParamsContainerEl || v.containerEl.querySelector('.search-params') || v.containerEl; sortEl = p.querySelector('select'); if (sortEl) sortHow = 'select in params'; }
  if (!sortEl) { sortEl = v.containerEl.querySelector('[aria-label*="sort" i]'); if (sortEl) sortHow = 'aria-label sort'; }
  if (sortEl) sortEl.setAttribute('data-cdp', 'sort-dropdown');`;

const READ_RESULTS = `${VIEW}
  const byEl = new Map();
  try { const lk = dom.resultDomLookup; if (lk && lk.forEach) lk.forEach((d, f) => { if (d && d.el && f) byEl.set(d.el, f.path || String(f)); }); } catch (e) {}
  const rows = Array.from(v.containerEl.querySelectorAll('.search-result')).map(el => {
    const title = el.querySelector('.search-result-file-title');
    const inner = title && title.querySelector('.tree-item-inner');
    const flair = title && title.querySelector('.tree-item-flair');
    const matches = Array.from(el.querySelectorAll('.search-result-file-match')).map(m => m.textContent);
    return { title: inner ? inner.textContent : (title ? title.textContent.trim() : null), flair: flair ? flair.textContent.trim() : null,
      path: byEl.get(el) || null, collapsed: el.classList.contains('is-collapsed'), rowClass: el.className, matchCount: matches.length, matches };
  });
  let vchildren = null;
  try { const vc = dom.vChildren; const arr = (vc && (vc.children || vc._children)) || null; if (arr) vchildren = Array.from(arr).map(c => (c.file && c.file.path) || null); } catch (e) {}
  const countEl = v.containerEl.querySelector('.search-results-result-count');
  const infoEl = v.containerEl.querySelector('.search-results-info');
  let state = null; try { state = v.getState(); } catch (e) { state = 'err:' + e.message; }
  return { rows, vchildren, count: countEl ? countEl.textContent.trim() : null, info: infoEl ? infoEl.textContent.trim() : null,
    sortOrder: dom.sortOrder, collapseAll: dom.collapseAll, extraContext: dom.extraContext, working: dom.working, state,
    resultLookupSize: (dom.resultDomLookup && dom.resultDomLookup.size) ?? null };`;

const MENU_DUMP = `
  return Array.from(document.querySelectorAll('.menu')).map(menu => ({ cls: menu.className,
    items: Array.from(menu.querySelectorAll('.menu-item, .menu-separator')).map(c => c.classList.contains('menu-separator') ? '---' : {
      title: ((c.querySelector('.menu-item-title') || c).textContent || '').trim(), section: c.dataset ? (c.dataset.section || null) : null,
      cls: c.className, icon: (c.querySelector('svg') && c.querySelector('svg').getAttribute('class')) || null }) }));`;

const FOCUS_STATE = `${VIEW}
  const ae = document.activeElement;
  const fi = dom.focusedItem;
  const focusEl = v.containerEl.querySelector('.has-focus, .is-focused, .is-active, [tabindex="0"].is-selected');
  let fiInfo = null;
  if (fi) {
    fiInfo = { keys: Object.keys(fi).slice(0, 30), cls: fi.el && fi.el.className, path: (fi.file && fi.file.path) || (fi.parent && fi.parent.file && fi.parent.file.path) || null,
      text: fi.el ? fi.el.textContent.trim().slice(0, 100) : null, isMatch: !!(fi.parent && !fi.file), collapsed: fi.collapsed === undefined ? null : fi.collapsed };
  }
  const rowStates = Array.from(v.containerEl.querySelectorAll('.search-result')).map(el => ({ t: (el.querySelector('.tree-item-inner') || el).textContent.trim().slice(0, 40), collapsed: el.classList.contains('is-collapsed'),
    hasFocus: !!el.querySelector('.has-focus, .is-focused') }));
  return { activeEl: ae ? { tag: ae.tagName, cls: ae.className, id: ae.id } : null, focusedItem: fiInfo, focusEl: focusEl ? { cls: focusEl.className, text: focusEl.textContent.trim().slice(0, 100) } : null, rowStates };`;

const LEAVES = `
  const md = app.workspace.getLeavesOfType('markdown').map(l => ({ id: l.id, path: l.view && l.view.file ? l.view.file.path : null, mode: l.view && l.view.getMode ? l.view.getMode() : null,
    inRoot: l.getRoot() === app.workspace.rootSplit, pinned: !!l.pinned }));
  const al = app.workspace.activeLeaf;
  const af = app.workspace.getActiveFile();
  let cursor = null; try { cursor = al && al.view && al.view.editor ? al.view.editor.getCursor() : null; } catch (e) {}
  const allLeaves = []; app.workspace.iterateAllLeaves(l => allLeaves.push({ id: l.id, type: l.view && l.view.getViewType ? l.view.getViewType() : null, inRoot: l.getRoot() === app.workspace.rootSplit }));
  return { markdownLeaves: md, activeLeaf: al ? { id: al.id, type: al.view && al.view.getViewType ? al.view.getViewType() : null, inRoot: al.getRoot() === app.workspace.rootSplit } : null,
    activeFile: af ? af.path : null, cursor, allLeafCount: allLeaves.length, allLeaves };`;

const LOCAL_STORAGE = `
  const id = app.appId; const keys = [];
  for (let i = 0; i < localStorage.length; i++) keys.push(localStorage.key(i));
  const vaultKeys = keys.filter(k => id && k.startsWith(id));
  // values only for this vault's keys; other vaults share this localStorage, so only their key names are listed
  const hits = {}; const otherVaultKeyNames = [];
  for (const k of keys) if (/sort|recent-search|search/i.test(k)) { if (id && k.startsWith(id)) hits[k] = localStorage.getItem(k); else otherVaultKeyNames.push(k); }
  return { appId: id, vaultKeys, hits, otherVaultKeyNames };`;

function listVaultFiles() {
  const outp = [];
  (function walk(dir, rel) {
    let ents = []; try { ents = fs.readdirSync(dir, { withFileTypes: true }); } catch (e) { return; }
    for (const e of ents) { if (e.name === '.obsidian') continue; const r = rel ? rel + '/' + e.name : e.name; if (e.isDirectory()) walk(path.join(dir, e.name), r); else outp.push(r); }
  })(VAULT, '');
  return outp.sort();
}

// ---------------------------------------------------------------------------
// run helpers
async function waitIdle(c, maxMs = 6000) {
  const end = Date.now() + maxMs; let last = null, stable = 0; const samples = [];
  while (Date.now() < end) {
    let s = null;
    try { s = await c.evaluate(`${VIEW} const e = v.containerEl.querySelector('.search-results-result-count'); return { working: !!dom.working, count: e ? e.textContent.trim() : null, rows: v.containerEl.querySelectorAll('.search-result').length };`); } catch (e) { s = { err: e.message }; }
    samples.push(s);
    const key = JSON.stringify(s);
    if (key === last && !s.working) { if (++stable >= 3) return { idle: true, samples: samples.slice(-6) }; } else stable = 0;
    last = key; await sleep(150);
  }
  return { idle: false, samples: samples.slice(-6) };
}

// tag an element (found by a renderer expression returning the element) so clickElement can target it
async function tag(c, exprReturningEl, name) {
  return c.evaluate(`${VIEW} const el = (() => { ${exprReturningEl} })(); if (!el) return null;
    document.querySelectorAll('[data-cdp="${name}"]').forEach(x => x.removeAttribute('data-cdp'));
    el.setAttribute('data-cdp', '${name}');
    const r = el.getBoundingClientRect();
    return { tag: el.tagName, cls: el.className, aria: el.getAttribute('aria-label'), x: r.x, y: r.y, w: r.width, h: r.height, html: el.outerHTML.slice(0, 600) };`);
}

async function sampleCount(c, ms = 900, every = 50) {
  const end = Date.now() + ms; const seq = []; let last;
  while (Date.now() < end) {
    let t = null; try { t = await c.evaluate(`${VIEW} const e = v.containerEl.querySelector('.search-results-result-count'); return (e ? e.textContent.trim() : null) + '|' + (dom.working ? 'working' : 'idle') + '|rows=' + v.containerEl.querySelectorAll('.search-result').length;`); } catch (e) { t = 'err'; }
    if (t !== last) { seq.push({ atMs: ms - (end - Date.now()), v: t }); last = t; }
    await sleep(every);
  }
  return seq;
}

async function rightClickAndDump(c, selector, opts = {}) {
  await c.pressKey('Escape'); await sleep(120);
  const before = await c.evaluate(`return document.querySelectorAll('.menu').length;`);
  const r = await c.rightClickElement(selector, opts);
  await sleep(450);
  const menus = await c.evaluate(MENU_DUMP);
  const labels = await c.openMenuItems();
  await c.pressKey('Escape'); await sleep(150);
  const after = await c.evaluate(`return document.querySelectorAll('.menu').length;`);
  return { clicked: r, menusBefore: before, menus, labels, menusAfterEscape: after };
}

function titlesOf(labels) { return (labels && labels[0]) ? labels[0] : null; }

// select a sort option; returns how it was done
const SEL = `${VIEW}${SORT_EL} const s = sortEl;`;
async function selectSortOption(c, wantedValue) {
  const info = await c.evaluate(`${SEL} if (!s) return null;
    return { tag: s.tagName, how: sortHow, value: s.value ?? null, selectedIndex: s.selectedIndex ?? null, options: s.options ? Array.from(s.options).map(o => o.value) : null };`);
  if (!info) throw new Error('no sort control element (v.sortOrderDropdown, select in params, aria-label sort all absent)');
  const res = { element: info.tag, how: info.how, wanted: wantedValue, presses: [] };
  if (info.tag === 'SELECT') {
    const target = info.options.indexOf(wantedValue);
    if (target < 0) throw new Error('option not in select: ' + wantedValue);
    res.focused = await c.evaluate(`${SEL} s.focus(); return document.activeElement === s;`);
    let idx = info.selectedIndex; let guard = 0;
    while (idx !== target && guard++ < 12) {
      await c.pressKey(idx < target ? 'ArrowDown' : 'ArrowUp'); await sleep(120);
      const now = await c.evaluate(`${SEL} return { selectedIndex: s.selectedIndex, value: s.value, domSort: dom.sortOrder };`);
      res.presses.push(now);
      if (now.selectedIndex === idx) break; // key did nothing
      idx = now.selectedIndex;
      if (idx !== target) { // an intermediate option: let it settle and zero the counters so counts reflect the final transition only
        await waitIdle(c, 2000); await c.evaluate(`window.__cdpCounts = {}; return true;`);
      }
    }
    const check = await c.evaluate(`${SEL} return { value: s.value, domSort: dom.sortOrder };`);
    if (check.value === wantedValue && check.domSort === wantedValue) { res.method = res.presses.length ? 'keyboard-on-focused-select' : 'already-selected (no key pressed)'; }
    else {
      await c.evaluate(`${SEL} s.value = ${JSON.stringify(wantedValue)}; s.dispatchEvent(new Event('change', { bubbles: true })); return true;`);
      await sleep(120);
      res.method = 'synthetic-change-event (keyboard did not switch: ' + JSON.stringify(check) + ')';
    }
    try { await c.evaluate(`${SEL} s.blur(); return true;`); } catch {}
  } else {
    // custom dropdown: click it, then the menu item whose text matches
    await c.clickElement('[data-cdp="sort-dropdown"]'); await sleep(350);
    const items = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(e => (e.querySelector('.menu-item-title') || e).textContent.trim());`);
    res.menuItems = items;
    const i = items.findIndex(t => t === wantedValue);
    if (i < 0) { await c.pressKey('Escape'); throw new Error('option not in menu: ' + wantedValue); }
    await c.clickElement('.menu .menu-item', { index: i }); res.method = 'click-menu-item';
  }
  res.after = await c.evaluate(`${SEL} return { value: s ? (s.value ?? s.textContent.trim()) : null, domSort: dom.sortOrder, state: v.getState() };`);
  return res;
}

// ---------------------------------------------------------------------------
let conn = null;
async function run() {
  await waitForObsidian();
  await sleep(2500);
  const c = await connect(); conn = c;
  out.vault = VAULT;

  await step('00-initial-disk', async () => {
    const r = {};
    for (const f of ['app.json', 'workspace.json']) { try { r[f] = fs.readFileSync(path.join(VAULT, '.obsidian', f), 'utf8'); } catch (e) { r[f] = 'absent: ' + e.message; } }
    try { r.obsidianDir = fs.readdirSync(path.join(VAULT, '.obsidian')); } catch (e) { r.obsidianDir = 'absent'; }
    return r;
  });

  await step('01-open-pane', async () => {
    const cmd = await c.evaluate(`return app.commands.executeCommandById('global-search:open');`);
    await sleep(800);
    const info = await c.evaluate(`${VIEW}
      const header = v.containerEl.querySelector('.search-input-container');
      const headerParent = header && header.parentElement;
      const icons = Array.from(v.containerEl.querySelectorAll('.clickable-icon')).map(e => ({ cls: e.className, aria: e.getAttribute('aria-label'), svg: e.querySelector('svg') && e.querySelector('svg').getAttribute('class'), text: e.textContent.trim().slice(0, 40) }));
      const navHeader = v.containerEl.querySelector('.nav-header');
      let st = null; try { st = v.getState(); } catch (e) { st = 'err:' + e.message; }
      const own = Object.keys(v);
      const domProto = Object.getOwnPropertyNames(Object.getPrototypeOf(dom)).filter(k => k !== 'constructor');
      return { cmd: null, leafId: leaf.id, inLeftSplit: leaf.getRoot() === app.workspace.leftSplit, viewType: v.getViewType(), displayText: v.getDisplayText(), initialState: st,
        containerChildren: Array.from(v.containerEl.children).map(e => e.className),
        headerParentHtml: headerParent ? headerParent.outerHTML.slice(0, 3500) : null, icons, navHeader: navHeader ? navHeader.outerHTML.slice(0, 800) : null,
        viewOwnKeys: own, domProtoMethods: domProto, hasSortDropdown: !!v.sortOrderDropdown, sortDropdownKeys: v.sortOrderDropdown ? Object.keys(v.sortOrderDropdown) : null };`);
    info.cmd = cmd;
    return info;
  });

  await step('02-type-needle', async () => {
    const r = {};
    r.clickInput = await c.clickElement('.search-input-container input');
    await sleep(150);
    r.activeBefore = await c.evaluate(`const a = document.activeElement; return a ? a.tagName + '.' + a.className : null;`);
    await c.typeText('needle');
    r.idle = await waitIdle(c);
    r.results = await c.evaluate(READ_RESULTS);
    r.firstRowHtml = await c.evaluate(`${VIEW} const e = v.containerEl.querySelector('.search-result'); return e ? e.outerHTML.slice(0, 2000) : null;`);
    r.resultContainerClasses = await c.evaluate(`${VIEW} return Array.from(v.containerEl.querySelectorAll('.search-result-container, .search-results-children, .search-results-info')).map(e => e.className);`);
    return r;
  });

  // ---- step 1: settings section --------------------------------------------
  await step('03-settings-section', async () => {
    const r = {};
    r.toggleEl = await tag(c, `return v.filterSectionToggleEl || v.containerEl.querySelector('.clickable-icon[aria-label="Search settings"]');`, 'settings-toggle');
    r.paramsBefore = await c.evaluate(`${VIEW} const p = v.searchParamsContainerEl || v.containerEl.querySelector('.search-params'); return p ? { display: getComputedStyle(p).display, cls: p.className, hidden: p.hidden } : null;`);
    r.click = await c.clickElement('[data-cdp="settings-toggle"]');
    await sleep(450);
    r.params = await c.evaluate(`${VIEW} const p = v.searchParamsContainerEl || v.containerEl.querySelector('.search-params'); if (!p) return null;
      const controls = Array.from(p.querySelectorAll('.setting-item')).map(s => ({ name: (s.querySelector('.setting-item-name') || {}).textContent || null, desc: (s.querySelector('.setting-item-description') || {}).textContent || null,
        cls: s.className, control: Array.from(s.querySelectorAll('.setting-item-control > *')).map(e => ({ tag: e.tagName, cls: e.className, aria: e.getAttribute('aria-label'), enabled: e.classList.contains('is-enabled'), value: e.value ?? null,
          options: e.options ? Array.from(e.options).map(o => ({ value: o.value, text: o.textContent, selected: o.selected })) : null })) }));
      const topLevel = Array.from(p.children).map(e => ({ cls: e.className, text: e.textContent.trim().slice(0, 80) }));
      return { display: getComputedStyle(p).display, cls: p.className, hidden: p.hidden, controls, topLevel, html: p.outerHTML.slice(0, 5000) };`);
    r.matchCaseDecorator = await c.evaluate(`${VIEW} const e = v.containerEl.querySelector('.input-right-decorator'); return e ? { cls: e.className, aria: e.getAttribute('aria-label'), html: e.outerHTML.slice(0, 400) } : null;`);
    r.toggleElAfter = await c.evaluate(`const e = document.querySelector('[data-cdp="settings-toggle"]'); return e ? { cls: e.className, aria: e.getAttribute('aria-label') } : null;`);
    return r;
  });

  // ---- step 2: sort dropdown ---------------------------------------------------
  const sortInfo = await step('04-sort-dropdown', async () => {
    const r = await c.evaluate(`${VIEW}${SORT_EL} const el = sortEl;
      if (!el) return { none: true, sortOrderDropdownKeys: d ? Object.keys(d) : null, paramsHtml: (v.searchParamsContainerEl || v.containerEl.querySelector('.search-params') || {}).outerHTML || null };
      const setting = el.closest('.setting-item');
      return { tag: el.tagName, how: sortHow, cls: el.className, value: el.value ?? null, selectedIndex: el.selectedIndex ?? null,
        options: el.options ? Array.from(el.options).map(o => ({ value: o.value, text: o.textContent, selected: o.selected })) : null,
        settingName: setting ? (setting.querySelector('.setting-item-name') || {}).textContent : null, html: el.outerHTML.slice(0, 1500), domSortOrder: dom.sortOrder,
        componentKeys: d ? Object.keys(d) : null, setSortOrderSrc: v.setSortOrder ? 'fn/' + v.setSortOrder.length : null,
        domChangedSrc: dom.changed ? 'fn/' + dom.changed.length : null, startSearchSrc: v.startSearch ? 'fn/' + v.startSearch.length : null };`);
    if (r.tag && r.tag !== 'SELECT') {
      // custom dropdown: open it for real and record what it shows
      await c.clickElement('[data-cdp="sort-dropdown"]'); await sleep(400);
      r.openedMenu = await c.evaluate(MENU_DUMP);
      r.openedLabels = await c.openMenuItems();
      r.popups = await c.evaluate(`return Array.from(document.querySelectorAll('.menu, .suggestion-container, .popover')).map(e => e.className);`);
      await c.pressKey('Escape'); await sleep(150);
    } else if (r.tag === 'SELECT') {
      r.note = 'native <select>: options read from its <option> children; the native popup is not in the DOM so it was not opened (see needs_human)';
    }
    return r;
  });

  // ---- step 3: each sort option ---------------------------------------------------
  const optionValues = (sortInfo && sortInfo.options) ? sortInfo.options.map(o => o.value) : (sortInfo && sortInfo.openedLabels && sortInfo.openedLabels[0]) ? sortInfo.openedLabels[0].filter(x => x !== '---') : [];
  await step('05-install-counters', async () => c.evaluate(`${VIEW}
    const proto = Object.getPrototypeOf(dom); const names = Object.getOwnPropertyNames(proto).filter(k => k !== 'constructor' && typeof Object.getOwnPropertyDescriptor(proto, k).value === 'function');
    // instance own properties (bound or debounced methods set in the constructor) are left alone: replacing them would change behaviour and 'delete' could not restore them
    const own = n => Object.prototype.hasOwnProperty.call(dom, n); const skippedOwn = names.filter(own); const wrap = names.filter(n => !own(n));
    window.__cdpCounts = {}; window.__cdpWrapped = wrap;
    for (const n of wrap) dom[n] = function (...a) { window.__cdpCounts[n] = (window.__cdpCounts[n] || 0) + 1; return proto[n].apply(this, a); };
    const vp = Object.getPrototypeOf(v); const vnames = ['startSearch', 'stopSearch', 'setSortOrder', 'renderSearchInfo', 'setQuery', 'onChange'].filter(n => typeof vp[n] === 'function' && !Object.prototype.hasOwnProperty.call(v, n));
    const skippedOwnView = ['startSearch', 'stopSearch', 'setSortOrder', 'renderSearchInfo', 'setQuery', 'onChange'].filter(n => Object.prototype.hasOwnProperty.call(v, n));
    for (const n of vnames) v[n] = function (...a) { window.__cdpCounts['view.' + n] = (window.__cdpCounts['view.' + n] || 0) + 1; return vp[n].apply(this, a); };
    window.__cdpWrappedView = vnames; return { domMethods: wrap, skippedOwnDomProps: skippedOwn, viewMethods: vnames, skippedOwnViewProps: skippedOwnView };`));

  // collapse the first row by its collapse icon so a sort switch shows whether rows reset
  await step('05b-collapse-first-row', async () => {
    const r = {};
    r.icon = await tag(c, `const e = v.containerEl.querySelector('.search-result .search-result-file-title .collapse-icon, .search-result .tree-item-icon'); return e;`, 'first-collapse');
    r.click = await c.clickElement('[data-cdp="first-collapse"]');
    await sleep(300);
    r.after = await c.evaluate(`${VIEW} return Array.from(v.containerEl.querySelectorAll('.search-result')).map(e => ({ t: (e.querySelector('.tree-item-inner') || e).textContent.trim().slice(0, 30), collapsed: e.classList.contains('is-collapsed') }));`);
    r.activeFile = await c.evaluate(`const f = app.workspace.getActiveFile(); return f ? f.path : null;`);
    return r;
  });

  for (const val of optionValues) {
    await step('06-sort-' + val, async () => {
      const r = {};
      r.before = await c.evaluate(`${VIEW} return { rows: Array.from(v.containerEl.querySelectorAll('.search-result')).map(e => ({ t: (e.querySelector('.tree-item-inner') || e).textContent.trim().slice(0, 30), collapsed: e.classList.contains('is-collapsed') })), counts: Object.assign({}, window.__cdpCounts) };`);
      await c.evaluate(`window.__cdpCounts = {}; return true;`);
      r.select = await selectSortOption(c, val);
      r.countSamples = await sampleCount(c, 900, 40);
      r.idle = await waitIdle(c, 4000);
      r.counts = await c.evaluate(`return Object.assign({}, window.__cdpCounts);`);
      r.results = await c.evaluate(READ_RESULTS);
      r.orderPaths = r.results.rows.map(x => x.path || x.title);
      r.orderTitles = r.results.rows.map(x => x.title);
      r.collapsedAfter = r.results.rows.map(x => ({ t: x.title, collapsed: x.collapsed }));
      r.plainTxtPresent = r.orderPaths.some(p => /plain\.txt$/i.test(p || '')) || r.orderTitles.some(t => /plain/i.test(t || ''));
      return r;
    });
  }

  await step('07-remove-counters', async () => c.evaluate(`${VIEW}
    for (const n of (window.__cdpWrapped || [])) delete dom[n]; for (const n of (window.__cdpWrappedView || [])) delete v[n];
    return { restored: true, domOwnFns: Object.keys(dom).filter(k => typeof dom[k] === 'function') };`));

  // back to default order for the menu/keyboard steps, then re-expand all rows
  await step('07b-restore-alphabetical', async () => {
    const r = {};
    const def = optionValues.includes('alphabetical') ? 'alphabetical' : optionValues[0];
    r.select = def ? await selectSortOption(c, def) : 'no options';
    r.idle = await waitIdle(c, 3000);
    r.rows = await c.evaluate(`${VIEW} return Array.from(v.containerEl.querySelectorAll('.search-result')).map(e => ({ t: (e.querySelector('.tree-item-inner') || e).textContent.trim().slice(0, 30), collapsed: e.classList.contains('is-collapsed') }));`);
    const collapsedIdx = r.rows.findIndex(x => x.collapsed);
    if (collapsedIdx >= 0) {
      await tag(c, `return v.containerEl.querySelectorAll('.search-result')[${collapsedIdx}].querySelector('.collapse-icon, .tree-item-icon');`, 'reexpand');
      r.reexpand = await c.clickElement('[data-cdp="reexpand"]'); await sleep(250);
      r.rowsAfter = await c.evaluate(`${VIEW} return Array.from(v.containerEl.querySelectorAll('.search-result')).map(e => e.classList.contains('is-collapsed'));`);
    }
    return r;
  });

  // ---- step 5: result count control ---------------------------------------------
  const countMenu = await step('08-result-count-click', async () => {
    const r = {};
    await c.pressKey('Escape'); await sleep(100);
    r.el = await tag(c, `return v.containerEl.querySelector('.search-results-result-count');`, 'result-count');
    r.menusBefore = await c.evaluate(`return document.querySelectorAll('.menu').length;`);
    r.click = await c.clickElement('[data-cdp="result-count"]');
    await sleep(500);
    r.menus = await c.evaluate(MENU_DUMP);
    r.labels = await c.openMenuItems();
    r.popups = await c.evaluate(`return Array.from(document.querySelectorAll('.menu, .popover, .modal-container, .suggestion-container, .tooltip')).map(e => ({ cls: e.className, text: e.textContent.trim().slice(0, 300) }));`);
    r.onCopyResultsClickSrc = await c.evaluate(`${VIEW} return v.onCopyResultsClick ? 'fn/' + v.onCopyResultsClick.length : null;`);
    await c.pressKey('Escape'); await sleep(200);
    r.menusAfterEscape = await c.evaluate(`return document.querySelectorAll('.menu').length;`);
    return r;
  });

  // ---- step 6: copy results -------------------------------------------------------
  await step('09-copy-results', async () => {
    const r = { where: null };
    let originalClipboard = null; try { originalClipboard = await c.readClipboard(); } catch {} // restored at the end, never recorded
    await c.writeClipboard('CDP-SENTINEL-' + Date.now());
    r.sentinel = await c.readClipboard();
    const findCopy = (labels) => { const l = titlesOf(labels) || []; return l.findIndex(t => /copy/i.test(t)); };
    let idx = countMenu && countMenu.labels ? findCopy(countMenu.labels) : -1;
    if (idx >= 0) {
      r.where = 'result-count menu'; r.itemLabel = countMenu.labels[0][idx];
      await c.clickElement('[data-cdp="result-count"]'); await sleep(450);
      const items = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(e => (e.querySelector('.menu-item-title') || e).textContent.trim());`);
      const i2 = items.findIndex(t => /copy/i.test(t));
      r.menuItemsLive = items;
      r.clickItem = await c.clickElement('.menu .menu-item', { index: i2 });
      await sleep(500);
    }
    if (!r.where) {
      // settings section: anything labelled copy
      const inSettings = await tag(c, `const p = v.searchParamsContainerEl || v.containerEl; return Array.from(p.querySelectorAll('[aria-label], button, .clickable-icon')).find(e => /copy/i.test((e.getAttribute('aria-label') || '') + ' ' + e.textContent));`, 'copy-ctl');
      r.settingsCandidate = inSettings;
      if (inSettings) { r.where = 'settings section'; r.click = await c.clickElement('[data-cdp="copy-ctl"]'); await sleep(500); }
    }
    if (!r.where) {
      // tab header menu
      await tag(c, `return leaf.tabHeaderEl || document.querySelector('.workspace-tab-header[data-type="search"]');`, 'search-tab');
      const m = await rightClickAndDump(c, '[data-cdp="search-tab"]');
      r.tabHeaderMenu = m;
      const i = findCopy(m.labels);
      if (i >= 0) {
        r.where = 'tab-header menu'; r.itemLabel = m.labels[0][i];
        await c.rightClickElement('[data-cdp="search-tab"]'); await sleep(450);
        const items = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(e => (e.querySelector('.menu-item-title') || e).textContent.trim());`);
        r.clickItem = await c.clickElement('.menu .menu-item', { index: items.findIndex(t => /copy/i.test(t)) }); await sleep(500);
      }
    }
    if (!r.where) {
      // any element in the view whose aria-label mentions copy
      const any = await tag(c, `return Array.from(v.containerEl.querySelectorAll('*')).find(e => /copy/i.test(e.getAttribute('aria-label') || ''));`, 'copy-any');
      r.anyCandidate = any;
      if (any) { r.where = 'element aria-label in view: ' + any.aria; r.click = await c.clickElement('[data-cdp="copy-any"]'); await sleep(500); }
    }
    r.clipboardAfterUi = await c.readClipboard();
    r.uiChangedClipboard = r.clipboardAfterUi !== r.sentinel;
    if (!r.uiChangedClipboard) {
      // last resort, clearly labelled: call the prototype method directly
      r.viaMethod = true;
      r.methodResult = await c.evaluate(`${VIEW} try { const ev = new MouseEvent('click'); const ret = v.onCopyResultsClick(ev); return { ok: true, ret: ret === undefined ? null : String(ret).slice(0, 100) }; } catch (e) { return { ok: false, error: e.message }; }`);
      await sleep(500);
      r.clipboardAfterMethod = await c.readClipboard();
    }
    const text = r.uiChangedClipboard ? r.clipboardAfterUi : r.clipboardAfterMethod;
    r.clipboardText = text;
    r.clipboardJson = JSON.stringify(text);
    r.clipboardLines = typeof text === 'string' ? text.split(/\r?\n/) : null;
    r.hasCRLF = typeof text === 'string' ? text.includes('\r\n') : null;
    r.tabIndented = typeof text === 'string' ? /^\t/m.test(text) : null;
    r.spaceIndented = typeof text === 'string' ? /^ +\S/m.test(text) : null;
    await c.pressKey('Escape');
    if (typeof originalClipboard === 'string') { try { await c.writeClipboard(originalClipboard); r.originalClipboardRestored = true; } catch (e) { r.originalClipboardRestored = 'failed: ' + e.message; } }
    return r;
  });

  // ---- step 9: context menus ----------------------------------------------------
  await step('10-menu-file-title', async () => {
    await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(300);
    const t = await tag(c, `let el = null; try { dom.resultDomLookup.forEach((d, f) => { if (f.path === 'apple.md') el = d.el; }); } catch (e) {}
      if (!el) el = Array.from(v.containerEl.querySelectorAll('.search-result')).find(e => (e.querySelector('.tree-item-inner') || e).textContent.trim() === 'apple');
      return el && el.querySelector('.search-result-file-title');`, 'apple-title');
    const m = await rightClickAndDump(c, '[data-cdp="apple-title"]');
    return { target: t, ...m };
  });
  await step('10-menu-match-line', async () => {
    const t = await tag(c, `let el = null; try { dom.resultDomLookup.forEach((d, f) => { if (f.path === 'apple.md') el = d.el; }); } catch (e) {}
      if (!el) el = v.containerEl.querySelector('.search-result'); return el && el.querySelector('.search-result-file-match');`, 'apple-match');
    const m = await rightClickAndDump(c, '[data-cdp="apple-match"]');
    return { target: t, ...m };
  });
  await step('10-menu-empty-space', async () => {
    const g = await c.evaluate(`${VIEW} const cont = v.containerEl.querySelector('.search-result-container'); if (!cont) return { noContainer: true, children: Array.from(v.containerEl.children).map(e => e.className) };
      const rows = v.containerEl.querySelectorAll('.search-result'); const last = rows[rows.length - 1];
      const cr = cont.getBoundingClientRect(); const lr = last ? last.getBoundingClientRect() : null;
      return { container: { x: cr.x, y: cr.y, w: cr.width, h: cr.height, bottom: cr.bottom }, lastRow: lr ? { bottom: lr.bottom } : null, scrollHeight: cont.scrollHeight, clientHeight: cont.clientHeight };`);
    const r = { geometry: g };
    if (g.noContainer) { r.note = 'no .search-result-container in the view; nothing clicked'; }
    else if (g.lastRow && g.lastRow.bottom + 12 < g.container.bottom) {
      const x = g.container.x + g.container.w / 2, y = Math.min(g.lastRow.bottom + 12, g.container.bottom - 4);
      await c.pressKey('Escape'); await sleep(100);
      r.clickedAt = { x, y };
      r.elementAtPoint = await c.evaluate(`const e = document.elementFromPoint(${x}, ${y}); return e ? e.className : null;`);
      await c.clickAt(x, y, { button: 'right' }); await sleep(450);
      r.menus = await c.evaluate(MENU_DUMP); r.labels = await c.openMenuItems();
      await c.pressKey('Escape'); await sleep(150);
    } else {
      r.note = 'no empty space below the last result (list fills the container); nothing clicked';
    }
    return r;
  });
  await step('10-menu-tab-header', async () => {
    const t = await tag(c, `return leaf.tabHeaderEl || document.querySelector('.workspace-tab-header[data-type="search"]');`, 'search-tab');
    const m = await rightClickAndDump(c, '[data-cdp="search-tab"]');
    return { target: t, ...m };
  });
  await step('10-menu-sidebar-header', async () => {
    const t = await tag(c, `const th = leaf.tabHeaderEl; const cont = th ? th.closest('.workspace-tab-header-container') : null; if (!cont) return null;
      return cont.querySelector('.workspace-tab-header-spacer') || cont.querySelector('.workspace-tab-header-container-inner') || cont;`, 'sidebar-header');
    const nav = await c.evaluate(`${VIEW} const n = v.containerEl.querySelector('.nav-header'); return n ? n.outerHTML.slice(0, 600) : null;`);
    const m = t ? await rightClickAndDump(c, '[data-cdp="sidebar-header"]') : { note: 'no tab header container found' };
    return { target: t, navHeaderInView: nav, ...m };
  });
  await step('10-menu-files-view-apple', async () => {
    const cmd = await c.evaluate(`return app.commands.executeCommandById('file-explorer:open');`); await sleep(600);
    const t = await tag(c, `const e = document.querySelector('.nav-file-title[data-path="apple.md"]'); if (e) e.scrollIntoView({ block: 'center' }); return e;`, 'files-apple');
    const m = await rightClickAndDump(c, '[data-cdp="files-apple"]');
    const searchMenu = out.steps['10-menu-file-title'] && out.steps['10-menu-file-title'].labels;
    const a = titlesOf(searchMenu), b = titlesOf(m.labels);
    const identical = !!(a && b) && JSON.stringify(a) === JSON.stringify(b);
    const r = { cmd, target: t, ...m, identicalToSearchFileTitleMenu: identical, searchTitleMenu: a, filesMenu: b };
    await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(400);
    return r;
  });

  // ---- step 8: keyboard ------------------------------------------------------------
  await step('11-keyboard', async () => {
    const r = { seq: [] };
    await c.pressKey('Escape'); await sleep(100);
    await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(300);
    r.clickInput = await c.clickElement('.search-input-container input'); await sleep(150);
    r.leavesBefore = await c.evaluate(LEAVES);
    const rec = async (label) => { const s = await c.evaluate(FOCUS_STATE); r.seq.push({ after: label, ...s }); };
    await rec('click-input');
    for (let i = 0; i < 5; i++) { await c.pressKey('ArrowDown'); await sleep(120); await rec('ArrowDown#' + (i + 1)); }
    await c.pressKey('ArrowUp'); await sleep(120); await rec('ArrowUp');
    await c.pressKey('ArrowLeft'); await sleep(150); await rec('ArrowLeft');
    await c.pressKey('ArrowLeft'); await sleep(150); await rec('ArrowLeft#2');
    await c.pressKey('ArrowRight'); await sleep(150); await rec('ArrowRight');
    await c.pressKey('ArrowRight'); await sleep(150); await rec('ArrowRight#2');
    await c.pressKey('Enter'); await sleep(700);
    r.afterEnter = await c.evaluate(LEAVES);
    // Ctrl+Enter on a focused row
    await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(300);
    await c.clickElement('.search-input-container input'); await sleep(120);
    await c.pressKey('ArrowDown'); await sleep(120); await c.pressKey('ArrowDown'); await sleep(120); await rec('before-ctrl-enter');
    await c.pressKey('Enter', { ctrl: true }); await sleep(700);
    r.afterCtrlEnter = await c.evaluate(LEAVES);
    // Shift+Enter
    await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(300);
    await c.clickElement('.search-input-container input'); await sleep(120);
    await c.pressKey('ArrowDown'); await sleep(120); await c.pressKey('ArrowDown'); await sleep(120); await c.pressKey('ArrowDown'); await sleep(120); await rec('before-shift-enter');
    await c.pressKey('Enter', { shift: true }); await sleep(700);
    r.afterShiftEnter = await c.evaluate(LEAVES);
    r.handlerSrc = await c.evaluate(`${VIEW} const p = Object.getPrototypeOf(v); const o = {}; for (const n of ['onKeyEnterInFocus', 'onKeyArrowLeftInFocus', 'onKeyArrowRightInFocus', 'onKeyArrowDownInFocus']) o[n] = typeof p[n] === 'function' ? 'fn/' + p[n].length : null; return o;`);
    // did any Enter variant create a file or a popout window? (vault bytes from disk; floating split from the workspace)
    r.vaultFilesAfter = listVaultFiles();
    r.newVaultFiles = r.vaultFilesAfter.filter(f => !(f in FIX));
    r.floatingWindows = await c.evaluate(`const fsplit = app.workspace.floatingSplit; return fsplit && fsplit.children ? fsplit.children.length : null;`);
    if (r.floatingWindows) { // detach the popout's leaves (public API; an emptied popout closes itself) so c.quit() sees one window
      r.detachedPopoutLeaves = await c.evaluate(`const ws = app.workspace; const main = [ws.rootSplit, ws.leftSplit, ws.rightSplit]; const gone = []; ws.iterateAllLeaves(l => { if (!main.includes(l.getRoot())) gone.push({ id: l.id, type: l.view && l.view.getViewType ? l.view.getViewType() : null }); });
        for (const g of gone) { try { const l = ws.getLeafById(g.id); if (l) l.detach(); } catch (e) { g.err = e.message; } } return gone;`);
      await sleep(500); r.floatingWindowsAfterDetach = await c.evaluate(`const fsplit = app.workspace.floatingSplit; return fsplit && fsplit.children ? fsplit.children.length : null;`); }
    await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(300);
    return r;
  }, 45000);

  // ---- step 7: toggles -------------------------------------------------------------
  await step('12-toggles', async () => {
    const r = { toggles: {} };
    await c.pressKey('Escape'); await sleep(100);
    // make sure the settings section is visible
    const vis = await c.evaluate(`${VIEW} const p = v.searchParamsContainerEl || v.containerEl.querySelector('.search-params'); return p ? getComputedStyle(p).display !== 'none' && !p.hidden : null;`);
    if (vis === false) { await c.clickElement('[data-cdp="settings-toggle"]'); await sleep(300); }
    r.baseline = await c.evaluate(READ_RESULTS);
    const list = await c.evaluate(`${VIEW} const p = v.searchParamsContainerEl || v.containerEl; const items = [];
      p.querySelectorAll('.setting-item').forEach((s, i) => { const t = s.querySelector('.checkbox-container'); if (t) { t.setAttribute('data-cdp', 'toggle-' + i); items.push({ idx: i, name: (s.querySelector('.setting-item-name') || {}).textContent || null, enabled: t.classList.contains('is-enabled') }); } });
      const mc = v.containerEl.querySelector('.input-right-decorator'); if (mc) { mc.setAttribute('data-cdp', 'toggle-matchcase'); items.push({ idx: 'matchcase', name: mc.getAttribute('aria-label'), enabled: mc.classList.contains('is-active'), cls: mc.className }); }
      return items;`);
    r.list = list;
    for (const t of list) {
      const sel = `[data-cdp="toggle-${t.idx}"]`; const one = { name: t.name };
      try {
        one.on = await c.clickElement(sel); await sleep(400); one.idleOn = await waitIdle(c, 3000);
        one.stateOn = await c.evaluate(`${VIEW} const e = document.querySelector('${sel}'); return { state: v.getState(), elCls: e && e.className, dom: { collapseAll: dom.collapseAll, extraContext: dom.extraContext, sortOrder: dom.sortOrder } };`);
        one.resultsOn = await c.evaluate(READ_RESULTS);
        one.matchHtmlSample = await c.evaluate(`${VIEW} const m = v.containerEl.querySelector('.search-result-file-match'); return m ? m.outerHTML.slice(0, 1200) : null;`);
        one.explainEls = await c.evaluate(`${VIEW} return Array.from(v.containerEl.querySelectorAll('[class*="explain"]')).map(e => ({ cls: e.className, text: e.textContent.trim().slice(0, 600), html: e.outerHTML.slice(0, 800) }));`);
        one.rowsCollapsed = one.resultsOn.rows.map(x => ({ t: x.title, collapsed: x.collapsed, matchCount: x.matchCount }));
        if (t.idx === 'matchcase') {
          // also type the query with a capital N
          await c.clickElement('.search-input-container input'); await sleep(100);
          await c.pressKey('a', { ctrl: true }); await sleep(80);
          one.selectAll1 = await c.evaluate(`const i = document.querySelector('.search-input-container input'); if (!i) return null; if (i.selectionStart === 0 && i.selectionEnd === i.value.length) return 'selected-by-ctrl-a'; i.focus(); i.select(); return 'selected-by-script (ctrl+a did not select all)';`);
          await c.typeText('Needle'); one.idleNeedleCap = await waitIdle(c, 3000);
          one.inputValueNeedleCapital = await c.evaluate(`return document.querySelector('.search-input-container input').value;`);
          one.resultsNeedleCapital = await c.evaluate(READ_RESULTS);
          one.orderNeedleCapital = one.resultsNeedleCapital.rows.map(x => x.path || x.title);
          await c.pressKey('a', { ctrl: true }); await sleep(80);
          one.selectAll2 = await c.evaluate(`const i = document.querySelector('.search-input-container input'); if (!i) return null; if (i.selectionStart === 0 && i.selectionEnd === i.value.length) return 'selected-by-ctrl-a'; i.focus(); i.select(); return 'selected-by-script (ctrl+a did not select all)';`);
          await c.typeText('needle'); await waitIdle(c, 3000);
          one.inputValueRestored = await c.evaluate(`return document.querySelector('.search-input-container input').value;`);
        }
        one.off = await c.clickElement(sel); await sleep(350); await waitIdle(c, 3000);
        one.stateOff = await c.evaluate(`${VIEW} return v.getState();`);
        one.orderOn = one.resultsOn.rows.map(x => x.path || x.title);
      } catch (e) { one.error = String(e).slice(0, 500); }
      r.toggles[String(t.idx)] = one;
      save();
    }
    return r;
  }, 60000);

  // ---- step 4 prep: leave sort on modified-time, two flags on, then quit ----------------
  await step('13-final-state', async () => {
    const r = {};
    const mod = (sortInfo && sortInfo.options || []).find(o => /modif/i.test(o.value + ' ' + o.text)) || null;
    r.modifiedOption = mod;
    r.select = mod ? await selectSortOption(c, mod.value) : 'no modified option found';
    r.idle = await waitIdle(c, 3000);
    r.resultsModified = await c.evaluate(READ_RESULTS);
    r.orderModified = r.resultsModified.rows.map(x => x.path || x.title);
    // leave extra-context and explain-search on so their stored keys/values are visible
    const want = await c.evaluate(`${VIEW} const p = v.searchParamsContainerEl || v.containerEl; const items = [];
      p.querySelectorAll('.setting-item').forEach((s, i) => { const t = s.querySelector('.checkbox-container'); if (t) { t.setAttribute('data-cdp', 'toggle-' + i); items.push({ idx: i, name: (s.querySelector('.setting-item-name') || {}).textContent || null, enabled: t.classList.contains('is-enabled') }); } });
      return items;`);
    r.turnedOn = [];
    for (const t of want) {
      if (/context|explain/i.test(t.name || '') && !t.enabled) { await c.clickElement(`[data-cdp="toggle-${t.idx}"]`); await sleep(300); r.turnedOn.push(t.name); }
    }
    await waitIdle(c, 3000);
    r.stateBeforeQuit = await c.evaluate(`${VIEW} return v.getState();`);
    r.localStorage = await c.evaluate(LOCAL_STORAGE);
    r.inMemoryLayoutSearchLeaf = await c.evaluate(`const lay = app.workspace.getLayout(); const found = []; (function walk(n) { if (!n || typeof n !== 'object') return; if (n.type === 'leaf' && n.state && n.state.type === 'search') found.push(n); for (const k in n) if (typeof n[k] === 'object') walk(n[k]); })(lay); return found;`);
    return r;
  });

  await step('14-quit', async () => {
    const r = {};
    r.quit = await c.quit();
    await sleep(800);
    for (const f of ['app.json', 'workspace.json']) { try { r[f] = fs.readFileSync(path.join(VAULT, '.obsidian', f), 'utf8'); } catch (e) { r[f] = 'absent: ' + e.message; } }
    try { r.obsidianDir = fs.readdirSync(path.join(VAULT, '.obsidian')); } catch (e) { r.obsidianDir = 'absent'; }
    r.searchLeafState = findSearchLeaf(r['workspace.json']);
    return r;
  }, 45000, { always: true });
}

function findSearchLeaf(text) {
  try {
    const j = JSON.parse(text); const found = [];
    (function walk(n) { if (!n || typeof n !== 'object') return; if (n.type === 'leaf' && n.state && n.state.type === 'search') found.push(n); for (const k in n) if (typeof n[k] === 'object') walk(n[k]); })(j);
    return found;
  } catch (e) { return 'parse error: ' + e.message; }
}

// ---------------------------------------------------------------------------
async function afterRestart() {
  await step('20-disk-after-restart', async () => {
    const r = {};
    for (const f of ['app.json', 'workspace.json']) { try { r[f] = fs.readFileSync(path.join(VAULT, '.obsidian', f), 'utf8'); } catch (e) { r[f] = 'absent: ' + e.message; } }
    r.searchLeafState = findSearchLeaf(r['workspace.json']);
    r.sortKeysInAppJson = typeof r['app.json'] === 'string' ? (r['app.json'].match(/"[^"]*sort[^"]*"/gi) || []) : null;
    r.sortKeysInWorkspace = typeof r['workspace.json'] === 'string' ? (r['workspace.json'].match(/"[^"]*sort[^"]*"\s*:\s*"[^"]*"/gi) || []) : null;
    return r;
  });
  await waitForObsidian();
  await sleep(3000);
  const c = await connect(); conn = c;
  await step('21-live-after-restart', async () => {
    const r = {};
    r.leafPresent = await c.evaluate(`const l = app.workspace.getLeavesOfType('search')[0]; if (!l) return null; let st = null; try { st = l.view.getState(); } catch (e) { st = 'err:' + e.message; }
      return { id: l.id, isDeferred: !!l.isDeferred, viewType: l.view && l.view.getViewType ? l.view.getViewType() : null, state: st, inLeftSplit: l.getRoot() === app.workspace.leftSplit, hasDom: !!(l.view && l.view.dom) };`);
    // reveal the pane so a deferred view loads and the restored query runs
    r.reveal = await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(1200);
    r.live = await c.evaluate(`${VIEW}${SORT_EL} const el = sortEl;
      return { sortHow, dropdown: el ? { tag: el.tagName, value: el.value ?? null, selectedText: el.options ? (el.options[el.selectedIndex] || {}).textContent : el.textContent.trim() } : null, domSortOrder: dom.sortOrder,
        collapseAll: dom.collapseAll, extraContext: dom.extraContext, state: v.getState(), inputValue: (v.containerEl.querySelector('.search-input-container input') || {}).value ?? null };`);
    r.idle = await waitIdle(c, 4000);
    r.results = await c.evaluate(READ_RESULTS);
    r.orderAfterRestart = r.results.rows.map(x => x.path || x.title);
    r.localStorage = await c.evaluate(LOCAL_STORAGE);
    r.toggleEls = await c.evaluate(`${VIEW} const p = v.searchParamsContainerEl || v.containerEl; return Array.from(p.querySelectorAll('.setting-item')).map(s => ({ name: (s.querySelector('.setting-item-name') || {}).textContent || null, enabled: !!(s.querySelector('.checkbox-container.is-enabled')) }));`);
    return r;
  });
  try { c.close(); } catch {}
}

// ---------------------------------------------------------------------------
try {
  if (phase === 'fixtures') fixtures();
  else if (phase === 'run') await run();
  else if (phase === 'after-restart') await afterRestart();
  else throw new Error('unknown phase: ' + phase);
} catch (e) {
  out.fatal = String((e && e.stack) || e).slice(0, 2000);
}
try { if (conn) conn.close(); } catch {} // an open socket would keep node alive
out.finishedAt = new Date().toISOString();
save();
console.log(JSON.stringify({ phase, steps: Object.fromEntries(Object.entries(out.steps).map(([k, v]) => [k, v.ok ? 'ok' : 'FAIL ' + (v.error || '').slice(0, 120)])), fatal: out.fatal || null }, null, 1));