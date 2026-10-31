// Bases (`.base`) format and write-back: what Obsidian writes when it saves a
// base, driven over CDP. Answers the questions phase 4 needs before Silica can
// open a `.base` at all: whether opening writes, whether closing writes, what
// one real edit writes, the key order at every level, the quoting and
// indentation the YAML emitter picks, whether comments survive, trailing
// newline, CRLF and BOM, where a newly added key lands, the unpublished layout
// keys (column width, row height, card size, image property, image fit, aspect
// ratio), what `bases:new-file`, `bases:insert` and `bases:copy-table` produce,
// whether unknown keys survive a UI round trip at every level, what a broken
// base does to its own bytes, and whether a base authored elsewhere is
// rewritten.
//
// Built on two things the reconnaissance runs established (2026-10-25):
// `query.getSerializable()` is the whole writer, and it spreads
// `unrecognizedData` FIRST; and a parsed statement keeps its original text in
// `formula.text` beside the AST, so "the statement changed" and "the emitter
// quoted it differently" are separable.
//
// Step numbers have gaps: s07-s09 drove the toolbar by `.menu-item` label and
// found nothing, because the Bases toolbar opens panels rather than item lists
// (s05 keeps that evidence), and s20-s24 replace them. `s15-final` runs last.
//
// Phases: fixtures | run   (node bases.mjs <phase>)
// ONLY_STEPS=s03,s04 re-runs a step or two (it never filters the fixtures
// phase: the runner makes a fresh vault every time).
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run'].includes(phase)) { console.error('phase must be fixtures|run'); process.exit(2); }

const T0 = Date.now();

// ---- results file ---------------------------------------------------------
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.bases = all.bases || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.bases[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

// ONLY_STEPS names run steps; it must never filter the fixtures phase, or the
// re-run starts on an empty vault and every open misses.
const ONLY = process.env.ONLY_STEPS && phase === 'run' ? process.env.ONLY_STEPS.split(',') : null;
async function step(name, fn) {
  if (ONLY && !ONLY.some((p) => name.startsWith(p))) return null;
  out.order.push(name);
  const t = Date.now();
  const partial = {};
  try { const v = await fn(partial); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e), partial }; }
  save();
  return out.steps[name];
}

// ---- host-side vault reads ------------------------------------------------
const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
const raw = (rel) => { try { return fs.readFileSync(vp(rel)); } catch { return null; } };
const mtime = (rel) => { try { return Math.round(fs.statSync(vp(rel)).mtimeMs); } catch { return null; } };
/** The byte-level questions: BOM, line endings, last byte. */
const shape = (rel) => {
  const b = raw(rel);
  if (!b) return null;
  return {
    size: b.length,
    mtimeMs: mtime(rel),
    bom: b.length >= 3 && b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf,
    crlf: b.includes(Buffer.from('\r\n')),
    lastByte: b.length ? b[b.length - 1] : null,
    endsWithNewline: b.length ? b[b.length - 1] === 0x0a : null,
    head16: b.subarray(0, 16).toString('hex'),
  };
};
const basesOf = () => fs.readdirSync(VAULT).filter((n) => n.endsWith('.base')).sort();
const snapshot = () => Object.fromEntries(basesOf().map((n) => [n, { ...shape(n), text: bytes(n) }]));

/** Wait until `rel`'s bytes differ from `before` (or the timeout passes). */
async function waitForWrite(rel, before, timeoutMs = 12000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    const now = bytes(rel);
    if (now !== before) { await sleep(600); return { changed: true, waitedMs: timeoutMs - (end - Date.now()), text: bytes(rel) }; }
    await sleep(200);
  }
  return { changed: false, waitedMs: timeoutMs, text: bytes(rel) };
}

// ---- fixtures -------------------------------------------------------------
// One base carrying every published top-level key in a scrambled order, both
// YAML quoting styles, comments at three depths, flow collections, 4-space
// indentation, unknown keys at top / view / property level, nested filters, and
// the two keys only the decompiled serializer names (`newItemFolder`,
// `newItemTemplate`). Nothing here is in a form Obsidian would write, so
// whatever comes back is Obsidian's own.
const LONG = 'if(status == "todo", "a rather long string that exists only to see whether the emitter folds a line at eighty columns", "no")';
const SCRAMBLED = `# a leading comment
summaries:
  customAverage: 'values.mean().round(3)'
  plainSummary: values.sum()
properties:
  status:
    displayName: Status
    unknownPropKey: kept
  formula.ppu:
    displayName: Price per unit
  file.ext:
    displayName: Extension
  note.unused: {}
formulas:
  ppu: "(density / silica).toFixed(2)"
  spaced: 'type + " "  + file.name'
  hashish: '"#" + status'
  colonish: 'if(status == "a", "x: y", "z")'
  bracketish: '[1, 2, 3].join(", ")'
  longOne: '${LONG}'
  trailingSpace: 'status + "x" '
filters:
  and:
    - 'status != "done"'
    - file.hasTag("rock")   # a comment after a filter
    - or:
        - 'density > 1'
        - not:
            - 'silica < 50'
views:
  - type: table
    name: My table
    # a comment inside a view
    order: [file.name, status, formula.ppu]
    sort:
      - property: status
        direction: DESC
    groupBy: {property: status, direction: ASC}
    limit: 10
    summaries:
      formula.ppu: Average
    unknownViewKey: kept
    nestedUnknown:
        deep:
            - 1
            - a
  - type: cards
    name: Cards
    order:
        - file.name
        - formula.ppu
newItemFolder: Notes
newItemTemplate: Templates/Rock.md
unknownTopKey:
  nested: 1
`;

// A view with no name, a layout nobody named, and an unknown key inside the
// `filters` map: the three places the first fixture does not reach.
const ODDITIES = `filters:
  and:
    - 'status != "done"'
  weirdFilterKey: 1
views:
  - type: list
  - type: table
    name: ''
    unknownViewKey: kept
`;

// The list and kanban layouts, whose option keys the registration dump leaves
// as minified constants: driving their config pages is the only way to see the
// key each control writes.
const LAYOUTS = `views:
  - type: list
    name: Listing
    order:
      - file.name
      - status
  - type: kanban
    name: Board
    order:
      - file.name
`;

const MINIMAL = `views:
  - type: table
    name: Plain
`;
const CARDS_ONLY = `views:
  - type: cards
    name: Gallery
    order:
      - file.name
`;

const note = (name, fm, body) => [name, `---\n${fm}\n---\n\n${body}\n`];
const NOTES = Object.fromEntries([
  note('Silica.md', 'type: rock\nstatus: todo\ndensity: 1.21\nsilica: 50\ncover: pic.png\ntags:\n  - rock', '# Silica\n\nFrothed basalt.'),
  note('Pumice.md', 'type: rock\nstatus: done\ndensity: 0.64\nsilica: 72\ncover: pic.png\ntags:\n  - rock', '# Pumice\n\nFloats.'),
  note('Basalt.md', 'type: rock\nstatus: todo\ndensity: 2.9\nsilica: 49\ntags:\n  - rock', '# Basalt\n\nDense.'),
  note('Empty.md', 'type: rock', '# Empty\n\nNo status, no density.'),
]);

