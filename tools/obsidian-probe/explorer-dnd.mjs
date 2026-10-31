// Block 5 of [[Interaction probes]]: the file explorer's drag-and-drop and
// inline rename. The 2026-10-18 pass dispatched synthetic events, which never
// raise `dragstart`, so no drop was ever established; this one uses
// `Input.setInterceptDrags` plus `Input.dispatchDragEvent` (see
// `cdp.mjs`'s `dragAndDrop`), which is the drag Chrome itself would deliver,
// and F2 plus real keystrokes for the rename.
// Phases: fixtures | run
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS;
const vp = (rel) => path.join(VAULT, rel);
const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.explorerDnd = all.explorerDnd || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.explorerDnd[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
const ONLY = process.env.ONLY_STEPS ? process.env.ONLY_STEPS.split(',') : null;
async function step(name, fn) {
  if (ONLY && !ONLY.some((p) => name.startsWith(p))) return null;
  out.order.push(name);
  const t = Date.now(); const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save(); return out.steps[name];
}

function tree() {
  const walk = (dir, prefix) => {
    const outp = [];
    for (const e of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (e.name === '.obsidian' || e.name === '.trash') continue;
      const rel = prefix ? `${prefix}/${e.name}` : e.name;
      if (e.isDirectory()) { outp.push(rel + '/'); outp.push(...walk(path.join(dir, e.name), rel)); }
      else outp.push(rel);
    }
    return outp;
  };
  return walk(VAULT, '');
}
const readIf = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch { return null; } };

if (phase === 'fixtures') {
  for (const d of ['.obsidian', 'Notes', 'Dest', 'Inbox', 'Deep/Sub']) fs.mkdirSync(vp(d), { recursive: true });
  fs.writeFileSync(vp('Root.md'), '# Root\n\nRoot body.\n');
  fs.writeFileSync(vp('Linker.md'), 'Links: [[Root]], [[Notes/Alpha|alpha]], [Markdown](Root.md), ![[cover.png]]\n');
  fs.writeFileSync(vp('Notes/Alpha.md'), '# Alpha\n');
  fs.writeFileSync(vp('Notes/Beta.md'), '# Beta\n');
  fs.writeFileSync(vp('Dest/Alpha.md'), '# Dest Alpha (a collision)\n');
  fs.writeFileSync(vp('Dest/Keep.md'), '# Keep\n');
  fs.writeFileSync(vp('Dest/Gamma.md'), '# Dest Gamma (a collision)\n');
  fs.writeFileSync(vp('Inbox/Gamma.md'), '# Inbox Gamma\n');
  fs.writeFileSync(vp('Deep/Sub/Inner.md'), '# Inner\n');
  fs.writeFileSync(vp('cover.png'), Buffer.from('89504e470d0a1a0a', 'hex'));
  fs.writeFileSync(vp('.obsidian/app.json'), JSON.stringify({ alwaysUpdateLinks: false }, null, 2));
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'command-palette': true, switcher: true }, null, 2));
  save(); process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
out.title = await c.evaluate(`return document.title;`);
out.config = await c.evaluate(`return { alwaysUpdateLinks: app.vault.getConfig('alwaysUpdateLinks'), promptDelete: app.vault.getConfig('promptDelete'), trashOption: app.vault.getConfig('trashOption') };`);
out.treeAtStart = tree();
save();

