// The window frame, the chrome's type metrics and icon names, and the sidebar
// width: what the "Window frame style" setting writes, what the title bar
// holds in each style, which Lucide icons the ribbon, dock tabs, view header
// and tab strip use, the computed font sizes and heights of the shell, and
// what dragging a sidebar's resize handle records in workspace.json.
// Phases: fixtures | run   (node frame-typography.mjs <phase>)
import fs from 'node:fs';
import path from 'node:path';
import { connect, sleep } from './cdp.mjs';

const phase = process.argv[2];
const VAULT = process.env.VAULT;
const RESULTS = process.env.RESULTS;
if (!VAULT || !RESULTS) { console.error('VAULT and RESULTS env vars are required'); process.exit(2); }
if (!['fixtures', 'run', 'after-restart'].includes(phase)) { console.error('phase must be fixtures|run|after-restart'); process.exit(2); }

const T0 = Date.now();
const BUDGET_MS = 170000;

function loadResults() { try { return JSON.parse(fs.readFileSync(RESULTS, 'utf8')); } catch { return {}; } }
const all = loadResults();
all.frame = all.frame || {};
const out = { startedAt: new Date().toISOString(), steps: {}, order: [] };
all.frame[phase] = out;
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
const bytes = (rel) => { try { return fs.readFileSync(vp(rel), 'utf8'); } catch (e) { return e.code === 'ENOENT' ? null : 'ERR:' + e.message; } };
async function waitForChange(rel, before, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (bytes(rel) !== before) return true; await sleep(150); }
  return false;
}

if (phase === 'fixtures') {
  fs.mkdirSync(vp('.obsidian'), { recursive: true });
  fs.writeFileSync(vp('Alpha.md'), '# Alpha\n\nA note that links to [[Beta]] and carries a #tag.\n\n## Second heading\n\nBody.\n');
  fs.writeFileSync(vp('Beta.md'), '# Beta\n\nLinks back to [[Alpha]].\n');
  fs.writeFileSync(vp('.obsidian/core-plugins.json'), JSON.stringify({ 'file-explorer': true, 'global-search': true, graph: true, backlink: true, bookmarks: true, 'command-palette': true, switcher: true, outline: true, 'tag-pane': true, 'page-preview': true, 'daily-notes': true, templates: true }, null, 2));
  out.wrote = fs.readdirSync(VAULT);
  save();
  process.exit(0);
}

if (phase === 'after-restart') { save(); process.exit(0); }

const c = await connect();
out.vaultPath = await c.vaultPath();
out.version = await c.evaluate(`return { ua: navigator.userAgent, platform: navigator.platform };`);
save();

const PROBE_JS = `
  const cs = (el, props) => { if (!el) return null; const s = getComputedStyle(el); const o = {}; for (const p of props) o[p] = s.getPropertyValue(p); return o; };
  const rect = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; };
  const BOX = ['height', 'width', 'min-height', 'padding', 'margin', 'background-color', 'border-bottom', 'border-right', 'border-left', 'font-size', 'font-family', 'font-weight', 'line-height', 'color', 'gap', 'border-radius', 'opacity', 'letter-spacing', 'text-transform'];
  const one = (sel) => { const el = document.querySelector(sel); return el ? { sel, cls: el.className && el.className.baseVal !== undefined ? el.className.baseVal : el.className, rect: rect(el), style: cs(el, BOX), aria: el.getAttribute('aria-label') } : { sel, missing: true }; };
  const svgName = (el) => { if (!el) return null; const s = el.querySelector('svg'); if (!s) return { noSvg: true, text: (el.textContent || '').trim().slice(0, 20), html: el.innerHTML.slice(0, 200) }; return { cls: s.getAttribute('class'), w: s.getAttribute('width'), h: s.getAttribute('height'), viewBox: s.getAttribute('viewBox'), stroke: s.getAttribute('stroke-width'), paths: s.children.length }; };
`;

