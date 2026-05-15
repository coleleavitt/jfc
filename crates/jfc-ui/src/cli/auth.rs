use clap::Subcommand;
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub(super) enum AuthSubcommand {
    /// Anthropic-specific account commands.
    Anthropic {
        #[command(subcommand)]
        sub: AnthropicAuthSubcommand,
    },
    /// OpenAI Codex / ChatGPT OAuth commands.
    Codex {
        #[command(subcommand)]
        sub: CodexAuthSubcommand,
    },
    /// LiteLLM proxy instance credentials.
    Litellm {
        #[command(subcommand)]
        sub: LiteLLMAuthSubcommand,
    },
    /// OpenWebUI account commands (Shibboleth + Duo OIDC, manual JWT, etc.).
    Openwebui {
        #[command(subcommand)]
        sub: OpenWebUIAuthSubcommand,
    },
}

#[derive(Subcommand, Debug)]
pub(super) enum OpenWebUIAuthSubcommand {
    /// Automated OIDC login (Shibboleth + Duo 2FA). Requires OWUI_USERNAME +
    /// OWUI_PASSWORD env vars; OWUI_DUO_PASSCODE is optional (uses push if unset).
    Login {
        /// OpenWebUI base URL (default: $OWUI_BASE_URL or https://chat.ai2s.org).
        base_url: Option<String>,
    },
    /// Add an account by manually pasting a JWT.
    Add {
        /// OpenWebUI base URL.
        base_url: String,
        /// JWT cookie value (3-segment).
        token: String,
    },
    /// List configured accounts.
    List,
    /// Switch to a different account.
    Use {
        /// Account name (e.g. user@example.com@chat.example.com).
        name: String,
    },
    /// Remove an account.
    Remove {
        /// Account name.
        name: String,
    },
    /// List models accessible to the active account.
    Models,
    /// Verify the active account's token + show user identity.
    Whoami,
    /// Show OpenWebUI instance config (name, version, features).
    Config,
}

