// The editing commands the backlog's "Declined until Obsidian is probed"
// section waits on, driven over CDP: the bare `editor:set-heading` cycle,
// `editor:cycle-list-checklist`, `editor:toggle-checklist-status` on lines
// with no checkbox, the six insert-block commands at three caret positions,
// `editor:clear-formatting` and the six highlight colours, the default
// hotkeys (Move line up/down among them), the formatting toggles across a
// multi-line selection, and `editor:insert-table` on a non-blank line.
//
// Every case seeds the note THROUGH THE VAULT (`app.vault.modify`), not with
// `editor.setValue`: a command that touches the file reloads the editor from
// disk, and with only the editor seeded that reload silently restores the
// previous case's bytes under the command's own edit. `seedOk` on every case
// records that the editor really held the seed before the command ran.
// Step s00 proves once that what the editor reports is what reaches the file.
//
// Commands are dispatched the way the palette does it
// (`app.commands.executeCommandById`). Where that changes nothing, the case
// also calls the command's own `editorCallback`/`editorCheckCallback` with an
// explicit editor and view, which separates "the dispatch never reached it"
// from "the operation is a no-op here".
//
// Phases: fixtures | run   (node editing-commands.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 420000;

function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.editingCommands = all.editingCommands || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.editingCommands[phase] = out;
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

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('Probe.md'), 'seed\n');
  fs.writeFileSync(vp('Footnotes.md'), 'no footnotes here\n');
  fs.writeFileSync(vp('Other.md'), '# Other\n\nA link target.\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({
    'file-explorer': true, 'global-search': true, graph: true, backlink: true, 'outgoing-link': true,
    bookmarks: true, 'command-palette': true, switcher: true, outline: true, 'tag-pane': true, 'page-preview': true,
  }, null, 2));
  save();
  process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
save();

// ---------------------------------------------------------------------------
// Renderer-side helpers, installed once on `window.__p`.
// ---------------------------------------------------------------------------
await c.evaluate(`
window.__p = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  // The ACTIVE markdown leaf, not the first one: a second leaf makes
  // executeCommandById act on a view the probe is neither seeding nor
  // reading, and every case then reads back as an untouched no-op.
  leaf() {
    const a = app.workspace.activeLeaf;
    if (a && a.view && a.view.getViewType() === 'markdown') return a;
    const ls = app.workspace.getLeavesOfType('markdown');
    return ls.length ? ls[0] : app.workspace.getLeaf(false);
  },
  view() { return this.leaf().view; },
  ed() { return this.view().editor; },
  // Reuse the one markdown leaf rather than opening another.
  async open(file, mode) {
    const f = app.vault.getAbstractFileByPath(file);
    const ls = app.workspace.getLeavesOfType('markdown');
    const leaf = ls.length ? ls[0] : app.workspace.getLeaf(false);
    await leaf.openFile(f);
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await this.sleep(400);
    await this.setMode(mode || 'live');
    await this.sleep(150);
    return { mode: this.mode(), leaves: app.workspace.getLeavesOfType('markdown').length, file: this.view().file.path };
  },
  async setMode(mode) {
    const leaf = this.leaf();
    const st = leaf.getViewState();
    st.state = Object.assign({}, st.state, {
      mode: mode === 'reading' ? 'preview' : 'source',
      source: mode === 'source',
    });
    await leaf.setViewState(st);
    await this.sleep(220);
    return this.mode();
  },
  mode() {
    const st = this.leaf().getViewState().state;
    return { mode: st.mode, source: st.source };
  },
  // Seed through the vault so the file and the editor agree: a command that
  // reloads the file then reloads the seed, not the previous case's bytes.
  async seed(spec) {
    const view = this.view();
    const ed = this.ed();
    await app.vault.modify(view.file, spec.content);
    await this.sleep(spec.settle || 140);
    let seedOk = ed.getValue() === spec.content;
    if (!seedOk) { ed.setValue(spec.content); await this.sleep(100); seedOk = ed.getValue() === spec.content; }
    ed.focus();
    if (spec.sel) ed.setSelection(spec.sel[0], spec.sel[1]);
    else ed.setCursor(spec.cursor || { line: 0, ch: 0 });
    const r = this.read();
    r.seedOk = seedOk;
    return r;
  },
  read() {
    const ed = this.ed();
    return {
      value: ed.getValue(),
      cursor: ed.getCursor(),
      from: ed.getCursor('from'),
      to: ed.getCursor('to'),
      sel: ed.getSelection(),
    };
  },
  popups() {
    const m = document.querySelector('.modal');
    const s = document.querySelector('.suggestion-container');
    return {
      modal: m ? { title: (m.querySelector('.modal-title') || {}).textContent, text: (m.querySelector('.modal-content') || {}).textContent.slice(0, 200) } : null,
      suggester: s ? { items: Array.from(s.querySelectorAll('.suggestion-item')).slice(0, 4).map((x) => x.textContent) } : null,
    };
  },
  // Call the command's own callback with an explicit editor and view. Used
  // only where executeCommandById changed nothing, to tell a dispatch that
  // never arrived from an operation that is a no-op in this position.
  direct(id) {
    const cmd = app.commands.commands[id];
    if (!cmd) return 'absent';
    const ed = this.ed(), view = this.view();
    if (cmd.editorCallback) { cmd.editorCallback(ed, view); return 'editorCallback'; }
    if (cmd.editorCheckCallback) { const r = cmd.editorCheckCallback(false, ed, view); return 'editorCheckCallback:' + r; }
    if (cmd.checkCallback) { const r = cmd.checkCallback(false); return 'checkCallback:' + r; }
    if (cmd.callback) { cmd.callback(); return 'callback'; }
    return 'none';
  },
  async run(spec) {
    if (spec.mode) await this.setMode(spec.mode);
    const before = await this.seed(spec);
    const steps = [];
    for (const cmd of spec.cmds) {
      let ran = null, err = null;
      try { ran = app.commands.executeCommandById(cmd); } catch (e) { err = String(e); }
      await this.sleep(spec.wait || 140);
      let r = this.read();
      r.cmd = cmd; r.ran = ran; if (err) r.err = err;
      // Unchanged? Try the callback directly before believing it.
      if (spec.probeDirect !== false && r.value === before.value && r.cursor.line === before.cursor.line && r.cursor.ch === before.cursor.ch) {
        try {
          r.dispatch = {
            activeEditorIsView: app.workspace.activeEditor === this.view(),
            activeLeafType: app.workspace.activeLeaf && app.workspace.activeLeaf.view.getViewType(),
            markdownLeaves: app.workspace.getLeavesOfType('markdown').length,
            activeElement: document.activeElement && document.activeElement.className,
          };
          r.directPath = this.direct(cmd);
          await this.sleep(spec.wait || 140);
          const after = this.read();
          r.directValue = after.value;
          r.directCursor = after.cursor;
          r.directChanged = after.value !== before.value;
        } catch (e) { r.directErr = String(e); }
      }
      const p = this.popups();
      if (p.modal || p.suggester) r.popups = p;
      steps.push(r);
    }
    return { before, steps };
  },
};
return true;`);