await c.evaluate(`
window.__dd = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  explorer() { return app.workspace.getLeavesOfType('file-explorer')[0]; },
  async reveal() {
    app.workspace.leftSplit.expand();
    app.workspace.revealLeaf(this.explorer());
    const v = this.explorer().view;
    if (v.tree && v.tree.setCollapseAll) v.tree.setCollapseAll(false);
    await this.sleep(600);
    return true;
  },
  rows() {
    const pane = this.explorer().view.containerEl;
    return Array.from(pane.querySelectorAll('.nav-file-title, .nav-folder-title')).map((t) => ({
      path: t.getAttribute('data-path'), cls: t.className,
      text: (t.querySelector('.nav-file-title-content, .nav-folder-title-content') || {}).textContent,
      rect: (() => { const b = t.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; })(),
      draggable: t.getAttribute('draggable'),
    }));
  },
  row(p) { return this.rows().find((r) => r.path === p) || null; },
  dragClasses() {
    const pane = this.explorer().view.containerEl;
    return {
      beingDragged: Array.from(pane.querySelectorAll('.is-being-dragged')).map((x) => x.getAttribute('data-path') || x.className),
      draggedOver: Array.from(pane.querySelectorAll('.is-being-dragged-over')).map((x) => x.getAttribute('data-path') || x.className),
    };
  },
  notices() { return Array.from(document.querySelectorAll('.notice')).map((n) => n.textContent); },
  renameEditor() {
    const pane = this.explorer().view.containerEl;
    const el = pane.querySelector('[contenteditable="true"]');
    return el ? { cls: el.className, text: el.textContent, parentCls: el.parentElement.className, parentPath: el.closest('[data-path]') ? el.closest('[data-path]').getAttribute('data-path') : null, selection: String(window.getSelection()) } : null;
  },
};
return true;`);

const ev = (b) => c.evaluate(b);
const row = (p) => ev(`return window.__dd.row(${JSON.stringify(p)});`);
await ev(`return await window.__dd.reveal();`);

await step('s01-f2-inline-rename', async (r) => {
  r.rowsAtStart = await ev(`return window.__dd.rows().map((x) => x.path);`);
  // Click a row so the explorer, not the editor, holds focus, then F2.
  const keep = await row('Dest/Keep.md');
  r.keep = keep;
  await c.clickAt(keep.rect.cx, keep.rect.cy);
  await sleep(600);
  const beta = await row('Notes/Beta.md');
  await c.clickAt(beta.rect.cx, beta.rect.cy);
  await sleep(600);
  r.focusBeforeF2 = await ev(`
    const a = document.activeElement;
    return { tag: a && a.tagName, cls: a && typeof a.className === 'string' ? a.className : '', inExplorer: !!(a && window.__dd.explorer().view.containerEl.contains(a)) };`);
  await c.pressKey('F2');
  await sleep(700);
  r.editorAfterF2 = await ev(`return window.__dd.renameEditor();`);
  if (r.editorAfterF2) {
    // Select all and type a new name, then Enter.
    await c.pressKey('a', { ctrl: true });
    await sleep(200);
    await c.typeText('Beta Renamed');
    await sleep(400);
    r.editorWhileTyping = await ev(`return window.__dd.renameEditor();`);
    await c.pressKey('Enter');
    await sleep(1500);
    r.afterEnter = { tree: tree(), editor: await ev(`return window.__dd.renameEditor();`), notices: await ev(`return window.__dd.notices();`), linker: readIf('Linker.md') };
  }
  return r;
});

await step('s02-rename-collision-and-extension', async (r) => {
  // Rename Dest/Keep.md to Alpha, which Dest/ already holds.
  const keep = await row('Dest/Keep.md');
  await c.clickAt(keep.rect.cx, keep.rect.cy);
  await sleep(500);
  await c.pressKey('F2');
  await sleep(700);
  await c.pressKey('a', { ctrl: true });
  await c.typeText('Alpha');
  await sleep(300);
  await c.pressKey('Enter');
  await sleep(1600);
  r.collision = { tree: tree(), notices: await ev(`return window.__dd.notices();`), editor: await ev(`return window.__dd.renameEditor();`), modal: await ev(`const m = document.querySelector('.modal'); return m ? { title: (m.querySelector('.modal-title') || {}).textContent, text: (m.querySelector('.modal-content') || {}).textContent } : null;`) };
  await c.pressKey('Escape');
  await sleep(400);
  // Rename a png with no extension typed.
  const png = await row('cover.png');
  r.pngRow = png;
  if (png) {
    await c.clickAt(png.rect.cx, png.rect.cy);
    await sleep(500);
    await c.pressKey('F2');
    await sleep(700);
    r.pngEditor = await ev(`return window.__dd.renameEditor();`);
    await c.pressKey('a', { ctrl: true });
    await c.typeText('cover shot');
    await sleep(300);
    await c.pressKey('Enter');
    await sleep(1600);
    r.pngAfter = { tree: tree(), notices: await ev(`return window.__dd.notices();`), linker: readIf('Linker.md') };
  }
  return r;
});