await step('s01-vars', async () => c.evaluate(PROBE_JS + `
  const names = ['--font-interface', '--font-interface-theme', '--font-interface-override', '--font-text', '--font-text-theme', '--font-monospace', '--font-monospace-theme', '--font-default', '--font-text-size', '--font-ui-smaller', '--font-ui-small', '--font-ui-medium', '--font-ui-large', '--font-smallest', '--font-smaller', '--font-small', '--line-height-normal', '--line-height-tight', '--h1-size', '--h2-size', '--h3-size', '--h4-size', '--h5-size', '--h6-size', '--h1-weight', '--h2-weight', '--h3-weight', '--h4-weight', '--h5-weight', '--h6-weight', '--h1-line-height', '--h2-line-height', '--h3-line-height', '--font-weight', '--bold-weight', '--header-height', '--ribbon-width', '--tab-container-background', '--titlebar-background', '--titlebar-background-focused', '--titlebar-height', '--tab-font-size', '--tab-font-weight', '--tab-text-color', '--tab-text-color-active', '--status-bar-font-size', '--status-bar-background', '--sidebar-tab-text-color', '--nav-item-size', '--nav-item-weight', '--nav-item-padding', '--file-line-width', '--file-margins', '--icon-size', '--icon-s', '--icon-m', '--icon-l', '--icon-stroke', '--icon-xs', '--icon-xs-stroke-width', '--icon-s-stroke-width', '--icon-m-stroke-width', '--background-primary', '--background-secondary', '--background-modifier-border', '--text-normal', '--text-muted', '--text-faint', '--text-accent', '--interactive-accent', '--radius-s', '--radius-m', '--radius-l', '--size-2-1', '--size-2-2', '--size-2-3', '--size-4-1', '--size-4-2', '--size-4-3', '--size-4-4', '--size-4-6', '--size-4-8', '--window-border-radius', '--frame-left-space', '--frame-right-space', '--sidebar-markdown-font-size'];
  const s = getComputedStyle(document.body);
  const vars = {}; for (const n of names) vars[n] = s.getPropertyValue(n).trim();
  return { bodyClass: document.body.className, vars, body: cs(document.body, ['font-family', 'font-size', 'line-height', 'font-weight', 'color', 'background-color']), zoom: window.devicePixelRatio, win: { w: window.innerWidth, h: window.innerHeight }, appearanceJson: ${JSON.stringify(bytes('.obsidian/appearance.json'))}, theme: app.vault.getConfig('theme'), fontDocument: document.fonts ? Array.from(document.fonts).map(f => f.family + ' ' + f.weight + ' ' + f.style + ' ' + f.status).slice(0, 40) : null };
`));

await step('s02-chrome', async () => c.evaluate(PROBE_JS + `
  const sels = ['.titlebar', '.titlebar-inner', '.titlebar-text', '.titlebar-button-container.mod-left', '.titlebar-button-container.mod-right', '.titlebar-button.mod-minimize', '.titlebar-button.mod-maximize', '.titlebar-button.mod-close', '.titlebar-button.mod-logo', '.titlebar-button.mod-back', '.titlebar-button.mod-forward',
    '.workspace-ribbon.mod-left', '.workspace-ribbon.mod-left .side-dock-actions', '.side-dock-ribbon-action', '.workspace-ribbon.mod-left .side-dock-settings', '.workspace-split.mod-left-split', '.workspace-split.mod-right-split', '.workspace-split.mod-root', '.mod-left-split .workspace-tab-header-container', '.mod-root .workspace-tab-header-container', '.mod-root .workspace-tab-header-container-inner', '.mod-root .workspace-tab-header', '.mod-root .workspace-tab-header.is-active', '.mod-root .workspace-tab-header-inner', '.mod-root .workspace-tab-header-inner-title', '.mod-root .workspace-tab-header-inner-close-button', '.mod-root .workspace-tab-header-new-tab', '.mod-root .workspace-tab-header-tab-list', '.mod-root .workspace-tab-header-spacer', '.mod-left-split .workspace-tab-header', '.mod-left-split .workspace-tab-header.is-active', '.mod-left-split .workspace-tab-header-inner-icon', '.mod-left-split .workspace-tab-header-spacer', '.sidebar-toggle-button.mod-left', '.sidebar-toggle-button.mod-right', '.sidebar-toggle-button.mod-left .clickable-icon', '.view-header', '.view-header-title-container', '.view-header-title', '.view-header-nav-buttons', '.view-header-nav-buttons .clickable-icon', '.view-actions', '.view-actions .clickable-icon', '.view-content', '.status-bar', '.status-bar-item', '.nav-header', '.nav-buttons-container', '.nav-action-button', '.nav-files-container', '.nav-file-title', '.nav-file-title-content', '.nav-folder-title', '.tree-item-self', '.tree-item-inner', '.cm-editor', '.cm-content', '.cm-scroller', '.markdown-source-view.mod-cm6 .cm-editor', '.inline-title', '.markdown-preview-view', '.mod-left-split .workspace-leaf-resize-handle', '.mod-right-split .workspace-leaf-resize-handle', '.mod-left-split > .workspace-leaf-resize-handle', '.workspace-leaf-resize-handle', '.workspace-tabs.mod-top', '.workspace-tabs.mod-top-left-space', '.workspace-tabs.mod-top-right-space', '.workspace .mod-top-left-space .workspace-tab-header-container', '.workspace-tab-header-container .workspace-tab-header-container-inner', '.horizontal-main-container', '.workspace', '.app-container', '.titlebar-button-container'];
  return { items: sels.map(one), leftWidth: app.workspace.leftSplit && app.workspace.leftSplit.size, rightWidth: app.workspace.rightSplit && app.workspace.rightSplit.size, leftCollapsed: app.workspace.leftSplit && app.workspace.leftSplit.collapsed, frameStyleGuess: { isFrameless: document.body.classList.contains('is-frameless'), isHiddenFrameless: document.body.classList.contains('is-hidden-frameless'), classes: Array.from(document.body.classList) } };
`));

