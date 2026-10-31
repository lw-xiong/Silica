# Probe generator

Candidate OFM probe notes from a local model, rendered by a real Obsidian,
triaged against `silica-markdown`.

This automates the front half of the workflow in `CLAUDE.md`: when Obsidian and
the parser disagree, a probe note goes into
`crates/silica-markdown/tests/corpus/vault/edge`, the vault is re-rendered, and
the observed behavior is matched and written into
`docs/silica/Specification.md`. The tedious part is *thinking of* the snippets,
and that part suits a small local model — because **Obsidian is the oracle**, a
weak snippet is only a boring test, never a wrong answer. Nothing the model
writes is trusted as a fact; it is trusted only as a thing to try.

The model never names anything that must be exact. Command ids, config keys and
file names come from observation (`silica-commands/data/commands.txt`,
`silica-formats/src/config.rs`), and a small model will produce a plausible
wrong string with no mark on it. The prompt forbids inventing them and the
areas below stay inside Markdown syntax for the same reason.

## The loop

```
tools/probe-gen/probe_gen.py areas                              # the syntax areas
tools/probe-gen/probe_gen.py generate --area callout --rounds 3 # candidates -> staging
tools/probe-gen/probe_gen.py render                             # real Obsidian renders them
tools/probe-gen/probe_gen.py triage                             # which ones disagree
tools/probe-gen/probe_gen.py promote --note "Probe Callouts" --only 0003,0019
```

`generate` asks in rounds, because one request for 60 snippets gets 20 and
repeats itself while three rounds at a high temperature cover more ground. Every
candidate is then validated — empty, over-long, prose with no Markdown in it,
duplicated within the batch, or already a line in the committed probes — and
each rejection is counted and printed, because a prompt whose output is 80%
rejected needs fixing and that is only visible if the drops are not silent.

Candidates are staged **one snippet per note**, in their own throwaway vault:

```
$PROBE_STAGING/            default /tmp/silica-probe-staging
  vault/
    .obsidian/             copied from the corpus vault, so the plugin is trusted
    .silica-autorun        the plugin renders on startup, then closes the window
    probe/0001.md …        one snippet each, nothing wrapped around it
  expected/                written by the plugin
  index.json               note -> snippet, boundary, area, model
  triage.json              the last triage's findings
```

One snippet per note is the point: a failing note *is* a failing snippet, so
triage needs no bisection. The committed corpus is never written to until
`promote`.

`triage` runs the corpus test's own comparator over the staging pair via
`CORPUS_DIR`, and reads the failures out of the `CORPUS_DUMP` directory rather
than parsing console output. Canonicalizing Obsidian's HTML is subtle — MathJax
containers, copy buttons, `dir="auto"` — and a second implementation here would
disagree with the gate it exists to predict.

`promote` appends the snippets you choose to an `edge/` note and then stops,
because the rest is editorial. It prints the three remaining steps: re-render
the real corpus vault so `expected/` covers the new note (until then the test
reports `missing expected HTML` and the probe silently gates nothing), decide
what Obsidian's behavior *is* and write it into the specification, then fix the
parser and raise `MIN_PASSING`.

## Configuration

The same environment variables as the `silica-llm-mcp` MCP server, so one place
configures both:

```
SILICA_LLM_URL     default http://127.0.0.1:8080/v1
SILICA_LLM_MODEL   default qwen3.6-35b-a3b-code
SILICA_LLM_KEY     bearer token, if the server wants one
PROBE_STAGING      default /tmp/silica-probe-staging
```

The model is misco's `llama-server` router (`hosts/misco/llm.nix` in the nix
flake), which serves five presets on one OpenAI-compatible endpoint. From any
other machine on the tailnet, point `SILICA_LLM_URL` at
`http://misco.tail.ragtux.com:8080/v1`.

**The default preset is deliberate.** The general presets (`qwen3.8-27b`,
`qwen3.6-35b-a3b`) set `presence-penalty = 1.5`, which is right for prose and
exactly wrong here: a syntax probe is *meant* to repeat itself (`==a==b ==
spaced == ==== four`), and penalising repeated tokens steers the model off the
snippets worth having. The `-code` presets set it to 0 and have thinking off,
so the answer arrives in one pass. `qwen3.6-35b-a3b-code` generates 18 usable
callout probes in about 11 s; `qwen3-coder-next` is the stronger model but
49 GB, so it is worth naming only if the smaller one is visibly weak on an
area.

Output shape is **grammar-enforced**: the request carries a JSON schema, which
llama-server compiles to GBNF and samples against, so a malformed answer is not
a failure mode. `--no-schema` falls back to plain JSON mode for a server
without `json_schema`.

## Traps

- **The render step needs your Obsidian closed.** It swaps
  `~/.config/obsidian/obsidian.json` to register the staging vault and restores
  it byte-identical on exit, the same dance as
  `tools/obsidian-probe/run-block.sh`; it refuses to start while an Obsidian is
  running, so it can never close yours. `--manual` prints what to do by hand
  instead, which is also how you get past a first-run "trust this plugin"
  prompt.
- `guard_real_vault` refuses to stage inside the gate vault, `docs/silica`,
  `docs/silica-demo` or the committed corpus. Keep it that way.
- **One resident model.** The router runs `--models-max 1`, so naming a
  different preset mid-pipeline evicts the loaded one and costs a 10-30 s
  reload. Pick one model per run. After 30 minutes idle
  (`--sleep-idle-seconds 1800`) the first request pays that reload too — a
  slow first call is the model waking, not a hang.
- VRAM mostly takes care of itself here: `--fit` measures free memory per
  device at load time, `fit-target = 2048` leaves 2 GiB per GPU for the
  compositor, and idle sleep gives the weights back. The thing to remember is
  that only 2e:00.0 is available to it — 02:00.0 is pinned to vfio-pci for the
  W11 VM — so a model fitted while the VM was down will not re-fit on wake.
- The model emits `<think>` blocks when a *general* preset is named; they are
  stripped, and the request also sends `enable_thinking: false`. The `-code`
  presets have `reasoning = off` already.
- A model repeats itself across rounds. That is what the dedupe is for; a high
  duplicate count is the signal to raise `--temperature`, not a bug.