await step('s03-folder-rename-and-menu', async (r) => {
  const deep = await row('Deep');
  r.deep = deep;
  await c.clickAt(deep.rect.cx, deep.rect.cy, { button: 'right' });
  await sleep(700);
  r.folderMenu = await c.openMenuItems();
  await c.pressKey('Escape');
  await sleep(400);
  await c.clickAt(deep.rect.cx, deep.rect.cy);
  await sleep(500);
  await c.pressKey('F2');
  await sleep(700);
  r.folderEditor = await ev(`return window.__dd.renameEditor();`);
  if (r.folderEditor) {
    await c.pressKey('a', { ctrl: true });
    await c.typeText('Deepest');
    await sleep(300);
    await c.pressKey('Enter');
    await sleep(1600);
    r.afterFolderRename = { tree: tree(), notices: await ev(`return window.__dd.notices();`) };
  }
  // Escape instead of Enter.
  const inbox = await row('Inbox');
  if (inbox) {
    await c.clickAt(inbox.rect.cx, inbox.rect.cy);
    await sleep(400);
    await c.pressKey('F2');
    await sleep(600);
    await c.pressKey('a', { ctrl: true });
    await c.typeText('Abandoned');
    await sleep(300);
    await c.pressKey('Escape');
    await sleep(1200);
    r.afterEscape = { tree: tree(), editor: await ev(`return window.__dd.renameEditor();`) };
  }
  return r;
});

await step('s04-drag-a-file-onto-a-folder', async (r) => {
  await ev(`return await window.__dd.reveal();`);
  const src = await row('Root.md');
  const dst = await row('Dest');
  r.src = src; r.dst = dst;
  r.linkerBefore = readIf('Linker.md');
  // Hover over the target mid-drag, to read the highlight, then drop.
  const held = await c.dragOverOnly(src.rect.cx, src.rect.cy, dst.rect.cx, dst.rect.cy);
  r.dragStarted = held.started;
  r.dragData = held.data ? { mimeTypes: (held.data.items || []).map((i) => i.mimeType), items: (held.data.items || []).map((i) => ({ mimeType: i.mimeType, data: String(i.data).slice(0, 200) })), files: held.data.files } : null;
  if (held.started) {
    await sleep(300);
    r.classesWhileOver = await ev(`return window.__dd.dragClasses();`);
    await held.finish(dst.rect.cx, dst.rect.cy);
    await sleep(1600);
  }
  r.afterDrop = {
    tree: tree(), classes: await ev(`return window.__dd.dragClasses();`),
    notices: await ev(`return window.__dd.notices();`),
    linkerImmediately: readIf('Linker.md'),
  };
  await sleep(4000);
  r.linkerAfter4s = readIf('Linker.md');
  return r;
});

await step('s05-drop-with-a-collision', async (r) => {
  await ev(`return await window.__dd.reveal();`);
  const src = await row('Notes/Alpha.md');
  const dst = await row('Dest');
  r.src = src; r.dst = dst;
  r.destAlphaBefore = readIf('Dest/Alpha.md');
  const drop = await c.dragAndDrop(src.rect.cx, src.rect.cy, dst.rect.cx, dst.rect.cy);
  r.drop = { started: drop.started, error: drop.error };
  await sleep(1800);
  r.after = {
    tree: tree(), notices: await ev(`return window.__dd.notices();`),
    modal: await ev(`const m = document.querySelector('.modal'); return m ? { title: (m.querySelector('.modal-title') || {}).textContent, text: (m.querySelector('.modal-content') || {}).textContent, buttons: Array.from(m.querySelectorAll('button')).map((b) => b.textContent) } : null;`),
    destAlpha: readIf('Dest/Alpha.md'), notesAlpha: readIf('Notes/Alpha.md'),
    linker: readIf('Linker.md'),
  };
  await c.pressKey('Escape');
  await sleep(400);
  return r;
});

