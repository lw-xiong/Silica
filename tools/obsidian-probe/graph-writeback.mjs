// Block 9 of [[Interaction probes]]: `graph.json` write-back under real
// input. The 2026-10-18 run's sections were collapsed, so no filter checkbox
// or slider row existed to click and its "zero writes" proved nothing; this
// one expands every section first, clicks the checkboxes, drags the sliders
// with a real mouse, types in the search box, presses the buttons, and polls
// the file to measure when a write lands.
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
all.graphWriteback = all.graphWriteback || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.graphWriteback[phase] = out;
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

const GJ = '.obsidian/graph.json';
const gjRaw = () => { try { return fs.readFileSync(vp(GJ), 'utf8'); } catch { return null; } };
const gj = () => { const t = gjRaw(); try { return t === null ? null : JSON.parse(t); } catch { return { parseError: t }; } };
// Poll until the file changes, and say how long it took.
async function waitForWrite(before, timeoutMs = 8000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    const now = gjRaw();
    if (now !== before) return { changedAfterMs: Date.now() - t0, raw: now };
    await sleep(100);
  }
  return { changedAfterMs: null, raw: gjRaw() };
}

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.mkdirSync(vp('notes'), { recursive: true });
  fs.writeFileSync(vp('Hub.md'), '---\ntags:\n  - hub\n---\n\nLinks to [[A]], [[B]], [[notes/C]], [[Missing Note]] and ![[pic.png]].\n');
  fs.writeFileSync(vp('A.md'), '#alpha\n\nBack to [[Hub]].\n');
  fs.writeFileSync(vp('B.md'), '#beta\n\nBack to [[Hub]] and over to [[A]].\n');
  fs.writeFileSync(vp('notes/C.md'), 'A note in a folder, linking [[A]].\n');
  fs.writeFileSync(vp('Orphan.md'), 'Nothing links here and it links nowhere.\n');
  fs.writeFileSync(vp('pic.png'), Buffer.from('89504e470d0a1a0a', 'hex'));
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  // A hand-written graph.json with an unknown key, to see whether it survives.
  fs.writeFileSync(vp(GJ), JSON.stringify({
    'collapse-filter': true, search: '', showTags: false, showAttachments: false,
    hideUnresolved: false, showOrphans: true, 'collapse-color-groups': true, colorGroups: [],
    'collapse-display': true, showArrow: false, textFadeMultiplier: 0, nodeSizeMultiplier: 1,
    lineSizeMultiplier: 1, 'collapse-forces': true, centerStrength: 0.518713248970312,
    repelStrength: 10, linkStrength: 1, linkDistance: 250, scale: 1, close: false,
    silicaUnknownKey: 'please keep me',
  }, null, 2));
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, graph: true, 'command-palette': true, 'tag-pane': true }, null, 2));
  save(); process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
out.title = await c.evaluate(`return document.title;`);
out.graphJsonAtStart = gj();
save();

