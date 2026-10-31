// The right dock's Tags, All properties and Outline panes driven over CDP:
// each pane's whole markup with computed styles and icons, the leaf's view
// state, the nav-header buttons and their tooltips, the sort menu and the row
// order under every entry, the in-pane search filter under real keystrokes,
// hover and click on a row and a header button, the Tags pane's nested-tags
// toggle (how a/b is drawn either way, parent counts, fold triangles), the
// Outline's follow-cursor marking in source and reading mode, its empty state
// and what switching notes does, and the leaf state each pane writes to
// workspace.json.
// Phases: fixtures | run | after-restart   (node dock-panes.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
const SHOTS = process.env.SHOTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 470000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.dockPanes = all.dockPanes || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.dockPanes[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
async function step(name, fn) {
  out.order.push(name);
  if (Date.now() - T0 > BUDGET_MS) { out.steps[name] = { ok: false, error: 'skipped: time budget exhausted' }; save(); return null; }
  const t = Date.now();
  const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save();
  return out.steps[name];
}
const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };

const TYPES = ['tag', 'all-properties', 'outline'];

if (phase === 'fixtures') {
  const filler = (label, n) => Array.from({ length: n }, (_, i) => `${label} body line ${i + 1} with some words so the reading view has height.`).join('\n');
  fs.writeFileSync(vp('Alpha.md'), [
    '---', 'tags:', '  - project/alpha', '  - common', 'title: Alpha note', 'Status: draft', '---',
    '# Alpha', '', 'Inline #common here and #project/alpha again. Links to [[Beta]] and [[Outline]].', '',
  ].join('\n'));
  fs.writeFileSync(vp('Beta.md'), [
    '---', 'tags: [project/beta, common]', 'status: review', 'count: 7', '---',
    '# Beta', '', 'A capitalised #Project tag and #common once more, and #Lambda capitalised.', '',
  ].join('\n'));
  fs.writeFileSync(vp('Gamma.md'), [
    '---', 'STATUS: final', 'title: Gamma', '---',
    '# Gamma', '', 'Deep nesting #project/beta/deep, a #topic tag, #lambda in lower case and #common.', '',
  ].join('\n'));
  fs.writeFileSync(vp('Delta.md'), '# Delta\n\n#common twice in one note: #common.\n');
  fs.writeFileSync(vp('Epsilon.md'), '# Epsilon\n\nCased #Common and plain #common, and #Kappa only ever capitalised.\n');
  fs.writeFileSync(vp('Props.md'), [
    '---',
    'title: Props note',
    'summary: A plain text value',
    'list:', '  - one', '  - two',
    'count: 42',
    'done: true',
    'due: 2026-10-22',
    'when: 2026-10-22T09:30:00',
    'tags:', '  - project/alpha', '  - meta',
    'aliases:', '  - PropsAlias',
    'cssclasses:', '  - wide',
    'shelf:', '  row: 1', '  label: top',
    'book.title: Dune',
    'book.year: 1965',
    'status: draft',
    '---',
    '# Props', '', 'Every property type lives in this note.', '',
  ].join('\n'));
  fs.writeFileSync(vp('Outline.md'), [
    '---', 'tags:', '  - single', '---',
    '# Top heading', filler('top', 14),
    '## Second level', filler('second', 14),
    '### Third level under second', filler('third', 14),
    '#### Fourth level', filler('fourth', 14),
    '## Another second', filler('another', 14),
    '#### Fourth under second, skipping third', filler('skip', 14),
    '# Second top', filler('second top', 14),
    '### Third under top, skipping second', filler('third skip', 14),
    '## Heading with **bold**, *italic*, `code`, [[Alpha]] and a #topic tag', filler('format', 14),
    '## A very long heading that goes on and on so that the outline pane must either wrap it onto several lines or ellipsize it at the three hundred pixel width of the sidebar', filler('long', 14),
    '# Last top', filler('last', 30),
    ...Array.from({ length: 24 }, (_, i) => `## Extra ${i + 1}\n${filler('extra ' + (i + 1), 3)}`),
  ].join('\n') + '\n');
  fs.writeFileSync(vp('NoHeadings.md'), 'A note with no headings at all, tagged #a once.\n\nSecond paragraph.\n');
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('.obsidian/types.json'), JSON.stringify({ types: { summary: 'text', list: 'multitext', count: 'number', done: 'checkbox', due: 'date', when: 'datetime' } }, null, 2) + '\n');
  out.files = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}

if (phase === 'after-restart') {
  const ws = JSON.parse(bytes('.obsidian/workspace.json'));
  const leaves = [];
  const walk = (n) => { if (!n || typeof n !== 'object') return; if (n.type === 'leaf') leaves.push(n.state); for (const k of ['children']) if (Array.isArray(n[k])) n[k].forEach(walk); };
  for (const k of ['left', 'right', 'main']) walk(ws[k]);
  out.leafStates = leaves.filter(l => l && TYPES.includes(l.type));
  out.rightSplit = { collapsed: ws.right && ws.right.collapsed, width: ws.right && ws.right.width };
  out.workspaceKeys = Object.keys(ws);
  save();
  const c2 = await connect();
  out.restoredBeforeActivation = await c2.evaluate(`const o = {}; for (const t of ${JSON.stringify(TYPES)}) { const l = app.workspace.getLeavesOfType(t)[0]; o[t] = l && { viewState: l.getViewState(), rows: l.view.containerEl.querySelectorAll('.tree-item-self').length }; } return o;`);
  out.restored = {};
  for (const [t, cmd] of [['tag', 'tag-pane:open'], ['all-properties', 'properties:open'], ['outline', 'outline:open']]) {
    await c2.evaluate(`await app.commands.executeCommandById(${JSON.stringify(cmd)}); return true;`);
    await sleep(1200);
    out.restored[t] = await c2.evaluate(`const l = app.workspace.getLeavesOfType(${JSON.stringify(t)})[0]; const sc = l.view.containerEl.querySelector('.search-input-container'); return l && { viewState: l.getViewState(), display: l.getDisplayText(), searchBox: sc && { display: getComputedStyle(sc).display, value: sc.querySelector('input').value }, buttons: Array.from(l.view.containerEl.querySelectorAll('.nav-action-button')).map(b => ({ label: b.getAttribute('aria-label'), cls: b.className })), rows: Array.from(l.view.containerEl.querySelectorAll('.tree-item')).map(it => { const r = it.querySelector(':scope > .tree-item-self'); const b = r && r.getBoundingClientRect(); return { itemCls: it.className, cls: r && r.className, text: r && r.textContent.trim().slice(0, 40), h: b && Math.round(b.height) }; }) };`);
  }
  await c2.quit();
  save();
  process.exit(0);
}

