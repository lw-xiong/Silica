//! Spike benchmark for the metadata index.
//!
//! The spec's first-phase gate reads: "vault index of a 20k-file vault under
//! 2 seconds cold". This binary has two subcommands:
//!
//! - `generate <dir>` writes a synthetic vault with realistic note structure
//!   (folders, frontmatter, headings, wikilinks of every form, tags, callouts,
//!   code fences, block ids, embeds of attachments, duplicate basenames).
//! - `index <vault>` times every stage of building the cache for a vault:
//!   scanning the folder, reading the notes, indexing them, and resolving every
//!   link. It reports per-stage times and whether the gate passed.
//!
//! The generator is deterministic for a given seed so numbers are comparable
//! between machines and commits.

use rayon::prelude::*;
use silica_cache::{index_markdown, CachedMetadata, LinkResolver};
use silica_vault::Vault;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

const USAGE: &str = "\
silica-bench: spike benchmarks for the metadata index

  silica-bench generate <dir> [--notes N] [--seed S] [--jobs J]
      Write a synthetic vault into <dir> (created if missing, must be empty).
      --notes  number of markdown notes to write (default 20000)
      --seed   PRNG seed (default 1)

  silica-bench index <vault> [--runs R] [--jobs J]
      Time scan, read, index, and resolve stages for an existing vault.
      --runs   repetitions (default 3); only the first can be cold
      --jobs   worker threads for read/index/resolve (default: all cores)

Run 1 is only cold if the OS page cache was evicted first, for example with
`sync; echo 3 | sudo tee /proc/sys/vm/drop_caches` on Linux.";

