//! Evidence that a change works: screenshots and short videos a worker records of the real thing.
//!
//! "The tests passed" is not what a person needs to see for UI work. A worker runs the app, records
//! the flow (a login, a trade, a page) and attaches it with `tendril evidence add`; the mission page
//! shows it, and the judge can refuse a UI change that comes without any. Files live in the plan's
//! own `Artifacts` folder, so the existing artifact handling, containment rules and cleanup apply, and
//! `Artifacts/evidence.json` holds the captions.

use crate::error::{Result, TendrilError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
pub const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "webm", "mov", "m4v"];
/// Past this a clip is too big to show in the app. Say so up front rather than store a file nobody can open.
pub const MAX_EVIDENCE_BYTES: u64 = 48 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceKind {
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceItem {
    pub kind: EvidenceKind,
    /// Absolute, as the app's file route wants it.
    pub path: String,
    /// Relative to the plan's `Artifacts` folder, and the key in the manifest.
    pub file: String,
    #[serde(default)]
    pub caption: String,
    /// What the worker was doing when it took it ("Logged in as a trader").
    #[serde(default)]
    pub step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    items: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestEntry {
    file: String,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    step: String,
    #[serde(default)]
    at: Option<String>,
}

fn artifacts_dir(plan_folder: &Path) -> PathBuf {
    plan_folder.join("Artifacts")
}

fn manifest_path(plan_folder: &Path) -> PathBuf {
    artifacts_dir(plan_folder).join("evidence.json")
}

fn read_manifest(plan_folder: &Path) -> Manifest {
    std::fs::read_to_string(manifest_path(plan_folder))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn kind_of(path: &Path) -> Option<EvidenceKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(EvidenceKind::Image)
    } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(EvidenceKind::Video)
    } else {
        None
    }
}

/// Every screenshot and video in the plan's `Artifacts`, with the caption the worker gave it when it has
/// one. A file dropped there without `tendril evidence add` still shows, just without a caption.
pub fn list_evidence(plan_folder: &Path) -> Vec<EvidenceItem> {
    let artifacts = artifacts_dir(plan_folder);
    let manifest = read_manifest(plan_folder);
    let mut found: Vec<(PathBuf, String)> = Vec::new();

    let mut scan = |dir: &Path, prefix: &str| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && kind_of(&path).is_some() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
                    found.push((path, rel));
                }
            }
        }
    };
    scan(&artifacts, "");
    scan(&artifacts.join("screenshots"), "screenshots");
    scan(&artifacts.join("videos"), "videos");

    let mut items: Vec<EvidenceItem> = found
        .into_iter()
        .filter_map(|(path, rel)| {
            let kind = kind_of(&path)?;
            let entry = manifest.items.iter().find(|e| e.file == rel);
            // Without a manifest entry, the file's own modified time says when it was recorded.
            let at = entry.and_then(|e| e.at.clone()).or_else(|| {
                std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
            });
            Some(EvidenceItem {
                kind,
                path: path.to_string_lossy().to_string(),
                file: rel,
                caption: entry.map(|e| e.caption.clone()).unwrap_or_default(),
                step: entry.map(|e| e.step.clone()).unwrap_or_default(),
                at,
            })
        })
        .collect();
    items.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.file.cmp(&b.file)));
    items
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == '-').to_string();
    if cleaned.is_empty() { "evidence".to_string() } else { cleaned }
}