await step('s06-drop-on-a-file-row-then-undo', async (r) => {
  await ev(`return await window.__dd.reveal();`);
  const src = await row('Inbox/Gamma.md');
  const dst = await row('Dest/Keep.md');
  r.src = src; r.dst = dst;
  if (src && dst) {
    const drop = await c.dragAndDrop(src.rect.cx, src.rect.cy, dst.rect.cx, dst.rect.cy);
    r.drop = { started: drop.started, error: drop.error };
    await sleep(1800);
    r.after = { tree: tree(), notices: await ev(`return window.__dd.notices();`) };
    await c.pressKey('z', { ctrl: true });
    await sleep(1500);
    r.afterUndo = { tree: tree(), notices: await ev(`return window.__dd.notices();`) };
  }
  return r;
});

await step('s07-drag-a-folder', async (r) => {
  await ev(`return await window.__dd.reveal();`);
  const src = await row('Deepest') || await row('Deep');
  const dst = await row('Dest');
  r.src = src; r.dst = dst;
  if (src && dst) {
    const drop = await c.dragAndDrop(src.rect.cx, src.rect.cy, dst.rect.cx, dst.rect.cy);
    r.drop = { started: drop.started, error: drop.error, mimeTypes: drop.data ? (drop.data.items || []).map((i) => i.mimeType) : null };
    await sleep(1800);
    r.after = { tree: tree(), notices: await ev(`return window.__dd.notices();`) };
  }
  // A folder onto its own descendant, which must not move.
  const outer = await row('Dest');
  const inner = await row('Dest/Deepest') || await row('Dest/Deep');
  if (outer && inner) {
    const drop2 = await c.dragAndDrop(outer.rect.cx, outer.rect.cy, inner.rect.cx, inner.rect.cy);
    r.ontoDescendant = { started: drop2.started };
    await sleep(1500);
    r.afterDescendant = { tree: tree(), notices: await ev(`return window.__dd.notices();`) };
  }
  return r;
});