await c.evaluate(`
window.__g = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  leaf() { return app.workspace.getLeavesOfType('graph')[0]; },
  localLeaf() { return app.workspace.getLeavesOfType('localgraph')[0]; },
  view() { const l = this.leaf(); return l && l.view; },
  async openGlobal() {
    app.workspace.detachLeavesOfType('graph');
    app.workspace.detachLeavesOfType('localgraph');
    await this.sleep(400);
    app.commands.executeCommandById('graph:open');
    await this.sleep(1600);
    const v = this.view();
    return v ? { type: 'graph', scale: v.renderer && v.renderer.scale, nodes: v.renderer && v.renderer.nodes ? v.renderer.nodes.length : null } : null;
  },
  controls() {
    const v = this.view(); if (!v) return null;
    const el = v.containerEl;
    return Array.from(el.querySelectorAll('.tree-item.graph-control-section, .graph-controls .tree-item')).map((s) => ({
      title: (s.querySelector('.tree-item-inner') || {}).textContent,
      collapsed: s.classList.contains('is-collapsed'),
      cls: s.className,
      rect: this.rect(s.querySelector('.tree-item-self') || s),
      rows: Array.from(s.querySelectorAll('.setting-item')).map((it) => ({
        name: (it.querySelector('.setting-item-name') || {}).textContent,
        kind: (() => {
          if (it.querySelector('input[type="range"]')) return 'slider';
          if (it.querySelector('.checkbox-container')) return 'toggle';
          if (it.querySelector('input[type="search"], input[type="text"]')) return 'text';
          if (it.querySelector('button')) return 'button';
          return 'none';
        })(),
        value: (() => {
          const r = it.querySelector('input[type="range"]');
          if (r) return { value: r.value, min: r.min, max: r.max, step: r.step, rect: (() => { const b = r.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; })() };
          const t = it.querySelector('.checkbox-container');
          if (t) return { on: t.classList.contains('is-enabled'), rect: (() => { const b = t.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; })() };
          const x = it.querySelector('input[type="search"], input[type="text"]');
          if (x) return { value: x.value, placeholder: x.getAttribute('placeholder') };
          const b2 = it.querySelector('button');
          if (b2) return { text: b2.textContent, rect: (() => { const b = b2.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; })() };
          return null;
        })(),
      })),
    }));
  },
  rect(el) { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; },
  options() { const v = this.view(); return v && v.options ? JSON.parse(JSON.stringify(v.options)) : null; },
  engineOptions() {
    const v = this.view();
    const e = v && v.renderer;
    if (!e) return null;
    const o = {};
    for (const k of ['scale', 'panX', 'panY']) o[k] = e[k];
    return o;
  },
  canvasRect() {
    const v = this.view(); if (!v) return null;
    const cv = v.containerEl.querySelector('canvas');
    return this.rect(cv);
  },
};
return true;`);

const ev = (b) => c.evaluate(b);

await step('s01-open-and-expand-every-section', async (r) => {
  r.opened = await ev(`return await window.__g.openGlobal();`);
  r.graphJsonAfterOpen = gj();
  r.controlsCollapsed = await ev(`return window.__g.controls();`);
  // Click each section header to expand it.
  const sections = r.controlsCollapsed || [];
  r.expanded = [];
  for (let i = 0; i < sections.length; i++) {
    const s = await ev(`return window.__g.controls()[${i}];`);
    if (!s || !s.rect) continue;
    if (s.collapsed) { await c.clickAt(s.rect.cx, s.rect.cy); await sleep(600); }
    r.expanded.push(await ev(`return window.__g.controls()[${i}];`));
  }
  r.graphJsonAfterExpanding = gj();
  r.controlsNow = await ev(`return window.__g.controls();`);
  return r;
});

await step('s02-filter-checkboxes', async (r) => {
  r.rows = [];
  const sections = await ev(`return window.__g.controls();`);
  const toggles = [];
  sections.forEach((s, si) => s.rows.forEach((row, ri) => { if (row.kind === 'toggle') toggles.push({ si, ri, section: s.title, name: row.name, rect: row.value && row.value.rect, on: row.value && row.value.on }); }));
  r.toggleRows = toggles.map((t) => ({ section: t.section, name: t.name, on: t.on }));
  for (const t of toggles) {
    if (!t.rect) continue;
    const before = gjRaw();
    await c.clickAt(t.rect.cx, t.rect.cy);
    const w = await waitForWrite(before, 6000);
    const after = (() => { try { return JSON.parse(w.raw); } catch { return { parseError: w.raw }; } })();
    const beforeObj = (() => { try { return JSON.parse(before); } catch { return null; } })();
    const diff = {};
    if (beforeObj && after && !after.parseError) {
      for (const k of new Set([...Object.keys(beforeObj), ...Object.keys(after)])) {
        if (JSON.stringify(beforeObj[k]) !== JSON.stringify(after[k])) diff[k] = { from: beforeObj[k], to: after[k] };
      }
    }
    r.rows.push({ section: t.section, name: t.name, changedAfterMs: w.changedAfterMs, diff, viewOptions: await ev(`return window.__g.options();`) });
    await sleep(300);
  }
  return r;
});

