//! The Properties dialog: what a document says about itself and what the app can see of it.

use adw::prelude::*;
use gtk::glib;
use specto_source::{Date, Metadata, Permissions};

use crate::i18n::{gettext, gettext_f};

/// What the dialog is made from.
pub struct Facts {
    pub name: String,
    /// The folder the file is in, where it has one.
    pub folder: Option<String>,
    /// The size of the file in bytes.
    pub bytes: u64,
    pub metadata: Metadata,
    /// The PDF version the file states, such as "1.7".
    pub version: &'static str,
    /// Width and height of every page as shown, in points.
    pub page_sizes: Vec<(f64, f64)>,
    /// What the file allows, if it is encrypted.
    pub permissions: Option<Permissions>,
    /// A password was needed to open the file.
    pub password: bool,
}

/// The dialog's rows, as label and value, in the order shown.
///
/// What the document doesn't state is left out. It may also be wrong: it is whatever the program
/// that wrote the file put there.
pub fn rows(facts: &Facts) -> Vec<(String, String)> {
    let m = &facts.metadata;
    let text = |label: String, value: &Option<String>| Some((label, value.clone()?));
    let date = |label: String, value: Option<Date>| Some((label, date_text(value?)));
    let rows = [
        Some((gettext("File Name"), facts.name.clone())),
        text(gettext("Folder"), &facts.folder),
        text(gettext("Title"), &m.title),
        text(gettext("Author"), &m.author),
        text(gettext("Subject"), &m.subject),
        text(gettext("Keywords"), &m.keywords),
        // Translators: the program a document was written in.
        text(gettext("Created With"), &m.creator),
        // Translators: the program that turned a document into a PDF.
        text(gettext("PDF Producer"), &m.producer),
        date(gettext("Created"), m.created),
        date(gettext("Modified"), m.modified),
        Some((gettext("Pages"), facts.page_sizes.len().to_string())),
        page_sizes_text(&facts.page_sizes).map(|size| (gettext("Page Size"), size)),
        Some((
            gettext("File Size"),
            glib::format_size(facts.bytes).to_string(),
        )),
        Some((gettext("PDF Version"), facts.version.to_owned())),
        Some((
            gettext("Protection"),
            match (facts.permissions.is_some(), facts.password) {
                (_, true) => gettext("Password to open"),
                (true, false) => gettext("Encrypted, opens without a password"),
                (false, false) => gettext("None"),
            },
        )),
        (facts.permissions.as_ref())
            .and_then(not_allowed)
            .map(|what| (gettext("Not Allowed"), what)),
    ];
    rows.into_iter().flatten().collect()
}

/// What `permissions` rules out, as a list for people, if anything.
fn not_allowed(permissions: &Permissions) -> Option<String> {
    let ruled_out: Vec<String> = [
        (permissions.printing, gettext("Printing")),
        (permissions.copying, gettext("Copying text and pictures")),
        (permissions.changing, gettext("Changing the pages")),
        (
            permissions.annotating,
            gettext("Marking up and filling in forms"),
        ),
    ]
    .into_iter()
    .filter(|(allowed, _)| !allowed)
    .map(|(_, what)| what)
    .collect();
    (!ruled_out.is_empty()).then(|| ruled_out.join("\n"))
}

/// A date from a document, on the reader's clock and in the reader's language.
///
/// A date that isn't one (the 31st of February) is shown as the document states it.
fn date_text(date: Date) -> String {
    let zone = glib::TimeZone::from_offset(i32::from(date.utc_offset_minutes) * 60);
    glib::DateTime::new(
        &zone,
        i32::from(date.year),
        i32::from(date.month),
        i32::from(date.day),
        i32::from(date.hour),
        i32::from(date.minute),
        f64::from(date.second),
    )
    .and_then(|d| d.to_local())
    // The date and the time the way the reader's language writes them.
    .and_then(|d| d.format("%x %X"))
    .map_or_else(|_| date.to_string(), Into::into)
}

/// Paper sizes people know by name: shorter side, longer side (in points), name, and whether they
/// are measured in inches.
const PAPER_SIZES: [(f64, f64, &str, bool); 7] = [
    (841.9, 1190.6, "A3", false),
    (595.3, 841.9, "A4", false),
    (419.5, 595.3, "A5", false),
    (498.9, 708.7, "B5", false),
    (612.0, 792.0, "Letter", true),
    (612.0, 1008.0, "Legal", true),
    (792.0, 1224.0, "Tabloid", true),
];

/// How far a page may be from a paper size and still be called by its name, in points.
const PAPER_TOLERANCE: f64 = 2.0;

