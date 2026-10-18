//! Translations (gettext, domain `gtkterm-sunshine`).
//!
//! Strings are marked with [`gettext`], [`gettext_f`] (named `{placeholders}`), [`ngettext_f`]
//! and, where a constant is needed, [`n_!`](crate::n_). `cargo xtask po` extracts them into
//! `po/gtkterm-sunshine.pot`.

use std::path::PathBuf;

pub use gettextrs::gettext;

/// Text domain and `.mo` file name.
pub const DOMAIN: &str = "gtkterm-sunshine";

/// Mark a string for translation in a constant; translate it with [`gettext`] where it is shown.
#[macro_export]
macro_rules! n_ {
    ($s:literal) => {
        $s
    };
}

/// Labels defined in the core and protocol crates and translated where they are shown
/// (`gettext(mode.label())`); listed here so that `cargo xtask po` extracts them.
#[allow(dead_code)]
const EXTERNAL_LABELS: &[&str] = &[
    // gtkterm_protocols::view::ViewMode::label
    crate::n_!("ASCII"),
    crate::n_!("Hexadecimal"),
    crate::n_!("Binary"),
    crate::n_!("Decimal"),
    crate::n_!("Mixed (Hex + ASCII)"),
    crate::n_!("Mixed (Binary + ASCII)"),
    crate::n_!("Mixed (Decimal + ASCII)"),
    // gtkterm_core::decoding::DecoderMode::label and Protocol::label
    crate::n_!("Off"),
    crate::n_!("Auto-detect"),
    crate::n_!("Raw chunks"),
    crate::n_!("AT commands"),
    crate::n_!("RS232 signals"),
    // gtkterm_protocols::sunshine::ChartKind::label
    crate::n_!("Line"),
    crate::n_!("Bar"),
    crate::n_!("Pie"),
    crate::n_!("Scatter"),
];

/// `$GTKTERM_LOCALEDIR`, else `<prefix>/share/locale` next to the binary, else the system one.
fn locale_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GTKTERM_LOCALEDIR") {
        return dir.into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share/locale")))
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| PathBuf::from("/usr/share/locale"))
}

/// Bind the text domain. Call before building any widget; GTK selects the user's locale
/// (`setlocale`) when it initializes.
pub fn init() {
    use gettextrs::{bind_textdomain_codeset, bindtextdomain, textdomain};
    let dir = locale_dir();
    if let Err(e) = bindtextdomain(DOMAIN, &dir) {
        tracing::debug!(dir = %dir.display(), error = %e, "no translations");
        return;
    }
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
}

fn fill(mut s: String, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

/// Translate `msgid` and replace `{name}` placeholders.
pub fn gettext_f(msgid: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(gettext(msgid), args)
}

/// Plural form of [`gettext_f`]; `{n}` is replaced by `n` as well.
pub fn ngettext_f(singular: &str, plural: &str, n: u32, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(gettextrs::ngettext(singular, plural, n), args).replace("{n}", &n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        assert_eq!(
            gettext_f(
                "Port {port} at {baud}",
                &[("port", &"/dev/ttyS0"), ("baud", &9600)]
            ),
            "Port /dev/ttyS0 at 9600"
        );
        assert_eq!(ngettext_f("{n} file", "{n} files", 1, &[]), "1 file");
        assert_eq!(ngettext_f("{n} file", "{n} files", 3, &[]), "3 files");
    }
}
