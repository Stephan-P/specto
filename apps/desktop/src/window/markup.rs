//! Markup: the bar with its tools and colors, typing text onto the page, signatures, and the list
//! in the sidebar.

use super::*;

/// The widgets of the markup bar that the window keeps hold of.
pub(super) struct MarkupBar {
    pub(super) markup_bar: gtk::Revealer,
    pub(super) markup_toggle: gtk::ToggleButton,
    pub(super) color_swatch: gtk::Box,
    pub(super) signature_menu: gtk::Popover,
    pub(super) text_controls: gtk::Revealer,
    pub(super) text_font: gtk::DropDown,
    pub(super) text_bold: gtk::ToggleButton,
    pub(super) text_italic: gtk::ToggleButton,
    pub(super) text_size: gtk::SpinButton,
}

/// Builds the markup bar and its button: the bar goes over the page view (the top of `content`),
/// the button into `header`.
pub(super) fn build_bar(header: &adw::HeaderBar, content: &gtk::Box) -> MarkupBar {
    // The markup bar, under the header while the pencil is pressed in: what to mark text with,
    // and in which color. A tool that is pressed in marks every selection as it is made. The
    // colors also recolor the annotation that is picked.
    let tools = gtk::Box::builder().css_classes(["linked"]).build();
    for (label, tool, shortcut) in [
        (gettext("Highlight"), "highlight", "H"),
        (gettext("Underline"), "underline", "U"),
        (gettext("Strikethrough"), "strike-out", "X"),
    ] {
        let kind = tool_named(tool).unwrap_or(HIGHLIGHTER);
        tools.append(
            &gtk::ToggleButton::builder()
                .child(&tool_icon(kind))
                .tooltip_text(gettext_f(
                    "{tool}: marks the selected text, and what is selected from now on ({shortcut} marks once)",
                    &[
                        ("tool", &label),
                        ("shortcut", &primary(&format!("Shift+{shortcut}"))),
                    ],
                ))
                .action_name("win.markup-tool")
                .action_target(&tool.to_variant())
                .build(),
        );
    }
    // The drawing tools, as pictures of what they draw: words would make the bar too wide.
    let draw_tools = gtk::Box::builder().css_classes(["linked"]).build();
    for (tool, tip) in [
        ("draw", gettext("Draw freehand")),
        ("rectangle", gettext("Draw rectangles")),
        ("ellipse", gettext("Draw ellipses")),
        ("line", gettext("Draw lines")),
        ("text", gettext("Add text: click where it goes")),
        ("note", gettext("Add notes: click where one goes")),
    ] {
        let kind = tool_named(tool).unwrap_or(HIGHLIGHTER);
        draw_tools.append(
            &gtk::ToggleButton::builder()
                .child(&tool_icon(kind))
                .tooltip_text(tip)
                .action_name("win.markup-tool")
                .action_target(&tool.to_variant())
                .build(),
        );
    }
    // The colors, under a button that shows the one in use.
    let colors = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let color_menu = gtk::Popover::builder().child(&colors).build();
    for (name, label, _) in markup_colors() {
        let swatch = gtk::Box::builder()
            .css_classes(["markup-swatch", &format!("markup-{name}")])
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let choose = gtk::ToggleButton::builder()
            .child(&swatch)
            .tooltip_text(label)
            .css_classes(["flat", "circular"])
            .action_name("win.markup-color")
            .action_target(&name.to_variant())
            .build();
        let menu = color_menu.downgrade();
        choose.connect_clicked(move |_| {
            if let Some(menu) = menu.upgrade() {
                menu.popdown();
            }
        });
        colors.append(&choose);
    }
    let color_swatch = gtk::Box::builder()
        .css_classes(["markup-swatch", "markup-yellow"])
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    let color_button = gtk::MenuButton::builder()
        .child(&color_swatch)
        .tooltip_text(gettext("Color"))
        .popover(&color_menu)
        .build();
    let markup_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .halign(gtk::Align::Center)
        .css_classes(["toolbar"])
        .build();
    markup_row.append(&tools);
    markup_row.append(&draw_tools);
    // Signatures: the saved ones to choose from, and making a new one.
    let signature_menu = gtk::Popover::new();
    let signature_button = gtk::MenuButton::builder()
        .child(&signature_icon())
        .tooltip_text(gettext("Signature"))
        .popover(&signature_menu)
        .build();
    markup_row.append(&signature_button);
    markup_row.append(&color_button);
    // How text boxes are set: shown with the text tool, and while a text box is picked or typed
    // into.
    let font_names = gtk::StringList::new(&[
        &gettext("Sans-serif"),
        &gettext("Serif"),
        &gettext("Monospace"),
    ]);
    // Each name in the kind of font it names.
    let in_its_font = gtk::SignalListItemFactory::new();
    in_its_font.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
        item.set_child(Some(&gtk::Label::builder().xalign(0.0).build()));
    });
    in_its_font.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
        let (Some(label), Some(name)) = (
            item.child().and_downcast::<gtk::Label>(),
            item.item().and_downcast::<gtk::StringObject>(),
        ) else {
            return;
        };
        label.set_label(&name.string());
        let class = ["font-sans", "font-serif", "font-mono"]
            .get(item.position() as usize)
            .copied()
            .unwrap_or("font-sans");
        label.set_css_classes(&[class]);
    });
    let text_font = gtk::DropDown::builder()
        .model(&font_names)
        .factory(&in_its_font)
        .tooltip_text(gettext("Font"))
        .build();
    let text_bold = gtk::ToggleButton::builder()
        .icon_name("format-text-bold-symbolic")
        .tooltip_text(gettext("Bold"))
        .build();
    let text_italic = gtk::ToggleButton::builder()
        .icon_name("format-text-italic-symbolic")
        .tooltip_text(gettext("Italic"))
        .build();
    let text_weight = gtk::Box::builder().css_classes(["linked"]).build();
    text_weight.append(&text_bold);
    text_weight.append(&text_italic);
    let text_size = gtk::SpinButton::with_range(6.0, 96.0, 1.0);
    text_size.set_value(12.0);
    // No wider than two digits need: the bar is full as it is.
    text_size.set_width_chars(2);
    text_size.set_max_width_chars(2);
    text_size.set_tooltip_text(Some(&gettext("Text size in points")));
    let text_controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    text_controls.append(&text_font);
    text_controls.append(&text_weight);
    text_controls.append(&text_size);
    // They slide in and out beside the other tools.
    let text_controls = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideRight)
        .child(&text_controls)
        .build();
    markup_row.append(&text_controls);
    // Over the page view only: the sidebar keeps its full height.
    let markup_strip = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["markup-bar"])
        .build();
    // Never wider than the page view: with everything on it, the bar is wider than a small
    // window has room for, and it mustn't push the sidebar or the page about as its controls
    // come and go. What doesn't fit is scrolled to.
    let markup_scroller = gtk::ScrolledWindow::builder()
        .child(&markup_row)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .build();
    markup_strip.append(&markup_scroller);
    let markup_bar = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&markup_strip)
        .build();
    content.prepend(&markup_bar);
    let markup_toggle = gtk::ToggleButton::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(gettext("Markup"))
        .build();
    markup_bar
        .bind_property("reveal-child", &markup_toggle, "active")
        .bidirectional()
        .sync_create()
        .build();
    header.pack_end(&markup_toggle);
    MarkupBar {
        markup_bar,
        markup_toggle,
        color_swatch,
        signature_menu,
        text_controls,
        text_font,
        text_bold,
        text_italic,
        text_size,
    }
}

