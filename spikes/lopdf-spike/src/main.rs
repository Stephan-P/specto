//! M0 spike: can lopdf handle the editing and saving Specto needs?
//!
//! Throwaway code. Each scenario writes a file, then re-opens it with hayro (our
//! renderer) and checks the result. Poppler is checked separately from the shell.
//!
//! Usage:
//!   lopdf-spike load <dir>                 # lopdf parse success rate over a corpus
//!   lopdf-spike ops <a.pdf> <b.pdf> <out>  # editing scenarios

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{
    Dictionary, Document, EncryptionState, EncryptionVersion, IncrementalDocument, Object,
    ObjectId, Permissions, Stream, dictionary,
};
use pdfrs_render::{Content, Document as _, HayroDocument, RenderRequest};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("load") => load_corpus(Path::new(&args[2])),
        Some("ops") => ops(
            Path::new(&args[2]),
            Path::new(&args[3]),
            Path::new(&args[4]),
        ),
        _ => bail!("usage: lopdf-spike load <dir> | ops <a.pdf> <b.pdf> <out-dir>"),
    }
}

// ---------------------------------------------------------------------------
// load: how many files can lopdf parse, compared with hayro?

fn load_corpus(dir: &Path) -> Result<()> {
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    let (mut lo_ok, mut hy_ok, mut both, mut lo_only, mut hy_only) = (0, 0, 0, 0, 0);
    let mut lo_panics = Vec::new();
    let mut lo_time = 0.0;
    for f in &files {
        let data = std::fs::read(f)?;
        let t = Instant::now();
        let lo = std::panic::catch_unwind(|| Document::load_mem(&data));
        lo_time += t.elapsed().as_secs_f64();
        let lo = match lo {
            Ok(r) => r.is_ok() || matches!(r, Err(lopdf::Error::Decryption(_))),
            Err(_) => {
                lo_panics.push(f.clone());
                false
            }
        };
        let hy = !matches!(
            HayroDocument::open(data, None),
            Err(pdfrs_render::OpenError::Invalid)
        );
        lo_ok += lo as u32;
        hy_ok += hy as u32;
        both += (lo && hy) as u32;
        lo_only += (lo && !hy) as u32;
        hy_only += (!lo && hy) as u32;
        if hy && !lo {
            println!("lopdf-only failure: {}", f.display());
        }
    }
    println!("\nfiles: {}", files.len());
    println!("lopdf parsed: {lo_ok}  (total {lo_time:.1} s)");
    println!("hayro parsed: {hy_ok}");
    println!("both: {both} · only lopdf: {lo_only} · only hayro: {hy_only}");
    println!("lopdf panics: {}", lo_panics.len());
    for p in lo_panics {
        println!("  {}", p.display());
    }
    Ok(())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            out.push(p);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ops: editing scenarios

fn ops(a: &Path, b: &Path, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let a_pages = Document::load(a)?.get_pages().len();
    let b_pages = Document::load(b)?.get_pages().len();
    println!("a: {a_pages} pages, b: {b_pages} pages");

    run("1 delete page 1 (full rewrite)", || {
        let mut doc = Document::load(a)?;
        doc.delete_pages(&[1]);
        doc.prune_objects();
        let p = out.join("1-deleted.pdf");
        doc.save(&p)?;
        expect_pages(&p, None, a_pages - 1)
    });

    run("2 merge a + b (full rewrite)", || {
        let mut doc = Document::load(a)?;
        let other = Document::load(b)?;
        append_document(&mut doc, other)?;
        let p = out.join("2-merged.pdf");
        doc.save(&p)?;
        expect_pages(&p, None, a_pages + b_pages)
    });

    run("3 reorder: reverse pages (full rewrite)", || {
        let mut doc = Document::load(a)?;
        reverse_pages(&mut doc)?;
        let p = out.join("3-reversed.pdf");
        doc.save(&p)?;
        expect_pages(&p, None, a_pages)
    });

    run("4 add highlight on page 1 (incremental)", || {
        let bytes = std::fs::read(a)?;
        let prev = Document::load_mem(&bytes)?;
        let mut inc = IncrementalDocument::create_from(bytes.clone(), prev);
        add_highlight(&mut inc)?;
        let p = out.join("4-highlight.pdf");
        inc.save(&p)?;
        let new = std::fs::read(&p)?;
        ensure!(new.starts_with(&bytes), "original bytes not preserved");
        expect_pages(&p, None, a_pages)?;
        expect_annotation_visible(&p, None)
    });

    let user_pw = "user";
    run("5 encrypt with AES-256 (full rewrite)", || {
        let mut doc = Document::load(a)?;
        let state = aes256_state(&doc, user_pw, "owner")?;
        doc.encrypt(&state)?;
        let p = out.join("5-encrypted.pdf");
        doc.save(&p)?;
        let data = std::fs::read(&p)?;
        ensure!(
            matches!(
                HayroDocument::open(data, None),
                Err(pdfrs_render::OpenError::PasswordRequired)
            ),
            "opened without a password"
        );
        expect_pages(&p, Some(user_pw), a_pages)
    });

    run("6 add highlight to encrypted file (incremental)", || {
        let src = out.join("5-encrypted.pdf");
        let bytes = std::fs::read(&src)?;
        let prev =
            Document::load_mem_with_options(&bytes, lopdf::LoadOptions::with_password(user_pw))?;
        let mut inc = IncrementalDocument::create_from(bytes.clone(), prev);
        add_highlight(&mut inc)?;
        let p = out.join("6-encrypted-highlight.pdf");
        inc.save(&p)?;
        expect_pages(&p, Some(user_pw), a_pages)?;
        expect_annotation_visible(&p, Some(user_pw))
    });

    run(
        "7 delete page from encrypted file, keep encryption (full rewrite)",
        || {
            let src = out.join("5-encrypted.pdf");
            let mut doc = Document::load_with_password(&src, user_pw)?;
            // load_with_password decrypts in memory but remembers the state.
            let state = doc
                .encryption_state
                .clone()
                .context("no encryption state after decrypt")?;
            doc.trailer.remove(b"Encrypt");
            if let Some(id) = state.encrypt_object_id() {
                doc.objects.remove(&id);
            }
            doc.delete_pages(&[1]);
            doc.prune_objects();
            doc.encrypt(&state)?;
            let p = out.join("7-encrypted-deleted.pdf");
            doc.save(&p)?;
            let data = std::fs::read(&p)?;
            ensure!(
                HayroDocument::open(data, None).is_err(),
                "opened without a password"
            );
            expect_pages(&p, Some(user_pw), a_pages - 1)
        },
    );

    Ok(())
}

fn run(name: &str, f: impl FnOnce() -> Result<()>) {
    match f() {
        Ok(()) => println!("PASS {name}"),
        Err(e) => println!("FAIL {name}: {e:#}"),
    }
}

fn expect_pages(path: &Path, password: Option<&str>, expected: usize) -> Result<()> {
    let doc = HayroDocument::open(std::fs::read(path)?, password)
        .map_err(|e| anyhow::anyhow!("hayro can't open result: {e}"))?;
    ensure!(
        doc.page_count() == expected,
        "hayro sees {} pages, expected {expected}",
        doc.page_count()
    );
    for i in 0..doc.page_count() {
        doc.render(&RenderRequest::page(i, 0.5))?;
    }
    Ok(())
}

/// The highlight must change page 1 when annotations are drawn.
fn expect_annotation_visible(path: &Path, password: Option<&str>) -> Result<()> {
    let doc =
        HayroDocument::open(std::fs::read(path)?, password).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut req = RenderRequest::page(0, 1.0);
    let with = doc.render(&req)?;
    req.content = Content::PageOnly;
    let without = doc.render(&req)?;
    ensure!(with.data != without.data, "annotation not visible");
    Ok(())
}

/// Appends all pages of `other` to `doc`.
///
/// lopdf has no "import pages" API: renumber the other document's objects past
/// ours, copy them all, and hang its pages under our page tree root.
fn append_document(doc: &mut Document, mut other: Document) -> Result<()> {
    // Inheritable page attributes live on intermediate /Pages nodes we're about
    // to discard, so push them down onto each page first.
    flatten_inherited(&mut other)?;
    other.renumber_objects_with(doc.max_id + 1);
    doc.max_id = other.max_id;

    let other_pages: Vec<ObjectId> = other.get_pages().into_values().collect();
    let other_catalog_root = other.trailer.get(b"Root")?.as_reference()?;
    let other_tree_root = other
        .get_dictionary(other_catalog_root)?
        .get(b"Pages")?
        .as_reference()?;

    for (id, obj) in other.objects {
        // Skip the other document's catalog and page tree nodes.
        if id == other_catalog_root {
            continue;
        }
        if let Object::Dictionary(d) = &obj
            && d.get(b"Type").and_then(Object::as_name).ok() == Some(b"Pages")
        {
            continue;
        }
        doc.objects.insert(id, obj);
    }
    let _ = other_tree_root;

    let root = doc.catalog()?.get(b"Pages")?.as_reference()?;
    for &page in &other_pages {
        doc.get_dictionary_mut(page)?.set("Parent", root);
    }
    let tree = doc.get_dictionary_mut(root)?;
    let kids = tree.get_mut(b"Kids")?.as_array_mut()?;
    kids.extend(other_pages.iter().map(|&id| Object::Reference(id)));
    let count = tree.get(b"Count")?.as_i64()? + other_pages.len() as i64;
    tree.set("Count", count);
    Ok(())
}

/// Copies inheritable attributes (Resources, MediaBox, CropBox, Rotate) from
/// ancestors onto every page that doesn't define them itself.
fn flatten_inherited(doc: &mut Document) -> Result<()> {
    const KEYS: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    for page in pages {
        let mut inherited = Dictionary::new();
        let mut node = doc
            .get_dictionary(page)?
            .get(b"Parent")
            .and_then(Object::as_reference)
            .ok();
        while let Some(id) = node {
            let d = doc.get_dictionary(id)?;
            for k in KEYS {
                if !inherited.has(k)
                    && let Ok(v) = d.get(k)
                {
                    inherited.set(k, v.clone());
                }
            }
            node = d.get(b"Parent").and_then(Object::as_reference).ok();
        }
        let p = doc.get_dictionary_mut(page)?;
        for (k, v) in inherited.iter() {
            if !p.has(k) {
                p.set(k.clone(), v.clone());
            }
        }
    }
    Ok(())
}

/// Rebuilds the page tree as one flat /Kids array in reverse order.
fn reverse_pages(doc: &mut Document) -> Result<()> {
    flatten_inherited(doc)?;
    let mut pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    pages.reverse();
    let root = doc.catalog()?.get(b"Pages")?.as_reference()?;
    for &p in &pages {
        doc.get_dictionary_mut(p)?.set("Parent", root);
    }
    let tree = doc.get_dictionary_mut(root)?;
    tree.set(
        "Kids",
        pages
            .iter()
            .map(|&id| Object::Reference(id))
            .collect::<Vec<_>>(),
    );
    tree.set("Count", pages.len() as i64);
    doc.prune_objects();
    Ok(())
}

/// Adds a yellow highlight with an appearance stream across the middle of page 1.
fn add_highlight(inc: &mut IncrementalDocument) -> Result<()> {
    let prev = inc.get_prev_documents();
    let page_id = *prev.get_pages().get(&1).context("no page 1")?;
    let media = prev
        .get_dictionary(page_id)?
        .get(b"MediaBox")
        .and_then(Object::as_array)
        .map(|a| {
            a.iter()
                .map(|o| o.as_float().unwrap_or(0.0))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|_| vec![0.0, 0.0, 612.0, 792.0]);
    let (x0, y0, x1, y1) = (media[0], media[1], media[2], media[3]);
    let (w, h) = (x1 - x0, y1 - y0);
    let rect = [x0 + w * 0.1, y0 + h * 0.45, x0 + w * 0.9, y0 + h * 0.5];
    let (rw, rh) = (rect[2] - rect[0], rect[3] - rect[1]);

    let doc = &mut inc.new_document;
    let gs = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "BM" => "Multiply",
        "CA" => 1.0, "ca" => 1.0,
    });
    let ap = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), rw.into(), rh.into()],
            "Resources" => dictionary! { "ExtGState" => dictionary! { "GS0" => gs } },
        },
        format!("q /GS0 gs 1 0.9 0 rg 0 0 {rw} {rh} re f Q").into_bytes(),
    ));
    let quad: Vec<Object> = [
        rect[0], rect[3], rect[2], rect[3], rect[0], rect[1], rect[2], rect[1],
    ]
    .into_iter()
    .map(Object::from)
    .collect();
    let annot = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Highlight",
        "Rect" => rect.iter().map(|&v| Object::from(v)).collect::<Vec<_>>(),
        "QuadPoints" => quad,
        "C" => vec![1.0.into(), 0.9.into(), 0.0.into()],
        "F" => 4, // Print
        "P" => page_id,
        "AP" => dictionary! { "N" => ap },
    });

    inc.opt_clone_object_to_new_document(page_id)?;
    // /Annots may be a direct array or a reference to one.
    let existing = inc
        .new_document
        .get_dictionary(page_id)?
        .get(b"Annots")
        .cloned()
        .ok();
    let mut annots = match existing {
        Some(Object::Array(a)) => a,
        Some(Object::Reference(r)) => inc.get_prev_documents().get_object(r)?.as_array()?.clone(),
        _ => Vec::new(),
    };
    annots.push(Object::Reference(annot));
    inc.new_document
        .get_dictionary_mut(page_id)?
        .set("Annots", annots);
    Ok(())
}

fn aes256_state(doc: &Document, user: &str, owner: &str) -> Result<EncryptionState> {
    let _ = doc;
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let version = EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: owner,
        user_password: user,
        permissions: Permissions::all(),
    };
    Ok(EncryptionState::try_from(version)?)
}
