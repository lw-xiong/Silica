// Silica corpus renderer.
//
// Renders every markdown note in the vault with Obsidian's own reading-view
// renderer and writes the HTML next to the vault, under ../expected/, mirroring
// the note paths. Silica's conformance test diffs its renderer against these
// files. Run the "Render corpus" command, or create a file named
// `.silica-autorun` in the vault to render on startup and close the window.
//
// Only the public plugin API is used (Plugin, MarkdownRenderer, Component,
// Vault); nothing here depends on Obsidian internals.

const { Plugin, MarkdownRenderer, Component, Notice } = require('obsidian');
const fs = require('fs');
const path = require('path');

module.exports = class SilicaCorpusPlugin extends Plugin {
  async onload() {
    this.addCommand({
      id: 'render-corpus',
      name: 'Render corpus: write every note as HTML to ../expected',
      callback: () => this.renderAll(false),
    });
    const autorun = await this.app.vault.adapter.exists('.silica-autorun');
    if (autorun) {
      this.app.workspace.onLayoutReady(() => this.renderAll(true));
    }
  }

  outDir() {
    const base = this.app.vault.adapter.getBasePath();
    return path.join(path.dirname(base), 'expected');
  }

  async renderNote(file) {
    const markdown = await this.app.vault.read(file);
    const el = document.createElement('div');
    const component = new Component();
    component.load();
    await MarkdownRenderer.render(this.app, markdown, el, file.path, component);
    // Give asynchronous post-processors (embeds, code highlighting) a tick.
    await new Promise((resolve) => setTimeout(resolve, 0));
    component.unload();
    return el.innerHTML;
  }

  async renderAll(closeWhenDone) {
    const out = this.outDir();
    const files = this.app.vault.getMarkdownFiles().sort((a, b) => a.path.localeCompare(b.path));
    const started = Date.now();
    let count = 0;
    const errors = [];
    for (const file of files) {
      try {
        const html = await this.renderNote(file);
        const target = path.join(out, file.path.replace(/\.md$/, '.html'));
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.writeFileSync(target, html);
        count += 1;
      } catch (err) {
        errors.push(`${file.path}: ${err && err.message ? err.message : err}`);
      }
    }
    const summary = {
      rendered: count,
      errors,
      millis: Date.now() - started,
    };
    fs.mkdirSync(out, { recursive: true });
    fs.writeFileSync(path.join(out, 'render-summary.json'), JSON.stringify(summary, null, 2) + '\n');
    new Notice(`Silica corpus: rendered ${count} notes in ${summary.millis} ms`);
    if (closeWhenDone) {
      fs.writeFileSync(path.join(out, '.done'), String(Date.now()));
      setTimeout(() => window.close(), 500);
    }
  }
};
