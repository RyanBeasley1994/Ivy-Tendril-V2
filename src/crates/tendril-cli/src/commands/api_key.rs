//! `tendril api-key`: keys for the public API (`/api/public/v1`), which lets scripts and other apps list
//! projects, read a manager's conversation and progress, and (with `--write`) message a manager. The key
//! is printed once, here, and only its hash is kept; the daemon reads the key file on each request, so
//! creating or revoking one needs no restart.

use clap::Subcommand;
use std::path::Path;
use tendril_server::public_keys::{self, Scope};

#[derive(Subcommand)]
pub enum ApiKeyCommands {
    #[command(about = "Create a key (shown once). Read-only unless --write is given")]
    Create {
        /// A name to tell keys apart, e.g. "ci-bot"
        name: String,
        #[arg(long, help = "Also allow messaging project managers")]
        write: bool,
    },

    #[command(about = "List keys (never the secret itself)")]
    List,

    #[command(about = "Revoke a key by id or name")]
    Revoke { id_or_name: String },
}

pub async fn handle_api_key_command(cmd: ApiKeyCommands, tendril_home: &Path) -> anyhow::Result<()> {
    match cmd {
        ApiKeyCommands::Create { name, write } => {
            let scope = if write { Scope::Write } else { Scope::Read };
            let (record, key) = public_keys::create(tendril_home, &name, scope).map_err(anyhow::Error::msg)?;
            println!("Created {} key '{}' ({}).", if write { "read+message" } else { "read-only" }, record.name, record.id);
            println!();
            println!("  {key}");
            println!();
            println!("Copy it now: it cannot be shown again. Use it as:");
            println!("  curl -H 'Authorization: Bearer {key}' https://<your-forge-url>/api/public/v1/projects");
        }
        ApiKeyCommands::List => {
            let keys = public_keys::list(tendril_home);
            if keys.is_empty() {
                println!("No API keys. Create one with: tendril api-key create <name> [--write]");
            }
            for k in keys {
                let scope = if k.scope.allows_write() { "read+message" } else { "read-only" };
                println!("{}  {:<20} {:<13} {}…  created {}", k.id, k.name, scope, k.prefix, k.created);
            }
        }
        ApiKeyCommands::Revoke { id_or_name } => {
            if public_keys::revoke(tendril_home, &id_or_name).map_err(anyhow::Error::msg)? {
                println!("Revoked '{id_or_name}'. It stops working immediately.");
            } else {
                anyhow::bail!("no key with id or name '{id_or_name}'");
            }
        }
    }
    Ok(())
}
