// The root tab strip's hover, driven over CDP: which box the hover wash paints
// (the 200 by 34 header or the inner box it pads), its colour, radius and
// inset, what hovering does to the title, the close button and the dividers
// of the hovered tab and of its neighbours, the same for the active tab and
// for a dock tab, in both themes; plus a screenshot of the strip per state.
// Phases: fixtures | run   (node tab-hover.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
const SHOTS = process.env.SHOTS || path.dirname(RESULTS);
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 170000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.tabHover = all.tabHover || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.tabHover[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
async function step(name, fn) {
  out.order.push(name);
  if (Date.now() - T0 > BUDGET_MS) { out.steps[name] = { ok: false, error: 'skipped: time budget exhausted' }; save(); return null; }
  const t = Date.now();
  try { const v = await fn(); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e) }; }
  save();
  return out.steps[name];
}

if (phase === 'fixtures') {
  for (const n of ['Alpha', 'Beta', 'Gamma', 'Delta']) fs.writeFileSync(path.join(VAULT, n + '.md'), `# ${n}\n\nA note.\n`);
  out.fixtures = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}

const c = await connect();
out.vaultPath = await c.vaultPath();
out.version = await c.evaluate(`return { ua: navigator.userAgent, api: (window.app && app.apiVersion) || null };`);
save();

// Everything hover can change, per tab: the header, its inner box, the title,
// the close button, and the ::before/::after of the header and the inner
// (where a divider drawn in CSS would live).
const PROBE_JS = `
  const P = ['display', 'visibility', 'opacity', 'background-color', 'color', 'border-radius', 'padding', 'margin', 'width', 'height', 'top', 'left', 'right', 'bottom', 'position', 'content', 'transition', 'box-shadow', 'border', 'gap'];
  const cs = (el, pseudo) => { if (!el) return null; const s = getComputedStyle(el, pseudo || null); const o = {}; for (const p of P) o[p] = s.getPropertyValue(p); return o; };
  const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x: +b.x.toFixed(2), y: +b.y.toFixed(2), w: +b.width.toFixed(2), h: +b.height.toFixed(2) }; };
  const tabs = (root) => Array.from(document.querySelectorAll(root + ' .workspace-tab-header')).map(h => ({
    cls: h.className, aria: h.getAttribute('aria-label'), rect: rect(h), style: cs(h), before: cs(h, '::before'), after: cs(h, '::after'),
    inner: (() => { const i = h.querySelector('.workspace-tab-header-inner'); return i && { rect: rect(i), style: cs(i), before: cs(i, '::before'), after: cs(i, '::after') }; })(),
    title: (() => { const t = h.querySelector('.workspace-tab-header-inner-title'); return t && { text: t.textContent, rect: rect(t), style: cs(t) }; })(),
    close: (() => { const b = h.querySelector('.workspace-tab-header-inner-close-button'); return b && { rect: rect(b), style: cs(b), svg: cs(b.querySelector('svg')) }; })(),
    icon: (() => { const b = h.querySelector('.workspace-tab-header-inner-icon'); return b && { rect: rect(b), style: cs(b) }; })(),
    hovered: h.matches(':hover'),
  }));
  const vars = (el, names) => { const s = getComputedStyle(el); const o = {}; for (const n of names) o[n] = s.getPropertyValue(n).trim(); return o; };
  const VARS = ['--tab-radius', '--tab-curve', '--tab-background-active', '--tab-text-color', '--tab-text-color-active', '--tab-text-color-focused', '--tab-text-color-focused-active', '--tab-text-color-focused-highlighted', '--tab-text-color-focused-active-current', '--tab-divider-color', '--tab-outline-color', '--tab-container-background', '--background-modifier-hover', '--background-modifier-active-hover', '--background-modifier-border', '--background-secondary', '--background-primary', '--tab-width', '--tab-max-width', '--tab-font-size', '--clickable-icon-radius', '--sidebar-tab-text-color', '--sidebar-tab-text-color-active', '--sidebar-tab-background-active'];
  const snap = () => ({ root: tabs('.mod-root'), left: tabs('.mod-left-split'), vars: vars(document.querySelector('.mod-root .workspace-tab-header') || document.body, VARS), theme: document.body.className });
`;

