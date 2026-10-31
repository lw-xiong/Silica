// Canvas view: how Obsidian draws and drives a `.canvas`, observed over CDP for
// Silica's canvas view. Settings file and keys, the leaf's viewState and what
// x/y/zoom mean, zoom and pan limits and steps, card defaults and snapping, the
// zoom threshold, edge geometry and styling, groups, the keyboard map, the
// menus, text editing, the dot grid, and save timing.
// Phases: fixtures | run   (node canvas-view.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { execFileSync } from 'node:child_process';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const only = process.argv[3]; // optional: run only steps whose name starts with this
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
const SHOTS = process.env.SHOTS; // optional directory for spectacle screenshots
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run'].includes(phase)) { console.error('phase must be fixtures|run'); process.exit(2); }

const T0 = Date.now();

// ---- results file ---------------------------------------------------------
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.canvasView = all.canvasView || {};
const prev = all.canvasView[phase];
const out = only && prev ? prev : { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.canvasView[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

async function step(name, fn) {
  if (only && !name.startsWith(only)) return out.steps[name];
  if (!out.order.includes(name)) out.order.push(name);
  const t = Date.now();
  const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save();
  console.error(`[${name}] ${out.steps[name].ok ? 'ok' : 'FAIL'} ${out.steps[name].ms}ms`);
  return out.steps[name];
}

// ---- host-side vault reads ------------------------------------------------
const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
const json = (rel) => { const t = bytes(rel); try { return t == null ? null : JSON.parse(t); } catch { return t; } };
const obsidianDir = () => fs.readdirSync(vp('.obsidian')).sort();

/** Wait until `rel`'s bytes differ from `before`; returns how long it took. */
async function waitForWrite(rel, before, timeoutMs = 12000) {
  const t = Date.now();
  while (Date.now() - t < timeoutMs) {
    if (bytes(rel) !== before) return { changed: true, afterMs: Date.now() - t };
    await sleep(100);
  }
  return { changed: false, afterMs: timeoutMs };
}

/** Full-desktop or active-window screenshot into SHOTS, if set. */
function shot(name, active = true) {
  if (!SHOTS) return null;
  fs.mkdirSync(SHOTS, { recursive: true });
  const file = path.join(SHOTS, name + '.png');
  try { execFileSync('spectacle', ['-b', '-n', active ? '-a' : '-f', '-o', file], { stdio: 'ignore', timeout: 20000 }); return file; }
  catch (e) { return 'ERR:' + e.message; }
}

// ---- fixtures -------------------------------------------------------------
// A tiny PNG: 64x48 solid orange, so a file node can embed an image and a group a background.
function png(w, h, rgb) {
  const crc = (buf) => { let c = ~0; for (const b of buf) { c ^= b; for (let i = 0; i < 8; i++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1)); } return (~c) >>> 0; };
  const chunk = (type, data) => { const len = Buffer.alloc(4); len.writeUInt32BE(data.length); const td = Buffer.concat([Buffer.from(type), data]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([len, td, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;
  const row = Buffer.concat([Buffer.from([0]), Buffer.from(Array.from({ length: w }, () => rgb).flat())]);
  const raw = Buffer.concat(Array.from({ length: h }, () => row));
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

// Every node type and every edge shape, in canvas units. Text cards sit in a
// row at y=0 so edges between neighbours are horizontal; the file, image and
// link cards below; the groups around them.
const FIXTURE = {
  nodes: [
    { id: 'g1', type: 'group', x: -60, y: -120, width: 1500, height: 320, label: 'Colored group', color: '4' },
    { id: 'g2', type: 'group', x: -60, y: 260, width: 1500, height: 560, label: 'Plain group' },
    { id: 'g3', type: 'group', x: 1600, y: -120, width: 500, height: 300, label: 'Picture group', background: 'pic.png', backgroundStyle: 'cover' },
    { id: 't0', type: 'text', text: '# Heading\n\n- item one\n- **bold** item\n\n[[Plan]]', x: 0, y: 0, width: 260, height: 160 },
    { id: 't1', type: 'text', text: 'Red 1', x: 320, y: 0, width: 160, height: 60, color: '1' },
    { id: 't2', type: 'text', text: 'Orange 2', x: 540, y: 0, width: 160, height: 60, color: '2' },
    { id: 't3', type: 'text', text: 'Yellow 3', x: 760, y: 0, width: 160, height: 60, color: '3' },
    { id: 't4', type: 'text', text: 'Green 4', x: 980, y: 0, width: 160, height: 60, color: '4' },
    { id: 't5', type: 'text', text: 'Cyan 5', x: 1200, y: 0, width: 160, height: 60, color: '5' },
    { id: 't6', type: 'text', text: 'Purple 6', x: 320, y: 120, width: 160, height: 60, color: '6' },
    { id: 't7', type: 'text', text: 'Hex', x: 540, y: 120, width: 160, height: 60, color: '#ff8800' },
    { id: 'f1', type: 'file', file: 'Plan.md', x: 0, y: 300, width: 400, height: 400 },
    { id: 'f2', type: 'file', file: 'pic.png', x: 500, y: 300, width: 320, height: 240 },
    { id: 'l1', type: 'link', url: 'https://example.com', x: 900, y: 300, width: 400, height: 300 },
  ],
  edges: [
    { id: 'e1', fromNode: 't0', fromSide: 'right', toNode: 't1', toSide: 'left', label: 'leads to', color: '2' },
    { id: 'e2', fromNode: 't2', fromSide: 'right', toNode: 't3', toSide: 'left', fromEnd: 'arrow', toEnd: 'none', label: 'back' },
    { id: 'e3', fromNode: 't4', fromSide: 'right', toNode: 't5', toSide: 'left', toEnd: 'none' },
    { id: 'e4', fromNode: 't6', fromSide: 'right', toNode: 't7', toSide: 'left', fromEnd: 'arrow' },
    { id: 'e5', fromNode: 'f1', fromSide: 'right', toNode: 'f2', toSide: 'left', color: '#00aaff', label: 'hex edge' },
    { id: 'e6', fromNode: 't0', fromSide: 'bottom', toNode: 'f1', toSide: 'top' },
    { id: 'e7', fromNode: 'f2', fromSide: 'bottom', toNode: 'l1', toSide: 'top', color: '5' },
  ],
};
// Two plain cards for geometry: Bézier control points, snapping, resize, selection.
const TWO = {
  nodes: [
    { id: 'a', type: 'text', text: 'A', x: 0, y: 0, width: 200, height: 100 },
    { id: 'b', type: 'text', text: 'B', x: 600, y: 0, width: 200, height: 100 },
  ],
  edges: [{ id: 'ab', fromNode: 'a', fromSide: 'right', toNode: 'b', toSide: 'left' }],
};
const FIXTURES = {
  'Fixture.canvas': JSON.stringify(FIXTURE, null, 2) + '\n',
  'Two.canvas': JSON.stringify(TWO, null, 2) + '\n',
  'Empty.canvas': JSON.stringify({ nodes: [], edges: [] }) + '\n',
  'Plan.md': '# Plan\n\n## Goals\n\nSome goals, with a [[Fixture.canvas|canvas link]].\n\n- one\n- two\n',
  'Long.md': '# Long\n\n' + Array.from({ length: 60 }, (_, i) => `Paragraph ${i + 1} of a long note so the file card scrolls.`).join('\n\n') + '\n',
};

if (phase === 'fixtures') {
  await step('fixtures', async () => {
    fs.mkdirSync(vp('.obsidian'), { recursive: true });
    fs.writeFileSync(vp('.obsidian/app.json'), '{}');
    for (const [name, text] of Object.entries(FIXTURES)) fs.writeFileSync(vp(name), text);
    fs.writeFileSync(vp('pic.png'), png(64, 48, [255, 136, 0]));
    return { vaultFiles: fs.readdirSync(VAULT).sort(), obsidian: obsidianDir() };
  });
  process.exit(0);
}

// ---- run ------------------------------------------------------------------
const c = await connect();
const vault = await c.vaultPath();
if (vault.replace(/\/+$/, '') !== VAULT.replace(/\/+$/, '')) { console.error('wrong vault open: ' + vault); c.close(); process.exit(3); }
out.vault = vault;
out.version = await c.evaluate(`return { title: document.title, ua: navigator.userAgent, apiVersion: typeof apiVersion !== 'undefined' ? apiVersion : null,
  inner: { w: window.innerWidth, h: window.innerHeight }, dpr: window.devicePixelRatio, theme: app.vault.getConfig('theme'), bodyTheme: document.body.className };`);

const openCanvas = async (name) => {
  await c.evaluate(`
    const f = app.vault.getAbstractFileByPath(${JSON.stringify(name)});
    if (!f) throw new Error('no such file ' + ${JSON.stringify(name)});
    const leaf = app.workspace.getLeaf(false); await leaf.openFile(f); app.workspace.setActiveLeaf(leaf, { focus: true }); return true;`);
  await sleep(1500);
  return c.evaluate(`
    const v = app.workspace.activeLeaf && app.workspace.activeLeaf.view;
    return { viewType: v && v.getViewType(), file: v && v.file && v.file.path,
             nodes: v && v.canvas ? v.canvas.nodes.size : null, edges: v && v.canvas ? v.canvas.edges.size : null };`);
};
const closeActive = async () => { await c.evaluate(`app.workspace.activeLeaf.detach(); return true;`); await sleep(800); };
const CV = `const leaf0 = app.workspace.getLeavesOfType('canvas')[0]; if (!leaf0) throw new Error('no canvas leaf'); const v = leaf0.view; const cv = v.canvas;`;
const protoNames = (expr) => c.evaluate(`${CV} let o = ${expr}; const r = {}; let depth = 0;
  while (o && o !== Object.prototype && depth < 6) { r[(o.constructor && o.constructor.name) || ('proto' + depth)] = Object.getOwnPropertyNames(o).filter(k => k !== 'constructor'); o = Object.getPrototypeOf(o); depth++; }
  return r;`);

await step('00-env', async () => ({ version: out.version, obsidianDir: obsidianDir(), appJson: json('.obsidian/app.json'), corePlugins: json('.obsidian/core-plugins.json'),
  canvasPlugin: await c.evaluate(`const p = app.internalPlugins.plugins.canvas; return { enabled: p.enabled, keys: Object.keys(p), instanceKeys: p.instance ? Object.keys(p.instance) : null, options: p.instance && p.instance.options, defaults: p.instance && p.instance.defaultOptions };`) }));

await step('01-surface', async (p) => {
  p.opened = await openCanvas('Fixture.canvas');
  p.canvasKeys = await c.evaluate(`${CV} return Object.keys(cv).map(k => [k, typeof cv[k] === 'object' && cv[k] !== null ? (cv[k].constructor && cv[k].constructor.name) : typeof cv[k] === 'function' ? 'fn' : JSON.stringify(cv[k])]);`);
  p.canvasProto = await protoNames('cv');
  p.viewKeys = await c.evaluate(`${CV} return Object.keys(v);`);
  p.viewProto = await protoNames('v');
  p.nodeProto = await protoNames(`cv.nodes.get('t0')`);
  p.nodeKeys = await c.evaluate(`${CV} const n = cv.nodes.get('t0'); return Object.keys(n).map(k => [k, typeof n[k] === 'object' && n[k] !== null ? (n[k].constructor && n[k].constructor.name) : typeof n[k] === 'function' ? 'fn' : JSON.stringify(n[k])]);`);
  p.edgeProto = await protoNames(`cv.edges.get('e1')`);
  p.edgeKeys = await c.evaluate(`${CV} const e = cv.edges.get('e1'); return Object.keys(e).map(k => [k, typeof e[k] === 'object' && e[k] !== null ? (e[k].constructor && e[k].constructor.name) : typeof e[k] === 'function' ? 'fn' : JSON.stringify(e[k])]);`);
  p.config = await c.evaluate(`${CV} return { config: cv.config, options: cv.options, snapToGrid: cv.snapToGrid, snapToObjects: cv.snapToObjects, zoom: cv.zoom, tZoom: cv.tZoom, x: cv.x, y: cv.y, tx: cv.tx, ty: cv.ty, scale: cv.scale, zoomCenter: cv.zoomCenter, screenshotting: cv.screenshotting, readonly: cv.readonly, viewportEl: cv.viewportEl && cv.viewportEl.className, wrapperEl: cv.wrapperEl && cv.wrapperEl.className, canvasEl: cv.canvasEl && cv.canvasEl.className };`);
  p.leafState = await c.evaluate(`${CV} const leaf = app.workspace.getLeavesOfType('canvas')[0]; return { viewState: leaf.getViewState(), state: v.getState(), icon: v.getIcon && v.getIcon(), title: v.getDisplayText && v.getDisplayText(), ephemeral: v.getEphemeralState && v.getEphemeralState() };`);
  p.dom = await c.evaluate(`${CV} const walk = (el, d) => d > 3 ? [] : Array.from(el.children).slice(0, 12).map(ch => ({ tag: ch.tagName, cls: ch.className && ch.className.baseVal !== undefined ? ch.className.baseVal : ch.className, kids: walk(ch, d + 1) }));
    return walk(v.contentEl, 0);`);
  return p;
});

await step('02-commands', async () => c.evaluate(`
  const ids = Object.keys(app.commands.commands).filter(k => k.startsWith('canvas:')).sort();
  const dk = app.hotkeyManager.defaultKeys || {};
  return { commands: ids.map(id => { const cmd = app.commands.commands[id]; return { id, name: cmd.name, icon: cmd.icon, hotkeys: cmd.hotkeys || null, defaultHotkeys: (app.hotkeyManager.getDefaultHotkeys ? app.hotkeyManager.getDefaultHotkeys(id) : dk[id]) || null, checkCallback: !!cmd.checkCallback, editorCallback: !!cmd.editorCallback }; }),
    allWithCanvasInName: Object.values(app.commands.commands).filter(c => /canvas/i.test(c.name) && !c.id.startsWith('canvas:')).map(c => [c.id, c.name]) };`));

const CSS_VARS = `const names = new Set(); for (const sheet of document.styleSheets) { let rules; try { rules = sheet.cssRules; } catch { continue; }
  for (const r of rules) { if (!r.style) continue; for (const p of r.style) if (p.startsWith('--canvas') || /^--color-(red|orange|yellow|green|cyan|blue|purple|pink)(-rgb)?$/.test(p)) names.add(p); } }
  const cs = getComputedStyle(document.body); const r = {}; for (const n of Array.from(names).sort()) r[n] = cs.getPropertyValue(n).trim(); return r;`;
const COMPUTED = `const pick = (sel, props, idx = 0) => { const el = document.querySelectorAll(sel)[idx]; if (!el) return null; const cs = getComputedStyle(el); const r = { _rect: (() => { const b = el.getBoundingClientRect(); return [b.x, b.y, b.width, b.height].map(x => Math.round(x * 100) / 100); })() }; for (const p of props) r[p] = cs.getPropertyValue(p); return r; };
  const box = ['width','height','padding','border-width','border-color','border-style','border-radius','background-color','color','font-size','font-weight','line-height','opacity','box-shadow','outline','outline-offset','transform','transform-origin','overflow','top','left','right','bottom','position','box-sizing','letter-spacing','text-transform','gap','margin'];
  const svg = ['stroke','stroke-width','fill','stroke-linecap','stroke-dasharray','opacity','pointer-events','marker-end','marker-start'];
  return { body: pick('body', ['--canvas-background','--canvas-color','--canvas-dot-pattern','--canvas-card-label-color','background-color','color','font-size','font-family']),
    wrapper: pick('.canvas-wrapper', box), canvas: pick('.canvas', box.concat(['background-image','background-size','background-position'])), background: pick('.canvas-background', box.concat(['background-image','background-size','background-position'])),
    bgPatternCircle: pick('.canvas-background circle, .canvas-background pattern circle', ['fill','r','cx','cy','fill-opacity']),
    node: pick('.canvas-node', box), nodeContainer: pick('.canvas-node-container', box), nodeContent: pick('.canvas-node-content', box), nodeLabel: pick('.canvas-node-label', box),
    groupLabel: pick('.canvas-group-label', box), groupNode: pick('.canvas-node-group', box), groupContainer: pick('.canvas-node-group .canvas-node-container', box),
    edgePath: pick('.canvas-edge path, .canvas-edges path', svg.concat(['d'])), edgeInteraction: pick('.canvas-interaction-path, .canvas-edge-interaction', svg), edgeDisplay: pick('.canvas-display-path', svg),
    edgeLabel: pick('.canvas-path-label, .canvas-edge-label', box), controls: pick('.canvas-controls', box), controlGroup: pick('.canvas-control-group', box), controlItem: pick('.canvas-control-item', box),
    cardMenu: pick('.canvas-card-menu', box), cardMenuButton: pick('.canvas-card-menu-button', box), nodeMenu: pick('.canvas-menu', box), nodeMenuItem: pick('.canvas-menu .clickable-icon', box),
    connectionPoint: pick('.canvas-node-connection-point', box), resizeHandle: pick('.canvas-node-resizer', box) };`;

await step('03-css-dark', async (p) => {
  p.theme = await c.evaluate(`app.changeTheme('obsidian'); await new Promise(r => setTimeout(r, 300)); return { theme: app.vault.getConfig('theme'), body: document.body.className };`);
  p.vars = await c.evaluate(CSS_VARS);
  p.computed = await c.evaluate(COMPUTED);
  return p;
});
await step('03-css-light', async (p) => {
  p.theme = await c.evaluate(`app.changeTheme('moonstone'); await new Promise(r => setTimeout(r, 300)); return { theme: app.vault.getConfig('theme'), body: document.body.className };`);
  p.vars = await c.evaluate(CSS_VARS);
  p.computed = await c.evaluate(COMPUTED);
  await c.evaluate(`app.changeTheme('obsidian'); return true;`);
  return p;
});


// ---- helpers for the interaction steps -------------------------------------
/** Pan and zoom state plus the wrapper's rect, so screen<->canvas can be checked by hand. */
const view = () => c.evaluate(`${CV} const w = cv.wrapperEl.getBoundingClientRect();
  return { x: cv.x, y: cv.y, zoom: cv.zoom, tZoom: cv.tZoom, tx: cv.tx, ty: cv.ty, scale: cv.scale, zoomedOut: cv.wrapperEl.classList.contains('mod-zoomed-out'),
    wrapper: { x: w.x, y: w.y, w: w.width, h: w.height, cx: w.x + w.width / 2, cy: w.y + w.height / 2 }, inner: { w: window.innerWidth, h: window.innerHeight },
    selection: Array.from(cv.selection).map(n => n.id), nodes: cv.nodes.size, edges: cv.edges.size };`);
const node = (id) => c.evaluate(`${CV} const n = cv.nodes.get(${JSON.stringify(id)}); if (!n) return null; const r = n.nodeEl.getBoundingClientRect();
  return { id: n.id, x: n.x, y: n.y, width: n.width, height: n.height, color: n.color, isEditing: n.isEditing, zIndex: n.zIndex, cls: n.nodeEl.className,
    screen: { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 }, data: n.getData() };`);
const nodeIds = () => c.evaluate(`${CV} return Array.from(cv.nodes.keys());`);
const edgesData = () => c.evaluate(`${CV} return Array.from(cv.edges.values()).map(e => e.getData());`);
const wheel = async (x, y, dy, mods = {}, dx = 0) => { const m = (mods.alt ? 1 : 0) | (mods.ctrl ? 2 : 0) | (mods.meta ? 4 : 0) | (mods.shift ? 8 : 0);
  await c.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: Math.round(x), y: Math.round(y), deltaX: dx, deltaY: dy, modifiers: m }); await sleep(250); };
const setViewport = (x, y, zoom) => c.evaluate(`${CV} cv.setViewport(${x}, ${y}, ${zoom}); cv.finishViewportAnimation = true; await new Promise(r => setTimeout(r, 400)); return { x: cv.x, y: cv.y, zoom: cv.zoom, scale: cv.scale };`);
const modal = () => c.evaluate(`const m = document.querySelector('.modal-container'); if (!m) return null; return { title: (m.querySelector('.modal-title') || {}).textContent, text: m.innerText.slice(0, 600), prompt: !!m.querySelector('.prompt') };`);
const ariaList = (sel) => c.evaluate(`return Array.from(document.querySelectorAll(${JSON.stringify(sel)})).map(el => ({ label: el.getAttribute('aria-label'), tip: el.getAttribute('data-tooltip-position'), cls: el.className, icon: (el.querySelector('svg') || {}).getAttribute ? el.querySelector('svg').getAttribute('class') : null, disabled: el.classList.contains('is-disabled') }));`);
/** Open a canvas fresh (closing other canvas leaves) and settle. */
const fresh = async (name) => { await c.evaluate(`for (const l of app.workspace.getLeavesOfType('canvas')) l.detach(); return true;`); await sleep(600);
  if (FIXTURES[name] && bytes(name) !== FIXTURES[name]) { fs.writeFileSync(vp(name), FIXTURES[name]); await sleep(2500); }
  let r = await openCanvas(name);
  const want = FIXTURES[name] ? JSON.parse(FIXTURES[name]).nodes.length : null;
  for (let i = 0; want !== null && r.nodes !== want && i < 3; i++) { await c.evaluate(`for (const l of app.workspace.getLeavesOfType('canvas')) l.detach(); return true;`); await sleep(2500); r = await openCanvas(name); }
  return r; };
/** A point on a card that starts a move, not a resize or a connection: centre-left of its body. */
const grab = (n) => ({ x: n.screen.x + n.screen.w * 0.25, y: n.screen.y + n.screen.h * 0.5 });
/** The interaction layer's handles for the node it currently targets (hover or select it first). */
const handles = () => c.evaluate(`${CV} const el = cv.nodeInteractionLayer.interactionEl; const target = cv.nodeInteractionLayer.target; const nr = target ? target.nodeEl.getBoundingClientRect() : null;
  const strips = Array.from(el.querySelectorAll('.canvas-node-resizer')).map(e => { const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); const cp = e.querySelector('.canvas-node-connection-point'); const cpr = cp && cp.getBoundingClientRect(); const after = cp && getComputedStyle(cp, '::after'); const before = cp && getComputedStyle(cp, '::before');
    const where = nr ? ((r.width > r.height ? (r.y + r.height / 2 < nr.y + nr.height / 2 ? 'top' : 'bottom') : (r.x + r.width / 2 < nr.x + nr.width / 2 ? 'left' : 'right'))) : null;
    const corner = nr && r.width === r.height ? ((r.y + r.height / 2 < nr.y + nr.height / 2 ? 'top' : 'bottom') + '-' + (r.x + r.width / 2 < nr.x + nr.width / 2 ? 'left' : 'right')) : null;
    return { cls: e.getAttribute('class'), rect: [r.x, r.y, r.width, r.height], cursor: cs.cursor, w: cs.width, h: cs.height, side: corner || where, cx: r.x + r.width / 2, cy: r.y + r.height / 2,
      dot: cp ? { rect: [cpr.x, cpr.y, cpr.width, cpr.height], cx: cpr.x + cpr.width / 2, cy: cpr.y + cpr.height / 2, w: getComputedStyle(cp).width, after: { w: after.width, h: after.height, bg: after.backgroundColor, radius: after.borderRadius, border: after.border, content: after.content, opacity: after.opacity, transform: after.transform, boxShadow: after.boxShadow, display: after.display }, before: { w: before.width, content: before.content, bg: before.backgroundColor } } : null }; });
  return { target: target && target.id, layerStyle: el.getAttribute('style'), nodeRect: nr && [nr.x, nr.y, nr.width, nr.height], strips };`);
const clickMenuItem = async (title) => { const idx = await c.evaluate(`const items = Array.from(document.querySelectorAll('.menu .menu-item')); return items.findIndex(m => ((m.querySelector('.menu-item-title') || {}).textContent || '').trim() === ${JSON.stringify(title)});`); if (idx < 0) return false; const r = await c.rect('.menu .menu-item', idx); await c.clickAt(r.cx, r.cy); await sleep(500); return true; };
const bgSpot = async () => { // a screen point on empty canvas: below the cards in Two.canvas after zoom-to-fit
  const v = await view(); return { x: v.wrapper.x + 40, y: v.wrapper.y + v.wrapper.h - 120 }; };

// Settings: the Canvas tab's rows, each toggle clicked, and what lands under .obsidian.
await step('05-settings', async (p) => {
  p.before = obsidianDir();
  p.tab = await c.evaluate(`app.setting.open(); app.setting.openTabById('canvas'); await new Promise(r => setTimeout(r, 1200));
    const t = app.setting.activeTab; const el = t.containerEl;
    return { id: t.id, name: t.name, items: Array.from(el.querySelectorAll('.setting-item')).map(it => ({ name: (it.querySelector('.setting-item-name') || {}).textContent, desc: (it.querySelector('.setting-item-description') || {}).textContent,
      toggle: it.querySelector('.checkbox-container') ? it.querySelector('.checkbox-container').classList.contains('is-enabled') : null, text: it.querySelector('input[type=text]') ? it.querySelector('input[type=text]').value : null,
      dropdown: it.querySelector('select') ? { value: it.querySelector('select').value, options: Array.from(it.querySelector('select').options).map(o => [o.value, o.textContent]) } : null, cls: it.className })) };`);
  p.toggles = [];
  const n = await c.evaluate(`return app.setting.activeTab.containerEl.querySelectorAll('.checkbox-container').length;`);
  for (let i = 0; i < n; i++) {
    const before = Object.fromEntries(obsidianDir().map(f => [f, bytes('.obsidian/' + f)]));
    const r = await c.evaluate(`const cb = app.setting.activeTab.containerEl.querySelectorAll('.checkbox-container')[${i}]; cb.click(); await new Promise(r => setTimeout(r, 2500)); return { enabled: cb.classList.contains('is-enabled'), options: JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options)) };`);
    const after = Object.fromEntries(obsidianDir().map(f => [f, bytes('.obsidian/' + f)]));
    const diff = {}; for (const f of new Set([...Object.keys(before), ...Object.keys(after)])) if (before[f] !== after[f]) diff[f] = { before: before[f] ?? null, after: after[f] ?? null };
    p.toggles.push({ i, ...r, diff });
    // toggle back
    await c.evaluate(`app.setting.activeTab.containerEl.querySelectorAll('.checkbox-container')[${i}].click(); await new Promise(r => setTimeout(r, 2500)); return true;`);
  }
  // Dropdowns: one option per evaluate (the whole loop in one call outruns CDP's 15 s evaluate timeout).
  p.dropdowns = [];
  const dds = await c.evaluate(`return Array.from(app.setting.activeTab.containerEl.querySelectorAll('select')).map(s => ({ value: s.value, options: Array.from(s.options).map(o => o.value) }));`);
  for (let d = 0; d < dds.length; d++) {
    for (const o of dds[d].options.concat([dds[d].value])) {
      const before = Object.fromEntries(obsidianDir().map(f => [f, bytes('.obsidian/' + f)]));
      const r = await c.evaluate(`const sel = app.setting.activeTab.containerEl.querySelectorAll('select')[${d}]; sel.value = ${JSON.stringify(o)}; sel.dispatchEvent(new Event('change', { bubbles: true })); await new Promise(r => setTimeout(r, 1800)); return { value: sel.value, options: JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options)) };`);
      const after = Object.fromEntries(obsidianDir().map(f => [f, bytes('.obsidian/' + f)]));
      const diff = {}; for (const f of new Set([...Object.keys(before), ...Object.keys(after)])) if (before[f] !== after[f]) diff[f] = after[f] ?? null;
      p.dropdowns.push({ dropdown: d, set: o, ...r, diff });
    }
  }
  // The folder text field and the zoom-threshold slider.
  p.textField = await c.evaluate(`const el = app.setting.activeTab.containerEl; const inp = el.querySelector('input[type=text]'); if (!inp) return null; inp.value = 'Canvases'; inp.dispatchEvent(new Event('input', { bubbles: true })); inp.dispatchEvent(new Event('change', { bubbles: true })); await new Promise(r => setTimeout(r, 1800)); const opts = JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options)); inp.value = ''; inp.dispatchEvent(new Event('input', { bubbles: true })); inp.dispatchEvent(new Event('change', { bubbles: true })); await new Promise(r => setTimeout(r, 1800)); return { placeholder: inp.placeholder, afterSet: opts, afterClear: JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options)) };`);
  p.textFieldFile = bytes('.obsidian/canvas.json');
  p.slider = await c.evaluate(`const el = app.setting.activeTab.containerEl; const sl = el.querySelector('input[type=range]'); if (!sl) return null; const info = { min: sl.min, max: sl.max, step: sl.step, value: sl.value, tooltip: sl.getAttribute('aria-label') }; const orig = sl.value;
    const results = []; for (const v of [sl.min, sl.max, '0.5', orig]) { sl.value = v; sl.dispatchEvent(new Event('input', { bubbles: true })); sl.dispatchEvent(new Event('change', { bubbles: true })); await new Promise(r => setTimeout(r, 1800)); results.push({ set: v, options: JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options)) }); }
    return { info, results };`);
  p.sliderFile = bytes('.obsidian/canvas.json');
  p.afterDropdowns = Object.fromEntries(obsidianDir().map(f => [f, bytes('.obsidian/' + f)]));
  await c.evaluate(`app.setting.close(); return true;`);
  await sleep(500);
  p.after = obsidianDir();
  p.files = Object.fromEntries(p.after.filter(f => f !== 'workspace.json').map(f => [f, bytes('.obsidian/' + f)]));
  return p;
});

// Viewport: what x, y, zoom mean, checked against the wrapper size; pan and zoom by API; the limits.
await step('06-viewport', async (p) => {
  p.opened = await fresh('Two.canvas');
  p.fit = await view();
  // The canvas point under the wrapper's centre, and under its top-left corner.
  p.centerPoint = await c.evaluate(`${CV} const w = cv.wrapperEl.getBoundingClientRect(); return { center: cv.posFromClient({ clientX: w.x + w.width / 2, clientY: w.y + w.height / 2 }), topLeft: cv.posFromClient({ clientX: w.x, clientY: w.y }), bbox: cv.getViewportBBox(), canvasRect: cv.canvasRect };`);
  p.nodeA = await node('a');
  p.nodeB = await node('b');
  // Set a known viewport and check where node a lands on screen.
  p.set = await setViewport(0, 0, 0);
  p.atZero = { view: await view(), a: await node('a'), under: await c.evaluate(`${CV} const w = cv.wrapperEl.getBoundingClientRect(); return cv.posFromClient({ clientX: w.x + w.width / 2, clientY: w.y + w.height / 2 });`) };
  p.set2 = await setViewport(100, 50, 1);
  p.atOne = { view: await view(), a: await node('a') };
  // Limits: zoom far in and out through the API, then through the wheel later.
  p.maxZoom = await c.evaluate(`${CV} cv.zoomBy(100); cv.finishViewportAnimation = true; await new Promise(r => setTimeout(r, 500)); return { zoom: cv.zoom, tZoom: cv.tZoom, scale: cv.scale };`);
  p.minZoom = await c.evaluate(`${CV} cv.zoomBy(-100); cv.finishViewportAnimation = true; await new Promise(r => setTimeout(r, 500)); return { zoom: cv.zoom, tZoom: cv.tZoom, scale: cv.scale };`);
  p.setBeyond = { hi: await setViewport(0, 0, 10), lo: await setViewport(0, 0, -10) };
  // Grid spacing and snap distance as the canvas reports them, per zoom.
  p.gridVsZoom = [];
  for (const z of [1, 0.5, 0, -0.5, -1, -1.5, -2, -2.5, -3, -3.5, -4]) { await setViewport(0, 0, z); p.gridVsZoom.push(await c.evaluate(`${CV} return { zoom: cv.zoom, scale: cv.scale, gridSpacing: cv.gridSpacing, snapDistance: cv.snapDistance, zoomBreakpoint: cv.zoomBreakpoint };`)); }
  // Zoom to fit on content far wider than the clamp allows: does the fit go below zoom -4?
  p.fitHuge = await c.evaluate(`${CV} const b = cv.nodes.get('b'); const old = { x: b.x, y: b.y }; b.moveTo({ x: 200000, y: 0 }); cv.zoomToFit(); cv.finishViewportAnimation = true; await new Promise(r => setTimeout(r, 600)); const r = { x: cv.x, y: cv.y, zoom: cv.zoom, tZoom: cv.tZoom, scale: cv.scale }; b.moveTo(old); return r;`);
  // Zoom to fit: the bbox vs the visible rect gives the padding.
  p.zoomToFit = await c.evaluate(`${CV} cv.zoomToFit(); cv.finishViewportAnimation = true; await new Promise(r => setTimeout(r, 600));
    const nodes = Array.from(cv.nodes.values()); const minX = Math.min(...nodes.map(n => n.x)), minY = Math.min(...nodes.map(n => n.y)), maxX = Math.max(...nodes.map(n => n.x + n.width)), maxY = Math.max(...nodes.map(n => n.y + n.height));
    return { x: cv.x, y: cv.y, zoom: cv.zoom, scale: cv.scale, content: { minX, minY, maxX, maxY, w: maxX - minX, h: maxY - minY }, visible: cv.getViewportBBox(), wrapper: { w: cv.canvasRect.width, h: cv.canvasRect.height } };`);
  p.zoomToSelection = await c.evaluate(`${CV} cv.selectOnly(cv.nodes.get('a')); cv.zoomToSelection(); cv.finishViewportAnimation = true; await new Promise(r => setTimeout(r, 600));
    return { x: cv.x, y: cv.y, zoom: cv.zoom, scale: cv.scale, visible: cv.getViewportBBox(), a: cv.nodes.get('a').getBBox ? cv.nodes.get('a').getBBox() : null };`);
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  return p;
});

// Wheel and drag: what each gesture does, and the step per notch.
await step('07-wheel-drag', async (p) => {
  await fresh('Two.canvas');
  await setViewport(400, 50, 0);
  const v0 = await view(); const cx = v0.wrapper.cx, cy = v0.wrapper.cy;
  const after = async (label, fn) => { const before = await view(); await fn(); const now = await view(); p[label] = { before: { x: before.x, y: before.y, zoom: before.zoom }, after: { x: now.x, y: now.y, zoom: now.zoom, tZoom: now.tZoom, scale: now.scale } }; await setViewport(400, 50, before.zoom); };
  await after('wheelDown100', () => wheel(cx, cy, 100));
  await after('wheelUp100', () => wheel(cx, cy, -100));
  await after('wheelDown120', () => wheel(cx, cy, 120));
  await after('wheelRight100', () => wheel(cx, cy, 0, {}, 100));
  await after('shiftWheelDown100', () => wheel(cx, cy, 100, { shift: true }));
  await after('ctrlWheelDown100', () => wheel(cx, cy, 100, { ctrl: true }));
  await after('ctrlWheelUp100', () => wheel(cx, cy, -100, { ctrl: true }));
  await after('ctrlWheelUp100-offcenter', () => wheel(cx + 200, cy + 100, -100, { ctrl: true }));
  await after('ctrlWheelUp10', () => wheel(cx, cy, -10, { ctrl: true }));
  await after('altWheelDown100', () => wheel(cx, cy, 100, { alt: true }));
  await after('metaWheelDown100', () => wheel(cx, cy, 100, { meta: true }));
  // Pinch: Chromium delivers a trackpad pinch as a ctrl+wheel with a fractional delta; also try the gesture synthesizer.
  await after('pinch', async () => { try { await c.send('Input.synthesizePinchGesture', { x: cx, y: cy, scaleFactor: 2, relativeSpeed: 800 }); await sleep(600); } catch (e) { p.pinchError = String(e); } });
  // Left drag on empty background.
  const spot = { x: v0.wrapper.x + 60, y: v0.wrapper.y + v0.wrapper.h - 150 };
  await after('leftDragEmpty', async () => { p.leftDragMid = null; await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: spot.x, y: spot.y }); await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: spot.x, y: spot.y, button: 'left', clickCount: 1 });
    for (let i = 1; i <= 8; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: spot.x + i * 25, y: spot.y - i * 20, button: 'left' }); await sleep(40); }
    p.leftDragMid = await c.evaluate(`${CV} const sel = document.querySelector('.canvas-selection'); return { selectionBox: sel ? { cls: sel.className, rect: sel.getBoundingClientRect().toJSON(), style: sel.getAttribute('style') } : null, isDragging: cv.isDragging, x: cv.x, y: cv.y, selected: Array.from(cv.selection).map(n => n.id) };`);
    await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: spot.x + 200, y: spot.y - 160, button: 'left', clickCount: 1 }); await sleep(300); });
  await after('middleDragEmpty', () => c.drag(spot.x, spot.y, spot.x + 200, spot.y - 160, { button: 'middle', steps: 8 }));
  await after('rightDragEmpty', () => c.drag(spot.x, spot.y, spot.x + 200, spot.y - 160, { button: 'right', steps: 8 }));
  await after('spaceDragEmpty', async () => { await c.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 }); await sleep(100); p.holdingSpace = await c.evaluate(`${CV} return cv.isHoldingSpace;`);
    await c.drag(spot.x, spot.y, spot.x + 200, spot.y - 160, { steps: 8 }); await c.send('Input.dispatchKeyEvent', { type: 'keyUp', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 }); });
  await after('ctrlDragEmpty', async () => { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: spot.x, y: spot.y, modifiers: 2 }); await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: spot.x, y: spot.y, button: 'left', clickCount: 1, modifiers: 2 });
    for (let i = 1; i <= 8; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: spot.x + i * 25, y: spot.y - i * 20, button: 'left', modifiers: 2 }); await sleep(40); }
    p.ctrlDragMid = await c.evaluate(`${CV} const sel = document.querySelector('.canvas-selection, .canvas-drag-rect, [class*=canvas][class*=drag]'); return sel ? { cls: sel.className, style: sel.getAttribute('style') } : null;`);
    await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: spot.x + 200, y: spot.y - 160, button: 'left', clickCount: 1, modifiers: 2 }); await sleep(500);
    p.ctrlDragMenu = { menu: await c.openMenuItems(), icons: await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(m => ({ title: (m.querySelector('.menu-item-title') || {}).textContent, icon: m.querySelector('svg') && m.querySelector('svg').getAttribute('class') }));`), modal: await modal(), nodes: (await nodeIds()).length };
    p.shotCtrlDragMenu = shot('ctrl-drag-menu');
    await c.pressKey('Escape'); await sleep(200); });
  // Marquee: full containment or intersection? Drag a box that clips node a (x 0..200, y 0..100) only partly, at a zoom where both cards are inside the wrapper.
  await setViewport(400, 50, -0.5);
  const a = await node('a');
  await after('marqueePartial', async () => { await c.drag(a.screen.x - 30, a.screen.y - 30, a.screen.x + 60, a.screen.y + 60, { steps: 8 }); p.marqueePartialSelected = (await view()).selection; });
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  await after('marqueeFull', async () => { await c.drag(a.screen.x - 30, a.screen.y - 30, a.screen.x + a.screen.w + 30, a.screen.y + a.screen.h + 30, { steps: 8 }); p.marqueeFullSelected = (await view()).selection; });
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  // A marquee that only touches a corner of a card.
  await after('marqueeCorner', async () => { await c.drag(a.screen.x - 40, a.screen.y - 40, a.screen.x + 10, a.screen.y + 10, { steps: 8 }); p.marqueeCornerSelected = (await view()).selection; });
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  // Shift-drag marquee adds? Click a, then shift-marquee b.
  const b = await node('b');
  await c.clickAt(a.screen.cx, a.screen.y + 8);
  await after('shiftMarqueeAdds', async () => { const m = 8; await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: b.screen.x - 30, y: b.screen.y - 30, modifiers: m }); await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: b.screen.x - 30, y: b.screen.y - 30, button: 'left', clickCount: 1, modifiers: m });
    for (let i = 1; i <= 8; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: b.screen.x - 30 + i * 30, y: b.screen.y - 30 + i * 20, button: 'left', modifiers: m }); await sleep(40); }
    await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: b.screen.x + 210, y: b.screen.y + 130, button: 'left', clickCount: 1, modifiers: m }); await sleep(300); p.shiftMarqueeSelected = (await view()).selection; });
  return p;
});

// Controls and the card menu: every button's tooltip and icon.
await step('08-controls', async (p) => {
  await fresh('Two.canvas');
  p.controls = await ariaList('.canvas-control-item');
  p.controlGroups = await c.evaluate(`return Array.from(document.querySelectorAll('.canvas-control-group')).map(g => Array.from(g.children).map(el => el.getAttribute('aria-label')));`);
  p.cardMenu = await ariaList('.canvas-card-menu-button');
  p.quickSettings = await c.evaluate(`${CV} return { aria: cv.quickSettingsButton.getAttribute('aria-label'), cls: cv.quickSettingsButton.className };`);
  // Open the quick settings menu and read it.
  await c.evaluate(`${CV} cv.quickSettingsButton.click(); return true;`); await sleep(400);
  p.quickSettingsMenu = await c.openMenuItems();
  p.quickSettingsMenuChecked = await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(m => ({ title: (m.querySelector('.menu-item-title') || {}).textContent, checked: !!m.querySelector('.menu-item-icon.mod-checked, .menu-item-checked, .mod-checked'), cls: m.className, icon: (m.querySelector('svg') || {}).getAttribute ? m.querySelector('svg').getAttribute('class') : null }));`);
  await c.pressKey('Escape'); await sleep(200);
  // Zoom controls: what one click changes.
  await setViewport(400, 50, 0);
  const items = await ariaList('.canvas-control-item');
  for (let i = 0; i < items.length; i++) {
    const before = await view();
    await c.clickElement('.canvas-control-item', { index: i }); await sleep(600);
    const now = await view();
    p['click' + i] = { label: items[i].label, before: { x: before.x, y: before.y, zoom: before.zoom }, after: { x: now.x, y: now.y, zoom: now.zoom, tZoom: now.tZoom }, modal: await modal(), menu: await c.openMenuItems() };
    await c.pressKey('Escape'); await sleep(200);
    await setViewport(400, 50, 0);
  }
  // View header actions and the three-dot menu.
  p.viewActions = await ariaList('.workspace-leaf.mod-active .view-actions .view-action, .workspace-leaf.mod-active .view-header .clickable-icon');
  await c.clickElement('.workspace-leaf.mod-active .view-header .view-actions .view-action[aria-label="More options"]'); await sleep(400);
  p.moreOptions = await c.openMenuItems();
  await c.pressKey('Escape'); await sleep(200);
  return p;
});