await step('s08-drop-targets-and-multiselect', async (r) => {
  await ev(`return await window.__dd.reveal();`);
  const rows = await ev(`return window.__dd.rows();`);
  r.rows = rows.map((x) => x.path);
  // Ctrl+click several rows: does the explorer multi-select?
  const picks = rows.filter((x) => x.path && x.path.endsWith('.md')).slice(0, 3);
  if (picks.length === 3) {
    await c.clickAt(picks[0].rect.cx, picks[0].rect.cy);
    await sleep(400);
    await c.clickAt(picks[1].rect.cx, picks[1].rect.cy, { mods: { ctrl: true } });
    await sleep(400);
    await c.clickAt(picks[2].rect.cx, picks[2].rect.cy, { mods: { ctrl: true } });
    await sleep(400);
    r.ctrlClick = await ev(`
      const pane = window.__dd.explorer().view.containerEl;
      const v = window.__dd.explorer().view;
      return {
        selectedClasses: Array.from(pane.querySelectorAll('.is-selected, .has-focus, .is-active')).map((x) => (x.getAttribute('data-path') || '') + '|' + x.className),
        viewSelection: (() => { try { return v.tree && v.tree.selectedDoms ? Array.from(v.tree.selectedDoms).map((d) => d.file ? d.file.path : null) : (v.dragFiles ? v.dragFiles.map((f) => f.path) : 'no selectedDoms/dragFiles'); } catch (e) { return 'error: ' + e.message; } })(),
      };`);
    await c.clickAt(picks[0].rect.cx, picks[0].rect.cy, { mods: { shift: true } });
    await sleep(400);
    r.shiftClick = await ev(`
      const pane = window.__dd.explorer().view.containerEl;
      return Array.from(pane.querySelectorAll('.is-selected')).map((x) => x.getAttribute('data-path'));`);
  }
  // What the drag highlights over three places: a folder row, the gap between
  // rows, and the editor pane.
  const src = rows.find((x) => x.path === 'Linker.md') || picks[0];
  const targets = await ev(`
    const u = window.__dd;
    const pane = u.explorer().view.containerEl;
    const container = pane.querySelector('.nav-files-container');
    const cb = container.getBoundingClientRect();
    const editor = document.querySelector('.workspace-leaf.mod-active .markdown-source-view, .workspace-leaf.mod-active .view-content');
    const eb = editor ? editor.getBoundingClientRect() : null;
    const folder = u.row('Dest');
    return {
      folder: folder && folder.rect,
      belowLastRow: { cx: cb.x + cb.width / 2, cy: cb.y + cb.height - 5 },
      editorPane: eb ? { cx: eb.x + eb.width / 2, cy: eb.y + eb.height / 2 } : null,
    };`);
  r.hoverTargets = {};
  for (const [label, t] of Object.entries(targets)) {
    if (!t) continue;
    const held = await c.dragOverOnly(src.rect.cx, src.rect.cy, t.cx, t.cy);
    if (!held.started) { r.hoverTargets[label] = 'drag did not start'; continue; }
    await sleep(300);
    r.hoverTargets[label] = {
      classes: await ev(`return window.__dd.dragClasses();`),
      dropZoneClasses: await ev(`return Array.from(document.querySelectorAll('.is-being-dragged-over, .drag-over, .mod-drop-target')).map((x) => x.className + '|' + (x.getAttribute('data-path') || ''));`),
    };
    await held.finish();            // dragCancel, so nothing moves
    await sleep(500);
  }
  r.treeAfterHovers = tree();
  return r;
});

// ---------------------------------------------------------------------------
// Second pass. The first attempt pressed F2 with focus still in the editor —
// clicking a row opens the file and hands focus to CodeMirror, which the
// explorer-keyboard block had already recorded — so Ctrl+A and the typing
// went to the note. These steps enter the tree through
// `file-explorer:reveal-active-file`, which leaves the row focused.
// ---------------------------------------------------------------------------

// Put focus on `path`'s row inside the explorer, and prove it is there.
async function focusRow(p) {
  return ev(`
    const u = window.__dd;
    await u.reveal();
    await app.workspace.openLinkText(${JSON.stringify(p)}, '', false);
    await u.sleep(500);
    app.commands.executeCommandById('file-explorer:reveal-active-file');
    await u.sleep(600);
    const a = document.activeElement;
    const pane = u.explorer().view.containerEl;
    return {
      focusedRows: Array.from(pane.querySelectorAll('.has-focus')).map((x) => x.getAttribute('data-path')),
      activeElement: { tag: a && a.tagName, cls: a && typeof a.className === 'string' ? a.className : '' },
      inExplorer: !!(a && pane.contains(a)),
    };`);
}