const FIXTURES = {
  // the format fixture, in five copies with five different fates
  'Untouched.base': SCRAMBLED,   // never opened: does the app rewrite a base it only indexes?
  'OpenClose.base': SCRAMBLED,   // opened and closed, no edit
  'Edited.base': SCRAMBLED,      // opened, one real UI edit
  'Layout.base': SCRAMBLED,      // the layout keys, through the menus
  'External.base': SCRAMBLED,    // rewritten from outside while open
  'Minimal.base': MINIMAL,
  'Layouts.base': LAYOUTS,
  'LayoutChange.base': 'views:\n  - type: list\n    name: Listing\n    order:\n      - file.name\n    markers: number\n    separator: "; "\n',
  'Cards.base': CARDS_ONLY,
  'NoTrailingNewline.base': MINIMAL.replace(/\n$/, ''),
  'Crlf.base': SCRAMBLED.replace(/\n/g, '\r\n'),
  'Bom.base': '﻿' + SCRAMBLED,
  'Zero.base': '',
  'EmptyMap.base': '{}\n',
  'Oddities.base': ODDITIES,
  // the three questions Oddities.base asks at once, split so each answers alone
  'NoName.base': 'views:\n  - type: list\n',
  'EmptyName.base': "views:\n  - type: table\n    name: ''\n",
  'FilterExtra.base': 'filters:\n  and:\n    - \'status != "done"\'\n  weirdFilterKey: 1\nviews:\n  - type: table\n    name: X\n',
  // the broken ones: one failure mode each
  'BrokenYaml.base': 'filters:\n  and:\n    - \'a == 1\'\n  bad: [unclosed\nviews:\n  - type: table\n',
  'BrokenView.base': 'views:\n  - type: nonsense\n    name: Odd\n    order:\n      - file.name\n',
  'BrokenFilter.base': 'filters:\n  and:\n    - \'status !!! "done"\'\nviews:\n  - type: table\n    name: BF\n',
  'BrokenFormula.base': 'formulas:\n  broken: \'foo((((\'\nviews:\n  - type: table\n    name: BFo\n    order:\n      - formula.broken\n',
  'BrokenSummary.base': 'summaries:\n  broken: \'values.\'\nviews:\n  - type: table\n    name: BS\n',
  'Insert.md': '# Insert target\n\nBEFORE\n\nAFTER\n',
  'pic.png': '',
  ...NOTES,
};

if (phase === 'fixtures') {
  await step('fixtures', async () => {
    fs.mkdirSync(vp('.obsidian'), { recursive: true });
    fs.writeFileSync(vp('.obsidian/app.json'), '{}');
    for (const [name, text] of Object.entries(FIXTURES)) fs.writeFileSync(vp(name), text);
    return { written: snapshot(), vaultFiles: fs.readdirSync(VAULT).sort(), scrambledFixture: SCRAMBLED };
  });
  process.exit(0);
}

// ---- run ------------------------------------------------------------------
const c = await connect();
const vault = await c.vaultPath();
if (vault.replace(/\/+$/, '') !== VAULT.replace(/\/+$/, '')) { console.error('wrong vault open: ' + vault); c.close(); process.exit(3); }
out.vault = vault;
out.version = await c.evaluate(`return { title: document.title, ua: navigator.userAgent,
  appVersion: app.appVersion || null, installer: app.installerVersion || null };`);
out.fixtureScrambled = SCRAMBLED;

/** Open a `.base` in a leaf of its own, so no other file's close-write fires.
 * The vault's own index can lag `layoutReady` by a second or two, so wait for
 * the file rather than throwing on the first miss. */
const openBase = async (p) => {
  await c.evaluate(`app.workspace.detachLeavesOfType('bases'); app.workspace.detachLeavesOfType('markdown'); return true;`);
  await sleep(1500);
  const r = await c.evaluate(`
    let f = null;
    for (let i = 0; i < 40 && !f; i++) {
      f = app.vault.getAbstractFileByPath(${JSON.stringify(p)});
      if (!f) await new Promise((r) => setTimeout(r, 500));
    }
    if (!f) throw new Error('missing ' + ${JSON.stringify(p)});
    const leaf = app.workspace.getLeaf('tab');
    await leaf.openFile(f);
    app.workspace.setActiveLeaf(leaf, { focus: true });
    return true;`);
  await sleep(2500);
  return r;
};
const closeActive = async () => { await c.evaluate(`app.workspace.activeLeaf.detach(); return true;`); await sleep(2500); };

/** The live model: what the writer would write, and everything that feeds it. */
const modelDump = () => c.evaluate(`
  const v = app.workspace.activeLeaf && app.workspace.activeLeaf.view;
  if (!v) return { noView: true };
  const safe = (f, d) => { try { return f(); } catch (e) { return 'ERR:' + e.message; } };
  const q = v.query;
  return {
    viewType: safe(() => v.getViewType()),
    file: v.file ? v.file.path : null,
    serializable: safe(() => JSON.parse(JSON.stringify(q.getSerializable()))),
    serializableKeys: safe(() => Object.keys(q.getSerializable())),
    unrecognizedData: safe(() => JSON.parse(JSON.stringify(q.unrecognizedData))),
    unrecognizedKeys: safe(() => Object.keys(q.unrecognizedData || {})),
    toString: safe(() => q.toString()),
    getViewData: safe(() => v.getViewData()),
    lastSavedData: safe(() => v.lastSavedData),
    data: safe(() => v.data),
    dirty: safe(() => v.dirty),
    // statements: the original text beside the parsed AST, which is how
    // "the text changed" and "the emitter quoted it differently" separate.
    formulas: safe(() => Object.fromEntries(Object.entries(q.formulas || {}).map(([k, f]) =>
      [k, { text: f.text, toString: String(f.toString()), ast: JSON.parse(JSON.stringify(f.formula === undefined ? null : f.formula)) }]))),
    summaryFormulas: safe(() => Object.fromEntries(Object.entries(q.summaryFormulas || {}).map(([k, f]) =>
      [k, { text: f && f.text, toString: f ? String(f.toString()) : null }]))),
    filtersJson: safe(() => JSON.stringify(q.filters, (k, val) => typeof val === 'function' ? '[fn]' : val)),
    filtersSerialize: safe(() => JSON.parse(JSON.stringify(q.filters ? q.filters.serialize() : null))),
    properties: safe(() => Object.fromEntries(Object.entries(q.properties || {}).map(([k, p]) =>
      [k, { keys: Object.keys(p), serialize: p.serialize ? JSON.parse(JSON.stringify(p.serialize() === undefined ? null : p.serialize())) : 'no serialize' }]))),
    viewsSerialized: safe(() => (q.views || []).map((vv) => JSON.parse(JSON.stringify(vv.serialize())))),
    viewsOwnKeys: safe(() => (q.views || []).map((vv) => Object.keys(vv))),
    viewsGetAll: safe(() => (q.views || []).map((vv) => { try { return JSON.parse(JSON.stringify(vv.getAll())); } catch (e) { return 'ERR:' + e.message; } })),
    newItemFolder: safe(() => q.newItemFolder === undefined ? '<undefined>' : q.newItemFolder),
    newItemTemplate: safe(() => q.newItemTemplate === undefined ? '<undefined>' : q.newItemTemplate),
    errors: safe(() => JSON.parse(JSON.stringify(v.controller ? v.controller.errors : null))),
    errorDom: safe(() => Array.from(document.querySelectorAll('.bases-error')).map((e) => ({ text: e.textContent, html: e.outerHTML.slice(0, 600) }))),
    rowCount: safe(() => document.querySelectorAll('.bases-table-container .bases-tr').length),
    viewName: safe(() => v.controller ? v.controller.viewName : null),
  };`);

