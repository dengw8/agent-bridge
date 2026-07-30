use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{
    AppConfig, Binding, CommandSlot, Direction, EncoderMode, InitializationSource, Layout,
    ThemePreference,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConfigCommand {
    SetCommand {
        slot: CommandSlot,
        binding: Binding,
    },
    SetJoystick {
        direction: Direction,
        binding: Binding,
    },
    SetEncoderMode {
        mode: EncoderMode,
    },
    PreviewImport,
    CommitImport {
        revision: String,
    },
    Undo,
    RestoreDefaults {
        confirmed: bool,
    },
    SetPreferences {
        #[serde(rename = "launchAtLogin")]
        launch_at_login: Option<bool>,
        #[serde(rename = "onboardingComplete")]
        onboarding_complete: Option<bool>,
        #[serde(rename = "themeMode")]
        theme_mode: Option<ThemePreference>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewItem {
    pub position: String,
    pub current: String,
    pub imported: String,
    pub result: String,
}

#[derive(Debug, Clone)]
struct PendingImport {
    revision: String,
    layout: Layout,
}

#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    current: AppConfig,
    undo: Option<AppConfig>,
    pending_import: Option<PendingImport>,
}

impl ConfigStore {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let existed = path.exists();
        let current = if existed {
            match fs::read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|raw| {
                    serde_json::from_str::<AppConfig>(&raw).map_err(|error| error.to_string())
                })
                .and_then(|config| {
                    config.layout.validate()?;
                    if config.schema_version != 1 {
                        return Err(format!(
                            "Unsupported config schema {}",
                            config.schema_version
                        ));
                    }
                    Ok(config)
                }) {
                Ok(config) => config,
                Err(error) => {
                    let backup = path.with_extension(format!(
                        "corrupt-{}.json",
                        Utc::now().format("%Y%m%dT%H%M%SZ")
                    ));
                    fs::rename(&path, &backup).map_err(|rename_error| {
                        format!("{error}; failed to preserve corrupt config: {rename_error}")
                    })?;
                    AppConfig::default()
                }
            }
        } else {
            AppConfig::default()
        };
        let store = Self {
            path,
            current,
            undo: None,
            pending_import: None,
        };
        if !existed {
            store.save_current()?;
        }
        Ok(store)
    }

    pub fn current(&self) -> &AppConfig {
        &self.current
    }

    pub fn needs_initialization(&self) -> bool {
        !self.current.initialization.complete
    }

    pub fn apply(&mut self, command: ConfigCommand) -> Result<(), String> {
        let current = self.current.clone();
        let undo = self.undo.clone();
        let pending_import = self.pending_import.clone();
        if let Err(error) = self.apply_mutating(command) {
            self.current = current;
            self.undo = undo;
            self.pending_import = pending_import;
            return Err(error);
        }
        Ok(())
    }

    fn apply_mutating(&mut self, command: ConfigCommand) -> Result<(), String> {
        let previous = self.current.clone();
        match command {
            ConfigCommand::SetCommand { slot, binding } => {
                binding.validate()?;
                self.current.layout.command_slots.insert(slot, binding);
                self.current.initialization.complete = true;
            }
            ConfigCommand::SetJoystick { direction, binding } => {
                binding.validate()?;
                if matches!(binding, Binding::Keycap { .. }) {
                    return Err("Joystick mappings must be a command, skill, or None.".into());
                }
                self.current.layout.joystick.insert(direction, binding);
                self.current.initialization.complete = true;
            }
            ConfigCommand::SetEncoderMode { mode } => {
                self.current.layout.encoder_mode = mode;
                self.current.initialization.complete = true;
            }
            ConfigCommand::Undo => {
                let prior = self
                    .undo
                    .take()
                    .ok_or_else(|| "Nothing to undo.".to_string())?;
                self.current = prior;
                self.save_current()?;
                self.pending_import = None;
                return Ok(());
            }
            ConfigCommand::RestoreDefaults { confirmed } => {
                if !confirmed {
                    return Err("Restoring defaults requires confirmation.".into());
                }
                self.current.layout = Layout::official_defaults();
                self.current.initialization.source = InitializationSource::OfficialDefaults;
                self.current.initialization.imported_at = None;
                self.current.initialization.imported_from_codex_version = None;
                self.current.initialization.complete = true;
            }
            ConfigCommand::SetPreferences {
                launch_at_login,
                onboarding_complete,
                theme_mode,
            } => {
                if let Some(value) = launch_at_login {
                    self.current.preferences.launch_at_login = value;
                }
                if let Some(value) = onboarding_complete {
                    self.current.preferences.onboarding_complete = value;
                }
                if let Some(value) = theme_mode {
                    self.current.preferences.theme_mode = value;
                }
            }
            ConfigCommand::PreviewImport | ConfigCommand::CommitImport { .. } => {
                return Err("Import commands require a prepared Renderer snapshot.".into())
            }
        }
        self.current.layout.validate()?;
        self.save_current()?;
        self.undo = Some(previous);
        self.pending_import = None;
        Ok(())
    }

    pub fn preview_import(
        &mut self,
        imported: Layout,
    ) -> Result<(String, Vec<ImportPreviewItem>), String> {
        imported.validate_importable()?;
        let revision = Uuid::new_v4().to_string();
        let preview = diff_layout(&self.current.layout, &imported);
        let imported = sanitized_import(&self.current.layout, imported);
        imported.validate()?;
        self.pending_import = Some(PendingImport {
            revision: revision.clone(),
            layout: imported,
        });
        Ok((revision, preview))
    }

    pub fn commit_import(
        &mut self,
        revision: &str,
        codex_version: Option<String>,
    ) -> Result<(), String> {
        let current = self.current.clone();
        let undo = self.undo.clone();
        let pending_import = self.pending_import.clone();
        if let Err(error) = self.commit_import_mutating(revision, codex_version) {
            self.current = current;
            self.undo = undo;
            self.pending_import = pending_import;
            return Err(error);
        }
        Ok(())
    }

    fn commit_import_mutating(
        &mut self,
        revision: &str,
        codex_version: Option<String>,
    ) -> Result<(), String> {
        let pending = self
            .pending_import
            .take()
            .ok_or_else(|| "No import preview is pending.".to_string())?;
        if pending.revision != revision {
            self.pending_import = Some(pending);
            return Err("Import preview is stale.".into());
        }
        self.install_import(pending.layout, codex_version)
    }

    pub fn initialize_from_codex(
        &mut self,
        imported: Layout,
        codex_version: Option<String>,
    ) -> Result<bool, String> {
        if !self.needs_initialization() {
            return Ok(false);
        }
        imported.validate_importable()?;
        let imported = sanitized_import(&self.current.layout, imported);
        self.install_import(imported, codex_version)?;
        Ok(true)
    }

    fn install_import(
        &mut self,
        imported: Layout,
        codex_version: Option<String>,
    ) -> Result<(), String> {
        imported.validate()?;
        let previous = self.current.clone();
        self.current.layout = imported;
        self.current.initialization.source = InitializationSource::CodexMicro;
        self.current.initialization.imported_from_codex_version = codex_version;
        self.current.initialization.imported_at = Some(Utc::now().to_rfc3339());
        self.current.initialization.complete = true;
        if let Err(error) = self.save_current() {
            self.current = previous;
            return Err(error);
        }
        self.undo = Some(previous);
        Ok(())
    }

    fn save_current(&self) -> Result<(), String> {
        atomic_write_json(&self.path, &self.current)
    }
}

