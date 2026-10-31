#!/usr/bin/env python3
"""Generate candidate OFM probe notes with a local model, render them in a real
Obsidian, and report which ones Silica's parser gets wrong.

The workflow this automates is the one in CLAUDE.md: when Obsidian and
`silica-markdown` disagree, a probe note goes into
`crates/silica-markdown/tests/corpus/vault/edge`, the vault is re-rendered,
and the observed behavior is matched and written down. The tedious half is
*thinking of* the snippets, and that is the half a small local model does well
-- because Obsidian itself is the oracle here, a weak snippet is only a boring
test, never a wrong answer.

    probe_gen.py areas
    probe_gen.py generate --area callout --count 40
    probe_gen.py render
    probe_gen.py triage
    probe_gen.py promote --note "Probe Callouts" --only 0003,0007,0019

Candidates are staged in their own vault, never written straight into the
committed corpus: the staging run is throwaway, and only snippets that actually
disagree are worth promoting.

The model is reached over the same environment variables as the
`silica-llm-mcp` MCP server, so one place configures both:

    SILICA_LLM_URL     default http://127.0.0.1:8080/v1   (misco's llama-server
                       router; from another machine use the tailnet name,
                       http://misco.tail.ragtux.com:8080/v1)
    SILICA_LLM_MODEL   default qwen3.6-35b-a3b-code
    SILICA_LLM_KEY     bearer token, if the server wants one

The default preset is deliberate. The general presets (`qwen3.8-27b`,
`qwen3.6-35b-a3b`) set `presence-penalty = 1.5`, which is right for prose and
wrong here: a syntax probe is *meant* to repeat itself (`==a==b == spaced ==
==== four`), and penalising repeated tokens steers the model away from exactly
the snippets worth having. The `-code` presets set it to 0 and have thinking
off, so the answer arrives in one pass.
"""

import argparse
import difflib
import json
import os
import re
import secrets
import shutil
import subprocess
import tempfile
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "crates/silica-markdown/tests/corpus"
EDGE = CORPUS / "vault/edge"
DEFAULT_STAGING = Path(
    os.environ.get("PROBE_STAGING", "/tmp/silica-probe-staging")
).expanduser()

LLM_URL = os.environ.get("SILICA_LLM_URL", "http://127.0.0.1:8080/v1").rstrip("/")
LLM_MODEL = os.environ.get("SILICA_LLM_MODEL", "qwen3.6-35b-a3b-code")
LLM_KEY = os.environ.get("SILICA_LLM_KEY") or None

# Anchored so it can never match this process; admits the flags Obsidian is
# launched with between the electron binary and the asar. Lifted from
# tools/obsidian-probe/run-block.sh, which learned it the hard way.
PGREP = r"^/nix/store/[^ ]*electron( --[^ ]*)* /nix/store/[^ ]*obsidian"

# ---------------------------------------------------------------------------
# The syntax areas

