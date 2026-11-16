//! `specto`: headless tooling for development and testing.

mod check;
mod corpus;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use specto_render::{Bitmap, Content, Document, HayroDocument, OpenError, RenderRequest};

#[derive(Parser)]
#[command(name = "specto", about = "specto development tools")]
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
        /// Also print the outline (table of contents) and links.
        #[arg(long)]
        outline: bool,
        /// Also print the annotations (markup) of every page.
        #[arg(long)]
        annotations: bool,
        /// Also print the fields of the document's form.
        #[arg(long)]
        forms: bool,
    },
    /// Render pages to PNG.
    Render(RenderArgs),
    /// Print the text layer (words separated by spaces, lines by newlines).
    Text {
        file: PathBuf,
        /// Pages, 1-based, such as "1,3,5-7". Default: all.
        #[arg(short, long)]
        pages: Option<String>,
        #[arg(long)]
        password: Option<String>,
        /// Include text drawn by annotations (form fields, notes).
        #[arg(long)]
        annotations: bool,
    },
    /// Write a copy of a PDF with pages picked, reordered or rotated, as the apps save edited
    /// documents.
    Save {
        file: PathBuf,
        /// Where to write, which may be the input file.
        #[arg(short, long)]
        output: PathBuf,
        /// Pages to keep, in order, 1-based, such as "3,1-2". Default: all.
        #[arg(short, long)]
        pages: Option<String>,
        /// Turn the kept pages clockwise by this many degrees (90, 180, 270).
        #[arg(short, long, default_value_t = 0)]
        rotate: i64,
        #[arg(long)]
        password: Option<String>,
    },
    /// Render a single file and print per-page results (used by `corpus`).
    #[command(hide = true)]
    Check(check::CheckArgs),
    /// Render every PDF in a directory, compare with a reference renderer and write a report.
    Corpus(corpus::CorpusArgs),
}

/// Makes writing to a closed pipe (`specto info … | head`) end the program quietly, as for other
/// command line tools. Rust ignores SIGPIPE, so `println!` would panic instead.
#[cfg(unix)]
#[allow(unsafe_code)]
fn default_sigpipe() {
    // SAFETY: called at the start of main, before any other threads exist. It only resets how the
    // process handles one signal.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Not for check and corpus, which talk to child processes through pipes and handle their
    // errors.
    #[cfg(unix)]
    if !matches!(cli.command, Command::Check(_) | Command::Corpus(_)) {
        default_sigpipe();
    }
    match cli.command {
        Command::Info {
            file,
            password,
            outline,
            annotations,
            forms,
        } => info(&file, password.as_deref(), outline, annotations, forms),
        Command::Render(args) => render(&args),
        Command::Text {
            file,
            pages,
            password,
            annotations,
        } => text(&file, pages.as_deref(), password.as_deref(), annotations),
        Command::Save {
            file,
            output,
            pages,
            rotate,
            password,
        } => save(
            &file,
            &output,
            pages.as_deref(),
            rotate,
            password.as_deref(),
        ),
        Command::Check(args) => check::run(&args),
        Command::Corpus(args) => corpus::run(&args),
    }
}

pub(crate) fn open(file: &Path, password: Option<&str>) -> Result<HayroDocument> {
    let data = specto_source::PdfData::open(file)
        .with_context(|| format!("reading {}", file.display()))?;
    match HayroDocument::open(data, password) {
        Ok(doc) => Ok(doc),
        Err(OpenError::PasswordRequired) if password.is_none() => {
            bail!("{} is password protected (use --password)", file.display())
        }
        Err(e) => Err(e).with_context(|| format!("opening {}", file.display())),
    }
}

