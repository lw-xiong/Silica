// The two rows of the editing backlog that need real keystrokes rather than a
// command: the auto-pair questions [[Specification#Observed auto-pair
// behavior, Obsidian 1.14.4]] still lists as unprobed, and Shift+Tab on
// ragged indentation.
//
// Keys go through Input.dispatchKeyEvent, so they carry focus and take the
// path a person's keyboard does — which is the whole point: the 2026-10-18
// auto-pair run typed with `document.execCommand`, and its Backspace and undo
// rows were synthetic events with no native editing fallback behind them.
//
// The note is seeded through the vault with a settle before the caret is
// placed: a file-change event landing after `setCursor` moves the caret under
// the keystroke, which is how an earlier run produced nonsense.
//
// Phases: fixtures | run   (node editing-typing.mjs <phase>)
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
all.editingTyping = all.editingTyping || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.editingTyping[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
const ONLY = process.env.ONLY_STEPS ? process.env.ONLY_STEPS.split(',') : null;
async function step(name, fn) {
  if (ONLY && !ONLY.some((p) => name.startsWith(p))) return null;
  out.order.push(name);
  if (Date.now() - T0 > BUDGET_MS) { out.steps[name] = { ok: false, error: 'skipped: time budget exhausted' }; save(); return null; }
  const t = Date.now(); const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save(); return out.steps[name];
}
const vp = (rel) => path.join(VAULT, rel);

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('Type.md'), 'seed\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'command-palette': true, switcher: true }, null, 2));
  save(); process.exit(0);
}
if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
save();

await c.evaluate(`
window.__t = {
  sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
  leaf() { const a = app.workspace.activeLeaf; if (a && a.view && a.view.getViewType() === 'markdown') return a; return app.workspace.getLeavesOfType('markdown')[0] || app.workspace.getLeaf(false); },
  view() { return this.leaf().view; },
  ed() { return this.view().editor; },
  async open(file, mode) {
    const leaf = this.leaf();
    await leaf.openFile(app.vault.getAbstractFileByPath(file));
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await this.sleep(400);
    await this.setMode(mode || 'live');
    return this.mode();
  },
  async setMode(mode) {
    const leaf = this.leaf(); const st = leaf.getViewState();
    st.state = Object.assign({}, st.state, { mode: mode === 'reading' ? 'preview' : 'source', source: mode === 'source' });
    await leaf.setViewState(st);
    await this.sleep(250);
    return this.mode();
  },
  mode() { const st = this.leaf().getViewState().state; return { mode: st.mode, source: st.source }; },
  async seed(spec) {
    const view = this.view(), ed = this.ed();
    await app.vault.modify(view.file, spec.content);
    await this.sleep(spec.settle || 450);
    let seedOk = ed.getValue() === spec.content;
    if (!seedOk) { ed.setValue(spec.content); await this.sleep(150); seedOk = ed.getValue() === spec.content; }
    ed.focus();
    if (spec.sel) ed.setSelection(spec.sel[0], spec.sel[1]); else ed.setCursor(spec.cursor || { line: 0, ch: 0 });
    const r = this.read(); r.seedOk = seedOk; return r;
  },
  read() { const ed = this.ed(); return { value: ed.getValue(), cursor: ed.getCursor(), sel: ed.getSelection() }; },
  config(k, v) { if (v === undefined) return app.vault.getConfig(k); app.vault.setConfig(k, v); return app.vault.getConfig(k); },
  popup() {
    const s = document.querySelector('.suggestion-container');
    return s ? { visible: s.offsetParent !== null, items: Array.from(s.querySelectorAll('.suggestion-item')).length } : null;
  },
};
return true;`);

const seed = (spec) => c.evaluate(`return await window.__t.seed(${JSON.stringify(spec)});`);
const read = () => c.evaluate(`return window.__t.read();`);
const setMode = (m) => c.evaluate(`return await window.__t.setMode(${JSON.stringify(m)});`);
const cfg = (k, v) => c.evaluate(`return window.__t.config(${JSON.stringify(k)}${v === undefined ? '' : ', ' + JSON.stringify(v)});`);