await step('s09-f2-with-focus-in-the-tree', async (r) => {
  r.focus = await focusRow('Notes/Beta.md');
  r.linkerBefore = readIf('Linker.md');
  await c.pressKey('F2');
  await sleep(800);
  r.editorAfterF2 = await ev(`return window.__dd.renameEditor();`);
  if (!r.editorAfterF2) return r;
  await c.pressKey('a', { ctrl: true });
  await sleep(250);
  r.afterSelectAll = await ev(`return window.__dd.renameEditor();`);
  await c.typeText('Beta Renamed');
  await sleep(400);
  r.whileTyping = await ev(`return window.__dd.renameEditor();`);
  await c.pressKey('Enter');
  await sleep(1800);
  r.afterEnter = {
    tree: tree(), editor: await ev(`return window.__dd.renameEditor();`),
    notices: await ev(`return window.__dd.notices();`),
    modal: await ev(`const m = document.querySelector('.modal'); return m ? { title: (m.querySelector('.modal-title') || {}).textContent, text: (m.querySelector('.modal-content') || {}).textContent, buttons: Array.from(m.querySelectorAll('button')).map((b) => b.textContent) } : null;`),
    linker: readIf('Linker.md'),
    focusAfter: await ev(`const a = document.activeElement; return { tag: a && a.tagName, cls: a && typeof a.className === 'string' ? a.className : '' };`),
  };
  await c.pressKey('Escape');
  await sleep(400);
  return r;
});

await step('s10-inline-rename-edge-cases', async (r) => {
  // Onto a name the folder already holds.
  r.collisionFocus = await focusRow('Dest/Keep.md');
  await c.pressKey('F2');
  await sleep(700);
  r.collisionEditorOpen = !!(await ev(`return window.__dd.renameEditor();`));
  await c.pressKey('a', { ctrl: true });
  await c.typeText('Alpha');
  await sleep(300);
  await c.pressKey('Enter');
  await sleep(1800);
  r.collision = {
    tree: tree(), notices: await ev(`return window.__dd.notices();`),
    editor: await ev(`return window.__dd.renameEditor();`),
    modal: await ev(`const m = document.querySelector('.modal'); return m ? { title: (m.querySelector('.modal-title') || {}).textContent, text: (m.querySelector('.modal-content') || {}).textContent } : null;`),
  };
  await c.pressKey('Escape');
  await sleep(500);
  // A non-markdown file renamed without typing an extension.
  r.pngFocus = await ev(`
    const u = window.__dd;
    await u.reveal();
    const rows = u.rows();
    const png = rows.find((x) => x.path && x.path.endsWith('.png'));
    return png;`);
  if (r.pngFocus) {
    // An image has no markdown editor to steal focus, but reveal-active-file
    // is still the way in.
    r.pngFocusState = await focusRow(r.pngFocus.path);
    await c.pressKey('F2');
    await sleep(700);
    r.pngEditor = await ev(`return window.__dd.renameEditor();`);
    if (r.pngEditor) {
      await c.pressKey('a', { ctrl: true });
      await c.typeText('renamed cover');
      await sleep(300);
      await c.pressKey('Enter');
      await sleep(1800);
      r.pngAfter = { tree: tree(), notices: await ev(`return window.__dd.notices();`), linker: readIf('Linker.md') };
    }
  }
  // Escape abandons.
  r.escapeFocus = await focusRow('Linker.md');
  await c.pressKey('F2');
  await sleep(700);
  r.escapeEditorOpen = !!(await ev(`return window.__dd.renameEditor();`));
  await c.pressKey('a', { ctrl: true });
  await c.typeText('Abandoned');
  await sleep(300);
  await c.pressKey('Escape');
  await sleep(1200);
  r.afterEscape = { tree: tree(), editor: await ev(`return window.__dd.renameEditor();`) };
  // An empty name.
  r.emptyFocus = await focusRow('Linker.md');
  await c.pressKey('F2');
  await sleep(700);
  await c.pressKey('a', { ctrl: true });
  await c.pressKey('Delete');
  await sleep(250);
  await c.pressKey('Enter');
  await sleep(1500);
  r.afterEmptyName = {
    tree: tree(), notices: await ev(`return window.__dd.notices();`),
    editor: await ev(`return window.__dd.renameEditor();`),
  };
  await c.pressKey('Escape');
  await sleep(400);
  return r;
});

