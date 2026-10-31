// Block 6 of [[Interaction probes]]: the code-block copy button, driven with
// a real mouse so the hover → reveal → click path a person takes is actually
// exercised, and the clipboard read back through Electron. The 2026-10-18
// pass clicked buttons whose computed `display` was `none` and tapped
// `navigator.clipboard.writeText` before every click, so it could not
// separate Obsidian's write from its own forwarding; nothing here replaces
// any clipboard API.
//
// Phases: fixtures | run   (node code-copy.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
const vp = (rel) => path.join(VAULT, rel);

const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.codeCopy = all.codeCopy || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.codeCopy[phase] = out;
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

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('Code.md'),
    '# Code\n\n' +
    'Para before.\n\n' +
    '```js\nconst CASE_LANG = 1;\n```\n\n' +
    '```\nno info string\n```\n\n' +
    '~~~python\nprint("tilde fence")\n~~~\n\n' +
    '```silica-made-up\nunregistered info\n```\n\n' +
    '```\n```\n\n' +
    '    four space indented\n    second line\n\n' +
    '> [!note] In a callout\n> ```js\n> const IN_CALLOUT = 2;\n> ```\n\n' +
    '> ```\n> in a blockquote\n> ```\n\n' +
    'Inline `code span` here.\n');
  fs.writeFileSync(vp('Flair.md'),
    '```js\njs fence\n```\n\n```python\npy fence\n```\n\n```rust\nrs fence\n```\n\n' +
    '```ts\nts fence\n```\n\n```c\nc fence\n```\n\n```yaml\nyaml: fence\n```\n\n' +
    '```silica-made-up\nunregistered\n```\n\n```\nnone\n```\n');
  fs.writeFileSync(vp('Trailing.md'), '```\ntab\tindented  \n\n\n```\n');
  fs.writeFileSync(vp('Unicode.md'), '```txt\ncafé — naïve ​ zero width ❤️\n&amp; &lt;b&gt; entity text\n```\n');
  fs.writeFileSync(vp('CRLF Code.md'), '```txt\r\nCRLF line one\r\nCRLF line two\r\n```\r\n');
  fs.writeFileSync(vp('Embedded.md'), '```js\nconst EMBEDDED = 3;\n```\n');
  fs.writeFileSync(vp('Embed Host.md'), 'Before the embed.\n\n![[Embedded]]\n\nAfter the embed.\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'command-palette': true }, null, 2));
  save(); process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
out.title = await c.evaluate(`return document.title;`);
save();

await c.evaluate(`
window.__cc = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  leaf() {
    const a = app.workspace.activeLeaf;
    if (a && a.view && a.view.getViewType() === 'markdown') return a;
    const ls = app.workspace.getLeavesOfType('markdown');
    return ls.length ? ls[0] : app.workspace.getLeaf(false);
  },
  view() { return this.leaf().view; },
  async open(file, mode) {
    const leaf = this.leaf();
    await leaf.openFile(app.vault.getAbstractFileByPath(file));
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await this.sleep(500);
    if (mode) await this.setMode(mode);
    await this.sleep(400);
    return { file: this.view().file.path, mode: this.mode() };
  },
  async setMode(mode) {
    const leaf = this.leaf(); const st = leaf.getViewState();
    st.state = Object.assign({}, st.state, { mode: mode === 'reading' ? 'preview' : 'source', source: mode === 'source' });
    await leaf.setViewState(st);
    await this.sleep(450);
    return this.mode();
  },
  mode() { const st = this.leaf().getViewState().state; return { mode: st.mode, source: st.source }; },
  rect(el) { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2 }; },
  // Only the reading-view container, so the editor's own duplicate pres do
  // not inflate the counts the way they did in the 2026-10-18 run.
  readingPres() {
    const el = this.view().containerEl.querySelector('.markdown-reading-view .markdown-preview-view, .markdown-preview-view');
    if (!el) return [];
    return Array.from(el.querySelectorAll('pre'));
  },
  preInfo(pre) {
    const code = pre.querySelector('code');
    const btn = pre.querySelector('button.copy-code-button');
    return {
      preClass: pre.className, codeClass: code ? code.className : null,
      domText: code ? code.textContent : pre.textContent,
      domTextLength: (code ? code.textContent : pre.textContent).length,
      hasButton: !!btn,
      buttonRect: this.rect(btn),
      buttonDisplay: btn ? getComputedStyle(btn).display : null,
      preDisplay: getComputedStyle(pre).display,
      preRect: this.rect(pre),
    };
  },
  describeReading() { return this.readingPres().map((p) => this.preInfo(p)); },
};
return true;`);

const ev = (b) => c.evaluate(b);
const open = (f, m) => ev(`return await window.__cc.open(${JSON.stringify(f)}${m ? ', ' + JSON.stringify(m) : ''});`);

// Hover the pre, then click the button that appears, then read the clipboard.
async function hoverAndCopy(index, sentinel) {
  await c.writeClipboard(sentinel);
  const before = await c.readClipboard();
  const preRect = await ev(`
    const u = window.__cc;
    const p = u.readingPres()[${index}];
    if (!p) return null;
    p.scrollIntoView({ block: 'center' });
    await u.sleep(300);
    return u.rect(p);`);
  if (!preRect) return { skipped: 'no such pre' };
  // Move the mouse onto the block so the hover rule reveals the button.
  await c.mouseMove(preRect.cx, preRect.cy);
  await sleep(400);
  const btn = await ev(`
    const u = window.__cc;
    const p = u.readingPres()[${index}];
    const b = p.querySelector('button.copy-code-button');
    return b ? { rect: u.rect(b), display: getComputedStyle(b).display, svg: b.querySelector('svg') ? b.querySelector('svg').getAttribute('class') : null, style: (() => { const s = getComputedStyle(b); return { padding: s.padding, margin: s.margin, borderRadius: s.borderRadius, color: s.color, backgroundColor: s.backgroundColor, fontSize: s.fontSize, position: s.position, top: s.top, insetInlineEnd: s.insetInlineEnd }; })() } : null;`);
  if (!btn || !btn.rect || btn.rect.w === 0) return { preRect, btn, skipped: 'button still not painted' };
  await c.clickAt(btn.rect.cx, btn.rect.cy);
  await sleep(350);
  const after = await c.readClipboard();
  const icon = await ev(`
    const u = window.__cc;
    const p = u.readingPres()[${index}];
    const b = p.querySelector('button.copy-code-button');
    return { svg: b.querySelector('svg') ? b.querySelector('svg').getAttribute('class') : null, cls: b.className, text: b.textContent, notice: !!document.querySelector('.notice') };`);
  return { preRect, btn, before, after, changed: after !== before, icon };
}

await step('s01-reading-view-every-shape', async (r) => {
  r.opened = await open('Code.md', 'reading');
  await sleep(800);
  r.pres = await ev(`return window.__cc.describeReading();`);
  r.copies = [];
  for (let i = 0; i < r.pres.length; i++) {
    const got = await hoverAndCopy(i, `SENTINEL-${i}`);
    r.copies.push({ index: i, preClass: r.pres[i].preClass, domText: r.pres[i].domText, ...got });
  }
  return r;
});

await step('s02-trailing-newlines-and-unicode', async (r) => {
  for (const [label, file] of [['trailing', 'Trailing.md'], ['unicode', 'Unicode.md'], ['crlf', 'CRLF Code.md']]) {
    await open(file, 'reading');
    await sleep(800);
    const pres = await ev(`return window.__cc.describeReading();`);
    const copies = [];
    for (let i = 0; i < pres.length; i++) copies.push({ index: i, ...(await hoverAndCopy(i, `S-${label}-${i}`)) });
    r[label] = {
      disk: fs.readFileSync(vp(file), 'utf8'),
      diskBytes: Array.from(fs.readFileSync(vp(file))).length,
      pres, copies,
      copiedCodes: copies.map((x) => (typeof x.after === 'string' ? Array.from(x.after).map((ch) => ch.codePointAt(0)) : null)),
    };
  }
  return r;
});

await step('s03-icon-revert-delay', async (r) => {
  await open('Code.md', 'reading');
  await sleep(800);
  const preRect = await ev(`
    const u = window.__cc;
    const p = u.readingPres()[0];
    p.scrollIntoView({ block: 'center' });
    await u.sleep(300);
    return u.rect(p);`);
  await c.mouseMove(preRect.cx, preRect.cy);
  await sleep(400);
  const btn = await ev(`
    const u = window.__cc;
    const b = u.readingPres()[0].querySelector('button.copy-code-button');
    return u.rect(b);`);
  if (!btn) return r;
  await c.clickAt(btn.cx, btn.cy);
  // Sample the icon until it reverts.
  r.samples = await ev(`
    const u = window.__cc;
    const b = u.readingPres()[0].querySelector('button.copy-code-button');
    const out = [];
    for (let t = 0; t <= 3000; t += 100) {
      out.push({ t, svg: b.querySelector('svg') ? b.querySelector('svg').getAttribute('class') : null });
      await u.sleep(100);
    }
    return out;`);
  r.revertedAt = (() => {
    const first = r.samples.findIndex((x) => x.svg && x.svg.includes('lucide-check'));
    const back = r.samples.findIndex((x, i) => i > first && x.svg && x.svg.includes('lucide-copy'));
    return { firstCheckAt: first >= 0 ? r.samples[first].t : null, backToCopyAt: back >= 0 ? r.samples[back].t : null };
  })();
  return r;
});

await step('s04-keyboard-reaches-the-button', async (r) => {
  await open('Code.md', 'reading');
  await sleep(800);
  await c.writeClipboard('KEYBOARD-SENTINEL');
  // Click the note body, then Tab repeatedly, reading what has focus.
  const body = await ev(`
    const u = window.__cc;
    const el = u.view().containerEl.querySelector('.markdown-preview-view');
    const p = Array.from(el.querySelectorAll('p')).find((x) => x.textContent.includes('Para before'));
    p.scrollIntoView({ block: 'center' });
    await u.sleep(300);
    return u.rect(p);`);
  await c.clickAt(body.cx, body.cy);
  await sleep(300);
  r.focusWalk = [];
  for (let i = 0; i < 8; i++) {
    await c.pressKey('Tab');
    await sleep(200);
    r.focusWalk.push(await ev(`
      const a = document.activeElement;
      return a ? { tag: a.tagName, cls: typeof a.className === 'string' ? a.className : '', display: getComputedStyle(a).display } : null;`));
  }
  const onButton = r.focusWalk.findIndex((x) => x && x.cls && x.cls.includes('copy-code-button'));
  r.reachedButtonAfterTabs = onButton >= 0 ? onButton + 1 : null;
  if (onButton >= 0) {
    await c.pressKey('Enter');
    await sleep(400);
    r.afterEnter = await c.readClipboard();
    r.enterCopied = r.afterEnter !== 'KEYBOARD-SENTINEL';
  }
  return r;
});

await step('s05-editor-flair', async (r) => {
  await open('Flair.md', 'live');
  await sleep(1000);
  r.flairs = await ev(`
    const u = window.__cc;
    const el = u.view().containerEl;
    const scroller = el.querySelector('.cm-scroller');
    const seen = new Map();
    const collect = () => {
      for (const f of el.querySelectorAll('.code-block-flair')) {
        const line = f.closest('.cm-line');
        const key = (line ? line.textContent : '') + '|' + f.textContent;
        if (!seen.has(key)) seen.set(key, {
          text: f.textContent, ariaLabel: f.getAttribute('aria-label'),
          contentEditable: f.getAttribute('contenteditable'),
          lineText: line ? line.textContent : null, lineClass: line ? line.className : null,
          rect: u.rect(f), tag: f.tagName, cls: f.className,
        });
      }
    };
    collect();
    for (let y = 0; y < scroller.scrollHeight + 400; y += 250) { scroller.scrollTop = y; await u.sleep(160); collect(); }
    scroller.scrollTop = 0;
    await u.sleep(200);
    return Array.from(seen.values());`);
  // Click one flair with a real mouse and see whether it copies.
  await c.writeClipboard('FLAIR-SENTINEL');
  r.clicked = await (async () => {
    const rect = await ev(`
      const u = window.__cc;
      const el = u.view().containerEl;
      el.querySelector('.cm-scroller').scrollTop = 0;
      await u.sleep(300);
      const f = el.querySelector('.code-block-flair');
      if (!f) return null;
      return { rect: u.rect(f), text: f.textContent };`);
    if (!rect || !rect.rect) return 'no flair';
    await c.mouseMove(rect.rect.cx, rect.rect.cy);
    await sleep(300);
    const hovered = await ev(`
      const f = window.__cc.view().containerEl.querySelector('.code-block-flair');
      const s = getComputedStyle(f);
      return { backgroundColor: s.backgroundColor, cursor: s.cursor, color: s.color, text: f.textContent };`);
    await c.clickAt(rect.rect.cx, rect.rect.cy);
    await sleep(500);
    const after = await c.readClipboard();
    return {
      flair: rect, hovered, clipboard: after, copied: after !== 'FLAIR-SENTINEL',
      editorValue: await ev(`return window.__cc.view().editor.getValue().slice(0, 80);`),
      notice: await ev(`const n = document.querySelector('.notice'); return n ? n.textContent : null;`),
      caret: await ev(`return window.__cc.view().editor.getCursor();`),
    };
  })();
  // Live Preview and source mode button counts, scoped to the editor only.
  r.counts = await ev(`
    const u = window.__cc;
    const scope = () => {
      const el = u.view().containerEl;
      return {
        mode: u.mode(),
        editorPres: el.querySelectorAll('.markdown-source-view pre').length,
        editorButtons: el.querySelectorAll('.markdown-source-view button.copy-code-button').length,
        flairs: el.querySelectorAll('.code-block-flair').length,
        readingPres: el.querySelectorAll('.markdown-reading-view pre').length,
      };
    };
    const live = scope();
    await u.setMode('source');
    await u.sleep(700);
    const source = scope();
    await u.setMode('live');
    await u.sleep(700);
    return { live, source };`);
  return r;
});

await step('s06-embedded-code-block', async (r) => {
  await open('Embed Host.md', 'reading');
  await sleep(1200);
  r.dom = await ev(`
    const u = window.__cc;
    const el = u.view().containerEl.querySelector('.markdown-preview-view');
    return {
      embeds: Array.from(el.querySelectorAll('.internal-embed')).map((e) => ({ cls: e.className, src: e.getAttribute('src'), html: e.innerHTML.slice(0, 300) })),
      pres: Array.from(el.querySelectorAll('pre')).map((p) => u.preInfo(p)),
    };`);
  r.copies = [];
  const n = r.dom.pres.length;
  for (let i = 0; i < n; i++) r.copies.push({ index: i, ...(await hoverAndCopy(i, `EMBED-${i}`)) });
  return r;
});

await step('s07-frontmatter-pre-reachable', async (r) => {
  await ev(`
    const u = window.__cc;
    await app.vault.modify(app.vault.getAbstractFileByPath('Code.md'), '---\\ntitle: Code Copy Probe\\ntags:\\n  - probe\\n---\\n\\n\`\`\`js\\nconst AFTER_FM = 1;\\n\`\`\`\\n');
    await u.sleep(900);
    await u.open('Code.md', 'reading');
    await u.sleep(900);
    return true;`);
  r.pres = await ev(`return window.__cc.describeReading();`);
  r.frontmatterPre = r.pres.find((p) => p.preClass && p.preClass.includes('frontmatter')) || null;
  r.copies = [];
  for (let i = 0; i < r.pres.length; i++) r.copies.push({ index: i, preClass: r.pres[i].preClass, ...(await hoverAndCopy(i, `FM-${i}`)) });
  return r;
});

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
