//! Annotations: markup that lies over a page without being part of its
//! content (highlights, drawings, shapes, notes).
//!
//! An [`Annotation`] here is plain data in the coordinates of its page of
//! the file (PDF space), so it stays where it is when the page is rotated
//! or moved in the document. [`import`] reads the ones a file already has.
//! What a document's annotations currently are (those of its files, minus
//! the deleted, plus the new) is kept by `specto-doc`; this crate knows
//! nothing of documents.
//!
//! Not every annotation in a file is markup: links belong to
//! `specto-source`, form fields will belong to the forms crate, and popups
//! are windows of other annotations. [`import`] leaves those out, and
//! saving leaves them alone.

mod import;
mod shapes;

use std::sync::Arc;

use specto_geom::{PdfBox, PdfPoint, PdfQuad};
pub use specto_source::Date;

pub use import::import;
pub use shapes::{AppearanceStream, GraphicsState, Shape, appearance, shapes};

/// A colour as PDF annotations have it: red, green and blue, each 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }
}

/// How text is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkupStyle {
    Highlight,
    Underline,
    StrikeOut,
    /// A wavy underline. Read from files; nothing here makes one.
    Squiggly,
}

/// What an annotation is, with what only that kind has.
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotKind {
    /// Marked text: one quad per stretch of text, usually one per line.
    TextMarkup {
        style: MarkupStyle,
        quads: Vec<PdfQuad>,
    },
    /// Freehand strokes, each a run of points.
    Ink {
        strokes: Vec<Vec<PdfPoint>>,
    },
    /// A rectangle along the annotation's `rect`.
    Square {
        fill: Option<Color>,
    },
    /// An ellipse inside the annotation's `rect`.
    Circle {
        fill: Option<Color>,
    },
    Line {
        from: PdfPoint,
        to: PdfPoint,
    },
    /// A picture placed on the page: a signature, for instance. Only for
    /// stamps made here; one read from a file is [`AnnotKind::Other`], since
    /// its appearance is all there is to know about it.
    Stamp(StampArt),
    /// Markup of a kind this crate has no parameters for (a text box, a
    /// note, a stamp, a polygon, …), named by its `/Subtype`. It is shown
    /// as the file draws it, and can be deleted but not changed.
    Other {
        subtype: String,
    },
}

/// What a stamp shows.
#[derive(Clone, Debug, PartialEq)]
pub enum StampArt {
    /// Strokes, as of a signature drawn by hand: they stay sharp at any
    /// zoom. In the unit square, scaled to the annotation's `rect`.
    Strokes(Arc<[Vec<(f32, f32)>]>),
    /// An image, shared rather than copied when the annotation is.
    Image(Arc<StampImage>),
}

/// The pixels of an image stamp: RGBA, 8 bits each, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A kind without its parameters, for naming an annotation ("Undo
/// Highlight") and for choosing how to treat it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnnotType {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Ink,
    Square,
    Circle,
    Line,
    Stamp,
    Other,
}

impl AnnotKind {
    pub fn annot_type(&self) -> AnnotType {
        match self {
            Self::TextMarkup { style, .. } => match style {
                MarkupStyle::Highlight => AnnotType::Highlight,
                MarkupStyle::Underline => AnnotType::Underline,
                MarkupStyle::StrikeOut => AnnotType::StrikeOut,
                MarkupStyle::Squiggly => AnnotType::Squiggly,
            },
            Self::Ink { .. } => AnnotType::Ink,
            Self::Square { .. } => AnnotType::Square,
            Self::Circle { .. } => AnnotType::Circle,
            Self::Line { .. } => AnnotType::Line,
            Self::Stamp(_) => AnnotType::Stamp,
            Self::Other { .. } => AnnotType::Other,
        }
    }
}

/// The `/F` flags of an annotation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(pub u32);

impl Flags {
    pub const HIDDEN: u32 = 1 << 1;
    pub const PRINT: u32 = 1 << 2;
    pub const NO_VIEW: u32 = 1 << 5;
    pub const READ_ONLY: u32 = 1 << 6;
    pub const LOCKED: u32 = 1 << 7;

    /// What a new annotation gets: it is printed with the page.
    pub const NEW: Self = Self(Self::PRINT);

    /// Whether the annotation is shown on screen.
    pub fn is_visible(self) -> bool {
        self.0 & (Self::HIDDEN | Self::NO_VIEW) == 0
    }

    /// Whether the file allows changing or deleting the annotation.
    pub fn is_editable(self) -> bool {
        self.0 & (Self::READ_ONLY | Self::LOCKED) == 0
    }
}

/// Where in its file an annotation was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Origin {
    /// Its position in the page's `/Annots` array, counting every entry.
    pub index: u32,
    /// Its object number and generation, unless it is written straight
    /// into the array.
    pub object: Option<(u32, u16)>,
}

/// How an annotation gets its look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    /// As the file draws it (its `/AP` stream), untouched.
    Original,
    /// The file gives none; it is drawn from the annotation's parameters.
    Missing,
    /// Made or changed here: drawn from the parameters, and an appearance
    /// is written for other programs when the document is saved.
    Generated,
}

/// One annotation on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub kind: AnnotKind,
    /// The area it takes up (`/Rect`), normalised.
    pub rect: PdfBox,
    /// Its colour (`/C`); for shapes, that of the outline.
    pub color: Option<Color>,
    /// 0 (invisible) to 1 (opaque): `/CA`.
    pub opacity: f32,
    /// The width of strokes and outlines, in points.
    pub line_width: f64,
    /// Who made it (`/T`).
    pub author: Option<String>,
    /// The note that goes with it (`/Contents`).
    pub contents: Option<String>,
    /// When it was last changed (`/M`).
    pub modified: Option<Date>,
    /// A name unique on its page (`/NM`), if the file gives one.
    pub name: Option<String>,
    pub flags: Flags,
    /// Where it was read from; `None` for one made here.
    pub origin: Option<Origin>,
    pub appearance: Appearance,
}

impl Annotation {
    /// A new annotation of `kind` over `rect`, with nothing else set.
    pub fn new(kind: AnnotKind, rect: PdfBox) -> Self {
        Self {
            kind,
            rect,
            color: None,
            opacity: 1.0,
            line_width: 1.0,
            author: None,
            contents: None,
            modified: None,
            name: None,
            flags: Flags::NEW,
            origin: None,
            appearance: Appearance::Generated,
        }
    }

    /// Marked text over `quads` (see [`AnnotKind::TextMarkup`]), in
    /// `color`. `None` without quads.
    pub fn text_markup(style: MarkupStyle, quads: Vec<PdfQuad>, color: Color) -> Option<Self> {
        let rect = quads
            .iter()
            .map(PdfQuad::bounds)
            .reduce(|a, b| a.union(&b))?;
        let mut a = Self::new(AnnotKind::TextMarkup { style, quads }, rect);
        a.color = Some(color);
        Some(a)
    }

    pub fn annot_type(&self) -> AnnotType {
        self.kind.annot_type()
    }

    /// Whether `p` is on the annotation: on its marked text for text
    /// markup, else within its area.
    pub fn contains(&self, p: PdfPoint) -> bool {
        match &self.kind {
            AnnotKind::TextMarkup { quads, .. } => quads.iter().any(|q| q.contains(p)),
            _ => self.rect.contains_inclusive(p),
        }
    }
}
