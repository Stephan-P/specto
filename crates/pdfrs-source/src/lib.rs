//! Read access to the structure of a PDF: opening (incl. decryption),
//! metadata, the outline (table of contents) and links.
//!
//! Rendering lives in `pdfrs-render`; this crate is about everything else a
//! viewer reads from the file. Everything here is defensive: outline and name
//! trees in broken files can be cyclic or huge, so every walk is bounded.

mod labels;
mod text;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use hayro_syntax::object::dict::keys::{
    A, ANNOTS, D, DEST, DESTS, FIRST, KIDS, LIMITS, NAMES, NEXT, NUMS, OUTLINES, P, PAGE_LABELS,
    RECT, S, ST, SUBTYPE, TITLE, URI,
};
use hayro_syntax::object::{Array, DateTime, Dict, MaybeRef, Name, Object, ObjectIdentifier};
use hayro_syntax::{DecryptionError, LoadPdfError, Pdf, PdfVersion};
use pdfrs_geom::{PdfBox, PdfPoint};

pub use hayro_syntax;
pub use labels::{LabelRange, LabelStyle, PageLabels};
pub use text::decode_text;

/// Outline items beyond this many are ignored (a guard against huge or
/// cyclic outlines in broken files).
const MAX_OUTLINE_ITEMS: usize = 100_000;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    #[error("the document is password protected")]
    PasswordRequired,
    #[error("the document uses an unsupported encryption scheme")]
    UnsupportedEncryption,
    #[error("the document's encryption dictionary is invalid")]
    InvalidEncryption,
    #[error("not a valid PDF file")]
    Invalid,
}

/// Lets hayro read from an `Arc<[u8]>` shared between threads without copying.
struct SharedBytes(Arc<[u8]>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Parses (and if needed decrypts) a PDF. Parsing is lazy: this reads the
/// cross-reference table and page tree, not the page contents.
pub fn open_pdf(data: Arc<[u8]>, password: Option<&str>) -> Result<Pdf, OpenError> {
    Pdf::new_with_password(Arc::new(SharedBytes(data)), password.unwrap_or("")).map_err(|e| match e
    {
        LoadPdfError::Decryption(DecryptionError::PasswordProtected) => OpenError::PasswordRequired,
        LoadPdfError::Decryption(DecryptionError::UnsupportedAlgorithm) => {
            OpenError::UnsupportedEncryption
        }
        LoadPdfError::Decryption(_) => OpenError::InvalidEncryption,
        LoadPdfError::Invalid => OpenError::Invalid,
    })
}

/// Document information (the `/Info` dictionary).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub created: Option<Date>,
    pub modified: Option<Date>,
}

/// A date and time as a document states it: on the clock of wherever it was
/// written, with that place's offset from UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Minutes east of UTC.
    pub utc_offset_minutes: i16,
}

/// ISO 8601 with the offset, e.g. `2024-03-05T14:22:10+01:00`.
impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let offset = self.utc_offset_minutes;
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            if offset < 0 { '-' } else { '+' },
            offset.unsigned_abs() / 60,
            offset.unsigned_abs() % 60,
        )
    }
}

impl From<DateTime> for Date {
    fn from(d: DateTime) -> Self {
        let hours = i16::from(d.utc_offset_hour);
        let minutes = i16::from(d.utc_offset_minute);
        Self {
            year: d.year,
            month: d.month,
            day: d.day,
            hour: d.hour,
            minute: d.minute,
            second: d.second,
            utc_offset_minutes: hours * 60 + if hours < 0 { -minutes } else { minutes },
        }
    }
}

/// Where a destination points: a page and, optionally, a position on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Destination {
    /// Zero-based page index.
    pub page: usize,
    /// The point that should end up at the top-left of the view, in PDF user
    /// space of that page. Either coordinate may be missing ("keep current").
    pub left: Option<f64>,
    pub top: Option<f64>,
}

impl Destination {
    /// The target position as a PDF-space point, with missing coordinates
    /// filled from `fallback` (usually the page's visible top-left corner).
    pub fn point_or(&self, fallback: PdfPoint) -> PdfPoint {
        PdfPoint::new(
            self.left.unwrap_or(fallback.x),
            self.top.unwrap_or(fallback.y),
        )
    }
}

/// Built-in navigation of the `/Named` action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedAction {
    NextPage,
    PrevPage,
    FirstPage,
    LastPage,
}

