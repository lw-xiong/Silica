// The Outgoing links pane (and, for comparison, the Backlinks pane) driven
// over CDP: the pane's markup, computed styles and icons; whether a link
// written twice is listed twice; how an unresolved link row looks and what
// clicking it does; what clicking a resolved row with a subpath or an alias
// opens; the Unlinked mentions section and the bytes its Link button writes;
// the section headers' collapse; the row context menu; and the leaf state
// each pane writes to workspace.json.
// Phases: fixtures | run | after-restart   (node outgoing-links.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 170000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.outgoingLinks = all.outgoingLinks || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.outgoingLinks[phase] = out;
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
async function waitForChange(rel, before, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (bytes(rel) !== before) return true; await sleep(150); }
  return false;
}

if (phase === 'fixtures') {
  fs.mkdirSync(vp('Sub'), { recursive: true });
  fs.writeFileSync(vp('Alpha.md'), [
    '---', 'related: "[[Gamma]]"', '---', '# Alpha', '',
    'See [[Beta]] and [[Beta#Heading|the alias]] and [[Beta]] again, then [[Missing]],',
    'a markdown [Beta link](Beta.md), an embed ![[Beta]], a self link [[#Local]], a block [[Gamma#^blk]],',
    'and a folder note [[Sub/Deep]].', '',
    'Gamma and gamma and GammaAlias and Delta appear unlinked here. Beta too, and delta.', '',
    '## Local', '', 'Text under Local.', '',
  ].join('\n'));
  fs.writeFileSync(vp('Beta.md'), '# Beta\n\nIntro, back to [[Alpha]] and [[Alpha#Local|local]].\n\n## Heading\n\nUnder the heading.\n');
  fs.writeFileSync(vp('Gamma.md'), '---\naliases:\n  - GammaAlias\n---\n# Gamma\n\nA block line ^blk\n');
  fs.writeFileSync(vp('Delta.md'), '# Delta\n\nMentions Alpha once.\n');
  fs.writeFileSync(vp('Sub/Deep.md'), '# Deep\n');
  out.files = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}

if (phase === 'after-restart') {
  const ws = JSON.parse(bytes('.obsidian/workspace.json'));
  const leaves = [];
  const walk = (n) => { if (!n || typeof n !== 'object') return; if (n.type === 'leaf') leaves.push(n.state); for (const k of ['children']) if (Array.isArray(n[k])) n[k].forEach(walk); };
  for (const k of ['left', 'right', 'main']) walk(ws[k]);
  out.leafStates = leaves.filter(l => l && ['outgoing-link', 'backlink'].includes(l.type));
  out.alpha = bytes('Alpha.md');
  save();
  const c2 = await connect();
  out.restored = await c2.evaluate(`const o = {}; for (const t of ['outgoing-link','backlink']) { const l = app.workspace.getLeavesOfType(t)[0]; o[t] = l && l.getViewState(); } return o;`);
  await c2.quit();
  save();
  process.exit(0);
}

const c = await connect();
const PROBE_JS = `
const cs = (el, props) => { const s = getComputedStyle(el); const o = {}; for (const p of props) o[p] = s.getPropertyValue(p); return o; };
const PROPS = ['display','font-size','font-weight','line-height','color','background-color','padding','margin','height','min-height','border-radius','border','opacity','cursor','white-space','overflow','text-overflow','gap','align-items','justify-content','flex','width','text-decoration','font-style'];
const rect = (el) => { const r = el.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; };
function dump(el, depth) {
  if (depth > 16) return { tag: el.tagName, truncated: true };
  const tag = el.tagName.toLowerCase();
  const cls = el.className && el.className.baseVal !== undefined ? el.className.baseVal : el.className;
  const o = { tag, cls, rect: rect(el) };
  for (const a of ['aria-label','data-path','data-href','href','placeholder','type','style','draggable','data-count']) if (el.hasAttribute(a)) o[a] = el.getAttribute(a);
  const own = Array.from(el.childNodes).filter(n => n.nodeType === 3).map(n => n.textContent).join('').trim(); if (own) o.text = own;
  if (tag === 'svg') return o;
  o.style = cs(el, PROPS);
  const kids = Array.from(el.children); if (kids.length) o.children = kids.map(k => dump(k, depth + 1));
  return o;
}
const leafOf = (t) => app.workspace.getLeavesOfType(t)[0];
const paneOf = (t) => leafOf(t).view.containerEl;
const vars = (el, names) => { const s = getComputedStyle(el); const o = {}; for (const n of names) o[n] = s.getPropertyValue(n).trim(); return o; };
`;

