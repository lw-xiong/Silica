// Settings → Files and links → "Default location for new attachments": the
// exact option labels, the string each one stores in `app.json`, and the
// dependent row the two folder options reveal. The last unperformed probe of
// [[Interaction probes#Attachment paste]], and what the inert row in Silica's
// Settings modal waits on.
//
// Its own script because the Settings modal has hung this harness's CDP
// endpoint before: a hang here must not cost another block's rows.
// Phases: fixtures | run
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT, RESULTS = process.env.RESULTS;
const vp = (rel) => path.join(VAULT, rel);
if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.mkdirSync(vp('Sub'), { recursive: true });
  fs.writeFileSync(vp('Target.md'), 'BEFORE AFTER\n');
  fs.writeFileSync(vp('Sub/Nested.md'), 'x\n');
  fs.writeFileSync(vp('.obsidian/app.json'), '{}');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'command-palette': true }, null, 2));
  fs.writeFileSync(RESULTS, '{}');
  process.exit(0);
}
if (phase === 'after-restart') process.exit(0);

const c = await connect();
const out = { vault: await c.vaultPath(), startedAt: new Date().toISOString() };
const save = () => fs.writeFileSync(RESULTS, JSON.stringify(out, null, 1));
const appJson = () => { try { return JSON.parse(fs.readFileSync(vp('.obsidian/app.json'), 'utf8')); } catch (e) { return { readError: String(e) }; } };
save();

out.version = await c.evaluate(`return { ua: navigator.userAgent, title: document.title };`);
out.before = appJson();

// Open the tab and describe every row, so the dropdown is identified by what
// Obsidian calls it rather than by a guess at its position.
out.tab = await c.evaluate(`
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  app.setting.open();
  app.setting.openTabById('file');
  await sleep(1200);
  const tab = app.setting.activeTab;
  const el = tab.containerEl;
  return {
    tabId: tab.id, tabName: tab.name,
    navItems: Array.from(app.setting.containerEl.querySelectorAll('.vertical-tab-nav-item')).map((x) => x.textContent),
    rows: Array.from(el.querySelectorAll('.setting-item')).map((it) => ({
      name: (it.querySelector('.setting-item-name') || {}).textContent,
      description: (it.querySelector('.setting-item-description') || {}).textContent,
      control: (() => {
        const sel = it.querySelector('select');
        if (sel) return { kind: 'dropdown', value: sel.value, options: Array.from(sel.options).map((o) => ({ value: o.value, label: o.textContent })) };
        const tog = it.querySelector('.checkbox-container');
        if (tog) return { kind: 'toggle', on: tog.classList.contains('is-enabled') };
        const txt = it.querySelector('input[type="text"], input[type="search"]');
        if (txt) return { kind: 'text', value: txt.value, placeholder: txt.getAttribute('placeholder') };
        const btn = it.querySelector('button');
        if (btn) return { kind: 'button', text: btn.textContent };
        return { kind: 'none' };
      })(),
      hidden: it.offsetParent === null,
      classes: it.className,
    })),
  };`);
save();

// Walk every option of the attachment dropdown, reading app.json after each.
const ROW = 'Default location for new attachments';
out.options = {};
const optionValues = (out.tab.rows.find((r) => r.name === ROW) || {}).control;
out.dropdown = optionValues;
if (optionValues && optionValues.options) {
  for (const opt of optionValues.options) {
    const picked = await c.evaluate(`
      const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
      const el = app.setting.activeTab.containerEl;
      const row = Array.from(el.querySelectorAll('.setting-item')).find((it) => ((it.querySelector('.setting-item-name') || {}).textContent || '') === ${JSON.stringify(ROW)});
      const sel = row.querySelector('select');
      sel.value = ${JSON.stringify(opt.value)};
      sel.dispatchEvent(new Event('change', { bubbles: true }));
      await sleep(1200);
      return {
        selectValue: sel.value,
        configValue: app.vault.getConfig('attachmentFolderPath'),
        revealed: Array.from(el.querySelectorAll('.setting-item')).filter((it) => it.offsetParent !== null).map((it) => ({
          name: (it.querySelector('.setting-item-name') || {}).textContent,
          text: (() => { const t = it.querySelector('input[type="text"]'); return t ? { value: t.value, placeholder: t.getAttribute('placeholder') } : null; })(),
        })).filter((x) => x.text),
      };`);
    out.options[opt.value] = { label: opt.label, picked, appJson: appJson() };
    save();
  }
  // The two options with a text field: type a path into it and read back.
  for (const opt of optionValues.options) {
    const typed = await c.evaluate(`
      const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
      const el = app.setting.activeTab.containerEl;
      const row = Array.from(el.querySelectorAll('.setting-item')).find((it) => ((it.querySelector('.setting-item-name') || {}).textContent || '') === ${JSON.stringify(ROW)});
      const sel = row.querySelector('select');
      sel.value = ${JSON.stringify(opt.value)};
      sel.dispatchEvent(new Event('change', { bubbles: true }));
      await sleep(900);
      const field = Array.from(el.querySelectorAll('.setting-item')).filter((it) => it.offsetParent !== null).map((it) => it.querySelector('input[type="text"]')).find(Boolean);
      if (!field) return 'no text field for this option';
      field.focus();
      field.value = 'Attachments';
      field.dispatchEvent(new Event('input', { bubbles: true }));
      field.dispatchEvent(new Event('change', { bubbles: true }));
      field.blur();
      await sleep(1200);
      return { fieldValue: field.value, configValue: app.vault.getConfig('attachmentFolderPath'), rowName: (() => { const r = field.closest('.setting-item'); return (r.querySelector('.setting-item-name') || {}).textContent; })() };`);
    out.options[opt.value].withPath = { typed, appJson: appJson() };
    save();
  }
}

// Where an attachment actually lands under each stored value, through the API
// the paste path uses.
out.resolved = await c.evaluate(`
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const out = {};
  const target = app.vault.getAbstractFileByPath('Target.md');
  const nested = app.vault.getAbstractFileByPath('Sub/Nested.md');
  for (const v of ['', '/', './Attachments', 'Attachments', './', 'att/deep/new']) {
    app.vault.setConfig('attachmentFolderPath', v);
    await sleep(250);
    out[JSON.stringify(v)] = {
      fromRoot: await app.fileManager.getAvailablePathForAttachment('photo.png', target.path),
      fromSubfolder: await app.fileManager.getAvailablePathForAttachment('photo.png', nested.path),
    };
  }
  return out;`);
save();

out.after = appJson();
await c.evaluate(`app.setting.close(); return true;`);
await sleep(500);
out.finishedAt = new Date().toISOString();
save();
c.close();
process.exit(0);
