// Block 4 of [[Interaction probes]]: what a real paste and a real drop do
// with an attachment. The 2026-10-18 run pasted through the API with the
// folders already in place, so it could not say whether a paste creates a
// missing folder, what a named file is called, or what happens outside the
// editor. This one puts an image on the Electron clipboard
// (`nativeImage.createFromPath`) and presses a real Ctrl+V, and drops files
// through `Input.dispatchDragEvent`'s `files` payload.
//
// FIXTURES must point at a directory of source files (the runner's caller
// makes it); the script copies them into the scratch vault's parent so the
// clipboard can be loaded from outside the vault.
// Phases: fixtures | run
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS;
const FIXTURES = process.env.FIXTURES;
const vp = (rel) => path.join(VAULT, rel);
const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.attachmentPaste = all.attachmentPaste || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.attachmentPaste[phase] = out;
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
    const o = [];
    for (const e of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (e.name === '.obsidian' || e.name === '.trash') continue;
      const rel = prefix ? `${prefix}/${e.name}` : e.name;
      if (e.isDirectory()) { o.push(rel + '/'); o.push(...walk(path.join(dir, e.name), rel)); }
      else o.push(rel + ' (' + fs.statSync(path.join(dir, e.name)).size + 'b)');
    }
    return o;
  };
  return walk(VAULT, '');
}
const readIf = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch { return null; } };
const SRC = path.join(path.dirname(VAULT), 'paste-fixtures');

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.mkdirSync(vp('Sub'), { recursive: true });
  fs.mkdirSync(SRC, { recursive: true });
  if (FIXTURES) for (const f of fs.readdirSync(FIXTURES)) fs.copyFileSync(path.join(FIXTURES, f), path.join(SRC, f));
  fs.writeFileSync(vp('Target.md'), 'BEFORE AFTER\n');
  fs.writeFileSync(vp('Sub/Nested.md'), 'x\n');
  fs.writeFileSync(vp('Board.canvas'), JSON.stringify({ nodes: [], edges: [] }, null, '\t') + '\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, canvas: true, 'command-palette': true }, null, 2));
  save(); process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
out.title = await c.evaluate(`return document.title;`);
out.sourceFiles = fs.existsSync(SRC) ? fs.readdirSync(SRC).sort() : [];
out.treeAtStart = tree();
save();

await c.evaluate(`
window.__ap = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  leaf() {
    const a = app.workspace.activeLeaf;
    if (a && a.view && a.view.getViewType() === 'markdown') return a;
    const ls = app.workspace.getLeavesOfType('markdown');
    return ls.length ? ls[0] : app.workspace.getLeaf(false);
  },
  view() { return this.leaf().view; },
  async open(file, mode) {
    const ls = app.workspace.getLeavesOfType('markdown');
    const leaf = ls.length ? ls[0] : app.workspace.getLeaf(false);
    await leaf.openFile(app.vault.getAbstractFileByPath(file));
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await this.sleep(500);
    if (mode) {
      const st = leaf.getViewState();
      st.state = Object.assign({}, st.state, { mode: mode === 'reading' ? 'preview' : 'source', source: mode === 'source' });
      await leaf.setViewState(st);
      await this.sleep(400);
    }
    return { file: leaf.view.file ? leaf.view.file.path : null, type: leaf.view.getViewType() };
  },
  // Put a real image on the system clipboard, from a path outside the vault.
  clipImage(p) {
    const { clipboard, nativeImage } = window.require('electron');
    const img = nativeImage.createFromPath(p);
    if (img.isEmpty()) return { ok: false, reason: 'nativeImage empty for ' + p };
    clipboard.writeImage(img);
    const back = clipboard.readImage();
    return { ok: !back.isEmpty(), size: back.getSize(), formats: clipboard.availableFormats() };
  },
  // A named file on the clipboard the way a file manager offers it.
  clipFile(p) {
    const { clipboard } = window.require('electron');
    const { nativeImage } = window.require('electron');
    const img = nativeImage.createFromPath(p);
    clipboard.write({ text: p, image: img.isEmpty() ? undefined : img });
    return { formats: clipboard.availableFormats() };
  },
  clipText(t) { window.require('electron').clipboard.writeText(t); return window.require('electron').clipboard.availableFormats(); },
  caret(line, ch) { const ed = this.view().editor; ed.focus(); ed.setCursor({ line, ch }); return ed.getCursor(); },
  value() { const v = this.view(); return v.editor ? v.editor.getValue() : null; },
  notices() { return Array.from(document.querySelectorAll('.notice')).map((n) => n.textContent); },
  config(k, v) { if (v === undefined) return app.vault.getConfig(k); app.vault.setConfig(k, v); return app.vault.getConfig(k); },
  async save() { try { await this.view().save(); } catch (e) {} await this.sleep(400); return true; },
  editorRect() {
    const el = this.view().containerEl.querySelector('.cm-content');
    if (!el) return null;
    const b = el.getBoundingClientRect();
    return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 };
  },
};
return true;`);

const ev = (b) => c.evaluate(b);
const src = (f) => path.join(SRC, f);

// Put the image on the clipboard, place the caret, press Ctrl+V, read back.
async function pasteAt(file, line, ch, imagePath, { mode = 'live', settle = 2500 } = {}) {
  const opened = await ev(`return await window.__ap.open(${JSON.stringify(file)}, ${JSON.stringify(mode)});`);
  const clip = await ev(`return window.__ap.clipImage(${JSON.stringify(imagePath)});`);
  if (mode !== 'reading') await ev(`return window.__ap.caret(${line}, ${ch});`);
  const before = { tree: tree(), value: await ev(`return window.__ap.value();`) };
  await c.pressKey('v', { ctrl: true });
  await sleep(settle);
  await ev(`return await window.__ap.save();`);
  await sleep(600);
  return {
    opened, clip, before,
    after: { tree: tree(), value: await ev(`return window.__ap.value();`), disk: readIf(file), notices: await ev(`return window.__ap.notices();`) },
  };
}

await step('s01-paste-into-a-folder-that-does-not-exist', async (r) => {
  r.config = await ev(`return { set: window.__ap.config('attachmentFolderPath', 'att/deep/new'), newLinkFormat: window.__ap.config('newLinkFormat'), useMarkdownLinks: window.__ap.config('useMarkdownLinks') };`);
  r.folderAbsentBefore = !fs.existsSync(vp('att'));
  r.paste = await pasteAt('Target.md', 0, 7, src('photo.png'));
  r.folderAfter = { att: fs.existsSync(vp('att')), deep: fs.existsSync(vp('att/deep')), newDir: fs.existsSync(vp('att/deep/new')) };
  return r;
});

await step('s02-same-folder-as-current-file', async (r) => {
  r.config = await ev(`return window.__ap.config('attachmentFolderPath', './');`);
  r.paste = await pasteAt('Sub/Nested.md', 0, 1, src('photo.png'));
  return r;
});

await step('s03-one-paste-per-format', async (r) => {
  await ev(`return window.__ap.config('attachmentFolderPath', '/');`);
  r.formats = {};
  for (const f of ['photo.png', 'shot.jpeg', 'vec.svg', 'pic.bmp', 'doc.pdf']) {
    if (!fs.existsSync(src(f))) { r.formats[f] = 'fixture missing'; continue; }
    const got = await pasteAt('Target.md', 0, 0, src(f), { settle: 2200 });
    r.formats[f] = { clip: got.clip, treeDelta: got.after.tree.filter((x) => !got.before.tree.includes(x)), value: got.after.value, notices: got.after.notices };
  }
  return r;
});

await step('s04-collision-across-folders', async (r) => {
  await ev(`return window.__ap.config('attachmentFolderPath', 'Attachments');`);
  // A file of the same name already at the root.
  fs.copyFileSync(src('photo.png'), vp('photo.png'));
  r.before = tree();
  r.first = await pasteAt('Target.md', 0, 0, src('photo.png'));
  r.second = await pasteAt('Target.md', 0, 0, src('photo.png'));
  r.after = tree();
  return r;
});

await step('s05-markdown-links-and-ugly-names', async (r) => {
  r.config = await ev(`
    const u = window.__ap;
    u.config('useMarkdownLinks', true);
    u.config('newLinkFormat', 'absolute');
    u.config('attachmentFolderPath', 'Attachments');
    return { useMarkdownLinks: u.config('useMarkdownLinks'), newLinkFormat: u.config('newLinkFormat') };`);
  r.fromSubfolder = await pasteAt('Sub/Nested.md', 0, 1, src('photo.png'));
  const ugly = 'a#b&c (1) cafe.png';
  if (fs.existsSync(src(ugly))) {
    // Rename the fixture to carry non-ASCII, so the clipboard name does too.
    const uglyUtf = src('a#b&c (1) café.png');
    fs.copyFileSync(src(ugly), uglyUtf);
    r.ugly = await pasteAt('Sub/Nested.md', 0, 1, uglyUtf);
  }
  r.restore = await ev(`
    const u = window.__ap;
    u.config('useMarkdownLinks', false);
    u.config('newLinkFormat', 'shortest');
    return { useMarkdownLinks: u.config('useMarkdownLinks'), newLinkFormat: u.config('newLinkFormat') };`);
  return r;
});

await step('s06-paste-into-reading-view-and-a-canvas', async (r) => {
  await ev(`return window.__ap.config('attachmentFolderPath', 'Attachments');`);
  r.readingView = await (async () => {
    const opened = await ev(`return await window.__ap.open('Target.md', 'reading');`);
    const clip = await ev(`return window.__ap.clipImage(${JSON.stringify(src('photo.png'))});`);
    const before = tree();
    // Click the rendered body first so the reading view has focus.
    const rect = await ev(`
      const el = window.__ap.view().containerEl.querySelector('.markdown-preview-view');
      const b = el.getBoundingClientRect();
      return { cx: b.x + b.width / 2, cy: b.y + 60 };`);
    await c.clickAt(rect.cx, rect.cy);
    await sleep(400);
    await c.pressKey('v', { ctrl: true });
    await sleep(2200);
    return { opened, clip, treeDelta: tree().filter((x) => !before.includes(x)), notices: await ev(`return window.__ap.notices();`), disk: readIf('Target.md') };
  })();
  r.canvas = await (async () => {
    const opened = await ev(`
      const u = window.__ap;
      const leaf = app.workspace.getLeaf(false);
      await leaf.openFile(app.vault.getAbstractFileByPath('Board.canvas'));
      app.workspace.setActiveLeaf(leaf, { focus: true });
      await u.sleep(1200);
      return { type: leaf.view.getViewType(), file: leaf.view.file.path };`);
    const clip = await ev(`return window.__ap.clipImage(${JSON.stringify(src('photo.png'))});`);
    const before = tree();
    const rect = await ev(`
      const l = app.workspace.activeLeaf;
      const el = l.view.containerEl.querySelector('.canvas-wrapper, .canvas, .view-content');
      const b = el.getBoundingClientRect();
      return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
    await c.clickAt(rect.cx, rect.cy);
    await sleep(500);
    await c.pressKey('v', { ctrl: true });
    await sleep(2500);
    return { opened, clip, treeDelta: tree().filter((x) => !before.includes(x)), canvasBytes: readIf('Board.canvas'), notices: await ev(`return window.__ap.notices();`) };
  })();
  return r;
});

await step('s07-undo-depth-and-when-it-hits-disk', async (r) => {
  await ev(`return window.__ap.config('attachmentFolderPath', 'Attachments');`);
  const opened = await ev(`return await window.__ap.open('Target.md', 'live');`);
  await ev(`
    const u = window.__ap;
    await app.vault.modify(u.view().file, 'BEFORE AFTER\\n');
    await u.sleep(800);
    u.caret(0, 7);
    return true;`);
  r.opened = opened;
  await ev(`return window.__ap.clipImage(${JSON.stringify(src('photo.png'))});`);
  const beforeTree = tree();
  await c.pressKey('v', { ctrl: true });
  // When does the note reach disk, without anything else touching it?
  r.diskTimeline = [];
  for (const t of [0, 500, 1000, 2000, 4000, 6000]) {
    await sleep(t === 0 ? 150 : t - (r.diskTimeline.length ? [0, 500, 1000, 2000, 4000, 6000][r.diskTimeline.length - 1] : 0));
    r.diskTimeline.push({ atMs: t, disk: readIf('Target.md'), editor: await ev(`return window.__ap.value();`) });
  }
  r.treeDelta = tree().filter((x) => !beforeTree.includes(x));
  r.undos = [];
  for (let i = 1; i <= 5; i++) {
    await c.pressKey('z', { ctrl: true });
    await sleep(700);
    r.undos.push({ press: i, editor: await ev(`return window.__ap.value();`), attachmentStillThere: fs.existsSync(vp('Attachments')) ? fs.readdirSync(vp('Attachments')) : null });
  }
  await ev(`return await window.__ap.save();`);
  await sleep(600);
  r.afterUndoDisk = readIf('Target.md');
  return r;
});

await step('s08-drop-a-file-into-the-editor', async (r) => {
  await ev(`return window.__ap.config('attachmentFolderPath', 'Attachments');`);
  await ev(`return await window.__ap.open('Target.md', 'live');`);
  await ev(`
    const u = window.__ap;
    await app.vault.modify(u.view().file, 'DROP HERE\\n');
    await u.sleep(700);
    return true;`);
  const rect = await ev(`return window.__ap.editorRect();`);
  r.rect = rect;
  const before = tree();
  // A files payload, the way a file manager's drag offers it.
  r.drop = await c.send('Input.dispatchDragEvent', {
    type: 'dragEnter', x: Math.round(rect.cx), y: Math.round(rect.cy),
    data: { items: [], files: [src('photo.png')], dragOperationsMask: 1 },
  }).then(() => c.send('Input.dispatchDragEvent', {
    type: 'dragOver', x: Math.round(rect.cx), y: Math.round(rect.cy),
    data: { items: [], files: [src('photo.png')], dragOperationsMask: 1 },
  })).then(() => c.send('Input.dispatchDragEvent', {
    type: 'drop', x: Math.round(rect.cx), y: Math.round(rect.cy),
    data: { items: [], files: [src('photo.png')], dragOperationsMask: 1 },
  })).then(() => 'dispatched').catch((e) => 'error: ' + e.message);
  await sleep(2500);
  await ev(`return await window.__ap.save();`);
  await sleep(600);
  r.after = { treeDelta: tree().filter((x) => !before.includes(x)), disk: readIf('Target.md'), notices: await ev(`return window.__ap.notices();`) };
  return r;
});

out.treeAtEnd = tree();
out.appJsonAtEnd = readIf('.obsidian/app.json');
out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