fn info(
    file: &Path,
    password: Option<&str>,
    outline: bool,
    annotations: bool,
    forms: bool,
) -> Result<()> {
    let doc = open(file, password)?;
    let pdf = doc.pdf();
    println!("version:  {:?}", pdf.version());
    println!("pages:    {}", doc.page_count());
    if outline {
        let data = specto_source::PdfData::open(file)?;
        let source = specto_source::SourceDocument::open(data, password)?;
        let m = source.metadata();
        for (k, v) in [
            ("title", &m.title),
            ("author", &m.author),
            ("producer", &m.producer),
        ] {
            if let Some(v) = v {
                println!("{k:<9} {v}");
            }
        }
        if let Some(labels) = source.page_labels() {
            let shown: Vec<String> = (0..source.page_count().min(12))
                .map(|i| labels.label(i))
                .collect();
            let more = if source.page_count() > 12 {
                ", …"
            } else {
                ""
            };
            println!("labels:   {}{more}", shown.join(", "));
        }
        println!("outline:");
        print_outline(&source.outline(), 1);
        for page in 0..source.page_count() {
            for link in source.links(page) {
                println!("link p{} {:?} → {:?}", page + 1, link.rect, link.target);
            }
        }
    }
    if forms {
        let data = specto_source::PdfData::open(file)?;
        let source = specto_source::SourceDocument::open(data, password)?;
        for f in specto_forms::fields(&source) {
            let places: Vec<String> = f
                .widgets
                .iter()
                .map(|w| format!("p{}", w.page + 1))
                .collect();
            println!(
                "field {:?} {:?}{}{} = {:?}{} on {}",
                f.name,
                f.kind,
                if f.read_only { " read-only" } else { "" },
                if f.required { " required" } else { "" },
                f.value,
                if f.unshown { " not shown" } else { "" },
                places.join(" "),
            );
        }
    }
    if annotations {
        let data = specto_source::PdfData::open(file)?;
        let source = specto_source::SourceDocument::open(data, password)?;
        for page in 0..source.page_count() {
            for a in specto_annot::import(&source, page) {
                let b = a.rect;
                let kind = match &a.kind {
                    specto_annot::AnnotKind::Other { subtype } => subtype.clone(),
                    k => format!("{:?}", k.annot_type()),
                };
                println!(
                    "annot p{} {kind} [{:.0} {:.0} {:.0} {:.0}] {:?}{}{}",
                    page + 1,
                    b.min.x,
                    b.min.y,
                    b.max.x,
                    b.max.y,
                    a.appearance,
                    if a.flags.is_visible() { "" } else { " hidden" },
                    a.contents
                        .map(|c| format!(" {:?}", c.chars().take(40).collect::<String>()))
                        .unwrap_or_default(),
                );
            }
        }
    }
    if let Some(title) = &pdf.metadata().title {
        println!("title:    {}", String::from_utf8_lossy(title));
    }
    for i in 0..doc.page_count().min(10) {
        let g = doc.page_geometry(i).expect("index in range");
        let s = g.size();
        let rot = match g.rotation().degrees() {
            0 => String::new(),
            d => format!(" (rotated {d}°)"),
        };
        let images = match doc.page_image_pixels(i) {
            0 => String::new(),
            px => format!(", images {:.1} MP", px as f64 / 1e6),
        };
        println!(
            "page {:>3}: {:.1} × {:.1} pt{rot}{images}",
            i + 1,
            s.width,
            s.height
        );
    }
    if doc.page_count() > 10 {
        println!("…");
    }
    Ok(())
}

fn text(file: &Path, pages: Option<&str>, password: Option<&str>, annotations: bool) -> Result<()> {
    let source =
        specto_source::SourceDocument::open(specto_source::PdfData::open(file)?, password)?;
    let pages = match pages {
        Some(spec) => parse_pages(spec, source.page_count())?,
        None => (0..source.page_count()).collect(),
    };
    let extractor = specto_text::Extractor::new(source.pdf()).with_annotations(annotations);
    for (i, page) in pages.into_iter().enumerate() {
        if i > 0 {
            println!("\u{c}"); // form feed between pages, like pdftotext
        }
        if let Some(t) = extractor.page(page) {
            println!("{}", t.text());
        }
    }
    Ok(())
}

