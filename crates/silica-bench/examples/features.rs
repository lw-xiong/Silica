//! Comprehensive feature benchmark for Silica.
//!
//! Usage:
//!   silica-features generate <dir> [--notes N]
//!   silica-features index <vault> [--runs R]
//!   silica-features search <vault> <query> [--runs R]
//!   silica-features parse <vault> [--runs R]
//!   silica-features export <vault> <out-dir>
//!   silica-features all <vault> [--runs R]

use rayon::prelude::*;
use silica_cache::{index_markdown, CachedMetadata};
use silica_markdown::{parse, render_html};
use silica_plugins::Registry;
use silica_search::{Search, SearchFile};
use silica_vault::Vault;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

const USAGE: &str = "
silica-features: comprehensive feature benchmark for Silica

  silica-features generate <dir> [--notes N]
      Write a synthetic vault into <dir>.

  silica-features index <vault> [--runs R]
      Time scan, read, index, and resolve stages.

  silica-features search <vault> <query> [--runs R]
      Time search query parsing and matching across all notes.

  silica-features parse <vault> [--runs R]
      Time markdown parsing of every note.

  silica-features export <vault> <out-dir>
      Time HTML export of every note.

  silica-features all <vault> [--runs R]
      Run all benchmarks sequentially.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        println!("{}", USAGE);
        return;
    }
    match args[0].as_str() {
        "generate" => {
            let dir = args.get(1).expect("missing <dir>");
            let notes: usize = args.iter().position(|a| a == "--notes")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(20000);
            let _ = generate_vault(Path::new(dir), notes, 1);
        }
        "index" => {
            let root = args.get(1).expect("missing <vault>");
            let runs: usize = args.iter().position(|a| a == "--runs")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            cmd_index(Path::new(root), runs);
        }
        "search" => {
            let root = args.get(1).expect("missing <vault>");
            let query = args.get(2).expect("missing <query>");
            let runs: usize = args.iter().position(|a| a == "--runs")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            cmd_search(Path::new(root), query, runs);
        }
        "parse" => {
            let root = args.get(1).expect("missing <vault>");
            let runs: usize = args.iter().position(|a| a == "--runs")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            cmd_parse(Path::new(root), runs);
        }
        "plugin" => {
            let root = args.get(1).expect("missing <vault>");
            let runs: usize = args.iter().position(|a| a == "--runs")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            cmd_plugin(Path::new(root), runs);
        }
        "export" => {
            let root = args.get(1).expect("missing <vault>");
            let out_dir = args.get(2).expect("missing <out-dir>");
            cmd_export(Path::new(root), Path::new(out_dir));
        }
        "all" => {
            let root = args.get(1).expect("missing <vault>");
            let runs: usize = args.iter().position(|a| a == "--runs")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            cmd_all(Path::new(root), runs);
        }
        _ => {
            eprintln!("unknown subcommand: {}", args[0]);
            println!("{}", USAGE);
            std::process::exit(1);
        }
    }
}