// 01 — the plugin, the commands, the registrations. The registration sources
// name the config keys each layout reads, which is cheaper and more exact than
// hunting them through the menus.
await step('s01-registrations', async () => c.evaluate(`
  const inst = app.internalPlugins.plugins.bases.instance;
  // A function's name and arity, never its body: Obsidian's own source must
  // not land in this repository (clean room, see CLAUDE.md).
  const src = (f) => { try { return typeof f === 'function' ? 'fn ' + (f.name || '(anonymous)') + '/' + f.length : String(f); } catch (e) { return 'ERR:' + e.message; } };
  const regs = inst.getRegistrations ? inst.getRegistrations() : null;
  const dumpReg = (r) => {
    const o = { keys: Object.keys(r) };
    for (const k of Object.keys(r)) {
      const val = r[k];
      o[k] = typeof val === 'function' ? src(val).slice(0, 4000) : (typeof val === 'object' && val ? JSON.stringify(val).slice(0, 2000) : val);
    }
    return o;
  };
  let registrations = null;
  if (regs) {
    if (regs instanceof Map) registrations = Object.fromEntries(Array.from(regs.entries()).map(([k, r]) => [k, dumpReg(r)]));
    else if (Array.isArray(regs)) registrations = regs.map(dumpReg);
    else registrations = Object.fromEntries(Object.entries(regs).map(([k, r]) => [k, dumpReg(r)]));
  }
  return {
    registrations,
    registrationsRaw: (() => { try { return JSON.stringify(inst.registrations, (k, v) => typeof v === 'function' ? '[fn]' : v).slice(0, 4000); } catch (e) { return 'ERR:' + e.message; } })(),
    commands: Object.keys(app.commands.commands).filter((id) => id.startsWith('bases')).map((id) => ({ id, name: app.commands.commands[id].name })),
    createNewBasesFileSrc: src(Object.getPrototypeOf(inst).createNewBasesFile),
    createAndOpenBaseSrc: src(Object.getPrototypeOf(inst).createAndOpenBase),
    createAndEmbedBaseSrc: src(Object.getPrototypeOf(inst).createAndEmbedBase),
  };`));

// 02 — does opening write? And what does the writer hold the moment it opens?
await step('s02-open-no-write', async (p) => {
  p.before = { ...shape('OpenClose.base'), text: bytes('OpenClose.base') };
  await openBase('OpenClose.base');
  p.afterOpenImmediate = { ...shape('OpenClose.base'), text: bytes('OpenClose.base') };
  await sleep(6000);          // longer than any debounce we have seen
  p.afterOpen6s = { ...shape('OpenClose.base'), text: bytes('OpenClose.base') };
  p.rewrittenOnOpen = p.afterOpen6s.text !== p.before.text;
  p.model = await modelDump();
  return p;
});

// 03 — the serializer and every part that feeds it, as source. This is the
// writer: Silica has to reproduce it, so it is recorded verbatim.
await step('s03-writer-source', async () => c.evaluate(`
  const v = app.workspace.activeLeaf.view; const q = v.query;
  // A function's name and arity, never its body: Obsidian's own source must
  // not land in this repository (clean room, see CLAUDE.md).
  const src = (f) => { try { return typeof f === 'function' ? 'fn ' + (f.name || '(anonymous)') + '/' + f.length : String(f); } catch (e) { return 'ERR:' + e.message; } };
  const protoSrc = (o, cap = 6000) => {
    if (!o) return null;
    const pr = Object.getPrototypeOf(o); if (!pr) return null;
    const r = {};
    for (const k of Object.getOwnPropertyNames(pr)) { try { if (typeof pr[k] === 'function') r[k] = src(pr[k]).slice(0, cap); } catch { r[k] = 'ERR'; } }
    return r;
  };
  return {
    queryProto: protoSrc(q),
    viewConfigProto: q.views && q.views[0] ? protoSrc(q.views[0]) : null,
    filtersProto: protoSrc(q.filters),
    propertyProto: q.properties && Object.keys(q.properties).length ? protoSrc(q.properties[Object.keys(q.properties)[0]]) : null,
    basesViewProto: v.controller && v.controller.view ? protoSrc(v.controller.view, 20000) : null,
    basesViewCtor: v.controller && v.controller.view ? (v.controller.view.constructor && v.controller.view.constructor.name) : null,
    basesViewOwnKeys: v.controller && v.controller.view ? Object.keys(v.controller.view) : null,
    requestSaveSrc: src(v.requestSave),
    saveQuerySrc: src(Object.getPrototypeOf(v).saveQuery),
    setViewDataSrc: src(Object.getPrototypeOf(v).setViewData),
    getViewDataSrc: src(Object.getPrototypeOf(v).getViewData),
  };`));

// 04 — does closing write, with nothing edited? This is the asymmetry canvas
// has, and the one that decides whether Silica may ever write on close.
await step('s04-close-writes', async (p) => {
  p.before = { ...shape('OpenClose.base'), text: bytes('OpenClose.base') };
  await closeActive();
  const w = await waitForWrite('OpenClose.base', p.before.text, 10000);
  p.changed = w.changed;
  p.after = { ...shape('OpenClose.base'), text: bytes('OpenClose.base') };
  p.sameAsFixture = p.after.text === SCRAMBLED;
  p.untouchedStillOriginal = bytes('Untouched.base') === SCRAMBLED;
  return p;
});

// 05 — the layout keys, by driving the real toolbar. Every menu is dumped
// first (rows and markup), because the published docs say these key names are
// unpublished and the menus are the only place they are named to a user.
const dismissMenus = () => c.evaluate(`document.querySelectorAll('.menu').forEach(m => m.remove()); return true;`);
const menuDump = () => c.evaluate(`
  const menus = Array.from(document.querySelectorAll('.menu'));
  return menus.map((menu) => ({
    rows: Array.from(menu.querySelectorAll('.menu-item, .menu-separator')).map((el) => ({
      sep: el.classList.contains('menu-separator'),
      h: el.offsetHeight,
      section: el.getAttribute('data-section'),
      title: (el.querySelector('.menu-item-title') || {}).textContent || null,
      checked: el.classList.contains('mod-checked') || null,
      classes: el.className,
    })),
    html: menu.outerHTML.slice(0, 20000),
  }));`);