# Each area names the surface to probe. The point of the list is that a model
# asked for "markdown edge cases" produces CommonMark trivia, while a model
# asked for *these* boundaries produces things Obsidian actually treats
# specially. `avoid` steers it off ground the corpus already covers heavily.
AREAS = {
    "wikilink": dict(
        surface="""`[[target]]`, `[[target|alias]]`, `[[target#Heading]]`,
            `[[target#^blockid]]`, `[[#Heading in this file]]`; escaped pipes and
            brackets inside the target and the alias; empty targets; targets with
            `#`, `|`, `[`, `]`, `\\`, leading and trailing spaces; a wikilink
            inside emphasis, a code span, a highlight, math, a comment and raw
            HTML; doubled and unbalanced brackets""",
        avoid="plain resolvable links with no punctuation",
    ),
    "embed": dict(
        surface="""`![[note]]`, `![[note#Heading]]`, `![[note#^blockid]]`,
            `![[image.png|100]]`, `![[image.png|100x200]]`, `![[a.pdf#page=3]]`;
            sizes that are not numbers, negative and zero sizes, several pipes,
            an alias and a size together; embedding a non-existent file; `!` with
            a space before the brackets; a markdown-style `![](x.png)` beside the
            wikilink form""",
        avoid="simple image embeds with no size or subpath",
    ),
    "emphasis": dict(
        surface="""`*`, `_`, `**`, `__`, `***` runs; intraword emphasis with
            both markers; left- and right-flanking delimiter boundaries against
            punctuation, digits and unicode; four, five and six marker runs;
            emphasis spanning a wikilink, a tag, a code span and math; emphasis
            interleaved with `~~` and `==` so the openers cross""",
        avoid="single well-formed *a* and **a**",
    ),
    "highlight-strike": dict(
        surface="""`==highlight==` and `~~strike~~`: spaced openers, runs of
            three to six markers, markers inside and across words, nesting each
            inside the other and inside emphasis, an unclosed opener at end of
            line, a marker pair spanning a code span or a wikilink""",
        avoid="a single ==a== or ~~a~~ on its own",
    ),
    "tag": dict(
        surface="""`#tag` boundaries: which characters end a tag (`/ - _ . : '
            " ( ) [ ] { } < > & * + = @ ! ? ,`), nested `#a/b/c`, a tag that is
            only digits, a tag starting with a digit, unicode and emoji tags,
            `##double`, `#tag#tag`, a tag right after a wikilink, a code span,
            bold text and a closing bracket, an escaped `\\#`, a tag inside
            frontmatter-looking text""",
        avoid="plain #tag and #nested/tag",
    ),
    "math": dict(
        surface="""`$...$` and `$$...$$`: a space just inside the delimiters,
            digits either side of the `$`, currency amounts that must not become
            math, `$` runs of three or more, inline `$$` mid-paragraph, a block
            `$$` opened and closed on the same line and across lines, math
            containing `[[`, `#`, `_`, `*`, backticks and a newline, an unclosed
            `$`""",
        avoid="a plain $x$ and a plain $$x$$ block",
    ),
    "callout": dict(
        surface="""`> [!note]`, the fold markers `+` and `-`, a custom title
            after the type, an unknown type, an empty type `[!]`, mixed case
            types, a callout nested inside a callout, a callout inside a list
            item, a callout whose first line is empty, a lazy continuation line
            with no `>`, a callout containing a code fence, a table and another
            blockquote, `>` with no space after it""",
        avoid="a single well-formed > [!note] with a body",
    ),
    "list": dict(
        surface="""markers `-`, `*`, `+`, `1.`, `1)`; indent widths of one
            through five spaces and a tab; a marker with no following space;
            checkboxes `[ ]`, `[x]`, `[X]`, `[-]`, `[/]` and an unknown status
            character; a task whose text starts with a wikilink; lazy
            continuation; a list interrupted by a blank line then resumed; an
            ordered list not starting at 1; a nested list under a task""",
        avoid="flat well-formed bullet and numbered lists",
    ),
    "table": dict(
        surface="""a pipe inside a code span, inside math and escaped with
            `\\|` in a cell; a wikilink and an embed in a cell; alignment rows
            with colons in every position; a header row with more or fewer cells
            than the body; a table with one column and no leading pipe; a table
            immediately after a paragraph with no blank line; an empty cell; a
            cell containing `<br>`""",
        avoid="a plain three-column table",
    ),
    "footnote": dict(
        surface="""`[^1]` references and definitions, an inline footnote
            `^[text]`, a named footnote, a definition before its reference, two
            definitions with the same name, a reference with no definition, a
            definition with no reference, a multi-paragraph definition, a
            footnote inside a table cell and inside a callout, nested inline
            footnotes""",
        avoid="one reference with one definition",
    ),
    "code": dict(
        surface="""backtick runs of one to five for code spans, a span whose
            content starts or ends with a backtick or a space, a span containing a
            newline; fenced blocks with tilde fences, a longer closing fence, an
            info string with spaces and braces, an unclosed fence at end of file,
            a fence indented by one to four spaces, a fence inside a list item and
            inside a callout; an indented code block right after a paragraph""",
        avoid="a plain `code` span and a plain ```js fence",
    ),
    "heading": dict(
        surface="""ATX `#` through `#######`, no space after the hashes,
            trailing hashes, a heading containing a wikilink, a tag, emphasis,
            math and a trailing `#tag`; setext underlines of `=` and `-` of
            various lengths, a setext underline under a list item and under a
            blockquote; a heading with only hashes; leading whitespace""",
        avoid="plain # Heading lines",
    ),
    "blockid": dict(
        surface="""`^blockid` at the end of a paragraph, after a trailing
            space, on its own line, after a list item, after a table, after a
            code fence, inside a callout; two ids on one block; an id with
            uppercase, digits, hyphens and underscores; an invalid id with other
            punctuation; `^` with no id""",
        avoid="a single well-formed ^blockid at the end of a paragraph",
    ),
    "html-comment": dict(
        surface="""`%%comment%%` inline and as a block, an unclosed `%%`, `%%`
            runs of four, a comment containing markdown and a wikilink, a comment
            spanning lines; raw inline HTML (`<b>`, `<span style>`, `<br/>`,
            `<script>`, `<iframe>`), a raw HTML block interrupting a paragraph,
            an HTML comment `<!-- -->`, an unclosed tag""",
        avoid="a plain %%comment%%",
    ),
    "frontmatter": dict(
        surface="""a `---` fence not on line 1, a fence with trailing spaces,
            four dashes, a closing fence of different length, an empty block, a
            block containing a list, a nested map, a quoted and an unquoted
            wikilink, a tags key as a string and as a list, duplicate keys, a
            `---` later in the body, CRLF line endings inside the block""",
        avoid="a simple title/tags frontmatter block",
    ),
}