await step('s03-icons', async () => c.evaluate(PROBE_JS + `
  const list = (sel) => Array.from(document.querySelectorAll(sel)).map(el => ({ aria: el.getAttribute('aria-label'), cls: el.className && el.className.baseVal !== undefined ? el.className.baseVal : el.className, icon: svgName(el), rect: rect(el), style: cs(el, ['width', 'height', 'padding', 'margin', 'color', 'opacity', 'border-radius']) }));
  const first = (sel) => list(sel)[0] || null;
  return {
    ribbon: list('.workspace-ribbon.mod-left .side-dock-ribbon-action'),
    ribbonSettings: list('.workspace-ribbon.mod-left .side-dock-settings .clickable-icon, .workspace-ribbon.mod-left .side-dock-settings'),
    leftDockTabs: list('.mod-left-split .workspace-tab-header'),
    rightDockTabs: list('.mod-right-split .workspace-tab-header'),
    sidebarToggles: list('.sidebar-toggle-button'),
    sidebarToggleIcons: list('.sidebar-toggle-button .clickable-icon'),
    titlebarButtons: list('.titlebar-button'),
    tabClose: first('.mod-root .workspace-tab-header-inner-close-button'),
    newTab: first('.mod-root .workspace-tab-header-new-tab .clickable-icon, .mod-root .workspace-tab-header-new-tab'),
    tabList: first('.mod-root .workspace-tab-header-tab-list .clickable-icon, .mod-root .workspace-tab-header-tab-list'),
    navButtons: list('.view-header-nav-buttons .clickable-icon'),
    viewActions: list('.view-actions .clickable-icon'),
    navHeader: list('.nav-header .nav-action-button, .nav-buttons-container .clickable-icon'),
    collapseIcons: list('.nav-folder-collapse-indicator, .tree-item-icon.collapse-icon'),
    statusBar: list('.status-bar-item'),
    svgDefault: (() => { const s = document.querySelector('.side-dock-ribbon-action svg'); return s ? cs(s, ['width', 'height', 'stroke-width', 'stroke', 'color']) : null; })(),
    anyIconSample: (() => { const s = document.querySelector('.side-dock-ribbon-action svg'); return s ? s.outerHTML.slice(0, 120) : null; })(),
  };
`));

