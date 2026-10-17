//! File transfer dialogs: choose the protocol (and the XMODEM file name), then the files or the
//! target folder.

use std::rc::Rc;

use adw::prelude::*;
use gtkterm_core::transfer::TransferProtocol;

/// What the user picked in the protocol dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Send the file's bytes as they are.
    Raw,
    /// A transfer protocol; `name` is the XMODEM file name when receiving.
    Protocol(TransferProtocol, Option<String>),
}

/// Ask for the protocol. `sending` adds the raw option; receiving adds the XMODEM name entry.
pub fn choose(
    parent: &impl IsA<gtk::Widget>,
    sending: bool,
    last: Option<&str>,
    on_choice: impl Fn(Choice) + 'static,
) -> adw::AlertDialog {
    let available: Vec<TransferProtocol> = TransferProtocol::ALL
        .into_iter()
        .filter(|p| p.available())
        .collect();
    let mut labels: Vec<String> = Vec::new();
    if sending {
        labels.push("Raw (send the bytes as they are)".into());
    }
    labels.extend(available.iter().map(|p| p.label().to_owned()));
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let drop = gtk::DropDown::builder()
        .model(&gtk::StringList::new(&refs))
        .build();
    let offset = u32::from(sending);
    if let Some(i) = last.and_then(|l| available.iter().position(|p| p.id() == l)) {
        drop.set_selected(offset + u32::try_from(i).unwrap_or(0));
    } else if let Some(i) = available.iter().position(|p| *p == TransferProtocol::Ymodem) {
        drop.set_selected(offset + u32::try_from(i).unwrap_or(0));
    }
    let name = gtk::Entry::builder()
        .placeholder_text("File name (XMODEM carries none)")
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.append(&drop);
    if !sending {
        content.append(&name);
    }
    let note = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .label(if TransferProtocol::Zmodem.available() {
            ""
        } else {
            "ZMODEM needs the lrzsz package (sz/rz)."
        })
        .build();
    note.add_css_class("dim-label");
    note.set_visible(!TransferProtocol::Zmodem.available());
    content.append(&note);
    let protocols = Rc::new(available);
    let update = {
        let (protocols, name) = (Rc::clone(&protocols), name.clone());
        move |d: &gtk::DropDown| {
            let p = d
                .selected()
                .checked_sub(offset)
                .and_then(|i| protocols.get(i as usize).copied());
            name.set_visible(!sending && p.is_some_and(|p| !p.batch()));
        }
    };
    update(&drop);
    drop.connect_selected_notify(update);

    let dialog = adw::AlertDialog::builder()
        .heading(if sending { "Send File" } else { "Receive File" })
        .body(if sending {
            "Start the receiver on the device first (e.g. rx, rb, rz, or the bootloader's upload prompt)."
        } else {
            "Choose the protocol and the folder; then start the sender on the device."
        })
        .extra_child(&content)
        .build();
    dialog.add_responses(&[
        ("cancel", "Cancel"),
        (
            "go",
            if sending {
                "Choose Files…"
            } else {
                "Choose Folder…"
            },
        ),
    ]);
    dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("go"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response != "go" {
            return;
        }
        let i = drop.selected();
        let choice = match i.checked_sub(offset) {
            None => Choice::Raw,
            Some(i) => match protocols.get(i as usize) {
                Some(p) => {
                    let n = name.text().trim().to_owned();
                    Choice::Protocol(*p, (!n.is_empty()).then_some(n))
                }
                None => return,
            },
        };
        on_choice(choice);
    });
    dialog.present(Some(parent));
    dialog
}