fn cmd_index(root: &Path, runs: usize) {
    let vault = Vault::open(root).expect("failed to open vault");
    let notes: Vec<&str> = vault.markdown_files().map(|f| f.path.as_str()).collect();

    println!("vault {} — {} notes", root.display(), notes.len());
    println!();

    let mut results = Vec::with_capacity(runs);
    for i in 0..runs {
        let run = index_once(&vault, &notes);
        println!(
            "  {:<4} scan={:>9} read={:>9} index={:>9} resolve={:>9} total={:>9}",
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
    let best = results.iter().map(|r| r.total()).min().unwrap_or(first);
    println!();
    let verdict = if first <= Duration::from_secs(2) { "PASS" } else { "FAIL" };
    println!(
        "gate: {} vs 2.00s budget -> {} (best run {})",
        human_duration(first),
        verdict,
        human_duration(best)
    );
}

fn index_once(vault: &Vault, notes: &[&str]) -> IndexRun {
    let mut run = IndexRun::default();

    let (_, scan) = timed(|| Vault::open(vault.root()));
    run.scan = scan;

    let (sources, read) = timed(|| {
        notes
            .par_iter()
            .map(|p| fs::read_to_string(vault.absolute(p)).ok())
            .collect::<Vec<Option<String>>>()
    });
    run.read = read;
    run.read_errors = sources.iter().filter(|s| s.is_none()).count();
    run.bytes = sources.iter().flatten().map(|s| s.len() as u64).sum();

    let (metas, index) = timed(|| {
        sources
            .par_iter()
            .map(|s| s.as_deref().map(index_markdown).unwrap_or_default())
            .collect::<Vec<CachedMetadata>>()
    });
    run.index = index;

    let (counts, resolve) = timed(|| {
        let resolver: silica_cache::LinkResolver = vault.link_resolver();
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
    }
    run
}

#[derive(Default)]
struct IndexRun {
    scan: Duration,
    read: Duration,
    index: Duration,
    resolve: Duration,
    #[expect(dead_code)]
    files: usize,
    #[expect(dead_code)]
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
}

impl IndexRun {
    fn total(&self) -> Duration {
        self.scan + self.read + self.index + self.resolve
    }
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let v = f();
    (v, t.elapsed())
}

fn cmd_search(root: &Path, query: &str, runs: usize) {
    let vault = Vault::open(root).expect("failed to open vault");
    let notes: Vec<&str> = vault.markdown_files().map(|f| f.path.as_str()).collect();

    let sources: Vec<(String, String)> = notes
        .iter()
        .filter_map(|p| {
            let text = fs::read_to_string(vault.absolute(p)).ok()?;
            Some((p.to_string(), text))
        })
        .collect();

    println!("vault {} — {} notes, query: {:?}", root.display(), notes.len(), query);
    println!();

    let mut results = Vec::with_capacity(runs);
    for i in 0..runs {
        let (search, parse_time) = timed(|| Search::new(query));
        let search = search.expect("query parse error");

        let (match_count, match_time) = timed(|| {
            let mut count = 0usize;
            for (path, text) in &sources {
                let meta = index_markdown(text);
                let file = SearchFile {
                    path: path.as_str(),
                    content: text.as_str(),
                    tags: &[],
                    frontmatter: meta.frontmatter.as_ref(),
                    blocks: &[],
                    sections: &[],
                    tasks: &[],
                };
                if search.matches(&file).is_some() {
                    count += 1;
                }
            }
            count
        });

        println!(
            "  {:<4} parse={:>9} match={:>9} results={:<9} total={:>9}",
            i + 1,
            human_duration(parse_time),
            human_duration(match_time),
            match_count,
            human_duration(parse_time + match_time)
        );
        results.push((parse_time, match_time, match_count));
    }

    let avg_parse = results.iter().map(|(p, _, _)| *p).sum::<Duration>() / results.len() as u32;
    let avg_match = results.iter().map(|(_, m, _)| *m).sum::<Duration>() / results.len() as u32;
    println!();
    println!(
        "avg: parse={:>9} match={:>9} total={:>9}",
        human_duration(avg_parse),
        human_duration(avg_match),
        human_duration(avg_parse + avg_match)
    );
}

fn cmd_parse(root: &Path, runs: usize) {
    let vault = Vault::open(root).expect("failed to open vault");
    let notes: Vec<&str> = vault.markdown_files().map(|f| f.path.as_str()).collect();

    println!("vault {} — {} notes", root.display(), notes.len());
    println!();

    let mut total_times = Vec::with_capacity(runs);
    for i in 0..runs {
        let (_, parse_time) = timed(|| {
            notes
                .par_iter()
                .map(|p| {
                    let text = fs::read_to_string(vault.absolute(p)).unwrap_or_default();
                    index_markdown(&text)
                })
                .collect::<Vec<CachedMetadata>>()
        });
        let per_note = Duration::from_nanos(parse_time.as_nanos() as u64 / notes.len() as u64);
        println!(
            "  {:<4} total={:>9} per_note={:>9}",
            i + 1,
            human_duration(parse_time),
            human_duration(per_note)
        );
        total_times.push(parse_time);
    }
    let avg = total_times.iter().sum::<Duration>() / total_times.len() as u32;
    let per_note = Duration::from_nanos(avg.as_nanos() as u64 / notes.len() as u64);
    println!();
    println!("avg: total={:>9} per_note={:>9}", human_duration(avg), human_duration(per_note));
}

fn cmd_plugin(root: &Path, runs: usize) {
    let vault = Vault::open(root).expect("failed to open vault");

    println!("vault {} — plugin discovery", root.display());
    println!();

    for i in 0..runs {
        let (plugins, scan_time) = timed(|| Registry::scan(&vault));
        let records = plugins.records();
        let native_count = records.iter().filter(|r| r.tier == silica_plugins::Tier::Native).count();
        let community_count = records.iter().filter(|r| r.enabled).count();

        println!(
            "  {:<4} scan={:>9} native={:<8} community={:<8}",
            i + 1,
            human_duration(scan_time),
            native_count,
            community_count
        );
    }
}

fn cmd_export(root: &Path, out_dir: &Path) {
    let vault = Vault::open(root).expect("failed to open vault");
    let notes: Vec<&str> = vault.markdown_files().map(|f| f.path.as_str()).collect();

    fs::create_dir_all(out_dir).expect("failed to create output dir");

    let started = Instant::now();
    let mut exported = 0usize;
    for path in &notes {
        let text = fs::read_to_string(vault.absolute(path)).unwrap_or_default();
        let doc = parse(&text);
        let html = render_html(&doc);
        let out_name = path.replace(['/', '\\'], "_");
        let out_path = out_dir.join(format!("{}.html", out_name));
        fs::write(&out_path, html).expect("failed to write");
        exported += 1;
    }
    let elapsed = started.elapsed();
    let notes_per_sec = exported as f64 / elapsed.as_secs_f64();

    println!("vault {} — {} notes exported to {}", root.display(), exported, out_dir.display());
    println!("total: {:>9}  notes/sec: {:.0}", human_duration(elapsed), notes_per_sec);
}

fn cmd_all(root: &Path, runs: usize) {
    println!("=== INDEX ===");
    cmd_index(root, runs);
    println!();
    println!("=== SEARCH ===");
    cmd_search(root, "note", runs);
    println!();
    println!("=== PARSE ===");
    cmd_parse(root, runs);
    println!();
    println!("=== PLUGIN ===");
    cmd_plugin(root, runs);
    println!();
    let out_dir = Path::new("/tmp/silica-export-bench");
    let _ = fs::remove_dir_all(out_dir);
    cmd_export(root, out_dir);
}

fn generate_vault(dir: &Path, notes: usize, seed: u64) -> bool {
    let _ = (dir, notes, seed);
    println!("generate not implemented in this build; use silica-bench generate");
    true
}


fn human_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{:.0} ms", ms)
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}