const c = await connect();
const PROBE_JS = `
const cs = (el, props) => { const s = getComputedStyle(el); const o = {}; for (const p of props) o[p] = s.getPropertyValue(p); return o; };
const PROPS = ['display','font-size','font-weight','line-height','color','background-color','padding','margin','height','min-height','border-radius','border','opacity','cursor','white-space','overflow','text-overflow','gap','align-items','justify-content','flex','width','text-decoration','font-style','position','top','right','left','bottom','flex-direction','flex-wrap','word-break','overflow-wrap','-webkit-line-clamp','padding-inline-start'];
const rect = (el) => { const r = el.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; };
function dump(el, depth) {
  if (depth > 26) return { tag: el.tagName, truncated: true };
  const tag = el.tagName.toLowerCase();
  const cls = el.className && el.className.baseVal !== undefined ? el.className.baseVal : el.className;
  const o = { tag, cls, rect: rect(el) };
  for (const a of Array.from(el.attributes)) if (a.name !== 'class') o['@' + a.name] = a.value;
  const own = Array.from(el.childNodes).filter(n => n.nodeType === 3).map(n => n.textContent).join('').trim(); if (own) o.text = own;
  if (tag === 'svg') { o.style = cs(el, ['width','height','color','stroke','opacity']); return o; }
  o.style = cs(el, PROPS);
  if (tag === 'input') o.value = el.value;
  const kids = Array.from(el.children); if (kids.length) o.children = kids.map(k => dump(k, depth + 1));
  return o;
}
const leafOf = (t) => app.workspace.getLeavesOfType(t)[0];
const paneOf = (t) => leafOf(t).view.containerEl;
const vars = (el, names) => { const s = getComputedStyle(el); const o = {}; for (const n of names) o[n] = s.getPropertyValue(n).trim(); return o; };
const depthOf = (el) => { let d = 0, p = el.parentElement; while (p) { if (p.classList.contains('tree-item-children')) d++; p = p.parentElement; } return d; };
const rows = (t) => Array.from(paneOf(t).querySelectorAll('.tree-item')).map(it => {
  const self = it.querySelector(':scope > .tree-item-self');
  const inner = self && self.querySelector('.tree-item-inner');
  const flair = self && self.querySelector('.tree-item-flair');
  const ci = self && self.querySelector('.collapse-icon');
  const icons = self ? Array.from(self.querySelectorAll('svg')).map(s => s.getAttribute('class')) : [];
  const o = { itemCls: it.className, selfCls: self && self.className, depth: depthOf(it), text: inner && inner.textContent.trim(), flair: flair && flair.textContent.trim(), icons,
    rect: self && rect(self), innerRect: inner && rect(inner), selfStyle: self && cs(self, ['padding','padding-inline-start','height','color','background-color','font-size','font-weight','border-radius','margin','opacity','display','align-items']),
    innerStyle: inner && cs(inner, ['color','font-size','font-weight','white-space','overflow','text-overflow','word-break','-webkit-line-clamp','display','flex','width','line-height','padding','margin']),
    flairStyle: flair && cs(flair, ['color','font-size','font-weight','background-color','border-radius','padding','margin','opacity']),
    collapseIcon: ci && { cls: ci.className, rect: rect(ci), display: getComputedStyle(ci).display, opacity: getComputedStyle(ci).opacity, transform: getComputedStyle(ci).transform, margin: getComputedStyle(ci).margin, color: getComputedStyle(ci).color, svg: ci.querySelector('svg') && { cls: ci.querySelector('svg').getAttribute('class'), rect: rect(ci.querySelector('svg')) } },
    attrs: {}, childrenRect: null };
  if (self) for (const a of Array.from(self.attributes)) if (a.name !== 'class' && a.name !== 'style') o.attrs[a.name] = a.value;
  if (self && self.getAttribute('style')) o.attrs.style = self.getAttribute('style');
  for (const a of Array.from(it.attributes)) if (a.name !== 'class') o.attrs['item:' + a.name] = a.value;
  const kids = it.querySelector(':scope > .tree-item-children'); if (kids) o.childrenRect = rect(kids), o.childrenStyle = cs(kids, ['margin','padding','border-left','display','margin-left','padding-left']), o.childCount = kids.querySelectorAll(':scope > .tree-item').length;
  const sub = inner && inner.querySelector('.tree-item-inner-subtext'); if (sub) o.subtext = sub.textContent.trim();
  return o;
});
const headerButtons = (t) => Array.from(paneOf(t).querySelectorAll('.nav-header .nav-action-button, .nav-buttons-container .clickable-icon')).map(b => ({ cls: b.className, label: b.getAttribute('aria-label'), icon: (b.querySelector('svg') || { getAttribute: () => null }).getAttribute('class'), rect: rect(b), style: cs(b, ['color','background-color','padding','border-radius','opacity','width','height','margin']) }));
const tooltip = () => Array.from(document.querySelectorAll('.tooltip')).map(t => ({ cls: t.className, text: t.textContent.trim(), rect: rect(t), style: cs(t, ['font-size','background-color','color','padding','border-radius']) }));
const menuDom = () => Array.from(document.querySelectorAll('.menu')).map(m => ({ cls: m.className, rect: rect(m), items: Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map(i => ({ cls: i.className, text: (i.querySelector('.menu-item-title') || i).textContent.trim(), icon: (i.querySelector('svg') || { getAttribute: () => null }).getAttribute('class'), rect: rect(i) })) }));
const searchState = () => { const l = app.workspace.getLeavesOfType('search')[0]; if (!l) return null; const inp = l.view.containerEl.querySelector('input'); return { viewState: l.getViewState(), inputValue: inp && inp.value, isActive: app.workspace.activeLeaf === l, leftCollapsed: app.workspace.leftSplit.collapsed, resultCount: l.view.containerEl.querySelectorAll('.search-result').length, rect: rect(l.view.containerEl), count: (l.view.containerEl.querySelector('.search-results-info') || {}).textContent }; };
const mdLeaf = () => app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit) || app.workspace.getLeavesOfType('markdown')[0];
const editorState = () => { const l = mdLeaf(); const v = l && l.view; if (!v) return null; return { file: v.file && v.file.path, mode: v.getMode && v.getMode(), cursor: v.editor && v.editor.getCursor(), scrollTop: v.editor && v.editor.getScrollInfo && v.editor.getScrollInfo().top, previewScroll: v.previewMode && v.previewMode.getScroll && v.previewMode.getScroll(), ephemeral: l.getEphemeralState && l.getEphemeralState(), activeLeafType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType(), activeEl: document.activeElement && (document.activeElement.tagName + '.' + document.activeElement.className) }; };
const marked = (t) => rows(t).filter(r => r.selfCls && /active|selected|focus|current|has-/.test(r.selfCls) || r.itemCls && /active|selected|current/.test(r.itemCls)).map(r => ({ text: r.text, selfCls: r.selfCls, itemCls: r.itemCls, selfStyle: r.selfStyle, innerStyle: r.innerStyle }));
const paneInfo = (t) => { const l = leafOf(t); return { viewState: l.getViewState(), display: l.getDisplayText(), tabHeader: l.tabHeaderEl && { cls: l.tabHeaderEl.className, label: l.tabHeaderEl.getAttribute('aria-label'), icon: l.tabHeaderEl.querySelector('svg') && l.tabHeaderEl.querySelector('svg').getAttribute('class'), title: (l.tabHeaderEl.querySelector('.workspace-tab-header-inner-title') || {}).textContent }, viewKeys: Object.keys(l.view).filter(k => !/El$|Component|^(app|leaf|containerEl|scope|navigation)$/.test(k)) }; };
`;

