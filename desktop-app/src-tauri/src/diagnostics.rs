use std::{
    collections::VecDeque,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::model::DiagnosticEntry;

const MAX_ENTRIES: usize = 200;

#[derive(Debug, Default)]
pub struct Diagnostics {
    entries: VecDeque<DiagnosticEntry>,
}

impl Diagnostics {
    pub fn record(
        &mut self,
        category: &'static str,
        outcome: &'static str,
        duration_ms: Option<u64>,
        code: Option<&'static str>,
    ) {
        if self.entries.len() == MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(DiagnosticEntry {
            at: Utc::now().to_rfc3339(),
            category: category.into(),
            outcome: outcome.into(),
            duration_ms,
            code: code.map(str::to_owned),
        });
    }

    pub fn recent(&self) -> Vec<DiagnosticEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn export(&self, directory: &Path) -> Result<PathBuf, String> {
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let path = directory.join(format!(
            "agent-bridge-diagnostics-{}.json",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        let envelope = DiagnosticExport {
            schema_version: 1,
            generated_at: Utc::now().to_rfc3339(),
            entries: self.recent(),
        };
        let bytes = serde_json::to_vec_pretty(&envelope).map_err(|error| error.to_string())?;
        let temporary = directory.join(format!(".diagnostics-{}.tmp", Uuid::new_v4()));
        let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, &path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            error.to_string()
        })?;
        Ok(path)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticExport {
    schema_version: u8,
    generated_at: String,
    entries: Vec<DiagnosticEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_contract_cannot_accept_or_export_user_content() {
        let directory = tempfile::tempdir().unwrap();
        let mut diagnostics = Diagnostics::default();
        diagnostics.record("dispatch", "failed", Some(4), Some("renderer_unavailable"));
        let path = diagnostics.export(directory.path()).unwrap();
        let raw = fs::read_to_string(path).unwrap();
        assert!(raw.contains("renderer_unavailable"));
        for forbidden in [
            "title",
            "body",
            "terminal",
            "diff",
            "audio",
            "token",
            "credential",
        ] {
            assert!(!raw.to_lowercase().contains(&format!("\"{forbidden}\"")));
        }
    }

    #[test]
    fn entries_are_bounded_and_clearable() {
        let mut diagnostics = Diagnostics::default();
        for _ in 0..250 {
            diagnostics.record("poll", "ok", None, None);
        }
        assert_eq!(diagnostics.recent().len(), MAX_ENTRIES);
        diagnostics.clear();
        assert!(diagnostics.recent().is_empty());
    }
}