#[derive(Subcommand, Debug)]
pub(super) enum AnthropicAuthSubcommand {
    /// Add a new account via the PKCE OAuth flow. Opens a browser-pasteable
    /// URL. By default jfc waits for a localhost callback; `--manual` falls
    /// back to the older paste-the-`code#state` flow.
    Login {
        /// Optional local alias. If omitted, jfc derives the canonical
        /// account identity from the OAuth profile automatically.
        name: Option<String>,
        /// Use the manual callback-page paste flow instead of the localhost callback flow.
        #[arg(long)]
        manual: bool,
    },
    /// List configured accounts with tier, runtime status, and active marker.
    List,
    /// Print the active account's name (the one that would be picked first).
    Active,
    /// Switch which account is preferred for the next request. Rotation may
    /// still bypass this if the picked account is rate-limited.
    Switch {
        /// Account name to mark active.
        name: String,
    },
    /// Disable an account so the rotation manager skips it permanently
    /// until re-enabled (e.g., after re-login).
    Disable {
        /// Account name to disable.
        name: String,
        /// Optional reason recorded in the store.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Remove an account entirely from the store. The refresh token on disk
    /// is wiped before deletion.
    Remove {
        /// Account name to remove.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub(super) enum CodexAuthSubcommand {
    /// Print the browser URL for ChatGPT/Codex OAuth login.
    Login,
    /// Start a device-code login and print the one-time code.
    Device,
    /// Show configured Codex OAuth token status.
    Status,
    /// Remove stored Codex OAuth tokens.
    Logout,
}

#[derive(Subcommand, Debug)]
pub(super) enum LiteLLMAuthSubcommand {
    /// Configure a LiteLLM proxy instance (API key + base URL).
    Login {
        /// Base URL of the LiteLLM proxy (e.g. https://api.example.com/v1).
        #[arg(long)]
        url: String,
        /// API key for authentication.
        #[arg(long)]
        key: String,
    },
    /// Show configured LiteLLM credentials.
    Status,
    /// Remove stored LiteLLM credentials.
    Logout,
}

pub(super) async fn run_auth_subcommand(sub: AuthSubcommand) -> anyhow::Result<()> {
    match sub {
        AuthSubcommand::Anthropic { sub } => run_anthropic_auth_subcommand(sub).await,
        AuthSubcommand::Codex { sub } => run_codex_auth_subcommand(sub).await,
        AuthSubcommand::Litellm { sub } => run_litellm_auth_subcommand(sub).await,
        AuthSubcommand::Openwebui { sub } => run_openwebui_auth_subcommand(sub).await,
    }
}

async fn run_openwebui_auth_subcommand(sub: OpenWebUIAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::openwebui::{
        Account, DuoMethod, OidcLoginOptions, default_store_path, fetch_instance_config,
        get_current, list_accounts, load_store, normalize_base_url, oidc_login, parse_jwt_claims,
        remove_account, set_current, upsert_account, verify_token,
    };

    let store_path = default_store_path();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let default_base =
        std::env::var("OWUI_BASE_URL").unwrap_or_else(|_| "https://chat.ai2s.org".to_owned());

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    match sub {
        OpenWebUIAuthSubcommand::Login { base_url } => {
            let base = normalize_base_url(&base_url.unwrap_or(default_base))?;
            let username = std::env::var("OWUI_USERNAME")
                .map_err(|_| anyhow::anyhow!("OWUI_USERNAME env var required"))?;
            let password = std::env::var("OWUI_PASSWORD")
                .map_err(|_| anyhow::anyhow!("OWUI_PASSWORD env var required"))?;
            let passcode = std::env::var("OWUI_DUO_PASSCODE").ok();
            let method = if passcode.is_some() {
                DuoMethod::Passcode
            } else {
                DuoMethod::Push
            };

            println!("→ logging in to {base} as {username}...");
            if matches!(method, DuoMethod::Push) {
                println!("→ no OWUI_DUO_PASSCODE set — sending Duo Push (approve on your phone)");
            } else {
                println!("→ using OWUI_DUO_PASSCODE for 2FA");
            }

            let mut opts = OidcLoginOptions::new(&base, &username, &password);
            opts.duo_passcode = passcode;
            opts.duo_method = method;
            let result = oidc_login(opts).await?;

            let user = verify_token(&client, &base, &result.token).await?;
            let cfg = fetch_instance_config(&client, &base).await.ok();
            let host = url::Url::parse(&base)?.host_str().unwrap_or("").to_owned();
            let name = format!("{}@{}", user.email, host);
            let now = now_ms();

            upsert_account(
                &store_path,
                Account {
                    name: name.clone(),
                    base_url: base.clone(),
                    token: result.token,
                    expires_at: Some(result.expires_at),
                    created_at: Some(now),
                    updated_at: Some(now),
                    ..Default::default()
                },
            )?;
            let _ = set_current(&store_path, &name);

            println!(
                "\n✓ logged in as {} <{}> ({})",
                user.name, user.email, user.role
            );
            if let Some(c) = cfg {
                println!("  instance: {} v{}", c.name, c.version);
            }
            let exp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(result.expires_at)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "?".into());
            println!("  token expires: {exp}");
            println!("  account stored as: {name}");
        }
        OpenWebUIAuthSubcommand::Add { base_url, token } => {
            let base = normalize_base_url(&base_url)?;
            let claims = parse_jwt_claims(&token)
                .ok_or_else(|| anyhow::anyhow!("token does not decode as a JWT"))?;
            let user = verify_token(&client, &base, &token).await?;
            let cfg = fetch_instance_config(&client, &base).await.ok();
            let host = url::Url::parse(&base)?.host_str().unwrap_or("").to_owned();
            let name = format!("{}@{}", user.email, host);
            let now = now_ms();
            upsert_account(
                &store_path,
                Account {
                    name: name.clone(),
                    base_url: base,
                    token,
                    expires_at: Some(claims.exp * 1000),
                    created_at: Some(now),
                    updated_at: Some(now),
                    ..Default::default()
                },
            )?;
            let exp = chrono::DateTime::<chrono::Utc>::from_timestamp(claims.exp, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "?".into());
            println!(
                "✓ added {name} (instance={} v{}, expires={exp})",
                cfg.as_ref().map(|c| c.name.as_str()).unwrap_or("unknown"),
                cfg.as_ref().map(|c| c.version.as_str()).unwrap_or("?")
            );
        }
        OpenWebUIAuthSubcommand::List => {
            let store = load_store(&store_path);
            let current = store.current.clone();
            let accounts = list_accounts(&store);
            if accounts.is_empty() {
                println!("(no accounts)");
            } else {
                for a in accounts {
                    let star = if Some(&a.name) == current.as_ref() {
                        "*"
                    } else {
                        " "
                    };
                    let exp = a
                        .expires_at
                        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_else(|| "?".into());
                    println!("{star} {:48}  {}  expires={exp}", a.name, a.base_url);
                }
            }
        }
        OpenWebUIAuthSubcommand::Use { name } => {
            if !set_current(&store_path, &name)? {
                anyhow::bail!("no account named {name}");
            }
            println!("current → {name}");
        }
        OpenWebUIAuthSubcommand::Remove { name } => {
            remove_account(&store_path, &name)?;
            println!("removed {name}");
        }
        OpenWebUIAuthSubcommand::Models => {
            let store = load_store(&store_path);
            let account =
                get_current(&store).ok_or_else(|| anyhow::anyhow!("no current account"))?;
            let res: serde_json::Value = client
                .get(format!(
                    "{}/api/models",
                    account.base_url.trim_end_matches('/')
                ))
                .header("Authorization", format!("Bearer {}", account.token))
                .header("Accept", "application/json")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if let Some(arr) = res.get("data").and_then(|v| v.as_array()) {
                for m in arr {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    println!("{id:48}  {name}");
                }
                println!("\n{} model(s) accessible to {}", arr.len(), account.name);
            }
        }
        OpenWebUIAuthSubcommand::Whoami => {
            let store = load_store(&store_path);
            let account =
                get_current(&store).ok_or_else(|| anyhow::anyhow!("no current account"))?;
            let user = verify_token(&client, &account.base_url, &account.token).await?;
            println!(
                "{}\n  {} {} <{}>",
                account.name, user.role, user.id, user.email
            );
        }
        OpenWebUIAuthSubcommand::Config => {
            let store = load_store(&store_path);
            let base = get_current(&store)
                .map(|a| a.base_url)
                .unwrap_or(default_base);
            let cfg = fetch_instance_config(&client, &base).await?;
            println!("instance:  {} v{}", cfg.name, cfg.version);
            println!("baseUrl:   {base}");
            println!(
                "status:    {}",
                if cfg.status { "online" } else { "offline" }
            );
            let enabled: Vec<&String> = cfg
                .features
                .iter()
                .filter(|(_, v)| v.as_bool().unwrap_or(false))
                .map(|(k, _)| k)
                .collect();
            if enabled.is_empty() {
                println!("features:  (none enabled)");
            } else {
                let mut joined: Vec<&str> = enabled.iter().map(|s| s.as_str()).collect();
                joined.sort();
                println!("features:  {}", joined.join(", "));
            }
        }
    }
    Ok(())
}

async fn run_codex_auth_subcommand(sub: CodexAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::codex_oauth::CodexOAuthProvider;
    use crate::providers::oauth_core::TokenStore;

    let provider = CodexOAuthProvider::new();
    match sub {
        CodexAuthSubcommand::Login => {
            let redirect_uri = "http://localhost:1455/auth/callback";
            let req = CodexOAuthProvider::authorize_url(redirect_uri);
            println!();
            println!("=== OpenAI Codex OAuth login ===");
            println!();
            println!("Open this URL in a browser:");
            println!();
            println!("   {}", req.url);
            println!();
            println!(
                "After approving, capture the callback code and exchange it through the Codex OAuth flow."
            );
            println!("Device-code flow is also available with: jfc auth codex device");
            println!("store: {}", provider.store_path().display());
            Ok(())
        }
        CodexAuthSubcommand::Device => {
            let code = provider.request_device_code().await?;
            println!();
            println!("=== OpenAI Codex device login ===");
            println!();
            println!("Open: {}", code.verification_url);
            println!("Code: {}", code.user_code);
            println!();
            println!("Waiting for authorization...");
            provider.poll_device_code(&code).await?;
            println!(
                "✓ Codex OAuth tokens stored at {}",
                provider.store_path().display()
            );
            Ok(())
        }
        CodexAuthSubcommand::Status => {
            let store = TokenStore::new(TokenStore::default_path());
            match store.get("codex")? {
                Some(crate::providers::oauth_core::AuthMethod::OAuth {
                    expires_at,
                    account_id,
                    ..
                }) => {
                    println!("codex: configured");
                    println!("account: {}", account_id.as_deref().unwrap_or("(unknown)"));
                    println!("expires_at: {expires_at}");
                }
                _ => println!(
                    "codex: not configured (run `jfc auth codex login` or `jfc auth codex device`)"
                ),
            }
            Ok(())
        }
        CodexAuthSubcommand::Logout => {
            let store = TokenStore::new(TokenStore::default_path());
            if store.remove("codex")? {
                println!("removed Codex OAuth tokens from {}", store.path().display());
            } else {
                println!("no Codex OAuth tokens found in {}", store.path().display());
            }
            Ok(())
        }
    }
}

async fn run_litellm_auth_subcommand(sub: LiteLLMAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::litellm;

    let cred_path = litellm::credentials_path();
    match sub {
        LiteLLMAuthSubcommand::Login { url, key } => {
            litellm::save_credentials(&url, &key)?;
            println!("✓ LiteLLM credentials saved to {}", cred_path.display());

            let client = reqwest::Client::new();
            let base = url.trim_end_matches('/');
            match client
                .get(format!("{base}/models"))
                .header("Authorization", format!("Bearer {key}"))
                .timeout(std::time::Duration::from_secs(8))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    println!("✓ Connection verified — instance is reachable");
                }
                Ok(resp) => {
                    println!(
                        "⚠ Instance returned HTTP {} — credentials may be invalid",
                        resp.status()
                    );
                }
                Err(e) => {
                    println!("⚠ Could not reach instance: {e}");
                    println!("  Credentials are saved; fix the URL and re-run login.");
                }
            }
            Ok(())
        }
        LiteLLMAuthSubcommand::Status => {
            match litellm::load_credentials() {
                Some(creds) => {
                    println!("litellm: configured");
                    println!("url: {}", creds.base_url);
                    println!(
                        "key: {}…{}",
                        &creds.api_key[..creds.api_key.len().min(4)],
                        &creds.api_key[creds.api_key.len().saturating_sub(4)..]
                    );
                    println!("store: {}", cred_path.display());
                }
                None => {
                    println!("litellm: not configured");
                    println!("  run: jfc auth litellm login --url <URL> --key <KEY>");
                }
            }
            Ok(())
        }
        LiteLLMAuthSubcommand::Logout => {
            if cred_path.exists() {
                std::fs::remove_file(&cred_path)?;
                println!("removed LiteLLM credentials from {}", cred_path.display());
            } else {
                println!("no LiteLLM credentials found at {}", cred_path.display());
            }
            Ok(())
        }
    }
}