await step('s03-sliders', async (r) => {
  r.rows = [];
  const sections = await ev(`return window.__g.controls();`);
  const sliders = [];
  sections.forEach((s) => s.rows.forEach((row) => { if (row.kind === 'slider') sliders.push({ section: s.title, name: row.name, ...row.value }); }));
  r.sliderRows = sliders.map((s) => ({ section: s.section, name: s.name, value: s.value, min: s.min, max: s.max, step: s.step }));
  for (const s of sliders) {
    if (!s.rect) continue;
    const before = gjRaw();
    // Drag the thumb to 25% along the track with a real mouse.
    const fromX = s.rect.x + s.rect.w * (Number(s.value) - Number(s.min)) / Math.max(1e-9, Number(s.max) - Number(s.min));
    const toX = s.rect.x + s.rect.w * 0.25;
    await c.drag(fromX, s.rect.cy, toX, s.rect.cy, { steps: 8, pauseMs: 60 });
    const w = await waitForWrite(before, 6000);
    const beforeObj = (() => { try { return JSON.parse(before); } catch { return null; } })();
    const after = (() => { try { return JSON.parse(w.raw); } catch { return { parseError: w.raw }; } })();
    const diff = {};
    if (beforeObj && after && !after.parseError) {
      for (const k of new Set([...Object.keys(beforeObj), ...Object.keys(after)])) {
        if (JSON.stringify(beforeObj[k]) !== JSON.stringify(after[k])) diff[k] = { from: beforeObj[k], to: after[k] };
      }
    }
    r.rows.push({ section: s.section, name: s.name, changedAfterMs: w.changedAfterMs, diff, newValue: await ev(`
      const s2 = window.__g.controls().flatMap((x) => x.rows).find((x) => x.name === ${JSON.stringify(s.name)});
      return s2 && s2.value ? s2.value.value : null;`) });
    await sleep(300);
  }
  return r;
});

await step('s04-search-box-and-buttons', async (r) => {
  const sections = await ev(`return window.__g.controls();`);
  const text = sections.flatMap((s) => s.rows.filter((x) => x.kind === 'text').map((x) => ({ section: s.title, name: x.name, value: x.value })));
  r.textRows = text;
  const inputRect = await ev(`
    const v = window.__g.view();
    const i = v.containerEl.querySelector('.graph-controls input[type="search"], .graph-controls input[type="text"], .search-input-container input');
    return i ? window.__g.rect(i) : null;`);
  r.inputRect = inputRect;
  if (inputRect) {
    const before = gjRaw();
    await c.clickAt(inputRect.cx, inputRect.cy);
    await sleep(300);
    await c.typeText('alpha');
    const w = await waitForWrite(before, 8000);
    r.afterTyping = { changedAfterMs: w.changedAfterMs, graphJson: (() => { try { return JSON.parse(w.raw); } catch { return { parseError: w.raw }; } })() };
    // Clear it again.
    await c.pressKey('a', { ctrl: true });
    await c.pressKey('Backspace');
    await sleep(2500);
    r.afterClearing = gj();
  }
  const buttons = sections.flatMap((s) => s.rows.filter((x) => x.kind === 'button').map((x) => ({ section: s.title, name: x.name, text: x.value && x.value.text, rect: x.value && x.value.rect })));
  r.buttonRows = buttons.map((b) => ({ section: b.section, name: b.name, text: b.text }));
  // The restore-defaults button, if there is one.
  const restore = buttons.find((b) => /restore|default/i.test((b.text || '') + (b.name || '')));
  if (restore && restore.rect) {
    const before = gjRaw();
    await c.clickAt(restore.rect.cx, restore.rect.cy);
    const w = await waitForWrite(before, 6000);
    r.afterRestore = { changedAfterMs: w.changedAfterMs, graphJson: (() => { try { return JSON.parse(w.raw); } catch { return { parseError: w.raw }; } })() };
  }
  return r;
});

