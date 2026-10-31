// Blocks 8 and 10 of [[Interaction probes]]: raw HTML in the reading view
// under a real anchor click, and the remote-image layout. One script because
// both need an HTTP server, which this process runs itself and whose request
// log it therefore owns — Obsidian's CSP blocks the renderer's own fetch to
// localhost, so the log has to be read from the host.
//
// The anchor clicks are guarded: a dispatched click on a raw `<a>` navigates
// the Electron frame and kills the app mid-run, so a bubble-phase listener
// records `defaultPrevented` as Obsidian left it and then cancels the default
// itself.
// Phases: fixtures | run
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';
import { connect, sleep } from './cdp.mjs';

const require = createRequire(import.meta.url);

const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS;
const PORT = Number(process.env.HTTP_PORT || 8731);
const BASE = `http://127.0.0.1:${PORT}`;
const vp = (rel) => path.join(VAULT, rel);
const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.rawHtmlRemote = all.rawHtmlRemote || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [], httpLog: [] };
all.rawHtmlRemote[phase] = out;
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

const PNG = (() => {
  // A 120x60 solid PNG, built here so the fixture needs no tool.
  const zlib = require('node:zlib');
  const w = 120, h = 60;
  let raw = Buffer.alloc(0);
  for (let y = 0; y < h; y++) raw = Buffer.concat([raw, Buffer.from([0]), Buffer.alloc(w * 3, 0x60)]);
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type), data]);
    const crcTable = [];
    for (let n = 0; n < 256; n++) { let cc = n; for (let k = 0; k < 8; k++) cc = cc & 1 ? 0xedb88320 ^ (cc >>> 1) : cc >>> 1; crcTable[n] = cc >>> 0; }
    let crc = 0xffffffff;
    for (const b of td) crc = crcTable[(crc ^ b) & 0xff] ^ (crc >>> 8);
    const c = Buffer.alloc(4); c.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
    return Buffer.concat([len, td, c]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
})();

const RAW = [
  '# Raw HTML probe',
  '',
  '## Images block',
  '',
  '<img src="pic.png" alt="vault relative">',
  `<img src="${BASE}/remote.png" alt="remote">`,
  `<img src="${BASE}/slow.png" alt="slow">`,
  `<img src="${BASE}/missing.png" alt="404">`,
  '<img src="pic.png" width="40" height="20" alt="sized">',
  '',
  '## Anchors',
  '',
  '<a id="anchor-noext" href="Other">bare basename</a>',
  '<a id="anchor-ext" href="Other.md">with extension</a>',
  '<a id="anchor-attachment" href="pic.png">an attachment</a>',
  '<a id="anchor-missing" href="No Such Note">missing note</a>',
  '<a id="anchor-hash" href="#Images block">in-note hash</a>',
  `<a id="anchor-external" href="${BASE}/page.html">an external page</a>`,
  '',
  '## Details',
  '',
  '<details><summary>a summary</summary>the details body</details>',
  '',
  '## Sup and sub',
  '',
  'Water is H<sub>2</sub>O and E = mc<sup>2</sup>.',
  '',
  '## Unwrap cases',
  '',
  '<object data="pic.png">object body</object>',
  '<embed src="pic.png">',
  '<form action="/x"><input name="q"></form>',
  '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>',
  '<style>.silica-probe { color: red }</style>',
  '<link rel="stylesheet" href="x.css">',
  '<meta name="silica" content="probe">',
  '<iframe src="' + BASE + '/frame.html" width="100" height="40"></iframe>',
  '<div style="background-image: url(' + BASE + '/bg.png)">background image</div>',
  '<script>window.__silicaRawScriptRan = true;</script>',
  '',
  '## In containers',
  '',
  '> [!note] A callout',
  '> <b>bold in a callout</b> and <img src="pic.png" alt="in callout">',
  '',
  '> <b>bold in a blockquote</b>',
  '',
  '| head | head |',
  '| --- | --- |',
  '| <b>bold in a cell</b> | <img src="pic.png" alt="in cell"> |',
  '',
  '- <b>bold in a list item</b>',
  '- plain item',
  '',
  'A paragraph that is <b>interrupted</b> by inline HTML.',
  '',
  'A paragraph then',
  '<div>a raw block interrupting it</div>',
  'and more text.',
  '',
].join('\n');

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('Raw HTML.md'), RAW);
  fs.writeFileSync(vp('Other.md'), '# Other\n\nThe anchor target.\n');
  fs.writeFileSync(vp('pic.png'), PNG);
  fs.writeFileSync(vp('Remote.md'), [
    '# Remote images',
    '',
    'Before the image.',
    '',
    `![remote](${BASE}/remote.png)`,
    '',
    'Between the images.',
    '',
    `![slow](${BASE}/slow.png)`,
    '',
    `![missing](${BASE}/missing.png)`,
    '',
    `![sized|60x30](${BASE}/remote.png)`,
    '',
    'After the images.',
    '',
  ].join('\n'));
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'command-palette': true }, null, 2));
  save(); process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

