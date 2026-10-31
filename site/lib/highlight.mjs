// A small, dependency-free highlighter for the handful of languages the manual
// uses. It tokenises with one ordered pass of alternatives per language, so a
// string can never be re-scanned for keywords inside it (the classic naive-regex
// bug). Unknown languages fall through to escaped plain text.

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' };
export const escapeHtml = (s) => String(s).replace(/[&<>"]/g, (c) => ESC[c]);

const span = (cls, text) => `<span class="hl-${cls}">${escapeHtml(text)}</span>`;

// Each rule is [class, regex]. The rules are tried in order at the current
// position; the first that matches at the cursor wins. `y` (sticky) anchors
// every test to the cursor, which is what makes the single pass correct.
const RULES = {
  shell: [
    ['comment', /#[^\n]*/y],
    ['string', /'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"/y],
    ['prompt', /^[$#](?=\s)/y],
    ['flag', /(?<=\s)--?[A-Za-z][\w-]*/y],
    ['var', /\$\{[^}]*\}|\$[A-Za-z_]\w*/y],
    ['num', /\b\d+(?:\.\d+)?\b/y],
    ['cmd', /\b(?:silica|lapstack|cargo|rustup|npm|node|just|git|nix-shell|exiftool|ffmpeg|darktable-cli|curl|python3|sudo|cd|ls|mkdir|export|set)\b/y],
    ['kw', /\b(?:if|then|else|fi|for|in|do|done|while|case|esac|function|return)\b/y],
  ],
  rust: [
    ['comment', /\/\/[^\n]*|\/\*[\s\S]*?\*\//y],
    ['string', /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)'/y],
    ['attr', /#!?\[[^\]]*\]/y],
    ['kw', /\b(?:as|async|await|break|const|continue|crate|dyn|else|enum|extern|false|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|true|type|unsafe|use|where|while)\b/y],
    ['type', /\b(?:[A-Z]\w*|u8|u16|u32|u64|usize|i8|i16|i32|i64|isize|f32|f64|bool|str|String|Vec|Option|Result)\b/y],
    ['num', /\b\d[\d_]*(?:\.\d+)?(?:[iuf]\d+)?\b/y],
    ['fn', /\b[a-z_]\w*(?=\()/y],
  ],
  json: [
    ['key', /"(?:[^"\\]|\\.)*"(?=\s*:)/y],
    ['string', /"(?:[^"\\]|\\.)*"/y],
    ['kw', /\b(?:true|false|null)\b/y],
    ['num', /-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/y],
  ],
  lua: [
    ['comment', /--\[\[[\s\S]*?\]\]|--[^\n]*/y],
    ['string', /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/y],
    ['kw', /\b(?:and|break|do|else|elseif|end|false|for|function|if|in|local|nil|not|or|repeat|return|then|true|until|while)\b/y],
    ['num', /\b\d+(?:\.\d+)?\b/y],
    ['fn', /\b[A-Za-z_]\w*(?=\s*\()/y],
  ],
  toml: [
    ['comment', /#[^\n]*/y],
    ['attr', /^\[[^\]\n]*\]/y],
    ['key', /^[A-Za-z_][\w.-]*(?=\s*=)/y],
    ['string', /"(?:[^"\\]|\\.)*"|'[^']*'/y],
    ['kw', /\b(?:true|false)\b/y],
    ['num', /\b\d+(?:\.\d+)?\b/y],
  ],
};

const ALIASES = {
  sh: 'shell', bash: 'shell', zsh: 'shell', console: 'shell', shell: 'shell',
  rs: 'rust', rust: 'rust',
  json: 'json', lua: 'lua', toml: 'toml',
};

export function highlight(code, lang) {
  const rules = RULES[ALIASES[(lang || '').toLowerCase()]];
  if (!rules) return escapeHtml(code);
  let out = '';
  let i = 0;
  // `plain` accumulates the characters no rule claimed, so runs of ordinary
  // text are emitted as one escaped chunk rather than one span per character.
  let plain = '';
  const flushPlain = () => { if (plain) { out += escapeHtml(plain); plain = ''; } };
  while (i < code.length) {
    let matched = false;
    for (const [cls, re] of rules) {
      re.lastIndex = i;
      // `^` in a sticky regex means start-of-input, not start-of-line, so the
      // line-anchored rules (a shell prompt, a TOML table header) are tested
      // only when the cursor actually sits at the head of a line.
      if (re.source.startsWith('^') && i !== 0 && code[i - 1] !== '\n') continue;
      const m = re.exec(code);
      if (m && m[0].length) {
        flushPlain();
        out += span(cls, m[0]);
        i += m[0].length;
        matched = true;
        break;
      }
    }
    if (!matched) { plain += code[i]; i += 1; }
  }
  flushPlain();
  return out;
}