// Double click on empty space, and the card menu's "add note" and "add media" cards.
await step('09-create', async (p) => {
  await fresh('Two.canvas');
  await setViewport(400, 50, 0);
  const v = await view(); const spot = { x: v.wrapper.x + 100, y: v.wrapper.y + v.wrapper.h - 150 };
  const before = await nodeIds();
  await c.clickAt(spot.x, spot.y, { clickCount: 2 }); await sleep(800);
  const ids = (await nodeIds()).filter(i => !before.includes(i));
  p.dblClick = { spotCanvas: await c.evaluate(`${CV} return cv.posFromClient({ clientX: ${spot.x}, clientY: ${spot.y} });`), created: ids, node: ids[0] ? await node(ids[0]) : null,
    editor: await c.evaluate(`return { cm: !!document.querySelector('.canvas-node .cm-editor'), focused: document.activeElement && document.activeElement.className, editing: document.querySelector('.canvas-node.is-editing') ? document.querySelector('.canvas-node.is-editing').className : null };`) };
  await c.typeText('typed'); await sleep(300);
  p.typed = ids[0] ? await node(ids[0]) : null;
  await c.pressKey('Escape'); await sleep(400);
  p.afterEscape = { node: ids[0] ? await node(ids[0]) : null, selection: (await view()).selection, editor: await c.evaluate(`return { cm: !!document.querySelector('.canvas-node .cm-editor'), focused: document.activeElement && document.activeElement.className };`) };
  // Double click, type nothing, Escape: is the empty card kept?
  const before2 = await nodeIds();
  await c.clickAt(spot.x + 300, spot.y, { clickCount: 2 }); await sleep(600);
  const ids2 = (await nodeIds()).filter(i => !before2.includes(i));
  await c.pressKey('Escape'); await sleep(500);
  p.emptyAfterEscape = { created: ids2, stillThere: (await nodeIds()).filter(i => ids2.includes(i)), edgesNodes: await c.evaluate(`${CV} return cv.getData();`) };
  // Click outside while editing.
  const before3 = await nodeIds();
  await c.clickAt(spot.x + 150, spot.y - 150, { clickCount: 2 }); await sleep(600);
  const ids3 = (await nodeIds()).filter(i => !before3.includes(i));
  await c.typeText('abc');
  await c.clickAt(spot.x + 500, spot.y - 300); await sleep(500);
  p.clickOutside = { created: ids3, node: ids3[0] ? await node(ids3[0]) : null, selection: (await view()).selection };
  // Card menu: add note from vault, through its modal.
  const before4 = await nodeIds();
  await c.clickElement('.canvas-card-menu-button', { index: 1 }); await sleep(600);
  p.noteModal = await modal();
  await c.typeText('Plan'); await sleep(500);
  p.noteModalSuggestions = await c.evaluate(`return Array.from(document.querySelectorAll('.suggestion-item')).map(e => e.textContent.trim()).slice(0, 10);`);
  await c.pressKey('Enter'); await sleep(1000);
  const ids4 = (await nodeIds()).filter(i => !before4.includes(i));
  p.noteCard = { created: ids4, node: ids4[0] ? await node(ids4[0]) : null, view: await view(), label: await c.evaluate(`return Array.from(document.querySelectorAll('.canvas-node-label')).map(l => ({ text: l.textContent, rect: l.getBoundingClientRect().toJSON(), cs: (() => { const s = getComputedStyle(l); return { fontSize: s.fontSize, color: s.color, top: s.top, left: s.left, transform: s.transform, display: s.display, opacity: s.opacity, lineHeight: s.lineHeight, fontWeight: s.fontWeight }; })() }));`) };
  await c.pressKey('Escape'); await sleep(200);
  // Add media from vault.
  const before5 = await nodeIds();
  await c.clickElement('.canvas-card-menu-button', { index: 2 }); await sleep(600);
  p.mediaModal = await modal();
  await c.typeText('pic'); await sleep(500);
  await c.pressKey('Enter'); await sleep(1500);
  const ids5 = (await nodeIds()).filter(i => !before5.includes(i));
  p.mediaCard = { created: ids5, node: ids5[0] ? await node(ids5[0]) : null };
  await c.pressKey('Escape'); await sleep(200);
  // Drag the "add card" button from the card menu onto the canvas.
  const before6 = await nodeIds();
  const r = await c.rect('.canvas-card-menu-button', 0);
  await c.drag(r.cx, r.cy, spot.x + 50, spot.y - 350, { steps: 10 }); await sleep(800);
  const ids6 = (await nodeIds()).filter(i => !before6.includes(i));
  p.dragCard = { dropCanvas: await c.evaluate(`${CV} return cv.posFromClient({ clientX: ${spot.x + 50}, clientY: ${spot.y - 350} });`), created: ids6, node: ids6[0] ? await node(ids6[0]) : null, editing: await c.evaluate(`return !!document.querySelector('.canvas-node .cm-editor');`) };
  await c.pressKey('Escape'); await sleep(200);
  // A click on the add-card button (not a drag).
  const before7 = await nodeIds();
  await c.clickElement('.canvas-card-menu-button', { index: 0 }); await sleep(800);
  const ids7 = (await nodeIds()).filter(i => !before7.includes(i));
  p.clickCard = { created: ids7, node: ids7[0] ? await node(ids7[0]) : null, view: await view() };
  await c.pressKey('Escape'); await sleep(200);
  p.data = await c.evaluate(`${CV} return cv.getData();`);
  return p;
});