const shot = async (name) => {
  const r = await c.evaluate(`const el = document.querySelector('.mod-root .workspace-tab-header-container'); const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, dpr: window.devicePixelRatio };`);
  const png = await c.send('Page.captureScreenshot', { format: 'png', clip: { x: r.x, y: r.y, width: r.w, height: r.h, scale: 2 } });
  fs.mkdirSync(SHOTS, { recursive: true });
  const file = path.join(SHOTS, `tab-hover-${name}.png`);
  fs.writeFileSync(file, Buffer.from(png.data, 'base64'));
  return { file, clip: r };
};

// Four tabs, the third active: tab 1 then carries a divider, tab 3 (after the
// active one) does not, tab 0 never does.
await step('s01-open', async () => {
  await c.evaluate(`
    app.workspace.leftSplit.expand(); app.workspace.rightSplit.collapse();
    for (const n of ['Alpha', 'Beta', 'Gamma', 'Delta']) { const f = app.vault.getAbstractFileByPath(n + '.md'); await app.workspace.getLeaf('tab').openFile(f); }
    const leaves = []; app.workspace.iterateRootLeaves(l => leaves.push(l));
    for (const l of leaves) if (!l.view.file) l.detach();
    const rest = []; app.workspace.iterateRootLeaves(l => rest.push(l));
    app.workspace.setActiveLeaf(rest[2], { focus: true });
    return rest.map(l => l.view.file && l.view.file.path);`);
  await sleep(600);
  // Park the pointer in the editor so nothing in the strip is hovered.
  const ed = await c.rect('.mod-root .view-content');
  await c.mouseMove(ed.cx, ed.cy);
  await sleep(200);
  return { tabs: await c.evaluate(`return Array.from(document.querySelectorAll('.mod-root .workspace-tab-header')).map(h => h.getAttribute('aria-label') + (h.classList.contains('is-active') ? ' *' : ''));`) };
});

const states = async (tag) => {
  await step(`${tag}-rest`, async () => { const ed = await c.rect('.mod-root .view-content'); await c.mouseMove(ed.cx, ed.cy); await sleep(300); return { ...(await c.evaluate(PROBE_JS + 'return snap();')), shot: await shot(`${tag}-rest`) }; });
  for (const ix of [0, 1, 2, 3]) {
    await step(`${tag}-hover-tab${ix}`, async () => {
      const r = await c.hover('.mod-root .workspace-tab-header', ix);
      await sleep(400);
      return { pointer: r, ...(await c.evaluate(PROBE_JS + 'return snap();')), shot: await shot(`${tag}-hover-tab${ix}`) };
    });
  }
  await step(`${tag}-hover-close1`, async () => {
    // The close button sits inside the hovered tab; hover the tab first so it shows.
    await c.hover('.mod-root .workspace-tab-header', 1); await sleep(300);
    const r = await c.hover('.mod-root .workspace-tab-header-inner-close-button', 1);
    await sleep(400);
    return { pointer: r, ...(await c.evaluate(PROBE_JS + 'return snap();')), shot: await shot(`${tag}-hover-close1`) };
  });
  await step(`${tag}-hover-dock1`, async () => {
    const r = await c.hover('.mod-left-split .workspace-tab-header', 1);
    await sleep(400);
    return { pointer: r, ...(await c.evaluate(PROBE_JS + 'return snap();')) };
  });
};

await states('dark');
await step('s02-light', async () => { await c.evaluate(`app.changeTheme('moonstone'); return document.body.className;`); await sleep(800); return true; });
await states('light');
await step('s03-dark-again', async () => { await c.evaluate(`app.changeTheme('obsidian'); return document.body.className;`); await sleep(300); return true; });

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