const run = (spec) => c.evaluate(`return await window.__p.run(${JSON.stringify(spec)});`);
const seed = (spec) => c.evaluate(`return await window.__p.seed(${JSON.stringify(spec)});`);
const read = () => c.evaluate(`return window.__p.read();`);
const setMode = (m) => c.evaluate(`return await window.__p.setMode(${JSON.stringify(m)});`);
const esc = async () => { await c.pressKey('Escape'); await sleep(120); };

async function once(content, cursorOrSel, cmd, extra = {}) {
  const spec = { content, cmds: [cmd], ...extra };
  if (Array.isArray(cursorOrSel)) spec.sel = cursorOrSel; else spec.cursor = cursorOrSel;
  const r = await run(spec);
  const s = r.steps[0];
  const o = { in: content, caret: cursorOrSel, out: s.value, cursor: s.cursor, sel: s.sel, ran: s.ran, seedOk: r.before.seedOk };
  if (s.directPath) { o.directPath = s.directPath; o.directValue = s.directValue; o.directChanged = s.directChanged; o.directCursor = s.directCursor; }
  if (s.popups) o.popups = s.popups;
  return o;
}
const cycle = (content, caret, cmd, n) => {
  const spec = { content, cmds: Array(n).fill(cmd), probeDirect: false };
  if (Array.isArray(caret)) spec.sel = caret; else spec.cursor = caret;
  return run(spec);
};

// ---------------------------------------------------------------------------