/// A page size for people: "210 × 297 mm (A4)", "11 × 8.5 in (Letter, landscape)", or just
/// "152 × 229 mm" for a size without a name.
///
/// Width first. `width` and `height` are in points.
fn page_size_text(width: f64, height: f64) -> String {
    let (short, long) = (width.min(height), width.max(height));
    let paper = PAPER_SIZES.iter().find(|(s, l, ..)| {
        (s - short).abs() <= PAPER_TOLERANCE && (l - long).abs() <= PAPER_TOLERANCE
    });
    let size = if paper.is_some_and(|p| p.3) {
        // "8.5", not "8.50".
        let inches = |points: f64| {
            let text = format!("{:.2}", points / 72.0);
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        };
        gettext_f(
            // Translators: a page size in inches, "8.5 × 11 in".
            "{width} × {height} in",
            &[("width", &inches(width)), ("height", &inches(height))],
        )
    } else {
        let millimeters = |points: f64| (points / 72.0 * 25.4).round();
        gettext_f(
            // Translators: a page size in millimeters, "210 × 297 mm".
            "{width} × {height} mm",
            &[
                ("width", &millimeters(width)),
                ("height", &millimeters(height)),
            ],
        )
    };
    match paper {
        Some((.., name, _)) if width > height => gettext_f(
            // Translators: a page size and the name of the paper, "297 × 210 mm (A4,
            // landscape)".
            "{size} ({paper}, landscape)",
            &[("size", &size), ("paper", name)],
        ),
        Some((.., name, _)) => gettext_f(
            // Translators: a page size and the name of the paper, "210 × 297 mm (A4)".
            "{size} ({paper})",
            &[("size", &size), ("paper", name)],
        ),
        None => size,
    }
}

/// The size of a document's pages: that of the first, with a note if others differ.
fn page_sizes_text(sizes: &[(f64, f64)]) -> Option<String> {
    let &(width, height) = sizes.first()?;
    let same = (sizes.iter()).all(|(w, h)| (w - width).abs() <= 1.0 && (h - height).abs() <= 1.0);
    let size = page_size_text(width, height);
    Some(if same {
        size
    } else {
        // Translators: {size} is the size of the first page, "210 × 297 mm (A4)".
        gettext_f("{size}, other pages differ", &[("size", &size)])
    })
}

/// Shows `rows` in a dialog over `parent`. The values can be selected and copied.
pub fn show(parent: &impl IsA<gtk::Widget>, rows: &[(String, String)]) -> adw::Dialog {
    let group = adw::PreferencesGroup::new();
    for (label, value) in rows {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(label))
            .subtitle(glib::markup_escape_text(value))
            .subtitle_selectable(true)
            .css_classes(["property"])
            .build();
        group.add(&row);
    }
    let page = adw::PreferencesPage::new();
    page.add(&group);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(gettext("Properties"))
        .content_width(460)
        .child(&view)
        .build();
    dialog.present(Some(parent));
    dialog
}

#[cfg(test)]
mod tests {
    use specto_source::{Date, Metadata, Permissions};

    use super::{Facts, page_size_text, page_sizes_text, rows};

    #[test]
    fn page_sizes_for_people() {
        assert_eq!(page_size_text(595.0, 842.0), "210 × 297 mm (A4)");
        assert_eq!(page_size_text(842.0, 595.0), "297 × 210 mm (A4, landscape)");
        assert_eq!(page_size_text(612.0, 792.0), "8.5 × 11 in (Letter)");
        assert_eq!(
            page_size_text(1224.0, 792.0),
            "17 × 11 in (Tabloid, landscape)"
        );
        assert_eq!(page_size_text(432.0, 648.0), "152 × 229 mm");

        assert_eq!(page_sizes_text(&[]), None);
        let a4 = (595.0, 842.0);
        assert_eq!(
            page_sizes_text(&[a4, (595.4, 841.6)]).unwrap(),
            "210 × 297 mm (A4)"
        );
        assert_eq!(
            page_sizes_text(&[a4, (842.0, 595.0)]).unwrap(),
            "210 × 297 mm (A4), other pages differ"
        );
    }

    #[test]
    fn rows_leave_out_what_isnt_stated() {
        let mut facts = Facts {
            name: "a.pdf".to_owned(),
            folder: None,
            bytes: 2048,
            metadata: Metadata::default(),
            version: "1.7",
            page_sizes: vec![(595.0, 842.0); 3],
            permissions: None,
            password: false,
        };
        let labels = |facts: &Facts| -> Vec<String> {
            rows(facts).into_iter().map(|(label, _)| label).collect()
        };
        assert_eq!(
            labels(&facts),
            [
                "File Name",
                "Pages",
                "Page Size",
                "File Size",
                "PDF Version",
                "Protection"
            ]
        );
        assert_eq!(rows(&facts)[1].1, "3");
        assert_eq!(rows(&facts)[5].1, "None");

        facts.metadata.title = Some("A Title".to_owned());
        // A date that isn't one is shown as stated.
        facts.metadata.created = Some(Date {
            year: 2024,
            month: 2,
            day: 31,
            hour: 9,
            minute: 5,
            second: 0,
            utc_offset_minutes: 60,
        });
        facts.permissions = Some(Permissions {
            printing: false,
            changing: true,
            copying: false,
            annotating: true,
        });
        let all = rows(&facts);
        let value = |label: &str| all.iter().find(|(l, _)| l == label).map(|(_, v)| &v[..]);
        assert_eq!(value("Title"), Some("A Title"));
        assert_eq!(value("Created"), Some("2024-02-31T09:05:00+01:00"));
        assert_eq!(
            value("Protection"),
            Some("Encrypted, opens without a password")
        );
        assert_eq!(
            value("Not Allowed"),
            Some("Printing\nCopying text and pictures")
        );
        facts.password = true;
        assert_eq!(rows(&facts).last().unwrap().0, "Not Allowed");
        assert!(rows(&facts).contains(&("Protection".to_owned(), "Password to open".to_owned())));
    }
}
