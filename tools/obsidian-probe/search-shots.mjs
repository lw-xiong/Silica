// Screenshots and computed styles of Obsidian's Search pane, for the parity pass.
import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';
import { connect, sleep } from './cdp.mjs';
const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS, SHOTS = process.env.SHOTS;
const FIX = {
  'apple.md': 'apple needle one\n',
  'notes/banana.md': 'banana has a needle in a folder\n',
  'mango.md': 'line before first\nmango needle first\nline after first\n\nmiddle paragraph with no hit at all\n\nmango needle second\nmango needle third on this line, and this line is long enough that the search result row has to cut it somewhere before the end of it\nline after third\nlast line\n',
  'zebra.md': 'zebra needle alpha\nfiller line\nzebra needle beta\n',
  'kiwi.md': 'kiwi has Needle with a capital N only\n',
  'Apple Pie.md': 'apple pie needle\n',
  '_needle.md': 'underscore file needle\n',
  'notes/apple.md': 'notes apple needle\n',
};
if (phase === 'fixtures') {
  for (const [rel, body] of Object.entries(FIX)) { const p = path.join(VAULT, rel); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); }
  process.exit(0);
}
if (phase !== 'run') process.exit(0);
const out = {};
const c = await connect();
let n = 0;
async function shot(name) {
  n++;
  const file = path.join(SHOTS, `${String(n).padStart(2, '0')}-${name}.png`);
  if (typeof c.send === 'function') {
    const r = await c.send('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(file, Buffer.from(r.data, 'base64'));
  } else {
    execSync(`spectacle -b -n -a -o ${file}`);
  }
  return file;
}
const PROPS = ['font-size','font-weight','font-family','line-height','color','background-color','padding','margin','border-radius','border','height','width','gap','opacity','display','align-items','justify-content','flex','min-height','max-height','text-overflow','white-space','overflow','box-shadow','cursor','letter-spacing'];
const styles = (sels) => c.evaluate(`
  const props = ${JSON.stringify(PROPS)};
  const res = {};
  for (const sel of ${JSON.stringify(sels)}) {
    const els = Array.from(document.querySelectorAll(sel)).slice(0, 3);
    res[sel] = els.map(el => { const cs = getComputedStyle(el); const r = el.getBoundingClientRect(); const o = { rect: [r.x, r.y, r.width, r.height].map(v => Math.round(v * 100) / 100), class: el.className && el.className.baseVal === undefined ? el.className : '', text: (el.textContent || '').trim().slice(0, 80), tag: el.tagName }; for (const p of props) o[p] = cs.getPropertyValue(p); return o; });
  }
  return res;`);
const SELS = ['.workspace-leaf-content[data-type="search"]', '.workspace-leaf-content[data-type="search"] > .view-content', '.search-row', '.search-input-container', '.search-input-container input', '.search-input-clear-button', '.input-right-decorator', '.search-row .clickable-icon', '.search-row .clickable-icon svg', '.search-results-result-count', '.search-results-result-count span', '.search-results-info', '.search-results-info .dropdown', '.search-info-container', '.search-params', '.search-params .setting-item', '.search-params .setting-item-name', '.search-params .checkbox-container', '.search-params .checkbox-container input', '.search-result-container', '.search-result', '.search-result-file-title', '.search-result-file-title .tree-item-inner', '.search-result-file-title .tree-item-flair', '.search-result-file-title .collapse-icon', '.search-result-file-title .collapse-icon svg', '.search-result-file-matches', '.search-result-file-match', '.search-result-file-matched-text', '.search-result-hover-button', '.search-empty-state', '.tree-item-self', '.nav-header', '.nav-buttons-container', '.workspace-leaf-content[data-type="search"] .view-header'];
await c.evaluate(`return app.commands.executeCommandById('global-search:open');`); await sleep(600);
out.theme = await c.evaluate(`return { theme: app.vault.getConfig('theme'), body: document.body.className };`);
await c.clickElement('.search-input-container input'); await sleep(150);
await c.typeText('needle'); await sleep(1200);
out.shot_results = await shot('results');
out.styles_results = await styles(SELS);
out.outerHTML = await c.evaluate(`const el = document.querySelector('.workspace-leaf-content[data-type="search"]'); return el ? el.outerHTML.slice(0, 20000) : null;`);
// hover a match row
await c.hover('.search-result-file-match', 2); await sleep(300);
out.shot_hover = await shot('hover-match');
out.styles_hover = await styles(['.search-result-file-match:hover', '.search-result-hover-button', '.search-result-hover-button svg']);
// focus a row from the keyboard
await c.clickElement('.search-input-container input'); await sleep(100);
await c.pressKey('ArrowDown'); await sleep(300);
out.shot_focus = await shot('row-focused');
out.styles_focus = await styles(['.search-result-file-title.has-focus', '.has-focus', '.search-result.is-collapsed']);
await c.pressKey('ArrowLeft'); await sleep(300);
out.shot_collapsed = await shot('row-collapsed');
await c.pressKey('ArrowRight'); await sleep(200);
// settings section
await c.clickElement('.search-row .clickable-icon[aria-label="Search settings"]'); await sleep(300);
out.shot_settings = await shot('settings-open');
out.styles_settings = await styles(['.search-params', '.search-params .setting-item', '.search-params .setting-item-name', '.search-params .setting-item-control', '.search-params .checkbox-container', '.search-params .checkbox-container input', '.search-row .clickable-icon.is-active']);
// explain
await c.clickElement('.search-params .checkbox-container', { index: 2 }); await sleep(400);
out.shot_explain = await shot('explain-on');
out.explain_html = await c.evaluate(`const el = document.querySelector('.search-info-container'); return el ? el.outerHTML : null;`);
out.styles_explain = await styles(['.search-info-container', '.search-info-container > *', '.search-info-container > * > *', '.search-info-more-matches']);
await c.clickElement('.search-params .checkbox-container', { index: 2 }); await sleep(200);
// more context
await c.clickElement('.search-params .checkbox-container', { index: 1 }); await sleep(600);
out.shot_context = await shot('more-context');
out.styles_context = await styles(['.search-result-file-match', '.search-result-file-matched-text']);
out.context_html = await c.evaluate(`return Array.from(document.querySelectorAll('.search-result-file-match')).slice(0,6).map(e => e.outerHTML);`);
await c.clickElement('.search-params .checkbox-container', { index: 1 }); await sleep(200);
// collapse results
await c.clickElement('.search-params .checkbox-container', { index: 0 }); await sleep(400);
out.shot_collapse_all = await shot('collapse-all');
await c.clickElement('.search-params .checkbox-container', { index: 0 }); await sleep(200);
// match case
await c.clickElement('.input-right-decorator'); await sleep(600);
out.shot_match_case = await shot('match-case');
out.styles_match_case = await styles(['.input-right-decorator.is-active', '.input-right-decorator.is-active svg']);
await c.clickElement('.input-right-decorator'); await sleep(300);
// close settings; count menu
await c.clickElement('.search-row .clickable-icon[aria-label="Search settings"]'); await sleep(200);
await c.clickElement('.search-results-result-count'); await sleep(400);
out.shot_count_menu = await shot('count-menu');
out.menu = await c.openMenuItems();
out.styles_menu = await styles(['.menu', '.menu-item', '.menu-item-icon', '.menu-item-title']);
await c.pressKey('Escape'); await sleep(200);
// sort dropdown focused (native popup is not capturable; the select itself is)
out.sort = await c.selectInfo('.search-results-info select');
// empty query
await c.clickElement('.search-input-container input'); await sleep(100);
await c.pressKey('a', { ctrl: true }); await c.pressKey('Backspace'); await sleep(500);
out.shot_empty = await shot('empty-after-search');
out.styles_empty = await styles(['.search-results-result-count', '.search-empty-state', '.search-result-container']);
// invalid regex
await c.typeText('/(/'); await sleep(500);
out.shot_bad_regex = await shot('bad-regex');
out.bad_regex_html = await c.evaluate(`const el = document.querySelector('.search-info-container'); return { info: el ? el.outerHTML : null, count: document.querySelector('.search-results-result-count')?.textContent, container: document.querySelector('.search-result-container')?.innerHTML.slice(0, 2000) };`);
// light theme
await c.evaluate(`app.changeTheme('moonstone'); return true;`); await sleep(500);
await c.pressKey('a', { ctrl: true }); await c.pressKey('Backspace'); await c.typeText('needle'); await sleep(1000);
out.shot_light = await shot('light-results');
out.styles_light = await styles(['.search-row', '.search-input-container input', '.search-results-info', '.search-results-result-count span', '.search-result-file-title', '.search-result-file-title .tree-item-inner', '.search-result-file-title .tree-item-flair', '.search-result-file-matches', '.search-result-file-match', '.search-result-file-matched-text', '.search-result-container']);
await c.evaluate(`app.changeTheme('obsidian'); return true;`);
fs.writeFileSync(RESULTS, JSON.stringify(out, null, 1));
await c.quit();