await step('s00-context', async (r) => {
  r.opened = await c.evaluate(`
    app.workspace.leftSplit.collapse();
    return await window.__p.open('Probe.md', 'live');`);
  r.ua = await c.evaluate(`return navigator.userAgent;`);
  r.versions = await c.evaluate(`return { apiVersion: String(app.apiVersion), appVersion: String(app.appVersion), keys: Object.keys(app).filter((k) => /version/i.test(k)) };`);
  r.mode = await c.evaluate(`return window.__p.mode();`);
  r.config = await c.evaluate(`
    const keys = ['autoPairBrackets','autoPairMarkdown','tabSize','useTab','livePreview','defaultViewMode','smartIndentList','foldHeading','foldIndent','showLineNumber','strictLineBreaks','readableLineLength','vimMode','spellcheck'];
    const o = {}; for (const k of keys) o[k] = app.vault.getConfig(k); return o;`);
  const IDS = [
    'editor:set-heading', 'editor:set-heading-0', 'editor:cycle-list-checklist', 'editor:toggle-checklist-status',
    'editor:insert-callout', 'editor:insert-codeblock', 'editor:insert-horizontal-rule', 'editor:insert-footnote',
    'editor:insert-mathblock', 'editor:insert-tag', 'editor:insert-table', 'editor:insert-embed', 'editor:insert-link',
    'editor:insert-wikilink', 'editor:clear-formatting', 'editor:toggle-highlight',
    'editor:toggle-highlight-red', 'editor:toggle-highlight-orange', 'editor:toggle-highlight-yellow',
    'editor:toggle-highlight-green', 'editor:toggle-highlight-blue', 'editor:toggle-highlight-purple',
    'editor:swap-line-up', 'editor:swap-line-down', 'editor:indent-list', 'editor:unindent-list',
    'editor:toggle-code', 'editor:toggle-inline-math', 'editor:toggle-comments', 'editor:toggle-bold',
    'editor:toggle-italics', 'editor:toggle-strikethrough', 'editor:toggle-bullet-list',
    'editor:toggle-numbered-list', 'editor:toggle-blockquote',
  ];
  r.commands = await c.evaluate(`
    const ids = ${JSON.stringify(IDS)};
    const hm = app.hotkeyManager;
    const fmt = (hk) => hk ? hk.map((h) => (h.modifiers || []).join('+') + (h.modifiers && h.modifiers.length ? '+' : '') + h.key) : null;
    const o = {};
    for (const id of ids) {
      const cmd = app.commands.commands[id];
      o[id] = {
        present: !!cmd, name: cmd ? cmd.name : null,
        kind: cmd ? (cmd.editorCallback ? 'editorCallback' : cmd.editorCheckCallback ? 'editorCheckCallback' : cmd.checkCallback ? 'checkCallback' : cmd.callback ? 'callback' : 'none') : null,
        defaultHotkeys: fmt(hm.defaultKeys && hm.defaultKeys[id]),
        customHotkeys: fmt(hm.customKeys && hm.customKeys[id]),
      };
    }
    return o;`);
  await c.evaluate(`
    await app.vault.modify(window.__p.view().file, 'control line\\n\\nsecond\\n');
    await window.__p.sleep(300);
    const ed = window.__p.ed();
    ed.focus(); ed.setCursor({ line: 0, ch: 7 });
    ed.replaceSelection('XX');
    await window.__p.sleep(200);
    await window.__p.view().save();
    await window.__p.sleep(600);
    return true;`);
  r.diskControl = {
    editor: await c.evaluate(`return window.__p.ed().getValue();`),
    disk: fs.readFileSync(vp('Probe.md'), 'utf8'),
    diskBytes: Array.from(fs.readFileSync(vp('Probe.md'))),
  };
  return r;
});