SYSTEM = """You write test fixtures for a Markdown parser. You output only \
Markdown snippets, never explanation, never commentary, never code fences \
around them. Your snippets are deliberately awkward: you are paid to find the \
boundary where two plausible parses disagree."""

PROMPT = """Write {count} short Obsidian-Flavored-Markdown snippets probing \
this syntax area: {area}.

The surface to cover: {surface}

Rules:
- Each snippet is a test case. Make each one probe a DIFFERENT boundary.
- Prefer the ambiguous, the unbalanced and the off-by-one over the well-formed.
  A snippet that every parser agrees on is worthless here.
- Keep each snippet under 200 characters. Most should be one line; use several
  lines only where the construct needs them (a fence, a table, a callout).
- Do not number or label the snippets. Do not wrap them in code fences.
- Invent nothing that must be exact: no plugin names, no settings keys, no
  command ids. Link targets may be any name.
- Avoid these, which the corpus already covers: {avoid}

These lines are the existing probes for this area. Do not repeat them; go \
further than they do:
{covered}

Answer with JSON only, in this shape:
{{"snippets": [{{"snippet": "...", "boundary": "what two parses disagree here"}}]}}
"""

# ---------------------------------------------------------------------------
# The model

# llama-server compiles a JSON schema into a GBNF grammar and samples against
# it, so the answer's shape is enforced rather than asked for -- no "one answer
# in five ignores the format" to work around. A server that does not support
# `json_schema` leaves `--no-schema` to fall back to plain JSON mode.
SNIPPET_SCHEMA = {
    "type": "object",
    "properties": {
        "snippets": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "snippet": {"type": "string"},
                    "boundary": {"type": "string"},
                },
                "required": ["snippet", "boundary"],
            },
        }
    },
    "required": ["snippets"],
}