/// The colors of the markup bar: a name (for the action's state and the style class of the swatch),
/// what a tooltip calls it, and the color.
///
/// Light ones for highlighting first, then a strong red, which is what lines under and through text
/// start with, as in Preview.
pub(super) fn markup_colors() -> [(&'static str, String, Color); 7] {
    [
        ("yellow", gettext("Yellow"), Color::new(1.0, 0.92, 0.23)),
        ("green", gettext("Green"), Color::new(0.56, 0.94, 0.64)),
        ("blue", gettext("Blue"), Color::new(0.6, 0.76, 0.95)),
        ("pink", gettext("Pink"), Color::new(0.97, 0.66, 0.82)),
        ("purple", gettext("Purple"), Color::new(0.86, 0.54, 0.87)),
        ("red", gettext("Red"), Color::new(0.88, 0.11, 0.14)),
        ("black", gettext("Black"), Color::new(0.14, 0.14, 0.14)),
    ]
}

/// The color of the markup bar called `name` (yellow if there is none).
pub(super) fn color_named(name: &str) -> Color {
    let colors = markup_colors();
    colors
        .iter()
        .find(|(n, _, _)| *n == name)
        .map_or(colors[0].2, |(_, _, c)| *c)
}

/// A picture of what a drawing tool draws, for its button: there are no stock icons for these.
///
/// Drawn in the button's text color, so it follows the light and the dark style.
pub(super) fn tool_icon(kind: Tool) -> gtk::DrawingArea {
    let icon = gtk::DrawingArea::builder()
        .content_width(16)
        .content_height(16)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    icon.set_draw_func(move |area, cr, width, height| {
        let color = area.color();
        cr.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            color.alpha().into(),
        );
        cr.set_line_width(1.5);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        // In a square of 16, whatever size the button gives.
        cr.scale(f64::from(width) / 16.0, f64::from(height) / 16.0);
        match kind {
            // A stroke as of a pen: a wave.
            Tool::Draw(DraftKind::Ink) => {
                cr.move_to(1.5, 11.0);
                cr.curve_to(4.0, 1.0, 7.0, 1.0, 8.0, 8.0);
                cr.curve_to(9.0, 15.0, 12.0, 15.0, 14.5, 5.0);
            }
            Tool::Draw(DraftKind::Square) => cr.rectangle(1.75, 3.25, 12.5, 9.5),
            Tool::Draw(DraftKind::Circle) => {
                let _ = cr.save();
                cr.translate(8.0, 8.0);
                cr.scale(6.25, 4.75);
                cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
                let _ = cr.restore();
            }
            Tool::Draw(DraftKind::Line) => {
                cr.move_to(2.5, 13.5);
                cr.line_to(13.5, 2.5);
            }
            // A capital T.
            Tool::Text => {
                cr.move_to(3.0, 3.0);
                cr.line_to(13.0, 3.0);
                cr.move_to(8.0, 3.0);
                cr.line_to(8.0, 13.5);
            }
            // A slip of paper with writing on it, like the mark on the page.
            Tool::Note => {
                cr.rectangle(2.25, 2.25, 11.5, 11.5);
                cr.move_to(5.0, 6.0);
                cr.line_to(11.0, 6.0);
                cr.move_to(5.0, 9.5);
                cr.line_to(9.0, 9.5);
            }
            // The text tools: a letter, with what the tool does to it.
            Tool::Mark(style) => {
                let letter = |cr: &gtk::cairo::Context, top: f64, bottom: f64| {
                    let bar = bottom - (bottom - top) * 0.36;
                    cr.move_to(4.0, bottom);
                    cr.line_to(8.0, top);
                    cr.line_to(12.0, bottom);
                    cr.move_to(5.4, bar);
                    cr.line_to(10.6, bar);
                };
                match style {
                    // A band of the marker behind it.
                    MarkupStyle::Highlight => {
                        let _ = cr.save();
                        cr.set_source_rgba(
                            color.red().into(),
                            color.green().into(),
                            color.blue().into(),
                            f64::from(color.alpha()) * 0.3,
                        );
                        cr.rectangle(1.0, 5.5, 14.0, 8.5);
                        let _ = cr.fill();
                        let _ = cr.restore();
                        letter(cr, 2.5, 12.5);
                    }
                    MarkupStyle::Underline | MarkupStyle::Squiggly => {
                        letter(cr, 1.5, 11.0);
                        cr.move_to(2.5, 14.5);
                        cr.line_to(13.5, 14.5);
                    }
                    MarkupStyle::StrikeOut => {
                        letter(cr, 2.5, 13.5);
                        cr.move_to(1.5, 7.5);
                        cr.line_to(14.5, 7.5);
                    }
                }
            }
        }
        let _ = cr.stroke();
    });
    icon
}

/// The picture on the signature button: a scribble over a line.
pub(super) fn signature_icon() -> gtk::DrawingArea {
    let icon = gtk::DrawingArea::builder()
        .content_width(18)
        .content_height(16)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    icon.set_draw_func(|area, cr, width, height| {
        let color = area.color();
        cr.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            color.alpha().into(),
        );
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        cr.scale(f64::from(width) / 18.0, f64::from(height) / 16.0);
        cr.set_line_width(1.4);
        cr.move_to(2.0, 10.0);
        cr.curve_to(4.0, 0.0, 7.0, 0.0, 6.0, 9.0);
        cr.curve_to(5.5, 13.0, 9.0, 3.0, 10.5, 7.0);
        cr.curve_to(11.5, 10.0, 13.0, 9.0, 16.0, 6.0);
        let _ = cr.stroke();
        cr.set_line_width(1.0);
        cr.move_to(1.5, 14.0);
        cr.line_to(16.5, 14.0);
        let _ = cr.stroke();
    });
    icon
}

/// A tool of the markup bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    /// Marks text as it is selected.
    Mark(MarkupStyle),
    /// Draws where the pointer is dragged.
    Draw(DraftKind),
    /// Puts a text box where the page is clicked, once its text is typed.
    Text,
    /// Puts a note where the page is clicked.
    Note,
}

/// The list a choice field of the form is chosen from, while it shows.
pub(super) struct Choices {
    pub(super) popover: gtk::Popover,
    /// What it lists.
    pub(super) labels: Vec<String>,
    /// The line to type a choice of one's own, for a field that takes one.
    pub(super) entry: Option<gtk::Entry>,
    /// The tick of each row, for a list that takes several.
    pub(super) ticks: Vec<gtk::CheckButton>,
}

/// The area in the page view that has `corners`.
fn area_of(corners: [ViewPoint; 4]) -> gdk::Rectangle {
    let (xs, ys) = (corners.map(|c| c.x), corners.map(|c| c.y));
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    gdk::Rectangle::new(
        min(xs) as i32,
        min(ys) as i32,
        (max(xs) - min(xs)).ceil() as i32,
        (max(ys) - min(ys)).ceil() as i32,
    )
}

