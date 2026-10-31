// Canvas view probe, part C: hover handles, resize, connection drop, clipboard, group drag, themes, leaf JSON.
import fs from 'node:fs';
import { connect, sleep } from '/home/ragtux/bstore2/raga/repos/silica/tools/obsidian-probe/cdp.mjs';

const OUT = process.env.OUT;
const out = {};
const save = () => fs.writeFileSync(OUT, JSON.stringify(out, null, 1));
const c = await connect();
const ev = (s) => c.evaluate(s);
async function step(name, fn) {
  try { const ok = await ev(`return app.workspace.getLeavesOfType('canvas').length;`); if (!ok) { console.log('leaf gone before', name); await reopen(); } } catch (e) { console.log('check failed', String(e)); }
  try { out[name] = await fn(); } catch (e) { out[name] = { error: String(e && e.message || e) }; }
  save();
  console.log(name, JSON.stringify(out[name]).slice(0, 500));
}
const KNOWN = ['g1','g2','g3','t0','t1','t2','t3','t4','t5','t6','t7','f1','f2','l1','e1','e2','e3','e4','e5','e6','e7'];
const C = `const v = app.workspace.getLeavesOfType('canvas')[0].view; const c = v.canvas;`;
const reopen = async () => {
  await ev(`app.workspace.leftSplit.collapse(); app.workspace.rightSplit.collapse(); for (const l of app.workspace.getLeavesOfType('canvas')) l.detach(); for (const l of app.workspace.getLeavesOfType('graph')) l.detach(); return true;`); await sleep(300);
  await ev(`const f = app.vault.getAbstractFileByPath('Fixture.canvas'); await app.workspace.getLeaf('tab').openFile(f); return true;`); await sleep(1500);
  const check = await ev(`${C} return { nodes: c.nodes.size, file: v.file.path };`); console.log('reopened', JSON.stringify(check));
};
const centre = async (x, y) => { await ev(`${C} c.deselectAll(); c.setViewport(${x}, ${y}, 0); return true;`); await sleep(350); };
const scr = (x, y) => ev(`${C} const r = c.canvasRect; return { x: r.left + (${x} - c.x) * c.scale + r.width / 2, y: r.top + (${y} - c.y) * c.scale + r.height / 2 };`);
const nodeInfo = (id) => ev(`${C} const n = c.nodes.get('${id}'); return n ? { x: n.x, y: n.y, w: n.width, h: n.height, sel: c.selection.has(n) } : null;`);
const positions = () => ev(`${C} return Object.fromEntries(Array.from(c.nodes.values()).map(n => [n.id, [n.x, n.y]]));`);

await reopen();
await step('rect', () => ev(`${C} const r = c.canvasRect; return { w: r.width, h: r.height, left: r.left, top: r.top, inner: [innerWidth, innerHeight] };`));

await step('hover', async () => {
  await centre(1000, 90);
  const mid = await scr(840, 30);
  await c.mouseMove(mid.x - 20, mid.y); await sleep(200); await c.mouseMove(mid.x, mid.y); await sleep(500);
  return ev(`${C} const n = c.nodes.get('t3'); const el = n.nodeEl; const cs = (e) => { const s = getComputedStyle(e); return { w: s.width, h: s.height, bg: s.backgroundColor, radius: s.borderRadius, border: s.borderWidth + ' ' + s.borderColor, top: s.top, left: s.left, right: s.right, bottom: s.bottom, transform: s.transform, cursor: s.cursor, opacity: s.opacity, display: s.display, margin: s.margin }; };
    const all = (sel) => Array.from(document.querySelectorAll(sel)).map(e => ({ cls: e.className, style: cs(e), rect: (() => { const r = e.getBoundingClientRect(); return [Math.round(r.x * 10) / 10, Math.round(r.y * 10) / 10, Math.round(r.width * 10) / 10, Math.round(r.height * 10) / 10]; })() }));
    const nr = el.getBoundingClientRect();
    const layer = c.nodeInteractionLayer; const lel = layer && (layer.interactionEl || layer.el || layer.containerEl);
    return { nodeRect: [nr.x, nr.y, nr.width, nr.height], cls: el.className, layerKeys: layer && Object.keys(layer), layerCls: lel && lel.className, layerHtml: lel && lel.outerHTML.slice(0, 1500), resizers: all('.canvas-node-resizer'), connections: all('.canvas-node-connection-point'), container: cs(el.querySelector('.canvas-node-container')) };`);
});