await step('s01-set-heading-bare', async (r) => {
  const lines = [
    ['plain text', { line: 0, ch: 3 }],
    ['# one', { line: 0, ch: 3 }],
    ['## two', { line: 0, ch: 4 }],
    ['### three', { line: 0, ch: 5 }],
    ['###### six', { line: 0, ch: 8 }],
    ['> quoted text', { line: 0, ch: 5 }],
    ['> # quoted h1', { line: 0, ch: 7 }],
    ['- bullet text', { line: 0, ch: 5 }],
    ['', { line: 0, ch: 0 }],
  ];
  r.once = [];
  for (const [content, cursor] of lines) r.once.push(await once(content, cursor, 'editor:set-heading'));
  r.withSelection = await once('plain text', [{ line: 0, ch: 0 }, { line: 0, ch: 5 }], 'editor:set-heading');
  r.multiLine = await once('one\ntwo', [{ line: 0, ch: 0 }, { line: 1, ch: 3 }], 'editor:set-heading');
  r.sourceMode = await once('plain text', { line: 0, ch: 3 }, 'editor:set-heading', { mode: 'source' });
  await setMode('live');
  // Does it remember a level set by the numbered commands?
  r.afterSetHeading2 = await run({ content: 'plain text', cursor: { line: 0, ch: 3 }, cmds: ['editor:set-heading-2', 'editor:set-heading', 'editor:set-heading', 'editor:set-heading'] });
  r.cycleFromPlain = await cycle('plain text', { line: 0, ch: 3 }, 'editor:set-heading', 8);
  r.cycleFromH1 = await cycle('# one', { line: 0, ch: 3 }, 'editor:set-heading', 8);
  r.setHeading0 = await once('# one', { line: 0, ch: 3 }, 'editor:set-heading-0');
  r.setHeading3 = await once('plain text', { line: 0, ch: 3 }, 'editor:set-heading-3');
  // A real keystroke, through a hotkey assigned for the probe.
  r.boundHotkey = await c.evaluate(`
    app.hotkeyManager.setHotkeys('editor:set-heading', [{ modifiers: ['Alt'], key: 'H' }]);
    app.hotkeyManager.bake();
    return app.hotkeyManager.customKeys['editor:set-heading'];`);
  await seed({ content: 'plain text', cursor: { line: 0, ch: 3 } });
  await c.pressKey('h', { alt: true });
  await sleep(350);
  r.afterAltH = await read();
  await c.evaluate(`app.hotkeyManager.removeHotkeys('editor:set-heading'); app.hotkeyManager.bake(); return true;`);
  return r;
});

await step('s02-cycle-list-checklist', async (r) => {
  const starts = [
    ['plain text', { line: 0, ch: 3 }],
    ['- bullet', { line: 0, ch: 4 }],
    ['* star', { line: 0, ch: 4 }],
    ['- [ ] open', { line: 0, ch: 8 }],
    ['- [x] done', { line: 0, ch: 8 }],
    ['- [?] q', { line: 0, ch: 7 }],
    ['1. ordered', { line: 0, ch: 6 }],
    ['> quoted', { line: 0, ch: 5 }],
    ['', { line: 0, ch: 0 }],
  ];
  r.cycles = {};
  for (const [content, cursor] of starts) r.cycles[content || '(empty)'] = await cycle(content, cursor, 'editor:cycle-list-checklist', 7);
  r.inOrderedList = await cycle('1. one\n2. two\n3. three', { line: 1, ch: 5 }, 'editor:cycle-list-checklist', 7);
  r.nested = await cycle('- one\n\t- two', { line: 1, ch: 5 }, 'editor:cycle-list-checklist', 5);
  r.multiLineSel = await cycle('one\n- two\n- [ ] three', [{ line: 0, ch: 0 }, { line: 2, ch: 11 }], 'editor:cycle-list-checklist', 5);
  r.blankInSel = await cycle('- one\n\n- [x] two', [{ line: 0, ch: 0 }, { line: 2, ch: 9 }], 'editor:cycle-list-checklist', 4);
  return r;
});

await step('s03-toggle-checklist-status', async (r) => {
  const cases = [
    ['plain text', { line: 0, ch: 3 }],
    ['- bullet', { line: 0, ch: 4 }],
    ['1. ordered', { line: 0, ch: 6 }],
    ['- [ ] open', { line: 0, ch: 8 }],
    ['- [x] done', { line: 0, ch: 8 }],
    ['- [?] q', { line: 0, ch: 7 }],
    ['- [-] dash', { line: 0, ch: 8 }],
    ['> quoted', { line: 0, ch: 5 }],
    ['# heading', { line: 0, ch: 4 }],
    ['', { line: 0, ch: 0 }],
    ['    indented', { line: 0, ch: 8 }],
  ];
  r.single = [];
  for (const [content, cursor] of cases) r.single.push(await once(content, cursor, 'editor:toggle-checklist-status'));
  r.twice = await cycle('plain text', { line: 0, ch: 3 }, 'editor:toggle-checklist-status', 4);
  // The selection matrix: the backlog's open question is the mixed case.
  const SEL = [
    ['all-unchecked', '- [ ] a\n- [ ] b'],
    ['all-checked', '- [x] a\n- [x] b'],
    ['mixed-statuses', '- [ ] a\n- [x] b'],
    ['no-checkboxes', '- a\n- b'],
    ['one-has-one-not', '- [ ] a\n- b'],
    ['one-not-one-has', '- a\n- [x] b'],
    ['plain-lines', 'a\nb'],
    ['with-blank-line', '- [ ] a\n\n- [x] b'],
    ['mixed-four', '- [ ] a\n- [x] b\n- c\nplain d'],
  ];
  r.selections = {};
  for (const [label, content] of SEL) {
    const last = content.split('\n').length - 1;
    const end = { line: last, ch: content.split('\n')[last].length };
    r.selections[label] = await cycle(content, [{ line: 0, ch: 0 }, end], 'editor:toggle-checklist-status', 3);
  }
  await seed({ content: 'plain text', cursor: { line: 0, ch: 3 } });
  await c.pressKey('l', { ctrl: true });
  await sleep(350);
  r.realModL = await read();
  return r;
});

