use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(super) enum MemorySubcommand {
    /// Sync local team memory with another local directory.
    Sync {
        /// Directory to sync with `<project>/.jfc/memory/team`.
        #[arg(long = "dir", value_name = "DIR")]
        dir: PathBuf,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Manage Anthropic SDK memory stores.
    Store {
        #[command(subcommand)]
        sub: MemoryStoreSubcommand,
    },
}

#[derive(Subcommand, Debug)]
pub(super) enum MemoryStoreSubcommand {
    /// List memory stores.
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        json: bool,
    },
    /// Create a memory store.
    Create {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Delete a memory store.
    Delete { store_id: String },
    /// Archive a memory store.
    Archive {
        store_id: String,
        #[arg(long)]
        json: bool,
    },
    /// List memories in a store.
    Memories {
        store_id: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        json: bool,
    },
    /// Create a memory in a store from JSON body.
    CreateMemory {
        store_id: String,
        /// JSON object body accepted by the Anthropic memory API.
        #[arg(long)]
        body: String,
        #[arg(long)]
        view: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Delete a memory from a store.
    DeleteMemory {
        store_id: String,
        memory_id: String,
        /// Expected content SHA256 required by the API.
        #[arg(long)]
        expected_content_sha256: String,
    },
    /// List memory versions for a store.
    Versions {
        store_id: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        json: bool,
    },
}

pub(super) async fn run_memory_subcommand(sub: MemorySubcommand) -> anyhow::Result<()> {
    match sub {
        MemorySubcommand::Sync { dir, json } => {
            let cwd = std::env::current_dir()?;
            let report = jfc_engine::memory::sync_team_memory(&cwd, &dir)
                .map_err(|e| anyhow::anyhow!("memory sync failed: {e}"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("team memory sync");
                println!("local: {}", report.local_dir.display());
                println!("remote: {}", report.remote_dir.display());
                println!("pushed: {}", report.pushed);
                println!("pulled: {}", report.pulled);
                println!("conflicts: {}", report.conflicts.len());
                for conflict in report.conflicts {
                    println!(
                        "- {} -> {}",
                        conflict.file_name,
                        conflict.conflict_path.display()
                    );
                }
            }
            Ok(())
        }
        MemorySubcommand::Store { sub } => run_memory_store_subcommand(sub).await,
    }
}

async fn run_memory_store_subcommand(sub: MemoryStoreSubcommand) -> anyhow::Result<()> {
    let Some(client) = jfc_engine::sdk_bridge::build_client() else {
        anyhow::bail!(
            "memory stores require an Anthropic API key profile; configure ANTHROPIC_API_KEY or jfc auth"
        );
    };
    let service = jfc_anthropic_sdk::memory_stores::MemoryStoreService::new(client);
    match sub {
        MemoryStoreSubcommand::List { limit, json } => {
            let page = service
                .list(&jfc_anthropic_sdk::pagination::ListParams {
                    limit: Some(limit),
                    ..Default::default()
                })
                .await?;
            print_memory_json_or_rows(json, &page, |out| {
                out.push_str("memory stores:\n");
                for store in &page.data {
                    out.push_str(&format!(
                        "- {} name={} description={}\n",
                        store.id,
                        store.name.as_deref().unwrap_or(""),
                        store.description.as_deref().unwrap_or("")
                    ));
                }
            })
        }
        MemoryStoreSubcommand::Create {
            name,
            description,
            json,
        } => {
            let store = service
                .create(&jfc_anthropic_sdk::memory_stores::MemoryStoreCreate {
                    name,
                    description,
                    extra: serde_json::Map::new(),
                })
                .await?;
            print_memory_json_or_rows(json, &store, |out| {
                out.push_str(&format!("created memory store: {}\n", store.id));
            })
        }
        MemoryStoreSubcommand::Delete { store_id } => {
            service.delete(&store_id).await?;
            println!("deleted memory store: {store_id}");
            Ok(())
        }
        MemoryStoreSubcommand::Archive { store_id, json } => {
            let store = service.archive(&store_id).await?;
            print_memory_json_or_rows(json, &store, |out| {
                out.push_str(&format!("archived memory store: {}\n", store.id));
            })
        }
        MemoryStoreSubcommand::Memories {
            store_id,
            limit,
            json,
        } => {
            let page = service
                .list_memories(
                    &store_id,
                    &jfc_anthropic_sdk::pagination::ListParams {
                        limit: Some(limit),
                        ..Default::default()
                    },
                )
                .await?;
            print_memory_json_or_rows(json, &page, |out| {
                out.push_str(&format!("memories in {store_id}:\n"));
                for memory in &page.data {
                    out.push_str(&format!("- {}\n", memory.id));
                }
            })
        }
        MemoryStoreSubcommand::CreateMemory {
            store_id,
            body,
            view,
            json,
        } => {
            let value = serde_json::from_str::<serde_json::Value>(&body)
                .map_err(|e| anyhow::anyhow!("--body must be valid JSON: {e}"))?;
            let Some(map) = value.as_object() else {
                anyhow::bail!("--body must be a JSON object");
            };
            let memory = service
                .create_memory(
                    &store_id,
                    &jfc_anthropic_sdk::memory_stores::MemoryCreate {
                        view,
                        body: map.clone(),
                    },
                )
                .await?;
            print_memory_json_or_rows(json, &memory, |out| {
                out.push_str(&format!("created memory: {}\n", memory.id));
            })
        }
        MemoryStoreSubcommand::DeleteMemory {
            store_id,
            memory_id,
            expected_content_sha256,
        } => {
            service
                .delete_memory(&store_id, &memory_id, &expected_content_sha256)
                .await?;
            println!("deleted memory: {memory_id}");
            Ok(())
        }
        MemoryStoreSubcommand::Versions {
            store_id,
            limit,
            json,
        } => {
            let page = service
                .list_memory_versions(
                    &store_id,
                    &jfc_anthropic_sdk::pagination::ListParams {
                        limit: Some(limit),
                        ..Default::default()
                    },
                )
                .await?;
            print_memory_json_or_rows(json, &page, |out| {
                out.push_str(&format!("memory versions in {store_id}:\n"));
                for version in &page.data {
                    out.push_str(&format!("- {}\n", version.id));
                }
            })
        }
    }
}

fn print_memory_json_or_rows<T: serde::Serialize>(
    json: bool,
    value: &T,
    render_text: impl FnOnce(&mut String),
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        let mut out = String::new();
        render_text(&mut out);
        print!("{out}");
    }
    Ok(())
}