await step('s04-frame-setting', async (r) => {
  await c.evaluate(`app.setting.open(); app.setting.openTabById('appearance'); return true;`);
  await sleep(800);
  r.appearanceSettings = await c.evaluate(`return Array.from(document.querySelectorAll('.vertical-tab-content .setting-item')).map(it => ({ name: (it.querySelector('.setting-item-name') || {}).textContent, desc: (it.querySelector('.setting-item-description') || {}).textContent, control: (() => { const s = it.querySelector('select'); if (s) return { select: s.value, options: Array.from(s.options).map(o => ({ v: o.value, t: o.textContent })) }; const t = it.querySelector('.checkbox-container'); if (t) return { toggle: t.classList.contains('is-enabled') }; const i = it.querySelector('input'); if (i) return { input: i.value, type: i.type }; const b = it.querySelector('button'); return b ? { button: b.textContent } : null; })() }));`);
  const frame = r.appearanceSettings.find((s) => /frame/i.test(s.name || ''));
  if (!frame) throw new Error('no frame setting among ' + r.appearanceSettings.map((s) => s.name).join(' | '));
  r.frame = frame;
  r.before = bytes('.obsidian/appearance.json');
  r.tries = [];
  const selSel = `Array.from(document.querySelectorAll('.vertical-tab-content .setting-item')).find(it => /frame/i.test((it.querySelector('.setting-item-name')||{}).textContent||'')).querySelector('select')`;
  for (const opt of frame.control.options) {
    const before = bytes('.obsidian/appearance.json');
    await c.evaluate(`const el = ${selSel}; el.value = ${JSON.stringify(opt.v)}; el.dispatchEvent(new Event('change', { bubbles: true })); return el.value;`);
    await waitForChange('.obsidian/appearance.json', before, 3000);
    await sleep(700);
    r.tries.push({ option: opt, appearanceJson: bytes('.obsidian/appearance.json'), bodyClass: await c.evaluate(`return document.body.className;`), titlebar: await c.evaluate(PROBE_JS + `return { bar: one('.titlebar'), inner: one('.titlebar-inner'), buttons: Array.from(document.querySelectorAll('.titlebar-button')).map(b => ({ cls: b.className, rect: rect(b), icon: svgName(b) })), text: (document.querySelector('.titlebar-text')||{}).textContent, notices: Array.from(document.querySelectorAll('.notice')).map(n => n.textContent) };`), configValue: await c.evaluate(`return { nativeMenus: app.vault.getConfig('nativeMenus'), keys: Object.keys(app.vault.config || {}) };`) });
  }
  // Back to what a fresh vault had.
  const before2 = bytes('.obsidian/appearance.json');
  await c.evaluate(`const el = ${selSel}; el.value = ${JSON.stringify(frame.control.select)}; el.dispatchEvent(new Event('change', { bubbles: true })); return el.value;`);
  await waitForChange('.obsidian/appearance.json', before2, 3000);
  r.restored = bytes('.obsidian/appearance.json');
  r.allConfigKeys = await c.evaluate(`return Object.keys(app.vault.config || {});`);
  await c.evaluate(`app.setting.close(); return true;`);
  await sleep(300);
  return r;
});

async function savedLayout() {
  const before = bytes('.obsidian/workspace.json');
  await c.evaluate(`if (app.workspace.saveLayout) { await app.workspace.saveLayout(); } else { app.workspace.requestSaveLayout(); } return true;`);
  await waitForChange('.obsidian/workspace.json', before, 4000);
  const raw = bytes('.obsidian/workspace.json');
  let j = null; try { j = JSON.parse(raw); } catch {}
  return { left: j && j.left, right: j && j.right, leftRibbon: j && j['left-ribbon'], keys: j && Object.keys(j), raw: raw && raw.slice(0, 3000) };
}