/// Typing into a text box on the page.
pub(super) struct Typing {
    /// The text field over the page.
    pub(super) view: gtk::Widget,
    /// What kind of text field it is.
    pub(super) input: TypingInput,
    /// Where the text box is.
    pub(super) place: TextBoxPlace,
    /// The size and font of what is typed, which the markup bar can change meanwhile.
    pub(super) style: Cell<(f64, TextFont)>,
    /// What the text is for.
    pub(super) target: TypingTarget,
    /// Where the field is over the page view: its left and top edge, and its width.
    pub(super) area: Cell<(i32, i32, i32)>,
}

/// The text field text is typed into on the page.
pub(super) enum TypingInput {
    /// One for text of any number of lines.
    Lines(gtk::TextView),
    /// One line that hides what is typed, for a password.
    Secret(gtk::Text),
}

impl Typing {
    /// What was typed.
    pub(super) fn text(&self) -> String {
        match &self.input {
            TypingInput::Lines(view) => {
                let buffer = view.buffer();
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string()
            }
            TypingInput::Secret(entry) => entry.text().to_string(),
        }
    }
}

/// What is typed on the page is for.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum TypingTarget {
    /// A text box yet to be made, of what is typed.
    NewBox,
    /// The picked text box.
    Box,
    /// A text field of the form: which, how high it is (in points), whether it takes several
    /// lines, whether what is typed is hidden, how many cells its letters go into if it is a
    /// comb, and where its text stands (0 for left, 1 for centered, 2 for right).
    Field {
        field: FieldId,
        height: f64,
        multiline: bool,
        password: bool,
        comb: Option<usize>,
        alignment: u8,
    },
}

impl TypingTarget {
    pub(super) fn is_field(self) -> bool {
        matches!(self, TypingTarget::Field { .. })
    }
}

/// The room between the edge of a form's text field and its text, in points, as the saved look
/// has it.
const FIELD_MARGIN: f64 = 2.0;

/// The height of a line in a form's text field, in text sizes, as the saved look has it.
const FIELD_LINE: f64 = 1.15;

thread_local! {
    /// The style of the text field for typing on the page, which follows the box's text and the
    /// zoom.
    pub(super) static TYPING_CSS: gtk::CssProvider = gtk::CssProvider::new();
}

/// How wide a signature is when it is placed, in points.
pub(super) const SIGNATURE_WIDTH: f64 = 150.0;

/// The fonts to ask the system for, for a family: the font PDF programs set it in, then others with
/// the same widths, then anything of the kind.
pub(super) fn font_stack(family: FontFamily) -> &'static str {
    match family {
        FontFamily::Sans => {
            "Helvetica, \"Nimbus Sans\", Arimo, \"Liberation Sans\", Arial, sans-serif"
        }
        FontFamily::Serif => {
            "Times, \"Nimbus Roman\", Tinos, \"Liberation Serif\", \"Times New Roman\", serif"
        }
        FontFamily::Mono => {
            "Courier, \"Nimbus Mono PS\", Cousine, \"Liberation Mono\", \"Courier New\", monospace"
        }
    }
}

/// The tool whose color the bar shows while none is pressed in.
pub(super) const HIGHLIGHTER: Tool = Tool::Mark(MarkupStyle::Highlight);

/// The markup bar's tools, as the `markup-tool` action names them.
pub(super) const TOOLS: [(&str, Tool); 9] = [
    ("highlight", Tool::Mark(MarkupStyle::Highlight)),
    ("underline", Tool::Mark(MarkupStyle::Underline)),
    ("strike-out", Tool::Mark(MarkupStyle::StrikeOut)),
    ("draw", Tool::Draw(DraftKind::Ink)),
    ("rectangle", Tool::Draw(DraftKind::Square)),
    ("ellipse", Tool::Draw(DraftKind::Circle)),
    ("line", Tool::Draw(DraftKind::Line)),
    ("text", Tool::Text),
    ("note", Tool::Note),
];

/// Where a tool is in `TOOLS` (and its color in `Inner::tool_colors`).
pub(super) fn tool_index(tool: Tool) -> usize {
    TOOLS.iter().position(|(_, t)| *t == tool).unwrap_or(0)
}

pub(super) fn tool_name(tool: Tool) -> &'static str {
    TOOLS[tool_index(tool)].0
}

