import { connect, sleep } from './cdp.mjs';
const c = await connect();
const setup = async (text, line, ch, selTo) => {
  await c.evaluate(`
    const ed = app.workspace.activeEditor.editor;
    ed.setValue(${JSON.stringify(text)});
    ${selTo != null
      ? `ed.setSelection({line:${line},ch:${ch}},{line:${line},ch:${selTo}});`
      : `ed.setCursor({line:${line},ch:${ch}});`}
    ed.focus(); return true;`);
  await sleep(150);
};
const read = async () => JSON.parse(await c.evaluate(`
  const ed = app.workspace.activeEditor.editor; const c0 = ed.getCursor();
  return JSON.stringify({ text: ed.getValue(), ch: c0.ch });`));

const out = [];
const t = async (name, pre, act) => {
  await setup(...pre);
  await act();
  await sleep(200);
  const r = await read();
  out.push({ case: name, result: JSON.stringify(r.text), caret: r.ch });
};

// Skip-over: caret before a closer Obsidian itself inserted.
await t('type ( then )', ['', 0, 0], async () => { await c.typeChar('('); await sleep(120); await c.typeChar(')'); });
// Skip-over: a closer that was never auto-inserted (planted, no opener).
await t('planted ) alone, type )', [')', 0, 0], async () => c.typeChar(')'));
// Planted full pair, caret inside.
await t('planted (), caret inside, type )', ['()', 0, 1], async () => c.typeChar(')'));
// Backspace on an empty auto-pair.
await t('type ( then Backspace', ['', 0, 0], async () => { await c.typeChar('('); await sleep(120); await c.pressKey('Backspace'); });
// Backspace on a planted empty pair.
await t('planted (), caret inside, Backspace', ['()', 0, 1], async () => c.pressKey('Backspace'));
// Wrap a selection.
await t('select word, type (', ['hello', 0, 0, 5], async () => c.typeChar('('));
await t('select word, type "', ['hello', 0, 0, 5], async () => c.typeChar('"'));
await t('select word, type *', ['hello', 0, 0, 5], async () => c.typeChar('*'));
// Apostrophe inside a word.
await t("apostrophe in don|t", ['dont', 0, 3], async () => c.typeChar("'"));
// Inside a fenced code block.
await t('( inside fenced code', ['```\n\n```', 1, 0], async () => c.typeChar('('));
// Wikilink trigger.
await t('type [ twice', ['', 0, 0], async () => { await c.typeChar('['); await sleep(150); await c.typeChar('['); });

console.table(out);
c.close();