await step('s05-sidebar-width', async (r) => {
  r.before = await savedLayout();
  r.handles = await c.evaluate(PROBE_JS + `return Array.from(document.querySelectorAll('.workspace-leaf-resize-handle')).map(h => ({ cls: h.className, parent: h.parentElement.className, rect: rect(h), style: cs(h, ['width', 'height', 'cursor', 'position', 'left', 'right', 'background-color']) }));`);
  r.leftSplitRect = await c.evaluate(PROBE_JS + `return rect(document.querySelector('.mod-left-split'));`);
  const h = r.handles.find((x) => /mod-left-split/.test(x.parent) && x.rect.h > x.rect.w && x.rect.w > 0);
  if (!h) throw new Error('no left handle: ' + JSON.stringify(r.handles));
  const cx = h.rect.x + h.rect.w / 2, cy = h.rect.y + h.rect.h / 2;
  await c.drag(cx, cy, cx + 120, cy, { steps: 12 });
  await sleep(600);
  r.afterDrag = await savedLayout();
  r.leftSplitRectAfter = await c.evaluate(PROBE_JS + `return { rect: rect(document.querySelector('.mod-left-split')), style: document.querySelector('.mod-left-split').getAttribute('style'), size: app.workspace.leftSplit.size };`);
  // Collapse via the toggle button, record where the toggle lives, expand again.
  await c.clickElement('.sidebar-toggle-button.mod-left');
  await sleep(500);
  r.collapsed = { layout: await savedLayout(), toggle: await c.evaluate(PROBE_JS + `return { btn: one('.sidebar-toggle-button.mod-left'), leftSplit: one('.workspace-split.mod-left-split'), ribbon: one('.workspace-ribbon.mod-left'), rootTabsLeftSpace: one('.mod-root .workspace-tab-header-container') };`) };
  await c.clickElement('.sidebar-toggle-button.mod-left');
  await sleep(500);
  r.expanded = await savedLayout();
  // Right side too.
  await c.clickElement('.sidebar-toggle-button.mod-right');
  await sleep(500);
  r.rightCollapsed = { layout: await savedLayout(), toggle: await c.evaluate(PROBE_JS + `return { btn: one('.sidebar-toggle-button.mod-right'), rightSplit: one('.workspace-split.mod-right-split') };`) };
  await c.clickElement('.sidebar-toggle-button.mod-right');
  await sleep(500);
  r.rightExpanded = await savedLayout();
  r.commands = await c.evaluate(`return ['app:toggle-left-sidebar', 'app:toggle-right-sidebar', 'app:toggle-ribbon', 'window:toggle-always-on-top', 'app:toggle-default-new-pane-mode'].map(id => ({ id, cmd: app.commands.commands[id] ? { name: app.commands.commands[id].name, hotkeys: app.commands.commands[id].hotkeys } : null }));`);
  return r;
});

await step('s06-text-metrics', async () => {
  await c.evaluate(`const f = app.vault.getAbstractFileByPath('Alpha.md'); await app.workspace.getLeaf(false).openFile(f); return true;`);
  await sleep(900);
  return c.evaluate(PROBE_JS + `
    const el = (sel) => one(sel);
    await app.workspace.activeLeaf.setViewState({ type: 'markdown', state: { file: 'Alpha.md', mode: 'preview' } });
    await new Promise(r => setTimeout(r, 700));
    const preview = { view: el('.markdown-preview-view'), sizer: el('.markdown-preview-sizer'), h1: el('.markdown-preview-view h1'), h2: el('.markdown-preview-view h2'), p: el('.markdown-preview-view p'), link: el('.markdown-preview-view a.internal-link'), tag: el('.markdown-preview-view a.tag'), inlineTitle: el('.inline-title') };
    await app.workspace.activeLeaf.setViewState({ type: 'markdown', state: { file: 'Alpha.md', mode: 'source', source: false } });
    await new Promise(r => setTimeout(r, 700));
    const live = { editor: el('.cm-editor'), content: el('.cm-content'), line: el('.cm-line'), h1: el('.cm-line.HyperMD-header-1'), h2: el('.cm-line.HyperMD-header-2'), scroller: el('.cm-scroller'), sizer: el('.cm-sizer'), inlineTitle: el('.inline-title') };
    return { preview, live, outlineRow: el('.outline .tree-item-inner'), backlinkRow: el('.backlink-pane .tree-item-inner'), searchInput: el('.search-input-container input'), explorerRow: el('.nav-file-title'), explorerTitle: el('.nav-file-title-content'), activeRow: el('.nav-file-title.is-active'), statusBar: el('.status-bar'), statusItem: el('.status-bar-item'), tabTitle: el('.mod-root .workspace-tab-header-inner-title'), viewTitle: el('.view-header-title'), dockHeaderIcon: el('.mod-left-split .workspace-tab-header-inner-icon svg'), ribbonIcon: el('.side-dock-ribbon-action svg') };
  `);
});

await step('s07-quit-layout', async (r) => {
  r.final = await savedLayout();
  return r;
});

c.close();
save();
process.exit(0);