// The interaction layer: where the connection dots and resize handles live.
await step('10a-interaction-layer', async (p) => {
  await fresh('Two.canvas'); await setViewport(400, 50, 0);
  const layerDom = () => c.evaluate(`${CV} const L = cv.nodeInteractionLayer; const el = L.interactionEl || L.el || L.containerEl || Object.values(L).find(v => v instanceof HTMLElement);
    const walk = (e, d) => d > 3 ? [] : Array.from(e.children).map(ch => { const r = ch.getBoundingClientRect(); const cs = getComputedStyle(ch); return { tag: ch.tagName, cls: ch.getAttribute('class'), rect: [r.x, r.y, r.width, r.height].map(v => Math.round(v * 100) / 100), w: cs.width, h: cs.height, top: cs.top, left: cs.left, right: cs.right, bottom: cs.bottom, bg: cs.backgroundColor, border: cs.border, radius: cs.borderRadius, transform: cs.transform, opacity: cs.opacity, display: cs.display, cursor: cs.cursor, margin: cs.margin, boxShadow: cs.boxShadow, kids: walk(ch, d + 1) }; });
    return { keys: Object.keys(L), elCls: el && el.className, elStyle: el && el.getAttribute('style'), target: L.target && L.target.id, kids: el ? walk(el, 0) : null };`);
  const a = await node('a');
  await c.mouseMove(a.screen.x + a.screen.w * 0.25, a.screen.y + a.screen.h * 0.5); await sleep(400);
  p.hovered = await layerDom();
  p.nodeRect = a.screen;
  await c.clickAt(a.screen.x + a.screen.w * 0.25, a.screen.y + 8); await sleep(400);
  p.selected = await layerDom();
  // Hover b while a is selected.
  const b = await node('b');
  await c.mouseMove(b.screen.x + b.screen.w * 0.25, b.screen.y + b.screen.h * 0.5); await sleep(400);
  p.selectedAndHoverOther = await layerDom();
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  return p;
});

