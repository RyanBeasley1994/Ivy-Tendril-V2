//! `tendril evidence`: runs the real binary, because what a worker sees printed is part of the contract.

use std::path::PathBuf;
use tendril_core::models::PlanVerificationEntry;
use tendril_core::plans::{create_plan, CreatePlanOptions};

struct Home {
    path: PathBuf,
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tendril-cli-evidence-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(path.join("Plans")).unwrap();
        Self { path }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_tendril"))
            .arg("--home")
            .arg(&self.path)
            .env("TENDRIL_PLANS", self.path.join("Plans"))
            .args(args)
            .output()
            .expect("run tendril")
    }
}

fn plan(home: &Home) -> String {
    let verifications: Vec<PlanVerificationEntry> = vec![];
    create_plan(
        &home.path.join("Plans"),
        CreatePlanOptions {
            title: "Evidence plan".into(),
            project: "p".into(),
            level: Some("Feature".into()),
            initial_prompt: None,
            source_url: None,
            execution_profile: None,
            priority: Some(0),
            repos: vec![],
            verifications,
            depends_on: vec![],
            related_plans: vec![],
            chat_session_id: None,
        },
    )
    .unwrap()
    .id()
    .to_string()
}

#[test]
fn a_worker_attaches_a_screenshot_and_a_recording_and_lists_them() {
    let home = Home::new();
    let id = plan(&home);
    let png = home.path.join("trade.png");
    let webm = home.path.join("flow.webm");
    std::fs::write(&png, b"png").unwrap();
    std::fs::write(&webm, b"webm").unwrap();

    for (file, caption) in [(&png, "The order ticket after a market buy"), (&webm, "Login to first trade")] {
        let out = home.run(&["evidence", "add", file.to_str().unwrap(), "--plan", &id, "--caption", caption, "--step", "Trade"]);
        assert!(out.status.success(), "add failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    let out = home.run(&["evidence", "list", "--plan", &id, "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let items: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), 2);
    let video = items.iter().find(|i| i["kind"] == "video").expect("a video");
    assert_eq!(video["caption"], "Login to first trade");
    assert!(video["path"].as_str().unwrap().ends_with("Artifacts/videos/flow.webm"));
    let image = items.iter().find(|i| i["kind"] == "image").expect("an image");
    assert_eq!(image["file"], "screenshots/trade.png");
}

#[test]
fn bad_input_is_refused_with_a_reason_a_worker_can_act_on() {
    let home = Home::new();
    let id = plan(&home);
    let log = home.path.join("run.log");
    std::fs::write(&log, b"x").unwrap();

    let not_media = home.run(&["evidence", "add", log.to_str().unwrap(), "--plan", &id]);
    assert!(!not_media.status.success());
    assert!(String::from_utf8_lossy(&not_media.stderr).contains("not an image"));

    let no_plan = home.run(&["evidence", "list", "--plan", "99999"]);
    assert!(!no_plan.status.success());
    assert!(String::from_utf8_lossy(&no_plan.stderr).contains("no plan"));
}
