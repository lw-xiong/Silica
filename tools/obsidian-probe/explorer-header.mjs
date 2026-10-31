// The Files pane's header, driven over CDP: the five nav-action buttons, what
// "Change sort order" offers and what each order does to the rows and to
// workspace.json / app.json, how "Auto-reveal current file" toggles and what
// opening a note in a collapsed folder does with it off and on, what "Expand
// all" turns into once pressed, and where the header's "New note" and "New
// folder" put their file.
// Phases: fixtures | run   (node explorer-header.mjs <phase>)
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
all.explorerHeader = all.explorerHeader || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.explorerHeader[phase] = out;
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
const listVault = () => {
  const outp = [];
  const walk = (dir, rel) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      if (e.name.startsWith('.')) continue;
      const r = rel ? rel + '/' + e.name : e.name;
      outp.push(e.isDirectory() ? r + '/' : r);
      if (e.isDirectory()) walk(path.join(dir, e.name), r);
    }
  };
  walk(VAULT, '');
  return outp.sort();
};

// Files are created in this order (so their birth times rise in it) and then
// given modification times in a different order, so the two time sorts can
// be told apart from each other and from the alphabetical ones.
const CREATED = ['Alpha.md', 'beta.md', 'Gamma.md', 'n2.md', 'n10.md', 'Projects/Plan.md', 'Projects/Deep/Inner.md', 'Archive/old.md', 'Projects/Budget.md', 'cover.png'];
const MTIME_ORDER = ['n10.md', 'Archive/old.md', 'Alpha.md', 'Projects/Budget.md', 'cover.png', 'Gamma.md', 'Projects/Deep/Inner.md', 'beta.md', 'Projects/Plan.md', 'n2.md'];

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.mkdirSync(vp('Projects/Deep'), { recursive: true });
  fs.mkdirSync(vp('Archive'), { recursive: true });
  fs.mkdirSync(vp('Empty'), { recursive: true });
  const t = Date.now();
  for (const rel of CREATED) {
    if (rel.endsWith('.png')) fs.writeFileSync(vp(rel), Buffer.from('89504e470d0a1a0a0000000d49484452', 'hex'));
    else fs.writeFileSync(vp(rel), `# ${path.basename(rel, '.md')}\n\nBody of ${rel}.\n`);
    const d = Date.now(); while (Date.now() - d < 30) { /* spin so birth times differ */ }
  }
  MTIME_ORDER.forEach((rel, i) => {
    const when = new Date(t - (MTIME_ORDER.length - i) * 3600 * 1000);
    fs.utimesSync(vp(rel), when, when);
  });
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'global-search': true, graph: true, backlink: true, bookmarks: true, 'command-palette': true, switcher: true, outline: true, 'tag-pane': true, 'page-preview': true }, null, 2));
  out.wrote = listVault();
  out.stats = Object.fromEntries(CREATED.map((rel) => { const s = fs.statSync(vp(rel)); return [rel, { birth: s.birthtimeMs, mtime: s.mtimeMs }]; }));
  save();
  process.exit(0);
}

if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
out.version = await c.evaluate(`return { ua: navigator.userAgent, platform: navigator.platform };`);
save();

const DOM_JS = `
  const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; };
  const cs = (el, props) => { if (!el) return null; const s = getComputedStyle(el); const o = {}; for (const p of props) o[p] = s.getPropertyValue(p); return o; };
  const svgName = (el) => { const s = el && el.querySelector('svg'); return s ? (s.getAttribute('class') || '') : null; };
  const pane = app.workspace.getLeavesOfType('file-explorer')[0].view.containerEl;
  const header = () => Array.from(pane.querySelectorAll('.nav-header .nav-action-button, .nav-buttons-container .clickable-icon')).map(b => ({ aria: b.getAttribute('aria-label'), cls: b.className, icon: svgName(b), rect: rect(b) }));
  const rows = () => Array.from(pane.querySelectorAll('.nav-files-container .tree-item')).map(it => {
    const self = it.querySelector(':scope > .tree-item-self');
    let depth = 0, e = it.parentElement; while (e && e !== pane) { if (e.classList.contains('tree-item-children')) depth++; e = e.parentElement; }
    return { name: (self.querySelector('.tree-item-inner') || {}).textContent, folder: self.classList.contains('nav-folder-title'), depth, collapsed: it.classList.contains('is-collapsed'), active: self.classList.contains('is-active'), renaming: self.classList.contains('is-being-renamed'), cls: it.className, visible: rect(self).h > 0 };
  });
  const leaf = app.workspace.getLeavesOfType('file-explorer')[0];
  const state = () => ({ view: leaf.view.getState(), appFileSortOrder: app.vault.getConfig('fileSortOrder'), sortOrderField: leaf.view.sortOrder === undefined ? 'undefined' : String(leaf.view.sortOrder), autoRevealField: leaf.view.autoReveal === undefined ? 'undefined' : String(leaf.view.autoReveal) });
`;
const snapshot = (extra = '') => c.evaluate(DOM_JS + `return { header: header(), rows: rows(), state: state(), ${extra} };`);
const button = (aria) => `.workspace-leaf-content[data-type="file-explorer"] .nav-action-button[aria-label="${aria}"]`;
async function savedLayout() {
  const before = bytes('.obsidian/workspace.json');
  await c.evaluate(`if (app.workspace.saveLayout) { await app.workspace.saveLayout(); } else { app.workspace.requestSaveLayout(); } return true;`);
  await waitForChange('.obsidian/workspace.json', before, 4000);
  await sleep(300);
  const text = bytes('.obsidian/workspace.json');
  let explorer = null;
  try {
    const walk = (n) => { if (!n || typeof n !== 'object') return; if (n.type === 'leaf' && n.state && n.state.type === 'file-explorer') explorer = n.state; for (const v of Object.values(n)) walk(v); };
    walk(JSON.parse(text).left);
  } catch { /* keep null */ }
  return { explorerLeaf: explorer, appJson: bytes('.obsidian/app.json') };
}