fn diff_layout(current: &Layout, imported: &Layout) -> Vec<ImportPreviewItem> {
    let mut items = Vec::new();
    for slot in CommandSlot::ALL {
        let before = current
            .command_slots
            .get(&slot)
            .map(Binding::label)
            .unwrap_or_default();
        let imported_binding = imported.command_slots.get(&slot);
        let after = imported_binding.map(Binding::label).unwrap_or_default();
        if before != after {
            let result = if matches!(imported_binding, Some(Binding::Unsupported { .. })) {
                format!("Unsupported · keeping {before}")
            } else {
                after.clone()
            };
            items.push(ImportPreviewItem {
                position: slot.as_str().into(),
                current: before,
                imported: after,
                result,
            });
        }
    }
    for direction in Direction::ALL {
        let before = current
            .joystick
            .get(&direction)
            .map(Binding::label)
            .unwrap_or_default();
        let imported_binding = imported.joystick.get(&direction);
        let after = imported_binding.map(Binding::label).unwrap_or_default();
        if before != after {
            let result = if matches!(imported_binding, Some(Binding::Unsupported { .. })) {
                format!("Unsupported · keeping {before}")
            } else {
                after.clone()
            };
            items.push(ImportPreviewItem {
                position: format!("joystick.{}", direction.as_str()),
                current: before,
                imported: after,
                result,
            });
        }
    }
    if current.encoder_mode != imported.encoder_mode {
        items.push(ImportPreviewItem {
            position: "encoderMode".into(),
            current: format!("{:?}", current.encoder_mode),
            imported: format!("{:?}", imported.encoder_mode),
            result: format!("{:?}", imported.encoder_mode),
        });
    }
    items
}