await step('s05-collapse-again-and-zoom', async (r) => {
  // Re-collapsing a section: false -> true.
  const s0 = await ev(`return window.__g.controls()[0];`);
  if (s0 && s0.rect) {
    const before = gjRaw();
    await c.clickAt(s0.rect.cx, s0.rect.cy);
    const w = await waitForWrite(before, 6000);
    r.recollapse = { title: s0.title, changedAfterMs: w.changedAfterMs, collapsedNow: (await ev(`return window.__g.controls()[0];`)).collapsed, graphJson: (() => { try { return JSON.parse(w.raw); } catch { return { parseError: w.raw }; } })() };
  }
  // Wheel zoom to both limits, reading the renderer's scale.
  const canvas = await ev(`return window.__g.canvasRect();`);
  r.canvas = canvas;
  if (canvas) {
    r.zoomIn = [];
    for (let i = 0; i < 25; i++) {
      await c.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: Math.round(canvas.cx), y: Math.round(canvas.cy), deltaX: 0, deltaY: -120 });
      await sleep(60);
      if (i % 5 === 4) r.zoomIn.push(await ev(`return window.__g.engineOptions();`));
    }
    r.scaleAfterZoomIn = await ev(`return window.__g.engineOptions();`);
    const before = gjRaw();
    const w = await waitForWrite(before, 6000);
    r.graphJsonAfterZoomIn = { changedAfterMs: w.changedAfterMs, scale: (() => { try { return JSON.parse(w.raw).scale; } catch { return null; } })() };
    r.zoomOut = [];
    for (let i = 0; i < 40; i++) {
      await c.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: Math.round(canvas.cx), y: Math.round(canvas.cy), deltaX: 0, deltaY: 120 });
      await sleep(60);
      if (i % 8 === 7) r.zoomOut.push(await ev(`return window.__g.engineOptions();`));
    }
    r.scaleAfterZoomOut = await ev(`return window.__g.engineOptions();`);
    await sleep(2500);
    r.graphJsonAfterZoomOut = gj();
    // Pan, then close and reopen: does pan persist?
    await c.drag(canvas.cx, canvas.cy, canvas.cx + 120, canvas.cy + 80, { steps: 10, pauseMs: 50 });
    await sleep(2500);
    r.afterPan = { engine: await ev(`return window.__g.engineOptions();`), graphJson: gj() };
    r.afterReopen = await ev(`
      const u = window.__g;
      const got = await u.openGlobal();
      await u.sleep(1200);
      return { got, engine: u.engineOptions() };`);
  }
  return r;
});

await step('s06-unknown-key-and-external-change', async (r) => {
  r.unknownKeyStillThere = (gj() || {}).silicaUnknownKey || null;
  r.graphJsonNow = gj();
  // Rewrite the file from outside and see whether the app picks it up.
  const written = Object.assign({}, gj(), { scale: 3, showTags: true, silicaUnknownKey: 'rewritten from outside' });
  fs.writeFileSync(vp(GJ), JSON.stringify(written, null, 2));
  await sleep(3000);
  r.afterExternalWrite = {
    viewOptions: await ev(`return window.__g.options();`),
    engine: await ev(`return window.__g.engineOptions();`),
    fileNow: gj(),
  };
  r.afterReopenFollowingExternalWrite = await ev(`
    const u = window.__g;
    const got = await u.openGlobal();
    await u.sleep(1400);
    return { got, options: u.options(), engine: u.engineOptions() };`);
  await sleep(2000);
  r.fileAfterReopen = gj();
  return r;
});

await step('s07-local-graph-options', async (r) => {
  r.workspaceBefore = (() => { try { return JSON.parse(fs.readFileSync(vp('.obsidian/workspace.json'), 'utf8')); } catch { return null; } })();
  r.graphJsonBefore = gj();
  r.opened = await ev(`
    const u = window.__g;
    app.workspace.detachLeavesOfType('graph');
    await u.sleep(400);
    await app.workspace.openLinkText('Hub', '', false);
    await u.sleep(500);
    app.commands.executeCommandById('graph:open-local');
    await u.sleep(1800);
    const l = u.localLeaf();
    return l ? { state: JSON.parse(JSON.stringify(l.getViewState())), scale: l.view.renderer && l.view.renderer.scale } : null;`);
  // Expand the local graph's sections and toggle one option.
  r.localControls = await ev(`
    const u = window.__g;
    const l = u.localLeaf(); if (!l) return null;
    const el = l.view.containerEl;
    const secs = Array.from(el.querySelectorAll('.tree-item'));
    for (const s of secs) {
      if (s.classList.contains('is-collapsed')) {
        const self = s.querySelector('.tree-item-self');
        if (self) self.click();
        await u.sleep(300);
      }
    }
    await u.sleep(400);
    return Array.from(el.querySelectorAll('.setting-item')).map((it) => ({
      name: (it.querySelector('.setting-item-name') || {}).textContent,
      kind: it.querySelector('input[type="range"]') ? 'slider' : it.querySelector('.checkbox-container') ? 'toggle' : it.querySelector('input') ? 'text' : it.querySelector('button') ? 'button' : 'none',
      rect: (() => { const t = it.querySelector('.checkbox-container') || it.querySelector('input[type="range"]'); if (!t) return null; const b = t.getBoundingClientRect(); return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; })(),
      on: (() => { const t = it.querySelector('.checkbox-container'); return t ? t.classList.contains('is-enabled') : null; })(),
    }));`);
  const tgl = (r.localControls || []).find((x) => x.kind === 'toggle' && x.rect);
  if (tgl) {
    const before = gjRaw();
    await c.clickAt(tgl.rect.cx, tgl.rect.cy);
    const w = await waitForWrite(before, 6000);
    r.afterLocalToggle = {
      name: tgl.name, graphJsonChangedAfterMs: w.changedAfterMs,
      graphJson: gj(),
      leafState: await ev(`const l = window.__g.localLeaf(); return l ? JSON.parse(JSON.stringify(l.getViewState())) : null;`),
    };
  }
  await sleep(2500);
  r.workspaceAfter = (() => { try { return JSON.parse(fs.readFileSync(vp('.obsidian/workspace.json'), 'utf8')); } catch { return null; } })();
  r.localOptionsInWorkspace = (() => {
    const s = JSON.stringify(r.workspaceAfter || {});
    return { mentionsLocalgraph: s.includes('localgraph'), mentionsLocalJumps: s.includes('localJumps') };
  })();
  return r;
});