// The probe's own HTTP server, so its request log is readable from the host.
const log = [];
const server = http.createServer((req, res) => {
  log.push({ at: Date.now() - T0, method: req.method, url: req.url, headers: { referer: req.headers.referer || null, accept: (req.headers.accept || '').slice(0, 60), 'user-agent': (req.headers['user-agent'] || '').slice(0, 40) } });
  const u = req.url.split('?')[0];
  if (u === '/remote.png' || u === '/bg.png') { res.writeHead(200, { 'Content-Type': 'image/png', 'Cache-Control': 'no-store' }); res.end(PNG); return; }
  if (u === '/slow.png') { setTimeout(() => { res.writeHead(200, { 'Content-Type': 'image/png' }); res.end(PNG); }, 2500); return; }
  if (u === '/missing.png') { res.writeHead(404, { 'Content-Type': 'text/plain' }); res.end('no'); return; }
  if (u === '/page.html' || u === '/frame.html') { res.writeHead(200, { 'Content-Type': 'text/html' }); res.end('<html><body>probe page</body></html>'); return; }
  res.writeHead(404); res.end('no');
});
await new Promise((r) => server.listen(PORT, '127.0.0.1', r));
out.httpBase = BASE;
save();

const c = await connect();
out.vaultPath = await c.vaultPath();
out.title = await c.evaluate(`return document.title;`);
const bytesBefore = { raw: fs.readFileSync(vp('Raw HTML.md'), 'utf8'), remote: fs.readFileSync(vp('Remote.md'), 'utf8') };
save();

await c.evaluate(`
window.__rh = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  leaf() {
    const a = app.workspace.activeLeaf;
    if (a && a.view && a.view.getViewType() === 'markdown') return a;
    const ls = app.workspace.getLeavesOfType('markdown');
    return ls.length ? ls[0] : app.workspace.getLeaf(false);
  },
  view() { return this.leaf().view; },
  async open(file, mode) {
    const ls = app.workspace.getLeavesOfType('markdown');
    const leaf = ls.length ? ls[0] : app.workspace.getLeaf(false);
    await leaf.openFile(app.vault.getAbstractFileByPath(file));
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await this.sleep(500);
    const st = leaf.getViewState();
    st.state = Object.assign({}, st.state, { mode: mode === 'reading' ? 'preview' : 'source', source: mode === 'source' });
    await leaf.setViewState(st);
    await this.sleep(700);
    return { file: leaf.view.file.path, mode: st.state.mode };
  },
  pv() { return this.view().containerEl.querySelector('.markdown-preview-view'); },
  async scrollAll() {
    const s = this.pv();
    for (let y = 0; y < s.scrollHeight + 600; y += 300) { s.scrollTop = y; await this.sleep(150); }
    s.scrollTop = 0;
    await this.sleep(300);
    return true;
  },
  rect(el) { if (!el) return null; const b = el.getBoundingClientRect(); return { x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.width * 100) / 100, h: Math.round(b.height * 100) / 100 }; },
  // Record what Obsidian left on a click without letting the frame navigate.
  armAnchorGuard() {
    window.__rhClicks = [];
    if (window.__rhGuard) return 'already armed';
    window.__rhGuard = (ev) => {
      const a = ev.target.closest ? ev.target.closest('a') : null;
      window.__rhClicks.push({
        id: a ? a.id : null, href: a ? a.getAttribute('href') : null,
        cls: a ? a.className : null, defaultPreventedByObsidian: ev.defaultPrevented,
        targetTag: ev.target.tagName,
      });
      ev.preventDefault();
      ev.stopPropagation();
    };
    document.addEventListener('click', window.__rhGuard, false);
    return 'armed';
  },
  clicks() { return window.__rhClicks || []; },
};
return true;`);

const ev = (b) => c.evaluate(b);