def model_call(
    prompt, system=SYSTEM, temperature=0.9, max_tokens=4096, timeout=600, schema=True
):
    """One blocking chat completion against the local server."""
    response_format = (
        {
            "type": "json_schema",
            "json_schema": {"name": "probe_snippets", "strict": True,
                            "schema": SNIPPET_SCHEMA},
        }
        if schema
        else {"type": "json_object"}
    )
    body = json.dumps(
        {
            "model": LLM_MODEL,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": prompt},
            ],
            "stream": False,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "response_format": response_format,
            # The `-code` presets have thinking off already; this covers a run
            # that names a general preset, which would otherwise spend most of
            # its budget deliberating.
            "chat_template_kwargs": {"enable_thinking": False},
        }
    ).encode()
    headers = {"Content-Type": "application/json"}
    if LLM_KEY:
        headers["Authorization"] = f"Bearer {LLM_KEY}"
    request = urllib.request.Request(
        f"{LLM_URL}/chat/completions", data=body, headers=headers
    )
    started = time.time()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            answer = json.load(response)
    except urllib.error.HTTPError as e:
        die(f"{LLM_URL} answered {e.code}: {e.read().decode(errors='replace')[:400]}")
    except urllib.error.URLError as e:
        die(
            f"cannot reach {LLM_URL}: {e.reason}. On misco check "
            f"`systemctl status llama-cpp`; from another machine SILICA_LLM_URL "
            f"should be http://misco.tail.ragtux.com:8080/v1."
        )
    text = answer["choices"][0]["message"]["content"]
    usage = answer.get("usage", {})
    elapsed = time.time() - started
    note(
        f"{answer.get('model', LLM_MODEL)}: "
        f"{usage.get('prompt_tokens', 0)} + {usage.get('completion_tokens', 0)} tokens, "
        f"{elapsed:.1f} s"
    )
    return strip_thought(text)


def strip_thought(text):
    """Drop a leading <think> block, which the Qwen3 family emits inline."""
    stripped = text.lstrip()
    if not stripped.startswith("<think>"):
        return text.strip()
    end = stripped.find("</think>")
    if end < 0:
        die("the model spent its whole budget in a <think> block; raise --max-tokens")
    return stripped[end + len("</think>") :].strip()


def parse_snippets(text):
    """Pull snippets out of the answer, tolerating a model that ignored the
    JSON shape -- which an 8B model does perhaps one time in five."""
    start = min(
        (i for i in (text.find("{"), text.find("[")) if i >= 0),
        default=-1,
    )
    if start >= 0:
        for end in range(len(text), start, -1):
            try:
                value = json.loads(text[start:end])
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict):
                value = value.get("snippets", [])
            out = []
            for item in value if isinstance(value, list) else []:
                if isinstance(item, str):
                    out.append((item, ""))
                elif isinstance(item, dict) and item.get("snippet"):
                    out.append((str(item["snippet"]), str(item.get("boundary", ""))))
            if out:
                return out
            break
    # Fall back to line scraping: strip bullets, numbers and stray fences.
    note("the answer was not JSON; scraping lines instead")
    out = []
    for line in text.splitlines():
        line = re.sub(r"^\s*(?:[-*+]|\d+[.)])\s+", "", line).strip()
        if line.startswith("```") or not line:
            continue
        out.append((line, ""))
    return out


# ---------------------------------------------------------------------------
# Validation

MARKDOWN_SIGNIFICANT = set("*_~=`#[]()|>$^!\\<&%-:\"'")


def normalize(snippet):
    return re.sub(r"\s+", " ", snippet).strip()


def existing_lines():
    """Every line already in the corpus's edge probes, normalized."""
    lines = set()
    for path in sorted(EDGE.glob("*.md")):
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            line = normalize(line)
            if line:
                lines.add(line)
    return lines


def covered_sample(area, limit=2500):
    """A sample of the existing probes, for the prompt. Truthful to the repo:
    read off disk rather than hardcoded, so it tracks what is really there."""
    keyword = area.split("-")[0]
    hits, rest = [], []
    for path in sorted(EDGE.glob("*.md")):
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            (hits if keyword in line.lower() else rest).append(line)
    chosen, total = [], 0
    for line in hits + rest:
        if total + len(line) > limit:
            break
        chosen.append(line)
        total += len(line)
    return "\n".join(chosen) if chosen else "(none yet)"