// The first run's Forces sliders sat at y 851-1072 in an 800 px window, so
// the drag hit nothing and their four rows read as "no write". This step
// scrolls each one into view first.
await step('s08-forces-sliders-scrolled-into-view', async (r) => {
  r.opened = await ev(`
    const u = window.__g;
    const got = await u.openGlobal();
    await u.sleep(600);
    const el = u.view().containerEl;
    for (const sec of Array.from(el.querySelectorAll('.tree-item'))) {
      if (sec.classList.contains('is-collapsed')) { const self = sec.querySelector('.tree-item-self'); if (self) self.click(); await u.sleep(250); }
    }
    await u.sleep(500);
    return got;`);
  const NAMES = ['Center force', 'Repel force', 'Link force', 'Link distance', 'Text fade threshold'];
  r.rows = [];
  for (const name of NAMES) {
    const info = await ev(`
      const u = window.__g;
      const el = u.view().containerEl;
      const it = Array.from(el.querySelectorAll('.setting-item')).find((x) => ((x.querySelector('.setting-item-name') || {}).textContent || '') === ${JSON.stringify('NAME')}.replace('NAME', ${JSON.stringify(name)}));
      if (!it) return null;
      const range = it.querySelector('input[type="range"]');
      if (!range) return null;
      it.scrollIntoView({ block: 'center' });
      await u.sleep(400);
      const b = range.getBoundingClientRect();
      return { value: range.value, min: range.min, max: range.max, step: range.step, rect: { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }, inView: b.y > 0 && b.y < window.innerHeight };`);
    if (!info || !info.inView) { r.rows.push({ name, skipped: 'not in view', info }); continue; }
    const before = gjRaw();
    const frac = (Number(info.value) - Number(info.min)) / Math.max(1e-9, Number(info.max) - Number(info.min));
    const fromX = info.rect.x + info.rect.w * frac;
    const toX = info.rect.x + info.rect.w * (frac > 0.5 ? 0.2 : 0.8);
    await c.drag(fromX, info.rect.cy, toX, info.rect.cy, { steps: 8, pauseMs: 60 });
    const w = await waitForWrite(before, 6000);
    const beforeObj = (() => { try { return JSON.parse(before); } catch { return null; } })();
    const after = (() => { try { return JSON.parse(w.raw); } catch { return { parseError: w.raw }; } })();
    const diff = {};
    if (beforeObj && after && !after.parseError) {
      for (const k of new Set([...Object.keys(beforeObj), ...Object.keys(after)])) {
        if (JSON.stringify(beforeObj[k]) !== JSON.stringify(after[k])) diff[k] = { from: beforeObj[k], to: after[k] };
      }
    }
    r.rows.push({ name, min: info.min, max: info.max, step: info.step, from: info.value, changedAfterMs: w.changedAfterMs, diff });
    await sleep(300);
  }
  return r;
});

out.graphJsonAtEnd = gj();
out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