await step('s01-raw-html-dom', async (r) => {
  r.opened = await ev(`return await window.__rh.open('Raw HTML.md', 'reading');`);
  await ev(`return await window.__rh.scrollAll();`);
  await sleep(3500);
  r.sections = await ev(`
    const u = window.__rh;
    const pv = u.pv();
    return Array.from(pv.querySelectorAll('.markdown-preview-sizer > *')).map((d) => ({
      cls: d.className, tag: d.tagName, text: (d.textContent || '').slice(0, 60),
      rect: u.rect(d), innerTags: Array.from(d.querySelectorAll('*')).slice(0, 12).map((x) => x.tagName + (x.className ? '.' + String(x.className).split(' ')[0] : '')),
    }));`);
  r.unwrapCases = await ev(`
    const u = window.__rh;
    const pv = u.pv();
    const tags = ['OBJECT', 'EMBED', 'FORM', 'INPUT', 'SVG', 'STYLE', 'LINK', 'META', 'IFRAME', 'SCRIPT', 'DETAILS', 'SUMMARY', 'SUP', 'SUB', 'B'];
    const o = {};
    for (const t of tags) {
      const els = Array.from(pv.querySelectorAll(t.toLowerCase()));
      o[t] = { count: els.length, first: els[0] ? { outer: els[0].outerHTML.slice(0, 160), rect: u.rect(els[0]) } : null };
    }
    o.scriptRan = !!window.__silicaRawScriptRan;
    o.probeStyleApplied = (() => { const d = document.createElement('div'); d.className = 'silica-probe'; pv.appendChild(d); const col = getComputedStyle(d).color; d.remove(); return col; })();
    o.rawTextLeftOver = Array.from(pv.querySelectorAll('p, div')).map((x) => x.textContent).filter((t) => t.indexOf('<') >= 0).slice(0, 8);
    return o;`);
  r.containers = await ev(`
    const u = window.__rh;
    const pv = u.pv();
    const grab = (sel) => Array.from(pv.querySelectorAll(sel)).map((x) => ({ html: x.outerHTML.slice(0, 260), rect: u.rect(x) }));
    return {
      calloutHtml: grab('.callout'),
      blockquoteHtml: grab('blockquote:not(.callout)'),
      tableCellHtml: grab('td'),
      listItemHtml: grab('li'),
      interruptedParagraph: Array.from(pv.querySelectorAll('p')).filter((p) => p.textContent.includes('interrupted')).map((p) => p.outerHTML.slice(0, 200)),
      rawBlockInParagraph: Array.from(pv.querySelectorAll('div, p')).filter((p) => p.textContent.includes('a raw block interrupting it')).map((p) => ({ tag: p.tagName, cls: p.className, outer: p.outerHTML.slice(0, 240) })),
    };`);
  r.bytesUnchanged = fs.readFileSync(vp('Raw HTML.md'), 'utf8') === bytesBefore.raw;
  return r;
});

await step('s02-anchor-clicks', async (r) => {
  r.armed = await ev(`return window.__rh.armAnchorGuard();`);
  r.anchors = await ev(`
    const u = window.__rh;
    const pv = u.pv();
    return Array.from(pv.querySelectorAll('a')).map((a) => ({
      id: a.id, href: a.getAttribute('href'), cls: a.className,
      dataHref: a.getAttribute('data-href'), target: a.getAttribute('target'),
      text: a.textContent, rect: u.rect(a),
    }));`);
  r.clicked = [];
  for (const a of r.anchors) {
    if (!a.rect || a.rect.w === 0) { r.clicked.push({ id: a.id, skipped: 'zero rect' }); continue; }
    const scrollBefore = await ev(`return window.__rh.pv().scrollTop;`);
    const fileBefore = await ev(`return window.__rh.view().file.path;`);
    await ev(`
      const u = window.__rh;
      const pv = u.pv();
      const a = Array.from(pv.querySelectorAll('a')).find((x) => x.id === ${JSON.stringify(a.id)});
      if (a) a.scrollIntoView({ block: 'center' });
      await u.sleep(250);
      return true;`);
    const rect = await ev(`
      const u = window.__rh;
      const a = Array.from(u.pv().querySelectorAll('a')).find((x) => x.id === ${JSON.stringify(a.id)});
      if (!a) return null;
      const b = a.getBoundingClientRect();
      return { cx: b.x + b.width / 2, cy: b.y + b.height / 2 };`);
    if (!rect) { r.clicked.push({ id: a.id, skipped: 'anchor gone' }); continue; }
    await c.clickAt(rect.cx, rect.cy);
    await sleep(900);
    r.clicked.push({
      id: a.id, href: a.href,
      events: await ev(`const e = window.__rh.clicks(); window.__rhClicks = []; return e;`),
      fileAfter: await ev(`return window.__rh.view().file ? window.__rh.view().file.path : null;`),
      fileBefore,
      scrollBefore, scrollAfter: await ev(`return window.__rh.pv().scrollTop;`),
      leaves: await ev(`return { markdown: app.workspace.getLeavesOfType('markdown').length, types: app.workspace.getLeavesOfType('markdown').map((l) => l.view.file && l.view.file.path) };`),
      windows: await ev(`return document.querySelectorAll('.mod-root .workspace-leaf').length;`),
    });
    if ((await ev(`return window.__rh.view().file.path;`)) !== 'Raw HTML.md') {
      await ev(`return await window.__rh.open('Raw HTML.md', 'reading');`);
      await ev(`return window.__rh.armAnchorGuard();`);
    }
  }
  return r;
});

