// Properties in the reading view: the whole editing path, driven over CDP.
// Phases: fixtures | run | after-restart   (node block-properties.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep, waitForObsidian } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 170000; // "run" must finish under 4 minutes: steps starting after this are skipped (a step already running may add ~30 s, the quit up to 40 s), the quit still happens.

// ---- results file ---------------------------------------------------------
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.properties = all.properties || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.properties[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

async function step(name, fn) {
  out.order.push(name);
  if (Date.now() - T0 > BUDGET_MS) { out.steps[name] = { ok: false, error: 'skipped: time budget exhausted' }; save(); return null; }
  const t = Date.now();
  const partial = {}; // steps fill this in as they go, so a throw half way keeps what was observed before it
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save();
  return out.steps[name];
}

// ---- host-side vault reads ------------------------------------------------
const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
function lsObsidian() {
  const dir = vp('.obsidian');
  try {
    return fs.readdirSync(dir, { withFileTypes: true }).map((d) => {
      const st = fs.statSync(path.join(dir, d.name));
      return { name: d.name, dir: d.isDirectory(), size: st.size, mtimeMs: Math.round(st.mtimeMs) };
    }).sort((a, b) => a.name.localeCompare(b.name));
  } catch (e) { return 'ERR:' + e.message; }
}
const snap = () => ({ at: new Date().toISOString(), appJson: bytes('.obsidian/app.json'), typesJson: bytes('.obsidian/types.json'), typesJsonExists: fs.existsSync(vp('.obsidian/types.json')), ls: lsObsidian() });
function lineDiff(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string') return null;
  const A = a.split('\n'), B = b.split('\n');
  return { removed: A.filter((l) => !B.includes(l)), added: B.filter((l) => !A.includes(l)), endsWithNewlineBefore: a.endsWith('\n'), endsWithNewlineAfter: b.endsWith('\n') };
}

// ---- fixtures -------------------------------------------------------------
const NOTE_A = '---\nStatus: draft\nzebra: alpha-note\nalpha: 1\ntags:\n  - one\n  - two\naliases:\n  - Alpha\ncount: 3\ndone: true\ndue: 2026-10-18\nlink: "[[Note B]]"\nlong value: A fairly long property value used to see how the sidebar lays a value out when the pane is narrow\n---\n\n# Note A\n\nBody text with a [[Note B]] link.\n';
const FIXTURES = {
  'Note A.md': NOTE_A,
  'Note B.md': '---\nstatus: shipped\nzebra: beta-note\ncount: 7\n---\n\n# Note B\n\nBody B.\n',
  'Note C.md': '---\nSTATUS: archived\nzebra: gamma-note\nother: x\n---\n\n# Note C\n\nBody C.\n',
  'No Frontmatter.md': '# No Frontmatter\n\nThis note has no properties block at all.\n',
  'Dotted.md': '---\nbook.title: Dune\nbook.year: 1965\n---\n\n# Dotted\n\nDotted keys.\n',
  'Nested.md': '---\nshelf:\n  title: Emma\n  year: 1815\n---\n\n# Nested\n\nA nested map, for contrast with Dotted.md.\n',
  'Past Date.md': '---\ndue: 2020-03-04\n---\n\n# Past Date\n\nA date well in the past.\n',
  'Plain.txt': 'a plain text file, not markdown\n',
};

if (phase === 'fixtures') {
  await step('fixtures', async () => {
    fs.mkdirSync(vp('.obsidian'), { recursive: true });
    fs.writeFileSync(vp('.obsidian/app.json'), '{}');
    for (const [name, text] of Object.entries(FIXTURES)) fs.writeFileSync(vp(name), text);
    const written = {};
    for (const name of Object.keys(FIXTURES)) written[name] = bytes(name);
    return { written, appJson: bytes('.obsidian/app.json'), ls: lsObsidian(), vaultFiles: fs.readdirSync(VAULT).sort() };
  });
  process.exit(0);
}

// ---- renderer preamble (prepended to every evaluate) ----------------------
const PRE = `
const PV = '.mod-root .markdown-reading-view .metadata-container';
const AP = '.workspace-leaf-content[data-type="all-properties"]';
const FP = '.workspace-leaf-content[data-type="file-properties"]';
const $ = (s, r) => (r || document).querySelector(s);
const $$ = (s, r) => Array.from((r || document).querySelectorAll(s));
const attr = (el, a) => el ? el.getAttribute(a) : null;
const trunc = (s, n) => { s = s == null ? '' : String(s); return s.length > n ? s.slice(0, n) + '...[len=' + s.length + ']' : s; };
const rct = (el) => { if (!el) return null; const r = el.getBoundingClientRect(); return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) }; };
const desc = (el, n) => el ? { tag: el.tagName, cls: attr(el, 'class') || '', text: trunc(el.innerText != null ? el.innerText : el.textContent, 400), value: el.value != null ? String(el.value) : undefined, rect: rct(el), display: getComputedStyle(el).display, html: trunc(el.outerHTML, n == null ? 1200 : n) } : null;
const act = () => { const a = document.activeElement; if (!a) return null; const row = a.closest ? a.closest('.metadata-property') : null; const sel = document.getSelection();
  return { tag: a.tagName, cls: attr(a, 'class') || '', type: attr(a, 'type'), contentEditable: a.isContentEditable === true, value: trunc(a.value != null ? a.value : a.innerText, 200), ariaLabel: attr(a, 'aria-label'), placeholder: attr(a, 'placeholder'),
    rowKey: row ? attr(row, 'data-property-key') : null, inReadingView: !!(a.closest && a.closest('.markdown-reading-view')), inFilePropsPane: !!(a.closest && a.closest(FP)), inAllPropsPane: !!(a.closest && a.closest(AP)), inModal: !!(a.closest && a.closest('.modal-container')),
    selection: sel ? { rangeCount: sel.rangeCount, collapsed: sel.isCollapsed, anchorOffset: sel.anchorOffset, focusOffset: sel.focusOffset, anchorInActive: sel.anchorNode ? a.contains(sel.anchorNode) : false } : null, rect: rct(a) }; };
const row = (k, scope) => $((scope || PV) + ' .metadata-property[data-property-key="' + k + '"]');
const rowInfo = (k, scope) => { const r = row(k, scope); if (!r) return null; const v = $('.metadata-property-value', r); const w = v ? v.firstElementChild : null;
  return { key: k, label: ($('.metadata-property-key-input', r) || {}).value, type: attr(v, 'data-property-type'), valueText: v ? trunc(v.innerText, 300) : null, valueInnerHTML: w ? trunc(w.innerHTML, 400) : null, widgetTag: w ? w.tagName : null, widgetType: attr(w, 'type'), widgetValue: w && w.value != null ? String(w.value) : undefined, checked: w && w.type === 'checkbox' ? w.checked : undefined,
    pills: $$('.multi-select-pill-content', r).map(p => p.textContent), icons: $$('.clickable-icon', r).map(i => ({ label: attr(i, 'aria-label'), cls: attr(i, 'class'), rect: rct(i), display: getComputedStyle(i).display })), html: trunc(r.outerHTML, 1800) }; };
const rows = (scope) => $$((scope || PV) + ' .metadata-property').map(r => { const v = $('.metadata-property-value', r); return { key: attr(r, 'data-property-key'), label: ($('.metadata-property-key-input', r) || {}).value, type: attr(v, 'data-property-type'), text: trunc(v ? v.innerText : '', 120), widgetValue: v && v.firstElementChild && v.firstElementChild.value != null ? String(v.firstElementChild.value) : undefined }; });
const foldState = () => { const c = $(PV); if (!c) return null; const content = $('.metadata-content', c); const ind = $('.collapse-indicator', c); const heading = $('.metadata-properties-heading', c);
  return { containerClass: attr(c, 'class'), count: attr(c, 'data-property-count'), headingClass: attr(heading, 'class'), indicatorClass: attr(ind, 'class'), contentDisplay: content ? getComputedStyle(content).display : null, contentRect: rct(content), firstRowRects: $$('.metadata-property', c).slice(0, 2).map(rct), containerText: trunc(c.innerText, 200) }; };
const menuItems = () => $$('.menu .menu-item').map(i => ({ label: (($('.menu-item-title', i) || i).textContent || '').trim(), checked: i.classList.contains('mod-checked'), disabled: i.classList.contains('is-disabled'), cls: attr(i, 'class') }));
const menuHtml = () => $$('.menu').map(m => trunc(m.outerHTML, 3000));
const modalInfo = () => { const m = $('.modal-container'); if (!m) return null; return { text: trunc(m.innerText, 1500), buttons: $$('button', m).map(b => ({ text: b.textContent.trim(), cls: attr(b, 'class') })), inputs: $$('input, select, textarea', m).map(i => ({ tag: i.tagName, type: attr(i, 'type'), value: trunc(i.value, 100), cls: attr(i, 'class'), placeholder: attr(i, 'placeholder') })), html: trunc(m.outerHTML, 4000) }; };
const splitName = (root) => root === app.workspace.rootSplit ? 'root' : root === app.workspace.leftSplit ? 'left' : root === app.workspace.rightSplit ? 'right' : 'other';
const leaves = () => { const r = []; app.workspace.iterateAllLeaves(l => { let t = null, f = null, mode = null; try { t = l.view.getViewType(); } catch (e) {} try { f = l.view.file ? l.view.file.path : null; } catch (e) {} try { mode = l.view.getMode ? l.view.getMode() : null; } catch (e) {}
  r.push({ type: t, file: f, mode, split: splitName(l.getRoot()), rect: rct(l.containerEl), active: l === app.workspace.activeLeaf }); }); return r; };
const mainMdLeaf = () => app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit) || null;
const openPreview = async (p) => { const f = app.vault.getAbstractFileByPath(p); if (!f) throw new Error('no file ' + p); let leaf = mainMdLeaf(); if (!leaf) leaf = app.workspace.getLeaf(false);
  await leaf.openFile(f); const st = leaf.getViewState(); st.state = Object.assign({}, st.state, { file: p, mode: 'preview' }); await leaf.setViewState(st); app.workspace.setActiveLeaf(leaf, { focus: false }); await new Promise(r => setTimeout(r, 800));
  return { file: leaf.view.file && leaf.view.file.path, mode: leaf.view.getMode ? leaf.view.getMode() : null, state: leaf.getViewState() }; };
const previewInfo = () => { const pv = $('.mod-root .markdown-preview-view'); return pv ? { cls: attr(pv, 'class'), showProperties: pv.classList.contains('show-properties'), metadataDisplayReading: getComputedStyle(pv).getPropertyValue('--metadata-display-reading').trim(), text: trunc(pv.innerText, 300), containerPresent: !!$(PV), containerDisplay: $(PV) ? getComputedStyle($(PV)).display : null, containerRect: rct($(PV)), frontmatterPre: $$('.mod-root .markdown-preview-view .el-pre.mod-frontmatter').map(e => trunc(e.outerHTML, 300)) } : null; };
`;

let c;
const ev = (body) => c.evaluate(PRE + body);

async function clickMenu(re) {
  const items = await ev(`return menuItems();`);
  const idx = items.findIndex((i) => re.test(i.label));
  if (idx < 0) { await c.pressKey('Escape'); await sleep(300); return { found: false, items, dismissed: 'Escape' }; }
  const r = await c.clickElement('.menu .menu-item', { index: idx });
  await sleep(450);
  return { found: true, idx, label: items[idx].label, rect: r, items };
}
async function clickBody() {
  // The first word of the body paragraph (the [[Note B]] link sits mid-sentence, so stay at the left edge); fall back to the heading.
  let r = await c.rect('.mod-root .markdown-reading-view .markdown-preview-sizer p');
  let target = 'p';
  if (!r || (r.w === 0 && r.h === 0)) { r = await c.rect('.mod-root .markdown-reading-view .markdown-preview-sizer h1'); target = 'h1'; }
  if (!r) return { clicked: false };
  await c.clickAt(r.x + 8, r.cy);
  return { clicked: true, target, at: { x: r.x + 8, y: r.cy } };
}
const rowSel = (k) => `.mod-root .markdown-reading-view .metadata-container .metadata-property[data-property-key="${k}"]`;
const AP = '.workspace-leaf-content[data-type="all-properties"]';
const FP = '.workspace-leaf-content[data-type="file-properties"]';

async function waitReady(ms = 30000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { if (await c.evaluate(`return !!(app && app.workspace && app.workspace.layoutReady);`)) return true; } catch {}
    await sleep(500);
  }
  return false;
}