await step('s04-insert-blocks', async (r) => {
  const CMDS = ['editor:insert-callout', 'editor:insert-codeblock', 'editor:insert-horizontal-rule',
    'editor:insert-mathblock', 'editor:insert-tag'];
  const POS = [
    ['blank-line-middle', 'before\n\nafter', { line: 1, ch: 0 }],
    ['empty-document', '', { line: 0, ch: 0 }],
    ['mid-paragraph', 'hello world', { line: 0, ch: 5 }],
    ['end-of-paragraph', 'hello world', { line: 0, ch: 11 }],
    ['end-of-file-no-newline', 'first\nlast line', { line: 1, ch: 9 }],
    ['selection', 'alpha beta', [{ line: 0, ch: 0 }, { line: 0, ch: 5 }]],
    ['in-list', '- item one', { line: 0, ch: 10 }],
  ];
  r.cases = {};
  for (const cmd of CMDS) {
    r.cases[cmd] = {};
    for (const [label, content, caret] of POS) { r.cases[cmd][label] = await once(content, caret, cmd); await esc(); }
  }
  // Footnotes: a long settle so the number is not read off a metadata cache
  // one edit behind. Still the same leaf — a reopen here broke every case
  // after it.
  r.footnotes = {};
  for (const [label, content, caret] of POS) {
    r.footnotes[label] = await once(content, caret, 'editor:insert-footnote', { settle: 900 });
    await esc();
  }
  return r;
});