// Snapping and resize.
await step('10-snap-resize', async (p) => {
  await fresh('Two.canvas');
  await setViewport(300, 100, 0);
  const setSnap = (grid, objects) => c.evaluate(`const o = app.internalPlugins.plugins.canvas.instance.options; o.snapToGrid = ${grid}; o.snapToObjects = ${objects}; return JSON.parse(JSON.stringify(o));`);
  p.snapSet = await setSnap(true, true);
  p.grid = await c.evaluate(`${CV} return { gridSpacing: cv.gridSpacing, snapDistance: cv.snapDistance, options: JSON.parse(JSON.stringify(cv.options)) };`);
  // Drag node a by (37, 23) screen px at scale 1 with grid snapping on.
  const moveBy = async (dx, dy, m = 0) => { const a0 = await node('a'); const g = grab(a0);
    await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: g.x, y: g.y, modifiers: m }); await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: g.x, y: g.y, button: 'left', clickCount: 1, modifiers: m });
    for (let i = 1; i <= 10; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: g.x + dx * i / 10, y: g.y + dy * i / 10, button: 'left', modifiers: m }); await sleep(40); }
    const mid = await c.evaluate(`${CV} return { snapGuides: document.querySelectorAll('.canvas-snap-guide, .canvas-snap-point, [class*=snap-line], [class*=snap-guide]').length, dragging: cv.isDragging, wrapperCls: cv.wrapperEl.className }; `);
    await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: g.x + dx, y: g.y + dy, button: 'left', clickCount: 1, modifiers: m }); await sleep(300);
    const a1 = await node('a'); return { from: { x: a0.x, y: a0.y }, to: { x: a1.x, y: a1.y }, moved: { dx: a1.x - a0.x, dy: a1.y - a0.y }, mid }; };
  p.dragSnapOn = await moveBy(37, 23);
  p.dragSnapOnAgain = await moveBy(-37, -23);
  p.dragAlt = await moveBy(37, 23, 1);
  p.dragAltBack = await moveBy(-37, -23, 1);
  p.dragShift = await moveBy(37, 23, 8);
  await c.evaluate(`${CV} cv.nodes.get('a').moveTo({ x: 0, y: 0 }); return true;`);
  p.dragCtrl = await moveBy(37, 23, 2);
  p.dragCtrlResult = { ids: await nodeIds(), data: await c.evaluate(`${CV} return cv.getData().nodes.map(n => ({ id: n.id, x: n.x, y: n.y, text: n.text }));`) };
  await fresh('Two.canvas'); await setViewport(300, 100, 0);
  // Grid snapping off.
  await c.evaluate(`${CV} cv.toggleGridSnapping(); return true;`); await sleep(2500);
  p.optionsAfterToggle = await c.evaluate(`${CV} return { opts: JSON.parse(JSON.stringify(cv.options)), plugin: JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options)) };`);
  p.settingsFileAfterToggle = bytes('.obsidian/canvas.json');
  p.dragSnapOff = await moveBy(37, 23);
  // Object snapping: with grid snapping off, bring a's left edge within 7 units of b's left edge (x 600) and its top 150 below.
  const a = await node('a'); const b = await node('b');
  p.dragObjectSnap = { aimed: { x: b.x + 7, y: a.y + 150 }, ...(await moveBy((b.x - a.x) + 7, 150)) };
  // Both off: does a drag snap at all?
  await setSnap(false, false); await fresh('Two.canvas'); await setViewport(300, 100, 0);
  p.dragBothOff = await moveBy(37, 23);
  // Object snap only: top edge alignment with b (y 0) when dropped 6 units off.
  await setSnap(false, true); await fresh('Two.canvas'); await setViewport(300, 100, 0);
  p.dragObjectSnapY = { aimed: { x: 150, y: 6 }, ...(await moveBy(150, 6)) };
  await fresh('Two.canvas'); await setViewport(300, 100, 0);
  p.dragObjectSnapFar = { aimed: { x: 150, y: 40 }, ...(await moveBy(150, 40)) };
  // Odd zoom: does a drag produce fractional coordinates?
  await setSnap(true, true); await fresh('Two.canvas'); await setViewport(300, 100, -0.7);
  p.dragOddZoomSnapOn = { scale: (await view()).scale, ...(await moveBy(41, 17)) };
  await setSnap(false, false);
  p.dragOddZoomSnapOff = { scale: (await view()).scale, ...(await moveBy(41, 17)) };
  await setSnap(true, true);
  // Resize through the interaction layer.
  await fresh('Two.canvas'); await setViewport(300, 100, 0);
  let a0 = await node('a'); let g = grab(a0);
  await c.mouseMove(g.x, g.y); await sleep(300); await c.clickAt(g.x, g.y); await sleep(300);
  p.layerSelected = await handles();
  p.selectedDom = await c.evaluate(`${CV} const n = cv.nodes.get('a'); const s = getComputedStyle(n.containerEl); return { cls: n.nodeEl.className, boxShadow: s.boxShadow, border: s.border, outline: s.outline, borderColor: s.borderColor, accent: getComputedStyle(document.body).getPropertyValue('--interactive-accent') };`);
  p.selectedShot = shot('selected-card');
  const resizeWith = async (side, dx, dy, m = 0) => { const before = await node('a'); const gg = grab(before); await c.mouseMove(gg.x, gg.y); await sleep(200); const h = (await handles()).strips.find(st => st.side === side); if (!h) return { error: 'no handle ' + side };
    // Side strips carry the connection dot at their midpoint: press a quarter of the way along instead.
    const px = h.rect[2] > h.rect[3] ? h.rect[0] + h.rect[2] * 0.25 : h.cx, py = h.rect[3] > h.rect[2] ? h.rect[1] + h.rect[3] * 0.25 : h.cy;
    await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px, y: py, modifiers: m }); await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: px, y: py, button: 'left', clickCount: 1, modifiers: m });
    for (let i = 1; i <= 10; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px + dx * i / 10, y: py + dy * i / 10, button: 'left', modifiers: m }); await sleep(40); }
    await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: px + dx, y: py + dy, button: 'left', clickCount: 1, modifiers: m }); await sleep(300);
    const after = await node('a'); return { handle: h.cls, cursor: h.cursor, before: { x: before.x, y: before.y, w: before.width, h: before.height }, after: { x: after.x, y: after.y, w: after.width, h: after.height } }; };
  // The connection dot's look while the pointer is over the card's edge zone, and over the dot itself.
  { const h = p.layerSelected.strips.find(st => st.side === 'right'); await c.mouseMove(h.rect[0] + h.rect[2] / 2, h.rect[1] + h.rect[3] * 0.25); await sleep(300); p.dotHoverStrip = (await handles()).strips.find(st => st.side === 'right').dot; await c.mouseMove(h.dot.cx, h.dot.cy); await sleep(300); p.dotHoverDot = (await handles()).strips.find(st => st.side === 'right').dot; const gg = grab(await node('a')); await c.mouseMove(gg.x, gg.y); await sleep(300); p.dotHoverBody = (await handles()).strips.find(st => st.side === 'right').dot; p.shotHoverDots = shot('hover-connection-dots'); }
  p.resizeBR = await resizeWith('bottom-right', 33, 27);
  p.resizeRight = await resizeWith('right', 33, 0);
  p.resizeTop = await resizeWith('top', 0, -27);
  p.resizeMin = await resizeWith('bottom-right', -400, -400);
  p.resizeTL = await resizeWith('top-left', -150, -50);
  p.resizeAlt = await resizeWith('bottom-right', 33, 27, 1);
  p.resizeShift = await resizeWith('bottom-right', 33, 27, 8);
  // Handle thickness at another zoom: canvas units or screen pixels?
  await setViewport(300, 100, -1);
  a0 = await node('a'); g = grab(a0); await c.mouseMove(g.x, g.y); await sleep(300);
  p.layerAtHalf = await handles();
  await setSnap(false, false);
  await setViewport(300, 100, -0.7);
  p.resizeOddZoomNoSnap = { scale: (await view()).scale, ...(await resizeWith('top-left', -23, -11)) };
  await setSnap(true, true);
  await setViewport(300, 100, 0);
  // Double click on a handle: a card with a paragraph of text, so the fit is visible.
  await c.evaluate(`${CV} const n = cv.nodes.get('a'); n.setText ? n.setText('A paragraph of text long enough to wrap onto several lines inside a two hundred unit card.') : (n.text = 'A paragraph of text long enough to wrap onto several lines inside a two hundred unit card.', n.render()); n.moveAndResize({ x: 0, y: 0, width: 200, height: 300 }); return true;`); await sleep(400);
  a0 = await node('a'); g = grab(a0); await c.mouseMove(g.x, g.y); await sleep(200);
  const hb = (await handles()).strips.find(st => st.side === 'bottom');
  if (hb) { await c.clickAt(hb.cx, hb.cy, { clickCount: 2 }); await sleep(500); p.resizeDblClickBottom = { before: { w: a0.width, h: a0.height }, after: (({ x, y, width, height }) => ({ x, y, width, height }))(await node('a')) }; }
  a0 = await node('a'); g = grab(a0); await c.mouseMove(g.x, g.y); await sleep(200);
  const hr = (await handles()).strips.find(st => st.side === 'right');
  if (hr) { await c.clickAt(hr.cx, hr.cy, { clickCount: 2 }); await sleep(500); p.resizeDblClickRight = { before: { w: a0.width, h: a0.height }, after: (({ x, y, width, height }) => ({ x, y, width, height }))(await node('a')) }; }
  p.data = await c.evaluate(`${CV} return cv.getData();`);
  return p;
});