const GATE: Duration = Duration::from_secs(2);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("generate") => Opts::parse(&args[1..]).and_then(|o| cmd_generate(&o)),
        Some("index") => Opts::parse(&args[1..]).and_then(|o| cmd_index(&o)),
        Some("--help" | "-h") | None => {
            println!("{USAGE}");
            return;
        }
        Some(other) => Err(format!("unknown subcommand `{other}`\n\n{USAGE}")),
    };
    if let Err(msg) = result {
        eprintln!("error: {msg}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Argument parsing (deliberately dependency-free)

struct Opts {
    positional: Vec<String>,
    flags: BTreeMap<String, String>,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Opts, String> {
        let mut opts = Opts {
            positional: Vec::new(),
            flags: BTreeMap::new(),
        };
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if let Some(flag) = a.strip_prefix("--") {
                let (key, value) = match flag.split_once('=') {
                    Some((k, v)) => (k.to_string(), v.to_string()),
                    None => {
                        i += 1;
                        let v = args
                            .get(i)
                            .ok_or_else(|| format!("--{flag} needs a value"))?;
                        (flag.to_string(), v.clone())
                    }
                };
                opts.flags.insert(key, value);
            } else {
                opts.positional.push(a.clone());
            }
            i += 1;
        }
        Ok(opts)
    }

    fn path(&self, what: &str) -> Result<PathBuf, String> {
        self.positional
            .first()
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing <{what}> argument\n\n{USAGE}"))
    }

    fn flag<T: FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.flags.get(name) {
            None => Ok(default),
            Some(v) => v
                .parse()
                .map_err(|_| format!("--{name}: `{v}` is not a valid value")),
        }
    }

    /// Builds the global rayon pool from `--jobs`; 0 or absent means all cores.
    fn configure_jobs(&self) -> Result<(), String> {
        let jobs: usize = self.flag("jobs", 0)?;
        if jobs > 0 {
            rayon::ThreadPoolBuilder::new()
                .num_threads(jobs)
                .build_global()
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Synthetic vault generator

/// xorshift64*: small, deterministic, good enough for shaping test data.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const WORDS: &[&str] = &[
    "vault",
    "note",
    "graph",
    "link",
    "heading",
    "block",
    "canvas",
    "base",
    "plugin",
    "theme",
    "editor",
    "preview",
    "search",
    "backlink",
    "outline",
    "tag",
    "alias",
    "property",
    "daily",
    "template",
    "folder",
    "file",
    "sync",
    "publish",
    "render",
    "parser",
    "cache",
    "index",
    "query",
    "filter",
    "sort",
    "table",
    "card",
    "list",
    "kanban",
    "embed",
    "image",
    "audio",
    "video",
    "pdf",
    "math",
    "diagram",
    "callout",
    "footnote",
    "comment",
    "highlight",
    "task",
    "checkbox",
    "quote",
    "code",
    "fence",
    "rope",
    "cursor",
    "selection",
    "fold",
    "scroll",
    "pane",
    "tab",
    "split",
    "ribbon",
    "sidebar",
    "modal",
    "palette",
    "switcher",
    "hotkey",
    "command",
    "setting",
];

const AREAS: &[&str] = &[
    "Areas",
    "Projects",
    "Resources",
    "Archive",
    "People",
    "Meetings",
    "Reading",
    "Ideas",
];

const CALLOUTS: &[&str] = &[
    "note", "tip", "warning", "info", "quote", "example", "abstract", "question",
];

fn title_case(w: impl AsRef<str>) -> String {
    let mut c = w.as_ref().chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// A planned note: where it lives and what it is called, before content exists.
struct Plan {
    path: String,
    title: String,
    headings: Vec<String>,
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

fn plan_notes(rng: &mut Rng, notes: usize) -> Vec<Plan> {
    let folder_count = (notes / 200).clamp(4, 400);
    let mut folders = Vec::with_capacity(folder_count);
    for _ in 0..folder_count {
        let area = rng.pick(AREAS);
        let depth = rng.range(1, 3);
        let mut f = area.to_string();
        for _ in 1..depth {
            let w = title_case(rng.pick(WORDS));
            let n = rng.below(40);
            let _ = write!(f, "/{w} {n}");
        }
        folders.push(f);
    }
    folders.sort();
    folders.dedup();

    let daily = notes / 7;
    let mut plans = Vec::with_capacity(notes);
    // Each folder starts with an `Index.md` so basenames collide across folders,
    // the case that exercises Obsidian's shortest-path tie-break.
    for f in &folders {
        if plans.len() >= notes {
            break;
        }
        plans.push(Plan {
            path: format!("{f}/Index.md"),
            title: "Index".to_string(),
            headings: vec!["Overview".to_string(), "Links".to_string()],
        });
    }
    let mut day = 0u32;
    while plans.len() < notes {
        let i = plans.len();
        if i % 7 == 0 && day < daily as u32 {
            let year = 2021 + day / 365;
            let doy = day % 365;
            let month = doy / 31 + 1;
            let dom = doy % 31 + 1;
            let name = format!("{year}-{month:02}-{dom:02}");
            plans.push(Plan {
                path: format!("Daily/{year}/{month:02}/{name}.md"),
                title: name,
                headings: vec!["Log".to_string(), "Tasks".to_string()],
            });
            day += 1;
            continue;
        }
        let folder = rng.pick(&folders);
        let a = title_case(rng.pick(WORDS));
        let b = title_case(rng.pick(WORDS));
        let title = format!("{a} {b} {i}");
        let hcount = rng.range(1, 4);
        let headings = (0..hcount)
            .map(|_| {
                let w = title_case(rng.pick(WORDS));
                format!("{w} {}", rng.below(100))
            })
            .collect();
        plans.push(Plan {
            path: format!("{folder}/{title}.md"),
            title,
            headings,
        });
    }
    plans
}

fn sentence(rng: &mut Rng, min_words: usize, max_words: usize) -> String {
    let words = rng.range(min_words, max_words);
    let mut s = String::new();
    for k in 0..words {
        let w = rng.pick(WORDS);
        if k == 0 {
            s.push_str(&title_case(w));
        } else {
            s.push(' ');
            s.push_str(w);
        }
    }
    s.push('.');
    s
}

fn wikilink(rng: &mut Rng, plans: &[Plan], target: usize) -> String {
    let t = &plans[target];
    let name = stem(&t.path);
    match rng.below(100) {
        0..=4 => format!("[[Missing Note {}]]", rng.below(1_000_000)),
        5..=9 => format!("[[{}]]", t.path.trim_end_matches(".md")),
        10..=19 => format!("[[{}|{}]]", name, rng.pick(WORDS)),
        20..=29 => format!("[[{}#{}]]", name, rng.pick(&t.headings)),
        30..=32 => format!("[[{}#^{}{}]]", name, rng.pick(WORDS), rng.below(1000)),
        _ => format!("[[{name}]]"),
    }
}

fn any_link(rng: &mut Rng, plans: &[Plan]) -> String {
    let target = rng.below(plans.len());
    wikilink(rng, plans, target)
}

fn note_body(rng: &mut Rng, plans: &[Plan], me: usize, attachments: usize) -> String {
    let plan = &plans[me];
    let mut s = String::with_capacity(2500);

    // Frontmatter: tags as a list, sometimes aliases, a date, a link-valued property.
    s.push_str("---\n");
    let _ = writeln!(s, "title: {}", plan.title);
    s.push_str("tags:\n");
    for _ in 0..rng.range(1, 3) {
        let _ = writeln!(s, "  - {}/{}", rng.pick(WORDS), rng.pick(WORDS));
    }
    if rng.chance(40) {
        let _ = writeln!(
            s,
            "aliases:\n  - {} {}",
            title_case(rng.pick(WORDS)),
            title_case(rng.pick(WORDS))
        );
    }
    let _ = writeln!(
        s,
        "created: 20{:02}-{:02}-{:02}",
        rng.range(19, 25),
        rng.range(1, 12),
        rng.range(1, 28)
    );
    if rng.chance(30) {
        let _ = writeln!(
            s,
            "related: \"[[{}]]\"",
            stem(&plans[rng.below(plans.len())].path)
        );
    }
    s.push_str("---\n");

    let _ = writeln!(s, "# {}\n", plan.title);

    for (hi, heading) in plan.headings.iter().enumerate() {
        if hi > 0 {
            let _ = writeln!(s, "## {heading}\n");
        }
        for _ in 0..rng.range(1, 3) {
            let mut para = String::new();
            for _ in 0..rng.range(2, 5) {
                if !para.is_empty() {
                    para.push(' ');
                }
                para.push_str(&sentence(rng, 6, 16));
                if rng.chance(45) {
                    let _ = write!(para, " See {}.", any_link(rng, plans));
                }
                if rng.chance(15) {
                    let _ = write!(para, " #{}", rng.pick(WORDS));
                }
                if rng.chance(8) {
                    let _ = write!(para, " =={}==", rng.pick(WORDS));
                }
            }
            if rng.chance(10) {
                let _ = write!(para, " ^{}{}", rng.pick(WORDS), rng.below(1000));
            }
            s.push_str(&para);
            s.push_str("\n\n");
        }
        match rng.below(10) {
            0 | 1 => {
                for _ in 0..rng.range(2, 5) {
                    let done = if rng.chance(50) { "x" } else { " " };
                    let _ = writeln!(
                        s,
                        "- [{done}] {} {}",
                        sentence(rng, 3, 7),
                        any_link(rng, plans)
                    );
                }
                s.push('\n');
            }
            2 => {
                let fold = ["", "-", "+"][rng.below(3)];
                let _ = writeln!(
                    s,
                    "> [!{}]{fold} {}\n> {}\n",
                    rng.pick(CALLOUTS),
                    title_case(rng.pick(WORDS)),
                    sentence(rng, 9, 9)
                );
            }
            3 => {
                let _ = writeln!(
                    s,
                    "```rust\n// [[not a link]] #nottag\nfn {}() {{}}\n```\n",
                    rng.pick(WORDS)
                );
            }
            4 if attachments > 0 => {
                let _ = writeln!(s, "![[img-{}.png]]\n", rng.below(attachments));
            }
            5 => {
                let _ = writeln!(
                    s,
                    "| {} | {} |\n| --- | --- |\n| {} | {} |\n",
                    title_case(rng.pick(WORDS)),
                    title_case(rng.pick(WORDS)),
                    any_link(rng, plans),
                    rng.pick(WORDS)
                );
            }
            6 => {
                let _ = writeln!(s, "![[{}]]\n", stem(&plans[rng.below(plans.len())].path));
            }
            _ => {}
        }
    }
    s
}

fn cmd_generate(opts: &Opts) -> Result<(), String> {
    let dir = opts.path("dir")?;
    let notes: usize = opts.flag("notes", 20_000)?;
    let seed: u64 = opts.flag("seed", 1)?;
    opts.configure_jobs()?;

    if dir.exists() {
        let mut entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "{} is not empty; refusing to write into it",
                dir.display()
            ));
        }
    }
    let started = Instant::now();
    let stats = generate_vault(&dir, notes, seed)?;
    println!(
        "wrote {} notes and {} attachments ({}) to {} in {}",
        stats.notes,
        stats.attachments,
        human_bytes(stats.bytes),
        dir.display(),
        human_duration(started.elapsed())
    );
    Ok(())
}

struct GenerateStats {
    notes: usize,
    attachments: usize,
    bytes: u64,
}

fn generate_vault(dir: &Path, notes: usize, seed: u64) -> Result<GenerateStats, String> {
    let mut rng = Rng::new(seed);
    let plans = plan_notes(&mut rng, notes);
    let attachments = (notes / 10).max(1);

    // Content generation is sequential so the output is a pure function of the seed.
    let mut rngs = Vec::with_capacity(plans.len());
    for _ in 0..plans.len() {
        rngs.push(rng.next());
    }
    let bodies: Vec<String> = plans
        .par_iter()
        .enumerate()
        .map(|(i, _)| {
            let mut r = Rng::new(rngs[i]);
            note_body(&mut r, &plans, i, attachments)
        })
        .collect();

    let mut dirs: Vec<PathBuf> = plans
        .iter()
        .map(|p| dir.join(p.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")))
        .collect();
    dirs.push(dir.join("Attachments"));
    dirs.push(dir.join(".obsidian"));
    dirs.sort();
    dirs.dedup();
    for d in &dirs {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }

    let io = |path: &Path, bytes: &[u8]| -> Result<(), String> {
        fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    };
    plans
        .par_iter()
        .zip(bodies.par_iter())
        .try_for_each(|(p, body)| io(&dir.join(&p.path), body.as_bytes()))?;
    // Eight bytes of PNG signature is enough for a file-kind test; nothing renders these.
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    (0..attachments)
        .into_par_iter()
        .try_for_each(|i| io(&dir.join(format!("Attachments/img-{i}.png")), &png))?;
    io(&dir.join(".obsidian/app.json"), b"{}\n")?;

    Ok(GenerateStats {
        notes: plans.len(),
        attachments,
        bytes: bodies.iter().map(|b| b.len() as u64).sum(),
    })
}

// ---------------------------------------------------------------------------
// Index timing

#[derive(Default)]
struct IndexRun {
    scan: Duration,
    read: Duration,
    index: Duration,
    resolve: Duration,
    files: usize,
    notes: usize,
    bytes: u64,
    read_errors: usize,
    links: usize,
    embeds: usize,
    resolved: usize,
    unresolved: usize,
    tags: usize,
    headings: usize,
    blocks: usize,
    with_frontmatter: usize,
}

impl IndexRun {
    fn total(&self) -> Duration {
        self.scan + self.read + self.index + self.resolve
    }
}

fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let v = f();
    (v, t.elapsed())
}

/// Scans, reads, indexes, and resolves one vault once. Every stage is timed
/// separately so a regression points at a crate, not at "the index".
fn index_once(root: &Path) -> Result<IndexRun, String> {
    let mut run = IndexRun::default();

    let (vault, scan) = time(|| Vault::open(root));
    let vault = vault.map_err(|e| e.to_string())?;
    run.scan = scan;
    run.files = vault.files().len();

    let notes: Vec<&str> = vault.markdown_files().map(|f| f.path.as_str()).collect();
    run.notes = notes.len();

    let (sources, read) = time(|| {
        notes
            .par_iter()
            .map(|p| fs::read_to_string(vault.absolute(p)).ok())
            .collect::<Vec<Option<String>>>()
    });
    run.read = read;
    run.read_errors = sources.iter().filter(|s| s.is_none()).count();
    run.bytes = sources.iter().flatten().map(|s| s.len() as u64).sum();

    let (metas, index) = time(|| {
        sources
            .par_iter()
            .map(|s| s.as_deref().map(index_markdown).unwrap_or_default())
            .collect::<Vec<CachedMetadata>>()
    });
    run.index = index;

    let (counts, resolve) = time(|| {
        let resolver: LinkResolver = vault.link_resolver();
        notes
            .par_iter()
            .zip(metas.par_iter())
            .map(|(path, meta)| {
                let mut ok = 0usize;
                let mut missing = 0usize;
                for l in meta.links.iter().chain(meta.embeds.iter()) {
                    if resolver.resolve(&l.link, path).is_some() {
                        ok += 1;
                    } else {
                        missing += 1;
                    }
                }
                (ok, missing)
            })
            .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    });
    run.resolve = resolve;
    run.resolved = counts.0;
    run.unresolved = counts.1;

    for m in &metas {
        run.links += m.links.len();
        run.embeds += m.embeds.len();
        run.tags += m.tags.len();
        run.headings += m.headings.len();
        run.blocks += m.blocks.len();
        run.with_frontmatter += usize::from(m.frontmatter.is_some());
    }
    Ok(run)
}

fn cmd_index(opts: &Opts) -> Result<(), String> {
    let root = opts.path("vault")?;
    let runs: usize = opts.flag("runs", 3)?;
    opts.configure_jobs()?;
    let runs = runs.max(1);

    let mut results = Vec::with_capacity(runs);
    for i in 0..runs {
        let run = index_once(&root)?;
        if i == 0 {
            println!(
                "vault {} — {} files, {} notes, {} of markdown, {} threads",
                root.display(),
                group(run.files),
                group(run.notes),
                human_bytes(run.bytes),
                rayon::current_num_threads()
            );
            if run.read_errors > 0 {
                println!(
                    "  {} notes could not be read as UTF-8 text",
                    run.read_errors
                );
            }
            println!(
                "  links {} ({} resolved, {} unresolved), embeds {}, tags {}, headings {}, block ids {}, frontmatter {}",
                group(run.links),
                group(run.resolved),
                group(run.unresolved),
                group(run.embeds),
                group(run.tags),
                group(run.headings),
                group(run.blocks),
                group(run.with_frontmatter)
            );
            println!();
            println!("  run       scan      read     index   resolve     total");
        }
        println!(
            "  {:<4} {:>9} {:>9} {:>9} {:>9} {:>9}",
            i + 1,
            human_duration(run.scan),
            human_duration(run.read),
            human_duration(run.index),
            human_duration(run.resolve),
            human_duration(run.total())
        );
        results.push(run);
    }

    let first = results[0].total();
    let best = results.iter().map(IndexRun::total).min().unwrap_or(first);
    println!();
    let verdict = if first <= GATE { "PASS" } else { "FAIL" };
    println!(
        "gate: {} in run 1 against a {} budget -> {verdict} (best run {})",
        human_duration(first),
        human_duration(GATE),
        human_duration(best)
    );
    if results[0].files < 20_000 {
        println!(
            "note: the gate is specified for a 20,000-file vault; this one has {}",
            group(results[0].files)
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Formatting

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn human_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_vault_indexes_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let stats = generate_vault(dir.path(), 300, 7).unwrap();
        assert_eq!(stats.notes, 300);

        let run = index_once(dir.path()).unwrap();
        assert_eq!(run.notes, 300);
        assert_eq!(run.files, 300 + stats.attachments);
        assert_eq!(run.read_errors, 0);
        assert_eq!(run.with_frontmatter, 300);
        assert!(run.links > 300, "links: {}", run.links);
        assert!(run.headings >= 300);
        assert!(run.tags > 0);
        assert!(run.blocks > 0);
        // Only the deliberately dangling links (about 5 percent) should fail to resolve.
        let total = run.resolved + run.unresolved;
        assert!(
            run.unresolved * 100 / total < 12,
            "unresolved {} of {}",
            run.unresolved,
            total
        );
    }

    #[test]
    fn generation_is_deterministic() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        generate_vault(a.path(), 40, 3).unwrap();
        generate_vault(b.path(), 40, 3).unwrap();
        let va = Vault::open(a.path()).unwrap();
        let vb = Vault::open(b.path()).unwrap();
        // What the generator decides: the kind follows from the path, and
        // the modification times are the clock's, not its.
        let listing = |v: &Vault| -> Vec<(String, u64)> {
            v.files().iter().map(|f| (f.path.clone(), f.size)).collect()
        };
        assert_eq!(listing(&va), listing(&vb));
        for f in va.markdown_files() {
            assert_eq!(
                fs::read_to_string(va.absolute(&f.path)).unwrap(),
                fs::read_to_string(vb.absolute(&f.path)).unwrap()
            );
        }
    }

    #[test]
    fn parses_flags_and_positionals() {
        let args: Vec<String> = ["out", "--notes", "10", "--seed=4"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = Opts::parse(&args).unwrap();
        assert_eq!(o.positional, vec!["out"]);
        assert_eq!(o.flag("notes", 0usize).unwrap(), 10);
        assert_eq!(o.flag("seed", 0u64).unwrap(), 4);
        assert_eq!(o.flag("runs", 3usize).unwrap(), 3);
    }
}
