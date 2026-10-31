// The "Window frame style" setting: its label, options, default, what each
// option writes to appearance.json and what the title bar holds afterwards;
// the ribbon's classes and icons; and the light theme's shell colours.
// Companion to frame-typography.mjs, which stops at the hidden frame.
// Phases: fixtures | run   (node frame-settings.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 170000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.frameSettings = all.frameSettings || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.frameSettings[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
async function step(name, fn) {
  out.order.push(name);
  if (Date.now() - T0 > BUDGET_MS) { out.steps[name] = { ok: false, error: 'skipped: time budget exhausted' }; save(); return null; }
  const t = Date.now();
  const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save();
  return out.steps[name];
}
const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
async function waitForChange(rel, before, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (bytes(rel) !== before) return true; await sleep(150); }
  return false;
}

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('Alpha.md'), '# Alpha\n\nLinks to [[Beta]].\n');
  fs.writeFileSync(vp('Beta.md'), '# Beta\n\nBack to [[Alpha]].\n');
  out.wrote = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}
if (phase === 'after-restart') {
  // After a relaunch: which frame does the restored setting give?
  const c = await connect();
  out.vaultPath = await c.vaultPath();
  out.restart = await c.evaluate(`const r = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; }; return { bodyClass: document.body.className, appearance: ${JSON.stringify(bytes('.obsidian/appearance.json'))}, titlebar: r(document.querySelector('.titlebar')), text: (document.querySelector('.titlebar-text') || {}).textContent, buttons: Array.from(document.querySelectorAll('.titlebar-button')).map(b => ({ cls: b.className, rect: r(b) })), rootHeader: r(document.querySelector('.mod-root .workspace-tab-header-container')), rootHeaderPadding: getComputedStyle(document.querySelector('.mod-root .workspace-tab-header-container')).padding };`);
  save();
  c.close();
  process.exit(0);
}

const c = await connect();
out.vaultPath = await c.vaultPath();
save();

const PROBE_JS = `
  const cs = (el, props) => { if (!el) return null; const s = getComputedStyle(el); const o = {}; for (const p of props) o[p] = s.getPropertyValue(p); return o; };
  const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; };
  const svgName = (el) => { if (!el) return null; const s = el.querySelector('svg'); return s ? s.getAttribute('class') : null; };
`;

await step('s01-ribbon', async () => c.evaluate(PROBE_JS + `
  const act = document.querySelector('.side-dock-ribbon-action');
  const chain = []; let e = act; while (e && e !== document.body) { chain.push({ cls: e.className, rect: rect(e), style: cs(e, ['width', 'height', 'padding', 'margin', 'background-color', 'border-right', 'gap', 'flex-direction']) }); e = e.parentElement; }
  return { chain, actions: Array.from(document.querySelectorAll('.side-dock-ribbon-action')).map(a => ({ aria: a.getAttribute('aria-label'), icon: svgName(a), rect: rect(a) })), settings: Array.from(document.querySelectorAll('.side-dock-settings .clickable-icon, .side-dock-settings')).map(a => ({ cls: a.className, aria: a.getAttribute('aria-label'), icon: svgName(a), rect: rect(a) })) };
`));

async function appearanceItems() {
  for (let i = 0; i < 20; i++) {
    const items = await c.evaluate(`return Array.from(document.querySelectorAll('.modal.mod-settings .setting-item, .vertical-tab-content .setting-item, .modal-container .setting-item')).map(it => ({ name: ((it.querySelector('.setting-item-name') || {}).textContent || '').trim(), desc: ((it.querySelector('.setting-item-description') || {}).textContent || '').trim(), control: (() => { const s = it.querySelector('select'); if (s) return { select: s.value, options: Array.from(s.options).map(o => ({ v: o.value, t: o.textContent })) }; const t = it.querySelector('.checkbox-container'); if (t) return { toggle: t.classList.contains('is-enabled') }; const i = it.querySelector('input'); if (i) return { input: i.value, type: i.type }; const b = it.querySelector('button'); return b ? { button: b.textContent } : null; })() }));`);
    if (items.length) return items;
    await sleep(300);
  }
  return [];
}