async fn run_anthropic_auth_subcommand(sub: AnthropicAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::anthropic_accounts::AccountManager;
    use crate::providers::anthropic_oauth::default_store_path;
    use crate::providers::anthropic_oauth_login as login;

    let store_path = default_store_path();
    let mgr = AccountManager::load(store_path.clone()).await?;

    match sub {
        AnthropicAuthSubcommand::Login { name, manual } => {
            let requested_name = name.as_deref().unwrap_or("");
            if manual {
                let req = login::authorize();
                println!();
                println!("=== Anthropic OAuth login ===");
                println!();
                println!("1. Open this URL in a browser:");
                println!();
                println!("   {}", req.url);
                println!();
                println!("2. After approving, the callback page will show a string like:");
                println!("      <code>#<state>");
                println!("3. Paste the entire string (with the `#`) here, then press Enter.");
                println!();
                print!("code#state> ");
                use std::io::Write;
                std::io::stdout().flush().ok();

                let mut paste = String::new();
                std::io::stdin().read_line(&mut paste)?;
                let paste = paste.trim();
                if paste.is_empty() {
                    anyhow::bail!("login: no input provided");
                }

                match login::login(&mgr, requested_name, paste, &req.verifier, &req.state).await {
                    Ok(resolved_name) => {
                        println!("\n✓ logged in as '{resolved_name}'.");
                        println!("  store: {}", store_path.display());
                        Ok(())
                    }
                    Err(e) => Err(anyhow::anyhow!("login failed: {e}")),
                }
            } else {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
                let port = listener.local_addr()?.port();
                let req = login::authorize_with_redirect(
                    crate::providers::anthropic_oauth_login::RedirectTarget::Localhost(port),
                );
                println!();
                println!("=== Anthropic OAuth login ===");
                println!();
                println!("Open this URL in a browser:");
                println!();
                println!("   {}", req.url);
                println!();
                println!("Waiting for callback on http://localhost:{port}/callback ...");
                println!("If that fails, rerun with: jfc auth anthropic login --manual");

                let (code, returned_state) = wait_for_oauth_callback(listener).await?;
                match login::login_with_code_and_state(
                    &mgr,
                    requested_name,
                    &code,
                    &returned_state,
                    &req.verifier,
                    &req.state,
                    &req.redirect_uri,
                )
                .await
                {
                    Ok(resolved_name) => {
                        println!("\n✓ logged in as '{resolved_name}'.");
                        println!("  store: {}", store_path.display());
                        Ok(())
                    }
                    Err(e) => Err(anyhow::anyhow!("login failed: {e}")),
                }
            }
        }
        AnthropicAuthSubcommand::List => {
            let pairs = mgr.list_with_runtime().await;
            if pairs.is_empty() {
                println!("(no accounts in {})", store_path.display());
                println!("Run `jfc auth anthropic login <name>` to add one.");
                return Ok(());
            }
            let active_name = mgr.active_account().await.map(|a| a.name);
            println!(
                "{:<20} {:<8} {:<22} {:<10} {:<14}",
                "NAME", "ACTIVE", "TIER", "ENABLED", "RUNTIME"
            );
            for (acct, rt) in pairs {
                let is_active = active_name.as_deref() == Some(acct.name.as_str());
                let active_marker = if is_active { "*" } else { "" };
                let tier = acct.rate_limit_tier.as_deref().unwrap_or("(unknown)");
                let enabled = if acct.is_enabled() { "yes" } else { "no" };
                let runtime = format_runtime_state(&acct, &rt);
                println!(
                    "{:<20} {:<8} {:<22} {:<10} {:<14}",
                    acct.name, active_marker, tier, enabled, runtime
                );
            }
            Ok(())
        }
        AnthropicAuthSubcommand::Active => match mgr.active_account().await {
            Some(a) => {
                println!("{}", a.name);
                Ok(())
            }
            None => {
                eprintln!("(no active account)");
                std::process::exit(1);
            }
        },
        AnthropicAuthSubcommand::Switch { name } => {
            if mgr.atomic_set_active(&name).await? {
                println!("active = {name}");
                Ok(())
            } else {
                Err(anyhow::anyhow!("switch: account '{name}' not found"))
            }
        }
        AnthropicAuthSubcommand::Disable { name, reason } => {
            mgr.atomic_disable_account(&name, reason.as_deref().unwrap_or("manual"))
                .await?;
            println!("disabled '{name}'");
            Ok(())
        }
        AnthropicAuthSubcommand::Remove { name } => {
            mgr.atomic_clear_refresh_token(&name).await.ok();
            if mgr.atomic_remove_account(&name).await? {
                println!("removed '{name}'");
                Ok(())
            } else {
                Err(anyhow::anyhow!("remove: account '{name}' not found"))
            }
        }
    }
}