def validate(candidates, known, max_len=400, max_lines=12):
    """Keep what is worth rendering. Every rejection is counted and reported:
    a generator whose output is 80% rejected needs a better prompt, and that
    is only visible if the rejections are not silent."""
    kept, reasons = [], {}
    seen = set()

    def reject(why):
        reasons[why] = reasons.get(why, 0) + 1

    for snippet, boundary in candidates:
        snippet = snippet.replace("\r\n", "\n").replace("\r", "\n").strip("\n")
        flat = normalize(snippet)
        if not flat:
            reject("empty")
        elif len(snippet) > max_len:
            reject(f"longer than {max_len} characters")
        elif snippet.count("\n") + 1 > max_lines:
            reject(f"more than {max_lines} lines")
        elif not any(c in MARKDOWN_SIGNIFICANT for c in snippet):
            reject("prose: no markdown-significant character")
        elif flat in seen:
            reject("duplicate within this batch")
        elif flat in known:
            reject("already a line in the corpus probes")
        elif len(flat) >= 12 and any(flat in line for line in known):
            # Short snippets are substrings of everything, so containment is
            # only trusted for ones long enough to be meant.
            reject("already inside a corpus probe line")
        else:
            seen.add(flat)
            kept.append((snippet, boundary))
            continue
    return kept, reasons


# ---------------------------------------------------------------------------
# Staging

def stage_vault(staging, notes, area, model):
    """Lay out `vault/` + `expected/`, the pair the corpus plugin writes and
    the corpus test reads. One snippet per note, so a failing note *is* a
    failing snippet and triage needs no bisection."""
    vault = staging / "vault"
    if vault.exists():
        shutil.rmtree(vault)
    (staging / "expected").mkdir(parents=True, exist_ok=True)
    for stale in ("expected/.done", "expected/render-summary.json"):
        (staging / stale).unlink(missing_ok=True)
    (vault / "probe").mkdir(parents=True)

    # The plugin must be present and trusted, so copy the corpus vault's own
    # `.obsidian` -- but not its workspace.json, whose layout names notes this
    # vault does not have.
    source = CORPUS / "vault/.obsidian"
    shutil.copytree(
        source,
        vault / ".obsidian",
        ignore=shutil.ignore_patterns("workspace.json", "workspace-mobile.json"),
    )
    # Render on startup and close the window when done.
    (vault / ".silica-autorun").write_text("")

    index = {"area": area, "model": model, "when": int(time.time()), "notes": {}}
    for i, (snippet, boundary) in enumerate(notes, start=1):
        name = f"{i:04d}"
        # The snippet is the whole note: no heading, no wrapper, nothing that
        # could itself be the thing that disagrees.
        (vault / "probe" / f"{name}.md").write_text(snippet + "\n", encoding="utf-8")
        index["notes"][f"probe/{name}.md"] = {
            "id": name,
            "snippet": snippet,
            "boundary": boundary,
        }
    (staging / "index.json").write_text(json.dumps(index, indent=2) + "\n")
    return index


def load_index(staging):
    path = staging / "index.json"
    if not path.exists():
        die(f"no index at {path}; run `generate` first")
    return json.loads(path.read_text())


# ---------------------------------------------------------------------------
# Commands

def cmd_areas(args):
    width = max(len(name) for name in AREAS)
    for name, area in AREAS.items():
        surface = re.sub(r"\s+", " ", area["surface"])
        print(f"{name:<{width}}  {surface[:100]}...")


def cmd_generate(args):
    staging = args.staging
    known = existing_lines()
    note(f"{len(known)} lines already in {EDGE.relative_to(REPO)}")
    area = AREAS[args.area]
    prompt = PROMPT.format(
        count=args.count,
        area=args.area,
        surface=re.sub(r"\s+", " ", area["surface"]),
        avoid=area["avoid"],
        covered=covered_sample(args.area),
    )
    if args.show_prompt:
        print(prompt)
        return
    # Ask in rounds: a small model asked for 60 snippets gives 20 and repeats
    # itself, while three rounds at a high temperature explore more ground.
    candidates = []
    for round_number in range(1, args.rounds + 1):
        note(f"round {round_number} of {args.rounds}")
        text = model_call(
            prompt,
            temperature=args.temperature,
            max_tokens=args.max_tokens,
            schema=not args.no_schema,
        )
        got = parse_snippets(text)
        note(f"  {len(got)} candidates")
        candidates.extend(got)

    kept, reasons = validate(candidates, known)
    for why, count in sorted(reasons.items(), key=lambda kv: -kv[1]):
        note(f"dropped {count}: {why}")
    note(f"{len(kept)} of {len(candidates)} candidates kept")
    if not kept:
        die("nothing survived validation; try --rounds 3 or a bigger model")
    if args.dry_run:
        for i, (snippet, boundary) in enumerate(kept, start=1):
            print(f"--- {i:04d} {boundary}")
            print(snippet)
        return

    stage_vault(staging, kept, args.area, LLM_MODEL)
    print(f"\n{len(kept)} candidate notes staged in {staging}/vault/probe")
    print(f"next: {me()} render --staging {staging}")