/** Click a `.menu-item` whose title matches, in any open menu. */
const clickMenuRow = async (title) => {
  const r = await c.evaluate(`
    const rows = Array.from(document.querySelectorAll('.menu .menu-item'));
    const el = rows.find((e) => ((e.querySelector('.menu-item-title') || {}).textContent || '').trim() === ${JSON.stringify(title)});
    if (!el) return null;
    const b = el.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  if (!r) return { clicked: false, reason: 'no row ' + title };
  await c.clickAt(r.x, r.y);
  await sleep(700);
  return { clicked: true, at: r };
};
const hoverMenuRow = async (title) => {
  const r = await c.evaluate(`
    const rows = Array.from(document.querySelectorAll('.menu .menu-item'));
    const el = rows.find((e) => ((e.querySelector('.menu-item-title') || {}).textContent || '').trim() === ${JSON.stringify(title)});
    if (!el) return null;
    const b = el.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  if (!r) return { hovered: false };
  await c.mouseMove(r.x, r.y);
  await sleep(700);
  return { hovered: true };
};

await step('s05-toolbar-menus', async (p) => {
  await openBase('Layout.base');
  p.toolbar = await c.evaluate(`
    return Array.from(document.querySelectorAll('.bases-toolbar-item')).map((e) => ({
      classes: e.className, text: e.textContent, aria: e.getAttribute('aria-label'),
      icon: (e.querySelector('svg') || {}).className ? String((e.querySelector('svg') || {}).getAttribute('class')) : null,
      html: e.outerHTML.slice(0, 1200) }));`);
  p.menus = {};
  const items = ['bases-toolbar-views-menu', 'bases-toolbar-results-menu', 'bases-toolbar-sort-menu',
    'bases-toolbar-group-menu', 'bases-toolbar-filter-menu', 'bases-toolbar-properties-menu',
    'bases-toolbar-new-item-menu'];
  for (const cls of items) {
    await dismissMenus();
    const r = await c.clickElement('.' + cls);
    await sleep(900);
    p.menus[cls] = { rect: r, menus: await menuDump() };
    save();
  }
  await dismissMenus();
  return p;
});

// 06 — a column resize: a real drag on `.bases-table-header-resizer`. The first
// real edit, so it also answers "what does one edit write".
await step('s06-column-resize', async (p) => {
  p.before = { ...shape('Layout.base'), text: bytes('Layout.base') };
  p.headers = await c.evaluate(`
    return Array.from(document.querySelectorAll('.bases-table-header')).map((e) => {
      const b = e.getBoundingClientRect();
      return { text: e.textContent, w: b.width, x: b.x, y: b.y, html: e.outerHTML.slice(0, 600) }; });`);
  const rz = await c.evaluate(`
    const els = Array.from(document.querySelectorAll('.bases-table-header-resizer'));
    const el = els[0]; if (!el) return null;
    const b = el.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2, count: els.length, html: el.outerHTML.slice(0, 300) };`);
  p.resizer = rz;
  if (!rz) return { ...p, note: 'no .bases-table-header-resizer' };
  await c.drag(rz.x, rz.y, rz.x + 140, rz.y, { steps: 14, pauseMs: 40 });
  await sleep(1200);
  const w = await waitForWrite('Layout.base', p.before.text, 12000);
  p.wrote = w.changed;
  p.after = { ...shape('Layout.base'), text: bytes('Layout.base') };
  p.model = await modelDump();
  p.headersAfter = await c.evaluate(`
    return Array.from(document.querySelectorAll('.bases-table-header')).map((e) => ({ text: e.textContent, w: e.getBoundingClientRect().width }));`);
  return p;
});

// 10 — the edited save in full: open Edited.base, make one edit through the
// model (the API path a command takes), and read the bytes.
await step('s10-edited-save', async (p) => {
  await closeActive();                 // close Layout.base
  await sleep(1500);
  p.layoutFinal = { ...shape('Layout.base'), text: bytes('Layout.base') };
  await openBase('Edited.base');
  p.before = { ...shape('Edited.base'), text: bytes('Edited.base') };
  p.setters = await c.evaluate(`
    const q = app.workspace.activeLeaf.view.query;
    const out = {};
    const t = (k, f) => { try { out[k] = { ok: true, value: f() }; } catch (e) { out[k] = { ok: false, error: e.message }; } };
    // the second view has no limit: setting one shows where a new key lands
    t('setLimit', () => { q.views[1].setLimit(5); return 'called'; });
    t('serializedAfter', () => JSON.parse(JSON.stringify(q.getSerializable())));
    return out;`);
  const w = await waitForWrite('Edited.base', p.before.text, 10000);
  p.wrote = w.changed;
  p.after = { ...shape('Edited.base'), text: bytes('Edited.base') };
  p.model = await modelDump();
  await closeActive();
  await sleep(2000);
  p.afterClose = { ...shape('Edited.base'), text: bytes('Edited.base') };
  return p;
});

// 11 — CRLF, BOM, no trailing newline, zero bytes, `{}`: open and close each
// and read the bytes back.
await step('s11-bytes', async (p) => {
  p.cases = {};
  for (const f of ['Crlf.base', 'Bom.base', 'NoTrailingNewline.base', 'Zero.base', 'EmptyMap.base', 'Minimal.base', 'Oddities.base']) {
    const before = { ...shape(f), text: bytes(f) };
    await openBase(f);
    const model = await modelDump();
    const afterOpen = { ...shape(f), text: bytes(f) };
    await closeActive();
    await sleep(1500);
    const afterClose = { ...shape(f), text: bytes(f) };
    p.cases[f] = { before, afterOpen, afterClose, rewrittenOnOpen: afterOpen.text !== before.text,
      rewrittenOnClose: afterClose.text !== afterOpen.text, model };
    save();
  }
  return p;
});

// 12 — the broken ones. A base that cannot be parsed, a view type nobody
// registered, and a statement the expression parser rejects, in a filter, a
// formula and a summary. The question that matters: are the bytes touched?
await step('s12-broken', async (p) => {
  p.cases = {};
  for (const f of ['BrokenYaml.base', 'BrokenView.base', 'BrokenFilter.base', 'BrokenFormula.base', 'BrokenSummary.base']) {
    const before = { ...shape(f), text: bytes(f) };
    let openErr = null;
    try { await openBase(f); } catch (e) { openErr = e.message; }
    const model = await modelDump();
    const notices = await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(e => e.textContent);`);
    const afterOpen = { ...shape(f), text: bytes(f) };
    await closeActive();
    await sleep(2000);
    const afterClose = { ...shape(f), text: bytes(f) };
    p.cases[f] = { before, openErr, notices, afterOpen, afterClose,
      rewrittenOnOpen: afterOpen.text !== before.text, rewrittenOnClose: afterClose.text !== afterOpen.text, model };
    save();
  }
  return p;
});

// 13 — the three commands: new-file, insert, copy-table.
await step('s13-commands', async (p) => {
  // new-file
  const was = basesOf();
  p.newFileRan = await c.evaluate(`return app.commands.executeCommandById('bases:new-file');`);
  await sleep(3000);
  p.created = basesOf().filter((n) => !was.includes(n));
  p.createdShape = p.created[0] ? { ...shape(p.created[0]), text: bytes(p.created[0]) } : null;
  p.afterNewFileView = await modelDump();
  p.modalAfterNewFile = await c.evaluate(`const m = document.querySelector('.modal-container'); return m ? m.outerHTML.slice(0, 4000) : null;`);
  // with a new base open and empty, does adding a view write it?
  p.addViewRan = await c.evaluate(`return app.commands.executeCommandById('bases:add-view');`);
  await sleep(1200);
  p.addViewMenus = await menuDump();
  p.addViewPanel = await c.evaluate(`
    const m = document.querySelector('.menu');
    return m ? m.outerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 20000) : null;`);
  p.addViewModal = await c.evaluate(`const m = document.querySelector('.modal-container'); return m ? m.outerHTML.slice(0, 8000) : null;`);
  if (p.created[0]) {
    const w = await waitForWrite(p.created[0], p.createdShape ? p.createdShape.text : null, 8000);
    p.afterAddView = p.created[0] ? { ...shape(p.created[0]), text: bytes(p.created[0]) } : null;
    p.addViewWrote = w.changed;
  }
  await dismissMenus(); await c.pressKey('Escape'); await sleep(400);
  // copy-table, with a real base view active
  await closeActive(); await sleep(800);
  await openBase('Layout.base');
  await c.writeClipboard('SENTINEL-BEFORE-COPY');
  p.copyTableListed = await c.evaluate(`
    const cmd = app.commands.commands['bases:copy-table'];
    // Which callbacks a command has, never their source: Obsidian's own code
    // must not land in this repository (clean room, see CLAUDE.md).
    return { name: cmd.name, hasCheckCallback: !!cmd.checkCallback, hasEditorCallback: !!cmd.editorCallback,
      hasCallback: !!cmd.callback, hasEditorCheckCallback: !!cmd.editorCheckCallback };`);
  p.copyTableRan = await c.evaluate(`return app.commands.executeCommandById('bases:copy-table');`);
  await sleep(4000);
  p.copyTableNotices = await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(e => e.textContent);`);
  p.copyTableModal = await c.evaluate(`const m = document.querySelector('.modal-container'); return m ? m.outerHTML.slice(0, 4000) : null;`);
  p.clipboardAfterCopy = await c.readClipboard();
  // insert, into a markdown note
  await closeActive(); await sleep(800);
  const insertBefore = bytes('Insert.md');
  p.insertSetup = await c.evaluate(`
    const f = app.vault.getAbstractFileByPath('Insert.md');
    const leaf = app.workspace.getLeaf('tab');
    await leaf.openFile(f);
    app.workspace.setActiveLeaf(leaf, { focus: true });
    return true;`);
  await sleep(1800);
  await c.evaluate(`
    const ed = app.workspace.activeEditor && app.workspace.activeEditor.editor;
    if (ed) ed.setCursor({ line: 2, ch: 6 });
    return ed ? ed.getValue().length : null;`);
  await sleep(400);
  p.insertRan = await c.evaluate(`return app.commands.executeCommandById('bases:insert');`);
  await sleep(1800);
  p.insertModal = await c.evaluate(`const m = document.querySelector('.modal-container'); return m ? m.outerHTML.slice(0, 12000) : null;`);
  p.insertMenus = await menuDump();
  p.insertEditorValue = await c.evaluate(`
    const ed = app.workspace.activeEditor && app.workspace.activeEditor.editor;
    return ed ? ed.getValue() : null;`);
  const iw = await waitForWrite('Insert.md', insertBefore, 6000);
  p.insertWroteFile = iw.changed;
  p.insertFileAfter = bytes('Insert.md');
  p.basesAfterInsert = basesOf();
  await dismissMenus(); await c.pressKey('Escape'); await sleep(500);
  return p;
});