// Zoom threshold: where mod-zoomed-out flips, and what a card shows below it.
await step('11-threshold', async (p) => {
  await fresh('Fixture.canvas');
  p.flip = [];
  for (const z of [0, -0.5, -0.9, -1, -1.1, -1.2, -1.3, -1.4, -1.5, -1.6, -1.7, -1.8, -2, -2.5, -3, -3.5, -4]) {
    const r = await setViewport(700, 350, z);
    p.flip.push({ zoom: z, got: r.zoom, scale: r.scale, zoomedOut: await c.evaluate(`${CV} return cv.wrapperEl.classList.contains('mod-zoomed-out');`) });
  }
  p.breakpointValue = await c.evaluate(`${CV} return { zoomBreakpoint: cv.zoomBreakpoint, option: app.internalPlugins.plugins.canvas.instance.options.zoomBreakpoint };`);
  // Move the setting and read where the flip goes.
  p.flipWithSetting = [];
  for (const setting of [0.5, -0.7, 0]) {
    await c.evaluate(`app.internalPlugins.plugins.canvas.instance.options.zoomBreakpoint = ${setting}; return true;`);
    await fresh('Fixture.canvas');
    const row = { setting, zoomBreakpoint: await c.evaluate(`${CV} return cv.zoomBreakpoint;`), flips: [] };
    for (const z of [-0.5, -0.9, -1.0, -1.1, -1.2, -1.3, -1.6, -1.7, -1.8, -2.1, -2.2, -2.3, -2.4, -2.5]) { await setViewport(700, 350, z); row.flips.push({ zoom: z, ...(await c.evaluate(`${CV} const n = cv.nodes.get('t0'); return { zoomedOut: cv.wrapperEl.classList.contains('mod-zoomed-out'), mounted: n.isContentMounted, placeholder: !!n.nodeEl.querySelector('.canvas-node-placeholder'), labelDisplay: getComputedStyle(cv.nodes.get('f1').nodeEl.querySelector('.canvas-node-label')).display };`)) }); }
    p.flipWithSetting.push(row);
  }
  // Card label, group label and border scaling across zooms.
  p.scaling = [];
  for (const z of [1, 0.5, 0, -0.5, -1, -1.5]) { await setViewport(700, 350, z); p.scaling.push({ zoom: z, ...(await c.evaluate(`${CV} const f = cv.nodes.get('f1'); const l = f.nodeEl.querySelector('.canvas-node-label'); const g = cv.nodes.get('g2').nodeEl.querySelector('.canvas-group-label'); const t = cv.nodes.get('t1'); const sc = getComputedStyle(t.containerEl); const sl = getComputedStyle(l); const sg = getComputedStyle(g);
      return { scale: cv.scale, label: { display: sl.display, fontSize: sl.fontSize, transform: sl.transform, top: sl.top, screenH: l.getBoundingClientRect().height }, group: { fontSize: sg.fontSize, transform: sg.transform, top: sg.top, padding: sg.padding, screenH: g.getBoundingClientRect().height }, border: { width: sc.borderTopWidth, screen: parseFloat(sc.borderTopWidth) * cv.scale, radius: sc.borderRadius, boxShadow: sc.boxShadow }, zoomMultiplier: cv.wrapperEl.style.getPropertyValue('--zoom-multiplier') || getComputedStyle(cv.wrapperEl).getPropertyValue('--zoom-multiplier') };`)) }); }
  await setViewport(700, 350, -2);
  p.zoomedOutDom = await c.evaluate(`${CV} const n = cv.nodes.get('t0'); const f = cv.nodes.get('f1'); const walk = (el, d) => d > 4 ? [] : Array.from(el.children).slice(0, 8).map(ch => ({ tag: ch.tagName, cls: String(ch.className), text: ch.children.length ? undefined : ch.textContent.slice(0, 80), kids: walk(ch, d + 1) }));
    return { text: walk(n.nodeEl, 0), file: walk(f.nodeEl, 0), isContentMounted: n.isContentMounted, placeholder: n.placeholderEl ? { cls: n.placeholderEl.className, text: n.placeholderEl.textContent, cs: (() => { const s = getComputedStyle(n.placeholderEl); return { fontSize: s.fontSize, fontWeight: s.fontWeight, color: s.color, display: s.display, padding: s.padding, textAlign: s.textAlign, alignItems: s.alignItems, justifyContent: s.justifyContent, lineHeight: s.lineHeight, overflow: s.overflow }; })() } : null,
      filePlaceholder: f.placeholderEl ? { text: f.placeholderEl.textContent, cls: f.placeholderEl.className } : null, labelsVisible: Array.from(document.querySelectorAll('.canvas-node-label')).map(l => ({ text: l.textContent, display: getComputedStyle(l).display, rect: l.getBoundingClientRect().toJSON() })), groupLabels: Array.from(document.querySelectorAll('.canvas-group-label')).map(l => ({ text: l.textContent, fontSize: getComputedStyle(l).fontSize, transform: getComputedStyle(l).transform, rect: l.getBoundingClientRect().toJSON() })) };`);
  p.shotZoomedOut = shot('zoomed-out-dark');
  await setViewport(700, 350, -1);
  p.atMinus1 = await c.evaluate(`${CV} const n = cv.nodes.get('t0'); return { isContentMounted: n.isContentMounted, cls: n.nodeEl.className, placeholderDisplay: n.placeholderEl ? getComputedStyle(n.placeholderEl).display : null, contentDisplay: getComputedStyle(n.contentEl).display, labels: Array.from(document.querySelectorAll('.canvas-node-label')).map(l => ({ text: l.textContent, display: getComputedStyle(l).display, fontSize: getComputedStyle(l).fontSize, transform: getComputedStyle(l).transform, rect: l.getBoundingClientRect().toJSON() })) };`);
  p.shotFitDark = (await c.evaluate(`${CV} cv.zoomToFit(); cv.finishViewportAnimation = true; await new Promise(r => setTimeout(r, 600)); return cv.zoom;`), shot('fixture-fit-dark'));
  await c.evaluate(`app.changeTheme('moonstone'); await new Promise(r => setTimeout(r, 500)); return true;`);
  p.shotFitLight = shot('fixture-fit-light');
  await setViewport(700, 350, -2);
  p.shotZoomedOutLight = shot('zoomed-out-light');
  await c.evaluate(`app.changeTheme('obsidian'); return true;`);
  // Card styling at scale 1 for the default and each colour.
  await setViewport(700, 100, 0);
  p.cardStyles = await c.evaluate(`${CV} const r = {}; for (const id of ['t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1','g1','g2','g3']) { const n = cv.nodes.get(id); if (!n) continue; const s = getComputedStyle(n.containerEl); const sc = getComputedStyle(n.contentEl); const sn = getComputedStyle(n.nodeEl);
      r[id] = { cls: n.nodeEl.className, color: n.color, nodeVar: sn.getPropertyValue('--canvas-color'), containerVar: s.getPropertyValue('--canvas-color'), border: s.borderTopWidth + ' ' + s.borderTopStyle + ' ' + s.borderTopColor, radius: s.borderRadius, boxShadow: s.boxShadow, bg: s.backgroundColor, contentBg: sc.backgroundColor, contentPadding: sc.padding, contentFont: sc.fontSize, contentColor: sc.color, outline: s.outline, nodeBg: sn.backgroundColor, bgImage: s.backgroundImage, bgSize: s.backgroundSize, bgRepeat: s.backgroundRepeat, bgPos: s.backgroundPosition, contentBgImage: sc.backgroundImage }; }
    return r;`);
  p.labelsAtScale1 = await c.evaluate(`${CV} return Array.from(document.querySelectorAll('.canvas-node-label, .canvas-group-label')).map(l => { const s = getComputedStyle(l); const r = l.getBoundingClientRect(); const parent = l.parentElement.getBoundingClientRect(); return { cls: l.className, text: l.textContent, fontSize: s.fontSize, lineHeight: s.lineHeight, fontWeight: s.fontWeight, color: s.color, bg: s.backgroundColor, padding: s.padding, radius: s.borderRadius, top: s.top, left: s.left, transform: s.transform, transformOrigin: s.transformOrigin, display: s.display, opacity: s.opacity, whiteSpace: s.whiteSpace, maxWidth: s.maxWidth, textOverflow: s.textOverflow, rect: [r.x, r.y, r.width, r.height], offsetFromParent: { dx: r.x - parent.x, dy: r.y - parent.y, bottomToParentTop: parent.y - (r.y + r.height) } }; });`);
  // Hover state.
  const t1 = await node('t1');
  await c.mouseMove(t1.screen.cx, t1.screen.cy); await sleep(400);
  p.hover = await c.evaluate(`${CV} const n = cv.nodes.get('t1'); const s = getComputedStyle(n.containerEl); return { cls: n.nodeEl.className, boxShadow: s.boxShadow, border: s.borderColor, connectionPoints: Array.from(n.nodeEl.querySelectorAll('.canvas-node-connection-point, [class*=connection]')).map(e => { const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); return { cls: e.className, rect: [r.x, r.y, r.width, r.height], w: cs.width, h: cs.height, bg: cs.backgroundColor, border: cs.border, radius: cs.borderRadius, top: cs.top, left: cs.left, right: cs.right, bottom: cs.bottom, transform: cs.transform, opacity: cs.opacity, display: cs.display, cursor: cs.cursor }; }), nodeRect: n.nodeEl.getBoundingClientRect().toJSON() };`);
  p.shotHover = shot('hover-card');
  await c.clickAt(t1.screen.cx, t1.screen.y + 6); await sleep(400);
  p.selectedColored = await c.evaluate(`${CV} const n = cv.nodes.get('t1'); const s = getComputedStyle(n.containerEl); return { cls: n.nodeEl.className, boxShadow: s.boxShadow, border: s.borderColor, outline: s.outline };`);
  p.shotSelectedMenu = shot('selected-card-menu');
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  return p;
});