async function ensureRowsVisible(t) {
  const check = () => c.evaluate(PROBE_JS + `const rs = rows(${JSON.stringify(t)}); return { total: rs.length, hidden: rs.filter(r => !r.rect || !r.rect[2]).length, viewState: leafOf(${JSON.stringify(t)}).getViewState().state };`);
  let st = await check();
  const log = [{ before: st }];
  if (st.hidden === st.total && st.viewState.showSearch) {
    await clickButton(t, 'Show search filter');
    st = await check(); log.push({ afterHidingSearch: st });
  }
  return log;
}
const btnSel = (t, label) => `.workspace-leaf-content[data-type="${t}"] .nav-action-button[aria-label="${label}"]`;
async function clickButton(t, label) {
  const r = await c.clickElement(btnSel(t, label));
  if (!r) throw new Error('no header button ' + label + ' in ' + t);
  if (r.skipped) throw new Error('header button ' + label + ' in ' + t + ' has no size (pane hidden?)');
  await sleep(500);
  return r;
}
async function clickMenuItem(label) {
  if (label === undefined) throw new Error('clickMenuItem: no label (menu did not open?)');
  const r = await c.evaluate(`const items = Array.from(document.querySelectorAll('.menu .menu-item')); const it = items.find(i => ((i.querySelector('.menu-item-title') || i).textContent || '').trim() === ${JSON.stringify(label)}); if (!it) return null; const b = it.getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2, w: b.width, h: b.height };`);
  if (!r) throw new Error('no menu item ' + label);
  await c.clickAt(r.x, r.y);
  await sleep(600);
  return r;
}
async function hoverButton(t, label) {
  const r = await c.hover(btnSel(t, label));
  await sleep(1000);
  return { pointer: r, ...(await c.evaluate(PROBE_JS + `const b = document.querySelector(${JSON.stringify(btnSel(t, label))}); return { hovered: b.matches(':hover'), cls: b.className, style: cs(b, ['color','background-color','border-radius','padding','opacity']), tooltip: tooltip() };`)) };
}
async function shot(name) {
  if (!SHOTS) return null;
  try {
    fs.mkdirSync(SHOTS, { recursive: true });
    const r = await c.send('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(path.join(SHOTS, name + '.png'), Buffer.from(r.data, 'base64'));
    return name + '.png';
  } catch (e) { return 'ERR:' + e.message; }
}
const VARS = ['--nav-item-size','--nav-item-color','--nav-item-weight','--nav-item-padding-y','--nav-item-padding-x','--nav-item-background-hover','--nav-item-color-hover','--nav-item-background-active','--nav-item-color-active','--nav-item-weight-active','--nav-item-weight-hover','--nav-item-children-padding-start','--nav-item-children-margin-start','--nav-indentation-guide-width','--nav-indentation-guide-color','--nav-collapse-icon-color','--nav-collapse-icon-color-collapsed','--nav-heading-color','--nav-heading-weight','--text-muted','--text-faint','--text-normal','--text-accent','--font-ui-small','--font-ui-smaller','--font-ui-medium','--size-4-1','--size-4-2','--size-4-3','--background-primary','--background-secondary','--background-modifier-border','--background-modifier-hover','--background-modifier-active-hover','--text-highlight-bg','--radius-s','--radius-m','--tag-size','--tag-color','--tag-background','--tag-padding-x','--tag-padding-y','--tag-radius','--tag-weight','--tag-decoration','--tag-color-hover','--tag-background-hover','--icon-size','--icon-s','--icon-color','--icon-color-hover','--icon-color-active','--icon-opacity','--icon-opacity-hover','--icon-opacity-active','--clickable-icon-radius','--interactive-accent','--input-height','--input-radius','--metadata-label-text-color'];

// ---------------------------------------------------------------------------
// Tags pane
// ---------------------------------------------------------------------------
await step('s01-open-tags', async () => {
  await c.evaluate(`
    const f = app.vault.getAbstractFileByPath('Alpha.md');
    await app.workspace.getLeaf(false).openFile(f);
    app.workspace.rightSplit.expand();
    await app.commands.executeCommandById('tag-pane:open');
    return true;`);
  await sleep(1500);
  return c.evaluate(PROBE_JS + `
    const leaves = {}; app.workspace.iterateAllLeaves(l => { leaves[l.view.getViewType()] = (leaves[l.view.getViewType()] || 0) + 1; });
    return { leaves, active: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, rightWidth: app.workspace.rightSplit.containerEl.getBoundingClientRect().width, info: paneInfo('tag'),
      commands: ['tag-pane:open','properties:open','outline:open'].map(id => ({ id, name: app.commands.commands[id] && app.commands.commands[id].name })),
      tagCounts: app.metadataCache.getTags && app.metadataCache.getTags(), tagCache: Object.entries(app.metadataCache.getCache ? {} : {}) };`);
});

await step('s02-tags-dom', async () => ({ shot: await shot('tags-default'), ...(await c.evaluate(PROBE_JS + `const pane = paneOf('tag'); return { dom: dump(pane, 0), vars: vars(pane, ${JSON.stringify(VARS)}), rows: rows('tag'), headers: headerButtons('tag'), viewState: leafOf('tag').getViewState() };`)) }));

await step('s03-tags-hover-row', async () => {
  const r = await c.hover('.workspace-leaf-content[data-type="tag"] .tree-item-self', 1);
  await sleep(500);
  return { pointer: r, ...(await c.evaluate(PROBE_JS + `
    const p = paneOf('tag'); const r = p.querySelectorAll('.tree-item-self')[1]; const inner = r.querySelector('.tree-item-inner'); const flair = r.querySelector('.tree-item-flair'); const ci = r.querySelector('.collapse-icon');
    return { hovered: r.matches(':hover'), text: r.textContent.trim(), cls: r.className, style: cs(r, ['background-color','color','border-radius']), inner: inner && cs(inner, ['color','text-decoration']), flair: flair && cs(flair, ['color','background-color']), collapseIcon: ci && { display: getComputedStyle(ci).display, opacity: getComputedStyle(ci).opacity, color: getComputedStyle(ci).color }, tooltip: tooltip() };`)) };
});

await step('s04-tags-hover-buttons', async () => {
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('tag').map(b => b.label);`);
  const o = { labels };
  for (const l of labels) o[l] = await hoverButton('tag', l);
  return o;
});

await step('s05-tags-sort-menu', async () => {
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('tag').map(b => b.label);`);
  const sortLabel = labels.find(l => /sort/i.test(l));
  if (!sortLabel) return { skipped: 'no sort button', labels };
  const o = { sortLabel };
  o.clicked = await clickButton('tag', sortLabel);
  o.items = await c.openMenuItems();
  o.menuDom = await c.evaluate(PROBE_JS + `return menuDom();`);
  await c.pressKey('Escape'); await sleep(300);
  o.orders = {};
  const names = o.items.flat().filter(x => x !== '---');
  for (const name of names) {
    await clickButton('tag', sortLabel);
    await clickMenuItem(name);
    o.orders[name] = await c.evaluate(PROBE_JS + `return { viewState: leafOf('tag').getViewState(), rows: rows('tag').map(r => ({ depth: r.depth, text: r.text, flair: r.flair })), checked: null };`);
    await clickButton('tag', sortLabel);
    o.orders[name].menuAfter = await c.evaluate(PROBE_JS + `return menuDom().map(m => m.items.map(i => ({ text: i.text, cls: i.cls, icon: i.icon })));`);
    await c.pressKey('Escape'); await sleep(300);
  }
  // Back to the first (default) entry.
  await clickButton('tag', sortLabel); await clickMenuItem(names[0]);
  o.restored = await c.evaluate(PROBE_JS + `return leafOf('tag').getViewState();`);
  return o;
});

