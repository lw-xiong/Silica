// Block 7 of [[Interaction probes]]: the heading-fold questions the
// 2026-10-18 plugin run could not reach — the cold restart, the localStorage
// key for a note in a subfolder, what `{0,0}` is, fold-more/fold-less,
// toggle-fold-properties, raw source mode, `foldIndent: false`, a click on
// heading text in reading view, a link into a folded section, and the edge
// headings (inside a callout or blockquote, setext, line 0, no trailing
// newline, an empty last section).
//
// The run phase ends by quitting Obsidian so `run-block.sh` relaunches it and
// the `after-restart` phase can read what survived.
// Phases: fixtures | run | after-restart
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS;
const vp = (rel) => path.join(VAULT, rel);
const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.headingFold = all.headingFold || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.headingFold[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
async function step(name, fn) {
  out.order.push(name);
  const t = Date.now(); const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save(); return out.steps[name];
}

const NOTE = [
  '---', 'tags:', '  - probe', '  - fold', '---', '', 'PREAMBLE paragraph before any heading.', '',
  '# H1 Top', '', 'Body of H1.', '', '## Section A', '', 'Body of A.', '', '### Sub A1', '',
  'Body of Sub A1.', '', '## Section B', '', '- list parent', '\t- child one', '\t- child two', '',
  '## Section C', '', 'Body of C.', '', '[^1]: a footnote definition', '',
].join('\n');

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.mkdirSync(vp('Sub Folder'), { recursive: true });
  fs.writeFileSync(vp('Fold Probe.md'), NOTE);
  fs.writeFileSync(vp('Sub Folder/Fold Probe.md'), NOTE);
  fs.writeFileSync(vp('Fold Edges.md'),
    '# Line zero heading\n\nbody\n\nSetext Heading\n==============\n\nsetext body\n\n' +
    '> [!note] Callout\n> ## Heading in a callout\n> callout body\n\n' +
    '> ## Heading in a blockquote\n> quote body\n\n' +
    '## Empty Last Section');
  fs.writeFileSync(vp('Link Source.md'), 'Jump to [[Fold Probe#Section B]] here.\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'command-palette': true, switcher: true }, null, 2));
  save(); process.exit(0);
}

const c = await connect();
out.vaultPath = await c.vaultPath();
out.title = await c.evaluate(`return document.title;`);
save();

await c.evaluate(`
window.__hf = {
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
    await this.sleep(500);
    return this.mode();
  },
  mode() { const st = this.leaf().getViewState().state; return { mode: st.mode, source: st.source }; },
  foldKeys() {
    return Object.keys(localStorage).filter((k) => k.indexOf('fold') >= 0).sort()
      .map((k) => ({ key: k, value: localStorage.getItem(k) }));
  },
  liveFolds() {
    const v = this.view();
    try { return v.currentMode && v.currentMode.getFoldInfo ? v.currentMode.getFoldInfo() : (v.editMode && v.editMode.getFoldInfo ? v.editMode.getFoldInfo() : null); }
    catch (e) { return 'error: ' + e.message; }
  },
  rendered() {
    const v = this.view();
    const el = v.containerEl;
    const cm = el.querySelector('.markdown-source-view .cm-content');
    if (cm) return Array.from(cm.querySelectorAll('.cm-line')).map((l) => l.textContent);
    const pv = el.querySelector('.markdown-preview-view');
    return pv ? Array.from(pv.querySelectorAll('.markdown-preview-sizer > div')).map((d) => d.className + '|' + d.textContent.slice(0, 30)) : null;
  },
  snap() { return { mode: this.mode(), live: this.liveFolds(), stored: this.foldKeys(), rendered: this.rendered() }; },
  caretTo(line, ch) { const ed = this.view().editor; ed.focus(); ed.setCursor({ line, ch: ch || 0 }); return ed.getCursor(); },
  run(id) { return app.commands.executeCommandById(id); },
};
return true;`);

const ev = (b) => c.evaluate(b);
const snap = () => ev(`return window.__hf.snap();`);

if (phase === 'run') {
  await step('s01-fold-in-editor-and-the-key', async (r) => {
    r.opened = await ev(`return await window.__hf.open('Fold Probe.md', 'live');`);
    r.before = await snap();
    r.foldSectionA = await ev(`
      const u = window.__hf;
      u.caretTo(12, 0);
      const ran = u.run('editor:toggle-fold');
      await u.sleep(600);
      return { ran, snap: u.snap() };`);
    r.noteBytesAfterFold = fs.readFileSync(vp('Fold Probe.md'), 'utf8') === NOTE ? 'byte-identical' : 'CHANGED';
    r.obsidianDirAfterFold = fs.readdirSync(vp('.obsidian')).sort();
    return r;
  });

  await step('s02-subfolder-key', async (r) => {
    r.opened = await ev(`return await window.__hf.open('Sub Folder/Fold Probe.md', 'live');`);
    r.folded = await ev(`
      const u = window.__hf;
      u.caretTo(12, 0);
      const ran = u.run('editor:toggle-fold');
      await u.sleep(700);
      return { ran, snap: u.snap() };`);
    r.keys = await ev(`return window.__hf.foldKeys();`);
    return r;
  });

  await step('s03-fold-more-less-and-zero-zero', async (r) => {
    await ev(`return await window.__hf.open('Fold Probe.md', 'live');`);
    r.unfoldAll = await ev(`
      const u = window.__hf;
      u.run('editor:unfold-all');
      await u.sleep(600);
      return u.snap();`);
    r.foldAll = await ev(`
      const u = window.__hf;
      u.run('editor:fold-all');
      await u.sleep(700);
      return u.snap();`);
    r.unfoldAllAgain = await ev(`
      const u = window.__hf;
      u.run('editor:unfold-all');
      await u.sleep(600);
      return u.snap();`);
    r.foldMore = [];
    for (let i = 0; i < 4; i++) {
      r.foldMore.push(await ev(`
        const u = window.__hf;
        u.caretTo(12, 0);
        const ran = u.run('editor:fold-more');
        await u.sleep(500);
        return { ran, live: u.liveFolds(), lines: u.rendered().length };`));
    }
    r.foldLess = [];
    for (let i = 0; i < 4; i++) {
      r.foldLess.push(await ev(`
        const u = window.__hf;
        const ran = u.run('editor:fold-less');
        await u.sleep(500);
        return { ran, live: u.liveFolds(), lines: u.rendered().length };`));
    }
    r.toggleFoldProperties = [];
    for (let i = 0; i < 3; i++) {
      r.toggleFoldProperties.push(await ev(`
        const u = window.__hf;
        u.caretTo(10, 0);
        const ran = u.run('editor:toggle-fold-properties');
        await u.sleep(600);
        return { ran, live: u.liveFolds(), stored: u.foldKeys(), firstLines: u.rendered().slice(0, 4) };`));
    }
    return r;
  });

  await step('s04-source-mode-and-foldIndent', async (r) => {
    r.sourceMode = await ev(`
      const u = window.__hf;
      await u.open('Fold Probe.md', 'source');
      u.run('editor:unfold-all');
      await u.sleep(500);
      u.caretTo(12, 0);
      const ran = u.run('editor:toggle-fold');
      await u.sleep(700);
      const el = u.view().containerEl;
      return {
        ran, snap: u.snap(),
        indicators: el.querySelectorAll('.cm-fold-indicator').length,
        collapseIcons: el.querySelectorAll('.collapse-indicator').length,
      };`);
    r.foldIndentFalse = await ev(`
      const u = window.__hf;
      await u.open('Fold Probe.md', 'live');
      u.run('editor:unfold-all');
      await u.sleep(400);
      app.vault.setConfig('foldIndent', false);
      await u.sleep(600);
      await u.open('Fold Probe.md', 'live');
      await u.sleep(600);
      u.caretTo(22, 0);
      const listRan = u.run('editor:toggle-fold');
      await u.sleep(600);
      const afterList = u.snap();
      u.caretTo(12, 0);
      const headRan = u.run('editor:toggle-fold');
      await u.sleep(600);
      const afterHead = u.snap();
      app.vault.setConfig('foldIndent', true);
      await u.sleep(400);
      return { listRan, afterList, headRan, afterHead, config: { foldIndent: app.vault.getConfig('foldIndent'), foldHeading: app.vault.getConfig('foldHeading') } };`);
    return r;
  });

  await step('s05-reading-view-clicks', async (r) => {
    await ev(`
      const u = window.__hf;
      await u.open('Fold Probe.md', 'live');
      u.run('editor:unfold-all');
      await u.sleep(500);
      await u.setMode('reading');
      await u.sleep(800);
      return true;`);
    r.before = await snap();
    // Click the heading TEXT with a real mouse.
    const textRect = await ev(`
      const u = window.__hf;
      const pv = u.view().containerEl.querySelector('.markdown-preview-view');
      const h = Array.from(pv.querySelectorAll('h2')).find((x) => x.textContent.includes('Section A'));
      if (!h) return null;
      h.scrollIntoView({ block: 'center' });
      await u.sleep(300);
      const b = h.getBoundingClientRect();
      return { x: b.x, y: b.y, w: b.width, h: b.height, textCx: b.x + 40, cy: b.y + b.height / 2, farRightCx: b.x + b.width - 20 };`);
    r.textRect = textRect;
    if (textRect) {
      await c.clickAt(textRect.textCx, textRect.cy);
      await sleep(700);
      r.afterTextClick = await snap();
      await c.clickAt(textRect.farRightCx, textRect.cy);
      await sleep(700);
      r.afterEmptySpaceClick = await snap();
      // And the indicator itself, hovered first.
      const ind = await ev(`
        const u = window.__hf;
        const pv = u.view().containerEl.querySelector('.markdown-preview-view');
        const h = Array.from(pv.querySelectorAll('h2')).find((x) => x.textContent.includes('Section A'));
        const s = h.querySelector('.heading-collapse-indicator');
        if (!s) return null;
        const b = s.getBoundingClientRect();
        return { x: b.x, y: b.y, w: b.width, h: b.height, cx: b.x + b.width / 2, cy: b.y + b.height / 2, style: (() => { const cs = getComputedStyle(s); return { opacity: cs.opacity, display: cs.display, left: cs.left, position: cs.position }; })() };`);
      r.indicator = ind;
      if (ind) {
        await c.mouseMove(ind.cx, ind.cy);
        await sleep(300);
        r.indicatorHovered = await ev(`
          const pv = window.__hf.view().containerEl.querySelector('.markdown-preview-view');
          const h = Array.from(pv.querySelectorAll('h2')).find((x) => x.textContent.includes('Section A'));
          const s = h.querySelector('.heading-collapse-indicator');
          const cs = getComputedStyle(s);
          return { opacity: cs.opacity, display: cs.display };`);
        await c.clickAt(ind.cx, ind.cy);
        await sleep(800);
        r.afterIndicatorClick = await snap();
        await c.clickAt(ind.cx, ind.cy);
        await sleep(800);
        r.afterIndicatorClickAgain = await snap();
      }
    }
    return r;
  });

  await step('s06-edge-headings', async (r) => {
    r.opened = await ev(`return await window.__hf.open('Fold Edges.md', 'live');`);
    r.lines = await ev(`return window.__hf.rendered();`);
    r.cases = {};
    const TARGETS = [
      ['line-zero-heading', 0], ['setext-heading', 4], ['setext-underline', 5],
      ['heading-in-callout', 10], ['heading-in-blockquote', 13], ['empty-last-section', 16],
    ];
    for (const [label, line] of TARGETS) {
      r.cases[label] = await ev(`
        const u = window.__hf;
        u.run('editor:unfold-all');
        await u.sleep(400);
        const caret = u.caretTo(${line}, 0);
        const lineText = u.view().editor.getLine(${line});
        const ran = u.run('editor:toggle-fold');
        await u.sleep(600);
        return { caret, lineText, ran, live: u.liveFolds(), renderedCount: u.rendered().length };`);
    }
    r.noTrailingNewline = await ev(`
      const u = window.__hf;
      const f = app.vault.getAbstractFileByPath('Fold Edges.md');
      const before = await app.vault.read(f);
      return { endsWithNewline: before.endsWith('\\n'), length: before.length };`);
    r.bytesAfter = fs.readFileSync(vp('Fold Edges.md'), 'utf8');
    return r;
  });

  await step('s07-keyboard-and-link-jump', async (r) => {
    r.hotkeys = await ev(`
      const hm = app.hotkeyManager;
      const fmt = (hk) => hk ? hk.map((h) => (h.modifiers || []).join('+') + (h.modifiers && h.modifiers.length ? '+' : '') + h.key) : null;
      const ids = ['editor:toggle-fold', 'editor:fold-all', 'editor:unfold-all', 'editor:fold-more', 'editor:fold-less', 'editor:toggle-fold-properties'];
      const o = {};
      for (const id of ids) o[id] = { name: app.commands.commands[id].name, defaults: fmt(hm.defaultKeys[id]), custom: fmt(hm.customKeys && hm.customKeys[id]) };
      return o;`);
    // The chords CodeMirror itself binds for folding.
    await ev(`
      const u = window.__hf;
      await u.open('Fold Probe.md', 'live');
      u.run('editor:unfold-all');
      await u.sleep(500);
      u.caretTo(12, 0);
      return true;`);
    for (const [label, key, mods] of [
      ['ctrl-shift-bracketleft', '[', { ctrl: true, shift: true }],
      ['ctrl-shift-bracketright', ']', { ctrl: true, shift: true }],
      ['ctrl-alt-bracketleft', '[', { ctrl: true, alt: true }],
    ]) {
      await c.pressKey(key, mods);
      await sleep(500);
      r['key_' + label] = await ev(`return { live: window.__hf.liveFolds(), lines: window.__hf.rendered().length };`);
    }
    // A link into a folded section.
    r.linkJump = await ev(`
      const u = window.__hf;
      await u.open('Fold Probe.md', 'live');
      u.run('editor:unfold-all');
      await u.sleep(400);
      u.caretTo(20, 0);
      u.run('editor:toggle-fold');
      await u.sleep(600);
      const foldedBefore = u.liveFolds();
      await u.open('Link Source.md', 'live');
      await u.sleep(500);
      await app.workspace.openLinkText('Fold Probe#Section B', 'Link Source.md', false);
      await u.sleep(1200);
      return { foldedBefore, after: u.snap(), caret: u.view().editor.getCursor() };`);
    return r;
  });

  // Leave a reading-view-written record behind, then quit so the
  // after-restart phase sees a cold load.
  await step('s08-leave-a-record-and-quit', async (r) => {
    r.setUp = await ev(`
      const u = window.__hf;
      await u.open('Fold Probe.md', 'live');
      u.run('editor:unfold-all');
      await u.sleep(500);
      u.caretTo(12, 0); u.run('editor:toggle-fold'); await u.sleep(500);
      u.caretTo(20, 0); u.run('editor:toggle-fold'); await u.sleep(500);
      const inEditor = u.snap();
      await u.setMode('reading');
      await u.sleep(1000);
      const inReading = u.snap();
      return { inEditor, inReading };`);
    r.recordBeforeQuit = await ev(`return window.__hf.foldKeys();`);
    r.localStorageAll = await ev(`return Object.keys(localStorage).sort();`);
    save();
    await c.quit();
    return r;
  });
  out.finishedAt = new Date().toISOString();
  save();
  process.exit(0);
}

if (phase === 'after-restart') {
  await step('s09-cold-load', async (r) => {
    r.keysAtLaunch = await ev(`return window.__hf.foldKeys();`);
    r.openedInEditor = await ev(`
      const u = window.__hf;
      await u.open('Fold Probe.md', 'live');
      await u.sleep(1200);
      return u.snap();`);
    r.renderedLineCount = (r.openedInEditor.rendered || []).length;
    r.placeholders = await ev(`
      const el = window.__hf.view().containerEl;
      return Array.from(el.querySelectorAll('.cm-foldPlaceholder')).map((x) => x.textContent);`);
    r.thenReading = await ev(`
      const u = window.__hf;
      await u.setMode('reading');
      await u.sleep(1000);
      return u.snap();`);
    r.backInEditor = await ev(`
      const u = window.__hf;
      await u.setMode('live');
      await u.sleep(1000);
      return u.snap();`);
    r.subfolderKeyStillThere = await ev(`return window.__hf.foldKeys();`);
    return r;
  });
  out.finishedAt = new Date().toISOString();
  save();
  c.close();
  process.exit(0);
}
