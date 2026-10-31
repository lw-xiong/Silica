// Tab-group splits and the Bookmarks core plugin, driven over CDP.
// Splits: what `workspace:split-vertical` / `split-horizontal` put in the layout
// (does the new group get a copy of the leaf, where focus goes, the nesting
// `workspace.json` records), what the resize handle writes, what
// `editor:focus-left/right/top/bottom` do, what closing a group's last tab and
// `workspace:close-tab-group` leave behind, where `editor:open-link-in-new-split`
// opens, and whether a nested layout survives a restart byte for byte.
// Bookmarks: the bytes `bookmarks.json` takes for every item type, the modal
// each command opens, what `bookmark-current-section` writes into the note,
// the pane's DOM, and the file menu's labels.
// Phases: fixtures | run | after-restart   (node splits-bookmarks.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.splitsBookmarks = all.splitsBookmarks || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.splitsBookmarks[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

async function step(name, fn, budgetMs = 20000) {
  out.order.push(name);
  const t = Date.now();
  const partial = {};
  try {
    const v = await Promise.race([fn(partial), sleep(budgetMs).then(() => { throw new Error('step budget exceeded'); })]);
    out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) };
  } catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save();
  return out.steps[name];
}

const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
const mtime = (rel) => { try { return Math.round(fs.statSync(vp(rel)).mtimeMs); } catch { return null; } };
async function waitForChange(rel, before, timeoutMs = 6000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) { if (bytes(rel) !== before) { await sleep(600); return true; } await sleep(200); }
  return false;
}