await step('s01-open', async () => {
  await c.evaluate(`
    const f = app.vault.getAbstractFileByPath('Alpha.md');
    await app.workspace.getLeaf(false).openFile(f);
    app.workspace.rightSplit.expand();
    await app.commands.executeCommandById('outgoing-links:open');
    return true;`);
  await sleep(1500);
  return await c.evaluate(PROBE_JS + `
    const leaves = {}; app.workspace.iterateAllLeaves(l => { leaves[l.view.getViewType()] = (leaves[l.view.getViewType()] || 0) + 1; });
    const leaf = leafOf('outgoing-link');
    return { leaves, active: app.workspace.getActiveFile() && app.workspace.getActiveFile().path,
      viewState: leaf && leaf.getViewState(), rightWidth: app.workspace.rightSplit.containerEl.getBoundingClientRect().width,
      tabHeader: leaf && leaf.tabHeaderEl && { cls: leaf.tabHeaderEl.className, label: leaf.tabHeaderEl.getAttribute('aria-label'), icon: (leaf.tabHeaderEl.querySelector('svg') || {}).getAttribute && leaf.tabHeaderEl.querySelector('svg').getAttribute('class') } };`);
});

await step('s02-outgoing-dom', async () => c.evaluate(PROBE_JS + `
  const pane = paneOf('outgoing-link');
  return { dom: dump(pane, 0), vars: vars(pane, ['--nav-item-size','--nav-item-color','--nav-item-weight','--nav-item-padding-y','--nav-item-padding-x','--nav-item-background-hover','--nav-item-color-hover','--nav-heading-color','--nav-heading-weight','--text-muted','--text-faint','--text-normal','--font-ui-small','--font-ui-smaller','--size-4-1','--size-4-2','--size-4-3','--background-primary','--background-secondary','--background-modifier-border','--text-highlight-bg','--radius-s','--radius-m','--link-unresolved-color','--link-unresolved-opacity','--link-unresolved-decoration-style','--link-unresolved-filter']) };`));

await step('s03-outgoing-rows', async () => c.evaluate(PROBE_JS + `
  const pane = paneOf('outgoing-link');
  const rows = Array.from(pane.querySelectorAll('.tree-item-self')).map(r => ({ cls: r.className, text: r.textContent.trim(), rect: rect(r),
    icon: (() => { const s = r.querySelector('svg'); return s && s.getAttribute('class'); })(),
    inner: (() => { const i = r.querySelector('.tree-item-inner'); return i && { cls: i.className, text: i.textContent.trim(), style: cs(i, ['color','font-size','font-weight','opacity','text-decoration']) }; })(),
    flair: (() => { const f = r.querySelector('.tree-item-flair'); return f && { cls: f.className, text: f.textContent.trim(), style: cs(f, ['color','font-size','background-color','border-radius','padding']) }; })(),
    style: cs(r, ['color','font-size','font-weight','padding','height','background-color','border-radius','opacity']) }));
  const buttons = Array.from(pane.querySelectorAll('button')).map(b => ({ cls: b.className, text: b.textContent.trim(), rect: rect(b), style: cs(b, ['font-size','padding','height','background-color','color','border-radius','box-shadow','font-weight']) }));
  const headers = Array.from(pane.querySelectorAll('.nav-header, .nav-buttons-container, .nav-action-button')).map(h => ({ cls: h.className, label: h.getAttribute('aria-label'), icon: (h.querySelector('svg') || { getAttribute: () => null }).getAttribute('class'), rect: rect(h) }));
  return { rows, buttons, headers };`));

await step('s04-hover-row', async () => {
  const r = await c.hover('.outgoing-link-pane .tree-item-self', 0);
  await sleep(400);
  return { pointer: r, ...(await c.evaluate(PROBE_JS + `
    const pane = paneOf('outgoing-link');
    const r = pane.querySelectorAll('.tree-item-self')[0];
    return { hovered: r.matches(':hover'), cls: r.className, style: cs(r, ['background-color','color','border-radius']), inner: cs(r.querySelector('.tree-item-inner') || r, ['color','text-decoration']) };`)) };
});

await step('s05-hover-unresolved', async () => c.evaluate(PROBE_JS + `
  const pane = paneOf('outgoing-link');
  const rows = Array.from(pane.querySelectorAll('.tree-item-self'));
  const un = rows.find(r => /Missing/.test(r.textContent));
  if (!un) return { none: true, texts: rows.map(r => r.textContent.trim()) };
  const inner = un.querySelector('.tree-item-inner') || un;
  return { cls: un.className, innerCls: inner.className, style: cs(un, ['color','opacity']), inner: cs(inner, ['color','opacity','text-decoration','font-style']) };`));

