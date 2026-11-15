//! Runs `specto check` over every PDF in a directory and writes a report.

use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Args;
use wait_timeout::ChildExt;

#[derive(Args)]
pub struct CorpusArgs {
    /// Directory searched recursively for *.pdf files.
    pub dir: PathBuf,
    /// Output directory for report.md, results.tsv and differing pages.
    #[arg(short, long, default_value = "out/corpus")]
    pub output: PathBuf,
    #[arg(long, default_value_t = 2.0)]
    pub scale: f64,
    #[arg(long, default_value_t = 10)]
    pub max_pages: usize,
    /// Per-file timeout in seconds.
    #[arg(long, default_value_t = 60)]
    pub timeout: u64,
    /// Compare against pdftoppm.
    #[arg(long)]
    pub reference: bool,
    /// Parallel jobs (default: number of CPUs).
    #[arg(short, long)]
    pub jobs: Option<usize>,
}

#[derive(Debug)]
enum FileOutcome {
    Pages(Vec<PageResult>),
    OpenFailed(String),
    Crashed(String),
    Timeout,
}

#[derive(Debug)]
struct PageResult {
    page: usize,
    ms: f64,
    status: String,
    diff: Option<f64>,
}

pub fn run(args: &CorpusArgs) -> Result<()> {
    let mut files = Vec::new();
    collect_pdfs(&args.dir, &mut files)?;
    files.sort();
    anyhow::ensure!(!files.is_empty(), "no PDFs found in {}", args.dir.display());
    std::fs::create_dir_all(&args.output)?;
    let exe = std::env::current_exe()?;
    let jobs = args
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));

    eprintln!("checking {} files with {jobs} jobs…", files.len());
    let queue = Mutex::new(files.iter().collect::<Vec<_>>());
    let results = Mutex::new(Vec::new());
    let started = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let Some(file) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    let outcome = check_file(&exe, file, args);
                    let mut r = results.lock().unwrap();
                    r.push((file.clone(), outcome));
                    if r.len() % 25 == 0 {
                        eprintln!("  {}/{}", r.len(), files.len());
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.cmp(&b.0));

    write_tsv(&args.output.join("results.tsv"), &args.dir, &results)?;
    let report = report(args, &results, started.elapsed());
    std::fs::write(args.output.join("report.md"), &report)?;
    println!("{report}");
    Ok(())
}

fn collect_pdfs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_pdfs(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn check_file(exe: &Path, file: &Path, args: &CorpusArgs) -> FileOutcome {
    let mut cmd = Command::new(exe);
    cmd.arg("check")
        .arg(file)
        .args(["--scale", &args.scale.to_string()])
        .args(["--max-pages", &args.max_pages.to_string()]);
    if args.reference {
        cmd.arg("--reference")
            .arg("--save-diffs")
            .arg(args.output.join("diffs"));
    }
    let mut child = match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return FileOutcome::Crashed(format!("spawn: {e}")),
    };
    // Drain pipes on threads so a chatty child can't block on a full pipe.
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let status = match child.wait_timeout(Duration::from_secs(args.timeout)) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return FileOutcome::Timeout;
        }
        Err(e) => return FileOutcome::Crashed(format!("wait: {e}")),
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();

    if !status.success() {
        let msg = stderr
            .lines()
            .find(|l| l.contains("panicked") || l.contains("overflow"))
            .or_else(|| stderr.lines().last())
            .unwrap_or("no output");
        return FileOutcome::Crashed(format!("{status}: {msg}"));
    }

    let mut pages = Vec::new();
    for line in stdout.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        if let Some(reason) = f[4].strip_prefix("open:") {
            return FileOutcome::OpenFailed(reason.to_string());
        }
        pages.push(PageResult {
            page: f[0].parse().unwrap_or(0),
            ms: f[3].parse().unwrap_or(0.0),
            status: f[4].to_string(),
            diff: f[5].parse().ok(),
        });
    }
    FileOutcome::Pages(pages)
}

