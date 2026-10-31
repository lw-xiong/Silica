// Canvas write-back: what Obsidian writes when it saves a `.canvas`, driven over CDP.
// Answers the "Unprobed" items of Specification.md's canvas section: the key order
// of file, link and group nodes, where the optional edge keys sit, whether a canvas
// that is opened but not edited is rewritten, what a new canvas and a canvas without
// edges come back as, and whether unknown keys survive.
// Phases: fixtures | run   (node canvas.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run'].includes(phase)) { console.error('phase must be fixtures|run'); process.exit(2); }

const T0 = Date.now();

// ---- results file ---------------------------------------------------------
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.canvas = all.canvas || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.canvas[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

async function step(name, fn) {
  out.order.push(name);
  const t = Date.now();
  const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save();
  return out.steps[name];
}

// ---- host-side vault reads ------------------------------------------------
const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
const stat = (rel) => { try { const s = fs.statSync(vp(rel)); return { size: s.size, mtimeMs: Math.round(s.mtimeMs) }; } catch { return null; } };
const canvases = () => fs.readdirSync(VAULT).filter((n) => n.endsWith('.canvas')).sort();
const snapshot = () => Object.fromEntries(canvases().map((n) => [n, { ...stat(n), text: bytes(n) }]));

/** Wait until `rel`'s bytes differ from `before` (or the timeout passes). */
async function waitForWrite(rel, before, timeoutMs = 12000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    const now = bytes(rel);
    if (now !== before) { await sleep(800); return { changed: true, waitedMs: timeoutMs - (end - Date.now()) }; }
    await sleep(250);
  }
  return { changed: false, waitedMs: timeoutMs };
}

// ---- fixtures -------------------------------------------------------------
// Every optional key of every node and edge type, in a deliberately scrambled
// order, 2-space pretty-printed with a trailing newline — nothing Obsidian would
// write, so whatever comes back is Obsidian's own form. Unknown keys at every
// level check that plugin data survives.
const SCRAMBLED = {
  pluginTop: { k: 1 },
  edges: [
    { toSide: 'left', label: 'leads to', color: '2', id: 'e1', toEnd: 'arrow', fromEnd: 'none', toNode: 'f1', fromSide: 'right', fromNode: 't1', pluginEdge: true },
    { id: 'e2', fromNode: 'l1', toNode: 'f1', toEnd: 'none', fromEnd: 'arrow', fromSide: 'top', toSide: 'bottom' },
    { id: 'e3', fromNode: 'f1', toNode: 't1', fromSide: 'bottom', toSide: 'top', color: '#00ff00' },
  ],
  nodes: [
    { backgroundStyle: 'cover', background: 'pic.png', label: 'Area', x: -20, y: -20, width: 900, height: 800, type: 'group', id: 'g1', color: '4' },
    { color: '1', text: 'Hello', height: 60, width: 250, y: 0, x: 0, type: 'text', id: 't1', pluginNode: { k: 2 } },
    { subpath: '#Goals', file: 'Plan.md', height: 400, width: 400, y: 0, x: 300, type: 'file', id: 'f1', color: '#ff0000' },
    { url: 'https://example.com', x: 0, y: 200, width: 300, height: 200, type: 'link', id: 'l1' },
    { id: 'g2', type: 'group', x: 1000, y: 1000, width: 200, height: 100 },
  ],
};
const NO_EDGES = {
  nodes: [{ id: 'only', type: 'text', text: 'Alone', x: 0, y: 0, width: 250, height: 60 }],
};
// Nested shapes at every level, to learn when the formatter keeps a container on
// one line and when it expands it. Opened and closed, never edited.
const FORMATTER = {
  meta: { a: { b: {} } },
  list: [{ x: 1 }],
  flat: [1, 'two', null, true, 2.5, -3],
  empties: { e1: {}, e2: [] },
  nums: [1, 2],
  strs: ['a', 'b'],
  bools: [true, false],
  nulls: [null],
  objNull: { a: null },
  objBool: { a: true },
  mixed: [1, 'a'],
  nodes: [
    { id: 'n1', type: 'text', text: 'T', x: 0, y: 0, width: 100, height: 50, e1: {}, s: 'a{b}[c], d' },
    { id: 'n2', type: 'text', text: 'U', x: 0, y: 100, width: 100, height: 50, e2: [] },
    { id: 'n3', type: 'text', text: 'V', x: 0, y: 200, width: 100, height: 50, n1: [1, 2], n4: ['a', 'b'], n5: [true, null], n7: { t: ['a'] } },
    { id: 'n4', type: 'text', text: 'W', x: 0, y: 300, width: 100, height: 50, n2: [[1]], n3: { k: { j: 1 } } },
    { id: 'n5', type: 'text', text: 'X\nY "q" \\ \t \u00e9 \u2028', x: 0.5, y: -0, width: 1e3, height: 50 },
  ],
  edges: [],
};
// Groups and other nodes interleaved: an edited save reorders them (seen in the first run).
const ORDER = {
  nodes: [
    { id: 't1', type: 'text', text: 'A', x: 0, y: 0, width: 100, height: 50 },
    { id: 'g1', type: 'group', x: -10, y: -10, width: 400, height: 400, label: 'G1' },
    { id: 't2', type: 'text', text: 'B', x: 0, y: 100, width: 100, height: 50, color: '2' },
    { id: 'g2', type: 'group', x: 500, y: 500, width: 100, height: 100 },
    { id: 't3', type: 'text', text: 'C', x: 0, y: 200, width: 100, height: 50 },
  ],
  edges: [
    { id: 'e2', fromNode: 't2', fromSide: 'bottom', toNode: 't3', toSide: 'top' },
    { id: 'e1', fromNode: 't1', fromSide: 'bottom', toNode: 't2', toSide: 'top' },
    { label: 'all four', color: '5', toEnd: 'none', fromEnd: 'arrow', id: 'e3', fromNode: 't3', fromSide: 'right', toNode: 't1', toSide: 'right' },
  ],
};
const FIXTURES = {
  'Scrambled.canvas': JSON.stringify(SCRAMBLED, null, 2) + '\n',
  'Formatter.canvas': JSON.stringify(FORMATTER, null, 2) + '\n',
  'Order.canvas': JSON.stringify(ORDER, null, 2) + '\n',
  'Untouched.canvas': JSON.stringify(SCRAMBLED, null, 2) + '\n',
  'NoEdges.canvas': JSON.stringify(NO_EDGES, null, 2) + '\n',
  'Plan.md': '# Plan\n\n## Goals\n\nSome goals.\n',
};

