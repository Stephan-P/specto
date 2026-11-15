//! Forms: the fields a PDF can be filled in through (an AcroForm).
//!
//! A field has a name, a kind, a value, and one or more *widgets*: the places on the pages where
//! it is shown and filled in. Most fields have one. The buttons of a radio group are the widgets
//! of one field, and a text field can show on several pages.
//!
//! [`fields`] reads them. What the user fills in is kept by `specto-doc`, as changed values by
//! field. This crate knows nothing of documents being edited.
//!
//! Fields are in a tree in the file, in which a field passes its kind, flags and value on to
//! those below it. Trees in broken files can be cyclic or huge, so the walk is bounded.

mod import;

use specto_annot::Color;
use specto_geom::PdfBox;

pub use import::fields;

/// Names a field: the number and generation of its dictionary's object in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(pub u32, pub u16);

/// What kind of field it is.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    /// Text to type in.
    Text {
        /// More than one line.
        multiline: bool,
        /// Shown as dots.
        password: bool,
        /// How many characters it takes at most, if it says.
        max_length: Option<u32>,
    },
    /// A box to tick. Its widgets (usually one) each say what they are called when ticked.
    Checkbox,
    /// One of several to choose. Each widget is one of the choices.
    Radio,
    /// A list or a menu to choose from. Read, but not filled in here yet.
    Choice,
    /// A button to press, which fills nothing in.
    Button,
    /// A place for a digital signature.
    Signature,
    /// A kind this crate doesn't know.
    Other,
}

/// What a field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    /// The text of a text field.
    Text(String),
    /// Whether a checkbox is ticked.
    Check(bool),
    /// Which button of a radio group is chosen, by the name of its state. `None` for none.
    Choice(Option<String>),
}

/// One place where a field is shown on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    /// The page of the file it is on, from 0.
    pub page: usize,
    /// The area it takes up on the page.
    pub rect: PdfBox,
    /// The number and generation of its annotation's object. For a field with a single widget
    /// this can be the field's own object.
    pub object: (u32, u16),
    /// For a checkbox or a radio button: what its "on" state is called. Its other state is
    /// `Off`.
    pub on_state: Option<String>,
    /// Hidden by its flags: not shown, and not to be filled in.
    pub hidden: bool,
}

/// How a text field sets its text (its default appearance string, `/DA`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextLook {
    /// The name of the font among the form's resources, such as `Helv`.
    pub font: String,
    /// The size in points, or 0 for "as large as fits".
    pub size: f64,
    pub color: Color,
    /// 0 for left-aligned, 1 for centered, 2 for right-aligned.
    pub alignment: u8,
}

impl Default for TextLook {
    fn default() -> Self {
        Self {
            font: "Helv".to_owned(),
            size: 0.0,
            color: Color::new(0.0, 0.0, 0.0),
            alignment: 0,
        }
    }
}

/// A field of a form.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: FieldId,
    /// Its full name: the names down the tree to it, joined by dots.
    pub name: String,
    /// What to call it for the user (`/TU`), if the form says.
    pub label: Option<String>,
    pub kind: FieldKind,
    /// The form doesn't let it be changed.
    pub read_only: bool,
    /// The form wants it filled in.
    pub required: bool,
    /// What it holds in the file. `None` for kinds without a value here.
    pub value: Option<FieldValue>,
    pub look: TextLook,
    pub widgets: Vec<Widget>,
}

impl Field {
    /// Whether the field can be filled in here: a text field or a checkbox or radio group that
    /// the form lets be changed.
    pub fn is_fillable(&self) -> bool {
        !self.read_only
            && matches!(
                self.kind,
                FieldKind::Text { .. } | FieldKind::Checkbox | FieldKind::Radio
            )
    }
}
