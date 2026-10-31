// The vault-manager window: what "Manage vaults..." raises, which the
// switcher probe of 2026-10-29 could only tell was *not* a modal. It is a
// window of its own, so it is a CDP page target of its own with no `app`
// in it — `cdp.mjs`'s `connect()` waits for a booted vault window and
// cannot see it. This script drives the vault window to open it, then
// attaches to the new target by hand and reads it: the list of remembered
// vaults on the left (is a vault whose folder is gone listed, and how?),
// the per-vault menu behind the row's ellipsis, and the panel on the
// right — its rows, their explanations and their buttons.
// Phases: fixtures | run   (node vault-manager.mjs <phase>)
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
const SHOTS = process.env.SHOTS || path.dirname(RESULTS);
const BASE = process.env.CDP_BASE || 'http://127.0.0.1:9222';
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run'].includes(phase)) { console.error('phase must be fixtures|run'); process.exit(2); }

const CFG = path.join(os.homedir(), '.config/obsidian/obsidian.json');
const EXTRA = path.join(path.dirname(VAULT), 'silica-manager-probe');

const T0 = Date.now();
const BUDGET_MS = 170000;
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.vaultManager = all.vaultManager || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.vaultManager[phase] = out;
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
  // Two live vaults and one whose folder is gone, so the list has to say
  // what it does with each.
  const made = [];
  for (const name of ['Field notes', 'archive']) {
    const dir = path.join(EXTRA, name);
    fs.mkdirSync(path.join(dir, '.obsidian'), { recursive: true });
    fs.writeFileSync(path.join(dir, '.obsidian/app.json'), '{}');
    fs.writeFileSync(path.join(dir, 'Note.md'), '# Note\n');
    made.push(dir);
  }
  const gone = path.join(EXTRA, 'deleted-vault');
  const doc = JSON.parse(fs.readFileSync(CFG, 'utf8'));
  const ids = ['bb11cc22dd33ee40', 'bb11cc22dd33ee41', 'bb11cc22dd33ee42'];
  const paths = [...made, gone];
  paths.forEach((p, i) => { doc.vaults[ids[i]] = { path: p, ts: 1792373000000 + i }; });
  fs.writeFileSync(CFG, JSON.stringify(doc));
  out.fixtures = { vault: VAULT, extra: paths, registry: doc.vaults };
  save();
  process.exit(0);
}

// ---- a bare CDP client for a page with no `app` in it ---------------------
async function pageTargets() {
  const ts = await (await fetch(`${BASE}/json`)).json();
  return ts.filter((x) => x.type === 'page');
}

async function attach(wsUrl) {
  const ws = new WebSocket(wsUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 0;
  const pending = new Map();
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
      const { res, rej } = pending.get(m.id);
      pending.delete(m.id);
      m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result);
    }
  };
  const send = (method, params = {}) => new Promise((res, rej) => {
    const i = ++id;
    pending.set(i, { res, rej });
    ws.send(JSON.stringify({ id: i, method, params }));
    setTimeout(() => { if (pending.has(i)) { pending.delete(i); rej(new Error(method + ' timeout')); } }, 15000);
  });
  const evaluate = async (expr) => {
    const r = await send('Runtime.evaluate', { expression: `(async () => { ${expr} })()`, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) {
      const d = r.exceptionDetails;
      throw new Error((d.exception && d.exception.description) || d.text || JSON.stringify(d).slice(0, 400));
    }
    return r.result.value;
  };
  const clickAt = async (x, y) => {
    for (const type of ['mousePressed', 'mouseReleased']) {
      await send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, buttons: type === 'mousePressed' ? 1 : 0 });
      await sleep(40);
    }
  };
  const clickSel = async (sel, index = 0) => {
    const r = await evaluate(`
      const els = document.querySelectorAll(${JSON.stringify(sel)});
      const el = els[${index}]; if (!el) return null;
      const b = el.getBoundingClientRect();
      return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
    if (!r) return null;
    await clickAt(r.cx, r.cy);
    return r;
  };
  const shot = async (name) => {
    const r = await send('Page.captureScreenshot', { format: 'png' });
    fs.mkdirSync(SHOTS, { recursive: true });
    const p = path.join(SHOTS, name);
    fs.writeFileSync(p, Buffer.from(r.data, 'base64'));
    return p;
  };
  return { send, evaluate, clickAt, clickSel, shot, close: () => ws.close() };
}

// What to read off an element, in the shape the other probes record.
const PROBE_JS = `
  const P = ['display','opacity','background-color','color','border-radius','padding','margin','width','height','align-items','justify-content','gap','position','font-size','font-weight','line-height','white-space','overflow','text-overflow','word-break','max-width','min-width','box-shadow','border','flex-direction'];
  const cs = (el) => { if (!el) return null; const s = getComputedStyle(el); const o = {}; for (const p of P) o[p] = s.getPropertyValue(p); return o; };
  const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x:+b.x.toFixed(2), y:+b.y.toFixed(2), w:+b.width.toFixed(2), h:+b.height.toFixed(2) }; };
  const dump = (el) => el ? { tag: el.tagName, cls: el.className, aria: el.getAttribute('aria-label'), text: (el.textContent || '').slice(0, 300), rect: rect(el), style: cs(el) } : null;
  const all = (sel) => Array.from(document.querySelectorAll(sel)).map(dump);
  // The whole tree, trimmed: tag, class and own text, to depth 8.
  const tree = (el, depth = 0) => !el || depth > 8 ? null : ({
    tag: el.tagName, cls: typeof el.className === 'string' ? el.className : '', rect: rect(el),
    own: Array.from(el.childNodes).filter(n => n.nodeType === 3).map(n => n.textContent.trim()).filter(Boolean).join(' ').slice(0, 200),
    kids: Array.from(el.children).map(k => tree(k, depth + 1)).filter(Boolean),
  });
