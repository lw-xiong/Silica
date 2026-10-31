// Canvas view probe, part B: input events.
import fs from 'node:fs';
import { connect, sleep } from '/home/ragtux/bstore2/raga/repos/silica/tools/obsidian-probe/cdp.mjs';

const OUT = process.env.OUT;
const out = {};
const save = () => fs.writeFileSync(OUT, JSON.stringify(out, null, 1));
const c = await connect();
const ev = (s) => c.evaluate(s);
async function step(name, fn) {
  try { out[name] = await fn(); } catch (e) { out[name] = { error: String(e && e.message || e) }; }
  save();
  console.log(name, JSON.stringify(out[name]).slice(0, 400));
}
const C = `const v = app.workspace.getLeavesOfType('canvas')[0].view; const c = v.canvas;`;
// screen point of a canvas point
const scr = (x, y) => ev(`${C} const r = c.canvasRect; return { x: r.left + (${x} - c.x) * c.scale + r.width / 2, y: r.top + (${y} - c.y) * c.scale + r.height / 2 };`);
const nodeInfo = (id) => ev(`${C} const n = c.nodes.get('${id}'); return n ? { x: n.x, y: n.y, w: n.width, h: n.height, editing: !!n.isEditing, sel: c.selection.has(n) } : null;`);
const reset = async () => { await ev(`${C} c.deselectAll(); c.setViewport(700, 300, 0); return true;`); await sleep(400); };
const wheel = async (x, y, dx, dy, mods = 0) => c.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: Math.round(x), y: Math.round(y), deltaX: dx, deltaY: dy, modifiers: mods });

await step('config', () => ev(`${C} return { config: JSON.parse(JSON.stringify(c.config)), options: app.internalPlugins.plugins.canvas.instance.options, snapping: c.getSnapping ? c.getSnapping() : null, canSnap: c.canSnap && c.canSnap.length };`));

await step('threshold-search', async () => {
  let lo = -1.7, hi = -1.5;
  for (let i = 0; i < 10; i++) {
    const mid = (lo + hi) / 2;
    await ev(`${C} c.setViewport(700, 300, ${mid}); return true;`); await sleep(250);
    const zoomedOut = await ev(`${C} return c.wrapperEl.classList.contains('mod-zoomed-out');`);
    if (zoomedOut) lo = mid; else hi = mid;
  }
  return { zoomBetween: [lo, hi], scaleBetween: [Math.pow(2, lo), Math.pow(2, hi)] };
});

await reset();
await step('wheel', async () => {
  const center = await scr(700, 300);
  const before = await ev(`${C} return { x: c.x, y: c.y, zoom: c.zoom };`);
  await wheel(center.x, center.y, 0, 100); await sleep(400);
  const plain = await ev(`${C} return { x: c.x, y: c.y, zoom: c.zoom, tx: c.tx, ty: c.ty };`);
  await wheel(center.x, center.y, 0, 100, 8); await sleep(400);
  const shift = await ev(`${C} return { x: c.x, y: c.y, zoom: c.zoom, tx: c.tx, ty: c.ty };`);
  await wheel(center.x, center.y, 0, 100, 2); await sleep(600);
  const ctrl = await ev(`${C} return { x: c.x, y: c.y, zoom: c.zoom, tZoom: c.tZoom };`);
  await wheel(center.x + 200, center.y, 0, -100, 2); await sleep(600);
  const ctrlUpOff = await ev(`${C} return { x: c.x, y: c.y, zoom: c.zoom, tZoom: c.tZoom };`);
  await wheel(center.x, center.y, 100, 0); await sleep(400);
  const horiz = await ev(`${C} return { x: c.x, y: c.y, zoom: c.zoom };`);
  return { before, plain, shift, ctrl, ctrlUpOff, horiz };
});