// ============================================================================
if (phase === 'run') {
  await waitForObsidian(90000);
  await sleep(2000);
  c = await connect();

  await step('00-connect', async (r) => {
    r.ready = await waitReady();
    await sleep(1500);
    const info = r.info = await ev(`return { ua: navigator.userAgent, vault: app.vault.getName(), layoutReady: app.workspace.layoutReady, modal: modalInfo(), leaves: leaves(), innerWidth: innerWidth, innerHeight: innerHeight, configDir: app.vault.configDir };`);
    if (info.modal) { await c.pressKey('Escape'); await sleep(300); info.modalAfterEscape = await ev(`return modalInfo();`); }
    return { ready: r.ready, ...info };
  });

  await step('00b-plugins', async () => {
    const before = await ev(`const p = app.internalPlugins.plugins; return { properties: p.properties ? p.properties.enabled : null, dailyNotes: p['daily-notes'] ? p['daily-notes'].enabled : null, corePluginsJson: null };`);
    before.corePluginsJson = bytes('.obsidian/core-plugins.json');
    let enabled = null, dailyEnabled = null;
    if (before.properties === false) { enabled = await ev(`await app.internalPlugins.plugins.properties.enable(true); return app.internalPlugins.plugins.properties.enabled;`); }
    if (before.dailyNotes === false) { dailyEnabled = await ev(`await app.internalPlugins.plugins['daily-notes'].enable(true); return app.internalPlugins.plugins['daily-notes'].enabled;`); }
    await sleep(800);
    const after = await ev(`const p = app.internalPlugins.plugins; return { properties: p.properties ? p.properties.enabled : null, dailyNotes: p['daily-notes'] ? p['daily-notes'].enabled : null, commands: ['properties:open','properties:open-local','editor:toggle-fold-properties','markdown:add-metadata-property','markdown:clear-metadata-properties'].map(id => ({ id, name: app.commands.commands[id] ? app.commands.commands[id].name : null })) };`);
    after.corePluginsJson = bytes('.obsidian/core-plugins.json');
    return { before, enabledCall: enabled, dailyNotesEnableCall: dailyEnabled, after };
  });

  // 1. clean baseline ---------------------------------------------------------
  await step('01-baseline', async (r) => {
    r.disk = { ...snap(), noteA: bytes('Note A.md'), noteAMatchesFixture: bytes('Note A.md') === NOTE_A, noteB: bytes('Note B.md'), noteC: bytes('Note C.md'), vaultFiles: fs.readdirSync(VAULT).sort() };
    r.open = await ev(`return await openPreview('Note A.md');`);
    await sleep(700);
    r.dom = await ev(`return { preview: previewInfo(), containerHtml: trunc(($(PV) || {}).outerHTML, 6000), rows: rows(), due: rowInfo('due'), link: rowInfo('link'), fold: foldState(), addButton: desc($(PV + ' .metadata-add-button')), leaves: leaves(), propertiesInDocument: app.vault.getConfig('propertiesInDocument'), typeManagerRawTypes: (function(){ try { return JSON.parse(JSON.stringify(app.metadataTypeManager.types || null)); } catch (e) { return 'ERR:' + e; } })() };`);
    return r;
  });

  // 2. reading-view text edit -> disk ---------------------------------------
  await step('02-edit-zebra', async (r) => {
    r.before = await ev(`return rowInfo('zebra');`);
    r.click = await c.clickElement(rowSel('zebra') + ' .metadata-input-longtext');
    await sleep(350);
    r.activeAfterClick = await ev(`return act();`);
    r.caretAppeared = !!(r.activeAfterClick && r.activeAfterClick.contentEditable && r.activeAfterClick.selection && r.activeAfterClick.selection.anchorInActive);
    await c.pressKey('End'); // the click lands mid-text; End puts the caret at the end so the typed text is a suffix (recorded deviation)
    r.activeAfterEnd = await ev(`return act();`);
    await c.typeText(' edited');
    await sleep(200);
    r.fieldAfterTyping = await ev(`return rowInfo('zebra');`);
    r.bytesWhileFocused = bytes('Note A.md');
    r.bodyClick = await clickBody();
    await sleep(400);
    r.activeAfterBodyClick = await ev(`return act();`);
    r.bytesAfter400ms = bytes('Note A.md');
    await sleep(4600);
    r.bytesAfter5s = bytes('Note A.md');
    r.diffVsFixture = lineDiff(NOTE_A, r.bytesAfter5s);
    r.rowAfter = await ev(`return rowInfo('zebra');`);
    r.snap = snap();
    return r;
  });

  // 3. Enter / Tab / Escape / Ctrl+Z ----------------------------------------
  await step('03-keys', async (r) => {
    Object.assign(r, { enter: {}, escape: {}, tab: {}, undo: {} });
    const base = bytes('Note A.md');
    r.bytesBefore = base;
    // Enter in "long value"
    r.enter.click = await c.clickElement(rowSel('long value') + ' .metadata-input-longtext');
    await sleep(300);
    r.enter.activeBefore = await ev(`return act();`);
    await c.pressKey('End');
    await c.pressKey('Enter');
    await sleep(400);
    r.enter.activeAfter = await ev(`return act();`);
    r.enter.rowAfter = await ev(`return rowInfo('long value');`);
    await sleep(1200);
    r.enter.bytesAfter = bytes('Note A.md');
    r.enter.diff = lineDiff(base, r.enter.bytesAfter);
    await c.pressKey('Escape');
    await sleep(400);
    r.enter.activeAfterEscape = await ev(`return act();`);
    r.enter.rowAfterEscape = await ev(`return rowInfo('long value');`);
    await sleep(1000);
    r.enter.bytesAfterEscape = bytes('Note A.md');
    // Escape in zebra
    const base2 = bytes('Note A.md');
    r.escape.click = await c.clickElement(rowSel('zebra') + ' .metadata-input-longtext');
    await sleep(300);
    await c.pressKey('End');
    await c.typeText('zzz');
    await sleep(150);
    r.escape.fieldAfterTyping = await ev(`return rowInfo('zebra');`);
    await c.pressKey('Escape');
    await sleep(400);
    r.escape.activeAfter = await ev(`return act();`);
    r.escape.fieldAfterEscape = await ev(`return rowInfo('zebra');`);
    await sleep(1200);
    r.escape.bytesAfter = bytes('Note A.md');
    r.escape.diff = lineDiff(base2, r.escape.bytesAfter);
    // Tab in zebra
    r.tab.click = await c.clickElement(rowSel('zebra') + ' .metadata-input-longtext');
    await sleep(300);
    await c.pressKey('End');
    await c.typeText('qqq');
    await sleep(150);
    r.tab.fieldAfterTyping = await ev(`return rowInfo('zebra');`);
    await c.pressKey('Tab');
    await sleep(400);
    r.tab.activeAfter = await ev(`return act();`);
    r.tab.rowsAfter = await ev(`return rows();`);
    await sleep(1200);
    r.tab.bytesAfter = bytes('Note A.md');
    r.tab.diff = lineDiff(base2, r.tab.bytesAfter);
    // Ctrl+Z twice, wherever Tab left the focus
    await c.pressKey('z', { ctrl: true });
    await sleep(300);
    r.undo.activeAfterFirst = await ev(`return act();`);
    r.undo.zebraAfterFirst = await ev(`return rowInfo('zebra');`);
    await c.pressKey('z', { ctrl: true });
    await sleep(300);
    r.undo.activeAfterSecond = await ev(`return act();`);
    r.undo.zebraAfterSecond = await ev(`return rowInfo('zebra');`);
    r.undo.rowsAfter = await ev(`return rows();`);
    await sleep(1500);
    r.undo.bytesAfter = bytes('Note A.md');
    r.undo.diffVsAfterTab = lineDiff(r.tab.bytesAfter, r.undo.bytesAfter);
    r.undo.bodyClick = await clickBody();
    await sleep(1500);
    r.undo.bytesAfterBodyClick = bytes('Note A.md');
    r.undo.zebraFinal = await ev(`return rowInfo('zebra');`);
    r.snap = snap();
    return r;
  });

  // 4. non-text widgets -------------------------------------------------------
  await step('04-widgets', async (r) => {
    Object.assign(r, { count: {}, done: {}, due: {}, tags: {} });
    r.bytesBefore = bytes('Note A.md');
    // count (number)
    r.count.before = await ev(`return rowInfo('count');`);
    r.count.click = await c.clickElement(rowSel('count') + ' input');
    await sleep(300);
    r.count.active = await ev(`return act();`);
    await c.pressKey('End');
    await c.typeText('9');
    await sleep(200);
    r.count.after = await ev(`return rowInfo('count');`);
    // done (checkbox)
    r.done.before = await ev(`return rowInfo('done');`);
    r.done.click = await c.clickElement(rowSel('done') + ' input');
    await sleep(400);
    r.done.after = await ev(`return rowInfo('done');`);
    r.done.bytesAfter = bytes('Note A.md');
    await sleep(800);
    r.count.bytesAfterBlur = bytes('Note A.md');
    // due (date): click near the left edge (first segment), ArrowRight to the next segment, ArrowUp to bump it.
    r.due.before = await ev(`return rowInfo('due');`);
    const dr = await c.rect(rowSel('due') + ' input');
    r.due.inputRect = dr;
    if (dr) {
      await c.clickAt(dr.x + 12, dr.cy);
      await sleep(300);
      r.due.activeAfterClick = await ev(`return act();`);
      await c.pressKey('ArrowRight');
      await c.pressKey('ArrowUp');
      await sleep(300);
      r.due.afterArrows = await ev(`return rowInfo('due');`);
      if (r.due.afterArrows && r.due.afterArrows.widgetValue === (r.due.before && r.due.before.widgetValue)) {
        // the segment path did not change the value: type a full date the way a person would
        await c.clickAt(dr.x + 12, dr.cy);
        await sleep(200);
        if (r.due.activeAfterClick && r.due.activeAfterClick.type === 'date') await c.typeText('10202026');
        else { await c.pressKey('a', { ctrl: true }); await c.typeText('2026-10-20'); }
        await sleep(300);
        r.due.afterTyping = await ev(`return rowInfo('due');`);
      }
    }
    // tags: add "three", then remove "one"
    r.tags.before = await ev(`return rowInfo('tags');`);
    r.tags.click = await c.clickElement(rowSel('tags') + ' .multi-select-input');
    await sleep(300);
    r.tags.active = await ev(`return act();`);
    await c.typeText('three');
    await sleep(200);
    r.tags.suggestions = await ev(`const s = $('.suggestion-container'); return s ? { items: $$('.suggestion-item', s).slice(0, 8).map(i => i.textContent.trim()), html: trunc(s.outerHTML, 1500) } : null;`);
    await c.pressKey('Enter');
    await sleep(500);
    r.tags.afterEnter = await ev(`return rowInfo('tags');`);
    r.tags.bytesAfterEnter = bytes('Note A.md');
    const pillIdx = await ev(`return $$(PV + ' .metadata-property[data-property-key="tags"] .multi-select-pill').findIndex(p => (($('.multi-select-pill-content', p) || p).textContent || '').trim() === 'one');`);
    r.tags.onePillIndex = pillIdx;
    if (pillIdx >= 0) {
      r.tags.pillHtml = await ev(`return trunc($$(PV + ' .metadata-property[data-property-key="tags"] .multi-select-pill')[${pillIdx}].outerHTML, 800);`);
      await c.hover(rowSel('tags') + ' .multi-select-pill', pillIdx);
      await sleep(250);
      r.tags.removeButtonRect = await c.rect(rowSel('tags') + ' .multi-select-pill .multi-select-pill-remove-button', pillIdx);
      r.tags.removeClick = await c.clickElement(rowSel('tags') + ' .multi-select-pill .multi-select-pill-remove-button', { index: pillIdx });
      if (!r.tags.removeClick || r.tags.removeClick.skipped) {
        r.tags.removeFallback = await ev(`const b = $$(PV + ' .metadata-property[data-property-key="tags"] .multi-select-pill .multi-select-pill-remove-button')[${pillIdx}]; if (!b) return 'no button'; b.click(); return 'synthetic .click() used because the button had no rect';`);
      }
      await sleep(500);
      r.tags.afterRemove = await ev(`return rowInfo('tags');`);
    }
    r.bodyClick = await clickBody();
    await sleep(5000);
    r.bytesAfter5s = bytes('Note A.md');
    r.diff = lineDiff(r.bytesBefore, r.bytesAfter5s);
    r.rowsAfter = await ev(`return rows();`);
    r.dueRowAfter = await ev(`return rowInfo('due');`);
    r.snap = snap();
    return r;
  });

  // 5. Add property -----------------------------------------------------------
  await step('05-add-property', async (r) => {
    Object.assign(r, { first: {}, second: {} });
    r.bytesBefore = bytes('Note A.md');
    r.first.button = await ev(`return desc($(PV + ' .metadata-add-button'));`);
    r.first.click = await c.clickElement('.mod-root .markdown-reading-view .metadata-container .metadata-add-button');
    await sleep(500);
    r.first.ui = await ev(`return { active: act(), rows: rows(), suggestion: (function(){ const s = $('.suggestion-container'); return s ? { items: $$('.suggestion-item', s).slice(0, 10).map(i => i.textContent.trim()), html: trunc(s.outerHTML, 2500) } : null; })(), modal: modalInfo(), lastRowHtml: trunc(($$(PV + ' .metadata-property').pop() || {}).outerHTML, 1500) };`);
    r.first.typed = !!(r.first.click && !r.first.click.skipped);
    if (r.first.typed) await c.typeText('demo'); // nothing was clicked: do not type into whatever happens to be focused
    await sleep(400);
    r.first.suggestionAfterTyping = await ev(`const s = $('.suggestion-container'); return s ? { items: $$('.suggestion-item', s).slice(0, 10).map(i => i.textContent.trim()) } : null;`);
    await c.pressKey('Enter');
    await sleep(500);
    r.first.activeAfterEnter = await ev(`return act();`);
    r.first.rowsAfterEnter = await ev(`return rows();`);
    r.first.bytesAfterEnter = bytes('Note A.md');
    if (r.first.typed) await c.typeText('hello');
    await sleep(200);
    r.first.activeAfterValue = await ev(`return act();`);
    r.first.bodyClick = await clickBody();
    await sleep(1800);
    r.first.bytesAfter = bytes('Note A.md');
    r.first.diff = lineDiff(r.bytesBefore, r.first.bytesAfter);
    r.first.rowsAfter = await ev(`return rows();`);
    // second: empty key and value
    r.second.click = await c.clickElement('.mod-root .markdown-reading-view .metadata-container .metadata-add-button');
    await sleep(500);
    r.second.ui = await ev(`return { active: act(), rows: rows() };`);
    r.second.bodyClick = await clickBody();
    await sleep(1800);
    r.second.bytesAfter = bytes('Note A.md');
    r.second.diff = lineDiff(r.first.bytesAfter, r.second.bytesAfter);
    r.second.rowsAfter = await ev(`return rows();`);
    r.second.active = await ev(`return act();`);
    r.snap = snap();
    return r;
  });

  // 6. remove, type change, daily-note icon ----------------------------------
  await step('06-remove-retype', async (r) => {
    Object.assign(r, { menuReading: {}, menuSidebar: {}, remove: {}, retype: {}, pastDate: {} });
    r.bytesBefore = bytes('Note A.md');
    // context menu on the alpha row, reading view
    r.menuReading.rowHtml = await ev(`return trunc((row('alpha') || {}).outerHTML, 1500);`);
    r.menuReading.click = await c.rightClickElement(rowSel('alpha') + ' .metadata-property-key');
    await sleep(450);
    r.menuReading.items = await ev(`return menuItems();`);
    r.menuReading.harnessItems = await c.openMenuItems();
    r.menuReading.html = await ev(`return menuHtml();`);
    await c.pressKey('Escape');
    await sleep(300);
    r.menuReading.menuAfterEscape = await ev(`return menuItems();`);
    // the same row in the File properties sidebar pane
    r.menuSidebar.open = await ev(`const v = app.commands.executeCommandById('properties:open-local'); await new Promise(r => setTimeout(r, 900)); return { ran: v, leaves: leaves(), paneRows: rows(FP) };`);
    r.menuSidebar.click = await c.rightClickElement(FP + ' .metadata-property[data-property-key="alpha"] .metadata-property-key');
    await sleep(450);
    r.menuSidebar.items = await ev(`return menuItems();`);
    await c.pressKey('Escape');
    await sleep(300);
    // also: hover a reading-view row to see whether any remove affordance appears
    await c.hover(rowSel('alpha'));
    await sleep(300);
    r.menuReading.rowHtmlOnHover = await ev(`return trunc((row('alpha') || {}).outerHTML, 1500);`);
    // remove alpha from the reading view
    r.remove.click = await c.rightClickElement(rowSel('alpha') + ' .metadata-property-key');
    await sleep(450);
    r.remove.pick = await clickMenu(/remove|delete/i);
    await sleep(1500);
    r.remove.bytesAfter = bytes('Note A.md');
    r.remove.diff = lineDiff(r.bytesBefore, r.remove.bytesAfter);
    r.remove.rowsAfter = await ev(`return rows();`);
    r.remove.modal = await ev(`return modalInfo();`);
    if (r.remove.modal) { await c.pressKey('Escape'); await sleep(300); }
    // type icon on zebra -> Number
    r.retype.iconHtml = await ev(`return trunc(($(PV + ' .metadata-property[data-property-key="zebra"] .metadata-property-icon') || {}).outerHTML, 600);`);
    r.retype.click = await c.clickElement(rowSel('zebra') + ' .metadata-property-icon');
    await sleep(450);
    r.retype.items = await ev(`return menuItems();`);
    r.retype.menuHtml = await ev(`return menuHtml();`);
    r.retype.pick = await clickMenu(/^Number$/);
    await sleep(1500);
    r.retype.bytesAfter = bytes('Note A.md');
    r.retype.diff = lineDiff(r.remove.bytesAfter, r.retype.bytesAfter);
    r.retype.zebraRow = await ev(`return rowInfo('zebra');`);
    r.retype.modal = await ev(`return modalInfo();`);
    if (r.retype.modal) { await c.pressKey('Escape'); await sleep(300); }
    r.retype.typeManager = await ev(`const m = app.metadataTypeManager; let info = null; try { info = JSON.parse(JSON.stringify(m.getPropertyInfo ? m.getPropertyInfo('zebra') : null)); } catch (e) { info = 'ERR:' + e; } let types = null; try { types = JSON.parse(JSON.stringify(m.types || null)); } catch (e) { types = 'ERR:' + e; } return { info, types };`);
    r.retype.snapImmediate = snap();
    await sleep(3000);
    r.retype.snapAfter3s = snap();
    // Past Date.md
    r.pastDate.open = await ev(`return await openPreview('Past Date.md');`);
    await sleep(600);
    r.pastDate.dueRow = await ev(`return rowInfo('due');`);
    r.pastDate.rows = await ev(`return rows();`);
    r.pastDate.dailyNotesEnabled = await ev(`return app.internalPlugins.plugins['daily-notes'] ? app.internalPlugins.plugins['daily-notes'].enabled : null;`);
    r.backToA = await ev(`return await openPreview('Note A.md');`);
    r.noteADueRow = await ev(`return rowInfo('due');`);
    return r;
  });

  // 7. folding ---------------------------------------------------------------
  await step('07-fold', async (r) => {
    r.before = await ev(`return foldState();`);
    r.lsBefore = lsObsidian();
    r.triangleClick = await c.clickElement('.mod-root .markdown-reading-view .metadata-container .metadata-properties-heading .collapse-indicator');
    await sleep(600);
    r.afterTriangle = await ev(`return foldState();`);
    r.command = await ev(`const v = app.commands.executeCommandById('editor:toggle-fold-properties'); await new Promise(r => setTimeout(r, 600)); return { ran: v, fold: foldState(), active: act(), leaves: leaves() };`);
    r.commandAgain = await ev(`const v = app.commands.executeCommandById('editor:toggle-fold-properties'); await new Promise(r => setTimeout(r, 600)); return { ran: v, fold: foldState() };`);
    // make sure the block is folded before leaving the note, so the switch and the restart test a fold
    const isFolded = (f) => !!(f && ((f.containerClass || '').includes('is-collapsed') || f.contentDisplay === 'none' || (f.contentRect && f.contentRect.h === 0)));
    r.foldedBeforeSwitch = isFolded(r.commandAgain.fold);
    if (!r.foldedBeforeSwitch) {
      r.triangleClick2 = await c.clickElement('.mod-root .markdown-reading-view .metadata-container .metadata-properties-heading .collapse-indicator');
      await sleep(600);
      r.afterTriangle2 = await ev(`return foldState();`);
      r.foldedBeforeSwitch = isFolded(r.afterTriangle2);
    }
    r.switchToB = await ev(`return await openPreview('Note B.md');`);
    r.foldOnB = await ev(`return foldState();`);
    r.switchBack = await ev(`return await openPreview('Note A.md');`);
    await sleep(400);
    r.foldAfterSwitch = await ev(`return foldState();`);
    r.viewState = await ev(`const l = mainMdLeaf(); return l ? l.getViewState() : null;`);
    await sleep(10000);
    r.snapAfter10s = snap();
    r.localStorageFoldKeys = await ev(`return Object.keys(localStorage).filter(k => /fold|propert/i.test(k)).map(k => ({ k, v: trunc(localStorage.getItem(k), 500) }));`);
    return r;
  });

  // 8. propertiesInDocument modes --------------------------------------------
  await step('08-properties-in-document', async (r) => {
    Object.assign(r, { modes: {} });
    r.before = { config: await ev(`return app.vault.getConfig('propertiesInDocument');`), preview: await ev(`return previewInfo();`), snap: snap() };
    for (const mode of ['source', 'hidden', 'visible']) {
      const m = {};
      m.set = await ev(`app.vault.setConfig('propertiesInDocument', ${JSON.stringify(mode)}); return app.vault.getConfig('propertiesInDocument');`);
      await sleep(1000);
      m.previewAfter1s = await ev(`return previewInfo();`);
      m.foldAfter1s = await ev(`return foldState();`);
      m.appJsonAfter1s = bytes('.obsidian/app.json');
      m.reopen = await ev(`return await openPreview('Note A.md');`);
      await sleep(500);
      m.previewAfterReopen = await ev(`return previewInfo();`);
      m.foldAfterReopen = await ev(`return foldState();`);
      m.paneRowsAfterReopen = await ev(`return { fileProps: rows(FP).length, leaves: leaves().filter(l => l.type === 'file-properties' || l.type === 'all-properties') };`);
      await sleep(7500);
      m.snapAfter10s = snap();
      r.modes[mode] = m;
      save();
    }
    return r;
  });

  // 9. All properties: rename and sort ---------------------------------------
  await step('09-all-properties', async (r) => {
    Object.assign(r, { rename: {}, sort: {} });
    r.leavesBefore = await ev(`return leaves();`);
    r.open = await ev(`const v = app.commands.executeCommandById('properties:open'); await new Promise(r => setTimeout(r, 900)); return { ran: v, leaves: leaves() };`);
    r.pane = await ev(`const p = $(AP); if (!p) return null; return { headerHtml: trunc(($('.nav-header', p) || $('.nav-buttons-container', p) || {}).outerHTML, 2000), buttons: $$('.nav-action-button, .clickable-icon', p).map(b => ({ label: attr(b, 'aria-label'), cls: attr(b, 'class'), rect: rct(b) })), rows: $$('.tree-item', p).map(it => ({ name: ($('.tree-item-inner', it) || {}).textContent, flair: ($('.tree-item-flair', it) || {}).textContent, depth: (function(){ let d = 0, e = it.parentElement; while (e && e !== p) { if (e.classList.contains('tree-item-children')) d++; e = e.parentElement; } return d; })(), hasChildren: !!$('.tree-item-children', it), icon: (function(){ const s = $('svg', it); return s ? attr(s, 'class') : null; })(), cls: attr(it, 'class'), rect: rct(it) })), firstRowHtml: trunc(($('.tree-item', p) || {}).outerHTML, 2000), bookRowsHtml: $$('.tree-item', p).filter(it => /book|shelf/i.test(it.textContent)).map(it => trunc(it.outerHTML, 1500)), width: rct(p) };`);
    r.bytesBefore = { a: bytes('Note A.md'), b: bytes('Note B.md'), c: bytes('Note C.md') };
    const statusIdx = await ev(`return $$(AP + ' .tree-item-self').findIndex(s => /^status$/i.test((($('.tree-item-inner', s) || s).textContent || '').trim()));`);
    r.rename.statusRowIndex = statusIdx;
    r.rename.statusLabel = await ev(`const s = $$(AP + ' .tree-item-self')[${statusIdx}]; return s ? (($('.tree-item-inner', s) || s).textContent || '').trim() : null;`);
    if (statusIdx >= 0) {
      r.rename.rightClick = await c.clickElement(AP + ' .tree-item-self', { index: statusIdx, button: 'right' });
      await sleep(450);
      r.rename.menu = await ev(`return menuItems();`);
      r.rename.menuHtml = await ev(`return menuHtml();`);
      r.rename.pick = await clickMenu(/rename/i);
      await sleep(500);
      r.rename.ui = await ev(`return { active: act(), modal: modalInfo(), rowHtml: trunc(($$(AP + ' .tree-item-self')[${statusIdx}] || {}).outerHTML, 2000), paneText: trunc(($(AP) || {}).innerText, 600) };`);
      r.rename.typed = !!(r.rename.pick && r.rename.pick.found);
      if (r.rename.typed) { await c.pressKey('a', { ctrl: true }); await c.typeText('state'); } // no Rename item: leave the row alone
      await sleep(200);
      r.rename.activeAfterTyping = await ev(`return act();`);
      if (r.rename.typed) await c.pressKey('Enter');
      await sleep(800);
      r.rename.afterEnter = await ev(`return { active: act(), modal: modalInfo(), paneText: trunc(($(AP) || {}).innerText, 600) };`);
      if (r.rename.afterEnter.modal) {
        const btns = r.rename.afterEnter.modal.buttons || [];
        const bi = btns.findIndex((b) => /rename|confirm|yes|ok|apply|continue/i.test(b.text));
        r.rename.confirmButtonIndex = bi;
        if (bi >= 0) { r.rename.confirmClick = await c.clickElement('.modal-container button', { index: bi }); await sleep(1500); }
        else { await c.pressKey('Escape'); await sleep(400); r.rename.dismissedUnknownModal = true; }
        r.rename.modalAfter = await ev(`return modalInfo();`);
      }
      await sleep(1500);
      r.rename.bytesAfter = { a: bytes('Note A.md'), b: bytes('Note B.md'), c: bytes('Note C.md') };
      r.rename.diff = { a: lineDiff(r.bytesBefore.a, r.rename.bytesAfter.a), b: lineDiff(r.bytesBefore.b, r.rename.bytesAfter.b), c: lineDiff(r.bytesBefore.c, r.rename.bytesAfter.c) };
      r.rename.rowsAfter = await ev(`return $$(AP + ' .tree-item').map(it => (($('.tree-item-inner', it) || {}).textContent || '') + ' ' + (($('.tree-item-flair', it) || {}).textContent || ''));`);
      r.rename.readingRows = await ev(`return rows();`);
      r.rename.viewFields = await ev(`const l = app.workspace.getLeavesOfType('all-properties')[0]; if (!l) return null; const v = l.view; return { propertyBeingRenamed: v.propertyBeingRenamed === undefined ? 'undefined' : String(v.propertyBeingRenamed), propertyRenameScope: v.propertyRenameScope === undefined ? 'undefined' : String(v.propertyRenameScope), keys: Object.keys(v).filter(k => /rename|sort|scope|query/i.test(k)) };`);
    }
    // sort
    r.sort.lsBefore = lsObsidian();
    r.sort.viewStateBefore = await ev(`const l = app.workspace.getLeavesOfType('all-properties')[0]; return l ? l.getViewState() : null;`);
    r.sort.click = await c.clickElement(AP + ' [aria-label="Change sort order"]');
    if (!r.sort.click) { r.sort.click = await c.clickElement(AP + ' .nav-action-button'); r.sort.clickedFallback = 'first .nav-action-button'; }
    await sleep(450);
    r.sort.menu = await ev(`return menuItems();`);
    r.sort.pick = await clickMenu(/Property name \(A to Z\)/);
    await sleep(700);
    r.sort.rowsAfter = await ev(`return $$(AP + ' .tree-item').map(it => ({ name: ($('.tree-item-inner', it) || {}).textContent, flair: ($('.tree-item-flair', it) || {}).textContent, depth: (function(){ let d = 0, e = it.parentElement; while (e && !e.classList.contains('workspace-leaf-content')) { if (e.classList.contains('tree-item-children')) d++; e = e.parentElement; } return d; })() }));`);
    r.sort.viewStateAfter = await ev(`const l = app.workspace.getLeavesOfType('all-properties')[0]; return l ? l.getViewState() : null;`);
    r.sort.viewSortField = await ev(`const l = app.workspace.getLeavesOfType('all-properties')[0]; if (!l) return null; const v = l.view; return { sortOrder: v.sortOrder === undefined ? 'undefined' : String(v.sortOrder) }; `);
    await sleep(6000);
    r.sort.lsAfter6s = lsObsidian();
    r.sort.workspaceJsonAfter6s = bytes('.obsidian/workspace.json');
    r.sort.appJsonAfter6s = bytes('.obsidian/app.json');
    return r;
  });

  // 10. pane placement, Plain.txt, search filter ------------------------------
  await step('10-placement-plain-filter', async (r) => {
    Object.assign(r, { close: {}, reopen: {}, plain: {}, filter: {} });
    r.close.tabHeader = await ev(`const t = $('.workspace-tab-header[data-type="all-properties"]'); return desc(t, 800);`);
    r.close.rightClick = await c.rightClickElement('.workspace-tab-header[data-type="all-properties"]');
    await sleep(450);
    r.close.menu = await ev(`return menuItems();`);
    r.close.pick = await clickMenu(/^Close$/);
    await sleep(600);
    r.close.leavesAfter = await ev(`return leaves();`);
    r.reopen.run = await ev(`const v = app.commands.executeCommandById('properties:open'); await new Promise(r => setTimeout(r, 900)); const l = app.workspace.getLeavesOfType('all-properties')[0]; return { ran: v, leaves: leaves(), placement: l ? { split: splitName(l.getRoot()), leafRect: rct(l.containerEl), rightSplitRect: rct(app.workspace.rightSplit.containerEl), leftSplitRect: rct(app.workspace.leftSplit.containerEl), rootSplitRect: rct(app.workspace.rootSplit.containerEl), rightCollapsed: app.workspace.rightSplit.collapsed, innerWidth: innerWidth, widthFromRightEdge: innerWidth - rct(l.containerEl).x, tabHeaderHtml: trunc(($('.workspace-tab-header[data-type="all-properties"]') || {}).outerHTML, 600) } : null };`);
    // Plain.txt in the file explorer
    r.plain.showUnsupportedFiles = await ev(`return app.vault.getConfig('showUnsupportedFiles');`);
    r.plain.explorer = await ev(`const l = app.workspace.getLeavesOfType('file-explorer')[0]; if (l) await app.workspace.revealLeaf(l); await new Promise(r => setTimeout(r, 500)); const e = $('.nav-file-title[data-path="Plain.txt"]'); if (e) e.scrollIntoView(); return { explorerLeaf: !!l, plainVisible: !!e, titles: $$('.nav-file-title').map(t => attr(t, 'data-path')) };`);
    if (!r.plain.explorer.plainVisible) {
      r.plain.setShowUnsupported = await ev(`app.vault.setConfig('showUnsupportedFiles', true); await new Promise(r => setTimeout(r, 900)); const e = $('.nav-file-title[data-path="Plain.txt"]'); if (e) e.scrollIntoView(); return { plainVisible: !!e, titles: $$('.nav-file-title').map(t => attr(t, 'data-path')) };`);
    }
    r.plain.leavesBefore = await ev(`return leaves();`);
    r.plain.click = await c.clickElement('.nav-file-title[data-path="Plain.txt"]');
    await sleep(1500);
    r.plain.after = await ev(`return { leaves: leaves(), activeFile: (function(){ const f = app.workspace.getActiveFile(); return f ? f.path : null; })(), activeLeafType: app.workspace.activeLeaf ? app.workspace.activeLeaf.view.getViewType() : null, active: act(), modal: modalInfo(), filePropsPane: (function(){ const p = $(FP); return p ? { text: trunc(p.innerText, 400), rows: rows(FP), addButton: desc($('.metadata-add-button', p), 400), emptyState: $$('.metadata-properties-empty, .pane-empty, .metadata-empty', p).map(e => desc(e, 300)), html: trunc(($('.view-content', p) || p).outerHTML, 2500) } : null; })(), explorerSelected: $$('.nav-file-title.is-active, .nav-file-title.is-selected').map(t => attr(t, 'data-path')) };`);
    // search filter in the All properties header
    r.filter.click = await c.clickElement(AP + ' [aria-label="Show search filter"]');
    await sleep(450);
    r.filter.after = await ev(`const p = $(AP); return { active: act(), searchContainer: p ? desc($('.search-input-container', p), 800) : null };`);
    r.filter.typed = !!(r.filter.click && !r.filter.click.skipped);
    if (r.filter.typed) await c.typeText('zeb'); // the magnifier was not found: do not type into the explorer or a note
    await sleep(700);
    r.filter.rowsAfterTyping = await ev(`return { active: act(), rows: $$(AP + ' .tree-item').map(it => ({ name: ($('.tree-item-inner', it) || {}).textContent, flair: ($('.tree-item-flair', it) || {}).textContent, display: getComputedStyle(it).display, rect: rct(it) })), inputValue: (function(){ const i = $(AP + ' .search-input-container input'); return i ? i.value : null; })() };`);
    await c.pressKey('Escape');
    await sleep(300);
    r.filter.afterEscape = await ev(`const p = $(AP); return { active: act(), searchContainer: p ? desc($('.search-input-container', p), 300) : null, rowCount: $$(AP + ' .tree-item').length };`);
    r.snap = snap();
    return r;
  });

  // pre-quit: Note A open in reading mode, record state, then quit ------------
  await step('11-pre-quit', async (r) => {
    r.open = await ev(`return await openPreview('Note A.md');`);
    await sleep(500);
    r.fold = await ev(`return foldState();`);
    r.leaves = await ev(`return leaves();`);
    r.viewState = await ev(`const l = mainMdLeaf(); return l ? l.getViewState() : null;`);
    r.bytes = { a: bytes('Note A.md'), b: bytes('Note B.md'), c: bytes('Note C.md'), dotted: bytes('Dotted.md'), nested: bytes('Nested.md'), pastDate: bytes('Past Date.md') };
    r.snap = snap();
    r.workspaceJsonBeforeQuit = bytes('.obsidian/workspace.json');
    return r;
  });
  save();
  try { await c.quit(); out.quit = 'ok'; } catch (e) { out.quit = 'ERR:' + String(e); }
  out.lsAfterQuit = lsObsidian();
  out.workspaceJsonAfterQuit = bytes('.obsidian/workspace.json');
  save();
  process.exit(0);
}