/// Copies `source` into the plan's `Artifacts` as evidence and records its caption.
pub fn add_evidence(plan_folder: &Path, source: &Path, caption: &str, step: &str) -> Result<EvidenceItem> {
    let metadata = std::fs::metadata(source)
        .map_err(|_| TendrilError::Validation(format!("'{}' does not exist", source.display())))?;
    if !metadata.is_file() {
        return Err(TendrilError::Validation(format!("'{}' is not a file", source.display())));
    }
    let kind = kind_of(source).ok_or_else(|| {
        TendrilError::Validation(format!(
            "'{}' is not an image ({}) or a video ({})",
            source.display(),
            IMAGE_EXTENSIONS.join(", "),
            VIDEO_EXTENSIONS.join(", ")
        ))
    })?;
    if metadata.len() > MAX_EVIDENCE_BYTES {
        return Err(TendrilError::Validation(format!(
            "'{}' is {} MB, over the {} MB the app can show. Record a shorter clip or a smaller window.",
            source.display(),
            metadata.len() / (1024 * 1024),
            MAX_EVIDENCE_BYTES / (1024 * 1024)
        )));
    }

    let dir_name = if kind == EvidenceKind::Image { "screenshots" } else { "videos" };
    let dir = artifacts_dir(plan_folder).join(dir_name);
    std::fs::create_dir_all(&dir)?;

    let base = safe_name(source.file_name().and_then(|n| n.to_str()).unwrap_or("evidence"));
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (base.clone(), String::new()),
    };
    let mut dest = dir.join(&base);
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{stem}-{n}{ext}"));
        n += 1;
    }
    std::fs::copy(source, &dest)?;

    let file = format!("{dir_name}/{}", dest.file_name().and_then(|n| n.to_str()).unwrap_or_default());
    let at = chrono::Utc::now().to_rfc3339();
    let mut manifest = read_manifest(plan_folder);
    manifest.items.retain(|e| e.file != file);
    manifest.items.push(ManifestEntry {
        file: file.clone(),
        caption: caption.trim().to_string(),
        step: step.trim().to_string(),
        at: Some(at.clone()),
    });
    std::fs::write(manifest_path(plan_folder), serde_json::to_vec_pretty(&manifest).unwrap_or_default())?;

    Ok(EvidenceItem {
        kind,
        path: dest.to_string_lossy().to_string(),
        file,
        caption: caption.trim().to_string(),
        step: step.trim().to_string(),
        at: Some(at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tendril-evidence-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("Artifacts")).unwrap();
        dir
    }

    #[test]
    fn evidence_is_copied_into_the_right_folder_with_its_caption() {
        let plan = scratch();
        let png = plan.join("login.png");
        let webm = plan.join("flow.webm");
        std::fs::write(&png, b"png").unwrap();
        std::fs::write(&webm, b"webm").unwrap();

        let shot = add_evidence(&plan, &png, "  The trader dashboard after login ", "Logged in").unwrap();
        let clip = add_evidence(&plan, &webm, "Placing a market order", "Trade").unwrap();
        assert_eq!(shot.kind, EvidenceKind::Image);
        assert_eq!(shot.file, "screenshots/login.png");
        assert_eq!(shot.caption, "The trader dashboard after login");
        assert_eq!(clip.kind, EvidenceKind::Video);
        assert_eq!(clip.file, "videos/flow.webm");
        assert!(png.exists(), "the original is left alone: it is a copy");

        let listed = list_evidence(&plan);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed.iter().find(|i| i.file == "videos/flow.webm").unwrap().step, "Trade");
        let _ = std::fs::remove_dir_all(plan);
    }

    #[test]
    fn a_second_file_with_the_same_name_does_not_overwrite_the_first() {
        let plan = scratch();
        let png = plan.join("page.png");
        std::fs::write(&png, b"one").unwrap();
        let first = add_evidence(&plan, &png, "before", "").unwrap();
        std::fs::write(&png, b"two").unwrap();
        let second = add_evidence(&plan, &png, "after", "").unwrap();
        assert_ne!(first.file, second.file);
        assert_eq!(list_evidence(&plan).len(), 2);
        let _ = std::fs::remove_dir_all(plan);
    }

    #[test]
    fn files_dropped_in_without_the_command_still_show_uncaptioned() {
        let plan = scratch();
        std::fs::create_dir_all(plan.join("Artifacts/screenshots")).unwrap();
        std::fs::write(plan.join("Artifacts/screenshots/home.png"), b"x").unwrap();
        std::fs::write(plan.join("Artifacts/notes.txt"), b"not evidence").unwrap();
        let listed = list_evidence(&plan);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].caption, "");
        assert!(listed[0].at.is_some(), "the modified time stands in");
        let _ = std::fs::remove_dir_all(plan);
    }

    #[test]
    fn only_images_and_videos_of_a_showable_size_are_accepted() {
        let plan = scratch();
        let txt = plan.join("log.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert!(add_evidence(&plan, &txt, "", "").is_err());
        assert!(add_evidence(&plan, &plan.join("missing.png"), "", "").is_err());
        assert_eq!(kind_of(Path::new("A.PNG")), Some(EvidenceKind::Image));
        assert_eq!(kind_of(Path::new("clip.MP4")), Some(EvidenceKind::Video));
        assert_eq!(safe_name("my shot (1).png"), "my-shot--1-.png");
        let _ = std::fs::remove_dir_all(plan);
    }
}
