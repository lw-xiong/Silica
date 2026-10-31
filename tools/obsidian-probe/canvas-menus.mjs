// Canvas menus, for #48: every list the canvas shows, captured item by item.
//
// The 2026-10-21 run (results/2026-10-21-canvas-view.json, probes a/b/c) left
// three bad captures behind: probe-b2 right-clicked past its targets and got
// the background menu seven times, probe-c read the menu wrapper instead of
// its items, and probe-b's lists dropped every row without a
// `.menu-item-title` (two adjacent `---` is the tell). So this one:
//
//   - pans each target to the middle of the view and takes the target's own
//     DOM rect, rather than mapping canvas coordinates and hoping;
//   - checks `elementFromPoint` *before* the right-click and records what was
//     actually under the pointer, so a miss is visible in the JSON;
//   - walks `.menu > *` — class, title, icon, checked, disabled, swatches —
//     and keeps each menu's `outerHTML`, which is the ground truth a dropped
//     row can be recovered from;
//   - covers the two targets never captured (a group and a file card) and the
//     submenus, and *applies* Align and Arrange so the geometry is recorded
//     and not just the labels.
//
// Phases: fixtures | run   (node canvas-menus.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const only = process.env.ONLY_STEPS ? process.env.ONLY_STEPS.split(',') : null;
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run'].includes(phase)) { console.error('phase must be fixtures|run'); process.exit(2); }

