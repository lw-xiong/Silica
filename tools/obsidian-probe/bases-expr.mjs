// The Bases expression language: the grammar, read structurally off the parse
// tree, and the values, read off a real file-bound context and cross-checked
// against rendered table cells. Answers what phase 4 cannot guess: operator
// precedence and associativity for every pair, the lambda form `list.filter`
// and `map` and `reduce` take (the published docs name the functions but not
// their argument syntax), string literals and escapes, coercion, null
// propagation, duration arithmetic and its unit letters, the signature and
// return type of every function reachable from a value, what an invalid
// statement does in a filter, a formula and a summary, and what `this`
// resolves to.
//
// Two things make this cheap. First, a parsed statement exposes its AST as
// plain JSON (`{type, ...}` nodes), so precedence is *read*, not inferred from
// values. Second, `mockContext.evaluateIdent(s)` is `evaluateFormula(MQ.parse(s))`
// — patching `evaluateFormula` on the prototype captures the parse of an
// arbitrary string, which turns the private parser into an oracle. The mock
// context is not bound to a file (idents come back NaN there), so values are
// taken from a real context found by evaluating `density` on a known note and
// checking it comes back 1.21.
//
// Phases: fixtures | run   (node bases-expr.mjs <phase>)
// ONLY_STEPS=e03,e04 re-runs a step or two.
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run'].includes(phase)) { console.error('phase must be fixtures|run'); process.exit(2); }

const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.basesExpr = all.basesExpr || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.basesExpr[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();

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

const vp = (rel) => path.join(VAULT, rel);
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };

// ---- fixtures -------------------------------------------------------------
// One note with a property of every type the properties section knows, so a
// real context can be asked what each ident evaluates to; one note missing most
// of them, for null propagation; and `.base` files for the three places a
// statement can sit.
const NOTES = {
  'Silica.md': `---
type: rock
status: todo
density: 1.21
silica: 50
count: 3
ratio: 0.5
negative: -4
big: 1000000
zeroish: 0
flag: true
off: false
when: 2026-01-02
stamp: 2026-01-02T03:04:05
text: Frothed basalt
numish: "42"
spaced: "  padded  "
empty: ""
listish:
  - alpha
  - beta
  - alpha
nums:
  - 3
  - 1
  - 2
linkish: "[[Pumice]]"
tags:
  - rock
  - igneous/extrusive
aliases:
  - Cinder
---

# Silica

Frothed basalt, see [[Pumice]] and [[Basalt]].

## Notes

Some text with a #rock tag.
`,
  'Pumice.md': `---
type: rock
status: done
density: 0.64
silica: 72
count: 10
when: 2026-03-04
tags:
  - rock
---

# Pumice

Floats. [[Silica]].
`,
  'Basalt.md': `---
type: rock
status: todo
density: 2.9
silica: 49
tags:
  - rock
---

# Basalt
`,
  'Bare.md': `# Bare

No frontmatter at all.
`,
};

const MAIN_BASE = `views:
  - type: table
    name: All
    order:
      - file.name
      - status
      - density
`;
// the three places a statement can sit, each with one that cannot parse
const BAD_FILTER = `filters:
  and:
    - 'density >>> 1'
views:
  - type: table
    name: BadFilter
    order:
      - file.name
`;
const BAD_FORMULA = `formulas:
  broken: 'density +'
  alsoBroken: 'nosuchfunction(1)'
  badMethod: 'density.nosuchmethod()'
views:
  - type: table
    name: BadFormula
    order:
      - file.name
      - formula.broken
      - formula.alsoBroken
      - formula.badMethod
`;
const BAD_SUMMARY = `summaries:
  broken: 'values.'
  alsoBroken: 'values.nosuchmethod()'
views:
  - type: table
    name: BadSummary
    order:
      - file.name
      - density
    summaries:
      density: broken
`;
// a view whose columns are the expressions themselves, so rendered cells can be
// read as the ground truth a user actually sees
const CELL_EXPRS = {
  cStr: '"ab" + "cd"',
  cNum: '1 + 2 * 3',
  cCoerceGt: '"3" > 2',
  cCoerceAdd: '1 + "2"',
  cNullAdd: 'nosuchprop + 1',
  cNullLen: 'nosuchprop.length',
  cNullFixed: 'nosuchprop.toFixed(2)',
  cNullEq: 'nosuchprop == null',
  cNullEqEmpty: 'nosuchprop == ""',
  cDateCmp: 'when > "2026-01-01"',
  cDateDiff: 'when - "2026-01-01"',
  cNowMinus: '(now() - when).days()',
  cDur: 'now() - "1 week"',
  cListJoin: 'listish.join("|")',
  cListFilter: 'listish.filter(value != "beta").join("|")',
  cListMap: 'nums.map(value * 2).join("|")',
  cThis: 'this.file.name',
  cIf: 'if(density > 1, "heavy", "light")',
  cDiv0: '1 / 0',
  cZeroDiv: '0 / 0',
  cNumish: 'numish + 1',
  cFlag: 'flag && true',
  cLink: 'linkish.asFile().name',
};
const CELLS_BASE = `formulas:
${Object.entries(CELL_EXPRS).map(([k, v]) => `  ${k}: ${JSON.stringify(v)}`).join('\n')}
views:
  - type: table
    name: Cells
    order:
      - file.name
${Object.keys(CELL_EXPRS).map((k) => `      - formula.${k}`).join('\n')}
`;

const FIXTURES = {
  ...NOTES,
  'Main.base': MAIN_BASE,
  'BadFilter.base': BAD_FILTER,
  'BadFormula.base': BAD_FORMULA,
  'BadSummary.base': BAD_SUMMARY,
  'Cells.base': CELLS_BASE,
};

if (phase === 'fixtures') {
  await step('fixtures', async () => {
    fs.mkdirSync(vp('.obsidian'), { recursive: true });
    fs.writeFileSync(vp('.obsidian/app.json'), '{}');
    for (const [name, text] of Object.entries(FIXTURES)) fs.writeFileSync(vp(name), text);
    return { vaultFiles: fs.readdirSync(VAULT).sort(), cellExprs: CELL_EXPRS };
  });
  process.exit(0);
}