// 14 — a base rewritten from outside while it is open: does the view reload,
// and does Obsidian write over the external bytes?
await step('s14-external-rewrite', async (p) => {
  await closeActive(); await sleep(800);
  await openBase('External.base');
  p.opened = await c.evaluate(`const v = app.workspace.activeLeaf.view; return { file: v.file.path, name: v.controller ? v.controller.viewName : null };`);
  const external = SCRAMBLED.replace('name: My table', 'name: Renamed from outside').replace('limit: 10', 'limit: 3');
  fs.writeFileSync(vp('External.base'), external);
  p.wrotenExternally = true;
  await sleep(4000);
  p.modelAfterExternal = await modelDump();
  p.fileAfterExternal = { ...shape('External.base'), text: bytes('External.base') };
  p.reloaded = p.modelAfterExternal.serializable && JSON.stringify(p.modelAfterExternal.serializable).includes('Renamed from outside');
  await closeActive();
  await sleep(2500);
  p.afterClose = { ...shape('External.base'), text: bytes('External.base') };
  p.closeClobberedExternal = p.afterClose.text !== external;
  p.externalText = external;
  p.externalNameSurvived = !!(p.afterClose.text && p.afterClose.text.includes('Renamed from outside'));
  p.externalLimitSurvived = !!(p.afterClose.text && /limit: 3\b/.test(p.afterClose.text));
  return p;
});

// 20 — the toolbar panels in full. 1.14's Bases toolbar does not open
// `.menu-item` lists: each control opens a `.menu.bases-toolbar-menu` holding
// `.bases-toolbar-section`s of comboboxes, toggles and inputs (s05 read them as
// empty menus, which is how this was found). The layout keys the docs leave
// unpublished are named in these panels, so they are dumped whole.
const panelRead = () => c.evaluate(`
  const txt = (e) => (e && e.textContent || '').trim();
  return Array.from(document.querySelectorAll('.menu')).map((menu) => ({
    classes: menu.className,
    sections: Array.from(menu.querySelectorAll('.bases-toolbar-section')).map((s) => ({
      classes: s.className,
      header: txt(s.querySelector('.bases-toolbar-section-header')),
      controls: Array.from(s.querySelectorAll('.combobox-button, .checkbox-container, input, select, .clickable-icon, .menu-item, .slider')).map((e) => ({
        tag: e.tagName, classes: e.className,
        label: txt(e.querySelector('.combobox-button-label')) || txt(e.querySelector('.menu-item-title')) || null,
        placeholder: e.getAttribute('placeholder') || (e.querySelector('[placeholder]') ? e.querySelector('[placeholder]').getAttribute('placeholder') : null),
        value: e.value === undefined ? null : e.value,
        checked: e.classList.contains('is-enabled') || e.checked || null,
        aria: e.getAttribute('aria-label'),
        icon: e.querySelector('svg') ? e.querySelector('svg').getAttribute('class') : null,
      })),
    })),
    rows: Array.from(menu.querySelectorAll('.menu-item')).map((e) => ({ title: txt(e.querySelector('.menu-item-title')) || txt(e), section: e.getAttribute('data-section'), checked: e.classList.contains('mod-checked') || null })),
    html: menu.outerHTML.slice(0, 60000),
  }));`);

