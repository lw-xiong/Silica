import { Marked } from 'marked';
import { highlight, escapeHtml } from './highlight.mjs';

// --- front matter -----------------------------------------------------------
// A deliberately small YAML subset: `key: value` lines, `[a, b]` inline lists,
// and quoted strings. Everything the pages need, nothing that invites a parser.
export function frontMatter(src) {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(src);
  if (!m) return { data: {}, body: src };
  const data = {};
  for (const line of m[1].split(/\r?\n/)) {
    const kv = /^([A-Za-z_][\w-]*)\s*:\s*(.*)$/.exec(line.trim());
    if (!kv) continue;
    let v = kv[2].trim();
    if (/^\[.*\]$/.test(v)) {
      v = v.slice(1, -1).split(',').map((s) => s.trim().replace(/^["']|["']$/g, '')).filter(Boolean);
    } else {
      v = v.replace(/^["']|["']$/g, '');
      if (v === 'true') v = true;
      else if (v === 'false') v = false;
    }
    data[kv[1]] = v;
  }
  return { data, body: src.slice(m[0].length) };
}

// --- slugs ------------------------------------------------------------------
export function slugify(text) {
  return text
    .toLowerCase()
    .replace(/<[^>]+>/g, '')
    .replace(/&[a-z]+;/g, '')
    .replace(/[^\w\s-]/g, '')
    .trim()
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-') || 'section';
}

// --- rendering --------------------------------------------------------------
// Returns { html, toc, text }. `toc` is the h2/h3 outline the page template
// turns into the on-this-page rail; `text` is the stripped prose the search
// index is built from.
export function render(body, { baseUrl = '/', pageUrl = '/' } = {}) {
  const toc = [];
  const seen = new Map();
  const marked = new Marked({ gfm: true, breaks: false });

  const uniqueSlug = (base) => {
    const n = seen.get(base) || 0;
    seen.set(base, n + 1);
    return n ? `${base}-${n}` : base;
  };

  marked.use({
    renderer: {
      heading({ tokens, depth }) {
        const text = this.parser.parseInline(tokens);
        const plain = text.replace(/<[^>]+>/g, '');
        const id = uniqueSlug(slugify(plain));
        if (depth === 2 || depth === 3) toc.push({ depth, id, text: plain });
        // The anchor is the heading itself, so the whole line is a link target
        // on touch as well as the ¶ affordance on hover.
        return `<h${depth} id="${id}"><a class="anchor" href="#${id}">${text}<span class="anchor-mark" aria-hidden="true">#</span></a></h${depth}>\n`;
      },
      blockquote({ tokens }) {
        // A quote whose first words are **Note** / **Tip** / **Warning** becomes
        // a callout; every other quote stays an ordinary blockquote.
        const inner = this.parser.parse(tokens);
        const m = /^<p><strong>(Note|Tip|Warning|Caution)<\/strong>[\s:\u2014-]*/i.exec(inner);
        if (!m) return `<blockquote>\n${inner}</blockquote>\n`;
        const word = m[1][0].toUpperCase() + m[1].slice(1).toLowerCase();
        const kind = /warn|caution/i.test(word) ? 'warn' : /tip/i.test(word) ? 'tip' : 'note';
        const rest = `<p>${inner.slice(m[0].length)}`;
        return `<div class="callout ${kind}"><span class="callout-label">${word}</span>\n${rest}</div>\n`;
      },
      code({ text, lang }) {
        const label = (lang || '').split(/\s+/)[0];
        const body = highlight(text, label);
        const cls = label ? ` class="language-${escapeHtml(label)}"` : '';
        const tag = label ? `<span class="code-lang">${escapeHtml(label)}</span>` : '';
        return `<figure class="code">${tag}<pre><code${cls}>${body}</code></pre></figure>\n`;
      },
      table(token) {
        // Wrapped so a wide reference table scrolls inside its own box instead
        // of pushing the page sideways on a phone.
        const header = `<tr>${token.header.map((c, i) =>
          `<th${align(token.align[i])}>${this.parser.parseInline(c.tokens)}</th>`).join('')}</tr>`;
        const rows = token.rows.map((row) =>
          `<tr>${row.map((c, i) => `<td${align(token.align[i])}>${this.parser.parseInline(c.tokens)}</td>`).join('')}</tr>`).join('\n');
        return `<div class="table-scroll"><table>\n<thead>${header}</thead>\n<tbody>\n${rows}\n</tbody>\n</table></div>\n`;
      },
      link({ href, title, tokens }) {
        const text = this.parser.parseInline(tokens);
        const t = title ? ` title="${escapeHtml(title)}"` : '';
        const external = /^[a-z]+:/i.test(href) && !href.startsWith('mailto:');
        const rel = external ? ' target="_blank" rel="noopener noreferrer"' : '';
        const mark = external ? '<span class="ext" aria-hidden="true">↗</span>' : '';
        return `<a href="${escapeHtml(resolve(href, baseUrl, pageUrl))}"${t}${rel}>${text}${mark}</a>`;
      },
    },
  });

  const html = marked.parse(body);
  const text = body
    .replace(/^---[\s\S]*?---/, '')
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/[#*_>`|\-]+/g, ' ')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();

  return { html, toc, text };
}

const align = (a) => (a ? ` style="text-align:${a}"` : '');

// Content links are written site-relative (`/docs/app/fusion`) or relative to
// the page. Both come out as absolute paths under the site's base URL, so the
// same content builds at a root or under a prefix.
function resolve(href, baseUrl, pageUrl) {
  if (/^[a-z]+:/i.test(href) || href.startsWith('//') || href.startsWith('#')) return href;
  const base = baseUrl.endsWith('/') ? baseUrl : `${baseUrl}/`;
  if (href.startsWith('/')) return base + href.slice(1);
  const dir = pageUrl.replace(/[^/]*$/, '');
  return new URL(href, `http://x${dir.startsWith('/') ? dir : `/${dir}`}`).pathname.replace(/^\//, base);
}