const T0 = Date.now();
const load = () => { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } };
const all = load();
all.canvasMenus = all.canvasMenus || {};
const out = all.canvasMenus[phase] && only ? all.canvasMenus[phase] : { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.canvasMenus[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

async function step(name, fn) {
  if (only && !only.includes(name)) return out.steps[name];
  if (!out.order.includes(name)) out.order.push(name);
  const t = Date.now();
  try { const v = await fn(); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' && !Array.isArray(v) ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e) }; }
  save();
  console.error(`[${name}] ${out.steps[name].ok ? 'ok' : 'FAIL'} ${out.steps[name].ms}ms`);
  return out.steps[name];
}

const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };

// ---- fixtures -------------------------------------------------------------
// 64x48 solid orange, so a file card can hold an image and a group a background.
function png(w, h, rgb) {
  const crc = (buf) => { let c = ~0; for (const b of buf) { c ^= b; for (let i = 0; i < 8; i++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1)); } return (~c) >>> 0; };
  const chunk = (type, data) => { const len = Buffer.alloc(4); len.writeUInt32BE(data.length); const td = Buffer.concat([Buffer.from(type), data]); const ck = Buffer.alloc(4); ck.writeUInt32BE(crc(td)); return Buffer.concat([len, td, ck]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  const row = Buffer.concat([Buffer.from([0]), Buffer.from(Array.from({ length: w }, () => rgb).flat())]);
  const raw = Buffer.concat(Array.from({ length: h }, () => row));
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

// Targets far apart, so a right-click that lands wrong lands on nothing and
// says so rather than quietly hitting a neighbour.
const MENUS = {
  nodes: [
    { id: 't1', type: 'text', text: 'Text card', x: 0, y: 0, width: 220, height: 120 },
    { id: 't2', type: 'text', text: 'Second', x: 640, y: 0, width: 220, height: 120 },
    { id: 't3', type: 'text', text: 'Third', x: 0, y: 320, width: 220, height: 120 },
    { id: 't4', type: 'text', text: 'Fourth', x: 640, y: 320, width: 220, height: 120 },
    { id: 'c1', type: 'text', text: 'Coloured', x: 0, y: 640, width: 220, height: 120, color: '3' },
    { id: 'f1', type: 'file', file: 'Plan.md', x: 640, y: 640, width: 360, height: 260 },
    { id: 'f2', type: 'file', file: 'pic.png', x: 1200, y: 640, width: 260, height: 200 },
    { id: 'l1', type: 'link', url: 'https://example.com', x: 1200, y: 0, width: 360, height: 260 },
    { id: 'g1', type: 'group', x: 1800, y: 0, width: 520, height: 360, label: 'Group one' },
    { id: 'g2', type: 'group', x: 1800, y: 480, width: 520, height: 360, label: 'Coloured group', color: '5' },
    // Three cards at ragged positions, for Align and Arrange.
    { id: 'a1', type: 'text', text: 'A1', x: 2600, y: 0, width: 160, height: 80 },
    { id: 'a2', type: 'text', text: 'A2', x: 2820, y: 160, width: 240, height: 60 },
    { id: 'a3', type: 'text', text: 'A3', x: 2660, y: 320, width: 120, height: 140 },
  ],
  edges: [
    { id: 'e1', fromNode: 't1', fromSide: 'right', toNode: 't2', toSide: 'left', label: 'labelled' },
    { id: 'e2', fromNode: 't3', fromSide: 'right', toNode: 't4', toSide: 'left' },
  ],
};

const FIXTURES = {
  'Menus.canvas': JSON.stringify(MENUS, null, 2) + '\n',
  'Plan.md': '# Plan\n\nA note for the file card.\n\n- one\n- two\n',
};

if (phase === 'fixtures') {
  await step('fixtures', async () => {
    fs.mkdirSync(vp('.obsidian'), { recursive: true });
    fs.writeFileSync(vp('.obsidian/app.json'), '{}');
    for (const [name, text] of Object.entries(FIXTURES)) fs.writeFileSync(vp(name), text);
    fs.writeFileSync(vp('pic.png'), png(64, 48, [255, 136, 0]));
    return { vaultFiles: fs.readdirSync(VAULT).sort() };
  });
  process.exit(0);
}

// ---- run ------------------------------------------------------------------
const c = await connect();
const vault = await c.vaultPath();
if (vault.replace(/\/+$/, '') !== VAULT.replace(/\/+$/, '')) { console.error('wrong vault open: ' + vault); c.close(); process.exit(3); }
out.vault = vault;

const ev = (s) => c.evaluate(s);
const C = `const leaf = app.workspace.getLeavesOfType('canvas')[0]; if (!leaf) throw new Error('no canvas leaf'); const v = leaf.view; const cv = v.canvas;`;

// Every open `.menu`, item by item. 1.14.5 nests them: `.menu` holds a
// `.menu-grabber` and a `.menu-scroll`, and the items sit in `.menu-group`
// children of that, each tagged with a `data-section`. Walking `.menu > *` —
// which this script did on its first run, and probe-c did in 2026-10-21 —
// sees the two wrappers and nothing else. The group index and section are
// recorded because they are what the earlier capture read as stray `---`.
const DUMP = `
  const dump = () => Array.from(document.querySelectorAll('.menu')).map(m => {
    const groups = Array.from(m.querySelectorAll('.menu-group'));
    return {
      cls: m.className,
      groups: groups.length,
      items: Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map(e => ({
        cls: e.className,
        separator: e.classList.contains('menu-separator'),
        section: e.getAttribute('data-section'),
        group: groups.indexOf(e.closest('.menu-group')),
        title: e.querySelector('.menu-item-title') ? e.querySelector('.menu-item-title').textContent.trim() : null,
        text: (e.textContent || '').trim() || null,
        icon: e.querySelector('.menu-item-icon svg') ? e.querySelector('.menu-item-icon svg').getAttribute('class') : null,
        aria: e.getAttribute('aria-label'),
        checked: e.classList.contains('mod-checked') || e.getAttribute('aria-checked') === 'true' || !!e.querySelector('.menu-item-icon.mod-checked'),
        disabled: e.classList.contains('is-disabled') || e.classList.contains('mod-disabled'),
        submenu: e.classList.contains('mod-submenu') || /chevron-right/.test(e.innerHTML),
        swatches: Array.from(e.querySelectorAll('.canvas-color-picker-item')).map(s => ({ cls: s.className, aria: s.getAttribute('aria-label'), bg: getComputedStyle(s).backgroundColor })),
        // Separators come in pairs in the DOM. Only the painted ones belong in
        // a parity list, so every row carries its own box.
        h: e.offsetHeight, display: getComputedStyle(e).display,
        visible: !!e.offsetParent && e.offsetHeight > 0 && getComputedStyle(e).display !== 'none',
        html: e.outerHTML.slice(0, 300),
      })),
      html: m.outerHTML.slice(0, 20000),
    };
  });`;

const dumpMenus = () => ev(`${DUMP} return dump();`);
const closeMenus = async () => { await c.pressKey('Escape'); await sleep(150); await ev(`for (const m of document.querySelectorAll('.menu')) m.remove(); return true;`); await sleep(100); };

/** Pan `id` to the middle of the view at scale 1 and hand back its DOM rect. */
const frameNode = async (id, zoom = 0) => {
  const r = await ev(`${C} const n = cv.nodes.get(${JSON.stringify(id)}); if (!n) throw new Error('no node ' + ${JSON.stringify(id)});
    cv.setViewport(n.x + n.width / 2, n.y + n.height / 2, ${zoom}); return true;`);
  await sleep(500);
  return ev(`${C} const n = cv.nodes.get(${JSON.stringify(id)}); const b = n.nodeEl.getBoundingClientRect();
    return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2, label: (() => { const l = n.nodeEl.querySelector('.canvas-group-label'); if (!l) return null; const lb = l.getBoundingClientRect(); return { cx: lb.x + lb.width / 2, cy: lb.y + lb.height / 2 }; })() };`);
};

/** The screen point on an edge's own path, half way along it. */
const edgePoint = async (id) => ev(`${C} const e = cv.edges.get(${JSON.stringify(id)}); if (!e) throw new Error('no edge ' + ${JSON.stringify(id)});
  const from = cv.nodes.get(e.from.node.id), to = cv.nodes.get(e.to.node.id);
  cv.setViewport((from.x + to.x + to.width) / 2, (from.y + from.height / 2 + to.y + to.height / 2) / 2, 0);
  await new Promise(r => setTimeout(r, 500));
  const p = e.path && (e.path.display || e.path.interaction); if (!p) throw new Error('no edge path');
  const pt = p.getPointAtLength(p.getTotalLength() / 2); const m = p.getScreenCTM(); const sp = pt.matrixTransform(m);
  return { x: sp.x, y: sp.y, label: !!(e.label && e.label.length) };`);

/** What is under (x, y), and which canvas object it belongs to. */
const probePoint = (x, y) => ev(`${C} const el = document.elementFromPoint(${Math.round(x)}, ${Math.round(y)});
  if (!el) return { none: true };
  const chain = []; let n = el; for (let i = 0; i < 6 && n; i++, n = n.parentElement) chain.push(n.className && n.className.toString ? n.className.toString().slice(0, 60) : String(n.nodeName));
  let hit = null;
  for (const [id, node] of cv.nodes) if (node.nodeEl && node.nodeEl.contains(el)) { hit = { kind: 'node', id, type: node.getData().type }; break; }
  if (!hit) for (const [id, e] of cv.edges) { const els = [e.path && e.path.display, e.path && e.path.interaction, e.labelElement && e.labelElement.wrapperEl].filter(Boolean); if (els.some(x => x === el || x.contains(el))) { hit = { kind: 'edge', id }; break; } }
  return { tag: el.nodeName, cls: el.className && el.className.toString ? el.className.toString().slice(0, 80) : null, chain, hit };`);

/** Right-click (x, y) after recording what is there, then dump every menu. */
const rightClick = async (x, y, { expect, skipClose = false } = {}) => {
  if (!skipClose) await closeMenus();
  await c.mouseMove(x, y); await sleep(120);
  const under = await probePoint(x, y);
  await c.clickAt(x, y, { button: 'right' });
  await sleep(450);
  const menus = await dumpMenus();
  const selection = await ev(`${C} return Array.from(cv.selection).map(o => o.id);`);
  const hitWhat = under.hit ? `${under.hit.kind}:${under.hit.id}` : 'background';
  return { at: { x: Math.round(x), y: Math.round(y) }, under, selection, menus, expected: expect || null, hit: hitWhat, mismatch: expect ? hitWhat !== expect : null };
};

/** The floating icon row above the current selection. */
const floatingRow = () => ev(`
  return Array.from(document.querySelectorAll('.canvas-menu')).map(m => ({ cls: m.className, items: Array.from(m.children).map(e => ({
    cls: e.className, aria: e.getAttribute('aria-label'), text: (e.textContent || '').trim() || null,
    icon: e.querySelector('svg') ? e.querySelector('svg').getAttribute('class') : null })) }));`);

const selectOnly = async (ids) => {
  await ev(`${C} cv.deselectAll(); for (const id of ${JSON.stringify(ids)}) { const o = cv.nodes.get(id) || cv.edges.get(id); if (o) cv.select(o); } return true;`);
  await sleep(400);
};

const nodeGeom = () => ev(`${C} const g = {}; for (const [id, n] of cv.nodes) { const d = n.getData(); g[id] = { x: d.x, y: d.y, w: d.width, h: d.height }; } return g;`);

await step('env', () => ev(`return { title: document.title, ua: navigator.userAgent, apiVersion: typeof apiVersion !== 'undefined' ? apiVersion : null,
  inner: { w: window.innerWidth, h: window.innerHeight }, dpr: window.devicePixelRatio, theme: app.vault.getConfig('theme'),
  canvasOptions: JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options || {})) };`));

await step('open', async () => {
  const r = await ev(`const f = app.vault.getAbstractFileByPath('Menus.canvas'); if (!f) throw new Error('no Menus.canvas');
    const leaf = app.workspace.getLeaf(false); await leaf.openFile(f); app.workspace.setActiveLeaf(leaf, { focus: true }); return true;`);
  await sleep(1800);
  return ev(`${C} return { file: v.file && v.file.path, nodes: cv.nodes.size, edges: cv.edges.size, scale: cv.scale, readonly: !!cv.readonly, opened: ${JSON.stringify(r)} };`);
});

// ---- the card menu at the bottom of the view ------------------------------
await step('card-menu', () => ev(`${C} const m = cv.cardMenuEl; const b = m.getBoundingClientRect(); const w = cv.wrapperEl.getBoundingClientRect();
  return { cls: m.className, rect: { w: b.width, h: b.height, fromBottom: w.bottom - b.bottom, centreOffset: (b.left + b.width / 2) - (w.left + w.width / 2) },
    items: Array.from(m.children).map(e => ({ cls: e.className, aria: e.getAttribute('aria-label'), draggable: e.getAttribute('draggable'),
      icon: e.querySelector('svg') ? e.querySelector('svg').getAttribute('class') : null })) };`));

// ---- the floating row, per kind of selection ------------------------------
await step('floating-row', async () => {
  const kinds = {
    text: ['t1'], textColoured: ['c1'], fileMarkdown: ['f1'], fileImage: ['f2'], link: ['l1'],
    group: ['g1'], groupColoured: ['g2'], edgeLabelled: ['e1'], edgePlain: ['e2'],
    twoText: ['t1', 't2'], textAndFile: ['t1', 'f1'], twoGroups: ['g1', 'g2'],
    nodeAndEdge: ['t1', 'e1'], twoEdges: ['e1', 'e2'],
  };
  const res = {};
  for (const [name, ids] of Object.entries(kinds)) {
    await selectOnly(ids);
    res[name] = (await floatingRow())[0] || null;
  }
  await ev(`${C} cv.deselectAll(); return true;`);
  return res;
});

// ---- the right-click lists, one verified target at a time ----------------
await step('context-text-card', async () => { const r = await frameNode('t1'); return rightClick(r.cx, r.cy, { expect: 'node:t1' }); });
await step('context-file-card-md', async () => { const r = await frameNode('f1'); return rightClick(r.cx, r.cy, { expect: 'node:f1' }); });
await step('context-file-card-image', async () => { const r = await frameNode('f2'); return rightClick(r.cx, r.cy, { expect: 'node:f2' }); });
await step('context-link-card', async () => { const r = await frameNode('l1'); return rightClick(r.cx, r.cy, { expect: 'node:l1' }); });
await step('context-group-label', async () => { const r = await frameNode('g1'); if (!r.label) throw new Error('no group label element'); return rightClick(r.label.cx, r.label.cy, { expect: 'node:g1' }); });
await step('context-group-body', async () => { const r = await frameNode('g1'); return rightClick(r.cx, r.cy, { expect: 'node:g1' }); });
await step('context-edge-labelled', async () => { const p = await edgePoint('e1'); return rightClick(p.x, p.y, { expect: 'edge:e1' }); });
await step('context-edge-plain', async () => { const p = await edgePoint('e2'); return rightClick(p.x, p.y, { expect: 'edge:e2' }); });

await step('context-edge-labelled-offlabel', async () => {
  // The midpoint of e1 is where its label sits, and that right-click opened
  // nothing. Try a quarter along the path instead, and say whether the label
  // element was under the pointer either time.
  const p = await ev(`${C} const e = cv.edges.get('e1');
    const from = cv.nodes.get(e.from.node.id), to = cv.nodes.get(e.to.node.id);
    cv.setViewport((from.x + to.x + to.width) / 2, (from.y + from.height / 2 + to.y + to.height / 2) / 2, 0);
    await new Promise(r => setTimeout(r, 500));
    const path = e.path && (e.path.display || e.path.interaction);
    const at = (f) => { const pt = path.getPointAtLength(path.getTotalLength() * f); const sp = pt.matrixTransform(path.getScreenCTM()); return { x: sp.x, y: sp.y }; };
    const lab = e.labelElement && e.labelElement.wrapperEl ? e.labelElement.wrapperEl.getBoundingClientRect() : null;
    return { quarter: at(0.25), mid: at(0.5), label: lab ? { x: lab.x, y: lab.y, w: lab.width, h: lab.height } : null };`);
  const quarter = await rightClick(p.quarter.x, p.quarter.y, { expect: 'edge:e1' });
  const selectedFirst = await (async () => {
    // And with the edge already selected, the way probe-c reached it.
    await selectOnly(['e1']);
    return rightClick(p.mid.x, p.mid.y, { expect: 'edge:e1' });
  })();
  return { geometry: p, quarter, selectedFirst };
});

await step('context-background', async () => {
  // Pan to empty space between the clusters, then verify nothing is under the
  // pointer before clicking.
  await ev(`${C} cv.deselectAll(); cv.setViewport(-1200, -1200, 0); return true;`); await sleep(500);
  const p = await ev(`${C} const b = cv.wrapperEl.getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  return rightClick(p.x, p.y, { expect: 'background' });
});

await step('context-multi', async () => {
  await ev(`${C} cv.setViewport(430, 160, -1); return true;`); await sleep(500);
  await selectOnly(['t1', 't2']);
  const r = await ev(`${C} const b = cv.nodes.get('t1').nodeEl.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
  return rightClick(r.cx, r.cy, { expect: 'node:t1' });
});

await step('context-multi-escape', async () => {
  // Does a right-click inside a multi-selection keep it — and is the Escape
  // this harness presses to shut a previous menu what clears it? One step,
  // both orders, the selection read at every stage.
  await ev(`${C} cv.setViewport(430, 160, -1); return true;`); await sleep(500);
  const point = () => ev(`${C} const b = cv.nodes.get('t1').nodeEl.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
  const sel = () => ev(`${C} return Array.from(cv.selection).map(o => o.id);`);

  await selectOnly(['t1', 't2']);
  const beforeNoEscape = await sel();
  const noEscape = await rightClick((await point()).cx, (await point()).cy, { expect: 'node:t1', skipClose: true });

  await closeMenus();
  await selectOnly(['t1', 't2']);
  const beforeEscape = await sel();
  await c.pressKey('Escape'); await sleep(250);
  const afterEscape = await sel();
  const withEscape = await rightClick((await point()).cx, (await point()).cy, { expect: 'node:t1', skipClose: true });

  const names = (r) => r.menus.map((m) => m.items.filter((i) => i.visible).map((i) => (i.separator ? '---' : i.title)));
  return {
    noEscape: { before: beforeNoEscape, selectionAfter: noEscape.selection, menu: names(noEscape) },
    withEscape: { before: beforeEscape, afterEscape, selectionAfter: withEscape.selection, menu: names(withEscape) },
    escapeDeselects: beforeEscape.length > 0 && afterEscape.length === 0,
  };
});

await step('context-unselected-card', async () => {
  // Does a right-click select what it lands on? Select t2, right-click t1.
  await ev(`${C} cv.setViewport(430, 160, -1); return true;`); await sleep(500);
  await selectOnly(['t2']);
  const before = await ev(`${C} return Array.from(cv.selection).map(o => o.id);`);
  const r = await ev(`${C} const b = cv.nodes.get('t1').nodeEl.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
  const click = await rightClick(r.cx, r.cy, { expect: 'node:t1' });
  return { before, ...click };
});

await step('context-selection-rules', async () => {
  // The three steps above press Escape before clicking, and Escape clears the
  // canvas selection, so their "before" selections are meaningless. These ask
  // the same questions without it: the menu is dismissed by a left-click on
  // empty space instead.
  await ev(`${C} cv.setViewport(430, 160, -1); return true;`); await sleep(500);
  const sel = () => ev(`${C} return Array.from(cv.selection).map(o => o.id);`);
  const centre = (id) => ev(C + ' const b = cv.nodes.get(' + JSON.stringify(id) + ').nodeEl.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };');
  const dismiss = async () => { await ev(`for (const m of document.querySelectorAll('.menu')) m.remove(); return true;`); await sleep(150); };
  const names = (r) => r.menus.map((m) => m.items.filter((i) => i.visible).map((i) => (i.separator ? '---' : i.title)));

  // (a) t2 selected, right-click t1.
  await selectOnly(['t2']);
  const beforeA = await sel();
  const p1 = await centre('t1');
  const a = await rightClick(p1.cx, p1.cy, { expect: 'node:t1', skipClose: true });
  await dismiss();

  // (b) a card and an edge selected, right-click the card.
  await selectOnly(['t1', 'e1']);
  const beforeB = await sel();
  const b = await rightClick((await centre('t1')).cx, (await centre('t1')).cy, { expect: 'node:t1', skipClose: true });
  await dismiss();

  // (c) t1 selected, right-click the background.
  await selectOnly(['t1']);
  const beforeC = await sel();
  const bg = await ev(`${C} const w = cv.wrapperEl.getBoundingClientRect(); return { cx: w.x + 40, cy: w.y + w.height - 120 };`);
  const cc = await rightClick(bg.cx, bg.cy, { skipClose: true });
  await dismiss();

  return {
    unselectedCard: { before: beforeA, hit: a.hit, selectionAfter: a.selection, menu: names(a) },
    mixedSelection: { before: beforeB, hit: b.hit, selectionAfter: b.selection, menu: names(b) },
    backgroundWithSelection: { before: beforeC, hit: cc.hit, selectionAfter: cc.selection, menu: names(cc) },
  };
});

await step('context-mixed-selection', async () => {
  await ev(`${C} cv.setViewport(430, 160, -1); return true;`); await sleep(500);
  await selectOnly(['t1', 'e1']);
  const r = await ev(`${C} const b = cv.nodes.get('t1').nodeEl.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
  return rightClick(r.cx, r.cy, { expect: 'node:t1' });
});

// ---- the submenus --------------------------------------------------------
await step('submenus', async () => {
  const res = {};
  const open = async (ids, aria) => {
    await closeMenus();
    await selectOnly(ids);
    const r = await ev(`const b = Array.from(document.querySelectorAll('.canvas-menu .clickable-icon')).find(e => e.getAttribute('aria-label') === ${JSON.stringify(aria)});
      if (!b) return null; const x = b.getBoundingClientRect(); return { cx: x.x + x.width / 2, cy: x.y + x.height / 2 };`);
    if (!r) return { missing: aria };
    await c.clickAt(r.cx, r.cy); await sleep(450);
    const menus = await dumpMenus();
    const pickers = await ev(`return Array.from(document.querySelectorAll('.canvas-color-picker-item')).map(e => ({ cls: e.className, aria: e.getAttribute('aria-label'), bg: getComputedStyle(e).backgroundColor }));`);
    // Two of these buttons open no menu at all: one opens a modal, one starts
    // editing in place. Record both, or the step just says "null".
    const other = await ev(`const m = document.querySelector('.modal-container');
      const a = document.activeElement;
      return { modal: m ? { title: (m.querySelector('.modal-title') || {}).textContent || null,
          text: m.textContent.trim().slice(0, 300),
          inputs: Array.from(m.querySelectorAll('input, textarea, [contenteditable="true"]')).map(i => ({ tag: i.nodeName, cls: i.className, value: i.value !== undefined ? i.value : i.textContent, placeholder: i.placeholder || null })),
          buttons: Array.from(m.querySelectorAll('button, .mod-cta')).map(b => b.textContent.trim()),
          suggestions: Array.from(m.querySelectorAll('.suggestion-item')).slice(0, 8).map(s => s.textContent.trim()) } : null,
        focus: a ? { tag: a.nodeName, cls: a.className && a.className.toString ? a.className.toString().slice(0, 80) : null, editable: a.isContentEditable, text: (a.textContent || '').slice(0, 60) } : null };`);
    if (other.modal) { await c.pressKey('Escape'); await sleep(250); }
    await closeMenus();
    return { menus, pickers, ...other };
  };
  res.setColorCard = await open(['t1'], 'Set color');
  res.setColorEdge = await open(['e1'], 'Set color');
  res.lineDirection = await open(['e1'], 'Line direction');
  res.alignTwo = await open(['a1', 'a2'], 'Align');
  res.alignThree = await open(['a1', 'a2', 'a3'], 'Align');
  res.alignGroup = await open(['g1'], 'Align');
  res.setBackground = await open(['g1'], 'Set background');
  res.editLabelEdge = await open(['e1'], 'Edit label');
  res.editLabelGroup = await open(['g1'], 'Edit label');
  await closeMenus();
  await ev(`${C} cv.deselectAll(); return true;`);
  return res;
});

await step('quick-settings', async () => {
  await closeMenus();
  const r = await ev(`${C} const b = cv.quickSettingsButton; if (!b) return null; const x = b.getBoundingClientRect(); return { cx: x.x + x.width / 2, cy: x.y + x.height / 2 };`);
  if (!r) throw new Error('no quick settings button');
  await c.clickAt(r.cx, r.cy); await sleep(450);
  const menus = await dumpMenus();
  const options = await ev(`return JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options || {}));`);
  await closeMenus();
  return { menus, options };
});

await step('snap-persistence', async () => {
  // Toggle "Snap to grid" and look for where it is kept: the plugin's options,
  // the canvas file, .obsidian, localStorage.
  const before = { options: await ev(`return JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options || {}));`), canvas: bytes('Menus.canvas'), dir: fs.readdirSync(vp('.obsidian')).sort(), canvasJson: bytes('.obsidian/canvas.json') };
  const r = await ev(`${C} const b = cv.quickSettingsButton; const x = b.getBoundingClientRect(); return { cx: x.x + x.width / 2, cy: x.y + x.height / 2 };`);
  await c.clickAt(r.cx, r.cy); await sleep(400);
  const row = await ev(`const items = Array.from(document.querySelectorAll('.menu .menu-item'));
    const it = items.find(i => (i.textContent || '').trim().startsWith('Snap to grid')); if (!it) return null;
    const b = it.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2, text: it.textContent.trim(), cls: it.className };`);
  if (!row) { await closeMenus(); throw new Error('no Snap to grid row'); }
  await c.clickAt(row.cx, row.cy); await sleep(900);
  const after = { options: await ev(`return JSON.parse(JSON.stringify(app.internalPlugins.plugins.canvas.instance.options || {}));`), canvas: bytes('Menus.canvas'), dir: fs.readdirSync(vp('.obsidian')).sort(), canvasJson: bytes('.obsidian/canvas.json'), snapping: await ev(`${C} return { snapToGrid: cv.snapToGrid, snapToObjects: cv.snapToObjects, readonly: !!cv.readonly };`) };
  // Put it back.
  await closeMenus();
  const r2 = await ev(`${C} const b = cv.quickSettingsButton; const x = b.getBoundingClientRect(); return { cx: x.x + x.width / 2, cy: x.y + x.height / 2 };`);
  await c.clickAt(r2.cx, r2.cy); await sleep(400);
  const row2 = await ev(`const items = Array.from(document.querySelectorAll('.menu .menu-item'));
    const it = items.find(i => (i.textContent || '').trim().startsWith('Snap to grid')); if (!it) return null;
    const b = it.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2, text: it.textContent.trim() };`);
  if (row2) { await c.clickAt(row2.cx, row2.cy); await sleep(600); }
  await closeMenus();
  return { row, before: { ...before, canvas: undefined, canvasLen: before.canvas && before.canvas.length }, after: { ...after, canvas: undefined, canvasLen: after.canvas && after.canvas.length }, canvasChanged: before.canvas !== after.canvas, restored: row2 };
});

await step('readonly-toggle', async () => {
  const r = await ev(`${C} const b = cv.quickSettingsButton; const x = b.getBoundingClientRect(); return { cx: x.x + x.width / 2, cy: x.y + x.height / 2 };`);
  await c.clickAt(r.cx, r.cy); await sleep(400);
  const row = await ev(`const items = Array.from(document.querySelectorAll('.menu .menu-item'));
    const it = items.find(i => (i.textContent || '').trim().startsWith('Read-only')); if (!it) return null;
    const b = it.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2, text: it.textContent.trim(), cls: it.className };`);
  if (!row) { await closeMenus(); throw new Error('no Read-only row'); }
  await c.clickAt(row.cx, row.cy); await sleep(800);
  const on = await ev(`${C} return { readonly: !!cv.readonly, wrapperCls: cv.wrapperEl.className, cardMenuVisible: !!(cv.cardMenuEl && cv.cardMenuEl.offsetParent), canvasCls: cv.canvasEl ? cv.canvasEl.className : null };`);
  await selectOnly(['t1']);
  const rowWhileReadonly = await floatingRow();
  const ctxWhileReadonly = await (async () => { const b = await frameNode('t1'); return rightClick(b.cx, b.cy, { expect: 'node:t1' }); })();
  // Back off.
  await closeMenus();
  const r2 = await ev(`${C} const b = cv.quickSettingsButton; const x = b.getBoundingClientRect(); return { cx: x.x + x.width / 2, cy: x.y + x.height / 2 };`);
  await c.clickAt(r2.cx, r2.cy); await sleep(400);
  const row2 = await ev(`const items = Array.from(document.querySelectorAll('.menu .menu-item'));
    const it = items.find(i => (i.textContent || '').trim().startsWith('Read-only')); if (!it) return null;
    const b = it.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
  if (row2) { await c.clickAt(row2.cx, row2.cy); await sleep(600); }
  await closeMenus();
  const off = await ev(`${C} return { readonly: !!cv.readonly, wrapperCls: cv.wrapperEl.className };`);
  return { row, on, rowWhileReadonly, ctxWhileReadonly, off };
});

