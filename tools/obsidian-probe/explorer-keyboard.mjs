// The Files pane's keyboard, driven over CDP: what a click on a row does to
// focus, which rows carry `has-focus` and `is-active`, what Up, Down, Left,
// Right, Enter, Space, Home, End and Escape do on a file row and on a folder
// row (collapsed and open), whether walking the rows opens the files, what
// the ends of the list do, what `file-explorer:open` and
// `file-explorer:reveal-active-file` focus, and whether typing a letter jumps.
// Phases: fixtures | run   (node explorer-keyboard.mjs <phase>)
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
all.explorerKeyboard = all.explorerKeyboard || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.explorerKeyboard[phase] = out;
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

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.mkdirSync(vp('Projects/Deep'), { recursive: true });
  fs.mkdirSync(vp('Archive'), { recursive: true });
  for (const rel of ['Alpha.md', 'beta.md', 'Gamma.md', 'Projects/Plan.md', 'Projects/Deep/Inner.md', 'Projects/Budget.md', 'Archive/old.md']) {
    fs.writeFileSync(vp(rel), `# ${path.basename(rel, '.md')}\n\nBody of ${rel}.\n`);
  }
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'global-search': true, graph: true, backlink: true, bookmarks: true, 'command-palette': true, switcher: true, outline: true, 'tag-pane': true, 'page-preview': true }, null, 2));
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
  const leaf = app.workspace.getLeavesOfType('file-explorer')[0];
  const pane = leaf.view.containerEl;
  const rows = () => Array.from(pane.querySelectorAll('.nav-files-container .tree-item')).map(it => {
    const self = it.querySelector(':scope > .tree-item-self');
    return { name: (self.querySelector('.tree-item-inner') || {}).textContent, folder: self.classList.contains('nav-folder-title'), collapsed: it.classList.contains('is-collapsed'), active: self.classList.contains('is-active'), focus: self.classList.contains('has-focus'), selfCls: self.className };
  });
  const ae = document.activeElement;
  const focusInfo = () => ({ tag: ae && ae.tagName, cls: ae && (typeof ae.className === 'string' ? ae.className : ''), tabindex: ae && ae.getAttribute('tabindex'), inPane: !!(ae && pane.contains(ae)), text: ae && (ae.textContent || '').slice(0, 40) });
  const focused = () => { const r = rows().filter(x => x.focus).map(x => x.name); return r; };
  const vis = () => rows().map(x => (x.folder ? x.name + '/' : x.name) + (x.collapsed ? ' c' : '') + (x.active ? ' *' : '') + (x.focus ? ' F' : ''));
  const state = () => ({ activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, activeLeafType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType(), focusedRow: focused(), activeElement: focusInfo(), rows: vis(), containerTabindex: (pane.querySelector('.nav-files-container') || {}).getAttribute ? pane.querySelector('.nav-files-container').getAttribute('tabindex') : null, viewFocus: leaf.view.tree ? { focusedItem: leaf.view.tree.focusedItem ? (leaf.view.tree.focusedItem.file ? leaf.view.tree.focusedItem.file.path : 'no file') : null, keys: Object.keys(leaf.view.tree).slice(0, 40) } : 'no tree' });