// ---- run ------------------------------------------------------------------
const c = await connect();
const vault = await c.vaultPath();
if (vault.replace(/\/+$/, '') !== VAULT.replace(/\/+$/, '')) { console.error('wrong vault open: ' + vault); c.close(); process.exit(3); }
out.vault = vault;
out.version = await c.evaluate(`return { title: document.title, ua: navigator.userAgent };`);

const openBase = async (p) => {
  await c.evaluate(`app.workspace.detachLeavesOfType('bases'); app.workspace.detachLeavesOfType('markdown'); return true;`);
  await sleep(1200);
  await c.evaluate(`
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
};

// e01 — the controller's own evaluation surface, as source: which object is a
// per-file context, and what the view does with a formula.
await step('e01-surface', async (p) => {
  await openBase('Main.base');
  return c.evaluate(`
    const v = app.workspace.activeLeaf.view; const ctl = v.controller;
    // A function's name and arity, never its body (clean room, see CLAUDE.md).
    const src = (f) => { try { return typeof f === 'function' ? 'fn ' + (f.name || '(anonymous)') + '/' + f.length : String(f); } catch (e) { return 'ERR:' + e.message; } };
    const protoSrc = (o) => { if (!o) return null; const pr = Object.getPrototypeOf(o); if (!pr) return null;
      const r = {}; for (const k of Object.getOwnPropertyNames(pr)) { try { if (typeof pr[k] === 'function') r[k] = src(pr[k]); } catch { r[k] = 'ERR'; } } return r; };
    const shapeOf = (o) => o && typeof o === 'object' ? { ctor: o.constructor && o.constructor.name, own: Object.keys(o).slice(0, 40), proto: Object.getOwnPropertyNames(Object.getPrototypeOf(o) || {}).slice(0, 60) } : typeof o;
    let resultsDump = null;
    try {
      const r = ctl.results;
      if (r instanceof Map) resultsDump = { isMap: true, size: r.size, keys: Array.from(r.keys()).map(String).slice(0, 10),
        firstValue: shapeOf(Array.from(r.values())[0]),
        firstValueJson: (() => { try { return JSON.stringify(Array.from(r.values())[0], (k, val) => typeof val === 'function' ? '[fn]' : val).slice(0, 2000); } catch (e) { return 'ERR:' + e.message; } })() };
      else resultsDump = shapeOf(r);
    } catch (e) { resultsDump = 'ERR:' + e.message; }
    return {
      controllerProto: protoSrc(ctl),
      ctxShape: shapeOf(ctl.ctx),
      ctxProto: protoSrc(ctl.ctx),
      ctxJson: (() => { try { return JSON.stringify(ctl.ctx, (k, val) => typeof val === 'function' ? '[fn]' : val).slice(0, 3000); } catch (e) { return 'ERR:' + e.message; } })(),
      mockContextProto: protoSrc(ctl.mockContext),
      results: resultsDump,
      viewConfigGetEvaluatedFormula: src(Object.getPrototypeOf(v.query.views[0]).getEvaluatedFormula),
      evaluateRelevantProperties: src(Object.getPrototypeOf(ctl).evaluateRelevantProperties),
      queryState: (() => { try { return JSON.stringify(ctl.queryState, (k, val) => typeof val === 'function' ? '[fn]' : val).slice(0, 1500); } catch (e) { return 'ERR:' + e.message; } })(),
    };`);
});

// e02 — the oracle. Patch `evaluateFormula` on the prototype to capture the
// node `evaluateIdent` hands it, and find a context where `density` really is
// 1.21 (the mock context answers NaN, which is why it cannot be used for
// values). Everything after this calls `window.__probe`.
// Installing the oracle binds it to the controller of the base that is open at
// the time, so a step that opens a different base re-installs it first: the
// patch lives on the shared prototype, but `window.__probeCtx` does not.
const installOracle = () => c.evaluate(`
  const v = app.workspace.activeLeaf.view; const ctl = v.controller; const mc = ctl.mockContext;
  const proto = Object.getPrototypeOf(mc);
  if (!window.__probeOrig) window.__probeOrig = proto.evaluateFormula;
  let captured;
  proto.evaluateFormula = function (e) { captured = e; return window.__probeOrig.call(this, e); };
  const clean = (o) => { try { return JSON.parse(JSON.stringify(o)); } catch (e) { return 'ERR:' + e.message; } };
  const parse = (s) => {
    captured = undefined;
    let err = null, mockValue = null;
    try { mockValue = clean(mc.evaluateIdent(s)); } catch (e) { err = String((e && e.message) || e); }
    return { node: captured, ast: captured === undefined ? null : clean(captured), parseErr: err, mockValue };
  };
  // candidate per-file contexts, verified against a known property
  const file = app.vault.getAbstractFileByPath('Silica.md');
  const probe = parse('density');
  const candidates = [];
  const tryCtx = (name, fn) => {
    let ctx = null;
    try { ctx = fn(); } catch (e) { candidates.push({ name, ok: false, error: 'build: ' + e.message }); return; }
    if (!ctx || typeof ctx !== 'object') { candidates.push({ name, ok: false, error: 'not an object' }); return; }
    try {
      const val = probe.node.getValue(ctx);
      const got = val && val.data !== undefined ? String(val.data) : String(val);
      candidates.push({ name, ok: true, value: clean(val), str: got });
      if (!window.__probeCtx && got === '1.21') window.__probeCtx = ctx;
    } catch (e) { candidates.push({ name, ok: false, error: e.message }); }
  };
  tryCtx('mockContext', () => mc);
  tryCtx('controller.ctx', () => ctl.ctx);
  tryCtx('controller.ctx after regenerateLocal(file)', () => { ctl.ctx.regenerateLocal(file); return ctl.ctx; });
  tryCtx('controller.ctx.local', () => ctl.ctx.local);
  if (ctl.results instanceof Map) {
    const vals = Array.from(ctl.results.values());
    vals.slice(0, 6).forEach((val, i) => {
      tryCtx('results value ' + i, () => val);
      if (Array.isArray(val)) val.slice(0, 3).forEach((e2, j) => tryCtx('results value ' + i + '[' + j + ']', () => e2));
      if (val && val.ctx) tryCtx('results value ' + i + '.ctx', () => val.ctx);
    });
  }
  window.__probe = {
    parse,
    evalIn: (s) => {
      const r = parse(s);
      if (!r.node || !window.__probeCtx) return { ...r, value: null, valueErr: r.parseErr || 'no node or no ctx' };
      try { const val = r.node.getValue(window.__probeCtx);
        return { ...r, value: clean(val), str: val && val.data !== undefined ? String(val.data) : String(val),
                 icon: val && val.icon ? val.icon : null, ctor: val && val.constructor ? val.constructor.name : null }; }
      catch (e) { return { ...r, value: null, valueErr: String(e.message) }; }
    },
  };
  return {
    candidates,
    chosen: candidates.filter((x) => x.ok && x.str === '1.21').map((x) => x.name),
    haveCtx: !!window.__probeCtx,
    sanity: ['density', 'status', 'file.name', '1 + 2'].map((s) => {
      const r = window.__probe.evalIn(s);
      return { expr: s, str: r.str, icon: r.icon, value: r.value, parseErr: r.parseErr, valueErr: r.valueErr };
    }),
    parseWrapperShape: (() => { const r = parse('1 + 2'); return { ast: r.ast, ctor: r.node && r.node.constructor ? r.node.constructor.name : null,
      proto: r.node ? Object.getOwnPropertyNames(Object.getPrototypeOf(r.node) || {}) : null,
      hasText: r.node && r.node.text !== undefined ? r.node.text : '<no text>' }; })(),
  };`);

await step('e02-oracle', installOracle);

/** Parse a list of expressions and return their ASTs (no evaluation). */
const parseAll = (exprs) => c.evaluate(`
  const list = ${JSON.stringify(exprs)};
  const out = {};
  for (const s of list) { const r = window.__probe.parse(s); out[s] = { ast: r.ast, parseErr: r.parseErr }; }
  return out;`);
/** Parse and evaluate against the real context. */
const evalAll = (exprs) => c.evaluate(`
  const list = ${JSON.stringify(exprs)};
  const out = {};
  for (const s of list) { const r = window.__probe.evalIn(s);
    out[s] = { str: r.str, icon: r.icon, value: r.value, parseErr: r.parseErr, valueErr: r.valueErr, ast: r.ast }; }
  return out;`);

// e03 — precedence and associativity, read off the tree. Every pair of
// operators that can meet, plus the unary and postfix forms.
await step('e03-precedence', async () => parseAll([
  '1 + 2 * 3', '1 * 2 + 3', '1 - 2 - 3', '1 / 2 / 3', '2 * 3 % 4', '1 + 2 - 3', '1 - 2 + 3',
  '2 * 3 / 4', '1 + 2 + 3', '1 % 2 * 3', '(1 + 2) * 3',
  '-1 + 2', '-(1 + 2)', '- 1 * 2', '!a && b', '!(a && b)', '!a == b', '!a.b',
  'a && b || c', 'a || b && c', 'a && b && c', 'a || b || c',
  'a == b && c == d', 'a > b == c', 'a + b > c + d', 'a > b + c',
  'a == b == c', 'a != b != c', 'a < b < c',
  'a ? b : c', 'a ?? b', 'a ?? b ?? c', 'a ?? b + c',
  'a.b.c', 'a.b().c()', 'a.b(c).d', 'a.b.c()', '-a.b', 'a + b.c()',
  'a["k"]', 'a[0]', 'a[0].b', 'a.b[0]', 'a[b + 1]',
  '[1, 2]', '[1, 2][0]', '[]', '{}',
  'a && b == c || d', 'a + b * c - d / e',
  'a in b', 'a contains b', 'a and b', 'a or b', 'not a', 'a & b', 'a | b',
  '2 ** 3', '2 ^ 3', 'a === b', 'a !== b', 'a >= b <= c',
  'this', 'this.a', 'this.file.name', 'a.b = c',
]));

// e04 — the lambda form, the one blocking unknown. Every syntax a reasonable
// person would try, for filter, map and reduce.
await step('e04-lambda', async () => {
  const exprs = [
    'listish.filter(value != "beta")',
    'listish.filter(value)',
    'listish.filter(x => x != "beta")',
    'listish.filter(item => item != "beta")',
    'listish.filter(value => value != "beta")',
    'listish.filter((x) => x != "beta")',
    'listish.filter(v, v != "beta")',
    'listish.filter("value != @beta@")'.replace(/@/g, String.fromCharCode(39)),
    'nums.map(value * 2)',
    'nums.map(x => x * 2)',
    'nums.map(value)',
    'nums.map(item * 2)',
    'nums.reduce(value + 1)',
    'nums.reduce((a, b) => a + b)',
    'nums.reduce(acc, value => acc + value)',
    'nums.reduce(acc + value)',
    'nums.reduce(value, 0)',
    'nums.sort(value)',
    'nums.find(value > 1)',
    'nums.every(value > 0)',
    'nums.some(value > 2)',
    'nums.filter(index > 0)',
    'nums.map(index)',
    'nums.map(value, index)',
    'nums.flatMap(value)',
    'nums.indexOf(1)',
  ];
  const parsed = await parseAll(exprs);
  const valued = await evalAll(exprs);
  return { parsed, valued };
});

// e05 — string literals, escapes and primitives.
await step('e05-literals', async () => evalAll([
  '"abc"', "'abc'", '"a\\"b"', "'a\\'b'", '"a\'b"', "'a\"b'",
  '"a\\nb"', '"a\\tb"', '"a\\\\b"', '"a\\u00e9b"', '"café"', '"a\\qb"',
  '"unterminated', '`backtick`', '""', "''",
  '1', '1.5', '.5', '1.', '-1', '+1', '1e3', '0x10', '1_000', '01',
  'true', 'false', 'null', 'undefined', 'NaN', 'Infinity',
  '/re/', '/re/i', '"a" + 1', '1 + true',
]));

// e06 — values: coercion, null propagation, comparison across types, and what
// `this` is. Read against the real context, so these are the numbers a cell
// would show.
await step('e06-values', async () => evalAll([
  // coercion
  '"3" > 2', '2 > "3"', '"3" == 3', '"abc" > 1', 'true == 1', 'true + 1', 'false + 1',
  'numish + 1', 'numish > 41', '"10" > "9"', '10 > 9',
  // null and missing
  'nosuchprop', 'nosuchprop + 1', 'nosuchprop.length', 'nosuchprop.toFixed(2)',
  'nosuchprop == null', 'nosuchprop == ""', 'nosuchprop == 0', 'nosuchprop > 1',
  'empty', 'empty.length', 'empty == null', 'zeroish == null', 'off == null',
  'nosuchprop.nosuch.deeper', 'nosuchprop && 1', 'nosuchprop || 1',
  // arithmetic edges
  '1 / 0', '-1 / 0', '0 / 0', '1 % 0', '0.1 + 0.2', 'big * big',
  'density + silica', 'density.toFixed(3)', 'negative.abs()',
  // dates against strings
  'when', 'when > "2026-01-01"', 'when == "2026-01-02"', 'when - "2026-01-01"',
  'stamp', 'stamp > when', 'when.year', 'when.format("YYYY")',
  // this
  'this', 'this.file', 'this.file.name', 'this.file.path', 'this.status', 'this.density',
  'file', 'file.name', 'file.path', 'file.folder', 'file.ext', 'file.size', 'file.ctime', 'file.mtime',
  'file.tags', 'file.links', 'file.embeds', 'file.backlinks', 'file.properties', 'file.hasTag("rock")',
  // lists and links
  'listish', 'listish.length', 'listish.unique()', 'listish.join("|")', 'listish[0]',
  'tags', 'aliases', 'linkish', 'linkish.asFile()', 'linkish.asFile().name',
  'flag', 'flag && off', 'flag || off', '!flag',
]));

// e07 — durations: the units, the `M` versus `m` distinction, and what date
// arithmetic returns.
await step('e07-durations', async () => evalAll([
  'now()', 'today()', 'now() - "1 week"', 'now() - "1 day"', 'now() + "1 day"',
  '"1 week"', 'duration("1 week")', 'duration("1w")', 'duration(1, "week")',
  '"1y"', '"1M"', '"1m"', '"1d"', '"1w"', '"1h"', '"1s"', '"1 year"', '"1 month"', '"1 minute"',
  'duration("1y")', 'duration("1M")', 'duration("1m")', 'duration("1d")', 'duration("1w")',
  'duration("1h")', 'duration("1s")', 'duration("1y 2M 3d")', 'duration("-1d")',
  '(now() - when).days()', '(now() - when).years()', 'now() - when',
  'when + "1 month"', 'when - "1M"', 'when - "1m"',
  'date("2026-01-02")', 'date("2026-01-02") - date("2026-01-01")',
  'date("2026-01-02T03:04:05")', 'date(when)', 'date("not a date")',
  'now().format("YYYY-MM-DD")', 'now().date()', 'now().time()',
]));

// e08 — the function registry, by receiver. A seed value of each type is
// evaluated, its prototype walked, every method's source and arity recorded,
// and every method called with a plausible argument so the return type tag
// (`icon`) is observed rather than assumed.
// One seed value per receiver type, its prototype walked, every method's source
// and arity recorded, and every method called with a plausible argument so the
// return type tag (`icon`) is observed rather than assumed. One evaluate per
// seed: all of them in one call runs past the 15 s protocol timeout.
const SEEDS = {
  string: '"Frothed basalt"', number: '12.345', boolean: 'true', list: '[3, 1, 2]',
  date: 'now()', link: 'linkish', file: 'file', object: 'file.properties',
  regexp: '/ba(s)alt/', durationish: '"1 week"', missing: 'nosuchprop', tags: 'tags',
};
await step('e08-methods', async (p) => {
  p.seeds = SEEDS;
  p.byType = {};
  for (const [name, seed] of Object.entries(SEEDS)) {
    p.byType[name] = await c.evaluate(`
      const seed = ${JSON.stringify(seed)};
      const args = [[], ['"a"'], ['1'], ['2'], ['"YYYY-MM-DD"'], ['value > 1'], ['"a", "b"']];
      const r = window.__probe.evalIn(seed);
      const rec = { seed, str: r.str, icon: r.icon, value: r.value, parseErr: r.parseErr, valueErr: r.valueErr };
      let live = null;
      try { const pp = window.__probe.parse(seed); live = pp.node ? pp.node.getValue(window.__probeCtx) : null; }
      catch (e) { rec.liveErr = e.message; }
      if (live && typeof live === 'object') {
        const pr = Object.getPrototypeOf(live);
        rec.ctor = live.constructor && live.constructor.name;
        rec.own = Object.keys(live);
        rec.methods = {};
        for (const k of Object.getOwnPropertyNames(pr || {})) {
          let fn; try { fn = pr[k]; } catch { continue; }
          if (typeof fn !== 'function' || k === 'constructor') { rec.methods[k] = { notAFunction: typeof fn }; continue; }
          // The name and the arity, never the body: Obsidian's own source
          // must not land in this repository (clean room, see CLAUDE.md).
          rec.methods[k] = { arity: fn.length };
        }
        rec.calls = {};
        for (const k of Object.keys(rec.methods)) {
          if (rec.methods[k].notAFunction) continue;
          for (const a of args) {
            const expr = seed + '.' + k + '(' + a.join(', ') + ')';
            const rr = window.__probe.evalIn(expr);
            const good = !rr.parseErr && !rr.valueErr && rr.str !== undefined && rr.str !== 'null' && rr.str !== 'undefined' && rr.str !== 'NaN';
            if (good) { rec.calls[k] = { expr, str: rr.str, icon: rr.icon, value: rr.value }; break; }
            if (!rec.calls[k]) rec.calls[k] = { expr, str: rr.str, icon: rr.icon, parseErr: rr.parseErr, valueErr: rr.valueErr, value: rr.value };
          }
        }
        // the bare property form too, for the ones that are accessors
        rec.bare = {};
        for (const k of Object.keys(rec.methods)) {
          const rr = window.__probe.evalIn(seed + '.' + k);
          rec.bare[k] = { str: rr.str, icon: rr.icon, parseErr: rr.parseErr, valueErr: rr.valueErr };
        }
      }
      return rec;`);
    save();
  }
  return p;
});

// e14 — the function table, by brute force. The prototypes walked in e08 are
// NOT the Bases function table: `12.345.toFixed(2)` works although Number's
// prototype has only `renderTo`, and `[3,1,2].length()` is refused although
// List's prototype has `length`. What the table does give is four
// distinguishable refusals, which together are an existence-and-signature
// oracle:
//   Cannot find function "N" on type T      — no such function on that type
//   Cannot find "N" on type T               — no such property on that type
//   Cannot call function "N", not enough / too many arguments.
//   Type error in "N", parameter "P". Expected X, given Y.
// So every candidate name is called with 0..3 arguments of a type almost
// nothing expects (a regexp literal), and the messages are kept verbatim.
const NAMES = [
  'abs', 'add', 'asDate', 'asFile', 'asLink', 'asNumber', 'asString', 'average', 'avg',
  'capitalize', 'ceil', 'clamp', 'contains', 'containsAll', 'containsAny', 'containsAny',
  'count', 'date', 'day', 'dayOfWeek', 'days', 'diff', 'display', 'duration', 'earliest',
  'endOf', 'endsWith', 'every', 'ext', 'filter', 'find', 'first', 'flat', 'flatten', 'floor',
  'folder', 'format', 'groupBy', 'hasLink', 'hasProperty', 'hasTag', 'hour', 'hours',
  'humanize', 'icon', 'indexOf', 'inFolder', 'isEmpty', 'isTruthy', 'join', 'keys', 'last',
  'latest', 'length', 'link', 'linksTo', 'lower', 'map', 'matches', 'max', 'mean', 'median',
  'milliseconds', 'min', 'minute', 'minutes', 'month', 'months', 'name', 'not', 'number',
  'padEnd', 'padStart', 'path', 'percent', 'pow', 'properties', 'reduce', 'relative',
  'repeat', 'replace', 'reverse', 'round', 'search', 'second', 'seconds', 'sign', 'size',
  'slice', 'some', 'sort', 'split', 'sqrt', 'startOf', 'startsWith', 'stddev', 'sub',
  'substring', 'subtract', 'sum', 'tags', 'test', 'time', 'title', 'toFixed', 'toString',
  'trim', 'trunc', 'unique', 'unix', 'upper', 'value', 'values', 'week', 'weeks', 'year',
  'years',
];
const TYPE_SEEDS = {
  String: '"Frothed basalt"', Number: '12.345', Boolean: 'true', List: '[3, 1, 2]',
  Date: 'now()', DateTime: 'stamp', Link: 'linkish', File: 'file', Object: 'file.properties',
  Regexp: '/ba(s)alt/', Duration: '(now() - when)', Null: 'nosuchprop',
};
await step('e14-function-table', async (p) => {
  p.seeds = TYPE_SEEDS;
  p.byType = {};
  for (const [tname, seed] of Object.entries(TYPE_SEEDS)) {
    p.byType[tname] = await c.evaluate(`
      const seed = ${JSON.stringify(seed)};
      const names = ${JSON.stringify(NAMES)};
      const argsets = ['', '/zz/', '/zz/, /zz/', '/zz/, /zz/, /zz/'];
      const out = { fns: {}, props: {} };
      for (const n of names) {
        const tries = [];
        let exists = false;
        for (const a of argsets) {
          const r = window.__probe.evalIn(seed + '.' + n + '(' + a + ')');
          const msg = r.valueErr || null;
          tries.push({ args: a, str: r.str, icon: r.icon, err: msg, parseErr: r.parseErr });
          if (msg && /^Cannot find function/.test(msg)) break;
          if (!msg) { exists = true; break; }
          exists = true;
        }
        const first = tries[0];
        const absent = first.err && /^Cannot find function/.test(first.err);
        if (!absent) out.fns[n] = { exists: true, tries };
        // the property form, which has its own refusal
        const pr = window.__probe.evalIn(seed + '.' + n);
        const pAbsent = pr.valueErr && /^Cannot find "/.test(pr.valueErr);
        if (!pAbsent) out.props[n] = { str: pr.str, icon: pr.icon, err: pr.valueErr || null, parseErr: pr.parseErr };
      }
      return out;`);
    save();
  }
  return p;
});

// e15 — the same oracle on the globals: which names the table knows at all, and
// each one's parameters, read out of its type errors.
const GLOBALS = [
  'now', 'today', 'date', 'duration', 'number', 'string', 'bool', 'boolean', 'list', 'link',
  'file', 'icon', 'image', 'if', 'ifs', 'switch', 'choice', 'coalesce', 'min', 'max', 'sum',
  'average', 'avg', 'mean', 'median', 'round', 'floor', 'ceil', 'abs', 'sqrt', 'pow', 'log',
  'exp', 'trunc', 'sign', 'random', 'regexp', 'regex', 'object', 'type', 'empty', 'notEmpty',
  'isEmpty', 'concat', 'join', 'contains', 'length', 'count', 'first', 'last', 'reverse',
  'sort', 'unique', 'filter', 'map', 'reduce', 'html', 'element', 'markdown', 'embed', 'tag',
  'tags', 'prop', 'property', 'note', 'formula', 'taskList', 'progress', 'checkbox', 'color',
  'rgb', 'parseInt', 'parseFloat', 'toNumber', 'toString', 'asDate', 'asNumber', 'asString',
  'asLink', 'asFile', 'dateDiff', 'dateModify', 'nowUtc', 'utc', 'self', 'this',
];
await step('e15-globals-table', async () => c.evaluate(`
  const names = ${JSON.stringify(GLOBALS)};
  const argsets = ['', '/zz/', '/zz/, /zz/', '/zz/, /zz/, /zz/', '/zz/, /zz/, /zz/, /zz/'];
  const out = {};
  for (const n of names) {
    const tries = [];
    for (const a of argsets) {
      const r = window.__probe.evalIn(n + '(' + a + ')');
      const msg = r.valueErr || null;
      tries.push({ args: a, str: r.str, icon: r.icon, err: msg, parseErr: r.parseErr });
      if (msg && /^Cannot find function/.test(msg)) break;
      if (!msg) break;
    }
    const absent = tries[0].err && /^Cannot find function/.test(tries[0].err);
    out[n] = { exists: !absent, tries };
  }
  // and the case question, since Now() worked
  out['__case'] = ['now()', 'NOW()', 'Now()', 'nOw()', 'ToDay()'].map((e) => {
    const r = window.__probe.evalIn(e);
    return { expr: e, str: r.str, err: r.valueErr || null };
  });
  return out;`));

// e16 — `reduce`'s second parameter and the name of its accumulator, and what a
// Duration can be asked for: the two things e04 and e07 left open.
await step('e16-reduce-and-duration', async () => c.evaluate(`
  const exprs = [
    'nums.reduce(value, 0)', 'nums.reduce(value, 100)', 'nums.reduce(acc + value, 0)',
    'nums.reduce(accumulator + value, 0)', 'nums.reduce(total + value, 0)',
    'nums.reduce(result + value, 0)', 'nums.reduce(previous + value, 0)',
    'nums.reduce(carry + value, 0)', 'nums.reduce(sum + value, 0)',
    'nums.reduce(current + value, 0)', 'nums.reduce(prev + value, 0)',
    'nums.reduce(a + value, 0)', 'nums.reduce(acc, 0)', 'nums.reduce(index, 0)',
    'nums.reduce(value + index, 0)',
    'now() - when', '(now() - when).toString()', '(now() - when).relative()',
    'duration("1d") + duration("1d")', 'duration("1d") * 2', 'now() + duration("1d")',
    'when + duration("1M")', 'duration("1d") > duration("1h")',
    'listish.filter(value != "beta").length', 'nums.map(value * 2).sum()',
    'nums.filter(value > 1).join("-")',
  ];
  const out = {};
  for (const e of exprs) { const r = window.__probe.evalIn(e); out[e] = { str: r.str, icon: r.icon, err: r.valueErr || null, parseErr: r.parseErr }; }
  return out;`));

// e09 — the global functions: which names the parser knows, and what an unknown
// one does. The list is every global the published documentation names, plus a
// control that cannot exist.
await step('e09-globals', async () => {
  const exprs = [
    'now()', 'today()', 'date("2026-01-02")', 'duration("1d")', 'number("42")', 'string(42)',
    'list(1, 2)', 'list("a")', 'link("Pumice")', 'link("Pumice", "alias")', 'if(true, 1, 2)',
    'if(true, 1)', 'choice(true, 1, 2)', 'icon("rock")', 'image("pic.png")', 'file("Silica.md")',
    'min(1, 2)', 'max(1, 2)', 'sum(1, 2)', 'average(1, 2)', 'round(1.5)', 'floor(1.5)', 'ceil(1.5)',
    'abs(-1)', 'random()', 'regexp("a")', 'object()', 'type(1)', 'empty()', 'notEmpty(1)',
    'concat("a", "b")', 'join("a", "b")', 'contains("abc", "b")',
    'nosuchfunction(1)', 'NoSuchFunction()', 'Now()',
  ];
  return evalAll(exprs);
});

// e17 — the three questions e14-e16 left: whether names are case-insensitive
// everywhere or only for functions, the whole of File's property set in one
// place, and the `note.` / `formula.` / `file.` ident namespaces that decide
// whether a bare `density` is sugar for `note.density`.
await step('e17-case-and-namespaces', async () => c.evaluate(`
  const exprs = [
    // function name case (Now() worked, so: methods and properties too?)
    'now()', 'NOW()', '"abc".lower()', '"abc".LOWER()', '"abc".Lower()',
    '12.3.toFixed(1)', '12.3.TOFIXED(1)', '[1,2].sum()', '[1,2].SUM()',
    // property name case
    'file.name', 'file.NAME', 'file.Name', 'when.year', 'when.YEAR',
    // ident case
    'density', 'DENSITY', 'Density', 'status', 'STATUS',
    // object key case
    'file.properties.status', 'file.properties.STATUS',
    // tag and value case
    'file.hasTag("rock")', 'file.hasTag("Rock")', 'file.hasTag("#rock")',
    'listish.contains("alpha")', 'listish.contains("ALPHA")',
    // namespaces
    'note.density', 'note.status', 'note.nosuch', 'note',
    'formula.ppu', 'formula.nosuch',
    'file.name', 'file.links', 'file.embeds', 'file.backlinks', 'file.ctime',
    'file.mtime', 'file.size', 'file.folder', 'file.ext', 'file.path', 'file.tags',
    'file.properties', 'file.basename', 'file.asLink()', 'file.file',
    // the implicit-vs-qualified question from the other direction
    'this.file.name', 'this.note', 'this.formula',
  ];
  const out = {};
  for (const e of exprs) { const r = window.__probe.evalIn(e);
    out[e] = { str: r.str, icon: r.icon, err: r.valueErr || null, parseErr: r.parseErr, ast: r.ast }; }
  return out;`));

// e10 — an invalid statement in each of the three places: what the view shows,
// what the AST says, whether rows are filtered, and whether the bytes change.
await step('e10-invalid', async (p) => {
  p.cases = {};
  for (const f of ['BadFilter.base', 'BadFormula.base', 'BadSummary.base']) {
    const before = bytes(f);
    await openBase(f);
    const dump = await c.evaluate(`
      const v = app.workspace.activeLeaf.view; const ctl = v.controller;
      const clean = (o) => { try { return JSON.parse(JSON.stringify(o)); } catch (e) { return 'ERR:' + e.message; } };
      const safe = (f2) => { try { return f2(); } catch (e) { return 'ERR:' + e.message; } };
      return {
        errorDom: Array.from(document.querySelectorAll('.bases-error')).map((e) => ({ text: e.textContent, hidden: e.style.display === 'none' })),
        notices: Array.from(document.querySelectorAll('.notice')).map((e) => e.textContent),
        rowCount: document.querySelectorAll('.bases-table-container .bases-tr').length,
        resultCount: (document.querySelector('.bases-toolbar-result-count') || {}).textContent || null,
        cellText: Array.from(document.querySelectorAll('.bases-table-container .bases-tr')).slice(0, 4).map((tr) =>
          Array.from(tr.querySelectorAll('.bases-td')).map((td) => td.textContent)),
        headers: Array.from(document.querySelectorAll('.bases-table-header-name')).map((e) => e.textContent),
        controllerErrors: safe(() => clean(ctl.errors)),
        formulas: safe(() => Object.fromEntries(Object.entries(v.query.formulas || {}).map(([k, fo]) =>
          [k, { text: fo.text, toString: String(fo.toString()), ast: clean(fo.formula === undefined ? null : fo.formula) }]))),
        summaryFormulas: safe(() => Object.fromEntries(Object.entries(v.query.summaryFormulas || {}).map(([k, fo]) =>
          [k, { text: fo && fo.text, toString: fo ? String(fo.toString()) : null, ast: clean(fo && fo.formula !== undefined ? fo.formula : null) }]))),
        filtersJson: safe(() => JSON.stringify(v.query.filters, (k, val) => typeof val === 'function' ? '[fn]' : val).slice(0, 2000)),
        serializable: safe(() => clean(v.query.getSerializable())),
      };`);
    const afterOpen = bytes(f);
    await c.evaluate(`app.workspace.activeLeaf.detach(); return true;`);
    await sleep(2500);
    p.cases[f] = { dump, rewrittenOnOpen: afterOpen !== before, afterClose: bytes(f), rewrittenOnClose: bytes(f) !== afterOpen, before };
    save();
  }
  return p;
});

// e11 — rendered cells as the ground truth: one formula column per expression,
// read off the real table for two notes (one with every property, one missing
// most of them).
await step('e11-cells', async (p) => {
  await openBase('Cells.base');
  await sleep(2500);
  p.reinstall = await installOracle();
  p.exprs = CELL_EXPRS;
  p.table = await c.evaluate(`
    const headers = Array.from(document.querySelectorAll('.bases-table-header-name')).map((e) => e.textContent);
    const rows = Array.from(document.querySelectorAll('.bases-table-container .bases-tr')).map((tr) =>
      Array.from(tr.querySelectorAll('.bases-td')).map((td) => ({ text: td.textContent, classes: td.className })));
    // a blank cell is not always an empty value: a boolean renders as a
    // checkbox, whose textContent is ''. Keep one row's markup.
    const firstRowHtml = Array.from(document.querySelectorAll('.bases-table-container .bases-tr')).slice(0, 2).map((tr) =>
      Array.from(tr.querySelectorAll('.bases-td')).map((td) => td.innerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 400)));
    const colCount = headers.length;
    return { headers, rows, firstRowHtml, colCount, resultCount: (document.querySelector('.bases-toolbar-result-count') || {}).textContent || null,
      errorDom: Array.from(document.querySelectorAll('.bases-error')).map((e) => ({ text: e.textContent, hidden: e.style.display === 'none' })) };`);
  // and the same expressions through the oracle, for the comparison
  p.oracle = await c.evaluate(`
    const list = ${JSON.stringify(Object.values(CELL_EXPRS))};
    const out = {};
    for (const s of list) { const r = window.__probe.evalIn(s); out[s] = { str: r.str, icon: r.icon, parseErr: r.parseErr, valueErr: r.valueErr }; }
    return out;`);
  return p;
});

// e12 — the formula editor's own suggestions, which are the only place
// Obsidian names a function's signature to a user. Best effort: the editor is
// reached through the properties panel's "Add formula" row.
await step('e12-editor-suggestions', async (p) => {
  await openBase('Main.base');
  await c.evaluate(`document.querySelectorAll('.menu').forEach((m) => m.remove()); return true;`);
  await c.clickElement('.bases-toolbar-properties-menu');
  await sleep(1000);
  p.panel = await c.evaluate(`
    const m = document.querySelector('.menu');
    return m ? m.outerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 20000) : null;`);
  p.rows = await c.evaluate(`
    return Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-item')).map((e) => ({ text: (e.textContent || '').trim(), classes: e.className }));`);
  // the search box doubles as "Find or create..."; type a name and take the row
  const search = await c.rect('.menu input[type="search"]');
  if (search) {
    await c.clickAt(search.cx, search.cy); await sleep(300);
    await c.typeText('probeFormula', 40);
    await sleep(800);
    p.afterTyping = await c.evaluate(`
      return Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-item, .menu .suggestion-item')).map((e) => ({ text: (e.textContent || '').trim(), classes: e.className }));`);
    const create = await c.evaluate(`
      const els = Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-item, .menu .suggestion-item'));
      const el = els.find((e) => (e.textContent || '').trim() === 'Add formula');
      if (!el) return null; const b = el.getBoundingClientRect(); return { text: (el.textContent || '').trim(), x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
    p.createRow = create;
    if (create) {
      await c.clickAt(create.x, create.y); await sleep(1200);
      p.editorPanel = await c.evaluate(`
        const m = document.querySelector('.menu, .modal-container');
        return m ? m.outerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 20000) : null;`);
      // type into whatever input the editor offers and look for suggestions
      const inp = await c.evaluate(`
        const el = document.querySelector('.menu .cm-content, .menu input[type="text"], .modal-container .cm-content, .modal-container input[type="text"]');
        if (!el) return null; const b = el.getBoundingClientRect();
        return { tag: el.tagName, classes: el.className, x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
      p.editorInput = inp;
      if (inp) {
        await c.clickAt(inp.x, inp.y); await sleep(400);
        await c.typeText('probeName', 50);
        await c.pressKey('Tab'); await sleep(600);
        p.afterName = await c.evaluate(`
          const m = document.querySelector('.menu');
          return m ? m.outerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 20000) : null;`);
        const inp2 = await c.evaluate(`
          const el = document.querySelector('.menu .cm-content, .modal-container .cm-content, .menu textarea, .menu input[type="text"]:not([placeholder])');
          if (!el) return null; const b = el.getBoundingClientRect();
          return { tag: el.tagName, classes: el.className, x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
        p.formulaInput = inp2;
        if (inp2) { await c.clickAt(inp2.x, inp2.y); await sleep(300); }
        await c.typeText('file.', 60);
        await sleep(1200);
        p.suggestions = await c.evaluate(`
          return Array.from(document.querySelectorAll('.suggestion-container, .cm-tooltip-autocomplete, .suggestion')).map((m) => ({
            classes: m.className, text: (m.textContent || '').slice(0, 2000),
            items: Array.from(m.querySelectorAll('.suggestion-item, li, .cm-completionLabel')).map((e) => (e.textContent || '').trim()).slice(0, 120),
            html: m.outerHTML.replace(/<svg[\\s\\S]*?<\\/svg>/g, '<SVG>').slice(0, 8000) }));`);
        await c.typeText('name', 60);
        await sleep(1000);
        p.suggestionsAfterName = await c.evaluate(`
          return Array.from(document.querySelectorAll('.suggestion-container, .cm-tooltip-autocomplete, .suggestion')).map((m) => ({
            classes: m.className, items: Array.from(m.querySelectorAll('.suggestion-item, li, .cm-completionLabel')).map((e) => (e.textContent || '').trim()).slice(0, 120) }));`);
      }
    }
  }
  await c.pressKey('Escape'); await sleep(300);
  await c.evaluate(`document.querySelectorAll('.menu').forEach((m) => m.remove()); return true;`);
  return p;
});

// e18 — the formula editor's own completion list, which is the one place
// Obsidian publishes a signature to a user: each row is a name, and the info
// panel beside the selected row carries `name(param: Type?)` and a sentence.
// Typing a receiver and a dot lists exactly the functions and properties that
// receiver has, so this is the authoritative name list the error-message
// oracle of e14 can be checked against. The editor also shows a live
// "Valid formula" / error status, which is recorded per fragment.
await step('e18-completions', async (p) => {
  await openBase('Main.base');
  await c.evaluate(`document.querySelectorAll('.menu').forEach((m) => m.remove()); return true;`);
  await c.clickElement('.bases-toolbar-properties-menu');
  await sleep(1000);
  const search = await c.rect('.menu input[type="search"]');
  if (!search) return { failed: 'no search box in the properties panel' };
  await c.clickAt(search.cx, search.cy); await sleep(300);
  await c.typeText('probeFormula', 40);
  await sleep(700);
  const add = await c.evaluate(`
    const els = Array.from(document.querySelectorAll('.menu .bases-toolbar-menu-item, .menu .suggestion-item'));
    const el = els.find((e) => (e.textContent || '').trim() === 'Add formula');
    if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x + b.width / 2, y: b.y + b.height / 2 };`);
  p.addFormula = add;
  if (!add) return { ...p, failed: 'no "Add formula" row' };
  await c.clickAt(add.x, add.y); await sleep(1200);
  const ed = await c.evaluate(`
    const el = document.querySelector('.formula-editor .cm-content');
    if (!el) return null; const b = el.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2, placeholder: el.getAttribute('aria-placeholder') };`);
  p.editor = ed;
  if (!ed) return { ...p, failed: 'no formula editor' };
  const fragments = ['', 'file.', 'note.', '"a".', '12.', 'true.', '[1, 2].', 'now().',
    'linkish.', 'file.properties.', '/a/.', '(now() - when).', 'nosuchprop.', 'this.', 'formula.'];
  p.byFragment = {};
  for (const frag of fragments) {
    await c.clickAt(ed.x, ed.y); await sleep(250);
    await c.pressKey('a', { ctrl: true }); await sleep(150);
    await c.pressKey('Backspace'); await sleep(250);
    if (frag) await c.typeText(frag, 45);
    else { await c.typeChar('n'); await sleep(300); await c.pressKey('Backspace'); }
    await sleep(900);
    const dump = await c.evaluate(`
      const txt = (e) => (e && e.textContent || '').trim();
      const lists = Array.from(document.querySelectorAll('.cm-tooltip-autocomplete'));
      return {
        completions: lists.map((m) => Array.from(m.querySelectorAll('.cm-completionLabel')).map(txt)),
        info: Array.from(document.querySelectorAll('.cm-completionInfo')).map((e) => txt(e)),
        selected: lists.map((m) => { const el = m.querySelector('[aria-selected="true"]'); return el ? txt(el) : null; }),
        status: txt(document.querySelector('.formula-editor-status')),
        statusClasses: (document.querySelector('.formula-editor-status') || {}).className || null,
        content: txt(document.querySelector('.formula-editor .cm-content')),
      };`);
    // walk the list so each row's own info panel is captured
    const infos = [];
    const count = dump.completions[0] ? dump.completions[0].length : 0;
    for (let i = 0; i < Math.min(count, 40); i++) {
      const one = await c.evaluate(`
        const txt = (e) => (e && e.textContent || '').trim();
        const sel = document.querySelector('.cm-tooltip-autocomplete [aria-selected="true"]');
        return { label: sel ? txt(sel) : null, info: txt(document.querySelector('.cm-completionInfo')) };`);
      infos.push(one);
      await c.pressKey('ArrowDown'); await sleep(120);
    }
    p.byFragment[frag || '<empty>'] = { ...dump, infos };
    save();
  }
  await c.pressKey('Escape'); await sleep(300);
  await c.evaluate(`document.querySelectorAll('.menu').forEach((m) => m.remove()); return true;`);
  return p;
});

// e13 — restore the patched prototype, so nothing else in this session sees it.
await step('e13-restore', async () => c.evaluate(`
  const v = app.workspace.activeLeaf && app.workspace.activeLeaf.view;
  const mc = v && v.controller && v.controller.mockContext;
  if (mc && window.__probeOrig) { Object.getPrototypeOf(mc).evaluateFormula = window.__probeOrig; return 'restored'; }
  return 'nothing to restore';`));

save();
c.close();
process.exit(0);