/// What activating a link or outline item does.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Go to a place in this document.
    Destination(Destination),
    /// Open a URI (usually a web page). Opening it is the frontend's call.
    Uri(String),
    Named(NamedAction),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    /// `None` for items without a target, or with one we don't support
    /// (e.g. other files, JavaScript).
    pub target: Option<Target>,
    pub children: Vec<OutlineItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// The clickable area in PDF user space of its page.
    pub rect: PdfBox,
    pub target: Target,
}

impl Link {
    pub fn contains(&self, p: PdfPoint) -> bool {
        self.rect.min.x <= p.x
            && p.x <= self.rect.max.x
            && self.rect.min.y <= p.y
            && p.y <= self.rect.max.y
    }
}

/// An opened PDF, for everything except rendering.
pub struct SourceDocument {
    pdf: Pdf,
    /// Page object id → page index, for resolving destinations.
    page_ids: HashMap<ObjectIdentifier, usize>,
}

impl SourceDocument {
    pub fn open(data: impl Into<Arc<[u8]>>, password: Option<&str>) -> Result<Self, OpenError> {
        Ok(Self::from_pdf(open_pdf(data.into(), password)?))
    }

    pub fn from_pdf(pdf: Pdf) -> Self {
        let page_ids = pdf
            .pages()
            .iter()
            .enumerate()
            .filter_map(|(i, p)| Some((p.raw().obj_id()?, i)))
            .collect();
        Self { pdf, page_ids }
    }

    pub fn pdf(&self) -> &Pdf {
        &self.pdf
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    pub fn metadata(&self) -> Metadata {
        let m = self.pdf.metadata();
        let text = |b: &Option<Box<[u8]>>| {
            b.as_deref()
                .map(decode_text)
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        Metadata {
            title: text(&m.title),
            author: text(&m.author),
            subject: text(&m.subject),
            keywords: text(&m.keywords),
            creator: text(&m.creator),
            producer: text(&m.producer),
            created: m.creation_date.map(Date::from),
            modified: m.modification_date.map(Date::from),
        }
    }

    /// The version of PDF the file says it is written in, e.g. "1.7".
    pub fn version(&self) -> &'static str {
        match self.pdf.version() {
            PdfVersion::Pdf10 => "1.0",
            PdfVersion::Pdf11 => "1.1",
            PdfVersion::Pdf12 => "1.2",
            PdfVersion::Pdf13 => "1.3",
            PdfVersion::Pdf14 => "1.4",
            PdfVersion::Pdf15 => "1.5",
            PdfVersion::Pdf16 => "1.6",
            PdfVersion::Pdf17 => "1.7",
            PdfVersion::Pdf20 => "2.0",
        }
    }

    fn catalog(&self) -> Option<Dict<'_>> {
        let xref = self.pdf.xref();
        xref.get::<Dict<'_>>(xref.root_id())
    }

    /// The outline (bookmarks / table of contents). Empty if there is none.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let Some(first) = self
            .catalog()
            .and_then(|c| c.get::<Dict<'_>>(OUTLINES))
            .and_then(|o| o.get::<Dict<'_>>(FIRST))
        else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut budget = MAX_OUTLINE_ITEMS;
        self.outline_level(first, 0, &mut seen, &mut budget)
    }

    fn outline_level(
        &self,
        first: Dict<'_>,
        depth: usize,
        seen: &mut HashSet<ObjectIdentifier>,
        budget: &mut usize,
    ) -> Vec<OutlineItem> {
        let mut items = Vec::new();
        let mut node = Some(first);
        while let Some(item) = node {
            if *budget == 0 {
                break;
            }
            // Siblings and children are references; a repeat means a cycle.
            if let Some(id) = item.obj_id()
                && !seen.insert(id)
            {
                break;
            }
            *budget -= 1;
            let title = item
                .get::<hayro_syntax::object::String<'_>>(TITLE)
                .map(|t| decode_text(t.as_bytes()).trim().to_owned())
                .unwrap_or_default();
            let children = match item.get::<Dict<'_>>(FIRST) {
                Some(child) if depth < MAX_DEPTH => {
                    self.outline_level(child, depth + 1, seen, budget)
                }
                _ => Vec::new(),
            };
            items.push(OutlineItem {
                title,
                target: self.target_of(&item),
                children,
            });
            node = item.get::<Dict<'_>>(NEXT);
        }
        items
    }