fn sanitized_import(current: &Layout, mut imported: Layout) -> Layout {
    for slot in CommandSlot::ALL {
        if matches!(
            imported.command_slots.get(&slot),
            Some(Binding::Unsupported { .. })
        ) {
            if let Some(binding) = current.command_slots.get(&slot) {
                imported.command_slots.insert(slot, binding.clone());
            }
        }
    }
    for direction in Direction::ALL {
        if matches!(
            imported.joystick.get(&direction),
            Some(Binding::Unsupported { .. })
        ) {
            if let Some(binding) = current.joystick.get(&direction) {
                imported.joystick.insert(direction, binding.clone());
            }
        }
    }
    imported
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Config path has no parent.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temp = parent.join(format!(".config-{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let mut file = File::create(&temp).map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        error.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_single_changes_atomically_and_undoes_them() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut store = ConfigStore::load(path.clone()).unwrap();
        store
            .apply(ConfigCommand::SetCommand {
                slot: CommandSlot::Act06,
                binding: Binding::Keycap { id: "TERM".into() },
            })
            .unwrap();
        assert_eq!(
            store.current().layout.command_slots[&CommandSlot::Act06].label(),
            "TERM"
        );
        store.apply(ConfigCommand::Undo).unwrap();
        assert_eq!(
            store.current().layout.command_slots[&CommandSlot::Act06].label(),
            "FAST"
        );
        assert!(directory.path().read_dir().unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        let persisted: AppConfig =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(persisted, *store.current());
    }

    #[test]
    fn import_is_preview_only_until_exact_revision_is_committed() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = ConfigStore::load(directory.path().join("config.json")).unwrap();
        let mut imported = Layout::official_defaults();
        imported
            .command_slots
            .insert(CommandSlot::Act06, Binding::Keycap { id: "TERM".into() });
        let (revision, preview) = store.preview_import(imported).unwrap();
        assert_eq!(preview.len(), 1);
        assert_eq!(
            store.current().layout.command_slots[&CommandSlot::Act06].label(),
            "FAST"
        );
        assert!(store.commit_import("stale", None).is_err());
        store.commit_import(&revision, Some("26.0".into())).unwrap();
        assert_eq!(
            store.current().layout.command_slots[&CommandSlot::Act06].label(),
            "TERM"
        );
    }

    #[test]
    fn invalid_import_and_unconfirmed_restore_preserve_config() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = ConfigStore::load(directory.path().join("config.json")).unwrap();
        let before = store.current().clone();
        let mut invalid = Layout::official_defaults();
        invalid.command_slots.remove(&CommandSlot::Act12);
        assert!(store.preview_import(invalid).is_err());
        assert!(store
            .apply(ConfigCommand::RestoreDefaults { confirmed: false })
            .is_err());
        assert_eq!(before, *store.current());
    }

    #[test]
    fn preferences_do_not_skip_first_successful_codex_initialization() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = ConfigStore::load(directory.path().join("config.json")).unwrap();
        store
            .apply(ConfigCommand::SetPreferences {
                launch_at_login: Some(true),
                onboarding_complete: Some(true),
                theme_mode: None,
            })
            .unwrap();
        assert!(store.needs_initialization());
        let mut imported = Layout::official_defaults();
        imported
            .command_slots
            .insert(CommandSlot::Act06, Binding::Keycap { id: "TERM".into() });
        assert!(store
            .initialize_from_codex(imported, Some("26.721.81911".into()))
            .unwrap());
        assert!(!store.needs_initialization());
        assert_eq!(
            store.current().layout.command_slots[&CommandSlot::Act06].label(),
            "TERM"
        );
    }

    #[test]
    fn theme_preference_defaults_for_legacy_config_and_persists() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut legacy = serde_json::to_value(AppConfig::default()).unwrap();
        legacy["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("themeMode");
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

        let mut store = ConfigStore::load(path.clone()).unwrap();
        assert_eq!(
            store.current().preferences.theme_mode,
            crate::model::ThemePreference::System
        );
        store
            .apply(ConfigCommand::SetPreferences {
                launch_at_login: None,
                onboarding_complete: None,
                theme_mode: Some(crate::model::ThemePreference::Dark),
            })
            .unwrap();
        drop(store);

        let reloaded = ConfigStore::load(path).unwrap();
        assert_eq!(
            reloaded.current().preferences.theme_mode,
            crate::model::ThemePreference::Dark
        );
    }

    #[test]
    fn failed_save_preserves_runtime_config_and_transaction_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut store = ConfigStore::load(path.clone()).unwrap();
        let imported = Layout::official_defaults();
        let (pending_revision, _) = store.preview_import(imported).unwrap();
        let current_before = store.current.clone();
        let undo_before = store.undo.clone();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        let result = store.apply(ConfigCommand::SetCommand {
            slot: CommandSlot::Act06,
            binding: Binding::Keycap { id: "TERM".into() },
        });

        assert!(result.is_err());
        assert_eq!(store.current, current_before);
        assert_eq!(store.undo, undo_before);
        assert_eq!(
            store
                .pending_import
                .as_ref()
                .map(|pending| pending.revision.as_str()),
            Some(pending_revision.as_str())
        );
    }

    #[test]
    fn failed_import_commit_preserves_the_pending_revision() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut store = ConfigStore::load(path.clone()).unwrap();
        let current_before = store.current.clone();
        let undo_before = store.undo.clone();
        let mut imported = Layout::official_defaults();
        imported
            .command_slots
            .insert(CommandSlot::Act06, Binding::Keycap { id: "TERM".into() });
        let (revision, _) = store.preview_import(imported).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        let result = store.commit_import(&revision, Some("26.0".into()));

        assert!(result.is_err());
        assert_eq!(store.current, current_before);
        assert_eq!(store.undo, undo_before);
        assert_eq!(
            store
                .pending_import
                .as_ref()
                .map(|pending| pending.revision.as_str()),
            Some(revision.as_str())
        );
    }

    #[test]
    fn failed_undo_preserves_the_current_config_and_retryable_undo() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut store = ConfigStore::load(path.clone()).unwrap();
        store
            .apply(ConfigCommand::SetCommand {
                slot: CommandSlot::Act06,
                binding: Binding::Keycap { id: "TERM".into() },
            })
            .unwrap();
        let current_before = store.current.clone();
        let undo_before = store.undo.clone();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        let result = store.apply(ConfigCommand::Undo);

        assert!(result.is_err());
        assert_eq!(store.current, current_before);
        assert_eq!(store.undo, undo_before);
    }

    #[test]
    fn unsupported_import_binding_is_reported_and_keeps_the_current_position() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = ConfigStore::load(directory.path().join("config.json")).unwrap();
        let mut imported = Layout::official_defaults();
        imported.command_slots.insert(
            CommandSlot::Act06,
            Binding::Unsupported {
                label: "Unknown keycap FUTURE".into(),
            },
        );
        imported
            .command_slots
            .insert(CommandSlot::Act07, Binding::Keycap { id: "TERM".into() });

        let (revision, preview) = store.preview_import(imported).unwrap();

        let unsupported = preview
            .iter()
            .find(|item| item.position == "ACT06")
            .unwrap();
        assert_eq!(unsupported.imported, "Unknown keycap FUTURE");
        assert_eq!(unsupported.result, "Unsupported · keeping FAST");
        store
            .commit_import(&revision, Some("future".into()))
            .unwrap();
        assert_eq!(
            store.current().layout.command_slots[&CommandSlot::Act06].label(),
            "FAST"
        );
        assert_eq!(
            store.current().layout.command_slots[&CommandSlot::Act07].label(),
            "TERM"
        );
    }
}
