// The root tab strip's right end, driven over CDP: where the new-tab button,
// the tab-list button and the sidebar toggle sit against the tab (vertical
// centring), whether a divider follows the last tab when it is inactive, and
// the tab-list menu: its rows, icons, checks and separators, what picking a
// tab does, what Close all does, what Stack tabs toggles, what Bookmark all
// tabs opens. Screenshots of the strip and the menu go to SHOTS.
// Phases: fixtures | run   (node tab-list.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
const SHOTS = process.env.SHOTS || path.dirname(RESULTS);
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 170000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.tabList = all.tabList || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.tabList[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
async function step(name, fn) {
  out.order.push(name);
  if (Date.now() - T0 > BUDGET_MS) { out.steps[name] = { ok: false, error: 'skipped: time budget exhausted' }; save(); return null; }
  const t = Date.now();
  try { const v = await fn(); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e) }; }
  save();
  return out.steps[name];
}

if (phase === 'fixtures') {
  for (const n of ['Alpha', 'Beta', 'Gamma', 'Delta']) fs.writeFileSync(path.join(VAULT, n + '.md'), `# ${n}\n\nA note.\n`);
  out.fixtures = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}

const c = await connect();
out.vaultPath = await c.vaultPath();
save();

const PROBE_JS = `
  const P = ['display', 'opacity', 'background-color', 'color', 'border-radius', 'padding', 'margin', 'width', 'height', 'align-items', 'align-self', 'justify-content', 'gap', 'position', 'top', 'bottom', 'flex', 'min-width', 'max-width', 'font-size', 'font-weight', 'box-shadow', 'border'];
  const cs = (el, pseudo) => { if (!el) return null; const s = getComputedStyle(el, pseudo || null); const o = {}; for (const p of P) o[p] = s.getPropertyValue(p); return o; };
  const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x: +b.x.toFixed(2), y: +b.y.toFixed(2), w: +b.width.toFixed(2), h: +b.height.toFixed(2) }; };
  const one = (sel) => { const el = document.querySelector(sel); return el ? { sel, cls: el.className, aria: el.getAttribute('aria-label'), rect: rect(el), style: cs(el), svg: (() => { const s = el.querySelector('svg'); return s && { cls: s.getAttribute('class'), rect: rect(s) }; })() } : { sel, missing: true }; };
  const strip = () => ({
    container: one('.mod-root .workspace-tab-header-container'),
    inner: one('.mod-root .workspace-tab-header-container-inner'),
    tabs: Array.from(document.querySelectorAll('.mod-root .workspace-tab-header')).map(h => ({ aria: h.getAttribute('aria-label'), active: h.classList.contains('is-active'), rect: rect(h), innerRect: rect(h.querySelector('.workspace-tab-header-inner')), after: cs(h.querySelector('.workspace-tab-header-inner'), '::after') })),
    newTab: one('.mod-root .workspace-tab-header-new-tab'),
    newTabIcon: one('.mod-root .workspace-tab-header-new-tab .clickable-icon'),
    spacer: one('.mod-root .workspace-tab-header-spacer'),
    tabList: one('.mod-root .workspace-tab-header-tab-list'),
    tabListIcon: one('.mod-root .workspace-tab-header-tab-list .clickable-icon'),
    toggleRight: one('.sidebar-toggle-button.mod-right'),
    toggleRightIcon: one('.sidebar-toggle-button.mod-right .clickable-icon'),
    toggleLeft: one('.sidebar-toggle-button.mod-left'),
    toggleLeftIcon: one('.sidebar-toggle-button.mod-left .clickable-icon'),
  });
  const menu = () => Array.from(document.querySelectorAll('.menu')).map(m => ({ cls: m.className, rect: rect(m), style: cs(m), scroll: cs(m.querySelector('.menu-scroll')), items: Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map(it => ({
    cls: it.className, rect: rect(it), style: cs(it),
    title: (it.querySelector('.menu-item-title') || {}).textContent, icon: (() => { const s = it.querySelector('.menu-item-icon svg'); return s && s.getAttribute('class'); })(),
    iconRect: rect(it.querySelector('.menu-item-icon')), titleStyle: cs(it.querySelector('.menu-item-title')),
    checked: it.classList.contains('is-checked') || !!it.querySelector('.menu-item-icon.mod-checked, .menu-item-check'),
    section: it.getAttribute('data-section'), html: it.innerHTML.slice(0, 300) })) }));
`;

