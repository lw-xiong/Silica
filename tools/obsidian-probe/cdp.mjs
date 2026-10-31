// Drive a running Obsidian over the Chrome DevTools Protocol.
// Launch it as `obsidian --remote-debugging-port=9222` first. No dependencies:
// Node 24 has a built-in WebSocket.
//
// Input events go through Input.dispatch*, i.e. the browser injects them, so
// they carry focus and take the same path a person's keyboard and mouse do.
// `evaluate` runs in the renderer with `app` in scope.

const base = process.env.CDP_BASE || 'http://127.0.0.1:9222';

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function pageTargets() {
  const ts = await (await fetch(`${base}/json`)).json();
  return ts.filter((x) => x.type === 'page').map((x) => x.webSocketDebuggerUrl);
}

async function openSocket(url) {
  const ws = new WebSocket(url);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 0;
  const pending = new Map();
  // Protocol *events* (no `id`), by method name. Drag interception needs
  // them: Chrome answers a drag gesture with `Input.dragIntercepted`.
  const listeners = new Map();
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
      const { res, rej } = pending.get(m.id);
      pending.delete(m.id);
      m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result);
      return;
    }
    if (m.method && listeners.has(m.method)) for (const fn of listeners.get(m.method)) fn(m.params);
  };
  const on = (method, fn) => {
    if (!listeners.has(method)) listeners.set(method, new Set());
    listeners.get(method).add(fn);
    return () => listeners.get(method).delete(fn);
  };
  const once = (method, timeoutMs = 5000) => new Promise((res, rej) => {
    const off = on(method, (params) => { off(); clearTimeout(t); res(params); });
    const t = setTimeout(() => { off(); rej(new Error(method + ' event timeout')); }, timeoutMs);
  });
  const send = (method, params = {}) =>
    new Promise((res, rej) => {
      const i = ++id;
      pending.set(i, { res, rej });
      ws.send(JSON.stringify({ id: i, method, params }));
      setTimeout(() => { if (pending.has(i)) { pending.delete(i); rej(new Error(method + ' timeout')); } }, 15000);
    });
  return { ws, send, on, once };
}

/** Is this page the vault window, booted? */
async function vaultReady(send) {
  const r = await send('Runtime.evaluate', {
    expression: `typeof app !== 'undefined' && !!app.workspace && app.workspace.layoutReady === true`,
    returnByValue: true,
  });
  return r.result && r.result.value === true;
}

/**
 * Poll until a page target has a booted `app` (after a launch or relaunch).
 * The endpoint appears before the vault window does, and the first page
 * target can be the launcher, so "CDP answers" is not "Obsidian is ready".
 */
export async function waitForObsidian(timeoutMs = 90000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    let urls = [];
    try { urls = await pageTargets(); } catch { /* not up yet */ }
    for (const url of urls) {
      try {
        const { ws, send } = await openSocket(url);
        const ok = await vaultReady(send);
        ws.close();
        if (ok) return url;
      } catch { /* try the next */ }
    }
    await sleep(500);
  }
  throw new Error('no Obsidian vault window with a booted `app` within ' + timeoutMs + 'ms');
}