// ---- Align and Arrange, applied, so the geometry is recorded -------------
await step('align-geometry', async () => {
  const res = {};
  const items = await (async () => { await selectOnly(['a1', 'a2', 'a3']); return (await floatingRow())[0]; })();
  res.rowItems = items;
  const apply = async (label) => {
    await closeMenus();
    await selectOnly(['a1', 'a2', 'a3']);
    const before = await nodeGeom();
    const b = await ev(`const btn = Array.from(document.querySelectorAll('.canvas-menu .clickable-icon')).find(e => e.getAttribute('aria-label') === 'Align');
      if (!btn) return null; const x = btn.getBoundingClientRect(); return { cx: x.x + x.width / 2, cy: x.y + x.height / 2 };`);
    if (!b) return { missing: 'Align' };
    await c.clickAt(b.cx, b.cy); await sleep(400);
    const row = await ev(`const items = Array.from(document.querySelectorAll('.menu .menu-item'));
      const it = items.find(i => ((i.querySelector('.menu-item-title') || i).textContent || '').trim() === ${JSON.stringify(label)});
      if (!it) return null; const r = it.getBoundingClientRect(); return { cx: r.x + r.width / 2, cy: r.y + r.height / 2 };`);
    if (!row) { await closeMenus(); return { missing: label }; }
    await c.clickAt(row.cx, row.cy); await sleep(700);
    const after = await nodeGeom();
    const moved = {};
    for (const id of ['a1', 'a2', 'a3']) moved[id] = { before: before[id], after: after[id] };
    // Undo, so each case starts from the same ragged layout.
    await ev(`${C} cv.undo(); return true;`); await sleep(700);
    const restored = await nodeGeom();
    const back = ['a1', 'a2', 'a3'].every((id) => JSON.stringify(restored[id]) === JSON.stringify(before[id]));
    return { moved, undoRestored: back };
  };
  for (const label of ['Align left', 'Align center', 'Align right', 'Align top', 'Align middle', 'Align bottom',
    'Arrange in a row', 'Arrange in a column', 'Arrange in a grid',
    'Distribute horizontal spacing', 'Distribute vertical spacing', 'Justify horizontally', 'Justify vertically']) {
    res[label] = await apply(label);
  }
  await ev(`${C} cv.deselectAll(); return true;`);
  return res;
});