// ---- fixtures -------------------------------------------------------------
if (phase === 'fixtures') {
  fs.mkdirSync(vp('sub'), { recursive: true });
  fs.writeFileSync(vp('A.md'), '# Alpha\n\nLink to [[C]] and [[sub/D|Dee]].\n\nA paragraph to bookmark as a block.\n\n## Second heading\n\nMore text alpha.\n');
  fs.writeFileSync(vp('B.md'), '# Beta\n\nBeta links [[A]].\n');
  fs.writeFileSync(vp('C.md'), '# Gamma\n\nalpha appears here too.\n');
  fs.writeFileSync(vp('sub/D.md'), '# Delta\n\nIn a folder.\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'global-search': true, graph: true, backlink: true, bookmarks: true, 'command-palette': true, switcher: true, outline: true, 'tag-pane': true, 'page-preview': true }, null, 2));
  out.wrote = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}

// ---- renderer helpers -----------------------------------------------------
const LAYOUT_JS = `
  const leafInfo = (l) => { const vs = l.getViewState(); return { node: 'leaf', id: l.id, type: vs.type, file: vs.state && vs.state.file, mode: vs.state && vs.state.mode, source: vs.state && vs.state.source, pinned: vs.pinned, active: l === app.workspace.activeLeaf }; };
  const tree = (n) => { if (!n) return null; if (n.children) { const o = { node: n.direction ? 'split' : 'tabs', id: n.id, children: n.children.map(tree) }; if (n.direction) o.direction = n.direction; if (n.currentTab != null) o.currentTab = n.currentTab; if (n.dimension != null) o.dimension = n.dimension; return o; } return leafInfo(n); };
  const summary = () => ({ root: tree(app.workspace.rootSplit), active: app.workspace.activeLeaf && app.workspace.activeLeaf.id, activeFile: app.workspace.getActiveFile() && app.workspace.getActiveFile().path, groups: app.workspace.rootSplit.children.length });
`;
const layout = (c) => c.evaluate(LAYOUT_JS + ' return summary();');
async function savedLayout(c) {
  const before = bytes('.obsidian/workspace.json');
  await c.evaluate(`if (app.workspace.saveLayout) { await app.workspace.saveLayout(); } else { app.workspace.requestSaveLayout(); } return true;`);
  await waitForChange('.obsidian/workspace.json', before, 4000);
  const raw = bytes('.obsidian/workspace.json');
  let main = null; try { main = JSON.parse(raw).main; } catch {}
  return { raw, main, active: (() => { try { return JSON.parse(raw).active; } catch { return null; } })() };
}
const run = (c, id) => c.evaluate(`return app.commands.executeCommandById(${JSON.stringify(id)});`);
const openNote = (c, name) => c.evaluate(`await app.workspace.openLinkText(${JSON.stringify(name)}, '', false); return app.workspace.getActiveFile() && app.workspace.getActiveFile().path;`);
const modalInfo = (c) => c.evaluate(`
  const m = document.querySelector('.modal-container'); if (!m) return null;
  return { cls: m.className, title: (m.querySelector('.modal-title') || {}).textContent || null, text: m.innerText.slice(0, 600),
    inputs: Array.from(m.querySelectorAll('input, textarea')).map(i => ({ type: i.type, value: i.value, placeholder: i.placeholder, cls: i.className })),
    buttons: Array.from(m.querySelectorAll('button')).map(b => ({ text: b.textContent.trim(), cls: b.className })),
    settings: Array.from(m.querySelectorAll('.setting-item')).map(s => ({ name: (s.querySelector('.setting-item-name')||{}).textContent, desc: (s.querySelector('.setting-item-description')||{}).textContent })) };`);
const bookmarksJson = () => { const raw = bytes('.obsidian/bookmarks.json'); let parsed = null; try { parsed = JSON.parse(raw); } catch {} return { raw, parsed }; };
async function bookmarkViaCommand(c, id, r) {
  const before = bytes('.obsidian/bookmarks.json');
  r.cmd = await run(c, id); await sleep(500);
  r.modal = await modalInfo(c);
  if (r.modal) { await c.pressKey('Enter'); await sleep(400); r.modalAfterEnter = await modalInfo(c); if (r.modalAfterEnter) { await c.pressKey('Escape'); } }
  r.changed = await waitForChange('.obsidian/bookmarks.json', before, 5000);
  r.file = bookmarksJson();
  return r;
}

// ---- after-restart --------------------------------------------------------
if (phase === 'after-restart') {
  const c = await connect();
  await step('r01-layout-after-restart', async () => {
    const live = await layout(c);
    const raw = bytes('.obsidian/workspace.json');
    const before = all.splitsBookmarks.run && all.splitsBookmarks.run.finalWorkspaceJson;
    await sleep(3000);
    const rawLater = bytes('.obsidian/workspace.json');
    return { live, rawAtStart: raw, sameAsSavedBeforeQuit: raw === before, rewrittenWithin3s: rawLater !== raw, rawLater };
  });
  await step('r02-bookmarks-after-restart', async () => ({ file: bookmarksJson(), pane: await c.evaluate(`const p = document.querySelector('.workspace-leaf-content[data-type="bookmarks"]'); return p ? p.innerText.slice(0, 800) : null;`) }));
  c.close();
  process.exit(0);
}

// ---- run --------------------------------------------------------------------
const c = await connect();
out.vaultPath = await c.vaultPath();
out.version = await c.evaluate(`return { ua: navigator.userAgent, apiVersion: (window.app && app.apiVersion) || null };`);
save();

await step('s00-open-A', async () => ({ file: await openNote(c, 'A'), layout: await layout(c), saved: await savedLayout(c) }));

await step('s01-split-vertical', async (r) => {
  r.before = await layout(c);
  r.cmd = await run(c, 'workspace:split-vertical'); await sleep(700);
  r.after = await layout(c);
  r.saved = await savedLayout(c);
  r.handles = await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root .workspace-leaf-resize-handle')).map(h => { const b = h.getBoundingClientRect(); return { cls: h.className, parentCls: h.parentElement.className, x: b.x, y: b.y, w: b.width, h: b.height }; });`);
  r.rootClasses = await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root > .workspace-tabs, .mod-root > .workspace-split')).map(e => ({ cls: e.className, style: e.getAttribute('style') }));`);
  return r;
});

await step('s02-open-B-in-right-then-split-down', async (r) => {
  r.file = await openNote(c, 'B');
  r.cmd = await run(c, 'workspace:split-horizontal'); await sleep(700);
  r.after = await layout(c);
  r.saved = await savedLayout(c);
  r.rootClasses = await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root .workspace-tabs, .mod-root .workspace-split')).map(e => ({ cls: e.className, style: e.getAttribute('style'), parent: e.parentElement.className }));`);
  return r;
});

await step('s03-focus-commands', async (r) => {
  const seq = ['editor:focus-top', 'editor:focus-left', 'editor:focus-right', 'editor:focus-bottom', 'editor:focus-top', 'editor:focus-left', 'editor:focus-left', 'editor:focus-right'];
  r.steps = [];
  for (const id of seq) {
    const ok = await run(c, id); await sleep(300);
    const l = await layout(c);
    r.steps.push({ id, ok, active: l.active, activeFile: l.activeFile });
  }
  return r;
});

await step('s04-resize-handle-drag', async (r) => {
  r.before = await savedLayout(c);
  r.handles = await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root .workspace-leaf-resize-handle')).map(h => { const b = h.getBoundingClientRect(); return { cls: h.className, parentCls: h.parentElement.className, x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; });`);
  const vertical = r.handles.find((h) => h.h > h.w && h.w > 0);
  if (!vertical) throw new Error('no vertical handle found: ' + JSON.stringify(r.handles));
  await c.drag(vertical.cx, vertical.cy, vertical.cx - 150, vertical.cy, { steps: 15 });
  await sleep(600);
  r.after = await savedLayout(c);
  r.live = await layout(c);
  r.styles = await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root .workspace-tabs, .mod-root .workspace-split')).map(e => ({ cls: e.className, style: e.getAttribute('style') }));`);
  return r;
});

await step('s05-new-tab-in-group-and-currentTab', async (r) => {
  r.cmd = await run(c, 'workspace:new-tab'); await sleep(400);
  r.file = await openNote(c, 'C');
  r.after = await layout(c);
  r.saved = await savedLayout(c);
  return r;
});

await step('s06-open-link-in-new-split', async (r) => {
  // Back to the left group, cursor on the [[C]] link in A (line 2, inside the brackets).
  await run(c, 'editor:focus-left'); await sleep(200); await run(c, 'editor:focus-left'); await sleep(200);
  r.start = await layout(c);
  r.cursor = await c.evaluate(`const ed = app.workspace.activeEditor && app.workspace.activeEditor.editor; if (!ed) return 'no editor'; ed.setCursor({ line: 2, ch: 11 }); return ed.getCursor();`);
  r.cmd = await run(c, 'editor:open-link-in-new-split'); await sleep(800);
  r.after = await layout(c);
  r.saved = await savedLayout(c);
  return r;
});

await step('s07-close-last-tab-of-a-group', async (r) => {
  // The leaf that open-link-in-new-split just made has one tab; close it.
  r.before = await layout(c);
  r.cmd = await run(c, 'workspace:close'); await sleep(600);
  r.after = await layout(c);
  r.saved = await savedLayout(c);
  return r;
});

await step('s08-close-tab-group', async (r) => {
  // In the right column's bottom group (B): close the whole group.
  r.before = await layout(c);
  r.cmd = await run(c, 'workspace:close-tab-group'); await sleep(600);
  r.after = await layout(c);
  r.saved = await savedLayout(c);
  return r;
});

await step('s09-close-others-tab-group', async (r) => {
  r.before = await layout(c);
  r.cmd = await run(c, 'workspace:close-others-tab-group'); await sleep(600);
  r.after = await layout(c);
  r.saved = await savedLayout(c);
  return r;
});

await step('s10-split-on-empty-and-graph-tabs', async (r) => {
  await run(c, 'workspace:new-tab'); await sleep(300);
  r.emptySplit = await run(c, 'workspace:split-vertical'); await sleep(500);
  r.afterEmpty = await layout(c);
  r.graph = await run(c, 'graph:open'); await sleep(800);
  r.graphSplit = await run(c, 'workspace:split-horizontal'); await sleep(600);
  r.afterGraph = await layout(c);
  r.saved = await savedLayout(c);
  // Tidy: close everything but the first group so the bookmark steps run in one group.
  await run(c, 'editor:focus-left'); await sleep(200); await run(c, 'editor:focus-left'); await sleep(200); await run(c, 'editor:focus-top'); await sleep(200);
  await run(c, 'workspace:close-others-tab-group'); await sleep(500);
  await run(c, 'workspace:close-others'); await sleep(300);
  r.tidy = await layout(c);
  return r;
}, 30000);

await step('s11-tab-strip-dom', async (r) => {
  await openNote(c, 'A');
  await run(c, 'workspace:split-vertical'); await sleep(600);
  r.dom = await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root .workspace-tabs')).map(t => ({ cls: t.className, header: t.querySelector('.workspace-tab-header-container') && t.querySelector('.workspace-tab-header-container').className, tabs: Array.from(t.querySelectorAll('.workspace-tab-header')).map(h => ({ cls: h.className, title: h.getAttribute('aria-label'), text: h.innerText.trim() })), newTab: Array.from(t.querySelectorAll('.workspace-tab-header-new-tab, .workspace-tab-header-tab-list, .workspace-tab-header-container-inner > *')).map(e => ({ cls: e.className, label: e.getAttribute('aria-label') })) }));`);
  r.layout = await layout(c);
  // Click into the left group's content and see what changes.
  const left = await c.evaluate(`const t = document.querySelectorAll('.mod-root .workspace-tabs')[0]; const b = t.querySelector('.view-content').getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + 40 };`);
  await c.clickAt(left.cx, left.cy); await sleep(300);
  r.afterClickLeft = await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root .workspace-tabs')).map(t => t.className);`);
  r.layoutAfterClick = await layout(c);
  await run(c, 'workspace:close-others-tab-group'); await sleep(400);
  return r;
});

// ---- bookmarks ---------------------------------------------------------------
await step('b00-plugin-state', async () => c.evaluate(`const p = app.internalPlugins.getPluginById('bookmarks'); return { enabled: p && p.enabled, items: p && p.instance && p.instance.items, hasAddItem: !!(p && p.instance && p.instance.addItem), fileBefore: null, keys: p && p.instance ? Object.keys(p.instance).slice(0, 40) : null };`));
out.bookmarksBefore = bookmarksJson(); save();

await step('b01-bookmark-current-view-file', async (r) => { await openNote(c, 'A'); await sleep(300); return bookmarkViaCommand(c, 'bookmarks:bookmark-current-view', r); });
await step('b02-bookmark-current-view-again-duplicate', async (r) => bookmarkViaCommand(c, 'bookmarks:bookmark-current-view', r));
await step('b03-bookmark-current-heading', async (r) => {
  r.cursor = await c.evaluate(`const ed = app.workspace.activeEditor.editor; ed.setCursor({ line: 6, ch: 3 }); return ed.getCursor();`);
  return bookmarkViaCommand(c, 'bookmarks:bookmark-current-heading', r);
});
await step('b04-bookmark-current-section-block', async (r) => {
  r.noteBefore = bytes('A.md');
  r.cursor = await c.evaluate(`const ed = app.workspace.activeEditor.editor; ed.setCursor({ line: 4, ch: 5 }); return ed.getCursor();`);
  await bookmarkViaCommand(c, 'bookmarks:bookmark-current-section', r);
  await sleep(2500);
  r.noteAfter = bytes('A.md');
  return r;
});
await step('b05-bookmark-current-search', async (r) => {
  await run(c, 'global-search:open'); await sleep(500);
  await c.clickElement('.search-input-container input'); await sleep(150);
  await c.typeText('alpha'); await sleep(800);
  await bookmarkViaCommand(c, 'bookmarks:bookmark-current-search', r);
  await c.pressKey('Escape'); await sleep(200);
  return r;
});
await step('b06-bookmark-graph', async (r) => {
  await run(c, 'graph:open'); await sleep(900);
  await bookmarkViaCommand(c, 'bookmarks:bookmark-current-view', r);
  await run(c, 'graph:open'); await sleep(300); // toggles back? record
  r.layout = await layout(c);
  return r;
});
await step('b07-bookmark-all-tabs', async (r) => {
  await openNote(c, 'B'); await sleep(200);
  await run(c, 'workspace:new-tab'); await sleep(200);
  await openNote(c, 'C'); await sleep(300);
  await bookmarkViaCommand(c, 'bookmarks:bookmark-all-tabs', r);
  await run(c, 'workspace:close-others'); await sleep(200);
  return r;
});
await step('b08-folder-and-url-via-api', async (r) => {
  const before = bytes('.obsidian/bookmarks.json');
  r.api = await c.evaluate(`const inst = app.internalPlugins.getPluginById('bookmarks').instance; const f = app.vault.getAbstractFileByPath('sub'); inst.addItem({ type: 'folder', path: 'sub', ctime: Date.now() }); inst.addItem({ type: 'url', url: 'https://example.com/x', title: 'Example', ctime: Date.now() }); return { count: inst.items.length };`);
  r.changed = await waitForChange('.obsidian/bookmarks.json', before, 5000);
  r.file = bookmarksJson();
  return r;
});
await step('b09-file-menu-labels', async (r) => {
  r.folderClick = await c.rightClickElement('.nav-folder-title[data-path="sub"]'); await sleep(400);
  r.folderMenu = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  r.fileClick = await c.rightClickElement('.nav-file-title[data-path="C.md"]'); await sleep(400);
  r.fileMenu = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  r.bookmarkedFileClick = await c.rightClickElement('.nav-file-title[data-path="A.md"]'); await sleep(400);
  r.bookmarkedFileMenu = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  return r;
});
await step('b10-pane-dom', async (r) => {
  r.open = await run(c, 'bookmarks:open'); await sleep(700);
  r.pane = await c.evaluate(`
    const p = document.querySelector('.workspace-leaf-content[data-type="bookmarks"]'); if (!p) return null;
    const icon = (el) => { const s = el && el.querySelector('svg'); return s ? s.getAttribute('class') : null; };
    return { header: Array.from(p.querySelectorAll('.nav-header .nav-action-button, .nav-header .clickable-icon')).map(b => ({ label: b.getAttribute('aria-label'), icon: icon(b), cls: b.className })),
      items: Array.from(p.querySelectorAll('.tree-item')).map(t => ({ cls: t.className, selfCls: (t.querySelector(':scope > .tree-item-self')||{}).className, icon: icon(t.querySelector(':scope > .tree-item-self .tree-item-icon')), text: (t.querySelector(':scope > .tree-item-self .tree-item-inner')||{}).textContent, flair: (t.querySelector(':scope > .tree-item-self .tree-item-flair')||{}).textContent, depth: (t.closest('.tree-item-children') ? 1 : 0) })),
      text: p.innerText.slice(0, 1200) };`);
  r.layout = await layout(c);
  return r;
});
await step('b11-pane-click-opens', async (r) => {
  r.clickSearch = await c.evaluate(`const items = Array.from(document.querySelectorAll('.workspace-leaf-content[data-type="bookmarks"] .tree-item-self')); const s = items.find(i => i.textContent.includes('alpha')); if (!s) return null; s.setAttribute('data-cdp', 'bm-search'); return s.textContent;`);
  if (r.clickSearch) { await c.clickElement('[data-cdp="bm-search"]'); await sleep(600); r.afterSearch = await c.evaluate(`const i = document.querySelector('.search-input-container input'); return { query: i && i.value, leftActive: app.workspace.leftSplit && app.workspace.leftSplit.collapsed }; `); }
  r.clickHeading = await c.evaluate(`const items = Array.from(document.querySelectorAll('.workspace-leaf-content[data-type="bookmarks"] .tree-item-self')); const s = items.find(i => i.textContent.includes('Second heading')); if (!s) return null; s.setAttribute('data-cdp', 'bm-heading'); return s.textContent;`);
  if (r.clickHeading) { await c.clickElement('[data-cdp="bm-heading"]'); await sleep(700); r.afterHeading = await layout(c); r.scroll = await c.evaluate(`const ed = app.workspace.activeEditor && app.workspace.activeEditor.editor; return ed ? ed.getCursor() : null;`); }
  return r;
});
await step('b12-item-context-menu', async (r) => {
  r.tag = await c.evaluate(`const items = Array.from(document.querySelectorAll('.workspace-leaf-content[data-type="bookmarks"] .tree-item-self')); const s = items.find(i => i.textContent.trim() === 'A' || i.textContent.includes('A')); if (!s) return null; s.setAttribute('data-cdp', 'bm-a'); return s.textContent;`);
  if (r.tag) { await c.rightClickElement('[data-cdp="bm-a"]'); await sleep(400); r.menu = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200); }
  return r;
});
await step('b13-unbookmark-current-view', async (r) => { await openNote(c, 'A'); await sleep(300); const before = bytes('.obsidian/bookmarks.json'); r.cmd = await run(c, 'bookmarks:unbookmark-current-view'); r.changed = await waitForChange('.obsidian/bookmarks.json', before, 5000); r.file = bookmarksJson(); r.modal = await modalInfo(c); if (r.modal) await c.pressKey('Escape'); return r; });
await step('b14-command-availability', async () => c.evaluate(`
  const ids = ['bookmarks:bookmark-current-view','bookmarks:bookmark-current-search','bookmarks:unbookmark-current-view','bookmarks:bookmark-current-section','bookmarks:bookmark-current-heading','bookmarks:bookmark-all-tabs','bookmarks:open','workspace:split-vertical','workspace:split-horizontal','workspace:close-tab-group','workspace:close-others-tab-group','editor:focus-left','editor:focus-right','editor:focus-top','editor:focus-bottom','editor:open-link-in-new-split'];
  return ids.map(id => { const c = app.commands.commands[id]; return { id, name: c && c.name, hotkeys: c && c.hotkeys, hasCheck: !!(c && (c.checkCallback || c.editorCheckCallback)) }; });`));

await step('s99-final-layout-and-quit', async (r) => {
  await openNote(c, 'A'); await sleep(200);
  await run(c, 'workspace:split-vertical'); await sleep(500);
  await openNote(c, 'B'); await sleep(200);
  await run(c, 'workspace:split-horizontal'); await sleep(500);
  await openNote(c, 'C'); await sleep(300);
  r.layout = await layout(c);
  r.saved = await savedLayout(c);
  out.finalWorkspaceJson = r.saved.raw; save();
  await c.quit();
  await sleep(2500);
  r.afterQuitRaw = bytes('.obsidian/workspace.json');
  r.quitRewrote = r.afterQuitRaw !== r.saved.raw;
  out.finalWorkspaceJson = r.afterQuitRaw; save();
  return r;
}, 40000);

try { c.close(); } catch {}
process.exit(0);