await step('s06-tags-search', async () => {
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('tag').map(b => b.label);`);
  const l = labels.find(x => /search/i.test(x));
  if (!l) return { skipped: 'no search button', labels };
  const o = {};
  o.before = await c.evaluate(PROBE_JS + `const sc = paneOf('tag').querySelector('.search-input-container'); return { container: sc && dump(sc, 0), viewState: leafOf('tag').getViewState() };`);
  o.clicked = await clickButton('tag', l);
  o.shown = await c.evaluate(PROBE_JS + `const sc = paneOf('tag').querySelector('.search-input-container'); return { container: sc && dump(sc, 0), focused: document.activeElement && document.activeElement.outerHTML.slice(0, 200), viewState: leafOf('tag').getViewState(), button: headerButtons('tag').find(b => b.label === ${JSON.stringify(l)}) };`);
  await c.typeText('proj');
  await sleep(600);
  o.typed = await c.evaluate(PROBE_JS + `return { rows: rows('tag').map(r => ({ depth: r.depth, text: r.text, flair: r.flair, itemCls: r.itemCls, selfCls: r.selfCls })), viewState: leafOf('tag').getViewState(), inputValue: paneOf('tag').querySelector('input').value, empty: Array.from(paneOf('tag').querySelectorAll('.search-empty-state, .pane-empty')).map(e => e.textContent) };`);
  await c.pressKey('Backspace'); await c.pressKey('Backspace'); await c.pressKey('Backspace'); await c.pressKey('Backspace');
  await c.typeText('zzz'); await sleep(500);
  o.noMatch = await c.evaluate(PROBE_JS + `return { rows: rows('tag').map(r => r.text), viewState: leafOf('tag').getViewState(), empty: Array.from(paneOf('tag').querySelectorAll('.search-empty-state, .pane-empty')).map(e => ({ cls: e.className, text: e.textContent, style: cs(e, ['color','font-size','padding','margin']) })), dom: dump(paneOf('tag').querySelector('.view-content') || paneOf('tag'), 0) };`);
  o.shotNoMatch = await shot('tags-search-nomatch');
  const readSearch = () => c.evaluate(PROBE_JS + `const sc = paneOf('tag').querySelector('.search-input-container'); return { containerRect: sc && rect(sc), containerDisplay: sc && getComputedStyle(sc).display, inputValue: sc && sc.querySelector('input').value, rows: rows('tag').map(r => [r.text, r.itemCls, r.rect && r.rect[3]]), viewState: leafOf('tag').getViewState().state, activeEl: document.activeElement && (document.activeElement.tagName + '.' + document.activeElement.className).slice(0, 60), clearButton: (() => { const b = sc && sc.querySelector('.search-input-clear-button'); return b && { rect: rect(b), display: getComputedStyle(b).display }; })() };`);
  await c.pressKey('Escape'); await sleep(150);
  o.escaped1 = await readSearch();
  await sleep(1200);
  o.escaped1Later = await readSearch();
  await c.pressKey('Escape'); await sleep(400);
  o.escaped2 = await readSearch();
  // Type again, then use the clear button instead of Escape.
  if (o.escaped2.viewState.showSearch) {
    const inp = await c.rect('.workspace-leaf-content[data-type="tag"] .search-input-container input');
    if (inp && inp.w) { await c.clickAt(inp.cx, inp.cy); await c.typeText('zz'); await sleep(400); }
    o.retyped = await readSearch();
    const cb = await c.rect('.workspace-leaf-content[data-type="tag"] .search-input-clear-button');
    if (cb && cb.w) { await c.clickAt(cb.cx, cb.cy); await sleep(500); o.clearClicked = await readSearch(); }
    // Then hide the box with its header button.
    await clickButton('tag', l);
    o.hiddenByButton = await readSearch();
  }
  return o;
});

await step('s07-tags-click-row', async () => {
  const guard = await ensureRowsVisible('tag');
  const rowsNow = await c.evaluate(PROBE_JS + `return rows('tag').map(r => ({ text: r.text, depth: r.depth }));`);
  const o = { guard, rowsNow, before: await c.evaluate(PROBE_JS + `return { search: searchState(), activeLeaf: app.workspace.activeLeaf.view.getViewType(), leftCollapsed: app.workspace.leftSplit.collapsed };`) };
  // A nested leaf row first (project/alpha or alpha under project), then a parent row.
  let ix = rowsNow.findIndex(r => /alpha/.test(r.text));
  o.clickedLeaf = { ix, rect: await c.clickElement('.workspace-leaf-content[data-type="tag"] .tree-item-self', { index: ix }) };
  await sleep(900);
  o.afterLeaf = await c.evaluate(PROBE_JS + `return { search: searchState(), activeLeaf: app.workspace.activeLeaf.view.getViewType(), tagRows: rows('tag').map(r => ({ text: r.text, selfCls: r.selfCls })), tagViewState: leafOf('tag').getViewState(), activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, searchDom: (() => { const l = app.workspace.getLeavesOfType('search')[0]; const sc = l.view.containerEl.querySelector('.search-input-container'); return sc && dump(sc, 0); })(), searchResults: Array.from(app.workspace.getLeavesOfType('search')[0].view.containerEl.querySelectorAll('.search-result-file-title')).map(t => t.textContent.trim()) };`);
  ix = rowsNow.findIndex(r => /^project$/.test(r.text) || /^#?project$/.test(r.text));
  if (ix >= 0) {
    // The parent's label text, not its triangle.
    const inner = await c.rect('.workspace-leaf-content[data-type="tag"] .tree-item-self .tree-item-inner', ix);
    if (inner) await c.clickAt(inner.cx, inner.cy);
    await sleep(900);
    o.clickedParent = { ix, inner };
    o.afterParent = await c.evaluate(PROBE_JS + `return { search: searchState(), tagRows: rows('tag').map(r => ({ text: r.text, depth: r.depth, itemCls: r.itemCls })), tagViewState: leafOf('tag').getViewState() };`);
  }
  // A row with mixed casing (Project / project) and a 3-deep one.
  ix = rowsNow.findIndex(r => /deep/.test(r.text));
  if (ix >= 0) {
    o.clickedDeep = { ix, rect: await c.clickElement('.workspace-leaf-content[data-type="tag"] .tree-item-self', { index: ix }) };
    await sleep(900);
    o.afterDeep = await c.evaluate(PROBE_JS + `return { search: searchState() };`);
  }
  o.shot = await shot('tags-after-click');
  return o;
});

await step('s08-tags-context-menu', async () => {
  await c.rightClickElement('.workspace-leaf-content[data-type="tag"] .tree-item-self', { index: 1 });
  await sleep(500);
  const items = await c.openMenuItems();
  const dom = await c.evaluate(PROBE_JS + `return menuDom();`);
  await c.pressKey('Escape'); await sleep(200);
  return { items, dom };
});

await step('s09-tags-collapse-triangle', async () => {
  const o = { guard: await ensureRowsVisible('tag') };
  o.before = await c.evaluate(PROBE_JS + `return rows('tag').map(r => ({ text: r.text, depth: r.depth, itemCls: r.itemCls, selfCls: r.selfCls, collapse: r.collapseIcon && { display: r.collapseIcon.display, rect: r.collapseIcon.rect }, flair: r.flair }));`);
  const ci = await c.evaluate(PROBE_JS + `const p = paneOf('tag'); const r = Array.from(p.querySelectorAll('.tree-item-self')).find(s => s.querySelector('.collapse-icon')); if (!r) return null; const ci = r.querySelector('.collapse-icon'); const b = ci.getBoundingClientRect(); return { text: r.textContent.trim(), x: b.x + b.width / 2, y: b.y + b.height / 2, w: b.width, h: b.height };`);
  if (!ci) return { skipped: 'no collapse icon', before: o.before };
  await c.clickAt(ci.x, ci.y); await sleep(600);
  o.clicked = ci;
  o.afterFold = await c.evaluate(PROBE_JS + `return { rows: rows('tag').map(r => ({ text: r.text, depth: r.depth, itemCls: r.itemCls, selfCls: r.selfCls, flair: r.flair, collapse: r.collapseIcon && { transform: r.collapseIcon.transform, display: r.collapseIcon.display }, childrenRect: r.childrenRect })), viewState: leafOf('tag').getViewState(), search: searchState() };`);
  o.shotFolded = await shot('tags-folded');
  await c.clickAt(ci.x, ci.y); await sleep(600);
  o.afterUnfold = await c.evaluate(PROBE_JS + `return { rows: rows('tag').map(r => ({ text: r.text, itemCls: r.itemCls })), viewState: leafOf('tag').getViewState() };`);
  // Collapse all / Expand all header button, if any.
  // The Expand all / Collapse all button: label, icon and rows across three clicks.
  const readCA = () => c.evaluate(PROBE_JS + `return { rows: rows('tag').map(r => ({ text: r.text, itemCls: r.itemCls, childrenRect: r.childrenRect })), button: headerButtons('tag').map(b => ({ label: b.label, icon: b.icon, cls: b.cls }))[2], viewState: leafOf('tag').getViewState().state };`);
  o.ca0 = await readCA();
  for (let i = 1; i <= 3; i++) {
    const lab = (await c.evaluate(PROBE_JS + `return headerButtons('tag').map(b => b.label);`)).find(l => /collapse all|expand all/i.test(l));
    await clickButton('tag', lab); await sleep(300);
    o['ca' + i] = { clicked: lab, ...(await readCA()) };
  }
  return o;
});

await step('s10-tags-hierarchy-toggle', async () => {
  const guard = await ensureRowsVisible('tag');
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('tag').map(b => b.label);`);
  const l = labels.find(x => /nested|hierarch/i.test(x));
  if (!l) return { skipped: 'no nested toggle', labels };
  const o = { label: l, guard };
  const SPANS = `const spans = (t) => Array.from(paneOf('tag').querySelectorAll('.tree-item-self')).filter(s => /alpha|^common/.test(s.textContent)).map(s => ({ text: s.textContent.trim(), selfStyle: s.getAttribute('style'), rect: rect(s), parts: Array.from(s.querySelectorAll('.tree-item-inner *')).map(e => ({ tag: e.tagName, cls: e.className, text: e.textContent, rect: rect(e), style: cs(e, ['display','visibility','font-size','color','opacity','width','white-space','margin','padding']) })) }));`;
  o.before = await c.evaluate(PROBE_JS + SPANS + `return { viewState: leafOf('tag').getViewState(), button: headerButtons('tag').find(b => b.label === ${JSON.stringify(l)}), spans: spans() };`);
  o.clicked = await clickButton('tag', l);
  await sleep(500);
  o.flat = await c.evaluate(PROBE_JS + SPANS + `return { spans: spans(), viewState: leafOf('tag').getViewState(), button: headerButtons('tag').find(b => /nested|hierarch/i.test(b.label)), headers: headerButtons('tag').map(b => ({ label: b.label, cls: b.cls, icon: b.icon })), rows: rows('tag'), dom: dump(paneOf('tag').querySelector('.view-content') || paneOf('tag'), 0) };`);
  o.shotFlat = await shot('tags-flat');
  // Click a slash row when flat: which query.
  const flatRows = o.flat.rows.map(r => r.text);
  const ix = flatRows.findIndex(t => /project\/alpha/.test(t));
  if (ix >= 0) {
    await c.clickElement('.workspace-leaf-content[data-type="tag"] .tree-item-self', { index: ix });
    await sleep(900);
    o.flatClick = { ix, search: await c.evaluate(PROBE_JS + `return searchState();`) };
  }
  // Sort menu entries while flat.
  const sortLabel = labels.find(x => /sort/i.test(x));
  if (sortLabel) { await clickButton('tag', sortLabel); o.flatSortMenu = await c.evaluate(PROBE_JS + `return menuDom().map(m => m.items.map(i => ({ text: i.text, cls: i.cls })));`); await c.pressKey('Escape'); await sleep(300); }
  await clickButton('tag', l);
  await sleep(500);
  o.nestedAgain = await c.evaluate(PROBE_JS + `return { viewState: leafOf('tag').getViewState(), rows: rows('tag').map(r => ({ depth: r.depth, text: r.text, flair: r.flair, itemCls: r.itemCls })) };`);
  return o;
});

