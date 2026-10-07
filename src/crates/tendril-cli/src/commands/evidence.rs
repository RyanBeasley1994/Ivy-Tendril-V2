//! `tendril evidence`: attach proof that a change works. A worker that has changed anything a person
//! sees (a page, a flow, a screen) runs the real thing, records it, and attaches the result to its plan;
//! the mission page then shows it, and a judge can refuse UI work that comes without any.

use clap::Subcommand;
use std::path::{Path, PathBuf};
use tendril_core::config::{get_config_path, get_plans_dir_with_settings, load_config};
use tendril_core::plans::evidence::{add_evidence, list_evidence, EvidenceKind};
use tendril_core::plans::helpers::resolve_plan_folder;

#[derive(Subcommand)]
pub enum EvidenceCommands {
    #[command(about = "Attach a screenshot (png, jpg, gif, webp) or a short recording (mp4, webm, mov) to a plan as evidence")]
    Add {
        /// The image or video file. It is copied, not moved.
        file: PathBuf,
        #[arg(long, help = "The plan this is evidence for: its id, e.g. 00042")]
        plan: String,
        #[arg(long, default_value = "", help = "What it shows, in a sentence, e.g. \"The order ticket after a market buy\"")]
        caption: String,
        #[arg(long, default_value = "", help = "What you were doing, e.g. \"Logged in as a trader\"")]
        step: String,
    },

    #[command(about = "List a plan's evidence")]
    List {
        #[arg(long, help = "The plan's id")]
        plan: String,
        #[arg(long, help = "Print JSON")]
        json: bool,
    },
}

fn plan_folder(tendril_home: &Path, plan: &str) -> anyhow::Result<PathBuf> {
    let settings = load_config(&get_config_path(tendril_home))?;
    let plans_dir = get_plans_dir_with_settings(tendril_home, Some(&settings));
    resolve_plan_folder(plan, &plans_dir).map_err(|_| anyhow::anyhow!("There is no plan '{plan}'."))
}

pub async fn handle_evidence_command(cmd: EvidenceCommands, tendril_home: &Path) -> anyhow::Result<()> {
    match cmd {
        EvidenceCommands::Add { file, plan, caption, step } => {
            let folder = plan_folder(tendril_home, &plan)?;
            let item = add_evidence(&folder, &file, &caption, &step)?;
            let what = if item.kind == EvidenceKind::Image { "screenshot" } else { "video" };
            println!("Attached the {what} as {} on plan {plan}.", item.file);
        }
        EvidenceCommands::List { plan, json } => {
            let items = list_evidence(&plan_folder(tendril_home, &plan)?);
            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else if items.is_empty() {
                println!("Plan {plan} has no evidence yet.");
            } else {
                for i in &items {
                    let kind = if i.kind == EvidenceKind::Image { "image" } else { "video" };
                    println!("{kind:<6} {}  {}", i.file, if i.caption.is_empty() { "(no caption)" } else { &i.caption });
                }
            }
        }
    }
    Ok(())
}