def cmd_render(args):
    staging = args.staging
    vault = staging / "vault"
    if not vault.exists():
        die(f"no staged vault at {vault}; run `generate` first")
    guard_real_vault(vault)
    expected = staging / "expected"
    done = expected / ".done"
    done.unlink(missing_ok=True)
    (expected / "render-summary.json").unlink(missing_ok=True)

    if args.manual:
        print(f"Open this vault in Obsidian and trust its plugin:\n  {vault}")
        print("It renders on startup and closes itself. Then:")
        print(f"  {me()} triage --staging {staging}")
        return

    if obsidian_running():
        die(
            "Obsidian is already running; stop it first. This step swaps "
            "~/.config/obsidian/obsidian.json and must not touch your session."
        )
    config = Path.home() / ".config/obsidian/obsidian.json"
    if not config.exists():
        die(f"{config} does not exist; open Obsidian once, or use --manual")
    backup = config.read_bytes()
    log = Path(tempfile.mkstemp(prefix="probe-gen-obsidian-", suffix=".log")[1])
    try:
        for attempt in (1, 2):
            # A previous instance lingers for seconds after its window closes,
            # and a launch during that window is handed to the dying instance,
            # which then opens *its* vault. Wait for it to go, as
            # tools/obsidian-probe/run-block.sh does.
            for _ in range(40):
                if not obsidian_running():
                    break
                time.sleep(0.5)
            time.sleep(2)
            register(config, vault)
            note(f"launching Obsidian (attempt {attempt}); it renders and closes")
            with log.open("ab") as handle:
                subprocess.Popen(
                    ["obsidian"], stdout=handle, stderr=handle, start_new_session=True
                )
            deadline = time.time() + args.timeout
            while time.time() < deadline and not done.exists():
                time.sleep(0.5)
            if done.exists():
                break
            note(f"no {done.name} after {args.timeout} s")
            if obsidian_running():
                subprocess.run(["pkill", "-f", PGREP], check=False)
        if not done.exists():
            note(f"Obsidian's own output is in {log}")
            die(
                "the render never finished. Two usual causes: Obsidian is "
                "asking to trust the plugin (run with --manual and click "
                "through it once), or a stale instance held the single-instance "
                "lock."
            )
        summary = json.loads((expected / "render-summary.json").read_text())
        note(f"rendered {summary['rendered']} notes in {summary['millis']} ms")
        for error in summary.get("errors", []):
            note(f"  render error: {error}")
    finally:
        if obsidian_running():
            subprocess.run(["pkill", "-f", PGREP], check=False)
            time.sleep(2)
        config.write_bytes(backup)
        note("obsidian.json restored")
        log.unlink(missing_ok=True)
    print(f"next: {me()} triage --staging {staging}")


