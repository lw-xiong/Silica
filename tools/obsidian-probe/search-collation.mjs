// The one unexplained row of the search collation capture: Obsidian put
// `notes/apple` ahead of `apple.md` on a basename tie, which a path tie-break
// cannot explain. Several same-basename files at different depths and under
// differently named folders, read in the order the search pane renders them
// under every sort order.
// Phases: fixtures | run
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS;
const vp = (rel) => path.join(VAULT, rel);

const FILES = [
  'apple.md', 'a/apple.md', 'm/apple.md', 'z/apple.md', 'm/n/apple.md',
  'a/b/c/apple.md', 'zzz/apple.md', 'Apple.md'.replace('Apple', 'APPLE'),
  'b/banana.md', 'banana.md',
];

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  for (const f of FILES) {
    fs.mkdirSync(path.dirname(vp(f)), { recursive: true });
    fs.writeFileSync(vp(f), `needle in ${f}\n`);
  }
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'global-search': true, 'command-palette': true }, null, 2));
  fs.writeFileSync(RESULTS, '{}');
  process.exit(0);
}
if (phase === 'after-restart') process.exit(0);

const c = await connect();
const out = { vault: await c.vaultPath(), title: await c.evaluate(`return document.title;`), fixtures: FILES };
const save = () => fs.writeFileSync(RESULTS, JSON.stringify(out, null, 1));
save();

out.vaultOrder = await c.evaluate(`return app.vault.getMarkdownFiles().map((f) => f.path);`);
await c.evaluate(`
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  app.workspace.leftSplit.expand();
  app.commands.executeCommandById('global-search:open');
  await sleep(1200);
  return true;`);

out.sortOrders = {};
for (const order of ['alphabetical', 'alphabeticalReverse', 'byModifiedTime', 'byModifiedTimeReverse', 'byCreatedTime', 'byCreatedTimeReverse']) {
  out.sortOrders[order] = await c.evaluate(`
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const v = app.workspace.getLeavesOfType('search')[0].view;
    if (v.setSortOrder) v.setSortOrder(${JSON.stringify(order)});
    else if (v.dom) { v.dom.sortOrder = ${JSON.stringify(order)}; v.dom.changed(); }
    await sleep(600);
    const out = {};
    for (const q of ['needle', 'file:apple', 'path:apple']) {
      v.setQuery(q);
      await sleep(1800);
      out[q] = {
        sortOrder: v.dom ? v.dom.sortOrder : null,
        order: v.dom && v.dom.vChildren && v.dom.vChildren.children ? v.dom.vChildren.children.map((ch) => ch.file ? ch.file.path : null) : null,
      };
    }
    return out;`);
  save();
}

// The quick switcher's own order, for comparison.
out.switcher = await c.evaluate(`
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  app.commands.executeCommandById('switcher:open');
  await sleep(800);
  const input = document.querySelector('.prompt-input');
  input.focus(); input.value = 'apple';
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await sleep(1200);
  const rows = Array.from(document.querySelectorAll('.suggestion-item')).map((x) => x.textContent.trim());
  const esc = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true });
  document.dispatchEvent(esc);
  return rows;`);
await c.pressKey('Escape');
save();
out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