`;
const snap = () => c.evaluate(DOM_JS + `return state();`);
const row = (name, folder) => `.workspace-leaf-content[data-type="file-explorer"] .${folder ? 'nav-folder-title' : 'nav-file-title'}[data-path="${name}"]`;
async function press(key, mods) { await c.pressKey(key, mods); await sleep(350); return snap(); }

await step('s01-click-focus', async (r) => {
  await c.evaluate(`app.workspace.leftSplit.expand(); app.workspace.revealLeaf(app.workspace.getLeavesOfType('file-explorer')[0]); await app.workspace.openLinkText('Gamma', '', false); return true;`);
  await sleep(500);
  r.beforeClick = await snap();
  await c.clickElement(row('Alpha.md', false));
  await sleep(500);
  r.afterClickFile = await snap();
  r.treeProto = await c.evaluate(`const t = app.workspace.getLeavesOfType('file-explorer')[0].view.tree; if (!t) return null; const names = new Set(); let p = Object.getPrototypeOf(t); while (p && p !== Object.prototype) { Object.getOwnPropertyNames(p).forEach(n => names.add(n)); p = Object.getPrototypeOf(p); } return { own: Object.keys(t), proto: Array.from(names) };`);
  return r;
});

// Clicking a row that opens a file hands focus to the editor (s01), so the
// keyboard tests enter the tree through `reveal-active-file`, which leaves
// the active file's row focused with the explorer leaf active.
async function enterTree(name) {
  await c.evaluate(`await app.workspace.openLinkText(${JSON.stringify(name)}, '', false); return true;`);
  await sleep(400);
  await c.evaluate(`app.commands.executeCommandById('file-explorer:reveal-active-file'); return true;`);
  await sleep(400);
  return snap();
}
async function collapseAll() {
  await c.evaluate(`const v = app.workspace.getLeavesOfType('file-explorer')[0].view; v.tree.setCollapseAll(true); return true;`);
  await sleep(300);
}

await step('s02-arrows-on-files', async (r) => {
  await collapseAll();
  r.start = await enterTree('Alpha');
  r.down1 = await press('ArrowDown');
  r.down2 = await press('ArrowDown');
  r.downPastEnd = await press('ArrowDown');
  r.up1 = await press('ArrowUp');
  r.home = await press('Home');
  r.end = await press('End');
  r.pageUp = await press('PageUp');
  await c.clickElement(row('Alpha.md', false)); // Alpha is active: the explorer keeps focus
  await sleep(300);
  r.clickActiveRow = await snap();
  r.upFromAlpha = await press('ArrowUp');
  r.upAgain = await press('ArrowUp');
  r.upPastStart = await press('ArrowUp');
  return r;
});

await step('s03-enter-space-on-file', async (r) => {
  r.start = await enterTree('Alpha');
  r.down = await press('ArrowDown');
  r.enter = await press('Enter');
  r.start2 = await enterTree('beta');
  r.space = await press('Space');
  r.ctrlEnter = await press('Enter', { ctrl: true });
  return r;
});

await step('s04-folders', async (r) => {
  await collapseAll();
  r.start = await enterTree('Alpha');
  r.upOntoFolder = await press('ArrowUp');        // Projects, collapsed
  r.rightOnCollapsed = await press('ArrowRight');  // expands?
  r.rightOnOpen = await press('ArrowRight');       // into the first child?
  r.downIntoChild = await press('ArrowDown');
  r.leftOnChild = await press('ArrowLeft');        // to the parent? collapses it?
  r.leftOnOpenFolder = await press('ArrowLeft');
  r.leftOnCollapsed = await press('ArrowLeft');
  r.enterOnFolder = await press('Enter');
  r.enterOnFolderAgain = await press('Enter');
  r.spaceOnFolder = await press('Space');
  r.spaceOnFolderAgain = await press('Space');
  // A folder clicked with the mouse: toggled, and the keyboard position?
  await collapseAll();
  await enterTree('Alpha');
  const before = await snap();
  await c.clickElement(row('Archive', true));
  await sleep(400);
  r.clickFolder = { before, after: await snap() };
  r.downAfterFolderClick = await press('ArrowDown');
  r.enterOnOld = await press('Enter');
  return r;
});

await step('s05-commands', async (r) => {
  // Focus the editor, then the command.
  await c.evaluate(`await app.workspace.openLinkText('Inner', '', false); app.workspace.activeLeaf.view.editor.focus(); return true;`);
  await sleep(400);
  r.editorFocused = await snap();
  await c.evaluate(`app.commands.executeCommandById('file-explorer:open'); return true;`);
  await sleep(500);
  r.afterOpenCommand = await snap();
  r.downAfterOpen = await press('ArrowDown');
  await c.evaluate(`app.workspace.activeLeaf.view.editor && app.workspace.activeLeaf.view.editor.focus(); return true;`);
  await sleep(300);
  await c.evaluate(`app.commands.executeCommandById('file-explorer:reveal-active-file'); return true;`);
  await sleep(500);
  r.afterRevealCommand = await snap();
  r.downAfterReveal = await press('ArrowDown');
  return r;
});

await step('s06-escape-and-typing', async (r) => {
  r.start = await enterTree('Alpha');
  r.typedG = await press('g');
  r.escape = await press('Escape');
  r.downAfterEscape = await press('ArrowDown');
  r.tab = await press('Tab');
  r.start2 = await enterTree('Alpha');
  r.f2 = await press('F2');
  r.f2Escape = await press('Escape');
  r.delete = await press('Delete');
  r.deleteDialog = await c.evaluate(`const m = document.querySelector('.modal'); return m ? { title: (m.querySelector('.modal-title') || {}).textContent, text: (m.querySelector('.modal-content') || {}).textContent, buttons: Array.from(m.querySelectorAll('button')).map(b => b.textContent) } : null;`);
  await press('Escape');
  return r;
});

await step('s07-row-focus-style', async (r) => {
  await c.clickElement(row('Alpha.md', false));
  await sleep(300);
  r.styles = await c.evaluate(`const cs = (el, props) => { if (!el) return null; const s = getComputedStyle(el); const o = {}; for (const p of props) o[p] = s.getPropertyValue(p); return o; }; const P = ['background-color', 'color', 'box-shadow', 'outline']; const f = document.querySelector('.nav-file-title.has-focus'); const a = document.querySelector('.nav-file-title.is-active'); const plain = document.querySelector('.nav-file-title:not(.is-active):not(.has-focus)'); return { focusRow: f && { cls: f.className, style: cs(f, P) }, activeRow: a && { cls: a.className, style: cs(a, P) }, plain: plain && cs(plain, P), vars: (() => { const s = getComputedStyle(document.body); return { focus: s.getPropertyValue('--nav-item-background-active'), hover: s.getPropertyValue('--nav-item-background-hover'), selected: s.getPropertyValue('--nav-item-background-selected'), colorSel: s.getPropertyValue('--nav-item-color-selected'), colorActive: s.getPropertyValue('--nav-item-color-active') }; })() };`);
  return r;
});

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