    /// The document's page labels, or `None` if it has none.
    pub fn page_labels(&self) -> Option<PageLabels> {
        let root = self.catalog()?.get::<Dict<'_>>(PAGE_LABELS)?;
        let mut ranges = Vec::new();
        let mut budget = 100_000;
        collect_label_ranges(&root, 0, &mut ranges, &mut budget);
        PageLabels::new(ranges, self.page_count())
    }

    /// Links on page `page`, in drawing order.
    pub fn links(&self, page: usize) -> Vec<Link> {
        let Some(p) = self.pdf.pages().get(page) else {
            return Vec::new();
        };
        let Some(annots) = p.raw().get::<Array<'_>>(ANNOTS) else {
            return Vec::new();
        };
        annots
            .iter::<Dict<'_>>()
            .take(10_000)
            .filter(|a| a.get::<Name<'_>>(SUBTYPE).as_deref() == Some(b"Link"))
            .filter_map(|a| {
                let r: Vec<f64> = a.get::<Array<'_>>(RECT)?.iter::<f64>().collect();
                let [x0, y0, x1, y1] = r[..] else {
                    return None;
                };
                Some(Link {
                    rect: PdfBox::new(
                        PdfPoint::new(x0.min(x1), y0.min(y1)),
                        PdfPoint::new(x0.max(x1), y0.max(y1)),
                    ),
                    target: self.target_of(&a)?,
                })
            })
            .collect()
    }

    /// The topmost link at `p` (PDF space) on page `page`.
    pub fn link_at(&self, page: usize, p: PdfPoint) -> Option<Link> {
        self.links(page).into_iter().rev().find(|l| l.contains(p))
    }

    /// The target of an outline item or link annotation: `/Dest` or `/A`.
    fn target_of(&self, dict: &Dict<'_>) -> Option<Target> {
        if let Some(dest) = dict.get::<Object<'_>>(DEST) {
            return self.resolve_dest(dest, 0).map(Target::Destination);
        }
        let action = dict.get::<Dict<'_>>(A)?;
        match action.get::<Name<'_>>(S)?.as_ref() {
            b"GoTo" => self
                .resolve_dest(action.get::<Object<'_>>(D)?, 0)
                .map(Target::Destination),
            b"URI" => {
                let uri = action.get::<hayro_syntax::object::String<'_>>(URI)?;
                // URIs are 7-bit ASCII by spec; be lenient with what's there.
                let uri = String::from_utf8_lossy(uri.as_bytes()).trim().to_owned();
                (!uri.is_empty()).then_some(Target::Uri(uri))
            }
            b"Named" => {
                let name = action.get::<Name<'_>>(b"N")?;
                Some(Target::Named(match name.as_ref() {
                    b"NextPage" => NamedAction::NextPage,
                    b"PrevPage" => NamedAction::PrevPage,
                    b"FirstPage" => NamedAction::FirstPage,
                    b"LastPage" => NamedAction::LastPage,
                    _ => return None,
                }))
            }
            // GoToR, Launch, JavaScript, …: not supported.
            _ => None,
        }
    }

    /// Resolves an explicit destination (array), a named one (name or
    /// string), or a destination dictionary (`<< /D [...] >>`).
    fn resolve_dest(&self, dest: Object<'_>, depth: usize) -> Option<Destination> {
        if depth > 4 {
            return None;
        }
        match dest {
            Object::Array(a) => self.explicit_dest(&a),
            Object::Dict(d) => self.resolve_dest(d.get::<Object<'_>>(D)?, depth + 1),
            Object::Name(n) => self.resolve_dest(self.named_dest(n.as_ref())?, depth + 1),
            Object::String(s) => self.resolve_dest(self.named_dest(s.as_bytes())?, depth + 1),
            _ => None,
        }
    }

    /// `[page /XYZ left top zoom]`, `[page /Fit]`, `[page /FitH top]`, …
    fn explicit_dest(&self, a: &Array<'_>) -> Option<Destination> {
        let mut items = a.raw_iter();
        let page = match items.next()? {
            MaybeRef::Ref(r) => *self.page_ids.get(&ObjectIdentifier::from(r))?,
            // Remote destinations use page numbers; some broken files use
            // them for local ones too.
            MaybeRef::NotRef(Object::Number(n)) => usize::try_from(n.as_i64())
                .ok()
                .filter(|&p| p < self.page_count())?,
            _ => return None,
        };
        let kind = match items.next() {
            Some(MaybeRef::NotRef(Object::Name(n))) => n.as_ref().to_vec(),
            _ => Vec::new(),
        };
        let mut num = || match items.next() {
            Some(MaybeRef::NotRef(Object::Number(n))) => Some(n.as_f64()),
            _ => None, // null or missing: keep the current value
        };
        let (left, top) = match kind.as_slice() {
            b"XYZ" => {
                let left = num();
                (left, num())
            }
            b"FitH" | b"FitBH" => (None, num()),
            b"FitV" | b"FitBV" => (num(), None),
            b"FitR" => {
                let (l, _b, _r, t) = (num(), num(), num(), num());
                (l, t)
            }
            _ => (None, None), // Fit, FitB: the whole page
        };
        Some(Destination { page, left, top })
    }

    /// Looks up a named destination in `/Dests` (PDF 1.1) or the `/Names`
    /// name tree.
    fn named_dest(&self, name: &[u8]) -> Option<Object<'_>> {
        let catalog = self.catalog()?;
        if let Some(d) = catalog
            .get::<Dict<'_>>(DESTS)
            .and_then(|d| d.get::<Object<'_>>(name))
        {
            return Some(d);
        }
        let tree = catalog.get::<Dict<'_>>(NAMES)?.get::<Dict<'_>>(DESTS)?;
        name_tree_lookup(tree, name, 0)
    }
}