await step('s20-panels-table', async (p) => {
  await openBase('Layout.base');
  p.viewName = await c.evaluate(`const v = app.workspace.activeLeaf.view; return v.controller ? v.controller.viewName : null;`);
  p.panels = {};
  for (const cls of ['bases-toolbar-views-menu', 'bases-toolbar-results-menu', 'bases-toolbar-sort-menu',
    'bases-toolbar-group-menu', 'bases-toolbar-filter-menu', 'bases-toolbar-properties-menu',
    'bases-toolbar-new-item-menu']) {
    await dismissMenus(); await c.pressKey('Escape'); await sleep(300);
    const r = await c.clickElement('.' + cls);
    await sleep(1000);
    p.panels[cls] = { rect: r, panels: await panelRead() };
    save();
  }
  await dismissMenus();
  // the column header's own context menu, and the view header's more-options
  p.headerContext = await (async () => {
    await c.rightClickElement('.bases-table-header');
    await sleep(900);
    const m = await menuDump();
    await dismissMenus();
    return m;
  })();
  p.moreOptions = await (async () => {
    const r = await c.clickElement('.view-header .view-actions .clickable-icon', { index: 0 });
    await sleep(900);
    const m = await menuDump();
    await dismissMenus();
    return { rect: r, menus: m };
  })();
  p.viewHeaderActions = await c.evaluate(`
    return Array.from(document.querySelectorAll('.view-header .view-actions > *')).map(e => ({ classes: e.className, aria: e.getAttribute('aria-label'), icon: e.querySelector('svg') ? e.querySelector('svg').getAttribute('class') : null }));`);
  return p;
});

// 21 — the same panels with a cards view active, where card size, image
// property, image fit and aspect ratio should live.
await step('s21-panels-cards', async (p) => {
  await dismissMenus();
  p.switched = await c.evaluate(`
    const v = app.workspace.activeLeaf.view;
    const names = v.query.views.map(x => x.name);
    // the views panel's own list is the user path; this is the model path, so
    // the panel dump below is of the cards layout either way.
    const ctl = v.controller;
    let how = null;
    try { ctl.setViewName ? (ctl.setViewName('Cards'), how = 'setViewName') : null; } catch (e) { how = 'ERR:' + e.message; }
    return { names, how, viewName: ctl.viewName };`);
  if (!p.switched.how) {
    await c.clickElement('.bases-toolbar-views-menu'); await sleep(900);
    p.viewsPanel = await panelRead();
    p.clickCards = await c.evaluate(`
      const els = Array.from(document.querySelectorAll('.menu .bases-view-item, .menu .menu-item, .menu .bases-toolbar-item'));
      const el = els.find(e => (e.textContent || '').trim().startsWith('Cards'));
      if (!el) return null; const b = el.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2, text: el.textContent.trim(), classes: el.className };`);
    if (p.clickCards) { await c.clickAt(p.clickCards.x, p.clickCards.y); await sleep(2000); }
  }
  await dismissMenus(); await sleep(1200);
  p.viewNameNow = await c.evaluate(`const v = app.workspace.activeLeaf.view; return v.controller ? v.controller.viewName : null;`);
  p.dom = await c.evaluate(`return Array.from(document.querySelectorAll('[class*="bases-card"]')).slice(0, 25).map(e => e.className);`);
  p.panels = {};
  for (const cls of ['bases-toolbar-views-menu', 'bases-toolbar-properties-menu']) {
    await dismissMenus(); await c.pressKey('Escape'); await sleep(300);
    const r = await c.clickElement('.' + cls);
    await sleep(1000);
    p.panels[cls] = { rect: r, panels: await panelRead() };
    save();
  }
  await dismissMenus();
  p.file = { ...shape('Layout.base'), text: bytes('Layout.base') };
  p.model = await modelDump();
  return p;
});

