//! Reading and writing missions on disk: `TENDRIL_HOME/Missions/<id>-<SafeTitle>/mission.yaml`.

use crate::error::{Result, TendrilError};
use crate::fs_lock::{write_atomic, FileLock};
use crate::missions::model::{MissionFile, MissionYaml};
use crate::plans::helpers::{allocate_plan_id, to_safe_title};
use chrono::Utc;
use std::path::{Path, PathBuf};

pub const MISSION_FILE: &str = "mission.yaml";

pub fn missions_dir(tendril_home: &Path) -> PathBuf {
    tendril_home.join("Missions")
}

/// Creates the mission folder and its `mission.yaml`, allocating the next id.
pub fn create_mission(missions_dir: &Path, mission: &MissionYaml) -> Result<MissionFile> {
    std::fs::create_dir_all(missions_dir)?;
    let id = allocate_plan_id(missions_dir)?;
    let safe_title = to_safe_title(&mission.title);
    let safe_title = if safe_title.is_empty() {
        "Mission".to_string()
    } else {
        safe_title
    };
    let folder = missions_dir.join(format!("{}-{}", id, safe_title));
    if folder.exists() {
        return Err(TendrilError::Mission(format!(
            "Mission directory already exists: {}",
            folder.display()
        )));
    }
    std::fs::create_dir_all(&folder)?;
    if let Err(e) = write_mission(&folder, mission) {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(e);
    }
    read_mission_file(&folder)
}

pub fn read_mission(folder: &Path) -> Result<MissionYaml> {
    let path = folder.join(MISSION_FILE);
    if !path.exists() {
        return Err(TendrilError::MissionNotFound(format!(
            "{} not found in {}",
            MISSION_FILE,
            folder.display()
        )));
    }
    let raw = std::fs::read_to_string(&path)?;
    serde_yaml::from_str(&raw).map_err(|e| {
        TendrilError::Mission(format!("Failed to parse {}: {}", path.display(), e))
    })
}

pub fn read_mission_file(folder: &Path) -> Result<MissionFile> {
    let mission = read_mission(folder)?;
    let folder_name = folder
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    Ok(MissionFile {
        id: folder_name.chars().take(5).collect(),
        folder_name,
        folder_path: folder.to_string_lossy().to_string(),
        mission,
    })
}

/// Replaces `mission.yaml` atomically under its lock. Stamps `updated`.
pub fn write_mission(folder: &Path, mission: &MissionYaml) -> Result<()> {
    let path = folder.join(MISSION_FILE);
    let _lock = FileLock::acquire(&path)?;
    write_unlocked(&path, mission)
}

fn write_unlocked(path: &Path, mission: &MissionYaml) -> Result<()> {
    let mut mission = mission.clone();
    mission.updated = Utc::now();
    let raw = serde_yaml::to_string(&mission)
        .map_err(|e| TendrilError::Mission(format!("Failed to serialize mission: {}", e)))?;
    write_atomic(path, raw.as_bytes())
}

/// Read-modify-write under one lock, so the CLI (a decision, an approval) and the driver cannot drop
/// each other's change. `f` returns whatever the caller wants back; returning `Err` writes nothing.
pub fn update_mission<T>(
    folder: &Path,
    f: impl FnOnce(&mut MissionYaml) -> Result<T>,
) -> Result<T> {
    let path = folder.join(MISSION_FILE);
    let _lock = FileLock::acquire(&path)?;
    let mut mission = read_mission(folder)?;
    let out = f(&mut mission)?;
    write_unlocked(&path, &mission)?;
    Ok(out)
}

/// Every readable mission, newest first. Unreadable folders are skipped, not fatal.
pub fn list_missions(missions_dir: &Path) -> Vec<MissionFile> {
    let Ok(entries) = std::fs::read_dir(missions_dir) else {
        return Vec::new();
    };
    let mut out: Vec<MissionFile> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(MISSION_FILE).is_file())
        .filter_map(|p| read_mission_file(&p).ok())
        .collect();
    out.sort_by(|a, b| b.folder_name.cmp(&a.folder_name));
    out
}

/// Resolves `12`, `00012`, `00012-AddSso` or an absolute folder path to the mission folder.
pub fn resolve_mission_folder(reference: &str, missions_dir: &Path) -> Result<PathBuf> {
    let reference = reference.trim();
    let as_path = PathBuf::from(reference);
    if as_path.is_absolute() && as_path.join(MISSION_FILE).is_file() {
        return Ok(as_path);
    }
    let direct = missions_dir.join(reference);
    if direct.join(MISSION_FILE).is_file() {
        return Ok(direct);
    }
    let id = match reference.parse::<u32>() {
        Ok(n) => format!("{:05}", n),
        Err(_) => reference.to_string(),
    };
    if let Ok(entries) = std::fs::read_dir(missions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{}-", id)) && entry.path().join(MISSION_FILE).is_file() {
                return Ok(entry.path());
            }
        }
    }
    Err(TendrilError::MissionNotFound(format!(
        "No mission '{}' in {}",
        reference,
        missions_dir.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::missions::model::MissionState;

    #[test]
    fn create_list_resolve_and_update() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Missions");
        let a = create_mission(&root, &MissionYaml::new("Add SSO", "goal", "P")).unwrap();
        let b = create_mission(&root, &MissionYaml::new("Second one", "goal", "P")).unwrap();
        assert_eq!(a.folder_name, "00001-AddSSO");
        assert_eq!(b.id, "00002");

        let listed = list_missions(&root);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "00002", "newest first");

        assert_eq!(resolve_mission_folder("1", &root).unwrap(), root.join("00001-AddSSO"));
        assert_eq!(resolve_mission_folder("00001-AddSSO", &root).unwrap(), root.join("00001-AddSSO"));
        assert!(resolve_mission_folder("9", &root).is_err());

        let folder = PathBuf::from(&a.folder_path);
        update_mission(&folder, |m| {
            m.state = MissionState::Running;
            Ok(())
        })
        .unwrap();
        assert_eq!(read_mission(&folder).unwrap().state, MissionState::Running);

        // An Err from the closure writes nothing.
        let _ = update_mission(&folder, |m| -> Result<()> {
            m.state = MissionState::Cancelled;
            Err(TendrilError::Mission("no".into()))
        });
        assert_eq!(read_mission(&folder).unwrap().state, MissionState::Running);
    }
}
