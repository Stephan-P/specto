//! `pdfrs`: headless tooling for development and testing.

mod check;
mod corpus;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pdfrs_render::{Bitmap, Content, Document, HayroDocument, OpenError, RenderRequest};

#[derive(Parser)]
#[command(name = "pdfrs", about = "pdfrs development tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print basic information about a PDF.
    Info {
        file: PathBuf,
        #[arg(long)]
        password: Option<String>,
    },
    /// Render pages to PNG.
    Render {
        file: PathBuf,
        /// Output directory.
        #[arg(short, long, default_value = "out")]
        output: PathBuf,
        /// Pages to render, 1-based, e.g. "1,3,5-7". Default: all.
        #[arg(short, long)]
        pages: Option<String>,
        /// Device pixels per PDF point (2.0 = 144 dpi).
        #[arg(short, long, default_value_t = 2.0)]
        scale: f32,
        /// Don't draw annotations stored in the file.
        #[arg(long)]
        no_annotations: bool,
        #[arg(long)]
        password: Option<String>,
    },
    /// Render a single file and print per-page results (used by `corpus`).
    #[command(hide = true)]
    Check(check::CheckArgs),
    /// Render every PDF in a directory, compare with a reference renderer and write a report.
    Corpus(corpus::CorpusArgs),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { file, password } => info(&file, password.as_deref()),
        Command::Render {
            file,
            output,
            pages,
            scale,
            no_annotations,
            password,
        } => render(
            &file,
            &output,
            pages.as_deref(),
            scale,
            no_annotations,
            password.as_deref(),
        ),
        Command::Check(args) => check::run(&args),
        Command::Corpus(args) => corpus::run(&args),
    }
}

pub(crate) fn open(file: &Path, password: Option<&str>) -> Result<HayroDocument> {
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    match HayroDocument::open(data, password) {
        Ok(doc) => Ok(doc),
        Err(OpenError::PasswordRequired) if password.is_none() => {
            bail!("{} is password protected (use --password)", file.display())
        }
        Err(e) => Err(e).with_context(|| format!("opening {}", file.display())),
    }
}

fn info(file: &Path, password: Option<&str>) -> Result<()> {
    let doc = open(file, password)?;
    let pdf = doc.pdf();
    println!("version:  {:?}", pdf.version());
    println!("pages:    {}", doc.page_count());
    if let Some(title) = &pdf.metadata().title {
        println!("title:    {}", String::from_utf8_lossy(title));
    }
    for i in 0..doc.page_count().min(10) {
        let s = doc.page_size(i).expect("index in range");
        println!("page {:>3}: {:.1} × {:.1} pt", i + 1, s.width, s.height);
    }
    if doc.page_count() > 10 {
        println!("…");
    }
    Ok(())
}

fn render(
    file: &Path,
    output: &Path,
    pages: Option<&str>,
    scale: f32,
    no_annotations: bool,
    password: Option<&str>,
) -> Result<()> {
    let doc = open(file, password)?;
    let pages = match pages {
        Some(spec) => parse_pages(spec, doc.page_count())?,
        None => (0..doc.page_count()).collect(),
    };
    std::fs::create_dir_all(output)?;
    let stem = file.file_stem().unwrap_or_default().to_string_lossy();
    for page in pages {
        let mut req = RenderRequest::page(page, scale);
        if no_annotations {
            req.content = Content::PageOnly;
        }
        let start = Instant::now();
        let bitmap = doc.render(&req)?;
        let elapsed = start.elapsed();
        let path = output.join(format!("{stem}-{}.png", page + 1));
        save_png(&bitmap, &path)?;
        println!(
            "page {:>3}: {}×{} px in {:>7.1} ms → {}",
            page + 1,
            bitmap.width,
            bitmap.height,
            elapsed.as_secs_f64() * 1000.0,
            path.display()
        );
    }
    Ok(())
}

/// Parses "1,3,5-7" (1-based, inclusive) into zero-based indices.
pub(crate) fn parse_pages(spec: &str, count: usize) -> Result<Vec<usize>> {
    let mut pages = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<usize>()?, b.trim().parse::<usize>()?),
            None => {
                let n = part.parse::<usize>()?;
                (n, n)
            }
        };
        if a == 0 || b < a || b > count {
            bail!("invalid page range {part:?} (document has {count} pages)");
        }
        pages.extend(a - 1..b);
    }
    Ok(pages)
}

/// Saves a bitmap rendered on an opaque background (so premultiplied == straight alpha).
pub(crate) fn save_png(bitmap: &Bitmap, path: &Path) -> Result<()> {
    image::save_buffer(
        path,
        &bitmap.data,
        bitmap.width,
        bitmap.height,
        image::ExtendedColorType::Rgba8,
    )
    .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::parse_pages;

    #[test]
    fn page_specs() {
        assert_eq!(parse_pages("1,3,5-7", 10).unwrap(), vec![0, 2, 4, 5, 6]);
        assert!(parse_pages("0", 10).is_err());
        assert!(parse_pages("4-2", 10).is_err());
        assert!(parse_pages("11", 10).is_err());
    }
}