// Edges: the Bézier rule, the arrow head, the label, the hit area, selection.
await step('12-edges', async (p) => {
  await fresh('Two.canvas');
  await setViewport(400, 50, 0);
  p.edgeDom = await c.evaluate(`${CV} const e = cv.edges.get('ab'); const cls = (el) => (el && el.getAttribute && el.getAttribute('class')) || ''; const dump = (el) => el && el.tagName ? { tag: el.tagName, cls: cls(el), attrs: Object.fromEntries(Array.from(el.attributes).map(a => [a.name, a.value])), children: Array.from(el.children).map(ch => ({ tag: ch.tagName, cls: cls(ch), attrs: Object.fromEntries(Array.from(ch.attributes).map(a => [a.name, a.value])) })) } : (el ? String(el).slice(0, 80) : null);
    return { lineGroup: dump(e.lineGroupEl), lineEndGroup: dump(e.lineEndGroupEl), path: Object.keys(e.path), pathEls: Object.fromEntries(Object.entries(e.path).map(([k, v]) => [k, v && v.tagName ? dump(v) : String(v)])), bezier: e.bezier, from: { side: e.from.side, end: e.from.end, node: e.from.node && e.from.node.id }, to: { side: e.to.side, end: e.to.end, node: e.to.node && e.to.node.id }, toLineEnd: e.toLineEnd ? { keys: Object.keys(e.toLineEnd), el: dump(e.toLineEnd.el || e.toLineEnd.polygon || e.toLineEnd.lineEndEl) } : null, endGroupChildren: Array.from(e.lineEndGroupEl.children).map(dump), label: e.labelElement ? Object.keys(e.labelElement) : null,
      display: (() => { const s = getComputedStyle(e.lineGroupEl.querySelector('.canvas-display-path')); return { stroke: s.stroke, strokeWidth: s.strokeWidth, fill: s.fill, linecap: s.strokeLinecap }; })(), interaction: (() => { const s = getComputedStyle(e.lineGroupEl.querySelector('.canvas-interaction-path')); return { stroke: s.stroke, strokeWidth: s.strokeWidth, pointerEvents: s.pointerEvents }; })(),
      endStyle: (() => { const el = e.lineEndGroupEl.querySelector('path, polygon, polyline'); if (!el) return null; const s = getComputedStyle(el); return { tag: el.tagName, fill: s.fill, stroke: s.stroke, strokeWidth: s.strokeWidth, transform: el.getAttribute('transform') || s.transform, d: el.getAttribute('d') || el.getAttribute('points'), bbox: el.getBBox && (({ x, y, width, height }) => ({ x, y, width, height }))(el.getBBox()) }; })() };`);
  // Stroke width vs zoom: the computed stroke at several scales.
  p.strokeVsZoom = [];
  for (const z of [1, 0.5, 0, -0.5, -1, -1.5, -2]) { await setViewport(400, 50, z); p.strokeVsZoom.push({ zoom: z, ...(await c.evaluate(`${CV} const e = cv.edges.get('ab'); const s = getComputedStyle(e.lineGroupEl.querySelector('.canvas-display-path')); const i = getComputedStyle(e.lineGroupEl.querySelector('.canvas-interaction-path')); const end = e.lineEndGroupEl.querySelector('path, polygon, polyline'); return { scale: cv.scale, display: s.strokeWidth, interaction: i.strokeWidth, endAttrs: end ? Object.fromEntries(Array.from(end.attributes).map(a => [a.name, a.value])) : null, endBox: end && end.getBBox ? (({ x, y, width, height }) => ({ x, y, width, height }))(end.getBBox()) : null, edgesSvgStyle: cv.edgeContainerEl.getAttribute('style'), edgesSvgAttrs: Object.fromEntries(Array.from(cv.edgeContainerEl.attributes).map(a => [a.name, a.value])), canvasStyle: cv.canvasEl.getAttribute('style') };`)) }); }
  await setViewport(400, 50, 0);
  // Bézier table: move b around a (a stays at 0,0 200x100, anchors right/left) and read the path.
  p.bezier = [];
  const place = async (bx, by, fromSide, toSide) => c.evaluate(`${CV} const b = cv.nodes.get('b'); b.moveTo({ x: ${bx}, y: ${by} }); const e = cv.edges.get('ab'); const d = e.getData(); if (d.fromSide !== ${JSON.stringify(fromSide)} || d.toSide !== ${JSON.stringify(toSide)}) { d.fromSide = ${JSON.stringify(fromSide)}; d.toSide = ${JSON.stringify(toSide)}; e.setData(d); e.render(); } cv.requestFrame(); await new Promise(r => setTimeout(r, 300));
    return { b: { x: ${bx}, y: ${by} }, fromSide: e.from.side, toSide: e.to.side, d: e.lineGroupEl.querySelector('.canvas-display-path').getAttribute('d'), bezier: JSON.parse(JSON.stringify(e.bezier)), end: (() => { const el = e.lineEndGroupEl.querySelector('path, polygon, polyline'); return el ? Object.fromEntries(Array.from(el.attributes).map(a => [a.name, a.value])) : null; })(), labelPos: e.labelElement && e.labelElement.wrapperEl ? e.labelElement.wrapperEl.getAttribute('style') : null };`);
  for (const [bx, by] of [[600, 0], [400, 0], [300, 0], [250, 0], [220, 0], [1000, 0], [2000, 0], [600, 300], [600, -300], [600, 600], [300, 400], [-400, 0], [0, 300], [0, 600], [0, 150], [1500, 1500]]) p.bezier.push(await place(bx, by, 'right', 'left'));
  for (const [bx, by, fs, ts] of [[0, 300, 'bottom', 'top'], [0, 600, 'bottom', 'top'], [300, 300, 'bottom', 'top'], [600, 0, 'right', 'right'], [600, 0, 'left', 'right'], [600, 300, 'bottom', 'left'], [600, 300, 'right', 'top'], [600, 0, 'top', 'top'], [0, 300, 'bottom', 'bottom']]) p.bezier.push(await place(bx, by, fs, ts));
  await place(600, 0, 'right', 'left');
  // Label: set one and measure the pill at scale 1 and 0.5.
  p.label = [];
  for (const z of [0, -1, 1]) { await setViewport(400, 50, z); p.label.push(await c.evaluate(`${CV} const e = cv.edges.get('ab'); if (e.label !== 'a label') { e.setLabel('a label'); e.render(); } cv.requestFrame(); await new Promise(r => setTimeout(r, 300)); const le = e.labelElement; const el = le.wrapperEl || le.el || le.containerEl || le.labelEl; const inner = el.querySelector('*') || el; const s = getComputedStyle(el); const si = getComputedStyle(inner); const r = el.getBoundingClientRect();
    return { zoom: ${z}, scale: cv.scale, keys: Object.keys(le), cls: el.className, innerCls: inner.className, text: el.textContent, rect: [r.x, r.y, r.width, r.height], style: el.getAttribute('style'), fontSize: s.fontSize, innerFontSize: si.fontSize, padding: si.padding, bg: si.backgroundColor, color: si.color, radius: si.borderRadius, lineHeight: si.lineHeight, fontWeight: si.fontWeight, transform: s.transform, innerTransform: si.transform, center: e.getCenter(), bezierCenter: e.bezier && e.bezier.center, parent: el.parentElement.className }; }`)); }
  await setViewport(400, 50, 0);
  p.shotEdgeLabel = shot('edge-label');
  // Select the edge by clicking its middle.
  const mid = await c.evaluate(`${CV} const e = cv.edges.get('ab'); const cpt = e.getCenter(); return cv.posInViewport ? null : null;`);
  const midScreen = await c.evaluate(`${CV} const e = cv.edges.get('ab'); const cpt = e.getCenter(); const w = cv.wrapperEl.getBoundingClientRect(); const rect = cv.canvasRect; return { canvas: cpt, screen: { x: w.x + (cpt.x - cv.x) * cv.scale + w.width / 2, y: w.y + (cpt.y - cv.y) * cv.scale + w.height / 2 } };`);
  await c.clickAt(midScreen.screen.x + 120, midScreen.screen.y); await sleep(400); // on the straight middle of the line, away from the label
  p.edgeSelected = { clicked: midScreen, selection: await c.evaluate(`${CV} return { sel: Array.from(cv.selection).map(s => s.id), edgeCls: cv.edges.get('ab').lineGroupEl.getAttribute('class'), display: (() => { const s = getComputedStyle(cv.edges.get('ab').lineGroupEl.querySelector('.canvas-display-path')); return { stroke: s.stroke, strokeWidth: s.strokeWidth }; })(), focused: !!document.querySelector('.canvas-edge.is-focused, .is-focused'), menu: Array.from(document.querySelectorAll('.canvas-menu .clickable-icon, .canvas-menu-container .clickable-icon')).map(e => e.getAttribute('aria-label')) }; `) };
  p.shotEdgeSelected = shot('edge-selected');
  await c.rightClickElement('.canvas-menu .clickable-icon'); await sleep(100); await c.pressKey('Escape');
  // Right-click the edge.
  await c.clickAt(midScreen.screen.x + 120, midScreen.screen.y, { button: 'right' }); await sleep(400);
  p.edgeContextMenu = { items: await c.openMenuItems(), icons: await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(m => ({ title: (m.querySelector('.menu-item-title') || {}).textContent, icon: m.querySelector('svg') && m.querySelector('svg').getAttribute('class') }));`) };
  p.shotEdgeContext = shot('edge-context-menu');
  await c.pressKey('Escape'); await sleep(200);
  await c.evaluate(`${CV} cv.deselectAll(); cv.edges.get('ab').setLabel(''); return true;`);
  // Hit area: click at increasing distances from the straight part of the line.
  p.hit = [];
  for (const off of [2, 6, 10, 14, 18, 22, 26, 30]) { await c.evaluate(`${CV} cv.deselectAll(); return true;`); await c.clickAt(midScreen.screen.x + 150, midScreen.screen.y - off); await sleep(200); p.hit.push({ off, sel: (await view()).selection, hoverCls: await c.evaluate(`${CV} return cv.edges.get('ab').lineGroupEl.getAttribute('class');`) }); }
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  // Draw an edge: hover a, drag from its right connection dot to the middle of b.
  const a = await node('a'); const ga = grab(a);
  await c.mouseMove(ga.x, ga.y); await sleep(400);
  p.connectionPoints = await handles();
  const right = p.connectionPoints.strips.find(st => st.side === 'right');
  const b = await node('b'); const gb = grab(b);
  const edgesBefore = await edgesData();
  const drawFrom = async (st, tx, ty) => { await c.mouseMove(ga.x, ga.y); await sleep(300); const h = (await handles()).strips.find(x => x.side === st); const sx = h.dot.cx, sy = h.dot.cy;
    await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: sx, y: sy }); await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: sx, y: sy, button: 'left', clickCount: 1 });
    for (let i = 1; i <= 14; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: sx + (tx - sx) * i / 14, y: sy + (ty - sy) * i / 14, button: 'left' }); await sleep(40); }
    const mid = await c.evaluate(`${CV} return { tempEdge: cv.edges.size, dragging: cv.isDragging, wrapperCls: cv.wrapperEl.className, hoverTarget: cv.nodeInteractionLayer.target && cv.nodeInteractionLayer.target.id, bNodeCls: cv.nodes.get('b').nodeEl.className }; `);
    await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: tx, y: ty, button: 'left', clickCount: 1 }); await sleep(700); return mid; };
  if (right) {
    p.drawnToMiddle = { mid: await drawFrom('right', b.screen.cx, b.screen.cy), edges: (await edgesData()).filter(e => !edgesBefore.some(x => x.id === e.id)), active: await c.evaluate(`return document.activeElement && document.activeElement.className;`), selection: (await view()).selection };
    await c.pressKey('Escape'); await sleep(200);
    const edgesBefore2 = await edgesData();
    p.drawnToTopEdge = { mid: await drawFrom('right', b.screen.cx, b.screen.y + 4), edges: (await edgesData()).filter(e => !edgesBefore2.some(x => x.id === e.id)) };
    await c.pressKey('Escape'); await sleep(200);
    const edgesBefore2b = await edgesData();
    p.drawnToBottomLeftQuadrant = { mid: await drawFrom('right', b.screen.x + 30, b.screen.y + b.screen.h - 15), edges: (await edgesData()).filter(e => !edgesBefore2b.some(x => x.id === e.id)) };
    await c.pressKey('Escape'); await sleep(200);
    const edgesBefore2c = await edgesData();
    p.drawnFromBottomToB = { mid: await drawFrom('bottom', b.screen.cx, b.screen.cy), edges: (await edgesData()).filter(e => !edgesBefore2c.some(x => x.id === e.id)) };
    await c.pressKey('Escape'); await sleep(200);
    // Drop on empty space.
    const edgesBefore3 = await edgesData(); const nodesBefore3 = await nodeIds();
    const dropX = a.screen.x + a.screen.w + 250, dropY = a.screen.y + 300;
    p.droppedOnEmpty = { mid: await drawFrom('right', dropX, dropY), drop: { x: dropX, y: dropY }, edges: (await edgesData()).filter(e => !edgesBefore3.some(x => x.id === e.id)), newNodes: (await nodeIds()).filter(i => !nodesBefore3.includes(i)), menu: await c.openMenuItems(), menuIcons: await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(m => ({ title: (m.querySelector('.menu-item-title') || {}).textContent, icon: m.querySelector('svg') && m.querySelector('svg').getAttribute('class') }));`), modal: await modal(), editing: await c.evaluate(`return { cm: !!document.querySelector('.canvas-node .cm-editor'), editingNode: !!document.querySelector('.canvas-node.is-editing'), active: document.activeElement && document.activeElement.className };`) };
    p.shotDroppedOnEmpty = shot('edge-dropped-on-empty');
    // If a menu is open, choose "Add card" and see where the card goes.
    if (p.droppedOnEmpty.menu.length && p.droppedOnEmpty.menu[0].includes('Add card')) { await clickMenuItem('Add card'); await sleep(600); const ids = (await nodeIds()).filter(i => !nodesBefore3.includes(i)); p.droppedOnEmptyAddCard = { created: ids, node: ids[0] ? await node(ids[0]) : null, edges: (await edgesData()).filter(e => !edgesBefore3.some(x => x.id === e.id)), dropCanvas: await c.evaluate(`${CV} const w = cv.wrapperEl.getBoundingClientRect(); return { x: (${dropX} - (w.x + w.width / 2)) / cv.scale + cv.x, y: (${dropY} - (w.y + w.height / 2)) / cv.scale + cv.y }; `) }; }
    else { p.droppedOnEmptyNode = p.droppedOnEmpty.newNodes[0] ? await node(p.droppedOnEmpty.newNodes[0]) : null; }
    await c.pressKey('Escape'); await sleep(300); await c.pressKey('Escape'); await sleep(200);
  }
  p.data = await c.evaluate(`${CV} return cv.getData();`);
  // Edge colours per preset at scale 1, from the fixture.
  await fresh('Fixture.canvas'); await setViewport(700, 100, 0);
  p.edgeColors = await c.evaluate(`${CV} return Array.from(cv.edges.values()).map(e => { const s = getComputedStyle(e.lineGroupEl.querySelector('.canvas-display-path')); const end = e.lineEndGroupEl.querySelector('path, polygon, polyline'); return { id: e.id, color: e.color, cls: e.lineGroupEl.getAttribute('class'), stroke: s.stroke, strokeWidth: s.strokeWidth, endFill: end ? getComputedStyle(end).fill : null, ends: Array.from(e.lineEndGroupEl.children).map(ch => ({ tag: ch.tagName, cls: ch.getAttribute('class'), d: ch.getAttribute('d') || ch.getAttribute('points'), transform: ch.getAttribute('transform') })), fromEnd: e.from.end, toEnd: e.to.end, hasFromEnd: !!e.fromLineEnd, hasToEnd: !!e.toLineEnd }; });`);
  p.shotEdgesScale1 = shot('fixture-scale1-dark');
  return p;
});