/// Collects the ranges of a page-label number tree (`/Nums` pairs of start
/// page and label dictionary, possibly spread over `/Kids`).
fn collect_label_ranges(
    node: &Dict<'_>,
    depth: usize,
    out: &mut Vec<LabelRange>,
    budget: &mut usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Some(nums) = node.get::<Array<'_>>(NUMS) {
        let mut it = nums.iter::<Object<'_>>();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let (Object::Number(start), Object::Dict(d)) = (k, v) else {
                continue;
            };
            let Ok(start) = usize::try_from(start.as_i64()) else {
                continue;
            };
            let style = match d.get::<Name<'_>>(S).as_deref() {
                Some(b"D") => LabelStyle::Decimal,
                Some(b"R") => LabelStyle::UpperRoman,
                Some(b"r") => LabelStyle::LowerRoman,
                Some(b"A") => LabelStyle::UpperLetters,
                Some(b"a") => LabelStyle::LowerLetters,
                _ => LabelStyle::None,
            };
            let prefix = d
                .get::<hayro_syntax::object::String<'_>>(P)
                .map(|p| decode_text(p.as_bytes()))
                .unwrap_or_default();
            let first = d
                .get::<f64>(ST)
                .filter(|v| v.is_finite() && *v >= 1.0)
                .map_or(1, |v| v.min(u32::MAX as f64) as u32);
            out.push(LabelRange {
                start,
                style,
                prefix,
                first,
            });
        }
    }
    if let Some(kids) = node.get::<Array<'_>>(KIDS) {
        for kid in kids.iter::<Dict<'_>>().take(10_000) {
            collect_label_ranges(&kid, depth + 1, out, budget);
        }
    }
}

/// Finds `key` in a name tree, descending only into kids whose `/Limits`
/// can contain it.
fn name_tree_lookup<'a>(node: Dict<'a>, key: &[u8], depth: usize) -> Option<Object<'a>> {
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(names) = node.get::<Array<'_>>(NAMES) {
        let mut it = names.raw_iter();
        while let (Some(k), Some(v)) = (it.next(), it.next()) {
            if let MaybeRef::NotRef(Object::String(s)) = k
                && s.as_bytes() == key
            {
                return match v {
                    MaybeRef::NotRef(o) => Some(o),
                    MaybeRef::Ref(_) => names_value(&names, key),
                };
            }
        }
    }
    let kids = node.get::<Array<'_>>(KIDS)?;
    for kid in kids.iter::<Dict<'_>>().take(10_000) {
        let in_range = kid.get::<Array<'_>>(LIMITS).is_none_or(|limits| {
            let mut l = limits.iter::<hayro_syntax::object::String<'_>>();
            match (l.next(), l.next()) {
                (Some(lo), Some(hi)) => lo.as_bytes() <= key && key <= hi.as_bytes(),
                _ => true,
            }
        });
        if in_range && let Some(v) = name_tree_lookup(kid, key, depth + 1) {
            return Some(v);
        }
    }
    None
}

/// Value for `key` in a `/Names` array, resolving references (the raw
/// iterator above doesn't).
fn names_value<'a>(names: &Array<'a>, key: &[u8]) -> Option<Object<'a>> {
    let mut it = names.iter::<Object<'_>>();
    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        if let Object::String(s) = k
            && s.as_bytes() == key
        {
            return Some(v);
        }
    }
    None
}