await step('s01-header', async (r) => {
  await c.evaluate(`app.workspace.leftSplit.expand(); app.workspace.revealLeaf(app.workspace.getLeavesOfType('file-explorer')[0]); return true;`);
  await sleep(500);
  r.initial = await snapshot(`navHeader: (() => { const h = pane.querySelector('.nav-header'); const b = pane.querySelector('.nav-buttons-container'); return { header: { rect: rect(h), style: cs(h, ['height', 'padding', 'display', 'gap', 'border-bottom']) }, buttons: { rect: rect(b), style: cs(b, ['display', 'gap', 'padding', 'justify-content', 'flex-wrap']) } }; })()`);
  r.layout = await savedLayout();
  return r;
});

await step('s02-expand-all', async (r) => {
  r.before = (await snapshot()).rows;
  await c.clickElement(button('Expand all'));
  await sleep(400);
  r.afterExpand = await snapshot();
  const label = r.afterExpand.header[4].aria;
  await c.clickElement(button(label));
  await sleep(400);
  r.afterSecondPress = await snapshot();
  // Expand one folder by hand and see which label the button then wears.
  await c.clickElement('.workspace-leaf-content[data-type="file-explorer"] .nav-folder-title[data-path="Projects"]');
  await sleep(300);
  r.afterOneFolderOpen = await snapshot();
  await c.clickElement(button(r.afterOneFolderOpen.header[4].aria));
  await sleep(300);
  r.afterPressWithOneOpen = await snapshot();
  await c.clickElement(button(r.afterPressWithOneOpen.header[4].aria));
  await sleep(300);
  r.afterPressAgain = await snapshot();
  r.layout = await savedLayout();
  return r;
});

await step('s03-sort-menu', async (r) => {
  await c.clickElement(button('Change sort order'));
  await sleep(400);
  r.menu = await c.openMenuItems();
  r.menuDom = await c.evaluate(`const m = document.querySelector('.menu'); if (!m) return null; return { html: m.outerHTML.slice(0, 6000), items: Array.from(m.querySelectorAll('.menu-item')).map(i => ({ title: (i.querySelector('.menu-item-title') || {}).textContent, cls: i.className, icon: (i.querySelector('svg') || {}).getAttribute ? (i.querySelector('svg') || { getAttribute: () => null }).getAttribute('class') : null, checked: i.classList.contains('is-checked') || i.classList.contains('mod-checked') || i.classList.contains('is-active') })) };`);
  await c.pressKey('Escape');
  await sleep(200);
  r.orders = [];
  const labels = r.menu.flat().filter((l) => l !== '---');
  for (const label of labels) {
    await c.clickElement(button('Change sort order'));
    await sleep(300);
    const items = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(i => (i.querySelector('.menu-item-title') || {}).textContent);`);
    const ix = items.indexOf(label);
    if (ix < 0) { r.orders.push({ label, error: 'not in menu', items }); await c.pressKey('Escape'); continue; }
    await c.clickElement('.menu .menu-item', { index: ix });
    await sleep(400);
    const snap = await snapshot();
    const layout = await savedLayout();
    r.orders.push({ label, rows: snap.rows.map((x) => (x.folder ? x.name + '/' : x.name)), state: snap.state, explorerLeaf: layout.explorerLeaf, appJson: layout.appJson, checkedNow: await (async () => { await c.clickElement(button('Change sort order')); await sleep(250); const v = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).filter(i => /is-checked|mod-checked|is-active/.test(i.className)).map(i => (i.querySelector('.menu-item-title') || {}).textContent);`); await c.pressKey('Escape'); await sleep(150); return v; })() });
  }
  // Back to the first order.
  await c.clickElement(button('Change sort order'));
  await sleep(300);
  await c.clickElement('.menu .menu-item', { index: 0 });
  await sleep(300);
  return r;
});

