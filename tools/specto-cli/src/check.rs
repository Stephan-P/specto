//! Renders one file and compares it with a reference renderer.
//!
//! Runs in a child process of `specto corpus` so that hangs, aborts and stack
//! overflows in the renderer only take down this one file.
//!
//! Output: one tab-separated line per page on stdout:
//! `page  width  height  ms  status  diff_fraction  mean_abs_diff`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Args;
use specto_render::{Bitmap, Document, OpenError, RenderRequest};
use wait_timeout::ChildExt;

#[derive(Args)]
pub struct CheckArgs {
    pub file: PathBuf,
    #[arg(long, default_value_t = 2.0)]
    pub scale: f64,
    #[arg(long, default_value_t = 10)]
    pub max_pages: usize,
    /// Compare against `pdftoppm` (Poppler).
    #[arg(long)]
    pub reference: bool,
    /// Where to save PNGs of pages that differ from the reference.
    #[arg(long)]
    pub save_diffs: Option<PathBuf>,
    /// Fraction of differing pixels above which a page is saved.
    #[arg(long, default_value_t = 0.02)]
    pub diff_threshold: f64,
}

/// A channel difference below this is treated as anti-aliasing noise.
const PIXEL_TOLERANCE: u8 = 48;

pub fn run(args: &CheckArgs) -> Result<()> {
    let data = std::fs::read(&args.file)?;
    let doc = match specto_render::HayroDocument::open(data, None) {
        Ok(doc) => doc,
        Err(e) => {
            let status = match e {
                OpenError::PasswordRequired => "password",
                OpenError::UnsupportedEncryption => "unsupported-encryption",
                OpenError::InvalidEncryption => "invalid-encryption",
                OpenError::Invalid => "invalid",
            };
            println!("0\t0\t0\t0\topen:{status}\t\t");
            return Ok(());
        }
    };

    let tmp = std::env::temp_dir().join(format!("specto-check-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;

    for page in 0..doc.page_count().min(args.max_pages) {
        let start = Instant::now();
        let result = doc.render(&RenderRequest::page(page, args.scale));
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let bitmap = match result {
            Ok(b) => b,
            Err(e) => {
                println!("{}\t0\t0\t{ms:.1}\terror:{e}\t\t", page + 1);
                continue;
            }
        };

        let (mut frac, mut mad) = (String::new(), String::new());
        if args.reference {
            match reference_render(&args.file, page, args.scale, &tmp) {
                Ok(reference) => {
                    let d = diff(&bitmap, &reference);
                    frac = format!("{:.5}", d.fraction);
                    mad = format!("{:.3}", d.mean_abs);
                    if d.fraction > args.diff_threshold
                        && let Some(dir) = &args.save_diffs
                    {
                        save_pair(dir, &args.file, page, &bitmap, &reference)?;
                    }
                }
                Err(_) => frac = "ref-failed".into(),
            }
        }
        println!(
            "{}\t{}\t{}\t{ms:.1}\tok\t{frac}\t{mad}",
            page + 1,
            bitmap.width,
            bitmap.height
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

/// pdftoppm gets its own deadline, shorter than `corpus`'s per-file timeout.
/// Otherwise `corpus` kills this process first and pdftoppm is orphaned,
/// burning CPU in the background indefinitely.
const REFERENCE_TIMEOUT: Duration = Duration::from_secs(20);

fn reference_render(file: &Path, page: usize, scale: f64, tmp: &Path) -> Result<image::RgbImage> {
    let root = tmp.join("ref");
    let n = (page + 1).to_string();
    let mut child = Command::new("pdftoppm")
        .args(["-cropbox", "-png", "-singlefile", "-f", &n, "-l", &n, "-r"])
        .arg(format!("{}", 72.0 * scale))
        .arg(file)
        .arg(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("running pdftoppm")?;
    let Some(status) = child.wait_timeout(REFERENCE_TIMEOUT)? else {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("pdftoppm timed out");
    };
    anyhow::ensure!(status.success(), "pdftoppm failed");
    Ok(image::open(root.with_extension("png"))?.into_rgb8())
}

struct Diff {
    /// Fraction of pixels whose largest channel difference exceeds the tolerance.
    fraction: f64,
    /// Mean absolute channel difference (0–255).
    mean_abs: f64,
}

/// Compares the overlapping area; renderers may round page sizes differently by a pixel.
///
/// A pixel only counts as differing if *no* reference pixel within a 1 px
/// radius is close to it, so sub-pixel glyph positioning and anti-aliasing
/// differences don't drown out real rendering errors.
fn diff(ours: &Bitmap, reference: &image::RgbImage) -> Diff {
    let w = ours.width.min(reference.width());
    let h = ours.height.min(reference.height());
    let max_channel_diff =
        |a: &[u8], b: [u8; 3]| (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
    let (mut differing, mut total_abs) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * ours.width + x) * 4) as usize;
            let a = &ours.data[i..i + 3];
            let b = reference.get_pixel(x, y).0;
            total_abs += (0..3).map(|c| a[c].abs_diff(b[c]) as u64).sum::<u64>();
            if max_channel_diff(a, b) <= PIXEL_TOLERANCE {
                continue;
            }
            let near_match = (y.saturating_sub(1)..=(y + 1).min(h - 1)).any(|ny| {
                (x.saturating_sub(1)..=(x + 1).min(w - 1))
                    .any(|nx| max_channel_diff(a, reference.get_pixel(nx, ny).0) <= PIXEL_TOLERANCE)
            });
            if !near_match {
                differing += 1;
            }
        }
    }
    let n = (w as u64 * h as u64).max(1);
    // Count size mismatch beyond one pixel as entirely differing rows/columns.
    let ours_area = ours.width as u64 * ours.height as u64;
    let ref_area = reference.width() as u64 * reference.height() as u64;
    let size_penalty = if ours.width.abs_diff(reference.width()) > 1
        || ours.height.abs_diff(reference.height()) > 1
    {
        ours_area.max(ref_area) - n
    } else {
        0
    };
    Diff {
        fraction: (differing + size_penalty) as f64 / (n + size_penalty) as f64,
        mean_abs: total_abs as f64 / (n * 3) as f64,
    }
}

fn save_pair(
    dir: &Path,
    file: &Path,
    page: usize,
    ours: &Bitmap,
    reference: &image::RgbImage,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let stem = file.file_stem().unwrap_or_default().to_string_lossy();
    crate::save_png(ours, &dir.join(format!("{stem}-p{}-hayro.png", page + 1)))?;
    reference.save(dir.join(format!("{stem}-p{}-poppler.png", page + 1)))?;
    Ok(())
}