// Type a sequence of keys, reading the document after each one.
// A key is a single character, or {key, mods} for a named key.
async function typeSeq(content, caret, keys, extra = {}) {
  const spec = { content, ...extra };
  if (Array.isArray(caret)) spec.sel = caret; else spec.cursor = caret;
  const before = await seed(spec);
  const after = [];
  for (const k of keys) {
    if (typeof k === 'string') await c.typeChar(k);
    else await c.pressKey(k.key, k.mods || {});
    await sleep(k && k.wait ? k.wait : 140);
    const r = await read();
    r.key = typeof k === 'string' ? k : k.key + (k.mods ? ' ' + Object.keys(k.mods).join('+') : '');
    after.push(r);
  }
  return { in: content, caret, seedOk: before.seedOk, before: { value: before.value, cursor: before.cursor }, after };
}
// The common shape: one key, one answer.
async function one(content, caret, key, extra = {}) {
  const r = await typeSeq(content, caret, [key], extra);
  return { in: content, caret, key: r.after[0].key, out: r.after[0].value, cursor: r.after[0].cursor, seedOk: r.seedOk };
}

await step('s00-context', async (r) => {
  r.opened = await c.evaluate(`app.workspace.leftSplit.collapse(); return await window.__t.open('Type.md', 'live');`);
  r.ua = await c.evaluate(`return navigator.userAgent;`);
  r.config = await c.evaluate(`
    const keys = ['autoPairBrackets','autoPairMarkdown','tabSize','useTab','livePreview','smartIndentList'];
    const o = {}; for (const k of keys) o[k] = app.vault.getConfig(k); return o;`);
  // The control that real typing reaches the editor at all.
  r.typingControl = await one('', { line: 0, ch: 0 }, 'x');
  return r;
});

await step('s01-skip-over-each-closer', async (r) => {
  const PAIRS = [['(', ')'], ['[', ']'], ['{', '}'], ['"', '"'], ["'", "'"], ['`', '`'], ['*', '*'], ['_', '_']];
  r.fromEmpty = {};
  for (const [o_, c_] of PAIRS) r.fromEmpty[o_] = await typeSeq('', { line: 0, ch: 0 }, [o_, c_]);
  // After a space, which is where the markdown characters pair at all.
  r.afterSpace = {};
  for (const [o_, c_] of PAIRS) r.afterSpace[o_] = await typeSeq('say ', { line: 0, ch: 4 }, [o_, c_]);
  // A planted closer, no opener anywhere: does it still skip?
  r.planted = {};
  for (const [o_, c_] of PAIRS) r.planted[o_ + c_] = await one('x' + c_, { line: 0, ch: 1 }, c_);
  return r;
});

await step('s02-markdown-rule-halves', async (r) => {
  // Which half of "preceded by a space and the line ends here" is load bearing?
  const POS = [
    ['space-before-eol-ahead', 'say ', { line: 0, ch: 4 }],
    ['space-before-space-ahead', 'say  word', { line: 0, ch: 4 }],
    ['space-before-word-ahead', 'say x', { line: 0, ch: 4 }],
    ['space-before-punct-ahead', 'say .', { line: 0, ch: 4 }],
    ['space-before-space-then-eol', 'say  ', { line: 0, ch: 4 }],
    ['doc-start-eol-ahead', '', { line: 0, ch: 0 }],
    ['doc-start-word-ahead', 'word', { line: 0, ch: 0 }],
    ['after-punct-eol-ahead', 'say.', { line: 0, ch: 4 }],
    ['after-tab-eol-ahead', '\tx ', { line: 0, ch: 3 }],
  ];
  r.cases = {};
  for (const [label, content, caret] of POS) {
    r.cases[label] = {};
    for (const ch of ['*', '_', '`', '"', '(']) r.cases[label][ch] = await one(content, caret, ch);
  }
  return r;
});