async fn wait_for_oauth_callback(
    listener: tokio::net::TcpListener,
) -> anyhow::Result<(String, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for OAuth callback"))??;

    let mut buf = vec![0u8; 8192];
    let n = socket.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("malformed callback request"))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("malformed callback request line"))?;
    let url = reqwest::Url::parse(&format!("http://localhost{path}"))?;
    let code = url
        .query_pairs()
        .find_map(|(k, v)| (k == "code").then(|| v.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("callback missing code"))?;
    let state = url
        .query_pairs()
        .find_map(|(k, v)| (k == "state").then(|| v.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("callback missing state"))?;

    let body =
        "<html><body><h1>Anthropic login complete</h1><p>You can return to jfc.</p></body></html>";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(resp.as_bytes()).await.ok();
    socket.shutdown().await.ok();

    Ok((code, state))
}

fn format_runtime_state(
    acct: &crate::providers::anthropic_accounts::Account,
    rt: &crate::providers::anthropic_accounts::RuntimeState,
) -> String {
    if !acct.is_disk_rate_limit_cleared() {
        return "rate-limited".into();
    }
    if !rt.cooldown_cleared() {
        return "cooldown".into();
    }
    if rt.consecutive_failures > 0 {
        return format!("fails={}", rt.consecutive_failures);
    }
    if acct.is_token_expired() {
        return "token-expired".into();
    }
    "ok".into()
}