await step('s11-tags-keyboard', async () => {
  // Focus the pane by clicking a row, then arrows.
  await c.clickElement('.workspace-leaf-content[data-type="tag"] .tree-item-self', { index: 0 });
  await sleep(500);
  const o = { afterClick: await c.evaluate(PROBE_JS + `return { activeEl: document.activeElement && (document.activeElement.tagName + '.' + document.activeElement.className), marked: marked('tag'), search: searchState() && searchState().inputValue };`) };
  await c.pressKey('ArrowDown'); await sleep(300);
  o.afterDown = await c.evaluate(PROBE_JS + `return { marked: marked('tag'), rows: rows('tag').map(r => ({ text: r.text, selfCls: r.selfCls })) };`);
  await c.pressKey('Enter'); await sleep(700);
  o.afterEnter = await c.evaluate(PROBE_JS + `return { marked: marked('tag'), search: searchState() && searchState().inputValue };`);
  return o;
});

// ---------------------------------------------------------------------------
// All properties pane
// ---------------------------------------------------------------------------
await step('s20-open-properties', async () => {
  await c.evaluate(`await app.commands.executeCommandById('properties:open'); return true;`);
  await sleep(1500);
  return { shot: await shot('properties-default'), ...(await c.evaluate(PROBE_JS + `const pane = paneOf('all-properties'); return { info: paneInfo('all-properties'), dom: dump(pane, 0), vars: vars(pane, ${JSON.stringify(VARS)}), rows: rows('all-properties'), headers: headerButtons('all-properties'), registry: Object.values(app.metadataTypeManager.properties || {}).map(p => ({ name: p.name, type: p.type, count: p.count, widget: p.widget })) };`)) };
});

await step('s21-properties-hover', async () => {
  const r = await c.hover('.workspace-leaf-content[data-type="all-properties"] .tree-item-self', 2);
  await sleep(500);
  const row = await c.evaluate(PROBE_JS + `const p = paneOf('all-properties'); const r = p.querySelectorAll('.tree-item-self')[2]; const inner = r.querySelector('.tree-item-inner'); const flair = r.querySelector('.tree-item-flair'); const ic = r.querySelector('.tree-item-icon');
    return { hovered: r.matches(':hover'), text: r.textContent.trim(), cls: r.className, style: cs(r, ['background-color','color','border-radius']), inner: inner && cs(inner, ['color']), flair: flair && cs(flair, ['color']), icon: ic && cs(ic, ['color','opacity']), tooltip: tooltip() };`);
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('all-properties').map(b => b.label);`);
  const buttons = {};
  for (const l of labels) buttons[l] = await hoverButton('all-properties', l);
  return { pointer: r, row, buttons };
});

await step('s22-properties-sort-menu', async () => {
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('all-properties').map(b => b.label);`);
  const sortLabel = labels.find(l => /sort/i.test(l));
  if (!sortLabel) return { skipped: 'no sort button', labels };
  const o = { sortLabel };
  o.clicked = await clickButton('all-properties', sortLabel);
  o.items = await c.openMenuItems();
  o.menuDom = await c.evaluate(PROBE_JS + `return menuDom();`);
  await c.pressKey('Escape'); await sleep(300);
  o.orders = {};
  const names = o.items.flat().filter(x => x !== '---');
  for (const name of names) {
    await clickButton('all-properties', sortLabel);
    await clickMenuItem(name);
    o.orders[name] = await c.evaluate(PROBE_JS + `return { viewState: leafOf('all-properties').getViewState(), rows: rows('all-properties').map(r => ({ text: r.text, flair: r.flair, icons: r.icons })) };`);
  }
  await clickButton('all-properties', sortLabel); await clickMenuItem(names[names.length - 2]);
  o.restored = await c.evaluate(PROBE_JS + `return leafOf('all-properties').getViewState();`);
  return o;
});

await step('s23-properties-search', async () => {
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('all-properties').map(b => b.label);`);
  const l = labels.find(x => /search/i.test(x));
  if (!l) return { skipped: 'no search button', labels };
  const o = {};
  o.clicked = await clickButton('all-properties', l);
  o.shown = await c.evaluate(PROBE_JS + `const sc = paneOf('all-properties').querySelector('.search-input-container'); return { container: sc && dump(sc, 0), focused: document.activeElement && document.activeElement.outerHTML.slice(0, 200), viewState: leafOf('all-properties').getViewState() };`);
  await c.typeText('boo'); await sleep(600);
  o.typed = await c.evaluate(PROBE_JS + `return { rows: rows('all-properties').map(r => ({ text: r.text, flair: r.flair })), viewState: leafOf('all-properties').getViewState(), inputValue: paneOf('all-properties').querySelector('input').value };`);
  await c.pressKey('Backspace'); await c.pressKey('Backspace'); await c.pressKey('Backspace'); await c.typeText('zzz'); await sleep(500);
  o.noMatch = await c.evaluate(PROBE_JS + `return { rows: rows('all-properties').map(r => r.text), empty: Array.from(paneOf('all-properties').querySelectorAll('.search-empty-state, .pane-empty')).map(e => ({ cls: e.className, text: e.textContent, style: cs(e, ['color','font-size','padding','margin']) })), content: dump(paneOf('all-properties').querySelector('.view-content'), 0) };`);
  await c.pressKey('Escape'); await sleep(400);
  o.escaped = await c.evaluate(PROBE_JS + `const sc = paneOf('all-properties').querySelector('.search-input-container'); return { containerDisplay: sc && getComputedStyle(sc).display, inputValue: sc && sc.querySelector('input').value, rows: rows('all-properties').length, viewState: leafOf('all-properties').getViewState() };`);
  return o;
});

await step('s24-properties-click-row', async () => {
  const rowsNow = await c.evaluate(PROBE_JS + `return rows('all-properties').map(r => r.text);`);
  const o = { rowsNow, before: await c.evaluate(PROBE_JS + `return searchState();`) };
  for (const key of ['book.title', 'status', 'tags']) {
    const ix = rowsNow.findIndex(t => t.toLowerCase() === key);
    if (ix < 0) continue;
    await c.clickElement('.workspace-leaf-content[data-type="all-properties"] .tree-item-self', { index: ix });
    await sleep(900);
    o[key] = { ix, search: await c.evaluate(PROBE_JS + `return { search: searchState(), activeLeaf: app.workspace.activeLeaf.view.getViewType(), marked: marked('all-properties') };`) };
  }
  o.contextMenu = await (async () => { await c.rightClickElement('.workspace-leaf-content[data-type="all-properties"] .tree-item-self', { index: 2 }); await sleep(500); const items = await c.openMenuItems(); const dom = await c.evaluate(PROBE_JS + `return menuDom();`); await c.pressKey('Escape'); await sleep(200); return { items, dom }; })();
  o.shot = await shot('properties-after-click');
  return o;
});

// ---------------------------------------------------------------------------
// Outline pane
// ---------------------------------------------------------------------------
await step('s30-open-outline', async () => {
  await c.evaluate(`
    const f = app.vault.getAbstractFileByPath('Outline.md');
    const l = app.workspace.getLeaf(false); await l.openFile(f);
    const s = l.getViewState(); s.state.mode = 'source'; s.state.source = true; await l.setViewState(s);
    await app.commands.executeCommandById('outline:open');
    return true;`);
  await sleep(1500);
  return { shot: await shot('outline-source'), ...(await c.evaluate(PROBE_JS + `const pane = paneOf('outline'); return { info: paneInfo('outline'), editor: editorState(), dom: dump(pane, 0), vars: vars(pane, ${JSON.stringify(VARS)}), rows: rows('outline'), headers: headerButtons('outline'), marked: marked('outline'), headings: app.metadataCache.getFileCache(app.vault.getAbstractFileByPath('Outline.md')).headings.map(h => ({ level: h.level, heading: h.heading, line: h.position.start.line })) };`)) };
});

await step('s31-outline-hover', async () => {
  const r = await c.hover('.workspace-leaf-content[data-type="outline"] .tree-item-self', 2);
  await sleep(500);
  const row = await c.evaluate(PROBE_JS + `const p = paneOf('outline'); const r = p.querySelectorAll('.tree-item-self')[2]; const inner = r.querySelector('.tree-item-inner'); const ci = r.querySelector('.collapse-icon');
    return { hovered: r.matches(':hover'), text: r.textContent.trim(), cls: r.className, style: cs(r, ['background-color','color','border-radius']), inner: inner && cs(inner, ['color']), collapseIcon: ci && { display: getComputedStyle(ci).display, opacity: getComputedStyle(ci).opacity, color: getComputedStyle(ci).color, rect: rect(ci) }, tooltip: tooltip() };`);
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('outline').map(b => b.label);`);
  const buttons = {};
  for (const l of labels) buttons[l] = await hoverButton('outline', l);
  return { pointer: r, row, buttons };
});