await step('s03-word-boundary-for-the-rest', async (r) => {
  // `_ ~ = $ % <` were never run through the five positions.
  const POS = [
    ['before-word', 'word', { line: 0, ch: 0 }],
    ['inside-word', 'dont', { line: 0, ch: 3 }],
    ['after-word-eol', 'word', { line: 0, ch: 4 }],
    ['after-space-eol', 'say ', { line: 0, ch: 4 }],
    ['before-closer', '()', { line: 0, ch: 1 }],
  ];
  r.cases = {};
  for (const [label, content, caret] of POS) {
    r.cases[label] = {};
    for (const ch of ['_', '~', '=', '$', '%', '<']) r.cases[label][ch] = await one(content, caret, ch);
  }
  // A second one of the same character, where the first did not pair.
  r.doubled = {};
  for (const ch of ['~', '=', '$', '%', '<']) r.doubled[ch] = await typeSeq('say ', { line: 0, ch: 4 }, [ch, ch]);
  return r;
});

await step('s04-suppression-by-context', async (r) => {
  const CTX = [
    ['plain-control', 'say ', { line: 0, ch: 4 }],
    ['inline-code-closed', 'x `say ` tail', { line: 0, ch: 7 }],
    ['inline-code-at-eol', 'x `say `', { line: 0, ch: 7 }],
    ['blockquote', '> say ', { line: 0, ch: 6 }],
    ['nested-blockquote', '> > say ', { line: 0, ch: 8 }],
    ['fence-body', '```js\nsay \n```', { line: 1, ch: 4 }],
    ['math-block', '$$\nsay \n$$', { line: 1, ch: 4 }],
    ['heading', '# say ', { line: 0, ch: 6 }],
    ['list-item', '- say ', { line: 0, ch: 6 }],
    ['task-item', '- [ ] say ', { line: 0, ch: 10 }],
  ];
  r.live = {};
  for (const [label, content, caret] of CTX) {
    r.live[label] = {};
    for (const ch of ['(', '*', '`', '"']) r.live[label][ch] = await one(content, caret, ch);
  }
  await setMode('source');
  r.source = {};
  for (const [label, content, caret] of CTX) {
    r.source[label] = {};
    for (const ch of ['(', '*']) r.source[label][ch] = await one(content, caret, ch, { mode: 'source' });
  }
  await setMode('live');
  return r;
});

await step('s05-wikilink-brackets', async (r) => {
  r.twoOpeners = await typeSeq('', { line: 0, ch: 0 }, ['[', '[']);
  r.popupAfterTwo = await c.evaluate(`return window.__t.popup();`);
  r.thenCloser = await typeSeq('', { line: 0, ch: 0 }, ['[', '[', ']']);
  r.thenTwoClosers = await typeSeq('', { line: 0, ch: 0 }, ['[', '[', ']', ']']);
  r.escapeThenCloser = await typeSeq('', { line: 0, ch: 0 }, ['[', '[', { key: 'Escape' }, ']']);
  r.typedNameThenClosers = await typeSeq('', { line: 0, ch: 0 }, ['[', '[', 'T', 'y', ']', ']']);
  await c.pressKey('Escape'); await sleep(150);
  r.afterEscape = await read();
  r.threeOpeners = await typeSeq('', { line: 0, ch: 0 }, ['[', '[', '[']);
  await c.pressKey('Escape'); await sleep(150);
  r.embedOpener = await typeSeq('', { line: 0, ch: 0 }, ['!', '[', '[']);
  await c.pressKey('Escape'); await sleep(150);
  return r;
});

await step('s06-forward-delete-in-pair', async (r) => {
  // Auto-inserted pair: type the opener, then forward Delete.
  r.autoInserted = {};
  for (const o_ of ['(', '[', '{', '"', '*', '`']) {
    r.autoInserted[o_] = await typeSeq('', { line: 0, ch: 0 }, [o_, { key: 'Delete' }]);
  }
  // Planted pair, cursor between: Delete, and Backspace as the known control.
  r.plantedDelete = {};
  r.plantedBackspace = {};
  for (const p of ['()', '[]', '""', '**', '``']) {
    r.plantedDelete[p] = await one(p, { line: 0, ch: 1 }, { key: 'Delete' });
    r.plantedBackspace[p] = await one(p, { line: 0, ch: 1 }, { key: 'Backspace' });
  }
  r.contentInsideDelete = await one('(x)', { line: 0, ch: 1 }, { key: 'Delete' });
  r.plainControlDelete = await one('plain', { line: 0, ch: 2 }, { key: 'Delete' });
  return r;
});

