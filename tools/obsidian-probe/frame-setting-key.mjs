// The "Window frame style" setting alone. Obsidian 1.14 opens Settings in a
// popout window, so its rows are not in the vault window's document; they are
// reached through `app.setting.activeTab.containerEl`, which the JS side can
// query whatever document it lives in. Records the setting's label, options
// and default, and the appearance.json bytes each option writes.
// Phases: fixtures | run.
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
const out = {};
const save = () => fs.writeFileSync(RESULTS, JSON.stringify(out, null, 1));

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('Alpha.md'), '# Alpha\n');
  process.exit(0);
}
if (phase === 'after-restart') process.exit(0);

const c = await connect();
out.vaultPath = await c.vaultPath();
await c.evaluate(`app.setting.open(); return true;`);
await sleep(1200);
// The setting is not in Appearance in 1.14: look through every tab for it, and stay on the one that has it.
out.byTab = {};
for (const tab of ['interface', 'appearance', 'about', 'editor', 'file']) {
  await c.evaluate(`app.setting.openTabById(${JSON.stringify(tab)}); return true;`);
  await sleep(900);
  const names = await c.evaluate(`const root = app.setting.activeTab && app.setting.activeTab.containerEl; return root ? Array.from(root.querySelectorAll('.setting-item-name')).map(n => n.textContent.trim()) : null;`);
  out.byTab[tab] = names;
  if (names && names.some((n) => /frame/i.test(n))) break;
}
save();
const ITEMS = `const root = app.setting.activeTab && app.setting.activeTab.containerEl; if (!root) return { noRoot: true, activeTab: app.setting.activeTab && app.setting.activeTab.id };
  const items = Array.from(root.querySelectorAll('.setting-item')).map(it => ({ name: ((it.querySelector('.setting-item-name') || {}).textContent || '').trim(), desc: ((it.querySelector('.setting-item-description') || {}).textContent || '').trim().slice(0, 200), heading: it.classList.contains('setting-item-heading'), control: (() => { const s = it.querySelector('select'); if (s) return { select: s.value, options: Array.from(s.options).map(o => ({ v: o.value, t: o.textContent })) }; const t = it.querySelector('.checkbox-container'); if (t) return { toggle: t.classList.contains('is-enabled') }; const i = it.querySelector('input'); if (i) return { input: i.value, type: i.type }; const b = it.querySelector('button'); return b ? { button: b.textContent } : null; })() }));
  return { ownerIsMain: root.ownerDocument === document, items };`;
out.first = await c.evaluate(ITEMS);
save();
const items = (out.first && out.first.items) || [];
const frame = items.find((s) => /frame/i.test(s.name));
out.frame = frame || null;
out.appearanceBefore = bytes('.obsidian/appearance.json');
out.tries = [];
if (frame && frame.control && frame.control.options) {
  const selSel = `Array.from(app.setting.activeTab.containerEl.querySelectorAll('.setting-item')).find(it => /frame/i.test(((it.querySelector('.setting-item-name')||{}).textContent)||'')).querySelector('select')`;
  const probeTitlebar = `const r = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; }; const cs = (el, p) => el ? getComputedStyle(el).getPropertyValue(p) : null; return { bar: r(document.querySelector('.titlebar')), barStyle: { height: cs(document.querySelector('.titlebar'), 'height'), bg: cs(document.querySelector('.titlebar'), 'background-color'), display: cs(document.querySelector('.titlebar'), 'display') }, text: (document.querySelector('.titlebar-text') || {}).textContent, textStyle: { fontSize: cs(document.querySelector('.titlebar-text'), 'font-size'), weight: cs(document.querySelector('.titlebar-text'), 'font-weight'), color: cs(document.querySelector('.titlebar-text'), 'color'), opacity: cs(document.querySelector('.titlebar-text'), 'opacity') }, buttons: Array.from(document.querySelectorAll('.titlebar-button')).map(b => ({ cls: b.className, aria: b.getAttribute('aria-label'), rect: r(b) })), rootHeader: r(document.querySelector('.mod-root .workspace-tab-header-container')), rootHeaderPadding: cs(document.querySelector('.mod-root .workspace-tab-header-container'), 'padding'), leftToggle: r(document.querySelector('.sidebar-toggle-button.mod-left')), rightToggle: r(document.querySelector('.sidebar-toggle-button.mod-right')), frameLeft: cs(document.body, '--frame-left-space'), frameRight: cs(document.body, '--frame-right-space'), ribbon: r(document.querySelector('.workspace-ribbon.mod-left')), leftHeader: r(document.querySelector('.mod-left-split .workspace-tab-header-container')), win: { w: innerWidth, h: innerHeight } };`;
  out.titlebarHidden = await c.evaluate(probeTitlebar);
  for (const opt of frame.control.options) {
    const before = bytes('.obsidian/appearance.json');
    await c.evaluate(`const el = ${selSel}; el.value = ${JSON.stringify(opt.v)}; el.dispatchEvent(new Event('change', { bubbles: true })); return el.value;`);
    const end = Date.now() + 3000; while (Date.now() < end && bytes('.obsidian/appearance.json') === before) await sleep(150);
    await sleep(1000);
    out.tries.push({ option: opt, appearanceJson: bytes('.obsidian/appearance.json'), bodyClass: await c.evaluate(`return document.body.className;`), notices: await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(n => n.textContent);`), titlebar: await c.evaluate(probeTitlebar), configKeys: await c.evaluate(`return Object.keys(app.vault.config || {});`), getConfig: await c.evaluate(`return Object.entries(app.vault.config || {});`) });
    save();
  }
  // Leave the last option on, quit and relaunch so after-restart can see the frame it gives.
  await c.evaluate(`app.setting.close(); return true;`);
}
save();
c.close();
process.exit(0);
