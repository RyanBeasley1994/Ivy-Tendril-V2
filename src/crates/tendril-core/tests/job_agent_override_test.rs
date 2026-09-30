//! `StartOptions::agent`/`model`/`effort`: a job started with them is launched on that harness with
//! that model and effort, instead of the configured `codingAgent` and its profile. This is what lets a
//! mission run its planner, workers, judge and validator on different agents.

mod common;

use common::{plan_with, HomeFixture};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tendril_core::agents::providers::AgentProcessSpec;
use tendril_core::config::TendrilSettings;
use tendril_core::jobs::manager::{JobManager, StartOptions};
use tendril_core::models::{ExecutePlanArgs, JobArgs, JobStatus, PlanStatus, VerificationStatus};

#[cfg(unix)]
async fn launch(opts: StartOptions) -> (String, Option<String>, Option<String>, String) {
    let home = HomeFixture::new("job-agent-override");
    home.write_promptware("ExecutePlan");
    let plan = plan_with(PlanStatus::Draft, &[("Build", VerificationStatus::Pass)]);
    let folder = home.write_plan("00001-Override", &plan);
    let script = home.path.join("agent.sh");
    std::fs::write(&script, "exit 0\n").unwrap();

    let seen: Arc<Mutex<Option<(String, Option<String>, Option<String>)>>> = Arc::default();
    let capture = seen.clone();
    let workdir = home.path.clone();
    let settings = TendrilSettings { coding_agent: "claude".into(), ..Default::default() };
    let manager = JobManager::new(home.path.clone(), settings)
        .with_spec_builder(Arc::new(move |provider, config| {
            *capture.lock().unwrap() =
                Some((provider.to_string(), config.model.clone(), config.effort.clone()));
            AgentProcessSpec {
                command: "/bin/sh".into(),
                args: vec![script.to_string_lossy().to_string()],
                environment: Default::default(),
                working_directory: workdir.clone(),
                stdin_content: None,
                redirect_stdin: false,
                temp_files: vec![],
            }
        }))
        .share();

    let id = manager
        .start_job_with(
            JobArgs::ExecutePlan(ExecutePlanArgs { folder_path: folder.to_string_lossy().to_string(), note: None }),
            opts,
        )
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let job = manager.get_job(&id).await.unwrap().unwrap();
        if !matches!(job.status, JobStatus::Pending | JobStatus::Queued | JobStatus::Running) {
            let (provider, model, effort) = seen.lock().unwrap().clone().expect("the agent was launched");
            return (provider, model, effort, job.provider);
        }
        assert!(tokio::time::Instant::now() < deadline, "the job never finished");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn an_agent_override_picks_the_harness_model_and_effort() {
    let (provider, model, effort, recorded) = launch(StartOptions {
        agent: Some("Codex".into()),
        model: Some("gpt-5.6-sol".into()),
        effort: Some("high".into()),
        ..Default::default()
    })
    .await;
    assert_eq!(provider, "codex");
    assert_eq!(recorded, "codex");
    assert_eq!(model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(effort.as_deref(), Some("high"));
}

#[cfg(unix)]
#[tokio::test]
async fn no_override_keeps_the_configured_agent() {
    let (provider, _, _, recorded) = launch(StartOptions::default()).await;
    assert_eq!(provider, "claude");
    assert_eq!(recorded, "claude");
}
