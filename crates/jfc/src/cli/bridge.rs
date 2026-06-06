use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Subcommand;
use uuid::Uuid;

#[derive(Subcommand, Debug)]
pub(super) enum BridgeSubcommand {
    /// Run the self-hosted JFC worker bridge REST/SSE server.
    Serve {
        /// Address to bind, for example `127.0.0.1:8787` or `0.0.0.0:8787`.
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: SocketAddr,
        /// Public base URL workers should use in bootstrap responses.
        #[arg(long = "public-base-url")]
        public_base_url: Option<String>,
        /// HMAC/JWT signing secret. Defaults to JFC_BRIDGE_SECRET or an ephemeral secret.
        #[arg(long)]
        secret: Option<String>,
        /// Bootstrap bearer token for POST /bridge and POST /sessions.
        /// Defaults to JFC_BRIDGE_BOOTSTRAP_TOKEN or a generated token.
        #[arg(long = "bootstrap-token")]
        bootstrap_token: Option<String>,
        /// Allow unauthenticated bootstrap/session creation. Not recommended.
        #[arg(long = "no-bootstrap-auth")]
        no_bootstrap_auth: bool,
        /// Worker token lifetime in seconds.
        #[arg(long = "token-ttl-secs", default_value_t = 12 * 60 * 60)]
        token_ttl_secs: u64,
        /// Persist bridge sessions/workers/events to this JSON file.
        #[arg(long = "state-file")]
        state_file: Option<PathBuf>,
    },
}

pub(super) async fn run_bridge_subcommand(sub: BridgeSubcommand) -> anyhow::Result<()> {
    match sub {
        BridgeSubcommand::Serve {
            bind,
            public_base_url,
            secret,
            bootstrap_token,
            no_bootstrap_auth,
            token_ttl_secs,
            state_file,
        } => {
            let api_base_url =
                public_base_url.unwrap_or_else(|| format!("http://{}", normalize_bind_url(bind)));
            let secret = secret
                .or_else(|| std::env::var("JFC_BRIDGE_SECRET").ok())
                .unwrap_or_else(|| format!("secret_{}", Uuid::new_v4().simple()));
            let bootstrap_token = if no_bootstrap_auth {
                None
            } else {
                Some(
                    bootstrap_token
                        .or_else(|| std::env::var("JFC_BRIDGE_BOOTSTRAP_TOKEN").ok())
                        .unwrap_or_else(|| format!("boot_{}", Uuid::new_v4().simple())),
                )
            };

            println!("jfc bridge listening on http://{bind}");
            println!("bridge api_base_url: {api_base_url}");
            match bootstrap_token.as_deref() {
                Some(token) => println!("bootstrap bearer token: {token}"),
                None => println!("bootstrap bearer token: disabled"),
            }
            if std::env::var("JFC_BRIDGE_SECRET").is_err() && secret.starts_with("secret_") {
                println!("worker token secret: ephemeral for this process");
            }

            let mut config = jfc_bridge::BridgeConfig::new(api_base_url, secret.into_bytes());
            config.bootstrap_token = bootstrap_token;
            config.token_ttl = Duration::from_secs(token_ttl_secs);
            let state_file = state_file.or_else(|| {
                std::env::var("JFC_BRIDGE_STATE_FILE")
                    .ok()
                    .map(PathBuf::from)
            });
            let store: Arc<dyn jfc_bridge::BridgeStore> = match state_file {
                Some(path) => {
                    println!("bridge state file: {}", path.display());
                    Arc::new(jfc_bridge::MemoryBridgeStore::with_state_file(path)?)
                }
                None => Arc::new(jfc_bridge::MemoryBridgeStore::new()),
            };
            jfc_bridge::serve(bind, jfc_bridge::BridgeState::new(store, config)).await?;
            Ok(())
        }
    }
}

fn normalize_bind_url(bind: SocketAddr) -> String {
    if bind.ip().is_unspecified() {
        format!("127.0.0.1:{}", bind.port())
    } else {
        bind.to_string()
    }
}