#[derive(clap::Args)]
struct RenderArgs {
    file: PathBuf,
    /// Output directory.
    #[arg(short, long, default_value = "out")]
    output: PathBuf,
    /// Pages to render, 1-based, such as "1,3,5-7". Default: all.
    #[arg(short, long)]
    pages: Option<String>,
    /// Device pixels per PDF point (2.0 = 144 dpi).
    #[arg(short, long, default_value_t = 2.0)]
    scale: f64,
    /// Only render this part of the page bitmap, "x,y,width,height" in pixels at `--scale`, like a
    /// viewer tile.
    #[arg(long)]
    region: Option<String>,
    /// Don't draw annotations stored in the file.
    #[arg(long)]
    no_annotations: bool,
    #[arg(long)]
    password: Option<String>,
}

fn render(args: &RenderArgs) -> Result<()> {
    let doc = open(&args.file, args.password.as_deref())?;
    let pages = match &args.pages {
        Some(spec) => parse_pages(spec, doc.page_count())?,
        None => (0..doc.page_count()).collect(),
    };
    let region = args.region.as_deref().map(parse_region).transpose()?;
    std::fs::create_dir_all(&args.output)?;
    let stem = args.file.file_stem().unwrap_or_default().to_string_lossy();
    for page in pages {
        let mut req = RenderRequest::page(page, args.scale);
        req.region = region;
        if args.no_annotations {
            req.content = Content::PageOnly;
        }
        let start = Instant::now();
        let bitmap = doc.render(&req)?;
        let elapsed = start.elapsed();
        let path = args.output.join(format!("{stem}-{}.png", page + 1));
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

/// Parses "x,y,width,height".
fn parse_region(spec: &str) -> Result<specto_render::DeviceIntRect> {
    let v: Vec<u32> = spec
        .split(',')
        .map(|p| p.trim().parse())
        .collect::<Result<_, _>>()
        .with_context(|| format!("invalid region {spec:?}"))?;
    let [x, y, w, h] = v[..] else {
        bail!("region must be x,y,width,height, got {spec:?}");
    };
    Ok(specto_render::DeviceIntRect::new(
        (x, y).into(),
        (x + w, y + h).into(),
    ))
}

fn print_outline(items: &[specto_source::OutlineItem], depth: usize) {
    for item in items {
        let target = match &item.target {
            Some(specto_source::Target::Destination(d)) => format!("p{}", d.page + 1),
            Some(t) => format!("{t:?}"),
            None => "-".into(),
        };
        println!("{}{} → {target}", "  ".repeat(depth), item.title);
        print_outline(&item.children, depth + 1);
    }
}

/// Parses "1,3,5-7" (1-based, inclusive) into zero-based indices.
fn save(
    file: &Path,
    output: &Path,
    pages: Option<&str>,
    rotate: i64,
    password: Option<&str>,
) -> Result<()> {
    use specto_doc::{PageRef, Rotation, SourceId};
    if rotate % 90 != 0 {
        bail!("--rotate must be a multiple of 90");
    }
    let count = open(file, password)?.page_count();
    let pages = match pages {
        Some(spec) => parse_pages(spec, count)?,
        None => (0..count).collect(),
    };
    let rotation = Rotation::from_degrees(rotate);
    let pages: Vec<PageRef> = pages
        .into_iter()
        .map(|i| PageRef {
            rotation,
            ..PageRef::new(SourceId(0), i as u32)
        })
        .collect();
    let data = specto_source::PdfData::open(file)?;
    let source = specto_write::Source {
        id: SourceId(0),
        data: &data,
        password,
    };
    let start = Instant::now();
    specto_write::save(output, &[source], &pages)
        .with_context(|| format!("saving {}", output.display()))?;
    println!(
        "{} pages → {} in {:.2} s",
        pages.len(),
        output.display(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

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
