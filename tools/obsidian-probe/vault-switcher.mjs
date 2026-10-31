// The left sidebar's vault profile and the switcher it opens: what the
// footer row is made of, what clicking the vault's name opens, and — the
// question this block was written for — how a row labels a vault when two
// remembered vaults have the same folder name. The fixtures phase puts
// three extra vaults on `obsidian.json` (two named `vault` under different
// parents, one whose folder is gone) so the rows have to tell them apart.
// Phases: fixtures | run   (node vault-switcher.mjs <phase>)
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
const SHOTS = process.env.SHOTS || path.dirname(RESULTS);
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run'].includes(phase)) { console.error('phase must be fixtures|run'); process.exit(2); }

const CFG = path.join(os.homedir(), '.config/obsidian/obsidian.json');
const EXTRA = path.join(path.dirname(VAULT), 'silica-switcher-probe');

const T0 = Date.now();
const BUDGET_MS = 170000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.vaultSwitcher = all.vaultSwitcher || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.vaultSwitcher[phase] = out;
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
  fs.writeFileSync(path.join(VAULT, 'Welcome.md'), '# Welcome\n\nA note.\n');
  // Two live vaults sharing the folder name `vault`, and one that is gone.
  const made = [];
  for (const parent of ['one', 'two']) {
    const dir = path.join(EXTRA, parent, 'vault');
    fs.mkdirSync(path.join(dir, '.obsidian'), { recursive: true });
    fs.writeFileSync(path.join(dir, '.obsidian/app.json'), '{}');
    fs.writeFileSync(path.join(dir, 'Note.md'), '# Note\n');
    made.push(dir);
  }
  const gone = path.join(EXTRA, 'gone', 'vault');
  const doc = JSON.parse(fs.readFileSync(CFG, 'utf8'));
  const ids = ['aa11bb22cc33dd40', 'aa11bb22cc33dd41', 'aa11bb22cc33dd42'];
  const paths = [...made, gone];
  paths.forEach((p, i) => { doc.vaults[ids[i]] = { path: p, ts: 1792373000000 + i }; });
  fs.writeFileSync(CFG, JSON.stringify(doc));
  out.fixtures = { vault: VAULT, extra: paths, registry: doc.vaults };
  save();
  process.exit(0);
}

const c = await connect();
out.vaultPath = await c.vaultPath();
save();

const PROBE_JS = `
  const P = ['display','opacity','background-color','color','border-radius','padding','margin','width','height','align-items','justify-content','gap','position','font-size','font-weight','line-height','white-space','overflow','text-overflow','max-width','min-width','box-shadow','border'];
  const cs = (el, pseudo) => { if (!el) return null; const s = getComputedStyle(el, pseudo || null); const o = {}; for (const p of P) o[p] = s.getPropertyValue(p); return o; };
  const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x:+b.x.toFixed(2), y:+b.y.toFixed(2), w:+b.width.toFixed(2), h:+b.height.toFixed(2) }; };
  const one = (sel) => { const el = document.querySelector(sel); return el ? { sel, cls: el.className, aria: el.getAttribute('aria-label'), text: el.textContent, rect: rect(el), style: cs(el), html: el.outerHTML.slice(0, 2000) } : { sel, missing: true }; };
  const menus = () => Array.from(document.querySelectorAll('.menu')).map(m => ({
    cls: m.className, rect: rect(m), style: cs(m),
    items: Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map(it => ({
      cls: it.className,
      rect: rect(it),
      style: cs(it),
      title: (it.querySelector('.menu-item-title') || {}).textContent,
      titleHtml: (it.querySelector('.menu-item-title') || {}).innerHTML,
      titleStyle: cs(it.querySelector('.menu-item-title')),
      icon: (() => { const s = it.querySelector('.menu-item-icon svg'); return s && s.getAttribute('class'); })(),
      aria: it.getAttribute('aria-label'),
      text: it.textContent,
      html: it.outerHTML.slice(0, 1200),
    })),
  }));
`;

const q = (s) => JSON.stringify(s);

await step('footer', () => c.evaluate(`${PROBE_JS}
  const guesses = ['.workspace-sidedock-vault-profile', '.workspace-drawer-vault-switcher', '.side-dock-settings', '.workspace-sidedock-vault-profile .workspace-drawer-vault-name'];
  const footer = guesses.map(one);
  // Anything in the left split whose class mentions "vault".
  const anyVault = Array.from(document.querySelectorAll('.mod-left-split *')).filter(e => typeof e.className === 'string' && /vault/i.test(e.className)).map(e => ({ cls: e.className, text: e.textContent, rect: rect(e), html: e.outerHTML.slice(0, 1500) }));
  return { footer, anyVault, leftSplitTail: (() => { const l = document.querySelector('.mod-left-split'); return l ? Array.from(l.children).map(e => ({ cls: e.className, rect: rect(e) })) : null; })() };`));

await step('registry-as-obsidian-sees-it', () => c.evaluate(`
  let doc = null;
  try { doc = require('electron').ipcRenderer.sendSync('vault-list'); } catch (e) { doc = String(e); }
  return { doc };`));

await step('click-vault-name', async () => {
  const before = await c.evaluate(`return document.querySelectorAll('.menu').length;`);
  const sel = await c.evaluate(`
    const cands = ['.workspace-sidedock-vault-profile', '.workspace-drawer-vault-switcher', '.workspace-sidedock-vault-profile .workspace-drawer-vault-name'];
    for (const s of cands) if (document.querySelector(s)) return s;
    const el = Array.from(document.querySelectorAll('.mod-left-split *')).find(e => typeof e.className === 'string' && /vault-profile|vault-switcher/i.test(e.className));
    return el ? null : null;`);
  if (sel) await c.clickElement(sel);
  await sleep(600);
  const after = await c.evaluate(`${PROBE_JS} return { count: document.querySelectorAll('.menu').length, menus: menus() };`);
  return { clicked: sel, before, ...after };
});

await step('shot-menu', async () => {
  const r = await c.send('Page.captureScreenshot', { format: 'png' });
  const p = path.join(SHOTS, 'vault-switcher-menu.png');
  fs.mkdirSync(SHOTS, { recursive: true });
  fs.writeFileSync(p, Buffer.from(r.data, 'base64'));
  return { shot: p };
});

await step('escape', async () => { await c.pressKey('Escape'); await sleep(300); return c.evaluate(`return document.querySelectorAll('.menu').length;`); });

await step('command-app-open-vault', async () => {
  const windowsBefore = await c.evaluate(`try { return require('electron').remote ? null : null; } catch (e) { return null; }`);
  const ran = await c.evaluate(`return app.commands.executeCommandById('app:open-vault');`);
  await sleep(900);
  const after = await c.evaluate(`${PROBE_JS} return { count: document.querySelectorAll('.menu').length, menus: menus(), modal: one('.modal-container') };`);
  return { ran, windowsBefore, ...after };
});

await step('command-list', () => c.evaluate(`
  return Object.keys(app.commands.commands).filter(id => /vault/i.test(id)).map(id => ({ id, name: app.commands.commands[id].name }));`));

c.close();
save();
console.log('[vault-switcher] done');