def cmd_triage(args):
    staging = args.staging
    index = load_index(staging)
    expected = staging / "expected"
    if not (expected / ".done").exists() and not args.force:
        die(f"{expected} was never rendered; run `render` first (or --force)")

    dump = staging / "dump"
    if dump.exists():
        shutil.rmtree(dump)
    # The comparator is the corpus test's own: canonicalizing Obsidian's HTML
    # is subtle (MathJax, copy buttons, dir="auto"), and a second
    # implementation here would disagree with the gate it is meant to predict.
    note("running the corpus comparator over the staged notes")
    result = subprocess.run(
        [
            "cargo", "test", "-q", "-p", "silica-markdown",
            "--test", "corpus", "--", "--nocapture",
        ],
        cwd=REPO,
        env={**os.environ, "CORPUS_DIR": str(staging), "CORPUS_DUMP": str(dump)},
        capture_output=True,
        text=True,
    )
    output = (result.stdout + result.stderr).strip()
    tail = output.splitlines()
    counted = re.search(r"^corpus: (\d+)/(\d+) passing", output, re.M)
    if counted:
        note(counted.group(0))
    else:
        print("\n".join(tail[-25:]), file=sys.stderr)
        die("the comparator did not report a count; the test above did not run")
    # If corpus.rs ignores CORPUS_DIR the test silently grades the committed
    # corpus instead, reports its ~1009 notes, and every finding below would be
    # about the wrong vault. The note count is what gives that away.
    staged = len(index["notes"])
    if int(counted.group(2)) != staged:
        die(
            f"the comparator saw {counted.group(2)} notes, not the {staged} staged "
            f"ones: CORPUS_DIR is not being honored. Check that corpus_dir() in "
            f"crates/silica-markdown/tests/corpus.rs reads it."
        )

    # A failing note leaves `<dump>/<rel>.want` and `.got`, so the dump is the
    # failure list -- more reliable than parsing the test's console output.
    disagree, unrendered = [], []
    for rel, entry in sorted(index["notes"].items()):
        want = dump / rel.replace(".md", ".want")
        if not (expected / rel.replace(".md", ".html")).exists():
            unrendered.append(entry)
        elif want.exists():
            got = want.with_suffix(".got")
            entry = dict(entry, diff=first_difference(want, got))
            disagree.append(entry)

    report = {
        "area": index["area"],
        "model": index["model"],
        "total": len(index["notes"]),
        "disagree": disagree,
        "unrendered": [e["id"] for e in unrendered],
    }
    (staging / "triage.json").write_text(json.dumps(report, indent=2) + "\n")

    print()
    if unrendered:
        print(f"{len(unrendered)} notes have no rendered HTML: "
              f"{', '.join(e['id'] for e in unrendered[:10])}")
    print(f"{len(disagree)} of {len(index['notes'])} snippets disagree with Obsidian\n")
    for entry in disagree:
        print(f"--- {entry['id']}" + (f"  ({entry['boundary']})" if entry["boundary"] else ""))
        for line in entry["snippet"].splitlines():
            print(f"    {line}")
        for line in entry["diff"]:
            print(f"      {line}")
        print()
    if disagree:
        ids = ",".join(e["id"] for e in disagree[:8])
        print("Each of these is a real difference between Obsidian and the parser,")
        print("or a snippet whose rendering the comparator cannot canonicalize.")
        print("Read them before promoting any: that judgment is not the model's.")
        print(f"\n  {me()} promote --staging {staging} "
              f"--note \"Probe {index['area'].title()}\" --only {ids}")


def first_difference(want, got, context=3):
    """The first place the two canonical token streams part, as a few lines."""
    want_lines = want.read_text(errors="replace").splitlines()
    got_lines = got.read_text(errors="replace").splitlines()
    diff = difflib.unified_diff(
        want_lines, got_lines, fromfile="obsidian", tofile="silica", n=1, lineterm=""
    )
    out = [line for line in diff if not line.startswith("@@")]
    return out[: context * 2 + 2] or ["(identical canonical forms; the note failed for another reason)"]