await step('s02-frame-setting', async (r) => {
  await c.evaluate(`app.setting.open(); return true;`);
  await sleep(600);
  r.tabs = await c.evaluate(`return Array.from(document.querySelectorAll('.vertical-tab-nav-item')).map(t => t.textContent.trim());`);
  await c.evaluate(`app.setting.openTabById('appearance'); return true;`);
  await sleep(600);
  r.modalHtmlHead = await c.evaluate(`const m = document.querySelector('.modal-container'); return m ? m.outerHTML.slice(0, 600) : null;`);
  r.items = await appearanceItems();
  const frame = r.items.find((s) => /frame/i.test(s.name));
  if (!frame) throw new Error('no frame setting among ' + r.items.map((s) => s.name).join(' | '));
  r.frame = frame;
  r.before = bytes('.obsidian/appearance.json');
  const selSel = `Array.from(document.querySelectorAll('.modal-container .setting-item')).find(it => /frame/i.test(((it.querySelector('.setting-item-name')||{}).textContent)||'')).querySelector('select')`;
  r.tries = [];
  for (const opt of frame.control.options) {
    const before = bytes('.obsidian/appearance.json');
    await c.evaluate(`const el = ${selSel}; el.value = ${JSON.stringify(opt.v)}; el.dispatchEvent(new Event('change', { bubbles: true })); return el.value;`);
    await waitForChange('.obsidian/appearance.json', before, 3000);
    await sleep(800);
    r.tries.push({ option: opt, appearanceJson: bytes('.obsidian/appearance.json'), bodyClass: await c.evaluate(`return document.body.className;`), live: await c.evaluate(PROBE_JS + `return { bar: rect(document.querySelector('.titlebar')), barStyle: cs(document.querySelector('.titlebar'), ['height', 'background-color', 'display']), text: (document.querySelector('.titlebar-text')||{}).textContent, buttons: Array.from(document.querySelectorAll('.titlebar-button')).map(b => ({ cls: b.className, aria: b.getAttribute('aria-label'), rect: rect(b), icon: svgName(b) })), notices: Array.from(document.querySelectorAll('.notice')).map(n => n.textContent), rootHeader: rect(document.querySelector('.mod-root .workspace-tab-header-container')), rootHeaderPadding: getComputedStyle(document.querySelector('.mod-root .workspace-tab-header-container')).padding, leftToggle: rect(document.querySelector('.sidebar-toggle-button.mod-left')), rightToggle: rect(document.querySelector('.sidebar-toggle-button.mod-right')), frameLeft: getComputedStyle(document.body).getPropertyValue('--frame-left-space'), frameRight: getComputedStyle(document.body).getPropertyValue('--frame-right-space') };`) });
  }
  r.allConfigKeys = await c.evaluate(`return Object.keys(app.vault.config || {});`);
  return r;
});

