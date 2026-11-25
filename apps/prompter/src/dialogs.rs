//! The library's dialogs: naming a script or folder, and confirming a delete.

use prompter_core::store::StoreError;
use ui_kit::tokens::color;

use crate::workspace::Workspace;

/// What a name being typed is for.
#[derive(Debug, Clone)]
pub enum Naming {
    NewFolder,
    Folder(String),
    /// The script with this id.
    Script(String),
}

#[derive(Debug)]
pub enum Dialog {
    Name {
        naming: Naming,
        text: String,
        error: Option<String>,
        /// The text field still has to take the keyboard.
        focus: bool,
    },
    Delete {
        id: String,
        title: String,
    },
}

impl Dialog {
    /// A name dialog that starts with `text` in its field.
    pub fn name(naming: Naming, text: &str) -> Self {
        Self::Name { naming, text: text.to_owned(), error: None, focus: true }
    }

    /// Draws the dialog. Returns false once it is closed. A failed delete is returned for the
    /// caller to report. A refused name is shown in the dialog, which stays open.
    pub fn show(&mut self, ctx: &egui::Context, workspace: &mut Workspace) -> (bool, Option<StoreError>) {
        let mut open = true;
        let (mut accepted, mut cancelled) = (false, false);
        let mut failure = None;
        match self {
            Self::Name { naming, text, error, focus } => {
                let (title, button, hint) = match naming {
                    Naming::NewFolder => ("New folder", "Create", "Folder name"),
                    Naming::Folder(_) => ("Rename folder", "Rename", "Folder name"),
                    Naming::Script(_) => ("Rename script", "Rename", "Script title"),
                };
                let valid = !text.trim().is_empty();
                let mut entered = false;
                ui_kit::dialog(
                    ctx,
                    egui::Id::new("name-dialog"),
                    title,
                    &mut open,
                    |ui| {
                        ui.set_width(320.0);
                        let field = ui_kit::text_field(ui, text, hint);
                        if std::mem::take(focus) {
                            field.request_focus();
                        }
                        entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if field.changed() {
                            *error = None;
                        }
                        if let Some(error) = error {
                            ui.colored_label(color::TEXT_SECONDARY, error.as_str());
                        }
                    },
                    |ui| {
                        cancelled = ui.button("Cancel").clicked();
                        accepted = ui.add_enabled_ui(valid, |ui| ui_kit::primary_button(ui, button).clicked()).inner;
                    },
                );
                if (accepted || entered) && !text.trim().is_empty() {
                    let result = match naming {
                        Naming::NewFolder => workspace.create_folder(text),
                        Naming::Folder(from) => workspace.rename_folder(from, text),
                        Naming::Script(id) => workspace.rename(id, text),
                    };
                    match result {
                        Ok(()) => open = false,
                        Err(refused) => {
                            *error = Some(refused.to_string());
                            *focus = true;
                        }
                    }
                }
            }
            Self::Delete { id, title } => {
                ui_kit::dialog(
                    ctx,
                    egui::Id::new("delete-script-dialog"),
                    "Delete script?",
                    &mut open,
                    |ui| {
                        ui.set_width(320.0);
                        ui.label(format!("\"{title}\" will be removed from the library. A copy stays in its history folder."));
                    },
                    |ui| {
                        cancelled = ui.button("Cancel").clicked();
                        accepted = ui_kit::primary_button(ui, "Delete").clicked();
                    },
                );
                if accepted {
                    failure = workspace.delete(id).err();
                    open = false;
                }
            }
        }
        (open && !cancelled, failure)
    }
}
