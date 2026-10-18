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

/// Messages of `gtkterm-core` (errors, port events) as templates: the core writes English, the
/// window shows them through [`core`]. Keep in sync with the `#[error]`/`format!` texts there.
const CORE_MESSAGES: &[&str] = &[
    // SessionError
    crate::n_!("port is not open"),
    crate::n_!("the send queue is full; discard it with Flush Buffers"),
    crate::n_!("cancelled"),
    crate::n_!("session has shut down"),
    crate::n_!("timed out after {duration} waiting for a match"),
    crate::n_!("write failed: {error}"),
    crate::n_!("log file: {error}"),
    crate::n_!("not logging"),
    crate::n_!("a file transfer is in progress"),
    // PortError
    crate::n_!("{path} is locked by another program"),
    crate::n_!("connection to {endpoint} timed out"),
    crate::n_!("no remote address known yet; wait for a datagram or configure a remote host"),
    crate::n_!("not supported: {what}"),
    crate::n_!("port is closed"),
    crate::n_!("invalid port settings: {what}"),
    crate::n_!("cannot open {path}"),
    crate::n_!("cannot lock {path}"),
    crate::n_!("cannot configure {path}"),
    crate::n_!("{path} is not a serial device"),
    crate::n_!("cannot connect to {endpoint}"),
    crate::n_!("cannot listen on {address}"),
    crate::n_!("cannot bind UDP {address}"),
    crate::n_!("cannot resolve {host}"),
    crate::n_!("write to {path} failed"),
    crate::n_!("read from {path} failed"),
    crate::n_!("{what} (device has no modem lines)"),
    // Port and session events
    crate::n_!("client {peer} connected"),
    crate::n_!("client {peer} disconnected"),
    crate::n_!("client {peer} disconnected: {error}"),
    crate::n_!("client {peer} refused: {count} clients connected"),
    crate::n_!("client {peer} dropped: not reading"),
    crate::n_!("remote set to {address}"),
    crate::n_!("not sent: {error}"),
    crate::n_!("decoder: detected {protocol}"),
    crate::n_!("logging stopped: {error}"),
];

/// The templates of [`CORE_MESSAGES`] as anchored patterns with named groups.
fn core_patterns() -> &'static [(regex::Regex, &'static str)] {
    static PATTERNS: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> = std::sync::OnceLock::new();
    PATTERNS.get_or_init(|| {
        CORE_MESSAGES
            .iter()
            .filter_map(|t| {
                let mut re = String::from("^");
                let mut rest = *t;
                while let Some(start) = rest.find('{') {
                    let end = rest[start..].find('}')? + start;
                    re.push_str(&regex::escape(&rest[..start]));
                    re.push_str("(?P<");
                    re.push_str(&rest[start + 1..end]);
                    re.push_str(">.+?)");
                    rest = &rest[end + 1..];
                }
                re.push_str(&regex::escape(rest));
                re.push('$');
                Some((regex::Regex::new(&re).ok()?, *t))
            })
            .collect()
    })
}

/// Translate a message written by `gtkterm-core` (an error or port event). Known templates
/// are translated with their parts translated in turn (`write failed: cannot open …`); for
/// `context: cause` texts the context and the cause are translated separately. Anything else
/// (OS errors, already translated text) is returned unchanged.
pub fn core(text: &str) -> String {
    core_with(text, &|t| gettext(t), 0).unwrap_or_else(|| text.to_owned())
}

/// `None` if no template matched.
fn core_with(text: &str, tr: &dyn Fn(&str) -> String, depth: u8) -> Option<String> {
    if depth > 4 || text.is_empty() {
        return None;
    }
    let part = |t: &str| core_with(t, tr, depth + 1).unwrap_or_else(|| t.to_owned());
    for (re, template) in core_patterns() {
        if let Some(caps) = re.captures(text) {
            // Only an `{error}` may hold a "context: cause" text; a path that swallowed one
            // belongs to the split below.
            let swallowed = re
                .capture_names()
                .flatten()
                .any(|n| n != "error" && caps.name(n).is_some_and(|m| m.as_str().contains(": ")));
            if swallowed {
                continue;
            }
            let mut out = tr(template);
            for name in re.capture_names().flatten() {
                let value = caps.name(name).map_or("", |m| m.as_str());
                out = out.replace(&format!("{{{name}}}"), &part(value));
            }
            return Some(out);
        }
    }
    // `PortError::Io` is "{context}: {source}".
    let (head, tail) = text.split_once(": ")?;
    let head = core_with(head, tr, depth + 1)?;
    Some(format!("{head}: {}", part(tail)))
}

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
    fn core_messages_are_matched_with_their_parts() {
        // A fake catalogue that marks what it translated.
        let tr = |t: &str| format!("«{t}»");
        let t = |text: &str| core_with(text, &tr, 0);
        assert_eq!(t("port is not open").as_deref(), Some("«port is not open»"));
        assert_eq!(
            t("write failed: cannot open /dev/ttyUSB0: Permission denied (os error 13)").as_deref(),
            Some("«write failed: «cannot open /dev/ttyUSB0»: Permission denied (os error 13)»")
        );
        assert_eq!(
            t("client 10.0.0.2:5000 disconnected: connection reset").as_deref(),
            Some("«client 10.0.0.2:5000 disconnected: connection reset»")
        );
        assert_eq!(
            t("/dev/ttyS4 is locked by another program").as_deref(),
            Some("«/dev/ttyS4 is locked by another program»")
        );
        assert_eq!(t("something else entirely"), None);
        assert_eq!(t("Verbindung getrennt"), None, "translated text is left alone");
        // Without a catalogue the text comes back as it was.
        assert_eq!(
            core("write failed: port is closed"),
            "write failed: port is closed"
        );
        assert_eq!(
            core_patterns().len(),
            CORE_MESSAGES.len(),
            "every template compiles"
        );
    }

    #[test]
    fn core_message_templates_match_the_core_texts() {
        use gtkterm_core::port::PortError;
        use gtkterm_core::session::SessionError;
        let texts = [
            SessionError::NotOpen.to_string(),
            SessionError::Full.to_string(),
            SessionError::Busy.to_string(),
            SessionError::Timeout(std::time::Duration::from_secs(5)).to_string(),
            PortError::Locked("/dev/ttyS0".into()).to_string(),
            PortError::NoRemote.to_string(),
            PortError::Closed.to_string(),
        ];
        for t in texts {
            assert!(
                core_patterns().iter().any(|(re, _)| re.is_match(&t)),
                "no template for {t:?}"
            );
        }
    }

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
