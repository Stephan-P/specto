//! Translations, through gettext.
//!
//! Every string shown to people goes through one of these functions, with
//! the English text as the key. Sentences with something filled in use
//! named placeholders (`gettext_f("Couldn't open “{name}”", …)`) rather
//! than `format!`, so a translation gets the whole sentence and may move
//! the placeholders around.
//!
//! `po/update-pot.sh` collects the strings into `po/specto.pot`: it looks
//! for calls of these functions by name, with the text as a literal. A
//! comment before a call that starts with "Translators:" goes into the
//! template with the string.

use std::fmt::Display;
use std::path::PathBuf;

pub use gettextrs::gettext;

/// The name of the message catalogs: `specto.mo`.
const DOMAIN: &str = "specto";

/// Tells gettext where the catalogs are.
///
/// Without one for the user's language, every string stays as it is in the source.
///
/// The locale itself is set by GTK when the application starts (gettext-rs has `setlocale` only as
/// an unsafe function), so nothing is translated before that. Nothing is shown before that either.
pub fn init() {
    let result = (|| {
        if let Some(dir) = locale_dir() {
            gettextrs::bindtextdomain(DOMAIN, dir)?;
        }
        gettextrs::bind_textdomain_codeset(DOMAIN, "UTF-8")?;
        gettextrs::textdomain(DOMAIN)?;
        Ok::<_, std::io::Error>(())
    })();
    if let Err(e) = result {
        eprintln!("translations aren't available: {e}");
    }
}

/// Where the catalogs are: `share/locale` next to the `bin` the program is in, which fits `/usr`, a
/// Flatpak's `/app` and Homebrew alike.
///
/// `None` (gettext's own default) when there is no such directory, as in a build tree.
fn locale_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("share/locale");
    dir.is_dir().then_some(dir)
}

/// Translates `msgid` and fills in its `{name}` placeholders.
pub fn gettext_f(msgid: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(gettext(msgid), args)
}

/// Translates a text that depends on a number, choosing the form for `n`, and fills in `{n}` and
/// the other placeholders.
pub fn ngettext_f(one: &str, many: &str, n: usize, args: &[(&str, &dyn Display)]) -> String {
    // Plural forms depend on the last digits at most, which this keeps for numbers beyond u32.
    let count = u32::try_from(n).unwrap_or(1_000_000 + (n % 1_000_000) as u32);
    let text = fill(gettextrs::ngettext(one, many, count), args);
    text.replace("{n}", &n.to_string())
}

fn fill(mut text: String, args: &[(&str, &dyn Display)]) -> String {
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{gettext_f, ngettext_f};

    #[test]
    fn placeholders() {
        // Not literals in the calls, which keeps them out of the template.
        let open = "Couldn't open “{name}”: {error}";
        assert_eq!(
            gettext_f(open, &[("name", &"a.pdf"), ("error", &7)]),
            "Couldn't open “a.pdf”: 7"
        );
        // Untranslated, the English forms: singular for 1 only.
        let (one, many) = ("{n} page of {total}", "{n} pages of {total}");
        let pages = |n| ngettext_f(one, many, n, &[("total", &9)]);
        assert_eq!(pages(1), "1 page of 9");
        assert_eq!(pages(0), "0 pages of 9");
        assert_eq!(pages(5), "5 pages of 9");
    }
}