// 22 — the per-view configuration page, which is where the unpublished layout
// keys live: the views panel lists one row per view, and the row's own icon
// (`.bases-toolbar-menu-item-icon`) opens "Configure view". Each control is
// found by its row label and operated the way its widget wants (a native
// `<select>` set with a change event, a range slider clicked three quarters
// along its track, a combobox's option clicked in the `.suggestion-container`
// that opens outside the menu), with the file read in between, so the key each
// one writes is read off disk rather than guessed from the label.
const openViewConfig = async (viewName) => {
  for (let attempt = 0; attempt < 3; attempt++) {
    await dismissMenus(); await c.pressKey('Escape'); await sleep(400);
    await c.clickElement('.bases-toolbar-views-menu');
    await sleep(1000);
    const r = await c.evaluate(`
      const rows = Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-item'));
      const row = rows.find((e) => ((e.querySelector('.bases-toolbar-menu-item-name') || {}).textContent || '').trim() === ${JSON.stringify(viewName)});
      if (!row) return { found: false, rows: rows.map(e => ((e.querySelector('.bases-toolbar-menu-item-name') || {}).textContent || '').trim()) };
      const icon = row.querySelector('.bases-toolbar-menu-item-icon');
      const b = (icon || row).getBoundingClientRect();
      return { found: true, viaIcon: !!icon, x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
    if (!r.found) return r;
    await c.clickAt(r.x, r.y);
    // the page swaps in place: wait for the form rather than a fixed sleep
    for (let i = 0; i < 20; i++) {
      const ready = await c.evaluate(`return !!document.querySelector('.view-config-menu');`);
      if (ready) return { ...r, ready: true, attempt };
      await sleep(250);
    }
  }
  return { found: true, ready: false };
};
/** Every row of the open "Configure view" form, by its label. */
const configRows = () => c.evaluate(`
  const txt = (e) => (e && e.textContent || '').trim();
  const form = document.querySelector('.view-config-menu');
  if (!form) return null;
  return Array.from(form.querySelectorAll('.input-row')).map((row, i) => {
    const content = row.querySelector('.input-row-content') || row;
    const el = content.querySelector('select:not(.is-measuring), input, .combobox-button');
    const b = el ? el.getBoundingClientRect() : null;
    return { i, label: txt(row.querySelector('.input-row-label')) || null,
      widget: el ? (el.tagName === 'INPUT' ? 'input:' + (el.getAttribute('type') || 'text') : el.tagName.toLowerCase()) : null,
      classes: el ? el.className : null,
      placeholder: el ? el.getAttribute('placeholder') : null,
      value: el && el.value !== undefined ? el.value : null,
      comboLabel: txt(content.querySelector('.combobox-button-label')) || null,
      sliderValue: txt(content.querySelector('.slider-value')) || null,
      min: el ? el.getAttribute('min') : null, max: el ? el.getAttribute('max') : null, step: el ? el.getAttribute('step') : null,
      options: el && el.tagName === 'SELECT' ? Array.from(el.options).map(o => ({ value: o.value, label: txt(o) })) : null,
      x: b ? b.x : null, y: b ? b.y + b.height / 2 : null, w: b ? b.width : null, cx: b ? b.x + b.width / 2 : null };
  });`);

await step('s22-view-config', async (p) => {
  p.views = {};
  for (const [file, viewName] of [['Layout.base', 'My table'], ['Layout.base', 'Cards'],
    ['Layouts.base', 'Listing'], ['Layouts.base', 'Board']]) {
    if (!p.openFile || p.openFile !== file) { await openBase(file); p.openFile = file; }
    const target = file;
    const rec = { opened: await openViewConfig(viewName) };
    rec.page = await c.evaluate(`
      const m = document.querySelector('.menu');
      return m ? m.outerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 40000) : null;`);
    rec.rows = await configRows();
    rec.layoutProto = await c.evaluate(`
      const v = app.workspace.activeLeaf.view;
      const o = v.controller && v.controller.view; if (!o) return null;
      const pr = Object.getPrototypeOf(o); const r = {};
      for (const k of Object.getOwnPropertyNames(pr)) { try { if (typeof pr[k] === 'function') r[k] = String(pr[k]).slice(0, 8000); } catch { r[k] = 'ERR'; } }
      return { ctor: o.constructor && o.constructor.name, own: Object.keys(o), proto: r };`);
    rec.tries = [];
    for (const row of (rec.rows || [])) {
      if (!row.widget) continue;
      // Picking in the Layout combobox rewrites `type` and turns the view into
      // another layout, which destroys every control after it; it is recorded
      // once on its own below instead.
      if (row.label === 'Layout') { rec.tries.push({ label: row.label, widget: row.widget, comboBefore: row.comboLabel, skipped: 'changing the layout rewrites type and replaces the controls' }); continue; }
      const before = bytes(target);
      const re = await openViewConfig(viewName);
      const rows = await configRows();
      const r = (rows || []).find((x) => x.label === row.label && x.widget === row.widget);
      const t = { label: row.label, widget: row.widget, reopened: re.ready, before: row.value, sliderBefore: row.sliderValue, comboBefore: row.comboLabel, options: row.options, min: row.min, max: row.max, step: row.step };
      if (!r) { t.gone = true; rec.tries.push(t); save(); continue; }
      if (r.widget === 'select') {
        const pick = (r.options || []).find((o) => o.value !== r.value);
        t.picked = pick ? pick.value : null;
        if (pick) t.setTo = await c.selectSet('.view-config-menu select:not(.is-measuring)', pick.value);
      } else if (r.widget === 'input:range') {
        // a real click three quarters along the track
        await c.clickAt(r.x + r.w * 0.75, r.y);
        await sleep(600);
        t.sliderAfter = await c.evaluate(`const e = document.querySelector('.view-config-menu .slider-value'); return e ? e.textContent : null;`);
      } else if (r.widget === 'combobox-button' || r.classes && String(r.classes).includes('combobox-button')) {
        await c.clickAt(r.cx, r.y);
        await sleep(900);
        t.opened = await c.evaluate(`
          const txt = (e) => (e && e.textContent || '').trim();
          return Array.from(document.querySelectorAll('.suggestion-container')).map(m => ({ classes: m.className,
            items: Array.from(m.querySelectorAll('.suggestion-item')).map(e => ({ text: txt(e), classes: e.className, selected: e.classList.contains('is-selected') || null })) }));`);
        t.pickedItem = await c.evaluate(`
          const items = Array.from(document.querySelectorAll('.suggestion-container .suggestion-item'));
          const el = items.find(e => !e.classList.contains('is-selected'));
          if (!el) return null; const b = el.getBoundingClientRect();
          return { text: (el.textContent || '').trim(), x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
        if (t.pickedItem) { await c.clickAt(t.pickedItem.x, t.pickedItem.y); await sleep(1200); }
      } else if (r.widget === 'input:text' || r.widget === 'input:search') {
        await c.clickAt(r.cx, r.y); await sleep(300);
        await c.typeText('Probed', 40);
        await c.pressKey('Enter'); await sleep(800);
      }
      // close the panel: a blur can be what commits
      await dismissMenus(); await c.pressKey('Escape'); await sleep(600);
      const w = await waitForWrite(target, before, 7000);
      t.wrote = w.changed;
      t.text = w.changed ? w.text : null;
      t.serializable = await c.evaluate(`
        const v = app.workspace.activeLeaf.view;
        try { return JSON.parse(JSON.stringify(v.query.getSerializable())); } catch (e) { return 'ERR:' + e.message; }`);
      rec.tries.push(t); save();
    }
    rec.fileAfter = { ...shape(target), text: bytes(target) };
    p.views[file + ' / ' + viewName] = rec;
    save();
    // leave that view active, so the next layout's own DOM is the one on screen
    await dismissMenus(); await c.pressKey('Escape'); await sleep(300);
    await c.clickElement('.bases-toolbar-views-menu'); await sleep(1000);
    const sw = await c.evaluate(`
      const rows = Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-item'));
      const row = rows.find((e) => ((e.querySelector('.bases-toolbar-menu-item-name') || {}).textContent || '').trim() === ${JSON.stringify(viewName)});
      if (!row) return null; const info = row.querySelector('.bases-toolbar-menu-item-info') || row;
      const b = info.getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
    if (sw) { await c.clickAt(sw.x, sw.y); await sleep(2500); }
    await dismissMenus();
  }
  p.model = await modelDump();
  return p;
});

// 22b — the Layout combobox, deliberately, on a file of its own: changing a
// view's layout rewrites `type` and leaves the previous layout's keys behind.
// It cannot share Layout.base with s22, because s22 ends with another view
// active and the config page's rects move under it.
await step('s22b-layout-change', async (p) => {
  await openBase('LayoutChange.base');
  p.before = { ...shape('LayoutChange.base'), text: bytes('LayoutChange.base') };
  for (let attempt = 0; attempt < 3 && !(p.options && p.options.length); attempt++) {
    p.opened = await openViewConfig('Listing');
    const rows = await configRows();
    p.rows = rows;
    const row = (rows || []).find((x) => x.label === 'Layout');
    p.row = row || null;
    if (!row) continue;
    await c.clickAt(row.cx, row.y);
    await sleep(1200);
    p.options = await c.evaluate(`
      return Array.from(document.querySelectorAll('.suggestion-container .suggestion-item')).map((e) => ({ text: (e.textContent || '').trim(), selected: e.classList.contains('is-selected') || null }));`);
  }
  p.picked = await c.evaluate(`
    const items = Array.from(document.querySelectorAll('.suggestion-container .suggestion-item'));
    const el = items.find((e) => (e.textContent || '').trim() === 'Kanban');
    if (!el) return null; const b = el.getBoundingClientRect();
    return { text: 'Kanban', x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  if (p.picked) { await c.clickAt(p.picked.x, p.picked.y); await sleep(1500); }
  await dismissMenus(); await c.pressKey('Escape'); await sleep(600);
  const w = await waitForWrite('LayoutChange.base', p.before.text, 8000);
  p.wrote = w.changed;
  p.after = { ...shape('LayoutChange.base'), text: bytes('LayoutChange.base') };
  return p;
});

// 23 — the column header's own menu applied (Hide column, Summarize..., Sort,
// Edit property...), and the results panel's limit, Copy to clipboard and
// Export CSV rows.
await step('s23-header-and-results', async (p) => {
  // s22 leaves the cards view active and a cards view has no column header, so
  // open the base again and make sure the table view is the one on screen.
  await openBase('Layout.base');
  await dismissMenus(); await c.pressKey('Escape'); await sleep(300);
  await c.clickElement('.bases-toolbar-views-menu'); await sleep(1000);
  const toTable = await c.evaluate(`
    const rows = Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-item'));
    const row = rows.find((e) => ((e.querySelector('.bases-toolbar-menu-item-name') || {}).textContent || '').trim() === 'My table');
    if (!row) return null; const info = row.querySelector('.bases-toolbar-menu-item-info') || row;
    const b = info.getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  if (toTable) { await c.clickAt(toTable.x, toTable.y); await sleep(2500); }
  await dismissMenus();
  p.viewName = await c.evaluate(`const v = app.workspace.activeLeaf.view; return v.controller ? v.controller.viewName : null;`);
  p.hasHeader = await c.evaluate(`return document.querySelectorAll('.bases-table-header').length;`);
  p.steps = [];
  const doMenu = async (label, title, pick) => {
    const before = bytes('Layout.base');
    await dismissMenus(); await c.pressKey('Escape'); await sleep(300);
    const rc = await c.rightClickElement('.bases-table-header');
    await sleep(1000);
    const rows = await menuDump();
    const cl = await clickMenuRow(title);
    await sleep(1200);
    const sub = await menuDump();
    let picked = null;
    if (pick) { picked = await clickMenuRow(pick); await sleep(1000); }
    await dismissMenus(); await c.pressKey('Escape'); await sleep(400);
    const w = await waitForWrite('Layout.base', before, 7000);
    const rec = { label, title, rightClick: rc, rows: rows.map((m) => m.rows.map((r) => r.title || (r.sep ? '---' : '?'))), menuHtml: rows.map((m) => m.html.slice(0, 6000)), clicked: cl, sub: sub.map((m) => m.rows.map((r) => r.title || (r.sep ? '---' : '?'))), picked, wrote: w.changed, text: w.changed ? w.text : null };
    p.steps.push(rec); save();
    return rec;
  };
  await doMenu('sort-za', 'Sort Z → A');
  await doMenu('summarize', 'Summarize...', 'Average');
  await doMenu('hide-column', 'Hide column');
  // the results panel: the limit input, Copy to clipboard, Export CSV
  const panelControls = () => c.evaluate(`
    const txt = (e) => (e && e.textContent || '').trim();
    return Array.from(document.querySelectorAll('.menu input, .menu select, .menu .bases-toolbar-menu-action')).map((e) => {
      const b = e.getBoundingClientRect();
      const row = e.closest('.input-row') || e.parentElement;
      return { tag: e.tagName, type: e.getAttribute('type'), classes: e.className,
        rowLabel: txt(row && row.querySelector('.input-row-label')) || txt(e.querySelector('.bases-toolbar-menu-item-name')) || null,
        placeholder: e.getAttribute('placeholder'), value: e.value === undefined ? null : e.value,
        x: b.x + b.width / 2, y: b.y + b.height / 2 };
    });`);
  const before = bytes('Layout.base');
  await dismissMenus();
  await c.clickElement('.bases-toolbar-results-menu'); await sleep(1000);
  p.resultsControls = await panelControls();
  const num = (p.resultsControls || []).find((x) => x.tag === 'INPUT' && x.type === 'number');
  if (num) { await c.clickAt(num.x, num.y); await sleep(300); await c.typeText('3', 60); await c.pressKey('Enter'); await sleep(1200); }
  await dismissMenus(); await c.pressKey('Escape'); await sleep(400);
  const w = await waitForWrite('Layout.base', before, 7000);
  p.limitWrote = w.changed; p.limitText = w.changed ? w.text : null;
  // Copy to clipboard, with a sentinel so a no-op is visible
  await dismissMenus();
  await c.clickElement('.bases-toolbar-results-menu'); await sleep(1000);
  await c.writeClipboard('SENTINEL');
  p.copyRow = await c.evaluate(`
    const els = Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-action'));
    const el = els.find(e => (e.textContent || '').trim().startsWith('Copy to clipboard'));
    if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  if (p.copyRow) { await c.clickAt(p.copyRow.x, p.copyRow.y); await sleep(1800); }
  p.clipboard = await c.readClipboard();
  p.modalAfterCopy = await c.evaluate(`const m = document.querySelector('.modal-container'); return m ? m.outerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 6000) : null;`);
  p.notices = await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(e => e.textContent);`);
  await dismissMenus(); await c.pressKey('Escape'); await sleep(300);
  p.file = { ...shape('Layout.base'), text: bytes('Layout.base') };
  return p;
});

// 24 — the three things `Oddities.base` asked at once, each on its own: a view
// with no `name`, a view whose `name` is the empty string, and an unknown key
// inside the `filters` map. And `bases:copy-table` again, this time with the
// view really focused by a click, because the first pass got Obsidian's own
// "Unable to copy to your clipboard" notice.
await step('s24-names-and-copy', async (p) => {
  p.cases = {};
  for (const f of ['NoName.base', 'EmptyName.base', 'FilterExtra.base']) {
    const before = { ...shape(f), text: bytes(f) };
    await openBase(f);
    const model = await modelDump();
    const afterOpen = { ...shape(f), text: bytes(f) };
    await closeActive();
    await sleep(2000);
    const afterClose = { ...shape(f), text: bytes(f) };
    p.cases[f] = { before, afterOpen, afterClose, rewrittenOnOpen: afterOpen.text !== before.text,
      rewrittenOnClose: afterClose.text !== afterOpen.text, mtimeMoved: afterClose.mtimeMs !== before.mtimeMs,
      model };
    save();
  }
  // copy-table with the table really focused
  await openBase('Layout.base');
  p.viewBefore = await c.evaluate(`const v = app.workspace.activeLeaf.view; return v.controller ? v.controller.viewName : null;`);
  await c.clickElement('.bases-table-container .bases-tr');
  await sleep(600);
  p.focused = await c.evaluate(`return { active: document.activeElement ? document.activeElement.className : null, hasFocus: document.hasFocus() };`);
  await c.writeClipboard('SENTINEL-2');
  p.ran = await c.evaluate(`return app.commands.executeCommandById('bases:copy-table');`);
  await sleep(3000);
  p.notices = await c.evaluate(`return Array.from(document.querySelectorAll('.notice')).map(e => e.textContent);`);
  p.clipboard = await c.readClipboard();
  return p;
});

// 15 — the final snapshot of every `.base` in the vault, beside the fixture.
await step('s15-final', async () => ({
  snapshot: snapshot(),
  vaultFiles: fs.readdirSync(VAULT).sort(),
  untouchedStillOriginal: bytes('Untouched.base') === SCRAMBLED,
}));

save();
c.close();
process.exit(0);