await reset();
await step('drag-snap', async () => {
  const start = await scr(840, 30); // middle of t3 (760..920, 0..60)
  const a = await nodeInfo('t3');
  await c.drag(start.x, start.y, start.x + 33, start.y + 17, { steps: 10 }); await sleep(400);
  const after = await nodeInfo('t3');
  // alt-drag back
  await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: Math.round(start.x + 33), y: Math.round(start.y + 17), modifiers: 1 });
  await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: Math.round(start.x + 33), y: Math.round(start.y + 17), button: 'left', clickCount: 1, modifiers: 1 });
  for (let i = 1; i <= 10; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: Math.round(start.x + 33 - 3.3 * i), y: Math.round(start.y + 17 - 1.7 * i), button: 'left', modifiers: 1 }); await sleep(30); }
  await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: Math.round(start.x), y: Math.round(start.y), button: 'left', clickCount: 1, modifiers: 1 });
  await sleep(400);
  const afterAlt = await nodeInfo('t3');
  await ev(`${C} c.nodes.get('t3').moveTo({ x: 760, y: 0 }); return true;`);
  return { before: a, after, afterAlt };
});

await reset();
await step('marquee', async () => {
  // t3 at 760..920 x 0..60; t4 at 980..1140. Drag from empty (700,-100) to (850, 100): covers half of t3
  const p1 = await scr(700, -100); const p2 = await scr(850, 100);
  await c.drag(p1.x, p1.y, p2.x, p2.y, { steps: 10 }); await sleep(300);
  const partial = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  await ev(`${C} c.deselectAll(); return true;`);
  const p3 = await scr(1160, 100);
  await c.drag(p1.x, p1.y, p3.x, p3.y, { steps: 10 }); await sleep(300);
  const full = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  // what did the drag do to the viewport?
  const vp = await ev(`${C} return { x: c.x, y: c.y };`);
  await ev(`${C} c.deselectAll(); return true;`);
  return { partial, full, viewportAfterDrags: vp };
});