await step('s04-auto-reveal', async (r) => {
  // Everything collapsed, auto-reveal in its initial state.
  const collapseIfOpen = async () => { const s = await snapshot(); if (s.header[4].aria === 'Collapse all') { await c.clickElement(button('Collapse all')); await sleep(300); } };
  await collapseIfOpen();
  r.initialButton = (await snapshot()).header[3];
  r.initialState = (await snapshot()).state;
  await c.evaluate(`await app.workspace.openLinkText('Inner', '', false); return true;`);
  await sleep(600);
  r.openedWithInitial = await snapshot(`activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path`);
  // The command, on demand.
  await c.evaluate(`app.commands.executeCommandById('file-explorer:reveal-active-file'); return true;`);
  await sleep(500);
  r.afterRevealCommand = await snapshot();
  await collapseIfOpen();
  // Toggle it.
  await c.clickElement(button('Auto-reveal current file'));
  await sleep(400);
  r.afterToggle = await snapshot();
  r.layoutAfterToggle = await savedLayout();
  await c.evaluate(`await app.workspace.openLinkText('old', '', false); return true;`);
  await sleep(600);
  r.openedWithToggled = await snapshot(`activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path`);
  // And back.
  await c.clickElement(button('Auto-reveal current file'));
  await sleep(400);
  r.afterToggleBack = await snapshot();
  r.layoutAfterToggleBack = await savedLayout();
  return r;
});

await step('s05-new-note', async (r) => {
  // A note in a folder is active (newFileLocation unset = root).
  await c.evaluate(`await app.workspace.openLinkText('Plan', '', false); return true;`);
  await sleep(500);
  r.config = await c.evaluate(`return { newFileLocation: app.vault.getConfig('newFileLocation'), newFileFolderPath: app.vault.getConfig('newFileFolderPath') };`);
  const before = listVault();
  await c.clickElement(button('New note'));
  await sleep(900);
  r.created = listVault().filter((f) => !before.includes(f));
  r.after = await snapshot(`activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, activeViewType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType(), leaves: app.workspace.getLeavesOfType('markdown').length, focused: document.activeElement && (document.activeElement.className + ' ' + document.activeElement.tagName), inlineTitle: (document.querySelector('.inline-title') || {}).textContent, titleFocused: document.activeElement && document.activeElement.classList.contains('inline-title')`);
  await c.pressKey('Escape');
  await sleep(200);
  // With the location set to the current folder.
  await c.evaluate(`app.vault.setConfig('newFileLocation', 'current'); await app.workspace.openLinkText('Plan', '', false); return true;`);
  await sleep(500);
  const before2 = listVault();
  await c.clickElement(button('New note'));
  await sleep(900);
  r.createdInCurrent = listVault().filter((f) => !before2.includes(f));
  r.afterCurrent = await snapshot(`activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path`);
  await c.pressKey('Escape');
  await c.evaluate(`app.vault.setConfig('newFileLocation', 'root'); return true;`);
  await sleep(300);
  return r;
});

await step('s06-new-folder', async (r) => {
  await c.evaluate(`await app.workspace.openLinkText('Plan', '', false); return true;`);
  await sleep(400);
  // Make sure Projects is expanded so the new folder is on screen if it lands there.
  const s = await snapshot();
  if (s.header[4].aria === 'Expand all') { await c.clickElement(button('Expand all')); await sleep(300); }
  const before = listVault();
  await c.clickElement(button('New folder'));
  await sleep(900);
  r.created = listVault().filter((f) => !before.includes(f));
  r.after = await snapshot(`focused: document.activeElement && (document.activeElement.className + ' ' + document.activeElement.tagName), editing: (() => { const e = document.querySelector('.is-being-renamed .tree-item-inner'); return e ? { text: e.textContent, editable: e.getAttribute('contenteditable') } : null; })()`);
  await c.typeText('Fresh');
  await c.pressKey('Enter');
  await sleep(600);
  r.afterRename = { files: listVault().filter((f) => !before.includes(f)), rows: (await snapshot()).rows.map((x) => (x.folder ? x.name + '/' : x.name)) };
  // Second press: Escape on the inline rename.
  const before2 = listVault();
  await c.clickElement(button('New folder'));
  await sleep(700);
  r.createdSecond = listVault().filter((f) => !before2.includes(f));
  await c.pressKey('Escape');
  await sleep(600);
  r.afterEscape = listVault().filter((f) => !before2.includes(f));
  return r;
});

await step('s07-tooltips', async (r) => {
  await c.hover(button('New note'));
  await sleep(1200);
  r.tooltip = await c.evaluate(`const t = document.querySelector('.tooltip'); return t ? { text: t.textContent, cls: t.className, rect: (b => ({ x: b.x, y: b.y, w: b.width, h: b.height }))(t.getBoundingClientRect()) } : null;`);
  return r;
});

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