await step('resize', async () => {
  await centre(1000, 90);
  const mid = await scr(840, 30);
  await c.mouseMove(mid.x - 20, mid.y); await sleep(200); await c.mouseMove(mid.x, mid.y); await sleep(400);
  const handles = await ev(`return Array.from(document.querySelectorAll('.canvas-node-resizer')).map(e => { const r = e.getBoundingClientRect(); return { cls: e.className, x: r.x + r.width / 2, y: r.y + r.height / 2 }; });`);
  const br = handles.find(h => /mod-bottom-right|bottom.*right/.test(h.cls)) || handles.reduce((a, b) => (b.x + b.y > a.x + a.y ? b : a), handles[0]);
  if (!br) return { handles, none: true };
  await c.drag(br.x, br.y, br.x - 300, br.y - 300, { steps: 10 }); await sleep(400);
  const min = await nodeInfo('t3');
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  await centre(1000, 90);
  await c.mouseMove(mid.x - 20, mid.y); await sleep(200); await c.mouseMove(mid.x, mid.y); await sleep(400);
  const h2 = await ev(`const e = Array.from(document.querySelectorAll('.canvas-node-resizer')).find(e => e.className === ${JSON.stringify(br.cls)}); const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  await c.drag(h2.x, h2.y, h2.x + 13, h2.y + 7, { steps: 8 }); await sleep(400);
  const small = await nodeInfo('t3');
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  // top-left handle: which sides move
  await centre(1000, 90);
  await c.mouseMove(mid.x - 20, mid.y); await sleep(200); await c.mouseMove(mid.x, mid.y); await sleep(400);
  const tl = await ev(`const hs = Array.from(document.querySelectorAll('.canvas-node-resizer')).map(e => { const r = e.getBoundingClientRect(); return { cls: e.className, x: r.x + r.width / 2, y: r.y + r.height / 2 }; }); return hs.reduce((a, b) => (b.x + b.y < a.x + a.y ? b : a), hs[0]);`);
  await c.drag(tl.x, tl.y, tl.x - 27, tl.y - 13, { steps: 8 }); await sleep(400);
  const topLeft = await nodeInfo('t3');
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  return { handles, br: br.cls, min, small, tl: tl.cls, topLeft, after: await nodeInfo('t3') };
});

await step('connect', async () => {
  await centre(1000, 90);
  const mid = await scr(840, 30);
  await c.mouseMove(mid.x - 20, mid.y); await sleep(200); await c.mouseMove(mid.x, mid.y); await sleep(400);
  const pts = await ev(`return Array.from(document.querySelectorAll('.canvas-node-connection-point')).map(e => { const r = e.getBoundingClientRect(); return { cls: e.className, x: r.x + r.width / 2, y: r.y + r.height / 2, w: r.width, h: r.height }; });`);
  const right = pts.reduce((a, b) => (b.x > a.x ? b : a), pts[0]);
  if (!right) return { pts, none: true };
  const before = await ev(`${C} return { nodes: c.nodes.size, edges: c.edges.size };`);
  const target = await scr(1250, 180);
  await c.drag(right.x, right.y, target.x, target.y, { steps: 12 }); await sleep(700);
  const dropped = await ev(`${C} const n = Array.from(c.nodes.values()).find(n => !${JSON.stringify(KNOWN)}.includes(n.id)); const e = Array.from(c.edges.values()).find(e => !${JSON.stringify(KNOWN)}.includes(e.id)); return { nodes: c.nodes.size, edges: c.edges.size, newNode: n && { x: n.x, y: n.y, w: n.width, h: n.height, editing: !!n.isEditing, cls: n.nodeEl.className, data: n.getData() }, newEdge: e && e.getData(), menu: Array.from(document.querySelectorAll('.menu .menu-item')).map(i => (i.querySelector('.menu-item-title') || i).textContent.trim()), active: document.activeElement && document.activeElement.className.slice(0, 60) };`);
  await c.pressKey('Escape'); await sleep(300);
  const afterEsc = await ev(`${C} return { nodes: c.nodes.size, edges: c.edges.size };`);
  // clean: delete anything new
  await ev(`${C} for (const n of Array.from(c.nodes.values())) if (!${JSON.stringify(KNOWN)}.includes(n.id)) c.removeNode(n); for (const e of Array.from(c.edges.values())) if (!${JSON.stringify(KNOWN)}.includes(e.id)) c.removeEdge(e); return true;`);
  // onto a card: t4 (980..1140 x 0..60), drop near its top-left (1000, 8)
  await centre(1000, 90);
  await c.mouseMove(mid.x - 20, mid.y); await sleep(200); await c.mouseMove(mid.x, mid.y); await sleep(400);
  const pts2 = await ev(`return Array.from(document.querySelectorAll('.canvas-node-connection-point')).map(e => { const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; });`);
  const r2 = pts2.reduce((a, b) => (b.x > a.x ? b : a), pts2[0]);
  const t4 = await scr(1000, 8);
  await c.drag(r2.x, r2.y, t4.x, t4.y, { steps: 12 }); await sleep(500);
  const ontoCard = await ev(`${C} const e = Array.from(c.edges.values()).find(e => !${JSON.stringify(KNOWN)}.includes(e.id)); return { edge: e && e.getData(), sel: Array.from(c.selection).map(x => x.id), editingLabel: !!document.querySelector('.canvas-path-label[contenteditable="true"], .canvas-path-label-wrapper [contenteditable="true"]') };`);
  await c.pressKey('Escape'); await sleep(200);
  await ev(`${C} for (const e of Array.from(c.edges.values())) if (!${JSON.stringify(KNOWN)}.includes(e.id)) c.removeEdge(e); return true;`);
  // drop on the bottom area of t4 to see the side rule
  await centre(1000, 90);
  await c.mouseMove(mid.x - 20, mid.y); await sleep(200); await c.mouseMove(mid.x, mid.y); await sleep(400);
  const pts3 = await ev(`return Array.from(document.querySelectorAll('.canvas-node-connection-point')).map(e => { const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; });`);
  const r3 = pts3.reduce((a, b) => (b.x > a.x ? b : a), pts3[0]);
  const t4b = await scr(1060, 55);
  await c.drag(r3.x, r3.y, t4b.x, t4b.y, { steps: 12 }); await sleep(500);
  const ontoBottom = await ev(`${C} const e = Array.from(c.edges.values()).find(e => !${JSON.stringify(KNOWN)}.includes(e.id)); return e && e.getData();`);
  await c.pressKey('Escape'); await sleep(200);
  await ev(`${C} for (const e of Array.from(c.edges.values())) if (!${JSON.stringify(KNOWN)}.includes(e.id)) c.removeEdge(e); return true;`);
  return { pts, before, dropped, afterEsc, ontoCard, ontoBottom };
});

await step('group-drag', async () => {
  await centre(100, -60);
  const before = await positions();
  const lbl = await ev(`${C} const r = c.nodes.get('g1').nodeEl.querySelector('.canvas-group-label').getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  await c.drag(lbl.x, lbl.y, lbl.x + 40, lbl.y + 40, { steps: 10 }); await sleep(400);
  const after = await positions();
  const moved = Object.keys(after).filter(k => before[k][0] !== after[k][0] || before[k][1] !== after[k][1]);
  const sel = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  const restored = JSON.stringify(await positions()) === JSON.stringify(before);
  // click on the group's border/body
  await centre(850, -40);
  const inside = await scr(850, -90);
  await c.clickAt(inside.x, inside.y); await sleep(300);
  const clickBody = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  const edge = await scr(850, -120);
  await c.clickAt(edge.x, edge.y); await sleep(300);
  const clickBorder = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  // a node straddling the group edge: move t3 to y=170 (group bottom is 200) then drag the group
  await ev(`${C} c.deselectAll(); c.nodes.get('t3').moveTo({ x: 760, y: 170 }); return true;`); await sleep(200);
  await centre(100, -60);
  const b2 = await positions();
  const lbl2 = await ev(`${C} const r = c.nodes.get('g1').nodeEl.querySelector('.canvas-group-label').getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  await c.drag(lbl2.x, lbl2.y, lbl2.x + 40, lbl2.y + 40, { steps: 10 }); await sleep(400);
  const a2 = await positions();
  const movedStraddle = Object.keys(a2).filter(k => b2[k][0] !== a2[k][0] || b2[k][1] !== a2[k][1]);
  await ev(`${C} c.undo(); c.nodes.get('t3').moveTo({ x: 760, y: 0 }); return true;`); await sleep(300);
  return { moved, sel, restored, clickBody, clickBorder, movedStraddle };
});

await step('edge-context', async () => {
  await centre(290, 55);
  const p = await scr(290, 55);
  await c.clickAt(p.x, p.y); await sleep(300);
  const clickSel = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  await c.clickAt(p.x, p.y, { button: 'right' }); await sleep(400);
  const menu = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  await centre(1170, 30);
  const q = await scr(1170, 30);
  await c.clickAt(q.x, q.y); await sleep(300);
  const clickSel2 = await ev(`${C} return Array.from(c.selection).map(n => n.id);`);
  await c.clickAt(q.x, q.y, { button: 'right' }); await sleep(400);
  const menu2 = await c.openMenuItems(); await c.pressKey('Escape'); await sleep(200);
  // card context menu, precisely
  await centre(840, 30);
  const r = await scr(840, 30);
  await c.clickAt(r.x, r.y, { button: 'right' }); await sleep(400);
  const card = await ev(`const m = document.querySelector('.menu'); return m ? Array.from(m.children).map(e => ({ cls: e.className, title: (e.querySelector('.menu-item-title') || e).textContent.trim(), icon: (e.querySelector('.menu-item-icon svg') || {}).getAttribute ? e.querySelector('.menu-item-icon svg').getAttribute('class') : null })) : null;`);
  await c.pressKey('Escape'); await sleep(200);
  await centre(200, 500);
  const f = await scr(200, 500);
  await c.clickAt(f.x, f.y, { button: 'right' }); await sleep(400);
  const fileCard = await ev(`const m = document.querySelector('.menu'); return m ? Array.from(m.children).map(e => ({ cls: e.className, title: (e.querySelector('.menu-item-title') || e).textContent.trim() })) : null;`);
  await c.pressKey('Escape'); await sleep(200);
  await centre(850, -40);
  const gl = await ev(`${C} const r = c.nodes.get('g1').nodeEl.querySelector('.canvas-group-label').getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  await c.clickAt(gl.x, gl.y, { button: 'right' }); await sleep(400);
  const group = await ev(`const m = document.querySelector('.menu'); return m ? Array.from(m.children).map(e => ({ cls: e.className, title: (e.querySelector('.menu-item-title') || e).textContent.trim() })) : null;`);
  await c.pressKey('Escape'); await sleep(200);
  return { clickSel, menu, clickSel2, menu2, card, fileCard, group };
});

await step('clipboard', async () => {
  await centre(1000, 90);
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); c.select(c.nodes.get('t4')); return true;`); await sleep(200);
  await c.pressKey('c', { ctrl: true }); await sleep(300);
  const text = await c.readClipboard();
  await ev(`${C} c.deselectAll(); return true;`);
  const n0 = await ev(`${C} return c.nodes.size;`);
  const pm = await scr(1000, 300); await c.mouseMove(pm.x, pm.y); await sleep(100);
  await c.pressKey('v', { ctrl: true }); await sleep(500);
  const pasted = await ev(`${C} return { nodes: c.nodes.size, edges: c.edges.size, new: Array.from(c.nodes.values()).filter(n => !${JSON.stringify(KNOWN)}.includes(n.id)).map(n => n.getData()), newEdges: Array.from(c.edges.values()).filter(e => !${JSON.stringify(KNOWN)}.includes(e.id)).map(e => e.getData()), sel: Array.from(c.selection).map(n => n.id), pointer: c.pointer && { x: c.pointer.x, y: c.pointer.y } };`);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  await c.writeClipboard('plain text here');
  await c.pressKey('v', { ctrl: true }); await sleep(500);
  const pastedText = await ev(`${C} return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !${JSON.stringify(KNOWN)}.includes(n.id)).map(n => n.getData()) };`);
  if (pastedText.nodes > n0) { await ev(`${C} c.undo(); return true;`); await sleep(300); }
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); return true;`); await sleep(200);
  await c.pressKey('d', { ctrl: true }); await sleep(300);
  const dup = await ev(`${C} return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !${JSON.stringify(KNOWN)}.includes(n.id)).map(n => n.getData()) };`);
  if (dup.nodes > n0) { await ev(`${C} c.undo(); return true;`); await sleep(300); }
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); c.select(c.nodes.get('t4')); return true;`); await sleep(200);
  await c.pressKey('g', { ctrl: true }); await sleep(300);
  const grp = await ev(`${C} return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !${JSON.stringify(KNOWN)}.includes(n.id)).map(n => n.getData()) };`);
  if (grp.nodes > n0) { await ev(`${C} c.undo(); return true;`); await sleep(300); }
  await ev(`${C} c.deselectAll(); c.select(c.nodes.get('t3')); c.select(c.nodes.get('t4')); return true;`); await sleep(300);
  const grpMenu = await ev(`${C} const b = Array.from(document.querySelectorAll('.canvas-menu .clickable-icon')).find(e => e.getAttribute('aria-label') === 'Create group'); if (!b) return null; b.click(); await new Promise(r => setTimeout(r, 400)); return { nodes: c.nodes.size, new: Array.from(c.nodes.values()).filter(n => !${JSON.stringify(KNOWN)}.includes(n.id)).map(n => n.getData()), order: c.getData().nodes.map(n => n.id), editingLabel: !!document.querySelector('.canvas-group-label[contenteditable="true"]'), sel: Array.from(c.selection).map(n => n.id) };`);
  await c.pressKey('Escape'); await sleep(200);
  await ev(`${C} for (const n of Array.from(c.nodes.values())) if (!${JSON.stringify(KNOWN)}.includes(n.id)) c.removeNode(n); c.deselectAll(); return true;`); await sleep(200);
  // cut
  await ev(`${C} c.selectOnly(c.nodes.get('t3')); return true;`); await sleep(200);
  await c.pressKey('x', { ctrl: true }); await sleep(300);
  const cut = await ev(`${C} return { has: c.nodes.has('t3'), edges: c.edges.size };`);
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  return { clipboardText: text, pasted, pastedText, dup, grp, grpMenu, cut, restored: await ev(`${C} return { has: c.nodes.has('t3'), nodes: c.nodes.size, edges: c.edges.size };`) };
});

await step('themes', async () => {
  const read = () => ev(`${C} c.selectOnly(c.nodes.get('t0')); const n = c.nodes.get('t0'); const cs = (e) => { const s = getComputedStyle(e); return { border: s.borderWidth + ' ' + s.borderColor, shadow: s.boxShadow, bg: s.backgroundColor }; };
    const plain = c.nodes.get('f1'); const d = { selectedDefault: cs(n.nodeEl.querySelector('.canvas-node-container')), selectedContent: cs(n.nodeEl.querySelector('.canvas-node-content')), plainDefault: cs(plain.nodeEl.querySelector('.canvas-node-container')), plainContent: cs(plain.nodeEl.querySelector('.canvas-node-content')), bodyBg: getComputedStyle(document.body).backgroundColor, text: getComputedStyle(document.body).color, muted: getComputedStyle(document.body).getPropertyValue('--text-muted'), faint: getComputedStyle(document.body).getPropertyValue('--text-faint'), menuBg: (() => { const m = document.querySelector('.canvas-menu'); return m && getComputedStyle(m).backgroundColor + ' ' + getComputedStyle(m).borderColor; })(), controlBg: getComputedStyle(document.querySelector('.canvas-control-item')).backgroundColor + ' / ' + getComputedStyle(document.querySelector('.canvas-control-group')).backgroundColor + ' ' + getComputedStyle(document.querySelector('.canvas-control-group')).borderColor, controlColor: getComputedStyle(document.querySelector('.canvas-control-item')).color, cardMenuBg: getComputedStyle(c.cardMenuEl).backgroundColor, cardMenuColor: getComputedStyle(c.cardMenuEl.firstElementChild).color, labelColor: getComputedStyle(plain.nodeEl.querySelector('.canvas-node-label')).color, groupLabel: (() => { const g = c.nodes.get('g2').nodeEl.querySelector('.canvas-group-label'); return getComputedStyle(g).color + ' on ' + getComputedStyle(g).backgroundColor; })(), coloredGroupLabel: (() => { const g = c.nodes.get('g1').nodeEl.querySelector('.canvas-group-label'); return getComputedStyle(g).color + ' on ' + getComputedStyle(g).backgroundColor; })(), groupContainer: cs(c.nodes.get('g2').nodeEl.querySelector('.canvas-node-container')), groupContent: cs(c.nodes.get('g2').nodeEl.querySelector('.canvas-node-content')), coloredGroupContainer: cs(c.nodes.get('g1').nodeEl.querySelector('.canvas-node-container')), coloredGroupContent: cs(c.nodes.get('g1').nodeEl.querySelector('.canvas-node-content')), edgeDefault: getComputedStyle(c.edges.get('e3').path.display).stroke, arrow: (() => { const g = c.edgeEndContainerEl.querySelector('polygon'); return g && getComputedStyle(g).fill + ' ' + getComputedStyle(g).stroke; })(), edgeLabel: (() => { const e = c.edges.get('e2'); const el = e.labelElement && e.labelElement.wrapperEl ? e.labelElement.wrapperEl.firstElementChild : null; return el && { bg: getComputedStyle(el).backgroundColor, color: getComputedStyle(el).color, font: getComputedStyle(el).fontSize, padding: getComputedStyle(el).padding, radius: getComputedStyle(el).borderRadius, lh: getComputedStyle(el).lineHeight, cls: el.className, border: getComputedStyle(el).borderWidth + ' ' + getComputedStyle(el).borderColor }; })(), coloredEdgeLabel: (() => { const e = c.edges.get('e1'); const el = e.labelElement && e.labelElement.wrapperEl ? e.labelElement.wrapperEl.firstElementChild : null; return el && getComputedStyle(el).backgroundColor + ' ' + getComputedStyle(el).color; })(), selectedEdge: (() => { c.selectOnly(c.edges.get('e3')); const s = getComputedStyle(c.edges.get('e3').path.display); return s.stroke + ' ' + s.strokeWidth; })(), selectedColoredEdge: (() => { c.selectOnly(c.edges.get('e1')); const s = getComputedStyle(c.edges.get('e1').path.display); return s.stroke; })() }; c.deselectAll(); return d;`);
  await centre(700, 300);
  const dark = await read();
  await ev(`app.changeTheme('moonstone'); return true;`); await sleep(900);
  const light = await read();
  await ev(`app.changeTheme('obsidian'); return true;`); await sleep(900);
  return { dark, light };
});

await step('selected-colors', () => ev(`${C} const res = {}; for (const id of ['t0','t3','t7']) { const n = c.nodes.get(id); c.selectOnly(n); const s = getComputedStyle(n.nodeEl.querySelector('.canvas-node-container')); res[id] = { border: s.borderColor, shadow: s.boxShadow }; c.deselectAll(); const s2 = getComputedStyle(n.nodeEl.querySelector('.canvas-node-container')); res[id].plain = { border: s2.borderColor, shadow: s2.boxShadow, content: getComputedStyle(n.nodeEl.querySelector('.canvas-node-content')).backgroundColor }; } return res;`));

await step('workspace-leaf', async () => {
  await ev(`${C} c.setViewport(123.5, -45.25, -0.75); c.markViewportChanged(); app.workspace.requestSaveLayout(); return true;`); await sleep(3000);
  return ev(`const t = await app.vault.adapter.read('.obsidian/workspace.json'); const j = JSON.parse(t); const find = (n) => { if (!n) return null; if (n.type === 'leaf' && n.state && n.state.type === 'canvas') return n; for (const ch of (n.children || [])) { const f = find(ch); if (f) return f; } return null; }; return { leaf: find(j.main) };`);
});

await step('screenshot-setup', async () => {
  await ev(`${C} c.deselectAll(); c.setViewport(560, 260, -0.4); return true;`); await sleep(800);
  return ev(`${C} return { x: c.x, y: c.y, zoom: c.zoom, scale: c.scale };`);
});

c.close();
console.log('done');