const shot = async (name, sel) => {
  const r = await c.evaluate(`const el = document.querySelector(${JSON.stringify(sel)}); if (!el) return null; const b = el.getBoundingClientRect(); return { x: Math.max(0, b.x - 10), y: Math.max(0, b.y - 10), w: b.width + 20, h: b.height + 20 };`);
  if (!r) return { missing: sel };
  const png = await c.send('Page.captureScreenshot', { format: 'png', clip: { x: r.x, y: r.y, width: r.w, height: r.h, scale: 2 } });
  fs.mkdirSync(SHOTS, { recursive: true });
  const file = path.join(SHOTS, `tab-list-${name}.png`);
  fs.writeFileSync(file, Buffer.from(png.data, 'base64'));
  return { file, clip: r };
};
const park = async () => { const ed = await c.rect('.mod-root .view-content'); await c.mouseMove(ed.cx, ed.cy); await sleep(200); };
const openMenu = async () => { await c.clickElement('.mod-root .workspace-tab-header-tab-list .clickable-icon, .mod-root .workspace-tab-header-tab-list'); await sleep(500); return c.evaluate(PROBE_JS + 'return menu();'); };
const clickItem = async (title) => {
  // A prefix match: "Bookmark 4 tabs..." counts the tabs, "Stack tabs" becomes "Unstack tabs".
  const ix = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).findIndex(it => ((it.querySelector('.menu-item-title')||{}).textContent||'').trim().startsWith(${JSON.stringify(title)}));`);
  if (ix < 0) throw new Error('no menu item ' + title);
  await c.clickElement('.menu .menu-item', { index: ix }); await sleep(600);
  return ix;
};
const tabsNow = () => c.evaluate(`const l = []; app.workspace.iterateRootLeaves(x => l.push({ file: x.view.file && x.view.file.path, type: x.view.getViewType(), active: x === app.workspace.activeLeaf })); return l;`);

await step('s01-open', async () => {
  await c.evaluate(`
    app.workspace.leftSplit.expand(); app.workspace.rightSplit.collapse();
    for (const n of ['Alpha', 'Beta', 'Gamma', 'Delta']) { const f = app.vault.getAbstractFileByPath(n + '.md'); await app.workspace.getLeaf('tab').openFile(f); }
    const leaves = []; app.workspace.iterateRootLeaves(l => leaves.push(l));
    for (const l of leaves) if (!l.view.file) l.detach();
    const rest = []; app.workspace.iterateRootLeaves(l => rest.push(l));
    app.workspace.setActiveLeaf(rest[0], { focus: true });
    return true;`);
  await sleep(600); await park();
  return { tabs: await tabsNow(), strip: await c.evaluate(PROBE_JS + 'return strip();'), shot: await shot('strip-first-active', '.mod-root .workspace-tab-header-container') };
});
await step('s02-strip-last-active', async () => {
  await c.evaluate(`const rest = []; app.workspace.iterateRootLeaves(l => rest.push(l)); app.workspace.setActiveLeaf(rest[3], { focus: true }); return true;`);
  await sleep(400); await park();
  return { strip: await c.evaluate(PROBE_JS + 'return strip();'), shot: await shot('strip-last-active', '.mod-root .workspace-tab-header-container') };
});
await step('s03-menu', async () => {
  const m = await openMenu();
  const s = await shot('menu', '.menu');
  // Hover the third row for its hover style.
  await c.hover('.menu .menu-item', 2); await sleep(300);
  const hovered = await c.evaluate(PROBE_JS + 'return menu();');
  const s2 = await shot('menu-hover', '.menu');
  await c.pressKey('Escape'); await sleep(300);
  return { menu: m, hovered, shot: s, shotHover: s2, closedByEscape: await c.evaluate(`return document.querySelectorAll('.menu').length;`) };
});
await step('s04-pick-tab', async () => {
  await openMenu();
  const titles = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(it => ((it.querySelector('.menu-item-title')||{}).textContent||'').trim());`);
  const before = await tabsNow();
  const ix = await clickItem('Beta');
  const after = await tabsNow();
  const strip = await shot('strip-after-pick', '.mod-root .workspace-tab-header-container');
  // Put the four back if the pick lost any, so the later steps see four tabs.
  if (after.length < 4) await c.evaluate(`for (const n of ['Beta', 'Gamma', 'Delta']) { const f = app.vault.getAbstractFileByPath(n + '.md'); if (!app.workspace.getLeavesOfType('markdown').some(l => l.view.file && l.view.file.path === f.path)) await app.workspace.getLeaf('tab').openFile(f); } return true;`);
  await sleep(500);
  return { titles, ix, before, after, strip, restored: await tabsNow(), menusOpen: await c.evaluate(`return document.querySelectorAll('.menu').length;`) };
});
await step('s05-stack', async () => {
  const before = await c.evaluate(`return { stacked: app.workspace.activeTabGroup.isStacked, cls: app.workspace.activeTabGroup.containerEl.className };`);
  await openMenu(); await clickItem('Stack tabs');
  const after = await c.evaluate(`return { stacked: app.workspace.activeTabGroup.isStacked, cls: app.workspace.activeTabGroup.containerEl.className };`);
  const m = await openMenu();
  const s = await shot('menu-stacked', '.menu');
  await clickItem('Unstack tabs').catch(async () => clickItem('Stack tabs'));
  await sleep(300); await c.pressKey('Escape'); await sleep(200);
  const restored = await c.evaluate(`return { stacked: app.workspace.activeTabGroup.isStacked };`);
  return { before, after, menuStacked: m.map(x => x.items.map(i => ({ t: i.title, icon: i.icon, checked: i.checked }))), shot: s, restored };
});
await step('s06-bookmark-all', async () => {
  await openMenu(); await clickItem('Bookmark ');
  const modal = await c.evaluate(`const m = document.querySelector('.modal-container'); return m && { title: (m.querySelector('.modal-title')||{}).textContent, text: m.textContent.slice(0, 400), inputs: Array.from(m.querySelectorAll('input')).map(i => ({ type: i.type, value: i.value, placeholder: i.placeholder })), buttons: Array.from(m.querySelectorAll('button')).map(b => b.textContent) };`);
  const s = modal ? await shot('bookmark-all-modal', '.modal') : null;
  await c.pressKey('Escape'); await sleep(300);
  return { modal, shot: s, bookmarksJson: fs.existsSync(path.join(VAULT, '.obsidian/bookmarks.json')) ? fs.readFileSync(path.join(VAULT, '.obsidian/bookmarks.json'), 'utf8') : null };
});
await step('s07-close-all', async () => {
  const before = await tabsNow();
  const menusBefore = await c.evaluate(`return { menus: document.querySelectorAll('.menu').length, modal: !!document.querySelector('.modal-container') };`);
  if (menusBefore.modal) { await c.pressKey('Escape'); await sleep(300); }
  await openMenu(); await clickItem('Close all');
  await sleep(500);
  return { before, after: await tabsNow(), modal: await c.evaluate(`return !!document.querySelector('.modal-container');`), strip: await c.evaluate(PROBE_JS + 'return strip();'), shot: await shot('strip-after-close-all', '.mod-root .workspace-tab-header-container') };
});
await step('s08-menu-one-empty', async () => { const m = await openMenu(); await c.pressKey('Escape'); return { items: m.map(x => x.items.map(i => ({ t: i.title, icon: i.icon, checked: i.checked, cls: i.cls }))) }; });

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