fn write_tsv(path: &Path, base: &Path, results: &[(PathBuf, FileOutcome)]) -> Result<()> {
    let mut s = String::from("file\tpage\tms\tstatus\tdiff\n");
    for (file, outcome) in results {
        let name = file.strip_prefix(base).unwrap_or(file).display();
        match outcome {
            FileOutcome::Pages(pages) => {
                for p in pages {
                    let diff = p.diff.map(|d| format!("{d:.5}")).unwrap_or_default();
                    writeln!(s, "{name}\t{}\t{:.1}\t{}\t{diff}", p.page, p.ms, p.status)?;
                }
            }
            FileOutcome::OpenFailed(r) => writeln!(s, "{name}\t\t\topen:{r}\t")?,
            FileOutcome::Crashed(r) => writeln!(s, "{name}\t\t\tcrash:{}\t", r.replace('\t', " "))?,
            FileOutcome::Timeout => writeln!(s, "{name}\t\t\ttimeout\t")?,
        }
    }
    std::fs::write(path, s)?;
    Ok(())
}

fn report(args: &CorpusArgs, results: &[(PathBuf, FileOutcome)], elapsed: Duration) -> String {
    let mut times = Vec::new();
    let mut diffs = Vec::new();
    let (mut ok_files, mut page_errors) = (0, 0);
    let mut open_failed: Vec<(&Path, &str)> = Vec::new();
    let mut crashed: Vec<(&Path, &str)> = Vec::new();
    let mut timeouts: Vec<&Path> = Vec::new();

    for (file, outcome) in results {
        match outcome {
            FileOutcome::Pages(pages) => {
                ok_files += 1;
                for p in pages {
                    if p.status == "ok" {
                        times.push(p.ms);
                        if let Some(d) = p.diff {
                            diffs.push((d, file.as_path(), p.page));
                        }
                    } else {
                        page_errors += 1;
                    }
                }
            }
            FileOutcome::OpenFailed(r) => open_failed.push((file, r)),
            FileOutcome::Crashed(r) => crashed.push((file, r)),
            FileOutcome::Timeout => timeouts.push(file),
        }
    }
    times.sort_by(f64::total_cmp);
    diffs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let pct = |q: f64| {
        times
            .get(((times.len() as f64 - 1.0) * q) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    let rel = |p: &Path| p.strip_prefix(&args.dir).unwrap_or(p).display().to_string();

    let mut s = String::new();
    let _ = writeln!(s, "# Corpus report\n");
    let _ = writeln!(
        s,
        "Scale {} · first {} pages per file · timeout {} s · wall time {:.0} s\n",
        args.scale,
        args.max_pages,
        args.timeout,
        elapsed.as_secs_f64()
    );
    let _ = writeln!(s, "| | count |\n|---|---|");
    let _ = writeln!(s, "| files | {} |", results.len());
    let _ = writeln!(s, "| rendered | {ok_files} |");
    let _ = writeln!(s, "| failed to open | {} |", open_failed.len());
    let _ = writeln!(s, "| crashed (panic/abort) | {} |", crashed.len());
    let _ = writeln!(s, "| timed out | {} |", timeouts.len());
    let _ = writeln!(s, "| pages rendered | {} |", times.len());
    let _ = writeln!(s, "| page render errors | {page_errors} |\n");

    let _ = writeln!(s, "## Render time per page (ms)\n");
    let _ = writeln!(
        s,
        "median {:.1} · p90 {:.1} · p99 {:.1} · max {:.1}\n",
        pct(0.5),
        pct(0.9),
        pct(0.99),
        times.last().copied().unwrap_or(0.0)
    );

    if !diffs.is_empty() {
        let over = |t: f64| diffs.iter().filter(|d| d.0 > t).count();
        let _ = writeln!(s, "## Difference from pdftoppm\n");
        let _ = writeln!(
            s,
            "Pages compared: {} · >1% pixels differ: {} · >5%: {} · >20%: {}\n",
            diffs.len(),
            over(0.01),
            over(0.05),
            over(0.20)
        );
        let _ = writeln!(s, "Worst 30 (PNGs in `diffs/`):\n");
        for (d, file, page) in diffs.iter().take(30) {
            let _ = writeln!(s, "- {:.1}% {} p{page}", d * 100.0, rel(file));
        }
        let _ = writeln!(s);
    }
    for (title, list) in [("Crashes", &crashed), ("Open failures", &open_failed)] {
        if !list.is_empty() {
            let _ = writeln!(s, "## {title}\n");
            for (f, r) in list.iter() {
                let _ = writeln!(s, "- {}: {r}", rel(f));
            }
            let _ = writeln!(s);
        }
    }
    if !timeouts.is_empty() {
        let _ = writeln!(s, "## Timeouts\n");
        for f in &timeouts {
            let _ = writeln!(s, "- {}", rel(f));
        }
    }
    s
}