def cmd_promote(args):
    staging = args.staging
    index = load_index(staging)
    wanted = [w.strip() for w in args.only.split(",") if w.strip()]
    by_id = {entry["id"]: entry for entry in index["notes"].values()}
    missing = [w for w in wanted if w not in by_id]
    if missing:
        die(f"no such candidate: {', '.join(missing)}")

    target = EDGE / f"{args.note}.md"
    heading = f"## {index['area']} probes, {time.strftime('%Y-%m-%d')}"
    body = [heading, ""]
    for id_ in wanted:
        entry = by_id[id_]
        if entry["boundary"]:
            body.append(f"%% {entry['boundary']} %%")
        body.append(entry["snippet"])
        body.append("")

    if target.exists():
        existing = target.read_text(encoding="utf-8")
        text = existing.rstrip("\n") + "\n\n" + "\n".join(body)
    else:
        text = f"# {args.note}\n\n" + "\n".join(body)
    if args.dry_run:
        print(text if not target.exists() else "\n".join(body))
        return
    target.write_text(text, encoding="utf-8")
    print(f"wrote {len(wanted)} snippets to {target.relative_to(REPO)}")
    print()
    print("Three things remain, and none of them are automatic:")
    print("  1. Re-render the real corpus vault in Obsidian (its 'Render corpus'")
    print("     command) so expected/ covers the new note. Until then the test")
    print("     reports it as `missing expected HTML` and silently gates nothing.")
    print("  2. Decide what Obsidian's behavior IS, and write it into")
    print("     docs/silica/Specification.md. The snippet is evidence, not a rule.")
    print("  3. Fix the parser, then raise MIN_PASSING in tests/corpus.rs.")


# ---------------------------------------------------------------------------
# Helpers

def obsidian_running():
    return subprocess.run(["pgrep", "-f", PGREP], capture_output=True).returncode == 0


def register(config, vault):
    """Point Obsidian at the staging vault under a fresh 16-hex id. Reusing an
    id across folders inherits that id's IndexedDB cache; a malformed id is
    ignored and no window opens."""
    data = json.loads(config.read_text())
    for entry in data.get("vaults", {}).values():
        entry.pop("open", None)
    data.setdefault("vaults", {})[secrets.token_hex(8)] = {
        "path": str(vault),
        "ts": int(time.time() * 1000),
        "open": True,
    }
    config.write_text(json.dumps(data))


def guard_real_vault(vault):
    """Never render into a vault somebody cares about."""
    resolved = vault.resolve()
    for forbidden in (
        Path.home() / "bstore2/raga/plmcrzy",
        REPO / "docs/silica",
        REPO / "docs/silica-demo",
        CORPUS / "vault",
    ):
        if resolved == forbidden.resolve() or forbidden.resolve() in resolved.parents:
            die(f"{resolved} is inside {forbidden}; stage somewhere disposable")


def me():
    return "tools/probe-gen/probe_gen.py"


def note(message):
    print(f"[probe-gen] {message}", file=sys.stderr)


def die(message):
    print(f"[probe-gen] {message}", file=sys.stderr)
    raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--staging", type=Path, default=DEFAULT_STAGING,
        help=f"throwaway vault+expected pair (default {DEFAULT_STAGING})",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("areas", help="list the syntax areas").set_defaults(run=cmd_areas)

    generate = sub.add_parser("generate", help="ask the local model for candidates")
    generate.add_argument("--area", required=True, choices=sorted(AREAS))
    generate.add_argument("--count", type=int, default=25, help="snippets per round")
    generate.add_argument("--rounds", type=int, default=2)
    generate.add_argument("--temperature", type=float, default=0.9)
    generate.add_argument("--max-tokens", type=int, default=4096)
    generate.add_argument("--dry-run", action="store_true", help="print, do not stage")
    generate.add_argument("--show-prompt", action="store_true")
    generate.add_argument("--no-schema", action="store_true",
                          help="plain JSON mode instead of a grammar, for a "
                               "server without json_schema support")
    generate.set_defaults(run=cmd_generate)

    render = sub.add_parser("render", help="render the staged vault in real Obsidian")
    render.add_argument("--timeout", type=int, default=300)
    render.add_argument("--manual", action="store_true",
                        help="print what to do by hand instead of driving Obsidian")
    render.set_defaults(run=cmd_render)

    triage = sub.add_parser("triage", help="report which staged snippets disagree")
    triage.add_argument("--force", action="store_true",
                        help="triage even though .done is missing")
    triage.set_defaults(run=cmd_triage)

    promote = sub.add_parser("promote", help="append chosen snippets to an edge probe")
    promote.add_argument("--note", required=True, help='e.g. "Probe Callouts"')
    promote.add_argument("--only", required=True, help="candidate ids, comma-separated")
    promote.add_argument("--dry-run", action="store_true")
    promote.set_defaults(run=cmd_promote)

    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
