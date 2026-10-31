// The new-tab button, driven over CDP: with the active tab in the middle
// of four, where the tab a click on `.workspace-tab-header-new-tab` goes,
// whether it becomes active, what it shows, and the same for the
// `workspace:new-tab` command and for a middle click on a tab title.
// Phases: fixtures | run   (node new-tab.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.newTab = all.newTab || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.newTab[phase] = out;
const save = () => { out.elapsedMs = Date.now() - T0; fs.writeFileSync(RESULTS, JSON.stringify(all, null, 1)); };
save();
async function step(name, fn) {
  out.order.push(name);
  const t = Date.now();
  try { const v = await fn(); out.steps[name] = { ok: true, ms: Date.now() - t, ...(v && typeof v === 'object' ? v : { value: v }) }; }
  catch (e) { out.steps[name] = { ok: false, ms: Date.now() - t, error: String(e && e.stack ? e.stack : e) }; }
  save();
  return out.steps[name];
}

if (phase === 'fixtures') {
  for (const n of ['Alpha', 'Beta', 'Gamma', 'Delta']) fs.writeFileSync(path.join(VAULT, n + '.md'), `# ${n}\n\nA note.\n`);
  out.fixtures = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}

const c = await connect();
out.vaultPath = await c.vaultPath();
save();

const tabsNow = () => c.evaluate(`
  const headers = Array.from(document.querySelectorAll('.mod-root .workspace-tab-header')).map(h => ({ title: h.getAttribute('aria-label'), active: h.classList.contains('is-active') }));
  const leaves = []; app.workspace.iterateRootLeaves(l => leaves.push({ file: l.view.file && l.view.file.path, type: l.view.getViewType(), active: l === app.workspace.activeLeaf }));
  const empty = document.querySelector('.mod-root .workspace-leaf.mod-active .empty-state');
  return { headers, leaves, emptyState: empty && { title: (empty.querySelector('.empty-state-title') || {}).textContent, actions: Array.from(empty.querySelectorAll('.empty-state-action')).map(a => a.textContent) }, focus: document.activeElement && document.activeElement.className };`);

const setup = async () => {
  await c.evaluate(`
    app.workspace.leftSplit.expand(); app.workspace.rightSplit.collapse();
    // Open the four first, then drop everything else: detaching every
    // leaf takes the tab group with it and getLeaf('tab') then fails.
    const old = []; app.workspace.iterateRootLeaves(l => old.push(l));
    for (const n of ['Alpha', 'Beta', 'Gamma', 'Delta']) { const f = app.vault.getAbstractFileByPath(n + '.md'); await app.workspace.getLeaf('tab').openFile(f); }
    for (const l of old) l.detach();
    return true;`);
  await sleep(500);
  // Activate the second tab the way a person does, by clicking its header.
  await c.clickElement('.mod-root .workspace-tab-header[aria-label="Beta"]');
  await sleep(400);
  return tabsNow();
};

await step('s01-button', async () => {
  const before = await setup();
  await c.clickElement('.mod-root .workspace-tab-header-new-tab .clickable-icon, .mod-root .workspace-tab-header-new-tab');
  await sleep(600);
  return { before, after: await tabsNow() };
});
await step('s02-command', async () => {
  const before = await setup();
  await c.evaluate(`app.commands.executeCommandById('workspace:new-tab'); return true;`);
  await sleep(600);
  return { before, after: await tabsNow() };
});
await step('s03-second-press', async () => {
  const before = await setup();
  await c.clickElement('.mod-root .workspace-tab-header-new-tab .clickable-icon, .mod-root .workspace-tab-header-new-tab');
  await sleep(400);
  await c.clickElement('.mod-root .workspace-tab-header-new-tab .clickable-icon, .mod-root .workspace-tab-header-new-tab');
  await sleep(600);
  return { before, after: await tabsNow() };
});
await step('s04-open-file-in-new-tab', async () => {
  // A file opened in a new tab from the explorer (ctrl-click): where it goes.
  const before = await setup();
  const r = await c.rect('.nav-file-title[data-path="Delta.md"]');
  await c.clickAt(r.cx, r.cy, { modifiers: 2 });
  await sleep(600);
  return { before, after: await tabsNow() };
});

out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