await step('s03-remote-image-layout', async (r) => {
  const logBefore = log.length;
  r.opened = await ev(`return await window.__rh.open('Remote.md', 'reading');`);
  // Measure immediately, then after the slow image can have arrived.
  r.atOnce = await ev(`
    const u = window.__rh;
    const pv = u.pv();
    return Array.from(pv.querySelectorAll('img')).map((i) => ({
      alt: i.alt, src: i.getAttribute('src'), complete: i.complete,
      naturalWidth: i.naturalWidth, naturalHeight: i.naturalHeight,
      rect: u.rect(i), parentCls: i.parentElement.className, parentRect: u.rect(i.parentElement),
      loading: i.getAttribute('loading'), referrerPolicy: i.referrerPolicy,
      style: (() => { const s = getComputedStyle(i); return { width: s.width, height: s.height, display: s.display, objectFit: s.objectFit }; })(),
    }));`);
  await sleep(4000);
  r.afterLoad = await ev(`
    const u = window.__rh;
    const pv = u.pv();
    return {
      imgs: Array.from(pv.querySelectorAll('img')).map((i) => ({
        alt: i.alt, src: i.getAttribute('src'), complete: i.complete,
        naturalWidth: i.naturalWidth, naturalHeight: i.naturalHeight, rect: u.rect(i),
        parentCls: i.parentElement.className, parentRect: u.rect(i.parentElement),
        resizeCorner: !!i.parentElement.querySelector('.image-resize-corner'),
      })),
      sizerChildren: Array.from(pv.querySelectorAll('.markdown-preview-sizer > *')).map((d) => ({ cls: d.className, tag: d.tagName, text: (d.textContent || '').slice(0, 40), rect: u.rect(d) })),
    };`);
  r.httpRequests = log.slice(logBefore);
  r.bytesUnchanged = fs.readFileSync(vp('Remote.md'), 'utf8') === bytesBefore.remote;
  return r;
});

await step('s04-broken-image-and-query', async (r) => {
  r.broken = await ev(`
    const u = window.__rh;
    const pv = u.pv();
    const i = Array.from(pv.querySelectorAll('img')).find((x) => (x.getAttribute('src') || '').indexOf('missing') >= 0);
    if (!i) return null;
    return {
      src: i.getAttribute('src'), complete: i.complete, naturalWidth: i.naturalWidth,
      rect: u.rect(i), altText: i.alt,
      style: (() => { const s = getComputedStyle(i); return { width: s.width, height: s.height, minHeight: s.minHeight, display: s.display, visibility: s.visibility }; })(),
      parentOuter: i.parentElement.outerHTML.slice(0, 260),
    };`);
  // The ?<digits> query on a vault-relative image: is it the file mtime?
  r.localImage = await ev(`
    const u = window.__rh;
    await u.open('Raw HTML.md', 'reading');
    await u.sleep(900);
    const pv = u.pv();
    return Array.from(pv.querySelectorAll('img')).map((i) => ({ alt: i.alt, src: i.getAttribute('src'), currentSrc: i.currentSrc })).slice(0, 6);`);
  r.picStat = (() => { const s = fs.statSync(vp('pic.png')); return { mtimeMs: s.mtimeMs, mtime: Math.floor(s.mtimeMs), ctimeMs: s.ctimeMs, size: s.size }; })();
  return r;
});

out.httpLog = log;
out.finishedAt = new Date().toISOString();
save();
c.close();
server.close();
process.exit(0);
