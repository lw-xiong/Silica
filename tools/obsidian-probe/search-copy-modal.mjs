// The "Copy search results" modal: its controls, every format combination, and what the Copy button writes.
// Supersedes search.mjs step 09, which clicked the item without looking for the modal it opens.
import fs from 'node:fs'; import path from 'node:path';
import { connect, sleep } from './cdp.mjs';
const phase = process.argv[2]; const VAULT = process.env.VAULT; const RESULTS = process.env.RESULTS;
const out = { phase, steps: {} };
const save = () => { let all = {}; try { all = JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch {} all.copyModal2 = all.copyModal2 || {}; all.copyModal2[phase] = out; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
if (phase === 'fixtures') {
  const FIX = { 'apple.md': 'apple needle one\n', 'notes/banana.md': 'banana has a needle in a folder\n', 'mango.md': 'mango needle first\nno hit\nmango needle second\n', 'Apple Pie.md': 'apple pie needle\n', 'plain.txt': 'plain text needle\n' };
  for (const [rel, text] of Object.entries(FIX)) { const p = path.join(VAULT, rel); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, text); }
  out.steps.fixtures = { ok: true, files: FIX }; save(); process.exit(0);
}
if (phase !== 'run') process.exit(0);
const c = await connect();
const step = async (name, fn) => { try { out.steps[name] = { ok: true, ...(await fn()) }; } catch (e) { out.steps[name] = { ok: false, error: String(e).slice(0, 600) }; } save(); };
const MODAL = `
  const m = document.querySelector('.modal-container .modal'); if (!m) return null;
  const q = (s) => Array.from(m.querySelectorAll(s));
  return {
    title: (m.querySelector('.modal-title') || {}).textContent || null,
    cls: m.className,
    settings: q('.setting-item').map(si => ({
      name: (si.querySelector('.setting-item-name') || {}).textContent || '',
      desc: (si.querySelector('.setting-item-description') || {}).textContent || '',
      control: (() => { const cb = si.querySelector('input[type=checkbox]'); if (cb) return { type: 'toggle', checked: cb.checked, containerCls: (cb.closest('.checkbox-container')||{}).className };
        const sel = si.querySelector('select'); if (sel) return { type: 'select', value: sel.value, options: Array.from(sel.options).map(o => ({ value: o.value, label: o.textContent })) };
        const inp = si.querySelector('input:not([type=checkbox])'); if (inp) return { type: 'input', inputType: inp.type, value: inp.value };
        const ta = si.querySelector('textarea'); if (ta) return { type: 'textarea', value: ta.value }; return { type: 'none', html: si.outerHTML.slice(0, 300) }; })(),
    })),
    textareas: q('textarea').map(t => ({ value: t.value, rows: t.rows, readOnly: t.readOnly, cls: t.className })),
    buttons: q('button').map(b => ({ text: b.textContent.trim(), cls: b.className })),
    contentText: (m.querySelector('.modal-content') || m).innerText.slice(0, 1500),
    html: m.outerHTML.slice(0, 2500),
  };`;
await step('search', async () => { await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(700); await c.typeText('needle'); await sleep(1500);
  return { rows: await c.evaluate(`return Array.from(document.querySelectorAll('.search-result-file-title .tree-item-inner')).map(e => e.textContent.trim());`) }; });
await step('open-modal-by-real-click', async () => {
  await c.clickElement('.search-results-result-count'); await sleep(400);
  const items = await c.openMenuItems();
  const rect = await c.evaluate(`const it = Array.from(document.querySelectorAll('.menu .menu-item')).find(e => /copy search results/i.test(e.textContent)); if (!it) return null; const r = it.getBoundingClientRect(); return { cx: r.x + r.width/2, cy: r.y + r.height/2 };`);
  await c.clickAt(rect.cx, rect.cy); await sleep(700);
  return { items, modal: await c.evaluate(MODAL) };
});
// Toggle every toggle in turn and read the textarea after each, to get every format variant.
await step('variants', async () => {
  const base = await c.evaluate(MODAL); if (!base) return { noModal: true };
  const idxOf = (name) => base.settings.findIndex(x => x.name === name);
  const iPath = idxOf('Show path'), iLink = idxOf('Link style'), iPrefix = idxOf('List prefix');
  const setToggle = async (i, want) => { const now = await c.evaluate(MODAL); if (now.settings[i].control.checked !== want) { await c.clickElement('.modal-container .modal .setting-item .checkbox-container', { index: 0 }); await sleep(250); } };
  const setSelect = async (i, value) => c.evaluate(`const sel = document.querySelectorAll('.modal-container .modal .setting-item')[${i}].querySelector('select'); sel.value = ${JSON.stringify(value)}; sel.dispatchEvent(new Event('change', { bubbles: true })); return sel.value;`);
  const variants = [];
  for (const showPath of [false, true]) for (const link of ['none', 'wikilink', 'markdown']) for (const prefix of ['none', '-', '*', 'num']) {
    await setToggle(iPath, showPath); const l = await setSelect(iLink, link); const p = await setSelect(iPrefix, prefix); await sleep(250);
    const now = await c.evaluate(MODAL);
    variants.push({ showPath: now.settings[iPath].control.checked, link: l, prefix: p, text: now.textareas[0] ? now.textareas[0].value : null });
  }
  return { indexes: { iPath, iLink, iPrefix }, variants };
});
await step('press-copy-button', async () => {
  await c.writeClipboard('SENTINEL-M');
  const btns = await c.evaluate(`return Array.from(document.querySelectorAll('.modal-container .modal button')).map(b => b.textContent.trim());`);
  const idx = btns.findIndex(t => /copy/i.test(t));
  if (idx < 0) return { btns, found: false };
  await c.clickElement('.modal-container .modal button', { index: idx }); 
  const reads = []; for (const w of [200, 800, 2000]) { await sleep(w); reads.push({ afterMs: w, electron: await c.readClipboard() }); }
  return { btns, pressed: btns[idx], reads, modalStillOpen: !!(await c.evaluate(`return document.querySelector('.modal-container .modal');`)), notices: await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(n => n.textContent.trim());`) };
});
await step('cleanup', async () => { await c.pressKey('Escape'); await sleep(200); return { modal: await c.evaluate(`return !!document.querySelector('.modal-container .modal');`) }; });
await c.close();