await reset();
await step('group-click-and-drag', async () => {
  // g1 spans -60..1440 x -120..200; empty spot inside g1 at (700,-80)? t2 is 540..700 x 0..60; use (850,-90)
  const inside = await scr(850, -90);
  await c.clickAt(inside.x, inside.y); await sleep(300);
  const clickSel = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  await ev(`${C} c.deselectAll(); return true;`);
  const before = await ev(`${C} return Object.fromEntries(Array.from(c.nodes.values()).map(n => [n.id, [n.x, n.y]]));`);
  // drag the group by its body from (850,-90) by +40,+40 (screen px at scale 1)
  await c.drag(inside.x, inside.y, inside.x + 40, inside.y + 40, { steps: 10 }); await sleep(400);
  const after = await ev(`${C} return Object.fromEntries(Array.from(c.nodes.values()).map(n => [n.id, [n.x, n.y]]));`);
  const moved = Object.keys(after).filter(k => before[k][0] !== after[k][0] || before[k][1] !== after[k][1]);
  const sel = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  const undone = await ev(`${C} return Object.fromEntries(Array.from(c.nodes.values()).map(n => [n.id, [n.x, n.y]]));`);
  // drag by label
  const lbl = await ev(`${C} const r = c.nodes.get('g1').nodeEl.querySelector('.canvas-group-label').getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  await c.drag(lbl.x, lbl.y, lbl.x + 40, lbl.y + 40, { steps: 10 }); await sleep(400);
  const afterLabel = await ev(`${C} return Object.fromEntries(Array.from(c.nodes.values()).map(n => [n.id, [n.x, n.y]]));`);
  const movedLabel = Object.keys(afterLabel).filter(k => before[k][0] !== afterLabel[k][0] || before[k][1] !== afterLabel[k][1]);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  return { clickSel, moved, sel, undoRestored: JSON.stringify(undone) === JSON.stringify(before), movedLabel, g1Before: before.g1, g1After: after.g1 };
});

await reset();
await step('delete-group', async () => {
  const lbl = await ev(`${C} const r = c.nodes.get('g1').nodeEl.querySelector('.canvas-group-label').getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  await c.clickAt(lbl.x, lbl.y); await sleep(300);
  const sel = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  const before = await ev(`${C} return Array.from(c.nodes.keys());`);
  await c.pressKey('Delete'); await sleep(400);
  const after = await ev(`${C} return Array.from(c.nodes.keys());`);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  const restored = await ev(`${C} return Array.from(c.nodes.keys()).length;`);
  return { sel, removed: before.filter(k => !after.includes(k)), restored };
});

await reset();
await step('dblclick-create', async () => {
  const p = await scr(700, 500); // empty? f2 is 500..820 x 300..540 -> inside g2 group body. Use (1700, 500) outside everything
  const q = await scr(1200, 650); // inside g2 (260..820 y) but empty
  await c.clickAt(q.x, q.y, { clickCount: 1 }); await c.clickAt(q.x, q.y, { clickCount: 2 }); await sleep(600);
  const info = await ev(`${C} const n = Array.from(c.nodes.values()).find(n => !['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1'].includes(n.id)); return n ? { id: n.id, x: n.x, y: n.y, w: n.width, h: n.height, editing: !!n.isEditing, data: n.getData(), sel: c.selection.has(n), cls: n.nodeEl.className, active: document.activeElement.className.slice(0, 80) } : { none: true, count: c.nodes.size };`);
  await c.typeText('Hi there'); await sleep(200);
  const typed = await ev(`${C} const n = c.nodes.get('${info.id}'); return n && { text: n.text, data: n.getData(), editing: !!n.isEditing, h: n.height };`);
  await c.pressKey('Escape'); await sleep(300);
  const afterEsc = await ev(`${C} const n = c.nodes.get('${info.id}'); return n && { text: n.text, editing: !!n.isEditing, sel: c.selection.has(n), selSize: c.selection.size, active: document.activeElement.className.slice(0, 80) };`);
  await c.pressKey('Escape'); await sleep(200);
  const afterEsc2 = await ev(`${C} return { selSize: c.selection.size };`);
  // Enter on a selected card
  await ev(`${C} c.selectOnly(c.nodes.get('${info.id}')); return true;`); await sleep(200);
  await c.pressKey('Enter'); await sleep(300);
  const afterEnter = await ev(`${C} const n = c.nodes.get('${info.id}'); return { editing: !!n.isEditing };`);
  await c.pressKey('Escape'); await sleep(200);
  // dblclick on the card to edit
  const cp = await scr(info.x + 50, info.y + 20);
  await c.clickAt(cp.x, cp.y, { clickCount: 1 }); await c.clickAt(cp.x, cp.y, { clickCount: 2 }); await sleep(400);
  const afterDbl = await ev(`${C} const n = c.nodes.get('${info.id}'); return { editing: !!n.isEditing };`);
  // click outside ends editing?
  const o = await scr(1200, 750);
  await c.clickAt(o.x, o.y); await sleep(300);
  const afterOutside = await ev(`${C} const n = c.nodes.get('${info.id}'); return { editing: !!n.isEditing, sel: c.selection.size };`);
  // empty the text and leave: what is written
  await ev(`${C} c.selectOnly(c.nodes.get('${info.id}')); return true;`); await c.pressKey('Enter'); await sleep(300);
  await c.pressKey('a', { ctrl: true }); await c.pressKey('Backspace'); await sleep(200); await c.pressKey('Escape'); await sleep(300);
  const emptied = await ev(`${C} const n = c.nodes.get('${info.id}'); return n ? { text: n.text, data: n.getData(), exists: true } : { exists: false };`);
  await ev(`${C} const n = c.nodes.get('${info.id}'); if (n) c.removeNode(n); c.requestSave(); return true;`);
  return { created: info, typed, afterEsc, afterEsc2, afterEnter, afterDbl, afterOutside, emptied };
});

await reset();
await step('context-menus', async () => {
  const res = {};
  const cardP = await scr(840, 30);
  await c.clickAt(cardP.x, cardP.y, { button: 'right' }); await sleep(400);
  res.card = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  const fileP = await scr(200, 500);
  await c.clickAt(fileP.x, fileP.y, { button: 'right' }); await sleep(400);
  res.fileCard = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  const edgeP = await scr(290, 55); // e1 midpoint (label there)
  await c.clickAt(edgeP.x, edgeP.y, { button: 'right' }); await sleep(400);
  res.edge = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  const e3P = await scr(1170, 30);
  await c.clickAt(e3P.x, e3P.y, { button: 'right' }); await sleep(400);
  res.edgeNoLabel = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  const bgP = await scr(1200, 650);
  await c.clickAt(bgP.x, bgP.y, { button: 'right' }); await sleep(400);
  res.background = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  const grpP = await scr(850, -90);
  await c.clickAt(grpP.x, grpP.y, { button: 'right' }); await sleep(400);
  res.groupBody = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  await ev(`${C} c.deselectAll(); c.select(c.nodes.get('t3')); c.select(c.nodes.get('t4')); return true;`); await sleep(200);
  await c.clickAt(cardP.x, cardP.y, { button: 'right' }); await sleep(400);
  res.multi = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  await ev(`${C} c.deselectAll(); return true;`);
  // ctrl+drag on the background
  const d1 = await scr(1500, 500); const d2 = await scr(1700, 650);
  await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: Math.round(d1.x), y: Math.round(d1.y), modifiers: 2 });
  await c.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: Math.round(d1.x), y: Math.round(d1.y), button: 'left', clickCount: 1, modifiers: 2 });
  for (let i = 1; i <= 8; i++) { await c.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: Math.round(d1.x + (d2.x - d1.x) * i / 8), y: Math.round(d1.y + (d2.y - d1.y) * i / 8), button: 'left', modifiers: 2 }); await sleep(30); }
  await c.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: Math.round(d2.x), y: Math.round(d2.y), button: 'left', clickCount: 1, modifiers: 2 });
  await sleep(500);
  res.ctrlDrag = await c.openMenuItems(); res.ctrlDragNodes = await ev(`${C} return c.nodes.size;`); await c.pressKey('Escape'); await sleep(200);
  // the "Set color" submenu of a card
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); return true;`); await sleep(300);
  res.setColor = await ev(`${C} const b = Array.from(document.querySelectorAll('.canvas-menu .clickable-icon')).find(e => e.getAttribute('aria-label') === 'Set color'); if (!b) return null; b.click(); await new Promise(r => setTimeout(r, 300)); const m = document.querySelector('.menu'); const items = m ? Array.from(m.querySelectorAll('.menu-item')).map(i => ({ title: (i.querySelector('.menu-item-title') || i).textContent.trim(), cls: i.className, color: i.querySelector('.canvas-color-picker-item') ? getComputedStyle(i.querySelector('.canvas-color-picker-item')).backgroundColor : null })) : null; const picker = Array.from(document.querySelectorAll('.canvas-color-picker-item')).map(e => ({ cls: e.className, aria: e.getAttribute('aria-label'), bg: getComputedStyle(e).backgroundColor })); return { items, picker, menuHtml: (document.querySelector('.menu') || document.querySelector('.canvas-color-picker-wrapper') || {}).outerHTML?.slice(0, 800) };`);
  await c.pressKey('Escape'); await sleep(200);
  res.lineDirection = await ev(`${C} c.selectOnly(c.edges.get('e1')); await new Promise(r => setTimeout(r, 300)); const b = Array.from(document.querySelectorAll('.canvas-menu .clickable-icon')).find(e => e.getAttribute('aria-label') === 'Line direction'); if (!b) return null; b.click(); await new Promise(r => setTimeout(r, 300)); const m = document.querySelector('.menu'); return m ? Array.from(m.querySelectorAll('.menu-item')).map(i => ({ title: (i.querySelector('.menu-item-title') || i).textContent.trim(), checked: i.className })) : null;`);
  await c.pressKey('Escape'); await sleep(200);
  res.align = await ev(`${C} c.deselectAll(); c.select(c.nodes.get('t3')); c.select(c.nodes.get('t4')); await new Promise(r => setTimeout(r, 300)); const b = Array.from(document.querySelectorAll('.canvas-menu .clickable-icon')).find(e => e.getAttribute('aria-label') === 'Align'); if (!b) return null; b.click(); await new Promise(r => setTimeout(r, 300)); const m = document.querySelector('.menu'); return m ? Array.from(m.querySelectorAll('.menu-item')).map(i => (i.querySelector('.menu-item-title') || i).textContent.trim()) : null;`);
  await c.pressKey('Escape'); await sleep(200);
  res.quickSettings = await ev(`${C} c.quickSettingsButton.click(); await new Promise(r => setTimeout(r, 300)); const m = document.querySelector('.menu'); return m ? Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map(i => i.classList.contains('menu-separator') ? '---' : ((i.querySelector('.menu-item-title') || i).textContent.trim() + (i.classList.contains('mod-checked') || i.querySelector('.menu-item-checked') ? ' [x]' : ''))) : null;`);
  await c.pressKey('Escape'); await sleep(200);
  res.viewMore = await ev(`${C} v.moreOptionsButtonEl.click(); await new Promise(r => setTimeout(r, 300)); const m = document.querySelector('.menu'); return m ? Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map(i => i.classList.contains('menu-separator') ? '---' : (i.querySelector('.menu-item-title') || i).textContent.trim()) : null;`);
  await c.pressKey('Escape'); await sleep(200);
  await ev(`${C} c.deselectAll(); return true;`);
  return res;
});

await reset();
await step('hover-resize-connect', async () => {
  const mid = await scr(840, 30);
  await c.mouseMove(mid.x, mid.y); await sleep(400);
  const hover = await ev(`${C} const n = c.nodes.get('t3'); const el = n.nodeEl; const cs = (e) => { const s = getComputedStyle(e); return { w: s.width, h: s.height, bg: s.backgroundColor, radius: s.borderRadius, border: s.borderWidth + ' ' + s.borderColor, top: s.top, left: s.left, right: s.right, bottom: s.bottom, transform: s.transform, cursor: s.cursor, opacity: s.opacity, display: s.display }; };
    const layer = c.nodeInteractionLayer; const lel = layer && (layer.interactionEl || layer.el || layer.containerEl);
    const all = (root, sel) => Array.from((root || document).querySelectorAll(sel)).map(e => ({ cls: e.className, style: cs(e), rect: (() => { const r = e.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; })() }));
    const nr = el.getBoundingClientRect();
    return { nodeRect: [nr.x, nr.y, nr.width, nr.height], layerKeys: layer && Object.keys(layer), resizers: all(null, '.canvas-node-resizer'), connections: all(null, '.canvas-node-connection-point'), cls: el.className, hoverCls: lel && lel.className };`);
  // drag the bottom-right resizer far up-left
  const br = hover.resizers.find(r => /bottom.*right|right.*bottom|se/.test(r.cls)) || hover.resizers[hover.resizers.length - 1];
  let resized = null;
  if (br) {
    const [x, y, w, h] = br.rect;
    await c.drag(x + w / 2, y + h / 2, x + w / 2 - 300, y + h / 2 - 300, { steps: 10 }); await sleep(400);
    resized = await nodeInfo('t3');
    await ev(`${C} c.undo(); return true;`); await sleep(300);
    // small resize by +13,+7 for snapping of size
    await c.mouseMove(mid.x, mid.y); await sleep(300);
    const br2 = await ev(`${C} const e = Array.from(document.querySelectorAll('.canvas-node-resizer')).find(e => /${br.cls.split(' ').pop()}/.test(e.className)); const r = e.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];`);
    await c.drag(br2[0], br2[1], br2[0] + 13, br2[1] + 7, { steps: 8 }); await sleep(400);
    resized = { min: resized, small: await nodeInfo('t3') };
    await ev(`${C} c.undo(); return true;`); await sleep(300);
  }
  // drag from the right connection point to empty space
  await c.mouseMove(mid.x, mid.y); await sleep(300);
  const right = await ev(`${C} const pts = Array.from(document.querySelectorAll('.canvas-node-connection-point')); const nr = c.nodes.get('t3').nodeEl.getBoundingClientRect(); const e = pts.map(e => ({ e, r: e.getBoundingClientRect() })).sort((a, b) => b.r.x - a.r.x)[0]; return e ? { x: e.r.x + e.r.width / 2, y: e.r.y + e.r.height / 2, cls: e.e.className, n: pts.length } : null;`);
  let connect = null;
  if (right) {
    const before = await ev(`${C} return { nodes: c.nodes.size, edges: c.edges.size };`);
    const target = await scr(1300, 180);
    await c.drag(right.x, right.y, target.x, target.y, { steps: 12 }); await sleep(600);
    connect = await ev(`${C} const known = ['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1','e1','e2','e3','e4','e5','e6','e7']; const n = Array.from(c.nodes.values()).find(n => !known.includes(n.id)); const e = Array.from(c.edges.values()).find(e => !known.includes(e.id)); return { before: ${JSON.stringify(null)}, nodes: c.nodes.size, edges: c.edges.size, newNode: n && { x: n.x, y: n.y, w: n.width, h: n.height, editing: !!n.isEditing, data: n.getData() }, newEdge: e && e.getData(), drop: [1300, 180], menu: Array.from(document.querySelectorAll('.menu .menu-item')).map(i => (i.querySelector('.menu-item-title') || i).textContent.trim()) };`);
    connect.before = before;
    await c.pressKey('Escape'); await sleep(200);
    await ev(`${C} c.undo(); return true;`); await sleep(300);
    // drag onto another card: middle of t4 (980..1140 x 0..60): drop at (1000, 10) near the top-left
    await c.mouseMove(mid.x, mid.y); await sleep(300);
    const r2 = await ev(`${C} const pts = Array.from(document.querySelectorAll('.canvas-node-connection-point')); const e = pts.map(e => ({ e, r: e.getBoundingClientRect() })).sort((a, b) => b.r.x - a.r.x)[0]; return e ? { x: e.r.x + e.r.width / 2, y: e.r.y + e.r.height / 2 } : null;`);
    const t4 = await scr(1000, 8);
    await c.drag(r2.x, r2.y, t4.x, t4.y, { steps: 12 }); await sleep(500);
    connect.ontoCard = await ev(`${C} const known = ['e1','e2','e3','e4','e5','e6','e7']; const e = Array.from(c.edges.values()).find(e => !known.includes(e.id)); return e && e.getData();`);
    await ev(`${C} c.undo(); return true;`); await sleep(300);
  }
  return { hover, resized, right, connect };
});

await reset();
await step('arrows-nudge', async () => {
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); return true;`); await sleep(200);
  const a = await nodeInfo('t3');
  await c.pressKey('ArrowRight'); await sleep(200);
  const b = await nodeInfo('t3');
  await c.pressKey('ArrowDown', { shift: true }); await sleep(200);
  const d = await nodeInfo('t3');
  await ev(`${C} c.nodes.get('t3').moveTo({ x: 760, y: 0 }); c.deselectAll(); return true;`);
  return { a, right: b, shiftDown: d };
});