await step('s07-cursor-round-trip', async (r) => {
  // Does the auto closer survive a cursor round trip, with a before-state?
  r.endThenHome = await typeSeq('', { line: 0, ch: 0 }, ['(', { key: 'End' }, { key: 'Home' }]);
  r.roundTripThenCloser = await typeSeq('', { line: 0, ch: 0 }, ['(', { key: 'End' }, { key: 'Home' }, { key: 'ArrowRight' }, ')']);
  r.lineAwayAndBack = await typeSeq('first\nsecond', { line: 0, ch: 5 }, ['(', { key: 'ArrowDown' }, { key: 'ArrowUp' }, ')']);
  // Is an abandoned closer ever removed? Type into the pair, then leave.
  r.abandoned = await typeSeq('seed', { line: 0, ch: 4 }, ['(', { key: 'End' }, ' ', 't', 'a', 'i', 'l']);
  r.abandonedSaved = await c.evaluate(`
    await window.__t.view().save();
    await window.__t.sleep(500);
    return window.__t.read();`);
  r.abandonedDisk = fs.readFileSync(vp('Type.md'), 'utf8');
  // Fast typing inside a pair, then one undo.
  r.fastThenUndo = await typeSeq('seed', { line: 0, ch: 4 }, ['(', 'a', 'b', 'c', { key: 'z', mods: { ctrl: true } }, { key: 'z', mods: { ctrl: true } }], { settle: 500 });
  return r;
});

await step('s08-shift-tab-ragged', async (r) => {
  const SHAPES = [
    ['three-space-list', '- a\n   - b', { line: 1, ch: 6 }],
    ['one-space-list', '- a\n - b', { line: 1, ch: 4 }],
    ['two-space-list', '- a\n  - b', { line: 1, ch: 5 }],
    ['five-space-list', '- a\n     - b', { line: 1, ch: 8 }],
    ['tab-list', '- a\n\t- b', { line: 1, ch: 4 }],
    ['tab-plus-space-list', '- a\n\t - b', { line: 1, ch: 5 }],
    ['three-space-plain', '   plain', { line: 0, ch: 8 }],
    ['top-level-list', '- a', { line: 0, ch: 3 }],
    ['caret-in-indent', '- a\n   - b', { line: 1, ch: 2 }],
  ];
  for (const size of [2, 4]) {
    for (const useTab of [true, false]) {
      await cfg('tabSize', size);
      await cfg('useTab', useTab);
      // The editor rebuilds its indent extension when the view is recreated,
      // not when setConfig returns: without this the tabSize-2 rows were
      // driven by an editor still indenting four columns.
      await c.evaluate(`return await window.__t.open('Type.md', 'live');`);
      await sleep(500);
      const key = `tabSize${size}-useTab${useTab}`;
      // Calibration: what one Tab actually inserts at the start of a line.
      const calib = await one('- a\n- b', { line: 1, ch: 3 }, { key: 'Tab' });
      r[key] = {
        config: { tabSize: await cfg('tabSize'), useTab: await cfg('useTab') },
        calibration: { out: calib.out, inserted: calib.out.split('\n')[1].replace(/- b$/, '') },
        shiftTab: {}, tab: {},
      };
      for (const [label, content, caret] of SHAPES) {
        r[key].shiftTab[label] = await one(content, caret, { key: 'Tab', mods: { shift: true } });
        r[key].tab[label] = await one(content, caret, { key: 'Tab' });
      }
    }
  }
  await cfg('tabSize', 4); await cfg('useTab', true);
  // The commands, for comparison with the keystrokes.
  r.unindentCommand = {};
  for (const [label, content, caret] of SHAPES) {
    const spec = { content, cursor: caret };
    await seed(spec);
    await c.evaluate(`app.commands.executeCommandById('editor:unindent-list'); return true;`);
    await sleep(200);
    r.unindentCommand[label] = await read();
  }
  r.configAfter = await c.evaluate(`return { tabSize: app.vault.getConfig('tabSize'), useTab: app.vault.getConfig('useTab') };`);
  r.appJson = fs.readFileSync(vp('.obsidian/app.json'), 'utf8');
  return r;
});

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