await step('s03-light-theme', async (r) => {
  await c.evaluate(`app.setting.close(); return true;`);
  await sleep(300);
  const names = ['--background-primary', '--background-primary-alt', '--background-secondary', '--background-secondary-alt', '--background-modifier-border', '--background-modifier-hover', '--background-modifier-active-hover', '--text-normal', '--text-muted', '--text-faint', '--text-accent', '--text-accent-hover', '--text-on-accent', '--interactive-accent', '--interactive-normal', '--interactive-hover', '--tab-container-background', '--titlebar-background', '--titlebar-text-color', '--titlebar-text-color-focused', '--status-bar-background', '--status-bar-text-color', '--tab-text-color', '--tab-text-color-active', '--tab-text-color-focused', '--tab-text-color-focused-active', '--tab-background-active', '--sidebar-tab-text-color', '--sidebar-tab-text-color-active', '--sidebar-tab-background-active', '--nav-item-color', '--nav-item-color-hover', '--nav-item-color-active', '--nav-item-background-active', '--nav-item-background-hover', '--icon-color', '--icon-color-hover', '--icon-color-active', '--icon-color-focused', '--clickable-icon-radius', '--divider-color', '--divider-width', '--divider-color-hover', '--divider-width-hover', '--link-color', '--link-color-hover', '--tag-color', '--tag-background', '--code-background', '--text-highlight-bg', '--text-selection', '--scrollbar-thumb-bg', '--window-border-radius', '--titlebar-height', '--header-height', '--tab-radius-active', '--tab-curve', '--tab-width', '--tab-max-width', '--color-base-00', '--color-base-10', '--color-base-20', '--color-base-25', '--color-base-30', '--color-base-35', '--color-base-40', '--color-base-50', '--color-base-60', '--color-base-70', '--color-base-100', '--color-accent', '--color-accent-1', '--color-accent-2', '--accent-h', '--accent-s', '--accent-l'];
  const read = () => c.evaluate(`const s = getComputedStyle(document.body); const o = {}; for (const n of ${JSON.stringify(names)}) o[n] = s.getPropertyValue(n).trim(); return { bodyClass: document.body.className, vars: o, bodyBg: getComputedStyle(document.body).backgroundColor, bodyColor: getComputedStyle(document.body).color, tabBg: getComputedStyle(document.querySelector('.mod-root .workspace-tab-header-container')).backgroundColor, activeTabBg: getComputedStyle(document.querySelector('.mod-root .workspace-tab-header.is-active')).backgroundColor, rootBg: getComputedStyle(document.querySelector('.mod-root')).backgroundColor, leftBg: getComputedStyle(document.querySelector('.mod-left-split')).backgroundColor, border: getComputedStyle(document.querySelector('.mod-root .workspace-tab-header-container')).borderBottom, handleBorder: getComputedStyle(document.querySelector('.mod-left-split .workspace-leaf-resize-handle')).borderRight, statusBg: getComputedStyle(document.querySelector('.status-bar')).backgroundColor, statusBorder: getComputedStyle(document.querySelector('.status-bar')).borderLeft, statusRadius: getComputedStyle(document.querySelector('.status-bar')).borderRadius, navActive: (() => { const e = document.querySelector('.nav-file-title.is-active'); return e ? { bg: getComputedStyle(e).backgroundColor, color: getComputedStyle(e).color } : null; })(), ribbonBg: getComputedStyle(document.querySelector('.side-dock-ribbon-action').parentElement.parentElement).backgroundColor, ribbonBorder: getComputedStyle(document.querySelector('.side-dock-ribbon-action').parentElement.parentElement).borderRight };`);
  r.dark = await read();
  await c.evaluate(`app.changeTheme ? app.changeTheme('moonstone') : app.setTheme('moonstone'); return true;`);
  await sleep(800);
  r.light = await read();
  r.lightAppearance = bytes('.obsidian/appearance.json');
  await c.evaluate(`app.changeTheme ? app.changeTheme('obsidian') : app.setTheme('obsidian'); return true;`);
  await sleep(500);
  r.hover = await c.evaluate(`const s = getComputedStyle(document.body); return { hover: s.getPropertyValue('--background-modifier-hover'), activeHover: s.getPropertyValue('--background-modifier-active-hover'), iconHover: s.getPropertyValue('--icon-color-hover'), navHover: s.getPropertyValue('--nav-item-background-hover') };`);
  return r;
});

await step('s04-restart-with-obsidian-frame', async (r) => {
  // Leave the setting on the non-default option that keeps a frame of its own, then quit: after-restart reads the result.
  await c.evaluate(`app.setting.open(); app.setting.openTabById('appearance'); return true;`);
  await sleep(800);
  const items = await appearanceItems();
  const frame = items.find((s) => /frame/i.test(s.name));
  r.options = frame && frame.control.options;
  const target = frame && frame.control.options.find((o) => /obsidian/i.test(o.t)) || (frame && frame.control.options[1]);
  r.target = target;
  if (!target) throw new Error('no option to leave on');
  const selSel = `Array.from(document.querySelectorAll('.modal-container .setting-item')).find(it => /frame/i.test(((it.querySelector('.setting-item-name')||{}).textContent)||'')).querySelector('select')`;
  const before = bytes('.obsidian/appearance.json');
  await c.evaluate(`const el = ${selSel}; el.value = ${JSON.stringify(target.v)}; el.dispatchEvent(new Event('change', { bubbles: true })); return el.value;`);
  await waitForChange('.obsidian/appearance.json', before, 3000);
  r.appearance = bytes('.obsidian/appearance.json');
  await c.evaluate(`app.setting.close(); return true;`);
  await sleep(300);
  return r;
});

save();
await c.quit();
process.exit(0);