if (phase === 'fixtures') {
  await step('fixtures', async () => {
    fs.mkdirSync(vp('.obsidian'), { recursive: true });
    fs.writeFileSync(vp('.obsidian/app.json'), '{}');
    for (const [name, text] of Object.entries(FIXTURES)) fs.writeFileSync(vp(name), text);
    return { written: snapshot(), vaultFiles: fs.readdirSync(VAULT).sort() };
  });
  process.exit(0);
}

// ---- run ------------------------------------------------------------------
const c = await connect();
const vault = await c.vaultPath();
if (vault.replace(/\/+$/, '') !== VAULT.replace(/\/+$/, '')) { console.error('wrong vault open: ' + vault); c.close(); process.exit(3); }
out.vault = vault;
out.version = await c.evaluate(`return { title: document.title, ua: navigator.userAgent };`);

const openCanvas = async (name) => {
  await c.evaluate(`
    const f = app.vault.getAbstractFileByPath(${JSON.stringify(name)});
    if (!f) throw new Error('no such file ' + ${JSON.stringify(name)});
    await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(1500);
  return c.evaluate(`
    const v = app.workspace.activeLeaf && app.workspace.activeLeaf.view;
    return { viewType: v && v.getViewType(), file: v && v.file && v.file.path,
             nodes: v && v.canvas ? v.canvas.nodes.size : null,
             edges: v && v.canvas ? v.canvas.edges.size : null,
             nodeEls: document.querySelectorAll('.canvas-node').length };`);
};
const closeActive = async () => { await c.evaluate(`app.workspace.activeLeaf.detach(); return true;`); await sleep(1000); };

await step('00-before', async () => ({ snapshot: snapshot() }));

// Open, look, close — nobody edits. Is the file rewritten?
await step('01-untouched-open', async () => {
  const before = bytes('Untouched.canvas');
  const opened = await openCanvas('Untouched.canvas');
  await sleep(4000);
  const afterOpen = bytes('Untouched.canvas');
  await closeActive();
  await sleep(2500);
  const afterClose = bytes('Untouched.canvas');
  return { opened, rewrittenOnOpen: afterOpen !== before, rewrittenOnClose: afterClose !== before, text: afterClose, stat: stat('Untouched.canvas') };
});

// A real drag of the text node: a user's edit, then Obsidian's own save.
await step('02-scrambled-drag', async (p) => {
  const before = bytes('Scrambled.canvas');
  p.opened = await openCanvas('Scrambled.canvas');
  // Locate node t1 on screen through the canvas model.
  p.nodeRect = await c.evaluate(`
    const v = app.workspace.activeLeaf.view; const n = v.canvas.nodes.get('t1');
    if (!n) throw new Error('no node t1');
    const r = n.nodeEl.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2, model: { x: n.x, y: n.y } };`);
  const r = p.nodeRect;
  // Press near the top edge of the node, not the center: the center is the text body.
  await c.drag(r.cx, r.y + 6, r.cx + 120, r.y + 6 + 80, { steps: 16, pauseMs: 40 });
  await sleep(500);
  p.modelAfterDrag = await c.evaluate(`
    const n = app.workspace.activeLeaf.view.canvas.nodes.get('t1'); return { x: n.x, y: n.y };`);
  const w = await waitForWrite('Scrambled.canvas', before);
  p.write = w;
  const afterDrag = bytes('Scrambled.canvas');
  await closeActive();
  await sleep(2500);
  const afterClose = bytes('Scrambled.canvas');
  return { ...p, textAfterDrag: afterDrag, changedAgainOnClose: afterClose !== afterDrag, text: afterClose, stat: stat('Scrambled.canvas') };
});

// No `edges` key in the file at all: does Obsidian add an empty one?
await step('03-no-edges-drag', async (p) => {
  const before = bytes('NoEdges.canvas');
  p.opened = await openCanvas('NoEdges.canvas');
  const r = await c.evaluate(`
    const n = app.workspace.activeLeaf.view.canvas.nodes.get('only'); const r = n.nodeEl.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 };`);
  await c.drag(r.cx, r.y + 6, r.cx + 60, r.y + 6 + 40, { steps: 12, pauseMs: 40 });
  await sleep(500);
  p.dragWrite = await waitForWrite('NoEdges.canvas', before, 4000);
  // The first run's drag grabbed a connection handle; move through the model and ask for the save the UI would.
  p.moved = await c.evaluate(`
    const v = app.workspace.activeLeaf.view; const n = v.canvas.nodes.get('only');
    n.moveTo({ x: 40, y: 30 }); v.canvas.requestSave(); return { x: n.x, y: n.y };`);
  p.write = await waitForWrite('NoEdges.canvas', bytes('NoEdges.canvas') === before ? before : null);
  const afterDrag = bytes('NoEdges.canvas');
  await closeActive();
  await sleep(2500);
  return { ...p, textAfterDrag: afterDrag, text: bytes('NoEdges.canvas') };
});

// A brand-new canvas from the command: the file Obsidian creates before anything is in it.
await step('04-new-canvas', async (p) => {
  const names = canvases();
  p.ran = await c.evaluate(`return app.commands.executeCommandById('canvas:new-file');`);
  await sleep(3000);
  const created = canvases().filter((n) => !names.includes(n));
  p.created = created;
  const name = created[0];
  const text = name ? bytes(name) : null;
  // Add one text node through the UI's own API, so the empty file is followed by a one-node file.
  let afterNode = null;
  if (name) {
    await c.evaluate(`
      const v = app.workspace.activeLeaf.view;
      v.canvas.createTextNode({ pos: { x: 10, y: 20 }, size: { width: 250, height: 60 }, text: 'Typed', save: true, focus: false });
      v.canvas.requestSave(); return true;`);
    await waitForWrite(name, text);
    afterNode = bytes(name);
  }
  await closeActive();
  return { ...p, text, afterNode };
});

// The formatter alone: open and close, no edit.
await step('06-formatter-open-close', async (p) => {
  const before = bytes('Formatter.canvas');
  p.opened = await openCanvas('Formatter.canvas');
  await sleep(2000);
  p.rewrittenOnOpen = bytes('Formatter.canvas') !== before;
  await closeActive();
  await sleep(2500);
  return { ...p, text: bytes('Formatter.canvas') };
});

// Node order on an edited save: move one node through the model and ask for the save.
await step('07-order-edit', async (p) => {
  const before = bytes('Order.canvas');
  p.opened = await openCanvas('Order.canvas');
  p.modelOrder = await c.evaluate(`return Array.from(app.workspace.activeLeaf.view.canvas.nodes.keys());`);
  p.moved = await c.evaluate(`
    const v = app.workspace.activeLeaf.view; const n = v.canvas.nodes.get('t3');
    n.moveTo({ x: 10, y: 210 }); v.canvas.requestSave(); return { x: n.x, y: n.y };`);
  p.write = await waitForWrite('Order.canvas', before);
  p.textAfterEdit = bytes('Order.canvas');
  await closeActive();
  await sleep(2500);
  return { ...p, text: bytes('Order.canvas') };
});

// Nodes of every type created through the app's own API in a new canvas: the key
// order Obsidian writes when no file order exists to inherit.
await step('08-api-created-nodes', async (p) => {
  const names = canvases();
  await c.evaluate(`return app.commands.executeCommandById('canvas:new-file');`);
  await sleep(3000);
  const name = canvases().filter((n) => !names.includes(n))[0];
  p.created = name;
  const before = bytes(name);
  p.api = await c.evaluate(`
    const v = app.workspace.activeLeaf.view; const cv = v.canvas; const r = {};
    const tryIt = (k, f) => { try { r[k] = f(); } catch (e) { r[k] = 'ERR:' + e.message; } };
    tryIt('text', () => { const n = cv.createTextNode({ pos: { x: 0, y: 0 }, size: { width: 250, height: 60 }, text: 'T', save: false, focus: false }); return n.id; });
    tryIt('file', () => { const n = cv.createFileNode({ pos: { x: 300, y: 0 }, size: { width: 400, height: 400 }, file: app.vault.getAbstractFileByPath('Plan.md'), subpath: '#Goals', save: false, focus: false }); return n.id; });
    tryIt('link', () => { const n = cv.createLinkNode({ pos: { x: 0, y: 200 }, size: { width: 300, height: 200 }, url: 'https://example.com', save: false, focus: false }); return n.id; });
    tryIt('group', () => { const n = cv.createGroupNode({ pos: { x: -20, y: -20 }, size: { width: 900, height: 800 }, label: 'Area', save: false, focus: false }); return n.id; });
    tryIt('groupPlain', () => { const n = cv.createGroupNode({ pos: { x: 1000, y: 1000 }, size: { width: 100, height: 100 }, save: false, focus: false }); return n.id; });
    tryIt('colorText', () => { const n = cv.createTextNode({ pos: { x: 0, y: 400 }, size: { width: 250, height: 60 }, text: 'Colored', save: false, focus: false }); n.setColor('3'); return n.id; });
    tryIt('methods', () => Object.getOwnPropertyNames(Object.getPrototypeOf(cv)).filter(k => /create|add|Edge|save|Save|Data/.test(k)));
    tryIt('nodeMethods', () => { const n = cv.nodes.values().next().value; return Object.getOwnPropertyNames(Object.getPrototypeOf(n)).concat(Object.getOwnPropertyNames(Object.getPrototypeOf(Object.getPrototypeOf(n)))); });
    tryIt('getData', () => JSON.stringify(cv.getData()));
    cv.requestSave();
    return r;`);
  p.write = await waitForWrite(name, before);
  p.text = bytes(name);
  await closeActive();
  await sleep(2500);
  return { ...p, textAfterClose: bytes(name) };
});

// Was the untouched file still untouched by everything that happened after it?
await step('05-after', async () => ({ snapshot: snapshot(), untouchedStillOriginal: bytes('Untouched.canvas') === FIXTURES['Untouched.canvas'] }));

save();
c.close();
process.exit(0);