export async function connect() {
  const url = await waitForObsidian();
  const { ws, send, on, once } = await openSocket(url);

  /** Run `expr` (a function body; may `await`; `app` is in scope) in the renderer, return its value. */
  const evaluate = async (expr) => {
    const r = await send('Runtime.evaluate', {
      expression: `(async () => { ${expr} })()`,
      awaitPromise: true, returnByValue: true,
    });
    if (r.exceptionDetails) {
      const d = r.exceptionDetails;
      throw new Error((d.exception && d.exception.description) || d.text || JSON.stringify(d).slice(0, 400));
    }
    return r.result.value;
  };

  // ---- keyboard ----------------------------------------------------------
  const typeChar = async (ch) => {
    await send('Input.dispatchKeyEvent', { type: 'keyDown', text: ch, unmodifiedText: ch, key: ch });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key: ch });
  };
  const typeText = async (s, delay = 30) => { for (const ch of s) { await typeChar(ch); await sleep(delay); } };

  const KEYS = {
    Enter: ['Enter', 13], Tab: ['Tab', 9], Escape: ['Escape', 27], Backspace: ['Backspace', 8],
    Delete: ['Delete', 46], ArrowUp: ['ArrowUp', 38], ArrowDown: ['ArrowDown', 40],
    ArrowLeft: ['ArrowLeft', 37], ArrowRight: ['ArrowRight', 39], Home: ['Home', 36], End: ['End', 35],
    PageUp: ['PageUp', 33], PageDown: ['PageDown', 34], F2: ['F2', 113], Space: ['Space', 32],
  };
  const MOD = { alt: 1, ctrl: 2, meta: 4, shift: 8 };
  const modifiers = (m = {}) => (m.alt ? 1 : 0) | (m.ctrl ? 2 : 0) | (m.meta ? 4 : 0) | (m.shift ? 8 : 0);

  /** Press a named key (see KEYS) or a single character, with optional {ctrl,shift,alt,meta}. */
  const pressKey = async (name, mods = {}) => {
    const m = modifiers(mods);
    if (KEYS[name]) {
      const [code, vk] = KEYS[name];
      const p = { key: name, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: m };
      if (name === 'Enter') { p.text = '\r'; p.unmodifiedText = '\r'; }
      await send('Input.dispatchKeyEvent', { type: 'rawKeyDown', ...p });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', ...p });
      return;
    }
    // A letter with a modifier, e.g. ('z', {ctrl:true}) or ('v', {ctrl:true}).
    const up = name.toUpperCase();
    const vk = up.charCodeAt(0);
    const code = /[A-Z]/.test(up) ? 'Key' + up : /[0-9]/.test(up) ? 'Digit' + up : undefined;
    const p = { key: name, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: m };
    if (!m) { p.text = name; p.unmodifiedText = name; }
    await send('Input.dispatchKeyEvent', { type: m ? 'rawKeyDown' : 'keyDown', ...p });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', ...p });
  };

  // ---- mouse -------------------------------------------------------------
  /** Viewport rect of the first element matching `selector` (CSS pixels), or null. */
  const rect = async (selector, index = 0) => evaluate(`
    const els = document.querySelectorAll(${JSON.stringify(selector)});
    const el = els[${index}]; if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 };`);

  const mouse = async (type, x, y, extra = {}) =>
    send('Input.dispatchMouseEvent', { type, x: Math.round(x), y: Math.round(y), ...extra });

  const mouseMove = (x, y) => mouse('mouseMoved', x, y);

  /** A real click at viewport coordinates. button: 'left' | 'right' | 'middle'. */
  const clickAt = async (x, y, { button = 'left', clickCount = 1, mods = {} } = {}) => {
    const m = modifiers(mods);
    await mouse('mouseMoved', x, y, { modifiers: m });
    await mouse('mousePressed', x, y, { button, clickCount, modifiers: m });
    await mouse('mouseReleased', x, y, { button, clickCount, modifiers: m });
  };

  /** Click the first (or `index`th) element matching `selector`; returns the rect used or null. */
  const clickElement = async (selector, opts = {}) => {
    const r = await rect(selector, opts.index || 0);
    if (!r) return null;
    if (r.w === 0 && r.h === 0) return { ...r, skipped: 'zero rect (hidden)' };
    await clickAt(r.cx, r.cy, opts);
    return r;
  };
  const rightClickElement = (selector, opts = {}) => clickElement(selector, { ...opts, button: 'right' });
  const dblClickElement = (selector, opts = {}) => clickElement(selector, { ...opts, clickCount: 2 });
  const hover = async (selector, index = 0) => { const r = await rect(selector, index); if (r) await mouseMove(r.cx, r.cy); return r; };

  /** Press at (x1,y1), move in `steps` increments to (x2,y2), release. */
  const drag = async (x1, y1, x2, y2, { steps = 12, pauseMs = 40, button = 'left' } = {}) => {
    await mouse('mouseMoved', x1, y1);
    await mouse('mousePressed', x1, y1, { button, clickCount: 1 });
    for (let i = 1; i <= steps; i++) {
      await mouse('mouseMoved', x1 + ((x2 - x1) * i) / steps, y1 + ((y2 - y1) * i) / steps, { button });
      await sleep(pauseMs);
    }
    await mouse('mouseReleased', x2, y2, { button, clickCount: 1 });
  };

  /**
   * A real HTML5 drag and drop. `Input.dispatchMouseEvent` never raises
   * `dragstart`, which is why the synthetic-event passes could not establish
   * what a drop does; `Input.setInterceptDrags` makes Chrome hand the drag
   * data back as an `Input.dragIntercepted` event, and `Input.dispatchDragEvent`
   * then plays it into the page the way the compositor would.
   * Returns the intercepted drag data, so a caller can see what the source
   * offered (`mimeTypes`, and Obsidian's own payload) even if the drop fails.
   */
  const dragAndDrop = async (x1, y1, x2, y2, { steps = 10, pauseMs = 60 } = {}) => {
    await send('Input.setInterceptDrags', { enabled: true });
    const intercepted = once('Input.dragIntercepted', 6000);
    await mouse('mouseMoved', x1, y1);
    await mouse('mousePressed', x1, y1, { button: 'left', clickCount: 1 });
    // A few pixels of movement is what starts a drag.
    for (let i = 1; i <= 4; i++) await mouse('mouseMoved', x1 + i * 3, y1 + i * 2, { button: 'left' });
    let data;
    try { ({ data } = await intercepted); }
    catch (e) {
      await mouse('mouseReleased', x1, y1, { button: 'left', clickCount: 1 });
      await send('Input.setInterceptDrags', { enabled: false });
      return { started: false, error: e.message };
    }
    const dispatch = (type, x, y) => send('Input.dispatchDragEvent', { type, x: Math.round(x), y: Math.round(y), data });
    await dispatch('dragEnter', x1, y1);
    for (let i = 1; i <= steps; i++) {
      await dispatch('dragOver', x1 + ((x2 - x1) * i) / steps, y1 + ((y2 - y1) * i) / steps);
      await sleep(pauseMs);
    }
    await dispatch('drop', x2, y2);
    await send('Input.setInterceptDrags', { enabled: false });
    return { started: true, data };
  };

  /** Drag without dropping, so the hover state over a target can be read. */
  const dragOverOnly = async (x1, y1, x2, y2) => {
    await send('Input.setInterceptDrags', { enabled: true });
    const intercepted = once('Input.dragIntercepted', 6000);
    await mouse('mouseMoved', x1, y1);
    await mouse('mousePressed', x1, y1, { button: 'left', clickCount: 1 });
    for (let i = 1; i <= 4; i++) await mouse('mouseMoved', x1 + i * 3, y1 + i * 2, { button: 'left' });
    let data;
    try { ({ data } = await intercepted); }
    catch (e) {
      await mouse('mouseReleased', x1, y1, { button: 'left', clickCount: 1 });
      await send('Input.setInterceptDrags', { enabled: false });
      return { started: false, error: e.message };
    }
    await send('Input.dispatchDragEvent', { type: 'dragEnter', x: Math.round(x1), y: Math.round(y1), data });
    await send('Input.dispatchDragEvent', { type: 'dragOver', x: Math.round(x2), y: Math.round(y2), data });
    return { started: true, data, finish: async (dropX, dropY) => {
      await send('Input.dispatchDragEvent', { type: dropX === undefined ? 'dragCancel' : 'drop', x: Math.round(dropX === undefined ? x2 : dropX), y: Math.round(dropY === undefined ? y2 : dropY), data });
      await send('Input.setInterceptDrags', { enabled: false });
    } };
  };

  // ---- misc --------------------------------------------------------------
  /** System clipboard text, read through Electron in the renderer. */
  const readClipboard = () => evaluate(`return window.require('electron').clipboard.readText();`);
  const writeClipboard = (text) => evaluate(`window.require('electron').clipboard.writeText(${JSON.stringify(text)}); return true;`);

  /** Snapshot the open menu/popups: every .menu-item label in order, with separators. */
  const openMenuItems = () => evaluate(`
    const menus = Array.from(document.querySelectorAll('.menu'));
    return menus.map(menu => Array.from(menu.querySelectorAll('.menu-item, .menu-separator')).map(c =>
      c.classList.contains('menu-separator') ? '---' :
      ((c.querySelector('.menu-item-title') || c).textContent || '').trim()));`);

  /** Absolute path of the vault this window has open — check it, never assume. */
  const vaultPath = () => evaluate(`return app.vault.adapter.basePath || app.vault.adapter.getBasePath();`);

  /** Read a native <select>'s options and current value, and set it the way a user would (change event). */
  const selectInfo = (selector) => evaluate(`
    const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return null;
    return { value: el.value, options: Array.from(el.options).map(o => ({ value: o.value, label: o.textContent.trim(), selected: o.selected })) };`);
  const selectSet = (selector, value) => evaluate(`
    const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return null;
    el.value = ${JSON.stringify(value)}; el.dispatchEvent(new Event('change', { bubbles: true })); return el.value;`);

  /** Ask Obsidian to close its window cleanly (flushes workspace.json), then wait for the endpoint to go away. */
  const quit = async (timeoutMs = 40000) => {
    const gone = async () => { try { return (await pageTargets()).length === 0; } catch { return true; } };
    // Ask the window to close, then drop the debugger session: a page with an
    // attached debugger can hold its window open.
    try { await evaluate(`setTimeout(() => window.close(), 100); return true;`); } catch { /* socket may drop */ }
    try { ws.close(); } catch { /* ignore */ }
    const end = Date.now() + timeoutMs;
    let askedApp = false;
    while (Date.now() < end) {
      if (await gone()) return true;
      // Half way through, fall back to the app-level quit command through a fresh socket.
      if (!askedApp && Date.now() > end - timeoutMs / 2) {
        askedApp = true;
        try {
          const urls = await pageTargets();
          const { ws: w2, send: s2 } = await openSocket(urls[0]);
          await s2('Runtime.evaluate', { expression: `app.commands.executeCommandById('app:quit') || require('electron').ipcRenderer.send('quit')`, returnByValue: true });
          w2.close();
        } catch { /* ignore */ }
      }
      await sleep(400);
    }
    throw new Error('Obsidian did not exit');
  };

  return {
    send, evaluate, typeChar, typeText, pressKey, KEYS, rect, mouseMove, clickAt, clickElement,
    rightClickElement, dblClickElement, hover, drag, dragAndDrop, dragOverOnly,
    readClipboard, writeClipboard, openMenuItems,
    vaultPath, selectInfo, selectSet, on, once, quit, close: () => ws.close(),
  };
}