await step('s05-highlight-colours', async (r) => {
  const COLOURS = ['red', 'orange', 'yellow', 'green', 'blue', 'purple'];
  r.colours = {};
  for (const col of COLOURS) {
    r.colours[col] = {
      overWord: await once('plain word here', [{ line: 0, ch: 6 }, { line: 0, ch: 10 }], `editor:toggle-highlight-${col}`),
      twice: await cycle('plain word here', [{ line: 0, ch: 6 }, { line: 0, ch: 10 }], `editor:toggle-highlight-${col}`, 2),
      atCaret: await once('plain word here', { line: 0, ch: 6 }, `editor:toggle-highlight-${col}`),
    };
  }
  r.bareHighlight = {
    overWord: await once('plain word here', [{ line: 0, ch: 6 }, { line: 0, ch: 10 }], 'editor:toggle-highlight'),
    atCaret: await once('plain word here', { line: 0, ch: 6 }, 'editor:toggle-highlight'),
    twice: await cycle('plain word here', [{ line: 0, ch: 6 }, { line: 0, ch: 10 }], 'editor:toggle-highlight', 2),
  };
  r.colourOverPlain = await once('a ==word== b', [{ line: 0, ch: 4 }, { line: 0, ch: 8 }], 'editor:toggle-highlight-green');
  r.colourOverPlainWhole = await once('a ==word== b', [{ line: 0, ch: 2 }, { line: 0, ch: 10 }], 'editor:toggle-highlight-green');
  r.colourOverColour = await c.evaluate(`
    const p = window.__p;
    await p.seed({ content: 'plain word here', sel: [{ line: 0, ch: 6 }, { line: 0, ch: 10 }] });
    app.commands.executeCommandById('editor:toggle-highlight-red');
    await p.sleep(250);
    const first = p.read();
    const line = p.ed().getLine(0);
    const i = line.indexOf('word');
    p.ed().setSelection({ line: 0, ch: i }, { line: 0, ch: i + 4 });
    app.commands.executeCommandById('editor:toggle-highlight-blue');
    await p.sleep(250);
    return { first, second: p.read() };`);
  r.colourMultiLine = await once('one\ntwo\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], 'editor:toggle-highlight-yellow');
  // How does the renderer treat whatever markup the colour wrote?
  r.renderedColour = await c.evaluate(`
    const p = window.__p;
    await p.seed({ content: 'plain word here', sel: [{ line: 0, ch: 6 }, { line: 0, ch: 10 }] });
    app.commands.executeCommandById('editor:toggle-highlight-red');
    await p.sleep(250);
    const src = p.ed().getValue();
    await p.view().save();
    await p.sleep(400);
    await p.setMode('reading');
    await p.sleep(700);
    const el = p.view().containerEl.querySelector('.markdown-preview-view');
    const marks = Array.from(el.querySelectorAll('mark')).map((m) => ({
      html: m.outerHTML.slice(0, 300),
      text: m.textContent,
      style: (() => { const s = getComputedStyle(m); return { background: s.backgroundColor, color: s.color }; })(),
    }));
    const out = { src, html: el.innerHTML.slice(0, 600), marks };
    await p.setMode('live');
    await p.sleep(300);
    return out;`);
  return r;
});

await step('s06-clear-formatting', async (r) => {
  const CLEAR = [
    ['bold-italic-nested', '**bold _italic_ text**', [{ line: 0, ch: 0 }, { line: 0, ch: 22 }]],
    ['inner-word-only', '**bold _italic_ text**', [{ line: 0, ch: 7 }, { line: 0, ch: 15 }]],
    ['two-markers', '**==both==** tail', [{ line: 0, ch: 0 }, { line: 0, ch: 12 }]],
    ['strike-and-code', '~~gone~~ and `code`', [{ line: 0, ch: 0 }, { line: 0, ch: 19 }]],
    ['heading-line', '## Heading text', [{ line: 0, ch: 0 }, { line: 0, ch: 15 }]],
    ['heading-caret', '## Heading text', { line: 0, ch: 6 }],
    ['quote-line', '> quoted text', [{ line: 0, ch: 0 }, { line: 0, ch: 13 }]],
    ['bullet-line', '- **bold** item', [{ line: 0, ch: 0 }, { line: 0, ch: 15 }]],
    ['link', 'see [[Other|alias]] end', [{ line: 0, ch: 0 }, { line: 0, ch: 23 }]],
    ['caret-no-selection', '**bold** text', { line: 0, ch: 4 }],
    ['caret-inside-bold', '**bold** text', { line: 0, ch: 5 }],
    ['multi-line', '**one**\n_two_\n==three==', [{ line: 0, ch: 0 }, { line: 2, ch: 9 }]],
    ['math-and-tag', '$x^2$ #tag', [{ line: 0, ch: 0 }, { line: 0, ch: 10 }]],
    ['plain-text', 'nothing to clear', [{ line: 0, ch: 0 }, { line: 0, ch: 16 }]],
    ['partial-marker', 'a **bold** b', [{ line: 0, ch: 4 }, { line: 0, ch: 8 }]],
  ];
  r.clear = {};
  for (const [label, content, caret] of CLEAR) r.clear[label] = await once(content, caret, 'editor:clear-formatting');
  return r;
});

await step('s07-move-line', async (r) => {
  r.defaults = await c.evaluate(`
    const hm = app.hotkeyManager;
    const fmt = (hk) => hk ? hk.map((h) => (h.modifiers || []).join('+') + (h.modifiers && h.modifiers.length ? '+' : '') + h.key) : null;
    const withDefaults = {};
    for (const id of Object.keys(app.commands.commands)) { const d = fmt(hm.defaultKeys && hm.defaultKeys[id]); if (d) withDefaults[id] = d; }
    return { count: Object.keys(withDefaults).length, withDefaults, swapUp: withDefaults['editor:swap-line-up'] || null, swapDown: withDefaults['editor:swap-line-down'] || null };`);
  r.commandUp = await once('one\ntwo\nthree', { line: 1, ch: 1 }, 'editor:swap-line-up');
  r.commandDown = await once('one\ntwo\nthree', { line: 1, ch: 1 }, 'editor:swap-line-down');
  r.commandUpOnFirst = await once('one\ntwo', { line: 0, ch: 1 }, 'editor:swap-line-up');
  r.commandDownOnLast = await once('one\ntwo', { line: 1, ch: 1 }, 'editor:swap-line-down');
  r.commandUpSelection = await once('one\ntwo\nthree', [{ line: 1, ch: 0 }, { line: 2, ch: 5 }], 'editor:swap-line-up');
  await seed({ content: 'one\ntwo\nthree', cursor: { line: 1, ch: 1 } });
  await c.pressKey('ArrowUp', { alt: true });
  await sleep(350);
  r.altArrowUp = await read();
  await seed({ content: 'one\ntwo\nthree', cursor: { line: 1, ch: 1 } });
  await c.pressKey('ArrowDown', { alt: true });
  await sleep(350);
  r.altArrowDown = await read();
  return r;
});

await step('s08-toggles-multiline', async (r) => {
  const TOGGLES = ['editor:toggle-code', 'editor:toggle-inline-math', 'editor:toggle-comments',
    'editor:toggle-bold', 'editor:toggle-italics', 'editor:toggle-strikethrough',
    'editor:toggle-bullet-list', 'editor:toggle-numbered-list', 'editor:toggle-blockquote'];
  r.threeLines = {};
  for (const cmd of TOGGLES) {
    r.threeLines[cmd] = {
      wholeSelection: await once('one\ntwo\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], cmd),
      partialEnds: await once('one\ntwo\nthree', [{ line: 0, ch: 1 }, { line: 2, ch: 2 }], cmd),
      twice: await cycle('one\ntwo\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], cmd, 2),
      singleWord: await once('one two three', [{ line: 0, ch: 4 }, { line: 0, ch: 7 }], cmd),
      withBlankLine: await once('one\n\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], cmd),
      caretOnly: await once('one two', { line: 0, ch: 4 }, cmd),
    };
  }
  r.mixed = {
    bulletSomeHave: await once('- one\ntwo\n- three', [{ line: 0, ch: 0 }, { line: 2, ch: 7 }], 'editor:toggle-bullet-list'),
    bulletAllHave: await once('- one\n- two\n- three', [{ line: 0, ch: 0 }, { line: 2, ch: 7 }], 'editor:toggle-bullet-list'),
    quoteSomeHave: await once('> one\ntwo\n> three', [{ line: 0, ch: 0 }, { line: 2, ch: 7 }], 'editor:toggle-blockquote'),
    quoteAllHave: await once('> one\n> two', [{ line: 0, ch: 0 }, { line: 1, ch: 5 }], 'editor:toggle-blockquote'),
    numberedSomeHave: await once('1. one\ntwo\n3. three', [{ line: 0, ch: 0 }, { line: 2, ch: 8 }], 'editor:toggle-numbered-list'),
    numberedAllHave: await once('1. one\n2. two', [{ line: 0, ch: 0 }, { line: 1, ch: 6 }], 'editor:toggle-numbered-list'),
    numberedFromPlain: await once('a\nb\nc\nd', [{ line: 0, ch: 0 }, { line: 3, ch: 1 }], 'editor:toggle-numbered-list'),
    numberedAfterExisting: await once('1. one\na\nb', [{ line: 1, ch: 0 }, { line: 2, ch: 1 }], 'editor:toggle-numbered-list'),
    bulletIndented: await once('- one\n\t- two\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], 'editor:toggle-bullet-list'),
    bulletBlankBetween: await once('one\n\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], 'editor:toggle-bullet-list'),
    numberedBlankBetween: await once('one\n\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], 'editor:toggle-numbered-list'),
    quoteBlankBetween: await once('one\n\nthree', [{ line: 0, ch: 0 }, { line: 2, ch: 5 }], 'editor:toggle-blockquote'),
    commentsOnExisting: await once('%%one%%', [{ line: 0, ch: 0 }, { line: 0, ch: 7 }], 'editor:toggle-comments'),
    commentsMultiExisting: await once('%%\none\ntwo\n%%', [{ line: 1, ch: 0 }, { line: 2, ch: 3 }], 'editor:toggle-comments'),
    codeOnExisting: await once('`one`', [{ line: 0, ch: 0 }, { line: 0, ch: 5 }], 'editor:toggle-code'),
    mathOnExisting: await once('$x$', [{ line: 0, ch: 0 }, { line: 0, ch: 3 }], 'editor:toggle-inline-math'),
  };
  return r;
});

await step('s09-insert-table-non-blank', async (r) => {
  r.blankControl = await once('before\n\nafter', { line: 1, ch: 0 }, 'editor:insert-table');
  r.emptyDocument = await once('', { line: 0, ch: 0 }, 'editor:insert-table');
  r.endOfParagraph = await once('hello world', { line: 0, ch: 11 }, 'editor:insert-table');
  r.midParagraph = await once('hello world', { line: 0, ch: 5 }, 'editor:insert-table');
  r.startOfParagraph = await once('hello world', { line: 0, ch: 0 }, 'editor:insert-table');
  r.endOfFileAfterText = await once('para one\n\npara two', { line: 2, ch: 8 }, 'editor:insert-table');
  r.inList = await once('- item', { line: 0, ch: 6 }, 'editor:insert-table');
  r.inQuote = await once('> quoted', { line: 0, ch: 8 }, 'editor:insert-table');
  r.withSelection = await once('alpha beta', [{ line: 0, ch: 0 }, { line: 0, ch: 5 }], 'editor:insert-table');
  r.sourceMode = await once('hello world', { line: 0, ch: 11 }, 'editor:insert-table', { mode: 'source' });
  await setMode('live');
  return r;
});

// Last, because a Runtime.evaluate timeout in a settings step can hang the
// renderer's CDP endpoint: nothing above may depend on it.
await step('s10-hotkeys-tab', async (r) => {
  r.hotkeyTab = await c.evaluate(`
    app.setting.open(); app.setting.openTabById('hotkeys');
    await window.__p.sleep(600);
    const tab = app.setting.activeTab;
    const el = tab && tab.containerEl;
    const search = el && el.querySelector('input[type="search"], input[type="text"]');
    if (search) { search.value = 'Move line'; search.dispatchEvent(new Event('input', { bubbles: true })); }
    await window.__p.sleep(500);
    return el ? Array.from(el.querySelectorAll('.setting-item')).map((it) => ({
      name: (it.querySelector('.setting-item-name') || {}).textContent,
      keys: Array.from(it.querySelectorAll('.setting-hotkey')).map((k) => k.textContent),
    })) : null;`);
  await c.evaluate(`app.setting.close(); return true;`);
  await sleep(300);
  return r;
});

// Steps that switch the open file, and so must run after every step above:
// a reopen is what first made executeCommandById act on a view the probe was
// neither seeding nor reading.
await step('s11-followups', async (r) => {
  r.leaves = await c.evaluate(`return { markdown: app.workspace.getLeavesOfType('markdown').length, activeType: app.workspace.activeLeaf.view.getViewType(), file: window.__p.view().file.path };`);

  // The control that says the palette dispatch and a real keystroke agree:
  // Toggle bold ships Mod+B as a default hotkey.
  await seed({ content: 'one two three', sel: [{ line: 0, ch: 4 }, { line: 0, ch: 7 }] });
  await c.pressKey('b', { ctrl: true });
  await sleep(350);
  r.ctrlB = await read();
  r.execBold = await once('one two three', [{ line: 0, ch: 4 }, { line: 0, ch: 7 }], 'editor:toggle-bold');

  // cycle-list-checklist across a selection: the all / none / some question.
  const SEL = [
    ['all-plain', 'a\nb'],
    ['all-bullets', '- a\n- b'],
    ['all-unchecked', '- [ ] a\n- [ ] b'],
    ['all-checked', '- [x] a\n- [x] b'],
    ['bullet-and-unchecked', '- a\n- [ ] b'],
    ['plain-and-bullet', 'a\n- b'],
    ['plain-and-unchecked', 'a\n- [ ] b'],
    ['all-three-states', 'a\n- b\n- [ ] c'],
    ['with-blank-line', '- a\n\n- [x] b'],
  ];
  r.cycleSelections = {};
  for (const [label, content] of SEL) {
    const ls = content.split('\n');
    const end = { line: ls.length - 1, ch: ls[ls.length - 1].length };
    r.cycleSelections[label] = await cycle(content, [{ line: 0, ch: 0 }, end], 'editor:cycle-list-checklist', 4);
  }

  // Footnote numbering with definitions already in the file. Same leaf and a
  // long settle: reopening a file is what stops executeCommandById reaching
  // the view the probe seeds, and a short settle reads the number off a
  // metadata cache one edit behind.
  const FN = [
    ['noneYet', 'body text\n', { line: 0, ch: 9 }],
    ['oneExisting', 'body[^1] text\n\n[^1]: one\n', { line: 0, ch: 13 }],
    ['twoExisting', 'a[^1] b[^2] c\n\n[^1]: one\n[^2]: two\n', { line: 0, ch: 13 }],
    ['namedExisting', 'a[^note] b\n\n[^note]: named\n', { line: 0, ch: 10 }],
    ['gapExisting', 'a[^2] b\n\n[^2]: two\n', { line: 0, ch: 7 }],
    ['midDocument', '# Head\n\npara one\n\npara two\n', { line: 2, ch: 8 }],
    ['beforeExistingRef', 'a[^1] b\n\n[^1]: one\n', { line: 0, ch: 1 }],
  ];
  r.footnoteNumbering = {};
  for (const [label, content, caret] of FN) {
    r.footnoteNumbering[label] = await once(content, caret, 'editor:insert-footnote', { settle: 1100 });
  }
  return r;
});

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