await step('s32-outline-click-source', async () => {
  const rowsNow = await c.evaluate(PROBE_JS + `return rows('outline').map(r => ({ text: r.text, depth: r.depth }));`);
  const o = { rowsNow, before: await c.evaluate(PROBE_JS + `return editorState();`) };
  for (const pat of [/^Fourth level$/, /^Second top$/, /very long/]) {
    const ix = rowsNow.findIndex(r => pat.test(r.text));
    if (ix < 0) continue;
    const inner = await c.rect('.workspace-leaf-content[data-type="outline"] .tree-item-self .tree-item-inner', ix);
    await c.clickAt(inner.cx, inner.cy);
    await sleep(900);
    o[rowsNow[ix].text.slice(0, 20)] = { ix, inner, after: await c.evaluate(PROBE_JS + `return { editor: editorState(), marked: marked('outline'), outlineViewState: leafOf('outline').getViewState(), selection: (() => { const v = mdLeaf().view; return v.editor && { from: v.editor.getCursor('from'), to: v.editor.getCursor('to'), lineText: v.editor.getLine(v.editor.getCursor().line) }; })(), cmScroll: (() => { const v = mdLeaf().view; const cm = v.editor && v.editor.cm; return cm && { scrollTop: cm.scrollDOM.scrollTop, headingTop: (() => { const ln = cm.state.doc.line(v.editor.getCursor().line + 1); const co = cm.coordsAtPos(ln.from); const sr = cm.scrollDOM.getBoundingClientRect(); return co && { top: Math.round(co.top - sr.top), scrollDomTop: Math.round(sr.top) }; })() }; })() };`) };
  }
  o.shot = await shot('outline-after-click-source');
  return o;
});

await step('s33-outline-follow-cursor-source', async () => {
  const o = { viewState: await c.evaluate(PROBE_JS + `return leafOf('outline').getViewState();`) };
  const headings = await c.evaluate(`return app.metadataCache.getFileCache(app.vault.getAbstractFileByPath('Outline.md')).headings.map(h => ({ level: h.level, heading: h.heading, line: h.position.start.line }));`);
  o.headings = headings;
  // Lines inside: before the first heading is impossible (frontmatter); a line in the h4 section, in 'Another second', the last section, and a heading line itself.
  const targets = [{ name: 'h1-top-body', line: headings[0].line + 3 }, { name: 'h4-body', line: headings[3].line + 2 }, { name: 'h2-another-heading-line', line: headings[4].line }, { name: 'h4-skip-body', line: headings[5].line + 1 }, { name: 'last-top-body', line: headings[headings.length - 1].line + 5 }, { name: 'frontmatter', line: 1 }];
  o.moves = {};
  for (const t of targets) {
    await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; v.editor.setCursor({ line: ${t.line}, ch: 0 }); v.editor.scrollIntoView({ from: { line: ${t.line}, ch: 0 }, to: { line: ${t.line}, ch: 0 } }, true); return true;`);
    await sleep(700);
    o.moves[t.name] = { line: t.line, ...(await c.evaluate(PROBE_JS + `return { marked: marked('outline'), rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), selfCls: r.selfCls, itemCls: r.itemCls })), editor: editorState(), paneScroll: (paneOf('outline').querySelector('.view-content') || paneOf('outline')).scrollTop };`)) };
  }
  // A real click in the editor on the 'Fourth under second' section.
  const line = headings[5].line + 2;
  const pos = await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; const cm = v.editor.cm; v.editor.scrollIntoView({ from: { line: ${line}, ch: 0 }, to: { line: ${line}, ch: 0 } }, true); await new Promise(r => setTimeout(r, 300)); const ln = cm.state.doc.line(${line} + 1); const co = cm.coordsAtPos(ln.from + 5); return co && { x: co.left + 2, y: (co.top + co.bottom) / 2 };`);
  if (pos) { await c.clickAt(pos.x, pos.y); await sleep(700); o.realClick = { line, pos, marked: await c.evaluate(PROBE_JS + `return { marked: marked('outline'), editor: editorState() };`) }; }
  o.shot = await shot('outline-follow-source');
  // Toggle follow cursor off if a header button offers it, move again, read.
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('outline').map(b => b.label);`);
  const fl = labels.find(l => /auto-scroll|follow|cursor/i.test(l));
  o.followLabel = fl || null;
  const PANE = `const paneScroll = () => { const vc = paneOf('outline').querySelector('.view-content'); const m = paneOf('outline').querySelector('.tree-item-self.is-active'); return { viewContentScrollTop: vc.scrollTop, viewContentRect: rect(vc), scrollHeight: vc.scrollHeight, scrollers: Array.from(paneOf('outline').querySelectorAll('*')).filter(e => e.scrollTop > 0).map(e => ({ cls: e.className, scrollTop: e.scrollTop })), activeRect: m && rect(m), activeText: m && m.textContent.trim() }; };`;
  const extraLine = headings.find(h => /^Extra 20$/.test(h.heading));
  const goto = async (line) => { await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; v.editor.setCursor({ line: ${line}, ch: 0 }); v.editor.scrollIntoView({ from: { line: ${line}, ch: 0 }, to: { line: ${line}, ch: 0 } }, true); return true;`); await sleep(800); };
  // followCursor is false by default: the row is marked but does the pane scroll to it?
  o.offTop = await c.evaluate(PROBE_JS + PANE + `return paneScroll();`);
  if (extraLine) { await goto(extraLine.line + 1); o.offFar = await c.evaluate(PROBE_JS + PANE + `return { ...paneScroll(), marked: marked('outline').map(m => m.text) };`); }
  await goto(headings[0].line + 2);
  if (fl) {
    await clickButton('outline', fl);
    o.toggledOn = await c.evaluate(PROBE_JS + PANE + `return { viewState: leafOf('outline').getViewState(), button: headerButtons('outline').find(b => /auto-scroll|follow|cursor/i.test(b.label)), marked: marked('outline').map(m => m.text), pane: paneScroll() };`);
    if (extraLine) { await goto(extraLine.line + 1); o.onFar = await c.evaluate(PROBE_JS + PANE + `return { ...paneScroll(), marked: marked('outline').map(m => m.text) };`); }
    await goto(headings[0].line + 2);
    o.onBack = await c.evaluate(PROBE_JS + PANE + `return { ...paneScroll(), marked: marked('outline').map(m => m.text) };`);
    o.shotOn = await shot('outline-follow-on');
    await clickButton('outline', fl);
    o.toggledOff = await c.evaluate(PROBE_JS + `return { viewState: leafOf('outline').getViewState(), button: headerButtons('outline').find(b => /auto-scroll|follow|cursor/i.test(b.label)) };`);
  }
  return o;
});

