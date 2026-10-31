// `editor:insert-footnote`: the number it picks and where the definition
// lands, read without seeding the open file at all. Every fixture is written
// to disk in the `fixtures` phase, before Obsidian starts, so the metadata
// cache is settled and no file-change event can move the caret under the
// command — which is what garbled the first attempt at these rows.
// `editor:insert-footnote` turned out to work only ONCE per Obsidian launch
// when the probe reaches the file with `leaf.openFile`: the first case in a
// run lands, every later one is a no-op with the command still reporting it
// ran. So the script takes one case per run, named by FN_CASE.
// Phases: fixtures | run
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS;
const vp = (rel) => path.join(VAULT, rel);

const CASES = [
  ['none', 'Fn01.md', 'body text\n', 0, 9],
  ['one-numeric', 'Fn02.md', 'body[^1] text\n\n[^1]: one\n', 0, 13],
  ['two-numeric', 'Fn03.md', 'a[^1] b[^2] c\n\n[^1]: one\n[^2]: two\n', 0, 13],
  ['named-only', 'Fn04.md', 'a[^note] b\n\n[^note]: named\n', 0, 10],
  ['gap-starts-at-2', 'Fn05.md', 'a[^2] b\n\n[^2]: two\n', 0, 7],
  ['mid-document', 'Fn06.md', '# Head\n\npara one\n\npara two\n', 2, 8],
  ['caret-before-ref', 'Fn07.md', 'a[^1] b\n\n[^1]: one\n', 0, 1],
  ['ref-without-definition', 'Fn08.md', 'a[^1] b\n', 0, 7],
  ['no-trailing-newline', 'Fn09.md', 'body text', 0, 9],
  ['in-list-item', 'Fn10.md', '- item one\n- item two\n', 0, 10],
];

const ONLY = process.env.FN_CASE;
const SELECTED = ONLY ? CASES.filter(([l]) => l === ONLY) : CASES;
if (ONLY && !SELECTED.length) { console.error('no such case: ' + ONLY); process.exit(2); }

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  for (const [, name, body] of SELECTED) fs.writeFileSync(vp(name), body);
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'command-palette': true }, null, 2));
  fs.writeFileSync(RESULTS, '{}');
  process.exit(0);
}
if (phase === 'after-restart') process.exit(0);

const c = await connect();
const out = { vault: await c.vaultPath(), cases: {} };
const save = () => fs.writeFileSync(RESULTS, JSON.stringify(out, null, 1));
await c.evaluate(`app.workspace.leftSplit.collapse(); return true;`);

for (const [label, name, body, line, ch] of SELECTED) {
  out.cases[label] = await c.evaluate(`
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const ls = app.workspace.getLeavesOfType('markdown');
    const leaf = ls.length ? ls[0] : app.workspace.getLeaf(false);
    await leaf.openFile(app.vault.getAbstractFileByPath(${JSON.stringify(name)}));
    app.workspace.setActiveLeaf(leaf, { focus: true });
    await sleep(900);
    const ed = leaf.view.editor;
    const untouched = ed.getValue() === ${JSON.stringify(body)};
    ed.focus();
    ed.setCursor({ line: ${line}, ch: ${ch} });
    const caretBefore = ed.getCursor();
    const ran = app.commands.executeCommandById('editor:insert-footnote');
    await sleep(400);
    const value = ed.getValue();
    await leaf.view.save();
    await sleep(500);
    return { untouched, ran, caretBefore, value, cursor: ed.getCursor() };`);
  out.cases[label].disk = fs.readFileSync(vp(name), 'utf8');
  out.cases[label].in = body;
  save();
}
c.close();
process.exit(0);