await step('s11-folder-inline-rename', async (r) => {
  // A folder row cannot be reached through reveal-active-file; click it (a
  // folder click only toggles, it does not open a file) and read focus.
  r.state = await ev(`
    const u = window.__dd;
    await u.reveal();
    const f = u.row('Notes');
    return f;`);
  if (!r.state) return r;
  await c.clickAt(r.state.rect.cx, r.state.rect.cy);
  await sleep(600);
  r.focusAfterClick = await ev(`
    const u = window.__dd; const a = document.activeElement; const pane = u.explorer().view.containerEl;
    return { tag: a && a.tagName, cls: a && typeof a.className === 'string' ? a.className : '', inExplorer: !!(a && pane.contains(a)), focusedRows: Array.from(pane.querySelectorAll('.has-focus')).map((x) => x.getAttribute('data-path')) };`);
  await c.pressKey('F2');
  await sleep(800);
  r.editor = await ev(`return window.__dd.renameEditor();`);
  if (r.editor) {
    await c.pressKey('a', { ctrl: true });
    await c.typeText('Renamed Notes');
    await sleep(300);
    await c.pressKey('Enter');
    await sleep(1800);
    r.after = { tree: tree(), notices: await ev(`return window.__dd.notices();`), linker: readIf('Linker.md') };
  }
  // And the context menu's own Rename item, for comparison.
  const f2 = await ev(`return window.__dd.row('Inbox');`);
  if (f2) {
    await c.clickAt(f2.rect.cx, f2.rect.cy, { button: 'right' });
    await sleep(700);
    r.menu = await c.openMenuItems();
    const renameRect = await ev(`
      const it = Array.from(document.querySelectorAll('.menu-item')).find((x) => ((x.querySelector('.menu-item-title') || {}).textContent || '').indexOf('Rename') === 0);
      if (!it) return null;
      const b = it.getBoundingClientRect();
      return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
    if (renameRect) {
      await c.clickAt(renameRect.cx, renameRect.cy);
      await sleep(900);
      r.menuRename = {
        inlineEditor: await ev(`return window.__dd.renameEditor();`),
        modal: await ev(`const m = document.querySelector('.modal'); return m ? { title: (m.querySelector('.modal-title') || {}).textContent, inputs: Array.from(m.querySelectorAll('input')).map((i) => i.value), buttons: Array.from(m.querySelectorAll('button')).map((b) => b.textContent) } : null;`),
      };
      await c.pressKey('Escape');
      await sleep(400);
    }
  }
  return r;
});

await step('s12-drop-on-a-file-row-then-undo', async (r) => {
  const prep = await ev(`
    const u = window.__dd;
    await u.reveal();
    const v = u.explorer().view;
    if (v.tree && v.tree.setCollapseAll) v.tree.setCollapseAll(false);
    await u.sleep(700);
    return u.rows().map((x) => x.path);`);
  r.rows = prep;
  const src = await row('Inbox/Gamma.md');
  const dst = await row('Dest/Keep.md');
  r.src = src; r.dst = dst;
  if (!src || !dst) return r;
  r.destGammaBefore = readIf('Dest/Gamma.md');
  const drop = await c.dragAndDrop(src.rect.cx, src.rect.cy, dst.rect.cx, dst.rect.cy);
  r.drop = { started: drop.started, error: drop.error, mimeTypes: drop.data ? (drop.data.items || []).map((i) => i.mimeType) : null };
  await sleep(1800);
  r.after = {
    tree: tree(), notices: await ev(`return window.__dd.notices();`),
    destGamma: readIf('Dest/Gamma.md'), inboxGamma: readIf('Inbox/Gamma.md'),
  };
  // Ctrl+Z with the explorer focused.
  await ev(`
    const u = window.__dd;
    const pane = u.explorer().view.containerEl;
    const c2 = pane.querySelector('.nav-files-container');
    if (c2 && c2.focus) c2.focus();
    return document.activeElement && document.activeElement.className;`);
  await c.pressKey('z', { ctrl: true });
  await sleep(1600);
  r.afterUndo = { tree: tree(), notices: await ev(`return window.__dd.notices();`) };
  return r;
});

out.treeAtEnd = tree();
out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