`;

const vaultWindow = await connect();
out.vaultPath = await vaultWindow.vaultPath();
save();

await step('targets-before', async () => ({ targets: (await pageTargets()).map((t) => ({ title: t.title, url: t.url })) }));

// The two commands named for this window, before anything is clicked:
// "Manage vaults" (`app:open-vault`) and "Change vault..."
// (`app:switch-vault`). Neither leaves a modal behind (switcher probe of
// 2026-10-29), so what each does is a question of menus and windows.
await step('command-switch-vault', async () => {
  const ran = await vaultWindow.evaluate(`return app.commands.executeCommandById('app:switch-vault');`);
  await sleep(1200);
  const menus = await vaultWindow.evaluate(`
    const ms = Array.from(document.querySelectorAll('.menu'));
    return ms.map(m => Array.from(m.querySelectorAll('.menu-item')).map(i => i.textContent));`);
  const targets = (await pageTargets()).map((t) => ({ title: t.title, url: t.url }));
  await vaultWindow.pressKey('Escape');
  await sleep(300);
  return { ran, menus, targets };
});

await step('command-open-vault', async () => {
  const ran = await vaultWindow.evaluate(`return app.commands.executeCommandById('app:open-vault');`);
  await sleep(2000);
  const menus = await vaultWindow.evaluate(`
    const ms = Array.from(document.querySelectorAll('.menu'));
    return ms.map(m => Array.from(m.querySelectorAll('.menu-item')).map(i => i.textContent));`);
  const targets = (await pageTargets()).map((t) => ({ title: t.title, url: t.url }));
  return { ran, menus, targets, raisedStarter: targets.some((t) => /starter/.test(t.url)) };
});

// The switcher menu next: the row that raises the window, and its icon.
await step('switcher-menu', async () => {
  await vaultWindow.clickElement('.workspace-drawer-vault-switcher');
  await sleep(600);
  return vaultWindow.evaluate(`
    const menus = Array.from(document.querySelectorAll('.menu'));
    return { menus: menus.map(m => Array.from(m.querySelectorAll('.menu-item, .menu-separator')).map(it => ({
      cls: it.className,
      title: (it.querySelector('.menu-item-title') || {}).textContent,
      icon: (() => { const s = it.querySelector('.menu-item-icon svg'); return s && s.getAttribute('class'); })(),
      html: it.outerHTML.slice(0, 600),
    }))) };`);
});

await step('click-manage-vaults', async () => {
  const hit = await vaultWindow.evaluate(`
    const items = Array.from(document.querySelectorAll('.menu .menu-item'));
    const it = items.find(e => /manage vaults/i.test(e.textContent || ''));
    if (!it) return null;
    const b = it.getBoundingClientRect();
    return { cx: b.x + b.width / 2, cy: b.y + b.height / 2, text: it.textContent };`);
  if (!hit) throw new Error('no "Manage vaults..." item in the menu');
  await vaultWindow.clickAt(hit.cx, hit.cy);
  await sleep(2500);
  return { hit, targets: (await pageTargets()).map((t) => ({ title: t.title, url: t.url })) };
});

// The new target is the one that is not the vault window.
const after = await pageTargets();
let manager = null;
for (const t of after) {
  try {
    const c = await attach(t.webSocketDebuggerUrl);
    const kind = await c.evaluate(`return { hasApp: typeof app !== 'undefined', url: location.href, title: document.title, body: document.body ? document.body.className : null };`);
    if (!kind.hasApp) { manager = { target: t, client: c, kind }; break; }
    c.close();
  } catch { /* try the next */ }
}
out.managerFound = manager ? { url: manager.kind.url, title: manager.kind.title, body: manager.kind.body } : null;
save();

if (manager) {
  const m = manager.client;
  await step('window-shape', () => m.evaluate(`${PROBE_JS}
    return {
      size: { w: window.innerWidth, h: window.innerHeight, outerW: window.outerWidth, outerH: window.outerHeight, dpr: devicePixelRatio },
      title: document.title,
      bodyClass: document.body.className,
      tree: tree(document.body),
    };`));

  await step('vault-list', () => m.evaluate(`${PROBE_JS}
    // Everything that looks like a row of the remembered-vault list.
    const guesses = ['.modal-vault-list .modal-vault', '.modal-vault', '[class*="vault-list"] > *', '.starter-vault'];
    const found = {};
    for (const g of guesses) found[g] = all(g);
    return { found, html: document.body.outerHTML.slice(0, 20000) };`));

  await step('shot-manager', () => m.shot('vault-manager.png'));

  // What order the rows are in, against the document the window was built
  // from: key order, `ts`, and which folders are actually there. Read from
  // node, not the renderer, so it is the file on disk at this moment.
  await step('order-against-registry', async () => {
    const rows = await m.evaluate(`
      return Array.from(document.querySelectorAll('.recent-vaults-list-item')).map(e => ({
        name: (e.querySelector('.recent-vaults-list-item-name') || {}).textContent,
        parent: (e.querySelector('.recent-vaults-list-item-path') || {}).textContent,
        full: e.getAttribute('aria-label'),
      }));`);
    const doc = JSON.parse(fs.readFileSync(CFG, 'utf8'));
    const entries = Object.entries(doc.vaults).map(([id, v], ix) => ({
      id, keyIndex: ix, path: v.path, ts: v.ts, open: !!v.open,
      exists: fs.existsSync(v.path),
      name: path.basename(v.path),
    }));
    const byTs = [...entries].filter((e) => e.exists).sort((a, b) => b.ts - a.ts).map((e) => e.name);
    const byKey = entries.filter((e) => e.exists).map((e) => e.name);
    const shown = rows.map((r) => r.name);
    return {
      rows, entries, shown,
      tsDescending: byTs, keyOrder: byKey,
      matchesTsDescending: JSON.stringify(shown) === JSON.stringify(byTs),
      matchesTsAscending: JSON.stringify(shown) === JSON.stringify([...byTs].reverse()),
      matchesKeyOrder: JSON.stringify(shown) === JSON.stringify(byKey),
      matchesKeyOrderReversed: JSON.stringify(shown) === JSON.stringify([...byKey].reverse()),
      missingOmitted: entries.filter((e) => !e.exists).map((e) => e.name),
    };
  });

  await step('row-menu', async () => {
    // The ellipsis on the first row.
    const dots = await m.evaluate(`${PROBE_JS}
      const cands = Array.from(document.querySelectorAll('div,button,span')).filter(e => {
        const c = typeof e.className === 'string' ? e.className : '';
        return /option|more|ellipsis|menu|action/i.test(c) && e.getBoundingClientRect().width > 0 && e.getBoundingClientRect().width < 60;
      });
      return cands.map(dump);`);
    const first = (dots || [])[0];
    if (!first) return { dots, clicked: null };
    await m.clickAt(first.rect.x + first.rect.w / 2, first.rect.y + first.rect.h / 2);
    await sleep(800);
    const menu = await m.evaluate(`${PROBE_JS}
      const menus = Array.from(document.querySelectorAll('.menu'));
      return { count: menus.length, items: menus.map(mm => Array.from(mm.querySelectorAll('.menu-item, .menu-separator')).map(it => ({
        cls: it.className,
        title: (it.querySelector('.menu-item-title') || {}).textContent,
        text: (it.textContent || '').trim(),
        icon: (() => { const s = it.querySelector('.menu-item-icon svg'); return s && s.getAttribute('class'); })(),
      }))) };`);
    return { dots, clicked: first, menu, shot: await m.shot('vault-manager-row-menu.png') };
  });

  await step('buttons', () => m.evaluate(`${PROBE_JS}
    return { buttons: all('button'), setting: all('.setting-item').map((s, i) => s), selects: all('select').map(s => s) };`));

  await step('close-row-menu', async () => { await m.clickAt(10, 10); await sleep(300); return m.evaluate(`return document.querySelectorAll('.menu').length;`); });

  manager.client.close();
} else {
  await step('no-manager-target', async () => ({ targets: after.map((t) => ({ title: t.title, url: t.url })) }));
}

vaultWindow.close();
save();
console.log('[vault-manager] done');