await step('s06-click-alias-row', async () => {
  const before = await c.evaluate(`return { active: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, leaves: (() => { const l = []; app.workspace.iterateRootLeaves(x => l.push(x.view.file && x.view.file.path)); return l; })() };`);
  const rows = await c.evaluate(`return Array.from(document.querySelectorAll('.outgoing-link-pane .tree-item-self')).map(r => r.textContent.trim());`);
  const ix = rows.findIndex(t => /alias|Heading/.test(t));
  if (ix < 0) return { rows, skipped: 'no alias row' };
  const r = await c.clickElement('.outgoing-link-pane .tree-item-self', { index: ix });
  await sleep(800);
  const after = await c.evaluate(`
    const l = []; app.workspace.iterateRootLeaves(x => l.push({ path: x.view.file && x.view.file.path, mode: x.view.getMode && x.view.getMode(), state: x.getViewState().state }));
    const leaf = app.workspace.activeLeaf; const v = leaf && leaf.view;
    return { active: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, leaves: l, viewType: v && v.getViewType(), mode: v && v.getMode && v.getMode(),
      cursor: v && v.editor && v.editor.getCursor(), scrollTop: v && v.editor && v.editor.getScrollInfo && v.editor.getScrollInfo().top, ephemeral: leaf && leaf.getEphemeralState && leaf.getEphemeralState(),
      previewScroll: v && v.previewMode && v.previewMode.getScroll && v.previewMode.getScroll() };`);
  return { before, rows, clicked: ix, rect: r, after };
});