pub(super) fn tool_named(name: &str) -> Option<Tool> {
    TOOLS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

impl Window {
    /// Deletes the pages selected in the sidebar if there are several, else the current page.
    ///
    /// The last page can't be deleted. Marks the selected text: a highlight, or a line under or
    /// through it, in that tool's color. Returns whether there was text to mark.
    pub(super) fn mark(&self, style: MarkupStyle) -> bool {
        let color = color_named(self.0.tool_colors.get()[tool_index(Tool::Mark(style))]);
        self.annotate(|d| d.mark_selection(style, color))
    }

    /// The Highlight, Underline and Strikethrough actions: marks once.
    pub(super) fn mark_once(&self, style: MarkupStyle) {
        if !self.mark(style) {
            self.toast(&gettext("Select the text to mark first"));
        }
    }

    /// The tool that is pressed in on the markup bar, if any.
    pub(super) fn markup_tool(&self) -> Option<Tool> {
        let state = self.0.window.action_state("markup-tool")?;
        tool_named(state.str()?)
    }

    /// Presses a tool of the markup bar in (or none), and shows its color.
    pub(super) fn set_markup_tool(&self, tool: Option<Tool>) {
        let name = tool.map_or("none", tool_name);
        self.0
            .window
            .change_action_state("markup-tool", &name.to_variant());
        self.show_markup_color();
        self.set_draw_tool();
    }

    /// Tells the page view what drags do: draw, with a drawing tool pressed in (in its color), else
    /// select text.
    pub(super) fn set_draw_tool(&self) {
        let draw = match self.markup_tool() {
            Some(tool @ Tool::Draw(kind)) => {
                let color = color_named(self.0.tool_colors.get()[tool_index(tool)]);
                let width = if kind == DraftKind::Ink { 2.0 } else { 1.5 };
                Some((kind, color, width))
            }
            _ => None,
        };
        self.0.canvas.set_draw_tool(draw);
        let placing = self.0.placing.borrow().is_some();
        self.0
            .canvas
            .set_place_tool(placing || matches!(self.markup_tool(), Some(Tool::Text | Tool::Note)));
    }

    /// Fills the list of saved signatures under its button: each to be chosen or deleted, and a
    /// button to make a new one.
    pub(super) fn fill_signature_menu(&self) {
        let inner = &self.0;
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for (name, signature) in signatures::saved() {
            let choose = gtk::Button::builder()
                .child(&signatures::preview(&signature))
                .tooltip_text(gettext("Place this signature: click where it goes"))
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(inner);
            choose.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    inner.signature_menu.popdown();
                    Window(inner).place_signature(signature.clone());
                }
            });
            let delete = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(gettext("Delete this signature"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(inner);
            delete.connect_clicked(move |_| {
                signatures::delete(&name);
                if let Some(inner) = weak.upgrade() {
                    Window(inner).fill_signature_menu();
                }
            });
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&choose);
            row.append(&delete);
            rows.append(&row);
        }
        let new = gtk::Button::with_label(&gettext("New Signature…"));
        let weak = Rc::downgrade(inner);
        new.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.signature_menu.popdown();
                Window(inner).new_signature();
            }
        });
        rows.append(&new);
        inner.signature_menu.set_child(Some(&rows));
    }

    /// Shows the dialog for making a signature. Once it is saved, it is the one being placed.
    pub(super) fn new_signature(&self) {
        let weak = Rc::downgrade(&self.0);
        signatures::new_signature(&self.0.window, move |_, signature| {
            if let Some(inner) = weak.upgrade() {
                let w = Window(inner);
                w.fill_signature_menu();
                w.place_signature(signature);
            }
        });
    }

    /// The next click on a page places `signature`. Lets go of the markup bar's tool.
    pub(super) fn place_signature(&self, signature: Signature) {
        self.0.canvas.unpick_annotation();
        self.0.placing.replace(Some(signature));
        self.set_markup_tool(None);
        self.toast(&gettext("Click where the signature goes"));
    }

    /// The page was clicked at (`x`, `y`) with a placing tool: a note goes there and asks for its
    /// text, or text is asked for and becomes a text box there.
    pub(super) fn place(&self, x: f64, y: f64) {
        let p = ViewPoint::new(x, y);
        // A signature chosen to be placed: once, here.
        if let Some(signature) = self.0.placing.take() {
            let (w, h) = signature.size();
            // 150 points wide, less for a tall one.
            let width = SIGNATURE_WIDTH.min(SIGNATURE_WIDTH * 0.6 * w / h);
            self.edit(|d| {
                let placed = match signature {
                    Signature::Drawn { strokes, size } => d.add_drawing(
                        p,
                        width,
                        &strokes,
                        size,
                        color_named("black"),
                        signatures::PEN * width / size.0,
                    ),
                    Signature::Photo(image) => d.add_stamp(p, width, image),
                };
                placed.then(Edited::default)
            });
            self.set_draw_tool();
            return;
        }
        match self.markup_tool() {
            Some(tool @ Tool::Note) => {
                let color = color_named(self.0.tool_colors.get()[tool_index(tool)]);
                if self.annotate(|d| d.add_note(p, color)) {
                    self.edit_text(None);
                }
            }
            Some(Tool::Text) => self.edit_text(Some(p)),
            _ => {}
        }
    }

    /// Shows a small editor for text: that of the picked annotation (what a text box shows, the
    /// text of a note, a comment on anything else), or, with `new_at`, of a text box to be made
    /// there. Closing it (a click elsewhere, Escape) keeps the text.
    pub(super) fn edit_text(&self, new_at: Option<ViewPoint>) {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        // A text box is typed into where it is on the page. (Not on pages shown turned, where the
        // text would have to be typed sideways: those get the small editor below, like notes and
        // comments.)
        let on_page = match new_at {
            Some(p) => {
                let color = color_named(inner.tool_colors.get()[tool_index(Tool::Text)]);
                let style = inner.text_style.get();
                let place = doc.borrow().new_text_box_place(p, style, color);
                place.map(|place| (place, String::new(), TypingTarget::NewBox))
            }
            None => {
                let editing = doc.borrow_mut().begin_text_edit();
                editing.map(|(place, text)| (place, text, TypingTarget::Box))
            }
        };
        if let Some((place, text, target)) = on_page {
            return self.type_on_page(place, &text, target);
        }
        let (text, area) = match new_at {
            Some(p) => (
                String::new(),
                gdk::Rectangle::new(p.x as i32, p.y as i32, 1, 1),
            ),
            None => {
                let Some(text) = doc.borrow_mut().picked_text() else {
                    return;
                };
                let Some(area) = inner.canvas.picked_area() else {
                    return;
                };
                (text, area)
            }
        };
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(6)
            .bottom_margin(6)
            .left_margin(6)
            .right_margin(6)
            .build();
        view.buffer().set_text(&text);
        // A size of its own: left to the text view, which asks for no more
        // room than a letter, the editor would be tiny.
        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .width_request(360)
            .height_request(180)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let popover = gtk::Popover::builder().child(&scroller).build();
        popover.set_parent(&inner.canvas);
        popover.set_pointing_to(Some(&area));
        inner.text_editor.replace(Some(popover.clone()));
        let weak = Rc::downgrade(inner);
        let buffer = view.buffer();
        popover.connect_closed(move |popover| {
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            // The text at once: whatever closed the editor (a save, the window closing) must
            // find it in the document.
            if let Some(inner) = weak.upgrade() {
                let open = inner.text_editor.take().is_some();
                if open {
                    Window(inner).set_text(new_at, &text);
                }
            }
            // Later: the popover is still closing, and is taken off the page view once it has.
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
        view.grab_focus();
    }

    /// A form field that can be filled in was clicked at `(x, y)` of the page view: a box is ticked
    /// or unticked, a radio button chosen, a text field typed into.
    pub(super) fn fill_at(&self, x: f64, y: f64) {
        self.finish_text_entry();
        let mut clicked = None;
        self.annotate(|d| {
            clicked = d.click_field(ViewPoint::new(x, y));
            clicked == Some(FieldClick::Changed)
        });
        self.enter_field(clicked);
    }

    /// A key for the form field that has the keyboard: Tab and Shift+Tab move to the next field
    /// and the one before, Space and Enter do what a click does.
    pub(super) fn field_key(&self, key: FieldKey) {
        self.finish_text_entry();
        let mut clicked = None;
        self.annotate(|d| {
            clicked = match key {
                FieldKey::Next => d.tab_field(true),
                FieldKey::Previous => d.tab_field(false),
                FieldKey::Press => d.press_focused_field(),
            };
            clicked == Some(FieldClick::Changed)
        });
        // The view may have moved to show the field, and the frame is on another one.
        self.0.canvas.update_viewer(|_| {});
        self.enter_field(clicked);
    }

    /// After a click or a key on a form field: a text field is typed into.
    fn enter_field(&self, clicked: Option<FieldClick>) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        self.0.canvas.queue_draw();
        let (field, place, height, multiline, password, comb, alignment, text) = match clicked {
            Some(FieldClick::Text {
                field,
                place,
                height,
                multiline,
                password,
                comb,
                alignment,
                text,
            }) => (
                field, place, height, multiline, password, comb, alignment, text,
            ),
            Some(FieldClick::Edit {
                field,
                multiline,
                password,
                text,
                corners,
            }) => return self.edit_field_text(field, multiline, password, &text, corners),
            Some(FieldClick::Choose {
                field,
                labels,
                chosen,
                typed,
                corners,
            }) => return self.choose_from(field, labels, chosen, typed, corners),
            Some(FieldClick::ChooseSeveral {
                field,
                labels,
                chosen,
                corners,
            }) => return self.choose_several(field, labels, &chosen, corners),
            _ => return,
        };
        doc.borrow_mut().begin_field_text(field);
        let target = TypingTarget::Field {
            field,
            height,
            multiline,
            password,
            comb,
            alignment,
        };
        self.type_on_page(place, &text, target);
    }

    /// Shows what the choice field `field` offers, in a list at the field (its `corners` in the
    /// page view), with the option it holds marked. A click on a row, or Enter, chooses it.
    ///
    /// A field that takes text of one's own (`typed` is what it holds) has a line to type above
    /// the list. What was typed is kept when the list closes, unless Escape closed it.
    fn choose_from(
        &self,
        field: FieldId,
        labels: Vec<String>,
        chosen: Option<usize>,
        typed: Option<String>,
        corners: [ViewPoint; 4],
    ) {
        let inner = &self.0;
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .build();
        for label in &labels {
            let text = gtk::Label::builder()
                .label(label)
                .xalign(0.0)
                .margin_start(6)
                .margin_end(6)
                .build();
            list.append(&text);
        }
        let area = area_of(corners);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(320)
            .min_content_width(area.width().clamp(120, 480))
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let entry = typed.as_ref().map(|text| {
            let entry = gtk::Entry::builder().text(text.as_str()).build();
            content.append(&entry);
            entry
        });
        scroller.set_visible(!labels.is_empty());
        content.append(&scroller);
        let popover = gtk::Popover::builder()
            .child(&content)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.add_css_class("menu");
        popover.set_parent(&inner.canvas);
        popover.set_pointing_to(Some(&area));
        // Whether what is typed is kept when the list closes: not after Escape, and not once a
        // row was chosen.
        let keep = Rc::new(Cell::new(true));
        let weak = Rc::downgrade(inner);
        let shown = popover.downgrade();
        let kept = keep.clone();
        list.connect_row_activated(move |_, row| {
            let (Some(inner), Ok(index)) = (weak.upgrade(), usize::try_from(row.index())) else {
                return;
            };
            kept.set(false);
            if let Some(popover) = shown.upgrade() {
                popover.popdown();
            }
            Window(inner).annotate(|d| d.choose_option(field, index));
        });
        if let Some(entry) = &entry {
            let shown = popover.downgrade();
            entry.connect_activate(move |_| {
                if let Some(popover) = shown.upgrade() {
                    popover.popdown();
                }
            });
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let kept = keep.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    kept.set(false);
                }
                glib::Propagation::Proceed
            });
            popover.add_controller(keys);
        }
        let weak = Rc::downgrade(inner);
        let line = entry.clone();
        popover.connect_closed(move |popover| {
            if let Some(inner) = weak.upgrade() {
                // The text at once: whatever closed the list (a save, the window closing) must
                // find it in the document.
                let open = inner.choices.take().is_some();
                if let (true, true, Some(entry), Some(was)) = (open, keep.get(), &line, &typed) {
                    let text = entry.text().to_string();
                    if text != *was {
                        Window(inner.clone()).annotate(|d| d.set_field_text(field, &text));
                    }
                }
                inner.canvas.grab_focus();
            }
            // Later: the popover is still closing, and is taken off the page view once it has.
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        inner.choices.replace(Some(Choices {
            popover: popover.clone(),
            labels,
            entry: entry.clone(),
            ticks: Vec::new(),
        }));
        popover.popup();
        // The keyboard on what is chosen, or on the first: the arrow keys go from there.
        let row = list.row_at_index(chosen.unwrap_or(0) as i32);
        if chosen.is_some() || entry.is_none() {
            list.select_row(row.as_ref());
        }
        match (entry, row) {
            // To go on typing after what is there.
            (Some(entry), _) => {
                entry.grab_focus_without_selecting();
                entry.set_position(-1);
            }
            (None, Some(row)) => {
                row.grab_focus();
            }
            (None, None) => {}
        }
    }

    /// Puts a text field over the page where a text box is (or is to be), or a text field of the
    /// form, in its font, size and color, so that what is typed appears where it will stand.
    /// Escape, or a click elsewhere, keeps the text.
    pub(super) fn type_on_page(&self, place: TextBoxPlace, text: &str, target: TypingTarget) {
        let inner = &self.0;
        self.finish_typing();
        let (one_line, secret, comb, alignment) = match target {
            TypingTarget::Field {
                multiline,
                password,
                comb,
                alignment,
                ..
            } => (
                !multiline,
                password && !multiline,
                comb.is_some(),
                alignment,
            ),
            _ => (false, false, false, 0),
        };
        let input = if secret {
            let entry = gtk::Text::builder()
                .visibility(false)
                .input_purpose(gtk::InputPurpose::Password)
                .xalign([0.0, 0.5, 1.0][usize::from(alignment.min(2))])
                .halign(gtk::Align::Fill)
                .valign(gtk::Align::Fill)
                .css_classes(["page-text", "secret"])
                .build();
            entry.set_text(text);
            TypingInput::Secret(entry)
        } else {
            let lines = gtk::TextView::builder()
                .wrap_mode(gtk::WrapMode::WordChar)
                .accepts_tab(false)
                // Filling what the overlay gives it, which is the text box's place: aligned to a
                // side, it would be as narrow as a letter.
                .halign(gtk::Align::Fill)
                .valign(gtk::Align::Fill)
                .css_classes(["page-text"])
                .build();
            lines.buffer().set_text(text);
            lines.set_justification(match alignment {
                _ if comb => gtk::Justification::Left,
                1 => gtk::Justification::Center,
                2 => gtk::Justification::Right,
                _ => gtk::Justification::Left,
            });
            if comb {
                // Its letters stand in their cells, wherever the line would break.
                lines.set_wrap_mode(gtk::WrapMode::None);
                let weak = Rc::downgrade(inner);
                lines.buffer().connect_changed(move |_| {
                    if let Some(inner) = weak.upgrade() {
                        Window(inner).space_comb();
                    }
                });
            }
            TypingInput::Lines(lines)
        };
        let view: gtk::Widget = match &input {
            TypingInput::Lines(lines) => lines.clone().upcast(),
            TypingInput::Secret(entry) => entry.clone().upcast(),
        };
        inner.page_overlay.add_overlay(&view);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(inner);
        keys.connect_key_pressed(move |_, key, _, state| {
            // Enter ends a field of one line, as Escape does.
            let enter = matches!(
                key,
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
            );
            // Tab leaves a field of the form for the next one, and Shift+Tab for the one before.
            let tab = match key {
                _ if !target.is_field() => None,
                gdk::Key::ISO_Left_Tab => Some(FieldKey::Previous),
                gdk::Key::Tab | gdk::Key::KP_Tab
                    if state.contains(gdk::ModifierType::SHIFT_MASK) =>
                {
                    Some(FieldKey::Previous)
                }
                gdk::Key::Tab | gdk::Key::KP_Tab => Some(FieldKey::Next),
                _ => None,
            };
            if key != gdk::Key::Escape && !(one_line && enter) && tab.is_none() {
                return glib::Propagation::Proceed;
            }
            if let Some(inner) = weak.upgrade() {
                let window = Window(inner);
                window.finish_typing();
                if let Some(tab) = tab {
                    window.field_key(tab);
                }
            }
            glib::Propagation::Stop
        });
        view.add_controller(keys);
        // A click elsewhere takes the keyboard away: the text is kept. (Later: the focus is still
        // on its way.)
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(inner);
        let left = view.downgrade();
        focus.connect_leave(move |_| {
            let (weak, left) = (weak.clone(), left.clone());
            glib::idle_add_local_once(move || {
                let Some(inner) = weak.upgrade() else { return };
                // Only if this field is still the one typed into: Tab has moved on to another by
                // now, which must not end with this one.
                let current = inner.typing.borrow().as_ref().map(|t| t.view.clone());
                if current.is_none() || current != left.upgrade() {
                    return;
                }
                // Not for the controls that set the text's size and font: those act on what is
                // being typed.
                let on_controls = gtk::prelude::GtkWindowExt::focus(&inner.window)
                    .is_some_and(|f| f.is_ancestor(&inner.text_controls));
                if !on_controls {
                    Window(inner).finish_typing();
                }
            });
        });
        view.add_controller(focus);
        // The cursor after what is there.
        match &input {
            TypingInput::Lines(lines) => {
                let end = lines.buffer().end_iter();
                lines.buffer().place_cursor(&end);
            }
            TypingInput::Secret(entry) => entry.set_position(-1),
        }
        inner.typing.replace(Some(Typing {
            view: view.clone(),
            input,
            style: Cell::new((place.size, place.font)),
            place,
            target,
            area: Cell::new((0, 0, 1)),
        }));
        self.show_text_style();
        self.place_typing();
        // The redraw first: the page without the box, if it had it.
        inner.canvas.queue_draw();
        match view.downcast_ref::<gtk::Text>() {
            // Not `grab_focus`, which selects all of it.
            Some(entry) => {
                entry.grab_focus_without_selecting();
            }
            None => {
                view.grab_focus();
            }
        }
    }

    /// Shows what `field`, a list that takes several choices, offers, in a list at the field (its
    /// `corners` in the page view), each row with a tick. What is ticked when the list closes
    /// is chosen, unless Escape closed it.
    fn choose_several(
        &self,
        field: FieldId,
        labels: Vec<String>,
        chosen: &[usize],
        corners: [ViewPoint; 4],
    ) {
        let inner = &self.0;
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        let ticks: Vec<gtk::CheckButton> = (labels.iter().enumerate())
            .map(|(i, label)| {
                let tick = gtk::CheckButton::builder()
                    .label(label.as_str())
                    .active(chosen.contains(&i))
                    .margin_start(6)
                    .margin_end(6)
                    .build();
                list.append(&tick);
                tick
            })
            .collect();
        // The row has the keyboard, not the tick in it: Enter on it ticks, and so does Space
        // (below).
        let rows = ticks.clone();
        list.connect_row_activated(move |_, row| {
            if let Some(tick) = usize::try_from(row.index()).ok().and_then(|i| rows.get(i)) {
                tick.set_active(!tick.is_active());
            }
        });
        for tick in &ticks {
            tick.set_focusable(false);
        }
        let area = area_of(corners);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(320)
            .min_content_width(area.width().clamp(120, 480))
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroller)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.add_css_class("menu");
        popover.set_parent(&inner.canvas);
        popover.set_pointing_to(Some(&area));
        let keep = Rc::new(Cell::new(true));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let kept = keep.clone();
        let rows = list.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                kept.set(false);
            }
            // The toolkit has Space do nothing on a row.
            let row = rows
                .upgrade()
                .and_then(|list| list.focus_child())
                .and_downcast::<gtk::ListBoxRow>();
            match row {
                Some(row) if key == gdk::Key::space => {
                    row.emit_by_name::<()>("activate", &[]);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        popover.add_controller(keys);
        let weak = Rc::downgrade(inner);
        popover.connect_closed(move |popover| {
            if let Some(inner) = weak.upgrade() {
                // At once: whatever closed the list (a save, the window closing) must find
                // what is ticked in the document.
                if let Some(choices) = inner.choices.take().filter(|_| keep.get()) {
                    let ticked: Vec<usize> = (choices.ticks.iter().enumerate())
                        .filter(|(_, tick)| tick.is_active())
                        .map(|(i, _)| i)
                        .collect();
                    Window(inner.clone()).annotate(|d| d.choose_options(field, &ticked));
                }
                inner.canvas.grab_focus();
            }
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        inner.choices.replace(Some(Choices {
            popover: popover.clone(),
            labels,
            entry: None,
            ticks,
        }));
        popover.popup();
        // The keyboard on the first that is chosen, or on the first: the arrow keys go from
        // there.
        let first = chosen.first().copied().unwrap_or(0);
        if let Some(row) = list.row_at_index(first as i32) {
            row.grab_focus();
        }
    }

    /// Asks for the text of the text field `field` in a small editor at the field (its `corners`
    /// in the page view): on pages shown turned, where it can't be typed into in place. Closing
    /// it (a click elsewhere, Escape, Enter in a field of one line) keeps the text.
    fn edit_field_text(
        &self,
        field: FieldId,
        multiline: bool,
        password: bool,
        text: &str,
        corners: [ViewPoint; 4],
    ) {
        let inner = &self.0;
        let area = area_of(corners);
        let popover = gtk::Popover::new();
        let mut line = None;
        // What it holds when it closes.
        let read: Box<dyn Fn() -> String> = if multiline {
            let view = gtk::TextView::builder()
                .wrap_mode(gtk::WrapMode::WordChar)
                .top_margin(6)
                .bottom_margin(6)
                .left_margin(6)
                .right_margin(6)
                .build();
            view.buffer().set_text(text);
            let scroller = gtk::ScrolledWindow::builder()
                .child(&view)
                .width_request(360)
                .height_request(180)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();
            popover.set_child(Some(&scroller));
            let buffer = view.buffer();
            Box::new(move || {
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string()
            })
        } else {
            let entry = gtk::Entry::builder()
                .text(text)
                .visibility(!password)
                .width_request(280)
                .build();
            if password {
                entry.set_input_purpose(gtk::InputPurpose::Password);
            }
            let shown = popover.downgrade();
            entry.connect_activate(move |_| {
                if let Some(popover) = shown.upgrade() {
                    popover.popdown();
                }
            });
            popover.set_child(Some(&entry));
            line = Some(entry.clone());
            Box::new(move || entry.text().to_string())
        };
        popover.set_parent(&inner.canvas);
        popover.set_pointing_to(Some(&area));
        inner.text_editor.replace(Some(popover.clone()));
        let weak = Rc::downgrade(inner);
        popover.connect_closed(move |popover| {
            // The text at once: whatever closed the editor (a save, the window closing) must
            // find it in the document.
            if let Some(inner) = weak.upgrade() {
                let open = inner.text_editor.take().is_some();
                if open {
                    let text = read();
                    Window(inner.clone()).annotate(|d| d.set_field_text(field, &text));
                }
                inner.canvas.grab_focus();
            }
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
        // To go on typing after what is there: left to itself, the entry selects it all, and the
        // next key would replace it.
        if let Some(entry) = line {
            entry.grab_focus_without_selecting();
            entry.set_position(-1);
        }
    }

    /// Puts the text field where its text box is on screen now, at the size the text has there:
    /// when it is shown, and when the view moves.
    pub(super) fn place_typing(&self) {
        let inner = &self.0;
        let typing = inner.typing.borrow();
        let (Some(typing), Some(doc)) = (typing.as_ref(), inner.document.borrow().clone()) else {
            return;
        };
        let Some((corner, zoom)) = doc.borrow().text_box_corner(&typing.place) else {
            return typing.view.set_visible(false);
        };
        let place = &typing.place;
        let (size, font) = typing.style.get();
        let view = &typing.view;
        view.set_visible(true);
        let lines = match &typing.input {
            TypingInput::Lines(lines) => Some(lines),
            TypingInput::Secret(_) => None,
        };
        // Where the overlay puts it (see `Window::new`): its corner, and its width. Its height is
        // what its text needs at that width.
        typing.area.set((
            corner.x.round() as i32,
            corner.y.round() as i32,
            (place.width * zoom).round().max(1.0) as i32,
        ));
        view.queue_resize();
        let field = typing.target.is_field();
        let room = if field {
            FIELD_MARGIN
        } else {
            specto_annot::TEXT_MARGIN
        };
        let margin = (room * zoom).round() as i32;
        if let Some(lines) = lines {
            lines.set_left_margin(margin);
            lines.set_right_margin(margin);
        }
        // A line of the field is higher than the letters, half of the rest above them: less of a
        // margin on top makes up for it, so that the text stands where the box will have it.
        let above = match typing.target {
            // A field of one line has its text in the middle.
            TypingTarget::Field {
                height,
                multiline: false,
                ..
            } => (height - FIELD_LINE * size) / 2.0,
            _ => room - 0.12 * size,
        };
        if let Some(lines) = lines {
            lines.set_top_margin((above * zoom).max(0.0).round() as i32);
            lines.set_bottom_margin(if field { 0 } else { margin });
        }
        let line_height = if field { FIELD_LINE } else { 1.2 };
        let c = place.color;
        let byte = |v: f32| (v * 255.0).round() as u8;
        let rgb = format!("rgb({}, {}, {})", byte(c.r), byte(c.g), byte(c.b));
        // The box's font, or what stands in for it with the same widths, so that lines break where
        // they will in the box.
        let css = format!(
            ".page-text, .page-text text {{ background: none; color: {rgb}; caret-color: {rgb}; \
             font-family: {}; font-weight: {}; font-style: {}; \
             font-size: {:.2}px; line-height: {line_height}; }}\n\
             .page-text {{ outline: 1px solid @accent_bg_color; outline-offset: 2px; border-radius: 2px; }}\n\
             .page-text.secret {{ padding: 0 {margin}px; min-height: 0; }}\n",
            font_stack(font.family),
            if font.bold { "bold" } else { "normal" },
            if font.italic { "italic" } else { "normal" },
            size * zoom
        );
        TYPING_CSS.with(|provider| {
            provider.load_from_string(&css);
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
            }
        });
        // A comb's letters, once the text field has the new size of its text.
        if matches!(typing.target, TypingTarget::Field { comb: Some(_), .. }) {
            let weak = Rc::downgrade(inner);
            glib::idle_add_local_once(move || {
                if let Some(inner) = weak.upgrade() {
                    Window(inner).space_comb();
                }
            });
        }
    }

    /// Sets the letters typed into a comb field in the middle of their cells, as the saved look
    /// has them: after each change to the text, and to the size it has on screen.
    fn space_comb(&self) {
        let Ok(typing) = self.0.typing.try_borrow() else {
            return;
        };
        let Some(Typing {
            input: TypingInput::Lines(lines),
            target: TypingTarget::Field {
                comb: Some(cells), ..
            },
            area,
            ..
        }) = typing.as_ref()
        else {
            return;
        };
        let (lines, cells) = (lines.clone(), (*cells).max(1));
        let cell = f64::from(area.get().2) / cells as f64;
        drop(typing);
        let buffer = lines.buffer();
        // No more letters than cells. Later: the text is being changed right now.
        if buffer.char_count() > cells as i32 {
            glib::idle_add_local_once(move || {
                let buffer = lines.buffer();
                let (mut from, mut end) = (buffer.iter_at_offset(cells as i32), buffer.end_iter());
                buffer.delete(&mut from, &mut end);
            });
            return;
        }
        // Each letter is as wide as a cell with the room around it, half of it on each side.
        // The first one has none before it, which the margin makes up for.
        let table = buffer.tag_table();
        let mut old = Vec::new();
        table.foreach(|tag| old.push(tag.clone()));
        for tag in &old {
            table.remove(tag);
        }
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let mut first = 0.0;
        for (i, letter) in text.chars().enumerate() {
            let layout = lines.create_pango_layout(Some(letter.encode_utf8(&mut [0; 4])));
            let width = f64::from(layout.size().0) / f64::from(gtk::pango::SCALE);
            if i == 0 {
                first = width;
            }
            let room = ((cell - width) * f64::from(gtk::pango::SCALE)).round() as i32;
            let tag = gtk::TextTag::builder().letter_spacing(room).build();
            table.add(&tag);
            let at = i as i32;
            buffer.apply_tag(
                &tag,
                &buffer.iter_at_offset(at),
                &buffer.iter_at_offset(at + 1),
            );
        }
        let margin = ((cell - first) / 2.0).round().max(0.0) as i32;
        lines.set_left_margin(margin);
        lines.set_right_margin(0);
    }

    /// Ends whatever text is being entered, on the page, in the small editor or above the list of
    /// a choice field, keeping it: before anything that needs the document as the user sees it
    /// (saving, closing, another edit).
    pub(super) fn finish_text_entry(&self) {
        self.finish_typing();
        let editor = self.0.text_editor.borrow().clone();
        if let Some(editor) = editor {
            editor.popdown();
        }
        let choices = self.0.choices.borrow().as_ref().map(|c| c.popover.clone());
        if let Some(choices) = choices {
            choices.popdown();
        }
    }

    /// Ends typing on the page, if there is any: what was typed is the text box's text now (a new
    /// box, if there is text for one).
    pub(super) fn finish_typing(&self) {
        let inner = &self.0;
        let Some(typing) = inner.typing.take() else {
            return;
        };
        let text = typing.text();
        inner.page_overlay.remove_overlay(&typing.view);
        // In the size and font it has now, which may have been changed while typing.
        let style = typing.style.get();
        let place = TextBoxPlace {
            size: style.0,
            font: style.1,
            ..typing.place.clone()
        };
        let mut failed = None;
        self.edit(|d| {
            match typing.target {
                TypingTarget::NewBox => {
                    d.add_text_box_at(&place, &text);
                }
                TypingTarget::Box => {
                    if let Err(e) = d.end_text_edit(Some((&text, style))) {
                        failed = Some(e);
                    }
                }
                TypingTarget::Field { .. } => {
                    d.end_field_text(Some(&text));
                }
            }
            // Always: the page is drawn with the box again.
            Some(Edited::default())
        });
        if let Some(e) = failed {
            self.toast(&gettext_f(
                "Couldn't change the annotation: {error}",
                &[("error", &save_error(e))],
            ));
        }
        self.show_markup_color();
        inner.canvas.grab_focus();
    }

    /// The text editor was closed with `text` in it: for the picked annotation, or for a new text
    /// box at `new_at`.
    pub(super) fn set_text(&self, new_at: Option<ViewPoint>, text: &str) {
        let mut failed = None;
        let tool = Tool::Text;
        let color = color_named(self.0.tool_colors.get()[tool_index(tool)]);
        let style = self.0.text_style.get();
        self.edit(|d| {
            let changed = match new_at {
                Some(p) => d.add_text_box(p, text, style, color),
                None => d.set_picked_text(text).unwrap_or_else(|e| {
                    failed = Some(e);
                    false
                }),
            };
            changed.then(Edited::default)
        });
        if let Some(e) = failed {
            self.toast(&gettext_f(
                "Couldn't change the annotation: {error}",
                &[("error", &save_error(e))],
            ));
        }
        self.show_markup_color();
    }

    /// The size and font the text controls of the markup bar are set to.
    pub(super) fn text_controls_style(&self) -> (f64, TextFont) {
        let inner = &self.0;
        let family = match inner.text_font.selected() {
            1 => FontFamily::Serif,
            2 => FontFamily::Mono,
            _ => FontFamily::Sans,
        };
        (
            inner.text_size.value(),
            TextFont {
                family,
                bold: inner.text_bold.is_active(),
                italic: inner.text_italic.is_active(),
            },
        )
    }

    /// Shows the text controls while they have something to act on (the text tool, a text box
    /// picked or typed into), set to that thing's size and font.
    pub(super) fn show_text_style(&self) {
        let inner = &self.0;
        // Not for a field of the form, which has its own size and font.
        let typing = inner
            .typing
            .borrow()
            .as_ref()
            .filter(|t| !t.target.is_field())
            .map(|t| t.style.get());
        let picked = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow_mut().picked_text_style());
        let tool = self.markup_tool() == Some(Tool::Text);
        inner
            .text_controls
            .set_reveal_child(tool || typing.is_some() || picked.is_some());
        let (size, font) = typing.or(picked).unwrap_or(inner.text_style.get());
        inner.showing_text_style.set(true);
        inner.text_font.set_selected(match font.family {
            FontFamily::Sans => 0,
            FontFamily::Serif => 1,
            FontFamily::Mono => 2,
        });
        inner.text_bold.set_active(font.bold);
        inner.text_italic.set_active(font.italic);
        inner.text_size.set_value(size);
        inner.showing_text_style.set(false);
    }

    /// A text control of the markup bar was changed: what is typed, or the picked text box, is set
    /// that way, and so are text boxes from now on.
    pub(super) fn text_style_chosen(&self) {
        let inner = &self.0;
        if inner.showing_text_style.get() {
            return;
        }
        let style = self.text_controls_style();
        inner.text_style.set(style);
        let typing = inner.typing.borrow().as_ref().map(|t| {
            if !t.target.is_field() {
                t.style.set(style);
            }
            t.view.clone()
        });
        if let Some(view) = typing {
            self.place_typing();
            // Back to typing, unless a size is being typed in.
            let in_size = gtk::prelude::GtkWindowExt::focus(&inner.window)
                .is_some_and(|f| f.is_ancestor(&inner.text_size));
            if !in_size {
                view.grab_focus();
            }
            return;
        }
        let mut failed = None;
        self.edit(|d| match d.restyle_picked_text_box(style) {
            Ok(changed) => changed.then(Edited::default),
            Err(e) => {
                failed = Some(e);
                None
            }
        });
        if let Some(e) = failed {
            self.toast(&gettext_f(
                "Couldn't change the annotation: {error}",
                &[("error", &save_error(e))],
            ));
        }
    }

    /// Shows, on the markup bar, the color of the picked annotation if it is one of the bar's, else
    /// that of the tool pressed in (of the highlighter if none is).
    pub(super) fn show_markup_color(&self) {
        let inner = &self.0;
        let picked = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow_mut().picked_color())
            .map(|c| {
                markup_colors()
                    .into_iter()
                    .find(|(_, _, known)| *known == c)
                    .map_or("other", |(name, _, _)| name)
            });
        let tool = self.markup_tool().unwrap_or(HIGHLIGHTER);
        let name = picked.unwrap_or(inner.tool_colors.get()[tool_index(tool)]);
        inner
            .window
            .change_action_state("markup-color", &name.to_variant());
        inner
            .color_swatch
            .set_css_classes(&["markup-swatch", &format!("markup-{name}")]);
        self.show_text_style();
    }

    /// A color of the markup bar was chosen: the picked annotation gets it, if one is picked.
    ///
    /// Otherwise it is what the tool pressed in (the highlighter if none is) marks in from now on.
    pub(super) fn choose_markup_color(&self, name: &'static str) {
        let inner = &self.0;
        let picked = inner
            .document
            .borrow()
            .as_ref()
            .is_some_and(|d| d.borrow().picked.is_some());
        if picked {
            let mut failed = None;
            self.edit(|d| match d.recolor_picked_annotation(color_named(name)) {
                Ok(changed) => changed.then(Edited::default),
                Err(e) => {
                    failed = Some(e);
                    None
                }
            });
            if let Some(e) = failed {
                self.toast(&gettext_f(
                    "Couldn't change the annotation: {error}",
                    &[("error", &save_error(e))],
                ));
            }
        } else {
            let tool = self.markup_tool().unwrap_or(HIGHLIGHTER);
            let mut colors = inner.tool_colors.get();
            colors[tool_index(tool)] = name;
            inner.tool_colors.set(colors);
            self.set_draw_tool();
        }
        self.show_markup_color();
    }

    /// Text was selected with the pointer: a tool that is pressed in marks it.
    pub(super) fn selected(&self) {
        if let Some(Tool::Mark(style)) = self.markup_tool() {
            self.mark(style);
        }
    }

    /// Applies an edit to the annotations, then redraws and updates the menus if it changed
    /// anything, which it returns.
    ///
    /// The pages stay as they are, so there is less to do than after [`edit`](Self::edit).
    pub(super) fn annotate(&self, f: impl FnOnce(&mut Document) -> bool) -> bool {
        self.finish_typing();
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return false;
        };
        let changed = f(&mut doc.borrow_mut());
        if changed {
            inner.canvas.queue_draw();
            if let Some(s) = inner.sidebar.borrow().as_ref() {
                s.refresh();
            }
            self.update_edit_state();
        }
        changed
    }

    /// Fills the list of markup in the sidebar from the document, if it is what the sidebar shows:
    /// after an edit, and when it is turned to.
    pub(super) fn refresh_markup_list(&self) {
        let inner = &self.0;
        let shown = inner
            .sidebar_stack
            .borrow()
            .as_ref()
            .and_then(|s| s.visible_child_name())
            .is_some_and(|name| name == "markup");
        let (Some(doc), true) = (inner.document.borrow().clone(), shown) else {
            return;
        };
        let mut d = doc.borrow_mut();
        let entries = d
            .all_annotations()
            .into_iter()
            .map(|(position, id, a)| Entry::new((position, id), &a, d.page_label(position)))
            .collect();
        let picked = d.picked;
        drop(d);
        if let Some(list) = inner.markup_list.borrow().as_ref() {
            list.set(entries, picked);
        }
    }
}