// ---- is "bring to front" anywhere? --------------------------------------
await step('front-and-back', async () => {
  const hunt = async (ids, point) => {
    await selectOnly(ids);
    const r = point ? point : await (async () => { const b = await frameNode(ids[0]); return { cx: b.cx, cy: b.cy }; })();
    const click = await rightClick(r.cx, r.cy);
    const labels = click.menus.flatMap((m) => m.items.map((i) => i.title || i.text || ''));
    await closeMenus();
    return { labels, matches: labels.filter((l) => /front|back|raise|lower|order|layer/i.test(l)) };
  };
  return { card: await hunt(['t1']), group: await hunt(['g1']), twoCards: await hunt(['t1', 't2']) };
});

// ---- Convert to file, which writes bytes --------------------------------
await step('convert-to-file', async () => {
  await closeMenus();
  const before = { files: fs.readdirSync(VAULT).sort(), canvas: bytes('Menus.canvas') };
  // Start from one card: an inherited multi-selection answers with the
  // multi-selection menu, which has no Convert to file row.
  await selectOnly(['t1']);
  const r = await frameNode('t1');
  const click = await rightClick(r.cx, r.cy, { expect: 'node:t1' });
  const row = await ev(`const items = Array.from(document.querySelectorAll('.menu .menu-item'));
    const it = items.find(i => ((i.querySelector('.menu-item-title') || i).textContent || '').trim().startsWith('Convert to file'));
    if (!it) return null; const b = it.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2, text: it.textContent.trim() };`);
  if (!row) { await closeMenus(); return { click, missing: 'Convert to file' }; }
  await c.clickAt(row.cx, row.cy); await sleep(900);
  const modal = await ev(`const m = document.querySelector('.modal-container'); if (!m) return null;
    return { title: (m.querySelector('.modal-title') || {}).textContent || null, html: m.innerHTML.slice(0, 1200),
      inputs: Array.from(m.querySelectorAll('input, textarea, [contenteditable="true"]')).map(i => ({ tag: i.nodeName, cls: i.className, value: i.value !== undefined ? i.value : i.textContent, placeholder: i.placeholder || null })),
      buttons: Array.from(m.querySelectorAll('button')).map(b => b.textContent.trim()), text: m.textContent.trim().slice(0, 400) };`);
  // Enter does not submit this modal (the 2026-10-25 first run pressed it and
  // nothing was written), so press Save.
  let accepted = null;
  if (modal) {
    const enter = await (async () => { await c.pressKey('Enter'); await sleep(800); return { files: fs.readdirSync(VAULT).sort(), stillOpen: await ev(`return !!document.querySelector('.modal-container');`) }; })();
    const save = await ev(`const m = document.querySelector('.modal-container'); if (!m) return null;
      const b = Array.from(m.querySelectorAll('button, .mod-cta')).find(x => x.textContent.trim() === 'Save'); if (!b) return null;
      const r = b.getBoundingClientRect(); return { cx: r.x + r.width / 2, cy: r.y + r.height / 2 };`);
    if (save) { await c.clickAt(save.cx, save.cy); await sleep(1600); }
    accepted = { enter, save, modalGone: await ev(`return !document.querySelector('.modal-container');`) };
  }
  const nodeAfter = await ev(`${C} return { ids: Array.from(cv.nodes.keys()), t1: cv.nodes.has('t1') ? cv.nodes.get('t1').getData() : null,
    files: Array.from(cv.nodes.values()).map(n => n.getData()).filter(d => d.type === 'file'), dirty: !!cv.dirty };`);
  // The save is debounced; the first read of the file came back unchanged.
  let after = { files: fs.readdirSync(VAULT).sort(), canvas: bytes('Menus.canvas') };
  for (let i = 0; i < 20 && after.canvas === before.canvas; i++) { await sleep(500); after = { files: fs.readdirSync(VAULT).sort(), canvas: bytes('Menus.canvas') }; }
  const created = after.files.filter((f) => !before.files.includes(f));
  return { row, modal, accepted, created, createdBytes: created.length ? bytes(created[0]) : null, nodeAfter,
    canvasChanged: before.canvas !== after.canvas, canvasAfter: after.canvas, canvasBefore: before.canvas };
});

await step('final-state', async () => {
  await closeMenus();
  return ev(`${C} return { nodes: cv.nodes.size, edges: cv.edges.size, dirty: !!cv.dirty, readonly: !!cv.readonly, canvasFileOnDisk: v.file && v.file.path };`);
});

c.close();
save();
console.error('[canvas-menus] done');
process.exit(0);