await step('s07-reopen-alpha', async () => {
  await c.evaluate(`const f = app.vault.getAbstractFileByPath('Alpha.md'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(800);
  return c.evaluate(`return Array.from(document.querySelectorAll('.outgoing-link-pane .tree-item-self')).map(r => r.textContent.trim());`);
});

await step('s08-click-unresolved-row', async () => {
  const rows = await c.evaluate(`return Array.from(document.querySelectorAll('.outgoing-link-pane .tree-item-self')).map(r => r.textContent.trim());`);
  const ix = rows.findIndex(t => /^Missing$/.test(t));
  if (ix < 0) return { rows, skipped: 'no Missing row' };
  const r = await c.clickElement('.outgoing-link-pane .tree-item-self', { index: ix });
  await sleep(1200);
  return { clicked: ix, rect: r, missingFile: bytes('Missing.md'), vault: fs.readdirSync(VAULT).sort(),
    active: await c.evaluate(`return app.workspace.getActiveFile() && app.workspace.getActiveFile().path;`),
    modal: await c.evaluate(`const m = document.querySelector('.modal-container'); return m && m.textContent.trim().slice(0, 300);`) };
});

await step('s09-reopen-alpha-unlinked', async () => {
  await c.evaluate(`const f = app.vault.getAbstractFileByPath('Alpha.md'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(1500);
  await c.clickElement('.outgoing-link-pane .tree-item-self.is-collapsed', { index: 0 });
  await sleep(800);
  return c.evaluate(PROBE_JS + `
    const pane = paneOf('outgoing-link');
    const dom = dump(pane.querySelector('.outgoing-link-pane'), 0);
    const sections = Array.from(pane.children).map(ch => ({ cls: ch.className, rect: rect(ch), header: (ch.querySelector('.tree-item-self') || ch).textContent.trim().slice(0, 80) }));
    const matches = Array.from(pane.querySelectorAll('.search-result-file-match')).map(m => ({ cls: m.className, text: m.textContent.trim(), html: m.innerHTML.slice(0, 600), rect: rect(m), style: cs(m, ['font-size','padding','line-height','color','background-color','border-top','border-bottom']) }));
    const groups = Array.from(pane.querySelectorAll('.search-result')).map(g => ({ cls: g.className, title: (g.querySelector('.search-result-file-title') || g).textContent.trim(), flair: (g.querySelector('.tree-item-flair') || {}).textContent }));
    const button = Array.from(pane.querySelectorAll('button')).map(b => ({ cls: b.className, text: b.textContent.trim(), rect: rect(b), style: cs(b, ['font-size','padding','height','background-color','color','border-radius','box-shadow','font-weight','display','opacity']) }));
    const matched = Array.from(pane.querySelectorAll('.search-result-file-matched-text')).map(m => ({ text: m.textContent, style: cs(m, ['background-color','color','font-weight']) }));
    return { dom, sections, groups, matches, button, matched, viewState: leafOf('outgoing-link').getViewState() };`);
});

await step('s10-hover-match', async () => {
  const r = await c.hover('.outgoing-link-pane .search-result-file-match', 0);
  await sleep(400);
  return { pointer: r, ...(await c.evaluate(PROBE_JS + `
    const pane = paneOf('outgoing-link');
    const m = pane.querySelectorAll('.search-result-file-match')[0];
    if (!m) return { none: true };
    const b = m && m.querySelector('button');
    return { hovered: m && m.matches(':hover'), style: m && cs(m, ['background-color']), button: b && { visible: getComputedStyle(b).display !== 'none', opacity: getComputedStyle(b).opacity, rect: rect(b), style: cs(b, ['background-color','color','font-size','padding','height','border-radius']) },
      hoverButtons: Array.from(m.querySelectorAll('.search-result-hover-button')).map(h => ({ cls: h.className, label: h.getAttribute('aria-label'), icon: (h.querySelector('svg') || { getAttribute: () => null }).getAttribute('class'), display: getComputedStyle(h).display, opacity: getComputedStyle(h).opacity })) };`)) };
});

await step('s11-link-button', async () => {
  const before = bytes('Alpha.md');
  const info = await c.evaluate(PROBE_JS + `
    const pane = paneOf('outgoing-link');
    const ms = Array.from(pane.querySelectorAll('.search-result-file-match'));
    const first = ms.find(m => m.querySelector('button, .search-result-file-match-destination-file'));
    if (!first) return null;
    const b = first.querySelector('button, .search-result-file-match-destination-file');
    const r = b.getBoundingClientRect();
    return { text: first.textContent.trim(), matched: Array.from(first.querySelectorAll('.search-result-file-matched-text')).map(x => x.textContent), x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  if (!info) return { skipped: 'no Link button' };
  await c.clickAt(info.x, info.y);
  const changed = await waitForChange('Alpha.md', before, 4000);
  await sleep(800);
  return { info, changed, before, after: bytes('Alpha.md'), rowsNow: await c.evaluate(`return Array.from(document.querySelectorAll('.outgoing-link-pane .search-result-file-match')).map(m => m.textContent.trim());`),
    notice: await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(n => n.textContent.trim());`) };
});

await step('s11b-click-match-row', async () => {
  const info = await c.evaluate(PROBE_JS + `
    const pane = paneOf('outgoing-link');
    const m = pane.querySelectorAll('.search-result-file-match')[1]; if (!m) return null;
    const r = m.getBoundingClientRect(); return { text: m.textContent.trim().slice(0, 60), x: r.x + 40, y: r.y + 12 };`);
  if (!info) return { skipped: true };
  await c.clickAt(info.x, info.y);
  await sleep(800);
  return { info, after: await c.evaluate(`
    const leaf = app.workspace.activeLeaf; const v = leaf && leaf.view;
    return { active: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, viewType: v && v.getViewType(), mode: v && v.getMode && v.getMode(), cursor: v && v.editor && v.editor.getCursor(), sel: v && v.editor && v.editor.getSelection(), ephemeral: leaf.getEphemeralState && leaf.getEphemeralState() };`) };
});

await step('s12-collapse-sections', async () => {
  const r1 = await c.clickElement('.outgoing-link-pane > .tree-item-self, .outgoing-link-pane .tree-item-self.is-clickable', { index: 0 });
  await sleep(400);
  const a = await c.evaluate(PROBE_JS + `const pane = paneOf('outgoing-link'); return { top: Array.from(pane.children).map(ch => ({ cls: ch.className, rect: rect(ch) })), viewState: leafOf('outgoing-link').getViewState() };`);
  await c.clickElement('.outgoing-link-pane > .tree-item-self, .outgoing-link-pane .tree-item-self.is-clickable', { index: 0 });
  await sleep(400);
  return { clicked: r1, collapsed: a, reopened: await c.evaluate(PROBE_JS + `const pane = paneOf('outgoing-link'); return Array.from(pane.children).map(ch => ({ cls: ch.className, rect: rect(ch) }));`) };
});

await step('s12b-hover-links-header', async () => {
  const r = await c.hover('.outgoing-link-pane .tree-item-self', 0);
  await sleep(400);
  return { pointer: r, ...(await c.evaluate(PROBE_JS + `
    const pane = paneOf('outgoing-link');
    const h = pane.querySelectorAll('.tree-item-self')[0];
    const ic = h.querySelector('.collapse-icon');
    return { hovered: h.matches(':hover'), style: cs(h, ['background-color','color']), icon: ic && { rect: rect(ic), display: getComputedStyle(ic).display, opacity: getComputedStyle(ic).opacity, svg: ic.querySelector('svg') && rect(ic.querySelector('svg')) }, flair: cs(h.querySelector('.tree-item-flair'), ['color','background-color']) };`)) };
});

await step('s13-context-menu', async () => {
  await c.rightClickElement('.outgoing-link-pane .outgoing-link-item .tree-item-self, .outgoing-link-pane .tree-item-self', { index: 1 });
  await sleep(500);
  const items = await c.openMenuItems();
  await c.pressKey('Escape');
  await sleep(200);
  return { items };
});

await step('s14-backlinks-dom', async () => {
  await c.evaluate(`await app.commands.executeCommandById('backlink:open-backlinks'); return true;`);
  await sleep(1500);
  return c.evaluate(PROBE_JS + `
    const pane = paneOf('backlink');
    const headers = Array.from(pane.querySelectorAll('.nav-header, .nav-buttons-container, .nav-action-button')).map(h => ({ cls: h.className, label: h.getAttribute('aria-label'), icon: (h.querySelector('svg') || { getAttribute: () => null }).getAttribute('class'), rect: rect(h) }));
    const un = pane.querySelector('.backlink-pane .tree-item-self.is-collapsed'); if (un) { un.click(); await new Promise(r => setTimeout(r, 800)); }
    return { viewState: leafOf('backlink').getViewState(), headers, dom: dump(pane.querySelector('.backlink-pane'), 0) };`);
});

await step('s14b-backlinks-hover-match', async () => {
  const pane = '.backlink-pane';
  const r = await c.hover(pane + ' .search-result-file-match', 1);
  await sleep(400);
  const hov = await c.evaluate(PROBE_JS + `
    const p = document.querySelector('.backlink-pane'); const ms = p.querySelectorAll('.search-result-file-match'); const m = ms[1] || ms[0];
    const b = m.querySelector('button');
    return { text: m.textContent.trim(), hovered: m.matches(':hover'), bg: getComputedStyle(m).backgroundColor, button: b && { rect: rect(b), display: getComputedStyle(b).display, style: cs(b, ['background-color','color','font-size','padding','border-radius','height','line-height','font-weight','margin','position','top','right']) }, hoverButtons: Array.from(m.querySelectorAll('.search-result-hover-button')).map(h => ({ cls: h.className, rect: rect(h), display: getComputedStyle(h).display })) };`);
  const r2 = await c.hover(pane + ' .search-result-file-title', 0);
  await sleep(300);
  const titleHover = await c.evaluate(PROBE_JS + `const t = document.querySelector('.backlink-pane .search-result-file-title'); return { bg: getComputedStyle(t).backgroundColor, color: getComputedStyle(t).color, icon: rect(t.querySelector('.collapse-icon')), iconDisplay: getComputedStyle(t.querySelector('.collapse-icon')).display };`);
  return { pointer: r, hov, titleHover };
});

await step('s14c-backlinks-link-button', async () => {
  const before = bytes('Delta.md');
  const info = await c.evaluate(PROBE_JS + `
    const p = document.querySelector('.backlink-pane'); const b = p.querySelector('button.search-result-file-match-replace-button'); if (!b) return null;
    const m = b.closest('.search-result-file-match'); const mr = m.getBoundingClientRect();
    return { mx: mr.x + 30, my: mr.y + 10 };`);
  if (!info) return { skipped: 'no button' };
  await c.mouseMove(info.mx, info.my); await sleep(300);
  const br = await c.evaluate(PROBE_JS + `const b = document.querySelector('.backlink-pane button.search-result-file-match-replace-button'); const r = b.getBoundingClientRect(); return { rect: rect(b), x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  if (!br.rect[2]) return { skipped: 'button has no size', br };
  await c.clickAt(br.x, br.y);
  const changed = await waitForChange('Delta.md', before, 4000);
  await sleep(600);
  return { br, changed, before, after: bytes('Delta.md'), notice: await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(n => n.textContent.trim());`),
    rowsNow: await c.evaluate(`return Array.from(document.querySelectorAll('.backlink-pane .tree-item-self')).map(r => r.textContent.trim());`) };
});

await step('s15-quit', async () => { await c.quit(); return true; });
out.finishedAt = new Date().toISOString();
save();
process.exit(0);