// ============================================================================
if (phase === 'after-restart') {
  await step('r0-disk-before-connect', async () => ({ ...snap(), workspaceJson: bytes('.obsidian/workspace.json'), noteA: bytes('Note A.md') }));
  await waitForObsidian(90000);
  await sleep(3000);
  c = await connect();
  await step('r1-after-restart', async (r) => {
    r.ready = await waitReady();
    await sleep(2000);
    r.leaves = await ev(`return leaves();`);
    r.modal = await ev(`return modalInfo();`);
    if (r.modal) { await c.pressKey('Escape'); await sleep(300); }
    r.noteALeaf = await ev(`const l = app.workspace.getLeavesOfType('markdown').find(l => l.view.file && l.view.file.path === 'Note A.md'); return l ? { mode: l.view.getMode ? l.view.getMode() : null, split: splitName(l.getRoot()), state: l.getViewState() } : null;`);
    r.foldAsRestored = await ev(`return foldState();`);
    r.previewAsRestored = await ev(`return previewInfo();`);
    if (!r.noteALeaf || r.noteALeaf.mode !== 'preview' || !r.foldAsRestored) {
      r.reopened = await ev(`return await openPreview('Note A.md');`);
      await sleep(500);
      r.foldAfterReopen = await ev(`return foldState();`);
    }
    r.allPropertiesLeaf = await ev(`const l = app.workspace.getLeavesOfType('all-properties')[0]; return l ? { split: splitName(l.getRoot()), rect: rct(l.containerEl), state: l.getViewState(), rows: $$(AP + ' .tree-item').map(it => (($('.tree-item-inner', it) || {}).textContent || '') + ' ' + (($('.tree-item-flair', it) || {}).textContent || '')) } : null;`);
    r.typeManager = await ev(`const m = app.metadataTypeManager; let info = null; try { info = JSON.parse(JSON.stringify(m.getPropertyInfo ? m.getPropertyInfo('zebra') : null)); } catch (e) { info = 'ERR:' + e; } let types = null; try { types = JSON.parse(JSON.stringify(m.types || null)); } catch (e) { types = 'ERR:' + e; } return { info, types };`);
    r.localStorageFoldKeys = await ev(`return Object.keys(localStorage).filter(k => /fold|propert/i.test(k)).map(k => ({ k, v: trunc(localStorage.getItem(k), 500) }));`);
    r.snap = snap();
    const ws = bytes('.obsidian/workspace.json');
    r.workspaceJson = ws;
    try {
      const found = [];
      const walk = (o) => { if (!o || typeof o !== 'object') return; if (o.type === 'leaf' && o.state && (o.state.type === 'all-properties' || o.state.type === 'file-properties' || (o.state.state && o.state.state.file === 'Note A.md'))) found.push(o); for (const v of Object.values(o)) walk(v); };
      walk(JSON.parse(ws));
      r.workspaceLeavesOfInterest = found;
    } catch (e) { r.workspaceLeavesOfInterest = 'ERR:' + e.message; }
    r.bytes = { a: bytes('Note A.md'), b: bytes('Note B.md'), c: bytes('Note C.md'), dotted: bytes('Dotted.md'), nested: bytes('Nested.md'), pastDate: bytes('Past Date.md') };
    return r;
  });
  save();
  try { c.close(); } catch {}
  process.exit(0);
}