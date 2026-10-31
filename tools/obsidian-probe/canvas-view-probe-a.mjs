// Canvas view probe, part A: programmatic reads of the running canvas.
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
  console.log(name, JSON.stringify(out[name]).slice(0, 300));
}

// open the fixture
await step('open', async () => {
  await ev(`const f = app.vault.getAbstractFileByPath('Fixture.canvas'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(1500);
  return ev(`const v = app.workspace.getLeavesOfType('canvas')[0].view; return { file: v.file.path, hasCanvas: !!v.canvas };`);
});
const C = `const v = app.workspace.getLeavesOfType('canvas')[0].view; const c = v.canvas;`;

await step('config', () => ev(`${C} return { config: c.config, options: app.internalPlugins.plugins.canvas.instance.options, gridSpacing: c.gridSpacing(), snapDistance: c.snapDistance(), zoomBreakpoint: typeof c.zoomBreakpoint === 'function' ? String(c.zoomBreakpoint.length) : c.zoomBreakpoint, readonly: c.readonly };`));

await step('viewport', () => ev(`${C}
  const r = c.canvasRect; const bb = c.getViewportBBox(); const center = c.posCenter();
  const st = v.getState(); const leafState = v.leaf.getViewState();
  return { x: c.x, y: c.y, zoom: c.zoom, scale: c.scale, tx: c.tx, ty: c.ty, tZoom: c.tZoom, canvasRect: {x:r.x,y:r.y,w:r.width,h:r.height,left:r.left,top:r.top}, bbox: bb, center, viewState: st, leafState, pow: Math.pow(2, c.zoom), transform: getComputedStyle(c.canvasEl).transform };`));

await step('viewport-moved', async () => {
  await ev(`${C} c.setViewport(100, 50, 0); return true;`); await sleep(700);
  return ev(`${C} const r = c.canvasRect; return { x: c.x, y: c.y, zoom: c.zoom, scale: c.scale, transform: getComputedStyle(c.canvasEl).transform, rect: {w:r.width,h:r.height}, bbox: c.getViewportBBox(), center: c.posCenter(), state: v.getState() };`);
});

await step('zoom-limits', async () => {
  await ev(`${C} c.zoomBy(-50); return true;`); await sleep(900);
  const min = await ev(`${C} return { zoom: c.zoom, tZoom: c.tZoom, scale: c.scale };`);
  await ev(`${C} c.zoomBy(100); return true;`); await sleep(900);
  const max = await ev(`${C} return { zoom: c.zoom, tZoom: c.tZoom, scale: c.scale };`);
  await ev(`${C} c.setViewport(700, 300, -1); return true;`); await sleep(500);
  return { min, max, zoomByLength: await ev(`${C} return c.zoomBy.length;`) };
});

await step('defaults', async () => ev(`${C}
  const before = c.nodes.size;
  const t = c.createTextNode({ pos: { x: 3000, y: 3000 }, save: false, focus: false });
  const g = c.createGroupNode({ pos: { x: 3000, y: 3500 }, save: false, focus: false });
  const l = c.createLinkNode({ pos: { x: 3000, y: 4000 }, url: 'https://example.org', save: false, focus: false });
  const f = c.createFileNode({ pos: { x: 3000, y: 4500 }, file: app.vault.getAbstractFileByPath('Plan.md'), save: false, focus: false });
  const i = c.createFileNode({ pos: { x: 4000, y: 4500 }, file: app.vault.getAbstractFileByPath('pic.png'), save: false, focus: false });
  const d = (n) => n ? { id: n.id, x: n.x, y: n.y, w: n.width, h: n.height, data: n.getData() } : null;
  const res = { text: d(t), group: d(g), link: d(l), file: d(f), image: d(i), zIndexCounter: c.zIndexCounter, before, after: c.nodes.size };
  for (const n of [t, g, l, f, i]) if (n) c.removeNode(n);
  return res;`));

await step('scope-keys', () => ev(`${C}
  const keys = (s) => (s && s.keys || []).map(k => ({ mods: k.modifiers, key: k.key }));
  return { view: keys(v.scope), canvasKeys: c.keys, appScope: keys(app.scope).filter(k => true).length };`));

await step('chrome-labels', () => ev(`${C}
  const lab = (sel) => Array.from(document.querySelectorAll(sel)).map(e => ({ aria: e.getAttribute('aria-label'), cls: e.className, title: e.title }));
  const ctrlGroups = Array.from(document.querySelectorAll('.canvas-controls .canvas-control-group')).map(g => Array.from(g.children).map(e => ({ aria: e.getAttribute('aria-label'), cls: e.className })));
  return { cardMenu: lab('.canvas-card-menu > *'), controls: ctrlGroups, quickSettings: c.quickSettingsButton && c.quickSettingsButton.getAttribute('aria-label'), undo: c.undoBtnEl && c.undoBtnEl.className, controlsRect: (() => { const r = c.canvasControlsEl.getBoundingClientRect(); const w = c.wrapperEl.getBoundingClientRect(); return { right: w.right - r.right, top: r.top - w.top, bottom: w.bottom - r.bottom, w: r.width, h: r.height }; })(), cardMenuRect: (() => { const r = c.cardMenuEl.getBoundingClientRect(); const w = c.wrapperEl.getBoundingClientRect(); return { bottom: w.bottom - r.bottom, centerOff: (r.left + r.width/2) - (w.left + w.width/2), w: r.width, h: r.height }; })() };`));

await step('node-menu', async () => {
  const res = {};
  for (const id of ['t3', 'f1', 'l1', 'g1', 'f2']) {
    res[id] = await ev(`${C} const n = c.nodes.get('${id}'); c.selectOnly(n); await new Promise(r => setTimeout(r, 300));
      const m = document.querySelector('.canvas-menu'); if (!m) return { none: true, selection: c.selection.size };
      return Array.from(m.children).map(e => ({ aria: e.getAttribute('aria-label'), cls: e.className }));`);
  }
  res.edge = await ev(`${C} const e = c.edges.get('e1'); c.selectOnly(e); await new Promise(r => setTimeout(r, 300));
      const m = document.querySelector('.canvas-menu'); if (!m) return { none: true };
      return Array.from(m.children).map(e => ({ aria: e.getAttribute('aria-label'), cls: e.className }));`);
  res.multi = await ev(`${C} c.deselectAll(); c.select(c.nodes.get('t3')); c.select(c.nodes.get('t4')); await new Promise(r => setTimeout(r, 300));
      const m = document.querySelector('.canvas-menu'); if (!m) return { none: true };
      return Array.from(m.children).map(e => ({ aria: e.getAttribute('aria-label'), cls: e.className }));`);
  await ev(`${C} c.deselectAll(); return true;`);
  return res;
});

await step('node-dom', () => ev(`${C}
  const n = c.nodes.get('t3'); c.selectOnly(n);
  const el = n.nodeEl; const cs = (e) => { const s = getComputedStyle(e); return { w: s.width, h: s.height, border: s.borderWidth + ' ' + s.borderStyle + ' ' + s.borderColor, radius: s.borderRadius, bg: s.backgroundColor, shadow: s.boxShadow, outline: s.outline, color: s.color, font: s.fontSize, padding: s.padding, top: s.top, left: s.left, transform: s.transform, opacity: s.opacity }; };
  const q = (sel) => el.querySelector(sel);
  const all = (sel) => Array.from(el.querySelectorAll(sel)).map(e => ({ cls: e.className, style: cs(e), rect: (() => { const r = e.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })() }));
  return { nodeClass: el.className, container: q('.canvas-node-container') && cs(q('.canvas-node-container')), content: q('.canvas-node-content') && cs(q('.canvas-node-content')), label: q('.canvas-node-label') && { text: q('.canvas-node-label').textContent, style: cs(q('.canvas-node-label')) }, resizers: all('.canvas-node-resizer'), connections: all('.canvas-node-connection-point'), inner: q('.canvas-node-content') && q('.canvas-node-content').firstElementChild && q('.canvas-node-content').firstElementChild.className, innerStyle: q('.canvas-node-content') && q('.canvas-node-content').firstElementChild && cs(q('.canvas-node-content').firstElementChild), md: q('.markdown-preview-view, .markdown-embed, .markdown-source-view') && cs(q('.markdown-preview-view, .markdown-embed, .markdown-source-view')), scale: c.scale };`));

await step('labels-dom', () => ev(`${C}
  const cs = (e) => { const s = getComputedStyle(e); return { font: s.fontSize, lh: s.lineHeight, color: s.color, bg: s.backgroundColor, padding: s.padding, radius: s.borderRadius, top: s.top, left: s.left, transform: s.transform, origin: s.transformOrigin, weight: s.fontWeight, opacity: s.opacity, maxWidth: s.maxWidth, overflow: s.overflow, textOverflow: s.textOverflow, whiteSpace: s.whiteSpace }; };
  const f = c.nodes.get('f1'); const g = c.nodes.get('g1'); const l = c.nodes.get('l1'); const g2 = c.nodes.get('g2'); const g3 = c.nodes.get('g3');
  const lbl = (n) => n.nodeEl.querySelector('.canvas-node-label');
  const gl = (n) => n.nodeEl.querySelector('.canvas-group-label');
  const cont = (n) => n.nodeEl.querySelector('.canvas-node-container');
  return { scale: c.scale, fileLabel: lbl(f) && { text: lbl(f).textContent, style: cs(lbl(f)) }, linkLabel: lbl(l) && { text: lbl(l).textContent, style: cs(lbl(l)) }, textLabel: !!lbl(c.nodes.get('t3')), groupLabel: gl(g) && { text: gl(g).textContent, style: cs(gl(g)) }, plainGroupLabel: gl(g2) && { style: cs(gl(g2)) }, groupContainer: cs(cont(g)), plainGroupContainer: cs(cont(g2)), picGroupContainer: cs(cont(g3)), picGroupBg: (() => { const e = g3.nodeEl.querySelector('.canvas-node-content, .canvas-group-background, [style*="background"]'); return e && { cls: e.className, bgImage: getComputedStyle(e).backgroundImage.slice(0, 60), size: getComputedStyle(e).backgroundSize, repeat: getComputedStyle(e).backgroundRepeat, position: getComputedStyle(e).backgroundPosition }; })(), groupNodeClasses: g.nodeEl.className, picGroupHtml: g3.nodeEl.innerHTML.slice(0, 600) };`));

await step('edges-dom', () => ev(`${C}
  const res = [];
  for (const [id, e] of c.edges) {
    const p = e.path; const d = p && p.display && p.display.getAttribute('d');
    const inter = p && p.interaction && p.interaction.getAttribute('d');
    const cs = (el) => el ? { stroke: getComputedStyle(el).stroke, sw: getComputedStyle(el).strokeWidth, cls: el.className.baseVal } : null;
    const lbl = e.labelElement && e.labelElement.wrapperEl ? e.labelElement.wrapperEl : (e.labelEl || null);
    const ls = lbl ? getComputedStyle(lbl) : null;
    res.push({ id, from: { id: e.from.node.id, side: e.from.side, end: e.from.end }, to: { id: e.to.node.id, side: e.to.side, end: e.to.end }, d, inter: inter && inter.slice(0, 80), display: cs(p && p.display), interaction: cs(p && p.interaction), ends: { from: e.fromLineEnd && e.fromLineEnd.el && e.fromLineEnd.el.outerHTML.slice(0, 300), to: e.toLineEnd && e.toLineEnd.el && e.toLineEnd.el.outerHTML.slice(0, 300) }, label: lbl && { cls: lbl.className, text: lbl.textContent, font: ls.fontSize, padding: ls.padding, bg: ls.backgroundColor, radius: ls.borderRadius, lh: ls.lineHeight, transform: ls.transform, color: ls.color }, keys: Object.keys(e) });
  }
  return { scale: c.scale, edges: res, svgTransform: getComputedStyle(c.edgeContainerEl).transform, svgInCanvas: c.canvasEl.contains(c.edgeContainerEl), endSvg: c.edgeEndContainerEl.innerHTML.slice(0, 500) };`));

await step('bezier-samples', async () => {
  const samples = [];
  for (const [bx, by] of [[600, 0], [400, 50], [340, 50], [1000, 50], [140, 300], [140, -300], [-200, 50], [2000, 50], [340, 600]]) {
    samples.push(await ev(`${C} const a = c.nodes.get('t4'); const b = c.nodes.get('t5');
      b.moveTo({ x: ${bx + 980}, y: ${by} }); a.moveTo({ x: 980, y: 0 }); c.requestFrame(); await new Promise(r => requestAnimationFrame(() => setTimeout(r, 120)));
      const e = c.edges.get('e3'); return { b: [b.x, b.y, b.width, b.height], a: [a.x, a.y, a.width, a.height], d: e.path.display.getAttribute('d') };`));
  }
  await ev(`${C} c.nodes.get('t5').moveTo({ x: 1200, y: 0 }); return true;`);
  return samples;
});

await step('background', () => ev(`${C}
  const pat = c.backgroundPatternEl; const circ = pat.querySelector('circle');
  return { patternAttrs: Object.fromEntries(Array.from(pat.attributes).map(a => [a.name, a.value])), circle: circ && Object.fromEntries(Array.from(circ.attributes).map(a => [a.name, a.value])), scale: c.scale, x: c.x, y: c.y, rectW: c.canvasRect.width, rectH: c.canvasRect.height };`));

await step('background-zoomed', async () => {
  await ev(`${C} c.setViewport(700, 300, 0); return true;`); await sleep(600);
  const one = await ev(`${C} const pat = c.backgroundPatternEl; return { scale: c.scale, attrs: Object.fromEntries(Array.from(pat.attributes).map(a => [a.name, a.value])), circle: Object.fromEntries(Array.from(pat.querySelector('circle').attributes).map(a => [a.name, a.value])) };`);
  await ev(`${C} c.setViewport(700, 300, -2.5); return true;`); await sleep(600);
  const far = await ev(`${C} const pat = c.backgroundPatternEl; return { scale: c.scale, attrs: Object.fromEntries(Array.from(pat.attributes).map(a => [a.name, a.value])), circle: Object.fromEntries(Array.from(pat.querySelector('circle').attributes).map(a => [a.name, a.value])) };`);
  return { one, far };
});

await step('breakpoint', async () => {
  const res = {};
  for (const z of [0, -0.5, -1, -1.3, -1.5, -1.7, -2, -2.5]) {
    await ev(`${C} c.setViewport(700, 300, ${z}); return true;`); await sleep(500);
    res[z] = await ev(`${C} const n = c.nodes.get('t0'); const f = c.nodes.get('f1'); const g = c.nodes.get('g1');
      const ph = (n) => ({ cls: n.nodeEl.className, placeholder: !!n.nodeEl.querySelector('.canvas-node-placeholder'), content: n.nodeEl.querySelector('.canvas-node-content') ? n.nodeEl.querySelector('.canvas-node-content').innerHTML.slice(0, 160) : null, label: (() => { const l = n.nodeEl.querySelector('.canvas-node-label'); return l && getComputedStyle(l).display + ' ' + getComputedStyle(l).opacity; })() });
      return { scale: c.scale, t0: ph(n), f1: ph(f), g1: { cls: g.nodeEl.className, labelFont: getComputedStyle(g.nodeEl.querySelector('.canvas-group-label')).fontSize, labelTransform: getComputedStyle(g.nodeEl.querySelector('.canvas-group-label')).transform }, wrapperClasses: c.wrapperEl.className };`);
  }
  await ev(`${C} c.setViewport(700, 300, -1); return true;`);
  return res;
});

await step('placeholder-dom', async () => {
  await ev(`${C} c.setViewport(700, 300, -2.5); return true;`); await sleep(600);
  const r = await ev(`${C} const n = c.nodes.get('t0'); const f = c.nodes.get('f1');
    const dump = (n) => { const el = n.nodeEl.querySelector('.canvas-node-placeholder') || n.nodeEl.querySelector('.canvas-node-content'); if (!el) return null; const s = getComputedStyle(el); const inner = el.firstElementChild; const si = inner && getComputedStyle(inner); return { outer: el.outerHTML.slice(0, 400), font: s.fontSize, inner: inner && { cls: inner.className, font: si.fontSize, weight: si.fontWeight, lh: si.lineHeight, color: si.color, align: si.textAlign, overflow: si.overflow, wrap: si.whiteSpace, padding: si.padding }, display: s.display, justify: s.justifyContent, align: s.alignItems }; };
    return { scale: c.scale, t0: dump(n), f1: dump(f), t3: dump(c.nodes.get('t3')), l1: dump(c.nodes.get('l1')), f2: dump(c.nodes.get('f2')) };`);
  await ev(`${C} c.setViewport(700, 300, -1); return true;`);
  return r;
});

await step('selection-dom', () => ev(`${C} const n = c.nodes.get('t3'); c.selectOnly(n); c.requestFrame();
  return new Promise(res => requestAnimationFrame(() => setTimeout(() => {
    const cs = (e) => { const s = getComputedStyle(e); return { border: s.borderWidth + ' ' + s.borderColor, shadow: s.boxShadow, outline: s.outline, bg: s.backgroundColor, w: s.width, h: s.height, top: s.top, left: s.left, right: s.right, bottom: s.bottom, radius: s.borderRadius, transform: s.transform, cursor: s.cursor, opacity: s.opacity }; };
    const el = n.nodeEl;
    const all = (sel) => Array.from(el.querySelectorAll(sel)).map(e => ({ cls: e.className, style: cs(e) }));
    const focused = document.activeElement && document.activeElement.className;
    res({ cls: el.className, container: cs(el.querySelector('.canvas-node-container')), resizers: all('.canvas-node-resizer'), connections: all('.canvas-node-connection-point'), focused, menuRect: (() => { const m = document.querySelector('.canvas-menu'); if (!m) return null; const r = m.getBoundingClientRect(); const nr = el.getBoundingClientRect(); return { gapAboveNode: nr.top - r.bottom, w: r.width, h: r.height, centerOff: (r.left + r.width/2) - (nr.left + nr.width/2), style: cs(m) }; })() });
  }, 150)));`));

await step('history-and-save', async () => {
  const before = await ev(`${C} return { undo: c.history.data ? c.history.data.length : null, keys: Object.keys(c.history), current: c.history.current, mtime: app.vault.getAbstractFileByPath('Fixture.canvas').stat.mtime };`);
  const t0 = Date.now();
  await ev(`${C} const n = c.nodes.get('t3'); n.moveTo({ x: 760, y: 20 }); c.requestSave(); c.pushHistory(c.getData()); return true;`);
  let saved = null;
  for (let i = 0; i < 40; i++) { await sleep(250); const m = await ev(`return app.vault.getAbstractFileByPath('Fixture.canvas').stat.mtime;`); if (m !== before.mtime) { saved = Date.now() - t0; break; } }
  await ev(`${C} c.undo(); return true;`); await sleep(300);
  const after = await ev(`${C} const n = c.nodes.get('t3'); return { y: n.y, undo: c.history.data ? c.history.data.length : null, current: c.history.current };`);
  return { before, savedAfterMs: saved, after };
});

c.close();
console.log('done');