await step('s34-outline-reading', async () => {
  await c.evaluate(`const l = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit); const s = l.getViewState(); s.state.mode = 'preview'; await l.setViewState(s); return true;`);
  await sleep(1200);
  const o = { start: await c.evaluate(PROBE_JS + `return { editor: editorState(), marked: marked('outline'), viewState: leafOf('outline').getViewState() };`) };
  const rowsNow = await c.evaluate(PROBE_JS + `return rows('outline').map(r => r.text);`);
  for (const pat of [/^Second top$/, /^Fourth level$/]) {
    const ix = rowsNow.findIndex(t => pat.test(t));
    if (ix < 0) continue;
    const inner = await c.rect('.workspace-leaf-content[data-type="outline"] .tree-item-self .tree-item-inner', ix);
    await c.clickAt(inner.cx, inner.cy);
    await sleep(1000);
    o['click:' + rowsNow[ix]] = await c.evaluate(PROBE_JS + `const v = mdLeaf().view; const pv = v.previewMode; const el = pv.containerEl.querySelector('.markdown-preview-view'); const hs = Array.from(el.querySelectorAll('h1,h2,h3,h4')).map(h => ({ tag: h.tagName, text: h.textContent.trim().slice(0, 30), top: Math.round(h.getBoundingClientRect().top - el.getBoundingClientRect().top) }));
      return { editor: editorState(), marked: marked('outline'), scrollTop: el.scrollTop, previewTop: Math.round(el.getBoundingClientRect().top), headingsVisible: hs.filter(h => h.top >= -5 && h.top < 200) };`);
  }
  // Scroll the reading view with the wheel and read the marking.
  const pv = await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; const el = v.previewMode.containerEl.querySelector('.markdown-preview-view'); const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  o.wheel = {};
  await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; v.previewMode.applyScroll(0); return true;`);
  await sleep(500);
  o.wheel.top = await c.evaluate(PROBE_JS + `return { marked: marked('outline'), previewScroll: mdLeaf().view.previewMode.getScroll() };`);
  for (let i = 1; i <= 4; i++) {
    await c.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: Math.round(pv.x), y: Math.round(pv.y), deltaX: 0, deltaY: 1200 });
    await sleep(900);
    o.wheel['step' + i] = await c.evaluate(PROBE_JS + `const v = mdLeaf().view; const el = v.previewMode.containerEl.querySelector('.markdown-preview-view'); const er = el.getBoundingClientRect(); const hs = Array.from(el.querySelectorAll('h1,h2,h3,h4')).map(h => ({ tag: h.tagName, text: h.textContent.trim().slice(0, 30), top: Math.round(h.getBoundingClientRect().top - er.top) }));
      return { marked: marked('outline'), previewScroll: v.previewMode.getScroll(), scrollTop: el.scrollTop, headingsNearTop: hs.filter(h => h.top > -600 && h.top < 400) };`);
  }
  await c.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: Math.round(pv.x), y: Math.round(pv.y), deltaX: 0, deltaY: -9000 });
  await sleep(900);
  o.wheel.backToTop = await c.evaluate(PROBE_JS + `return { marked: marked('outline').map(m => m.text), previewScroll: mdLeaf().view.previewMode.getScroll() };`);
  o.shot = await shot('outline-reading-scrolled');
  // Back to source mode.
  await c.evaluate(`const l = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit); const s = l.getViewState(); s.state.mode = 'source'; await l.setViewState(s); return true;`);
  await sleep(800);
  return o;
});

await step('s35-outline-collapse', async () => {
  const o = {};
  const ci = await c.evaluate(PROBE_JS + `const p = paneOf('outline'); const r = Array.from(p.querySelectorAll('.tree-item-self')).find(s => s.querySelector('.collapse-icon')); if (!r) return null; const ci = r.querySelector('.collapse-icon'); const b = ci.getBoundingClientRect(); return { text: r.textContent.trim(), x: b.x + b.width / 2, y: b.y + b.height / 2, w: b.width, h: b.height, display: getComputedStyle(ci).display };`);
  if (!ci) return { skipped: 'no collapse icon' };
  o.icon = ci;
  await c.clickAt(ci.x, ci.y); await sleep(600);
  o.afterFold = await c.evaluate(PROBE_JS + `return { rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), depth: r.depth, itemCls: r.itemCls, selfCls: r.selfCls, collapse: r.collapseIcon && { transform: r.collapseIcon.transform }, childrenRect: r.childrenRect })), viewState: leafOf('outline').getViewState(), editor: editorState() };`);
  // With the parent folded, move the cursor into a hidden child's section: which row is marked.
  const headings = await c.evaluate(`return app.metadataCache.getFileCache(app.vault.getAbstractFileByPath('Outline.md')).headings.map(h => ({ level: h.level, heading: h.heading, line: h.position.start.line }));`);
  await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; v.editor.setCursor({ line: ${headings[3].line + 2}, ch: 0 }); return true;`);
  await sleep(700);
  o.foldedCursorInChild = await c.evaluate(PROBE_JS + `return { marked: marked('outline'), rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), itemCls: r.itemCls, selfCls: r.selfCls })) };`);
  o.shotFolded = await shot('outline-folded');
  await c.clickAt(ci.x, ci.y); await sleep(600);
  o.afterUnfold = await c.evaluate(PROBE_JS + `return { rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), itemCls: r.itemCls })), viewState: leafOf('outline').getViewState() };`);
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('outline').map(b => b.label);`);
  const ca = labels.find(l => /collapse all|expand all/i.test(l));
  if (ca) {
    await clickButton('outline', ca);
    o.collapseAll = { label: ca, after: await c.evaluate(PROBE_JS + `return { rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), depth: r.depth, itemCls: r.itemCls, childrenRect: r.childrenRect })), headers: headerButtons('outline').map(b => ({ label: b.label, icon: b.icon })), viewState: leafOf('outline').getViewState() };`) };
    const labels2 = await c.evaluate(PROBE_JS + `return headerButtons('outline').map(b => b.label);`);
    const ea = labels2.find(l => /collapse all|expand all/i.test(l));
    if (ea) { await clickButton('outline', ea); o.collapseAll.toggledBack = { label: ea, after: await c.evaluate(PROBE_JS + `return { rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), itemCls: r.itemCls })), headers: headerButtons('outline').map(b => b.label), viewState: leafOf('outline').getViewState() };`) }; }
  }
  return o;
});

await step('s36-outline-search', async () => {
  const labels = await c.evaluate(PROBE_JS + `return headerButtons('outline').map(b => b.label);`);
  const l = labels.find(x => /search/i.test(x));
  if (!l) return { skipped: 'no search button', labels };
  const o = {};
  o.clicked = await clickButton('outline', l);
  o.shown = await c.evaluate(PROBE_JS + `const sc = paneOf('outline').querySelector('.search-input-container'); return { container: sc && dump(sc, 0), focused: document.activeElement && document.activeElement.outerHTML.slice(0, 200), viewState: leafOf('outline').getViewState() };`);
  await c.typeText('second'); await sleep(600);
  o.typed = await c.evaluate(PROBE_JS + `return { rows: rows('outline').map(r => ({ text: r.text.slice(0, 40), depth: r.depth, itemCls: r.itemCls, selfCls: r.selfCls })), viewState: leafOf('outline').getViewState(), inputValue: paneOf('outline').querySelector('input').value, highlighted: Array.from(paneOf('outline').querySelectorAll('.tree-item-inner *')).map(e => ({ tag: e.tagName, cls: e.className, text: e.textContent })) };`);
  for (let i = 0; i < 6; i++) await c.pressKey('Backspace');
  await c.typeText('zzz'); await sleep(500);
  o.noMatch = await c.evaluate(PROBE_JS + `return { rows: rows('outline').map(r => r.text), empty: Array.from(paneOf('outline').querySelectorAll('.search-empty-state, .pane-empty')).map(e => ({ cls: e.className, text: e.textContent, style: cs(e, ['color','font-size','padding','margin']) })), content: dump(paneOf('outline').querySelector('.view-content'), 0) };`);
  await c.pressKey('Escape'); await sleep(400);
  o.escaped = await c.evaluate(PROBE_JS + `const sc = paneOf('outline').querySelector('.search-input-container'); return { containerDisplay: sc && getComputedStyle(sc).display, inputValue: sc && sc.querySelector('input').value, rows: rows('outline').length, viewState: leafOf('outline').getViewState() };`);
  return o;
});

await step('s37-outline-context-menu', async () => {
  await c.rightClickElement('.workspace-leaf-content[data-type="outline"] .tree-item-self', { index: 1 });
  await sleep(500);
  const items = await c.openMenuItems();
  const dom = await c.evaluate(PROBE_JS + `return menuDom();`);
  await c.pressKey('Escape'); await sleep(200);
  return { items, dom };
});

await step('s38-outline-switch-notes', async () => {
  const o = {};
  await c.evaluate(`const f = app.vault.getAbstractFileByPath('NoHeadings.md'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(1200);
  o.noHeadings = await c.evaluate(PROBE_JS + `return { info: paneInfo('outline'), content: dump(paneOf('outline').querySelector('.view-content'), 0), headers: headerButtons('outline').map(b => b.label), empty: Array.from(paneOf('outline').querySelectorAll('.pane-empty, .search-empty-state')).map(e => ({ cls: e.className, text: e.textContent, style: cs(e, ['color','font-size','padding','margin','text-align','display']) })) };`);
  o.shotEmpty = await shot('outline-empty');
  await c.evaluate(`const f = app.vault.getAbstractFileByPath('Alpha.md'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(1200);
  o.alpha = await c.evaluate(PROBE_JS + `return { info: paneInfo('outline'), rows: rows('outline').map(r => ({ text: r.text, depth: r.depth, selfCls: r.selfCls })), marked: marked('outline') };`);
  // Tags leaf is in the right dock: switch to it and check the tags pane still shows the whole vault.
  await c.evaluate(`const f = app.vault.getAbstractFileByPath('Outline.md'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(1200);
  o.backToOutline = await c.evaluate(PROBE_JS + `return { rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), depth: r.depth, itemCls: r.itemCls })), marked: marked('outline'), viewState: leafOf('outline').getViewState(), editor: editorState() };`);
  // A non-markdown leaf (an empty tab): what the outline shows.
  await c.evaluate(`app.workspace.getLeaf('tab'); return true;`);
  await sleep(1000);
  o.emptyTab = await c.evaluate(PROBE_JS + `return { info: paneInfo('outline'), content: dump(paneOf('outline').querySelector('.view-content'), 0) };`);
  await c.evaluate(`const l = app.workspace.activeLeaf; if (l && l.view.getViewType() === 'empty') l.detach(); const f = app.vault.getAbstractFileByPath('Outline.md'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(800);
  return o;
});

await step('s39-outline-live-edit', async () => {
  // Edit a heading in the editor and read the outline's row text: does it follow, and how quickly.
  const o = {};
  await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; const s = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).getViewState(); if (s.state.mode !== 'source') { s.state.mode = 'source'; await app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).setViewState(s); } return true;`);
  await sleep(500);
  const hl = await c.evaluate(`return app.metadataCache.getFileCache(app.vault.getAbstractFileByPath('Outline.md')).headings[0].position.start.line;`);
  await c.evaluate(`const v = app.workspace.getLeavesOfType('markdown').find(l => l.getRoot() === app.workspace.rootSplit).view; v.editor.setCursor({ line: ${hl}, ch: v.editor.getLine(${hl}).length }); v.editor.focus(); return true;`);
  await sleep(300);
  const t0 = Date.now();
  await c.typeText(' X');
  o.samples = [];
  while (Date.now() - t0 < 4500) { await sleep(250); o.samples.push({ ms: Date.now() - t0, first: await c.evaluate(PROBE_JS + `return rows('outline')[0] && rows('outline')[0].text;`), onDisk: /Top heading X/.test(bytes('Outline.md') || '') }); if (o.samples[o.samples.length - 1].first === 'Top heading X') break; }
  await c.pressKey('Backspace'); await c.pressKey('Backspace');
  await sleep(2500);
  o.reverted = await c.evaluate(PROBE_JS + `return rows('outline')[0] && rows('outline')[0].text;`);
  return o;
});

await step('s40-set-states-for-restart', async () => {
  // Leave non-default states behind: tags sorted by another entry and flat, outline folded + follow toggled, properties alphabetical, every search box open with text.
  const o = {};
  await c.evaluate(`await app.commands.executeCommandById('tag-pane:open'); return true;`); await sleep(800);
  const tagLabels = await c.evaluate(PROBE_JS + `return headerButtons('tag').map(b => b.label);`);
  const nested = tagLabels.find(x => /nested|hierarch/i.test(x));
  if (nested) await clickButton('tag', nested);
  const sortLabel = tagLabels.find(x => /sort/i.test(x));
  if (sortLabel) { let items = []; for (let i = 0; i < 3 && items.length < 2; i++) { await clickButton('tag', sortLabel); await sleep(300); items = (await c.openMenuItems()).flat().filter(x => x !== '---'); } if (items.length) await clickMenuItem(items[items.length - 1]); }
  const search = tagLabels.find(x => /search/i.test(x));
  if (search) { await clickButton('tag', search); const f = await c.evaluate(`return document.activeElement && document.activeElement.tagName;`); if (f === 'INPUT') await c.typeText('com'); await sleep(300); o.tagSearchFocus = f; }
  o.tag = await c.evaluate(PROBE_JS + `return leafOf('tag').getViewState();`);
  await c.evaluate(`await app.commands.executeCommandById('properties:open'); return true;`); await sleep(600);
  const pLabels = await c.evaluate(PROBE_JS + `return headerButtons('all-properties').map(b => b.label);`);
  const ps = pLabels.find(x => /sort/i.test(x));
  if (ps) { let items = []; for (let i = 0; i < 3 && items.length < 2; i++) { await clickButton('all-properties', ps); await sleep(300); items = (await c.openMenuItems()).flat().filter(x => x !== '---'); } o.propertiesMenuTries = items; if (items.length > 1) await clickMenuItem(items[1]); }
  const psearch = pLabels.find(x => /search/i.test(x));
  if (psearch) { await clickButton('all-properties', psearch); const f = await c.evaluate(`return document.activeElement && document.activeElement.tagName;`); if (f === 'INPUT') await c.typeText('st'); await sleep(300); o.propertiesSearchFocus = f; }
  o.properties = await c.evaluate(PROBE_JS + `return leafOf('all-properties').getViewState();`);
  await c.evaluate(`await app.commands.executeCommandById('outline:open'); return true;`); await sleep(600);
  const ci = await c.evaluate(PROBE_JS + `const p = paneOf('outline'); const r = Array.from(p.querySelectorAll('.tree-item-self')).find(s => s.querySelector('.collapse-icon')); if (!r) return null; const b = r.querySelector('.collapse-icon').getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  if (ci) { await c.clickAt(ci.x, ci.y); await sleep(400); }
  const oLabels = await c.evaluate(PROBE_JS + `return headerButtons('outline').map(b => b.label);`);
  const fl = oLabels.find(l => /auto-scroll|follow|cursor/i.test(l));
  if (fl) await clickButton('outline', fl);
  const osearch = oLabels.find(x => /search/i.test(x));
  if (osearch) { await clickButton('outline', osearch); const f = await c.evaluate(`return document.activeElement && document.activeElement.tagName;`); if (f === 'INPUT') await c.typeText('top'); await sleep(300); o.outlineSearchFocus = f; }
  o.outline = await c.evaluate(PROBE_JS + `return { viewState: leafOf('outline').getViewState(), rows: rows('outline').map(r => ({ text: r.text.slice(0, 30), itemCls: r.itemCls })), headers: headerButtons('outline').map(b => ({ label: b.label, cls: b.cls })) };`);
  await sleep(2500);
  const ws = JSON.parse(bytes('.obsidian/workspace.json') || '{}');
  const leaves = []; const walk = (n) => { if (!n || typeof n !== 'object') return; if (n.type === 'leaf') leaves.push(n.state); if (Array.isArray(n.children)) n.children.forEach(walk); };
  for (const k of ['left', 'right', 'main']) walk(ws[k]);
  o.workspaceBeforeQuit = leaves.filter(l => l && TYPES.includes(l.type));
  return o;
});

await step('s41-quit', async () => { await c.quit(); return true; });
out.finishedAt = new Date().toISOString();
save();
process.exit(0);