// Groups.
await step('13-groups', async (p) => {
  await fresh('Fixture.canvas'); await setViewport(700, 100, -0.5);
  p.labels = await c.evaluate(`${CV} return ['g1','g2','g3'].map(id => { const g = cv.nodes.get(id); const l = g.nodeEl.querySelector('.canvas-group-label'); const s = getComputedStyle(l); const cs = getComputedStyle(g.containerEl); const sc = getComputedStyle(g.contentEl || g.containerEl); return { id, label: g.label, cls: g.nodeEl.className, labelCls: l.className, fontSize: s.fontSize, lineHeight: s.lineHeight, fontWeight: s.fontWeight, color: s.color, bg: s.backgroundColor, padding: s.padding, radius: s.borderRadius, transform: s.transform, origin: s.transformOrigin, top: s.top, left: s.left, containerBg: cs.backgroundColor, containerBorder: cs.border, containerBoxShadow: cs.boxShadow, contentBg: sc.backgroundColor, bgImage: cs.backgroundImage, bgSize: cs.backgroundSize, bgRepeat: cs.backgroundRepeat, bgPosition: cs.backgroundPosition, contentBgImage: sc.backgroundImage, contentBgSize: sc.backgroundSize, contentBgRepeat: sc.backgroundRepeat, data: g.getData() }; });`);
  p.labelVsZoom = [];
  for (const z of [1, 0, -0.5, -1, -1.5, -2, -3]) { await setViewport(700, 100, z); p.labelVsZoom.push({ zoom: z, ...(await c.evaluate(`${CV} const l = cv.nodes.get('g2').nodeEl.querySelector('.canvas-group-label'); const s = getComputedStyle(l); const r = l.getBoundingClientRect(); return { scale: cv.scale, fontSize: s.fontSize, transform: s.transform, screenH: r.height, screenFont: parseFloat(s.fontSize) * cv.scale * (s.transform.startsWith('matrix') ? parseFloat(s.transform.slice(7)) : 1), style: l.getAttribute('style') }; `)) }); }
  // backgroundStyle variants on g3 (brought into view first: a node outside the viewport is not in the DOM).
  p.leavesBeforeBg = await c.evaluate(`return app.workspace.getLeavesOfType('canvas').length;`);
  if (!p.leavesBeforeBg) await fresh('Fixture.canvas');
  await setViewport(1850, 30, -0.5);
  p.backgroundStyles = {};
  for (const bs of ['cover', 'ratio', 'repeat', undefined]) { p.backgroundStyles[String(bs)] = await c.evaluate(`${CV} const g = cv.nodes.get('g3'); const d = g.getData(); ${bs ? `d.backgroundStyle = ${JSON.stringify(bs)};` : 'delete d.backgroundStyle;'} g.setData(d); g.render(); await new Promise(r => setTimeout(r, 300)); const els = [g.nodeEl, g.containerEl, g.contentEl].filter(Boolean); return els.map(el => { const s = getComputedStyle(el); return { cls: el.className, bgImage: s.backgroundImage.slice(0, 80), bgSize: s.backgroundSize, bgRepeat: s.backgroundRepeat, bgPosition: s.backgroundPosition }; }).concat([{ data: g.getData() }]);`); }
  await setViewport(560, 540, -1);
  p.shotGroups = shot('groups');
  // Click inside a group's empty area: selects the group, or starts a marquee?
  const g2 = await c.evaluate(`${CV} const g = cv.nodes.get('g2'); const r = g.nodeEl.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right, bottom: r.bottom };`);
  const empty = { x: g2.x + 60, y: g2.bottom - 40 }; // bottom-left corner area of g2, below f1
  p.emptyPoint = { empty, g2 };
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  await c.clickAt(empty.x, empty.y); await sleep(300);
  p.clickEmptyInGroup = { selection: (await view()).selection, hit: await c.evaluate(`${CV} const h = cv.hitTestNode ? cv.hitTestNode(cv.posFromClient({ clientX: ${empty.x}, clientY: ${empty.y} })) : null; return h ? (h.id || String(h)) : null;`) };
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  const before = await view();
  await c.drag(empty.x, empty.y, empty.x - 150, empty.y - 120, { steps: 8 }); await sleep(300);
  p.dragEmptyInGroup = { selection: (await view()).selection, panned: { dx: (await view()).x - before.x }, g2After: (({ x, y }) => ({ x, y }))(await node('g2')) };
  // The same drag, watched half way: a selection box or a moved group?
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: empty.x, y: empty.y }); await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: empty.x, y: empty.y, button: 'left', clickCount: 1 });
  for (let i = 1; i <= 6; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: empty.x + i * 20, y: empty.y - i * 15, button: 'left' }); await sleep(40); }
  p.dragEmptyInGroupMid = await c.evaluate(`${CV} const sel = document.querySelector('.canvas-selection'); return { selectionBox: sel ? sel.getAttribute('style') : null, g2: { x: cv.nodes.get('g2').x, y: cv.nodes.get('g2').y }, dragging: cv.isDragging }; `);
  await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: empty.x + 120, y: empty.y - 90, button: 'left', clickCount: 1 }); await sleep(300);
  await c.evaluate(`${CV} cv.nodes.get('g2').moveTo({ x: -60, y: 260 }); return true;`);
  // Click the label: selects the group.
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  const labRect = () => c.evaluate(`${CV} const l = cv.nodes.get('g2').nodeEl.querySelector('.canvas-group-label'); const r = l.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 };`);
  const lab = await labRect();
  await c.clickAt(lab.cx, lab.cy); await sleep(300);
  p.clickLabel = { lab, selection: (await view()).selection, labelEditing: await c.evaluate(`return document.activeElement && document.activeElement.className;`) };
  await c.clickAt(lab.cx, lab.cy, { clickCount: 2 }); await sleep(400);
  p.dblClickLabel = { selection: (await view()).selection, active: await c.evaluate(`return document.activeElement && document.activeElement.className;`), contenteditable: await c.evaluate(`${CV} return cv.nodes.get('g2').nodeEl.querySelector('.canvas-group-label').getAttribute('contenteditable');`) };
  await c.pressKey('Escape'); await sleep(200);
  // Click the group's border.
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  await c.clickAt(g2.x + 1, g2.y + g2.h / 2); await sleep(300);
  p.clickBorder = { at: { x: g2.x + 1, y: g2.y + g2.h / 2 }, selection: (await view()).selection };
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  await c.clickAt(g2.x + 8, g2.y + g2.h / 2); await sleep(300);
  p.clickInside8px = { selection: (await view()).selection };
  // Drag a group by its border.
  await c.evaluate(`${CV} cv.deselectAll(); return true;`);
  const posB0 = await c.evaluate(`${CV} return Object.fromEntries(['f1','g2'].map(id => [id, { x: cv.nodes.get(id).x, y: cv.nodes.get(id).y }]));`);
  await c.drag(g2.x + 1, g2.y + g2.h / 2, g2.x + 1 + 40, g2.y + g2.h / 2 + 30, { steps: 8 }); await sleep(300);
  p.dragBorder = { before: posB0, after: await c.evaluate(`${CV} return Object.fromEntries(['f1','g2'].map(id => [id, { x: cv.nodes.get(id).x, y: cv.nodes.get(id).y }]));`) };
  await c.evaluate(`${CV} cv.nodes.get('g2').moveTo({ x: -60, y: 260 }); cv.nodes.get('f1').moveTo({ x: 0, y: 300 }); return true;`);
  // Drag the group by its label: which nodes move? f1 (fully inside), l1 fully inside; make f2 straddle the border first.
  await c.evaluate(`${CV} cv.nodes.get('f2').moveTo({ x: 500, y: 700 }); cv.deselectAll(); return true;`); // f2 y 700..940 straddles g2's bottom at 820
  const pos0 = await c.evaluate(`${CV} return Object.fromEntries(['f1','f2','l1','t0','g2','g1'].map(id => [id, { x: cv.nodes.get(id).x, y: cv.nodes.get(id).y }]));`);
  p.containing = await c.evaluate(`${CV} const g = cv.nodes.get('g2'); return { getContainingNodes: cv.getContainingNodes(g.getBBox()).map(n => n.id), intersecting: cv.getIntersectingNodes(g.getBBox()).map(n => n.id), bbox: g.getBBox() };`);
  const lab2 = await c.evaluate(`${CV} const l = cv.nodes.get('g2').nodeEl.querySelector('.canvas-group-label'); const r = l.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  await c.drag(lab2.x, lab2.y, lab2.x + 60, lab2.y + 40, { steps: 10 }); await sleep(400);
  const pos1 = await c.evaluate(`${CV} return Object.fromEntries(['f1','f2','l1','t0','g2','g1'].map(id => [id, { x: cv.nodes.get(id).x, y: cv.nodes.get(id).y }]));`);
  p.dragGroup = { before: pos0, after: pos1, moved: Object.fromEntries(Object.keys(pos0).map(k => [k, { dx: pos1[k].x - pos0[k].x, dy: pos1[k].y - pos0[k].y }])) };
  // Create a group: select t1 and t2, right-click, "Create group" (Ctrl+G is Obsidian's global "Open graph view").
  await c.evaluate(`${CV} cv.deselectAll(); cv.select(cv.nodes.get('t1')); cv.select(cv.nodes.get('t2')); return true;`);
  const nodesBefore = await nodeIds();
  const t1g = grab(await node('t1')); await c.clickAt(t1g.x, t1g.y, { button: 'right' }); await sleep(500);
  p.multiContextMenu = await c.openMenuItems();
  p.groupItemClicked = await clickMenuItem('Create group'); await sleep(600);
  const newG = (await nodeIds()).filter(i => !nodesBefore.includes(i));
  p.ctrlG = { created: newG, node: newG[0] ? await node(newG[0]) : null, t1: await node('t1'), t2: await node('t2'), editing: await c.evaluate(`return { active: document.activeElement && document.activeElement.className, editable: !!document.querySelector('.canvas-group-label[contenteditable=true], .canvas-group-label input'), labelEl: (() => { const l = document.querySelector('.canvas-node.is-focused .canvas-group-label, .canvas-node.is-selected .canvas-group-label'); return l ? { cls: l.className, ce: l.getAttribute('contenteditable'), text: l.textContent } : null; })() };`), selection: (await view()).selection };
  p.shotCtrlG = shot('ctrl-g-group');
  await c.pressKey('Escape'); await sleep(300);
  p.ctrlGAfterEscape = { node: newG[0] ? await node(newG[0]) : null, selection: (await view()).selection };
  // Context menu of the selection.
  p.data = await c.evaluate(`${CV} return cv.getData();`);
  return p;
});

// Keyboard: the view's scope, the canvas's own key table, and each key's effect.
await step('14-keyboard', async (p) => {
  await fresh('Two.canvas'); await setViewport(300, 100, 0);
  p.scopeKeys = await c.evaluate(`${CV} const dump = (sc) => sc && sc.keys ? sc.keys.map(k => ({ modifiers: k.modifiers, key: k.key, scope: 'view' })) : null; return { view: dump(v.scope), canvasKeys: cv.keys ? Object.keys(cv.keys) : null, canvasKeysVal: cv.keys, appScope: app.scope.keys.filter(k => true).map(k => ({ modifiers: k.modifiers, key: k.key })).slice(0, 80), parent: v.scope && v.scope.parent ? 'has parent' : null };`);
  // Try each key with a selected.
  const tryKey = async (label, name, mods = {}) => { if (!(await c.evaluate(`return app.workspace.getLeavesOfType('canvas').length;`)) || !(await nodeIds()).includes('a')) { await fresh('Two.canvas'); await setViewport(300, 100, 0); }
    await c.evaluate(`${CV} cv.deselectAll(); return true;`); const ga = grab(await node('a')); await c.clickAt(ga.x, ga.y); await sleep(200); const b = await node('a'); const bv = await view(); const ids = await nodeIds(); const edges = (await edgesData()).length;
    await c.pressKey(name, mods); await sleep(500);
    const leafGone = !(await c.evaluate(`return app.workspace.getLeavesOfType('canvas').length;`)); if (leafGone) { p[label] = { key: name, mods, leafGone: true, activeView: await c.evaluate(`return app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType();`) }; await fresh('Two.canvas'); await setViewport(300, 100, 0); return; }
    const a = await node('a'); const av = await view();
    p[label] = { key: name, mods, moved: a ? { dx: a.x - b.x, dy: a.y - b.y } : 'node gone', editing: a ? a.isEditing : null, cm: await c.evaluate(`return !!document.querySelector('.canvas-node .cm-editor');`), selection: av.selection, nodes: (await nodeIds()).length - ids.length, edges: (await edgesData()).length - edges, zoom: av.zoom - bv.zoom, pan: { dx: av.x - bv.x, dy: av.y - bv.y }, modal: await modal(), menu: await c.openMenuItems(), active: await c.evaluate(`return document.activeElement && (document.activeElement.className || document.activeElement.tagName);`) };
    await c.pressKey('Escape'); await sleep(150); await c.evaluate(`${CV} cv.deselectAll(); return true;`); };
  await tryKey('ArrowRight', 'ArrowRight'); await tryKey('ArrowDown', 'ArrowDown'); await tryKey('ShiftArrowRight', 'ArrowRight', { shift: true }); await tryKey('AltArrowRight', 'ArrowRight', { alt: true }); await tryKey('CtrlArrowRight', 'ArrowRight', { ctrl: true });
  await c.evaluate(`${CV} cv.nodes.get('a').moveTo({ x: 0, y: 0 }); return true;`);
  await tryKey('Enter', 'Enter'); await tryKey('Space', 'Space'); await tryKey('F2', 'F2'); await tryKey('Tab', 'Tab');
  await tryKey('Shift1', '1', { shift: true }); await tryKey('Shift2', '2', { shift: true }); await tryKey('Shift0', '0', { shift: true }); await tryKey('Ctrl0', '0', { ctrl: true });
  await tryKey('CtrlEquals', '=', { ctrl: true }); await tryKey('CtrlMinus', '-', { ctrl: true }); await tryKey('CtrlPlus', '+', { ctrl: true, shift: true });
  await tryKey('CtrlA', 'a', { ctrl: true }); await tryKey('CtrlD', 'd', { ctrl: true }); await tryKey('CtrlG', 'g', { ctrl: true }); await tryKey('CtrlShiftG', 'g', { ctrl: true, shift: true }); await tryKey('CtrlE', 'e', { ctrl: true });
  // Copy: what the clipboard holds.
  await c.writeClipboard('');
  await tryKey('CtrlC', 'c', { ctrl: true }); p.CtrlC.clipboard = await c.readClipboard();
  p.clipboardHtml = await c.evaluate(`const cb = window.require('electron').clipboard; return { formats: cb.availableFormats(), html: cb.readHTML().slice(0, 500) };`);
  await tryKey('CtrlV', 'v', { ctrl: true }); p.CtrlV.data = await c.evaluate(`${CV} return cv.getData().nodes.map(n => ({ id: n.id, x: n.x, y: n.y, text: n.text }));`);
  p.CtrlV.pasteOffset = await c.evaluate(`${CV} return { mouse: cv.pointer, nodes: cv.nodes.size };`);
  await tryKey('CtrlX', 'x', { ctrl: true }); p.CtrlX.clipboard = await c.readClipboard();
  await tryKey('CtrlZ', 'z', { ctrl: true }); await tryKey('CtrlShiftZ', 'z', { ctrl: true, shift: true }); await tryKey('CtrlY', 'y', { ctrl: true });
  await tryKey('Delete', 'Delete'); await tryKey('Backspace', 'Backspace');
  p.dataAfter = await c.evaluate(`${CV} return cv.getData();`);
  // Paste plain text from the system clipboard onto the canvas: a new card?
  await c.writeClipboard('plain clipboard text');
  await c.evaluate(`${CV} cv.deselectAll(); cv.wrapperEl.focus(); return true;`);
  const ids = await nodeIds();
  await c.pressKey('v', { ctrl: true }); await sleep(600);
  const newIds = (await nodeIds()).filter(i => !ids.includes(i));
  p.pastePlainText = { created: newIds, node: newIds[0] ? await node(newIds[0]) : null };
  await c.writeClipboard('https://example.org/page');
  const ids2 = await nodeIds();
  await c.pressKey('v', { ctrl: true }); await sleep(600);
  const newIds2 = (await nodeIds()).filter(i => !ids2.includes(i));
  p.pasteUrl = { created: newIds2, node: newIds2[0] ? await node(newIds2[0]) : null };
  // Escape while a card is selected; Escape while editing is covered in 09.
  return p;
});

// Menus: the floating menu per node type, the context menus.
await step('15-menus', async (p) => {
  await fresh('Fixture.canvas'); await setViewport(700, 100, -0.3);
  const menuFor = async (id) => { await c.evaluate(`${CV} cv.deselectAll(); cv.selectOnly(cv.nodes.get(${JSON.stringify(id)})); return true;`); await sleep(400);
    return c.evaluate(`${CV} const m = cv.menu; const el = m.menuEl || m.containerEl || document.querySelector('.canvas-menu'); return { keys: Object.keys(m), cls: el && el.className, items: el ? Array.from(el.querySelectorAll('.clickable-icon, .canvas-menu-item, button')).map(b => ({ label: b.getAttribute('aria-label'), icon: b.querySelector('svg') && b.querySelector('svg').getAttribute('class'), cls: b.className })) : null, rect: el && el.getBoundingClientRect().toJSON(), style: el && el.getAttribute('style'), nodeRect: cv.nodes.get(${JSON.stringify(id)}).nodeEl.getBoundingClientRect().toJSON(), cs: el ? (() => { const s = getComputedStyle(el); return { bg: s.backgroundColor, radius: s.borderRadius, padding: s.padding, gap: s.gap, boxShadow: s.boxShadow, border: s.border, height: s.height }; })() : null }; `); };
  p.nodeIds = await nodeIds();
  for (const id of ['t0', 'f1', 'f2', 'l1', 'g1']) { try { p['menu_' + id] = await menuFor(id); } catch (e) { p['menu_' + id] = { error: String(e).slice(0, 200) }; } }
  p.shotMenuText = (await menuFor('t0'), shot('floating-menu-text'));
  // Click each floating menu item for the text card and record what opens.
  const items = p.menu_t0.items || [];
  for (let i = 0; i < items.length; i++) { await menuFor('t0'); const r = await c.rect('.canvas-menu .clickable-icon', i); if (!r) continue; await c.clickAt(r.cx, r.cy); await sleep(500);
    p['menuClick_t0_' + i] = { label: items[i].label, menu: await c.openMenuItems(), modal: await modal(), popover: await c.evaluate(`const pop = document.querySelector('.canvas-color-picker, .menu, .popover, .suggestion-container, .canvas-submenu'); return pop ? { cls: pop.className, text: pop.innerText.slice(0, 300), swatches: Array.from(pop.querySelectorAll('[class*=color]')).map(s => ({ cls: s.className, label: s.getAttribute('aria-label'), style: s.getAttribute('style') })) } : null;`), editing: await c.evaluate(`return !!document.querySelector('.canvas-node .cm-editor');`) };
    if (/color/i.test(items[i].label || '')) p.shotColorPicker = shot('color-picker');
    await c.pressKey('Escape'); await sleep(200); await c.pressKey('Escape'); await sleep(200); }
  // Edge floating menu and its items.
  await c.evaluate(`${CV} cv.deselectAll(); cv.selectOnly(cv.edges.get('e1')); return true;`); await sleep(400);
  p.menu_edge = await c.evaluate(`${CV} const el = document.querySelector('.canvas-menu'); return el ? Array.from(el.querySelectorAll('.clickable-icon')).map(b => ({ label: b.getAttribute('aria-label'), icon: b.querySelector('svg') && b.querySelector('svg').getAttribute('class') })) : null;`);
  for (let i = 0; i < (p.menu_edge || []).length; i++) { await c.evaluate(`${CV} cv.deselectAll(); cv.selectOnly(cv.edges.get('e1')); return true;`); await sleep(300); const r = await c.rect('.canvas-menu .clickable-icon', i); if (!r) continue; await c.clickAt(r.cx, r.cy); await sleep(500);
    p['menuClick_edge_' + i] = { label: p.menu_edge[i].label, menu: await c.openMenuItems(), menuChecked: await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(m => ({ title: (m.querySelector('.menu-item-title') || {}).textContent, checked: m.classList.contains('mod-checked') || !!m.querySelector('.mod-checked'), icon: m.querySelector('svg') && m.querySelector('svg').getAttribute('class') }));`), active: await c.evaluate(`return document.activeElement && document.activeElement.className;`) };
    await c.pressKey('Escape'); await sleep(200); await c.pressKey('Escape'); await sleep(200); }
  // Multi-selection menu.
  await c.evaluate(`${CV} cv.deselectAll(); cv.select(cv.nodes.get('t1')); cv.select(cv.nodes.get('t2')); return true;`); await sleep(400);
  p.menu_multi = await c.evaluate(`const el = document.querySelector('.canvas-menu'); return el ? Array.from(el.querySelectorAll('.clickable-icon')).map(b => ({ label: b.getAttribute('aria-label'), icon: b.querySelector('svg') && b.querySelector('svg').getAttribute('class') })) : null;`);
  // Context menus.
  const ctx = async (label, id, dy = 6) => { await c.evaluate(`${CV} cv.deselectAll(); return true;`); const n = await node(id); const g = id.startsWith('g') ? { x: n.screen.x + 2, y: n.screen.y + n.screen.h / 2 } : grab(n); await c.clickAt(g.x, g.y, { button: 'right' }); await sleep(500); p['ctx_' + label] = { items: await c.openMenuItems(), icons: await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(m => ({ title: (m.querySelector('.menu-item-title') || {}).textContent, icon: m.querySelector('svg') && m.querySelector('svg').getAttribute('class'), sub: m.classList.contains('mod-submenu') || !!m.querySelector('.menu-item-icon.mod-submenu'), cls: m.className }));`) }; if (label === 'text') p.shotCtxText = shot('context-menu-text'); await c.pressKey('Escape'); await sleep(200); };
  await ctx('text', 't0'); await ctx('file', 'f1'); await ctx('image', 'f2'); await ctx('link', 'l1'); await ctx('group', 'g1', 2);
  // Multi-selection context menu.
  await c.evaluate(`${CV} cv.deselectAll(); cv.select(cv.nodes.get('t1')); cv.select(cv.nodes.get('t2')); return true;`); const t1 = await node('t1'); const t1g = grab(t1); await c.clickAt(t1g.x, t1g.y, { button: 'right' }); await sleep(500);
  p.ctx_multi = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  // Empty canvas.
  await c.evaluate(`${CV} cv.deselectAll(); return true;`); const v = await view();
  await c.clickAt(v.wrapper.x + 40, v.wrapper.y + v.wrapper.h - 100, { button: 'right' }); await sleep(500);
  p.ctx_empty = { items: await c.openMenuItems(), icons: await c.evaluate(`return Array.from(document.querySelectorAll('.menu .menu-item')).map(m => ({ title: (m.querySelector('.menu-item-title') || {}).textContent, icon: m.querySelector('svg') && m.querySelector('svg').getAttribute('class') }));`) };
  p.shotCtxEmpty = shot('context-menu-empty');
  await c.pressKey('Escape'); await sleep(200);
  // Right-click on an edge is in 12. The tab's context menu and the view's three-dot menu are in 08.
  return p;
});

// Text editing: double-click a text card.
await step('16-text-edit', async (p) => {
  await fresh('Fixture.canvas'); await setViewport(130, 80, 0);
  const t0 = await node('t0');
  p.before = { node: t0, contentDom: await c.evaluate(`${CV} const n = cv.nodes.get('t0'); return { contentCls: n.contentEl.className, inner: Array.from(n.contentEl.children).map(e => e.className), html: n.contentEl.innerHTML.slice(0, 600) };`) };
  await c.clickAt(t0.screen.cx, t0.screen.cy, { clickCount: 2 }); await sleep(800);
  p.editing = await c.evaluate(`${CV} const n = cv.nodes.get('t0'); const cm = n.nodeEl.querySelector('.cm-editor'); return { isEditing: n.isEditing, cls: n.nodeEl.className, contentCls: n.contentEl.className, cm: cm ? { cls: cm.className, rect: cm.getBoundingClientRect().toJSON(), livePreview: cm.classList.contains('is-live-preview'), lines: Array.from(cm.querySelectorAll('.cm-line')).length } : null, nodeRect: n.nodeEl.getBoundingClientRect().toJSON(), active: document.activeElement && document.activeElement.className, inner: Array.from(n.contentEl.children).map(e => e.className), html: n.contentEl.innerHTML.slice(0, 800) };`);
  p.shotEditing = shot('text-editing');
  // Type a few more lines: does the card grow?
  await c.pressKey('End', { ctrl: true }); await c.pressKey('Enter'); await c.typeText('line one'); await c.pressKey('Enter'); await c.typeText('line two'); await c.pressKey('Enter'); await c.typeText('line three'); await c.pressKey('Enter'); await c.typeText('line four'); await c.pressKey('Enter'); await c.typeText('line five'); await sleep(500);
  p.afterTyping = { node: await node('t0'), cmHeight: await c.evaluate(`${CV} const n = cv.nodes.get('t0'); const cm = n.nodeEl.querySelector('.cm-editor'); return { cm: cm && cm.getBoundingClientRect().height, scroller: cm && cm.querySelector('.cm-scroller') && cm.querySelector('.cm-scroller').scrollHeight, content: n.contentEl.getBoundingClientRect().height }; `) };
  // Exit with Escape.
  await c.pressKey('Escape'); await sleep(500);
  p.afterEscape = { node: await node('t0'), selection: (await view()).selection, active: await c.evaluate(`return document.activeElement && (document.activeElement.className || document.activeElement.tagName);`) };
  await sleep(1500);
  p.afterEscape1500 = { isEditing: (await node('t0')).isEditing, cls: (await node('t0')).cls };
  await c.pressKey('Escape'); await sleep(500);
  p.afterSecondEscape = { isEditing: (await node('t0')).isEditing, cls: (await node('t0')).cls, selection: (await view()).selection };
  { const v0 = await view(); await c.mouseMove(v0.wrapper.x + 30, v0.wrapper.y + v0.wrapper.h - 60); await sleep(400); p.afterEscapeMouseMove = { isEditing: (await node('t0')).isEditing, cls: (await node('t0')).cls }; }
  p.write = await waitForWrite('Fixture.canvas', FIXTURES['Fixture.canvas'], 5000);
  // Edit again, click outside.
  const t0b = await node('t0');
  await c.clickAt(t0b.screen.cx, t0b.screen.cy, { clickCount: 2 }); await sleep(600);
  await c.typeText('X');
  const v = await view(); await c.clickAt(v.wrapper.x + 30, v.wrapper.y + v.wrapper.h - 60); await sleep(500);
  p.clickOutside = { node: await node('t0'), selection: (await view()).selection, cm: await c.evaluate(`return !!document.querySelector('.canvas-node .cm-editor');`) };
  // Empty the text and leave.
  const t1 = await node('t1');
  await c.clickAt(t1.screen.cx, t1.screen.cy, { clickCount: 2 }); await sleep(600);
  await c.pressKey('a', { ctrl: true }); await c.pressKey('Backspace'); await sleep(200);
  await c.pressKey('Escape'); await sleep(600);
  p.emptied = { node: await node('t1'), exists: (await nodeIds()).includes('t1'), text: await c.evaluate(`${CV} const n = cv.nodes.get('t1'); return n ? n.text : null;`) };
  // Single click on a selected card: starts editing?
  const t2 = await node('t2');
  await c.clickAt(t2.screen.cx, t2.screen.cy); await sleep(400);
  const once = await node('t2');
  await c.clickAt(t2.screen.cx, t2.screen.cy); await sleep(400);
  p.clickSelectedTwice = { afterFirst: once.isEditing, afterSecond: (await node('t2')).isEditing };
  await c.pressKey('Escape');
  // Enter on a selected card.
  await c.evaluate(`${CV} cv.deselectAll(); cv.selectOnly(cv.nodes.get('t3')); cv.wrapperEl.focus(); return true;`); await sleep(200);
  await c.pressKey('Enter'); await sleep(400);
  p.enterOnSelected = { isEditing: (await node('t3')).isEditing, active: await c.evaluate(`return document.activeElement && document.activeElement.className;`) };
  await c.pressKey('Escape'); await sleep(300);
  // Double click a file card and a link card.
  const f1 = await node('f1');
  await c.clickAt(f1.screen.cx, f1.screen.cy, { clickCount: 2 }); await sleep(800);
  p.dblClickFile = { isEditing: (await node('f1')).isEditing, cm: await c.evaluate(`${CV} const n = cv.nodes.get('f1'); return { cm: !!n.nodeEl.querySelector('.cm-editor'), inner: Array.from(n.contentEl.children).map(e => e.className), leaves: app.workspace.getLeavesOfType('markdown').length }; `) };
  await c.pressKey('Escape'); await sleep(300);
  p.text = bytes('Fixture.canvas');
  return p;
});

// Background dots vs zoom.
await step('17-background', async (p) => {
  await fresh('Two.canvas');
  p.vsZoom = [];
  for (const z of [2, 1, 0.5, 0, -0.5, -1, -1.5, -2, -3]) { await setViewport(400, 50, z); await c.evaluate(`${CV} cv.panBy(0.001, 0); await new Promise(r => setTimeout(r, 200)); return true;`);
    p.vsZoom.push({ zoom: z, ...(await c.evaluate(`${CV} const pat = cv.backgroundPatternEl; const circ = pat.querySelector('circle'); const rect = pat.parentElement.querySelector('rect') || cv.wrapperEl.querySelector('.canvas-background rect'); return { scale: cv.scale, x: cv.x, y: cv.y, pattern: Object.fromEntries(Array.from(pat.attributes).map(a => [a.name, a.value])), circle: Object.fromEntries(Array.from(circ.attributes).map(a => [a.name, a.value])), circleFill: getComputedStyle(circ).fill, rect: rect ? Object.fromEntries(Array.from(rect.attributes).map(a => [a.name, a.value])) : null, svg: Object.fromEntries(Array.from(cv.wrapperEl.querySelector('.canvas-background').attributes).map(a => [a.name, a.value])) }; `)) }); }
  // Where the grid spacing doubles: a fine sweep of zoom.
  p.gridSweep = [];
  for (let z = 0; z >= -4.001; z -= 0.1) { const zz = Math.round(z * 10) / 10; await c.evaluate(`${CV} cv.setViewport(400, 50, ${zz}); return true;`); await sleep(60); p.gridSweep.push(await c.evaluate(`${CV} return [cv.zoom, cv.gridSpacing, parseFloat(cv.backgroundPatternEl.getAttribute('width'))];`)); }
  // Pan by a known amount and read the pattern offset.
  await setViewport(400, 50, 0);
  const before = await c.evaluate(`${CV} return Object.fromEntries(Array.from(cv.backgroundPatternEl.attributes).map(a => [a.name, a.value]));`);
  await c.evaluate(`${CV} cv.panBy(37, 23); await new Promise(r => setTimeout(r, 300)); return true;`);
  p.pan = { before, after: await c.evaluate(`${CV} return { attrs: Object.fromEntries(Array.from(cv.backgroundPatternEl.attributes).map(a => [a.name, a.value])), x: cv.x, y: cv.y }; `) };
  return p;
});

// Save timing, and when workspace.json records the viewport.
await step('18-save', async (p) => {
  await fresh('Two.canvas'); await setViewport(300, 100, 0);
  const before = bytes('Two.canvas');
  const a = await node('a');
  const t = Date.now();
  const ga = grab(a); await c.drag(ga.x, ga.y, ga.x + 60, ga.y + 40, { steps: 6, pauseMs: 20 });
  const dragEnd = Date.now();
  p.afterDrag = await waitForWrite('Two.canvas', before, 10000);
  p.afterDrag.sinceDragEndMs = Date.now() - dragEnd - 0;
  p.text = bytes('Two.canvas');
  // Two quick edits: one write or two?
  const b2 = bytes('Two.canvas');
  const a2 = await node('a');
  { const g2 = grab(a2); await c.drag(g2.x, g2.y, g2.x + 40, g2.y, { steps: 4, pauseMs: 20 }); }
  await sleep(700);
  const a3 = await node('a');
  { const g3 = grab(a3); await c.drag(g3.x, g3.y, g3.x + 40, g3.y, { steps: 4, pauseMs: 20 }); }
  const t2 = Date.now();
  p.secondWrite = await waitForWrite('Two.canvas', b2, 10000);
  p.secondWriteText = bytes('Two.canvas');
  // workspace.json: pan, then watch.
  const ws0 = bytes('.obsidian/workspace.json');
  await setViewport(123, 45, -0.25);
  await c.evaluate(`${CV} cv.panBy(10, 10); return true;`);
  p.wsAfterPan = await waitForWrite('.obsidian/workspace.json', ws0, 8000);
  p.wsViewState = (() => { const w = json('.obsidian/workspace.json'); const find = (n) => { if (!n) return null; if (n.type === 'leaf' && n.state && n.state.type === 'canvas') return n; for (const ch of (n.children || [])) { const r = find(ch); if (r) return r; } return null; }; return w ? find(w.main) : null; })();
  // A real wheel pan then wait.
  const ws1 = bytes('.obsidian/workspace.json');
  const v = await view(); await wheel(v.wrapper.cx, v.wrapper.cy, 100); await sleep(100);
  p.wsAfterWheel = await waitForWrite('.obsidian/workspace.json', ws1, 8000);
  p.wsLeafAfterWheel = (() => { const w = json('.obsidian/workspace.json'); const find = (n) => { if (!n) return null; if (n.type === 'leaf' && n.state && n.state.type === 'canvas') return n; for (const ch of (n.children || [])) { const r = find(ch); if (r) return r; } return null; }; return w ? find(w.main) : null; })();
  p.liveState = await view();
  // Close the leaf: workspace.json then.
  const ws2 = bytes('.obsidian/workspace.json');
  await closeActive();
  p.wsAfterClose = await waitForWrite('.obsidian/workspace.json', ws2, 8000);
  p.wsFull = json('.obsidian/workspace.json');
  // Reopen and check the viewport is restored.
  await openCanvas('Two.canvas');
  p.reopened = await view();
  return p;
});

save();
c.close();
process.exit(0);