await step('space-and-middle-pan', async () => {
  await reset();
  const p = await scr(1200, 650);
  const before = await ev(`${C} return { x: c.x, y: c.y };`);
  await c.drag(p.x, p.y, p.x + 50, p.y + 30, { steps: 6, button: 'middle' }); await sleep(300);
  const middle = await ev(`${C} return { x: c.x, y: c.y, sel: c.selection.size };`);
  await reset();
  await c.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 });
  await sleep(100);
  await c.drag(p.x, p.y, p.x + 50, p.y + 30, { steps: 6 });
  await c.send('Input.dispatchKeyEvent', { type: 'keyUp', key: ' ', code: 'Space', windowsVirtualKeyCode: 32 });
  await sleep(300);
  const space = await ev(`${C} return { x: c.x, y: c.y, sel: c.selection.size, holding: c.isHoldingSpace };`);
  return { before, middle, space };
});

await step('clipboard', async () => {
  await reset();
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); c.select(c.nodes.get('t4')); return true;`); await sleep(200);
  await c.pressKey('c', { ctrl: true }); await sleep(300);
  const text = await c.readClipboard();
  await ev(`${C} c.deselectAll(); return true;`);
  const n0 = await ev(`${C} return c.nodes.size;`);
  await c.pressKey('v', { ctrl: true }); await sleep(500);
  const pasted = await ev(`${C} const known = ['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1']; return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !known.includes(n.id)).map(n => n.getData()), sel: Array.from(c.selection).map(n => n.id), pointer: c.pointer && { x: c.pointer.x, y: c.pointer.y } };`);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  await c.writeClipboard('plain text here');
  await c.pressKey('v', { ctrl: true }); await sleep(500);
  const pastedText = await ev(`${C} const known = ['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1']; return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !known.includes(n.id)).map(n => n.getData()) };`);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); return true;`); await sleep(200);
  await c.pressKey('d', { ctrl: true }); await sleep(300);
  const dup = await ev(`${C} const known = ['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1']; return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !known.includes(n.id)).map(n => n.getData()) };`);
  if (dup.nodes > n0) { await ev(`${C} c.undo(); return true;`); await sleep(300); }
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); c.select(c.nodes.get('t4')); return true;`); await sleep(200);
  await c.pressKey('g', { ctrl: true }); await sleep(300);
  const grp = await ev(`${C} const known = ['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1']; return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !known.includes(n.id)).map(n => n.getData()), order: Array.from(c.nodes.keys()) };`);
  if (grp.nodes > n0) { await ev(`${C} c.undo(); return true;`); await sleep(300); }
  // Create group via the menu button
  await ev(`${C} c.deselectAll(); c.select(c.nodes.get('t3')); c.select(c.nodes.get('t4')); return true;`); await sleep(300);
  const grpMenu = await ev(`${C} const b = Array.from(document.querySelectorAll('.canvas-menu .clickable-icon')).find(e => e.getAttribute('aria-label') === 'Create group'); if (!b) return null; b.click(); await new Promise(r => setTimeout(r, 400)); const known = ['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1']; return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !known.includes(n.id)).map(n => n.getData()), order: Array.from(c.nodes.keys()), editingLabel: !!document.querySelector('.canvas-group-label[contenteditable="true"]'), data: c.getData().nodes.map(n => n.id) };`);
  await c.pressKey('Escape'); await sleep(200);
  await ev(`${C} c.undo(); c.deselectAll(); return true;`); await sleep(300);
  return { clipboardText: text, pasted, pastedText, dup, grp, grpMenu };
});

await step('light-theme-colors', async () => {
  await ev(`app.changeTheme('moonstone'); return true;`); await sleep(800);
  const r = await ev(`${C} c.selectOnly(c.nodes.get('t0')); const n = c.nodes.get('t0'); const cs = (e) => { const s = getComputedStyle(e); return { border: s.borderWidth + ' ' + s.borderColor, shadow: s.boxShadow, bg: s.backgroundColor }; };
    const plain = c.nodes.get('f1'); const d = { selectedDefault: cs(n.nodeEl.querySelector('.canvas-node-container')), selectedContent: cs(n.nodeEl.querySelector('.canvas-node-content')), plainDefault: cs(plain.nodeEl.querySelector('.canvas-node-container')), plainContent: cs(plain.nodeEl.querySelector('.canvas-node-content')), bodyBg: getComputedStyle(document.body).backgroundColor, text: getComputedStyle(document.body).color, menuBg: (() => { const m = document.querySelector('.canvas-menu'); return m && getComputedStyle(m).backgroundColor + ' ' + getComputedStyle(m).borderColor; })(), controlBg: getComputedStyle(document.querySelector('.canvas-control-item')).backgroundColor + ' / ' + getComputedStyle(document.querySelector('.canvas-control-group')).backgroundColor + ' ' + getComputedStyle(document.querySelector('.canvas-control-group')).borderColor, controlColor: getComputedStyle(document.querySelector('.canvas-control-item')).color, cardMenuBg: getComputedStyle(c.cardMenuEl).backgroundColor, cardMenuColor: getComputedStyle(c.cardMenuEl.firstElementChild).color, labelColor: getComputedStyle(plain.nodeEl.querySelector('.canvas-node-label')).color, groupLabel: (() => { const g = c.nodes.get('g2').nodeEl.querySelector('.canvas-group-label'); return getComputedStyle(g).color + ' on ' + getComputedStyle(g).backgroundColor; })(), coloredGroupLabel: (() => { const g = c.nodes.get('g1').nodeEl.querySelector('.canvas-group-label'); return getComputedStyle(g).color + ' on ' + getComputedStyle(g).backgroundColor; })(), edgeDefault: getComputedStyle(c.edges.get('e3').path.display).stroke, edgeLabel: (() => { const e = c.edges.get('e2'); const el = e.labelElement && e.labelElement.wrapperEl ? e.labelElement.wrapperEl.firstElementChild : null; return el && getComputedStyle(el).backgroundColor + ' ' + getComputedStyle(el).color; })() }; c.deselectAll(); return d;`);
  const dark = await (async () => { await ev(`app.changeTheme('obsidian'); return true;`); await sleep(800); return ev(`${C} c.selectOnly(c.nodes.get('t0')); const n = c.nodes.get('t0'); const cs = (e) => { const s = getComputedStyle(e); return { border: s.borderWidth + ' ' + s.borderColor, shadow: s.boxShadow, bg: s.backgroundColor }; }; const plain = c.nodes.get('f1'); const d = { selectedDefault: cs(n.nodeEl.querySelector('.canvas-node-container')), plainDefault: cs(plain.nodeEl.querySelector('.canvas-node-container')), plainContent: cs(plain.nodeEl.querySelector('.canvas-node-content')), menuBg: (() => { const m = document.querySelector('.canvas-menu'); return m && getComputedStyle(m).backgroundColor + ' ' + getComputedStyle(m).borderColor; })(), controlColor: getComputedStyle(document.querySelector('.canvas-control-item')).color, cardMenuColor: getComputedStyle(c.cardMenuEl.firstElementChild).color, groupLabel: (() => { const g = c.nodes.get('g2').nodeEl.querySelector('.canvas-group-label'); return getComputedStyle(g).color + ' on ' + getComputedStyle(g).backgroundColor; })(), edgeLabel: (() => { const e = c.edges.get('e2'); const el = e.labelElement && e.labelElement.wrapperEl ? e.labelElement.wrapperEl.firstElementChild : null; return el && getComputedStyle(el).backgroundColor + ' ' + getComputedStyle(el).color + ' ' + getComputedStyle(el).fontSize + ' ' + getComputedStyle(el).padding + ' ' + getComputedStyle(el).borderRadius + ' ' + getComputedStyle(el).lineHeight + ' ' + el.className; })() }; c.deselectAll(); return d;`); })();
  return { light: r, dark };
});

await step('workspace-leaf', async () => {
  await ev(`${C} c.setViewport(123.5, -45.25, -0.75); c.markViewportChanged(); app.workspace.requestSaveLayout(); return true;`); await sleep(2500);
  return ev(`const t = await app.vault.adapter.read('.obsidian/workspace.json'); const j = JSON.parse(t); const find = (n) => { if (!n) return null; if (n.type === 'leaf' && n.state && n.state.type === 'canvas') return n; for (const ch of (n.children || [])) { const f = find(ch); if (f) return f; } return null; }; return { leaf: find(j.main), raw: t.slice(0, 200) };`);
});

c.close();
console.log('done');
