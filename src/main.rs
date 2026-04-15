//! Dolphin Milk: Autonomous AI agent with BSV micropayments.

use clap::Parser;
use std::path::PathBuf;

use dolphin_milk::audit::revelation::{Revelation, RevelationLog};
use dolphin_milk::auth::AuthriteClient;
use dolphin_milk::banner;
use dolphin_milk::cli::{AuditAction, Cli, Command};
use dolphin_milk::config::load_config;
use dolphin_milk::error::DmError;
use dolphin_milk::server;
use dolphin_milk::wallet::WalletClient;
use dolphin_milk::{logging, runner, think};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run {
            task,
            workspace,
            max_iterations,
        }) => {
            std::process::exit(cmd_run(task, workspace, max_iterations).await);
        }
        Some(Command::Status) => {
            std::process::exit(cmd_status().await);
        }
        Some(Command::Think { message, model }) => {
            std::process::exit(cmd_think(&message, &model).await);
        }
        Some(Command::Receive { suffix }) => {
            std::process::exit(cmd_receive(&suffix).await);
        }
        Some(Command::Fund { txid, vout, suffix }) => {
            std::process::exit(cmd_fund(&txid, vout, &suffix).await);
        }
        Some(Command::Init { data_dir }) => {
            std::process::exit(cmd_init(data_dir).await);
        }
        Some(Command::Split { count }) => {
            std::process::exit(cmd_split(count).await);
        }
        Some(Command::Start { port, workspace }) => {
            std::process::exit(cmd_start(port, workspace).await);
        }
        Some(Command::Serve { port, workspace }) => {
            std::process::exit(cmd_serve(port, workspace).await);
        }
        Some(Command::Mcp) => {
            std::process::exit(cmd_mcp().await);
        }
        Some(Command::Audit(args)) => {
            std::process::exit(cmd_audit(args.action).await);
        }
        Some(Command::VerifyWork { proof_file }) => {
            std::process::exit(cmd_verify_work(&proof_file));
        }
        None => {
            cmd_no_args().await;
        }
    }
}

async fn cmd_no_args() {
    // Try to load config and check wallet — show contextual guidance
    let cfg = load_config(None).ok();
    if let Some(ref cfg) = cfg {
        let wallet = WalletClient::from_config(&cfg.wallet);
        // Use a short timeout so no-args doesn't hang
        match tokio::time::timeout(std::time::Duration::from_secs(2), check_wallet(&wallet)).await {
            Ok(Ok((identity_key, balance))) => {
                banner::print_status_guide(&identity_key, balance);
                return;
            }
            _ => {
                // Wallet not reachable — guide them
                println!();
                println!("  Dolphin Milk v{VERSION}");
                println!();
                println!("  Wallet not reachable at {}", cfg.wallet.url);
                println!();
                println!("  Start your wallet:");
                println!("    bsv-wallet daemon");
                println!();
                println!("  Then try: dolphin-milk status");
                println!();
                return;
            }
        }
    }

    // No config found — first time user
    banner::print_getting_started();
}

async fn cmd_init(data_dir: Option<String>) -> i32 {
    println!();
    println!("  Dolphin Milk v{VERSION} \u{2014} Setup");
    println!();

    // Step 1: Resolve data directory
    let data_dir = data_dir.map(PathBuf::from).unwrap_or_else(|| {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".dolphin-milk")
    });

    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("  ERROR: Cannot create data directory: {e}");
        return 1;
    }
    println!("  Data directory: {}", data_dir.display());

    // Step 2: Config file
    let config_path = PathBuf::from("dolphin-milk.toml");
    let data_config = data_dir.join("dolphin-milk.toml");
    let config_exists = config_path.exists() || data_config.exists();

    if config_exists {
        let which = if config_path.exists() {
            config_path.display().to_string()
        } else {
            data_config.display().to_string()
        };
        println!("  Config:         {which} (exists, keeping)");
    } else {
        let minimal = format!(
            r#"# Dolphin Milk configuration
# Full reference: dolphin-milk.toml.example

[wallet]
url = "http://localhost:3322"
db_path = "{db_path}"

[llm]
# default_model = "gpt-5-mini"

[logging]
level = "INFO"
format = "text"
"#,
            db_path = data_dir.join("wallet.db").display()
        );
        if let Err(e) = std::fs::write(&config_path, &minimal) {
            eprintln!("  WARNING: Could not write config: {e}");
        } else {
            println!("  Config:         {} (created)", config_path.display());
        }
    }

    // Step 3: Workspace directory
    let workspace = data_dir.join("workspace");
    let _ = std::fs::create_dir_all(&workspace);
    println!("  Workspace:      {}", workspace.display());

    // Step 4: Wallet — create or verify
    let db_path = data_dir.join("wallet.db");
    let env_path = data_dir.join(".env");

    if db_path.exists() && env_path.exists() {
        // Wallet already exists — just check connectivity
        println!("  Wallet DB:      {} (exists)", db_path.display());
        println!();
        println!("  Checking wallet...");

        let cfg = load_config(None).unwrap_or_default();
        let wallet = WalletClient::from_config(&cfg.wallet);
        match check_wallet(&wallet).await {
            Ok((identity_key, balance)) => {
                println!("    Identity: {identity_key}");
                if let Some(b) = balance {
                    println!("    Balance:  {} sats", banner::format_sats_human(b));
                    if b == 0 {
                        print_funding_instructions(&wallet).await;
                    } else {
                        println!();
                        println!("  You're funded and ready to go!");
                    }
                }
            }
            Err(_) => {
                println!("    Wallet DB exists but not serving.");
                println!("    `dolphin-milk start` will launch it automatically.");
            }
        }
    } else {
        // Create wallet using embedded wallet
        println!();
        println!("  Creating wallet...");

        #[cfg(feature = "embedded-wallet")]
        {
            use rand::RngCore;
            let mut key_bytes = [0u8; 32];
            rand::rng().fill_bytes(&mut key_bytes);
            let root_key_hex = hex::encode(key_bytes);

            // Save root key to .env
            if let Err(e) = std::fs::write(&env_path, format!("ROOT_KEY={root_key_hex}\n")) {
                eprintln!("  ERROR: Cannot write root key: {e}");
                return 1;
            }

            match dolphin_milk::wallet::EmbeddedWalletClient::init(
                db_path.to_str().unwrap_or("wallet.db"),
                &root_key_hex,
                bsv_wallet_toolbox_rs::Chain::Main,
            )
            .await
            {
                Ok(embedded) => {
                    let identity = embedded.get_identity_key().await.unwrap_or_default();
                    println!("    Wallet DB: {} (created)", db_path.display());
                    println!("    Identity:  {identity}");
                    println!("    Balance:   0 sats");

                    // Show funding address
                    if let Ok((_, script, _)) = embedded.receive_address("1").await {
                        if let Ok(script_bytes) = hex::decode(&script) {
                            if script_bytes.len() >= 23 {
                                let pkh = &script_bytes[3..23];
                                use sha2::{Digest, Sha256};
                                let mut payload = vec![0x00u8];
                                payload.extend_from_slice(pkh);
                                let checksum = Sha256::digest(Sha256::digest(&payload));
                                payload.extend_from_slice(&checksum[..4]);
                                let address = base58_encode(&payload);
                                println!();
                                println!("  Fund your agent:");
                                println!("    1. Send BSV to: {address}");
                                println!("    2. Start the agent: dolphin-milk start");
                                println!("    3. Then run: dolphin-milk fund <TXID>");
                            }
                        }
                    }

                    // CRITICAL — wallet backup instructions.
                    // The user's BSV is locked behind two files. If they lose
                    // either one and haven't backed it up, the funds are gone
                    // forever. This warning is mandatory on every fresh init.
                    println!();
                    println!("  ⚠  BACK UP YOUR WALLET");
                    println!();
                    println!("  Two files contain everything needed to recover your funds.");
                    println!("  If you lose them, your BSV is gone forever.");
                    println!();
                    println!("    {}    (master key — 32 bytes)", env_path.display());
                    println!("    {}   (encrypted UTXO state)", db_path.display());
                    println!();
                    println!("  Copy both to a safe place NOW. A USB drive, a password manager,");
                    println!("  an encrypted backup. Anywhere except this disk only.");
                }
                Err(e) => {
                    eprintln!("  ERROR: Failed to create wallet: {e}");
                    // Clean up partial state
                    let _ = std::fs::remove_file(&db_path);
                    let _ = std::fs::remove_file(&env_path);
                    return 1;
                }
            }
        }

        #[cfg(not(feature = "embedded-wallet"))]
        {
            println!("  Embedded wallet not available (compile with --features embedded-wallet).");
            println!("  Install bsv-wallet-cli instead:");
            println!();
            println!("    cargo install bsv-wallet-cli");
            println!("    bsv-wallet init");
            println!("    bsv-wallet daemon");
            return 1;
        }
    }

    println!();
    println!("  ~200 sats per LLM call. A few thousand sats to explore.");
    println!();
    println!("  Next steps:");
    println!("    dolphin-milk start          Launch everything (wallet + web UI)");
    println!();
    0
}

async fn print_funding_instructions(wallet: &WalletClient) {
    println!();
    println!("  Your wallet is empty. To fund it:");
    println!();
    if let Ok((_, script, _)) = wallet.receive_address("1").await {
        if let Ok(script_bytes) = hex::decode(&script) {
            if script_bytes.len() >= 23 {
                let pkh = &script_bytes[3..23];
                use sha2::{Digest, Sha256};
                let mut payload = vec![0x00u8];
                payload.extend_from_slice(pkh);
                let checksum = Sha256::digest(Sha256::digest(&payload));
                payload.extend_from_slice(&checksum[..4]);
                let address = base58_encode(&payload);
                println!("    1. Send BSV to: {address}");
                println!("    2. Then run: dolphin-milk fund <TXID>");
                return;
            }
        }
    }
    println!("    1. Run: dolphin-milk receive");
    println!("    2. Send BSV to that address");
    println!("    3. Run: dolphin-milk fund <TXID>");
}

async fn cmd_start(port: u16, workspace: Option<String>) -> i32 {
    let mut cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    // Defend against multi-instance collision: if another wallet is on
    // cfg.wallet.url with a different identity than our .env's ROOT_KEY,
    // rewrite cfg.wallet.url to a free port so try_spawn_wallet launches
    // OUR wallet instead of latching onto someone else's.
    ensure_own_wallet_url(&mut cfg).await;

    // Auto-spawn wallet if not already running.
    // The browser UI needs an HTTP wallet for BRC-31 auth, so we must ensure
    // bsv-wallet serve is running. The embedded wallet (in create_app_state)
    // is only a server-side fallback when bsv-wallet binary isn't installed.
    let mut _wallet_child: Option<std::process::Child> = None;
    let wallet = WalletClient::from_config(&cfg.wallet);
    if check_wallet(&wallet).await.is_err() {
        _wallet_child = try_spawn_wallet(&cfg);
        if _wallet_child.is_some() {
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if check_wallet(&wallet).await.is_ok() {
                    break;
                }
            }
        }
    }

    // Pre-flight check
    match check_wallet(&wallet).await {
        Ok((identity_key, balance)) => {
            banner::print_startup_banner(
                port,
                &identity_key,
                balance,
                &cfg.llm.default_model,
                &cfg.wallet.url,
            );

            if let Some(b) = balance {
                if b == 0 {
                    println!("  WARNING: Wallet is empty. Fund it to use the agent.");
                    println!("  Run: dolphin-milk receive");
                    println!();
                }
            }
        }
        Err(_) => {
            // Wallet not reachable even after spawn attempt.
            // Embedded wallet in create_app_state() may still save us server-side,
            // but the UI won't have BRC-31 auth without an HTTP wallet.
            #[cfg_attr(not(feature = "embedded-wallet"), allow(unused_mut))]
            let mut can_start = false;

            #[cfg(feature = "embedded-wallet")]
            {
                let db_path = cfg.wallet.db_path.clone().unwrap_or_else(|| {
                    cfg.resolved_data_dir()
                        .join("wallet.db")
                        .to_string_lossy()
                        .into_owned()
                });
                if std::path::Path::new(&db_path).exists() {
                    println!();
                    println!("  Dolphin Milk v{VERSION}");
                    println!("  WARNING: No HTTP wallet — UI auth will not work.");
                    println!("  Server starting with embedded wallet (API-only mode).");
                    println!("  Port: {port}");
                    println!();
                    can_start = true;
                }
            }

            if !can_start {
                banner::print_wallet_error(&cfg.wallet.url);
                eprintln!("  Tip: Run `dolphin-milk init` first to create a wallet.");
                eprintln!();
                return 1;
            }
        }
    }

    let ws = match workspace {
        Some(ref w) => PathBuf::from(w),
        None if cfg.data_dir.is_some() => cfg.workspace_dir(),
        None => PathBuf::from("working"),
    };

    let result = match server::serve(cfg, ws, port).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Server error: {e}");
            1
        }
    };

    // Clean up child wallet process
    if let Some(ref mut child) = _wallet_child {
        let _ = child.kill();
    }

    result
}

/// Try to spawn `bsv-wallet daemon` as a child process for the wallet DB.
/// Uses `daemon` (not `serve`) so the Monitor background tasks run —
/// including `check_for_proofs` which syncs merkle proofs after confirmation.
/// Ensure the wallet dolphin-milk will use actually belongs to THIS data dir,
/// not some other wallet that happens to be listening on the same port.
///
/// This handles the "multi-instance" case where another dolphin-milk (or
/// bsv-wallet-cli — e.g. a DMS agent) is already listening on the default
/// wallet port `:3322`. Without this check, `cmd_serve` would probe that
/// port, find it responsive, and silently latch onto someone else's wallet,
/// using THEIR identity for BRC-31 signing and THEIR UTXO set for spending.
///
/// Flow:
/// 1. Read our `.env` ROOT_KEY and compute the expected identity.
/// 2. Probe `cfg.wallet.url`. If reachable AND identity matches → OK.
/// 3. Otherwise (unreachable OR identity mismatch): pick a free port,
///    rewrite `cfg.wallet.url` in place, return it to the caller so
///    `try_spawn_wallet` can launch `bsv-wallet daemon` on that port
///    pointing at OUR `wallet.db`.
///
/// Returns the expected identity key (for cross-check after spawn) or None
/// if we have no local wallet state to defend against.
async fn ensure_own_wallet_url(cfg: &mut dolphin_milk::config::DmConfig) -> Option<String> {
    // Step 1: do we have local wallet state? If not, nothing to defend.
    let db_path = cfg.wallet.db_path.clone().unwrap_or_else(|| {
        cfg.resolved_data_dir()
            .join("wallet.db")
            .to_string_lossy()
            .into_owned()
    });
    let db_p = std::path::Path::new(&db_path);
    if !db_p.exists() {
        return None;
    }
    let env_path = db_p
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(".env");
    if !env_path.exists() {
        return None;
    }

    // Step 2: derive the expected identity from our ROOT_KEY.
    let env_content = std::fs::read_to_string(&env_path).ok()?;
    let root_key_hex = env_content.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("ROOT_KEY=")
            .or_else(|| line.strip_prefix("SERVER_PRIVATE_KEY="))
            .map(|v| v.trim().trim_matches('"').to_string())
    })?;

    // Compute identity from ROOT_KEY using bsv-rs primitives.
    let expected_identity = {
        use bsv::primitives::PrivateKey;
        let pk = PrivateKey::from_hex(&root_key_hex).ok()?;
        pk.public_key().to_hex()
    };

    // Step 3: probe the configured URL and compare identity.
    let probed = WalletClient::from_config(&cfg.wallet);
    let matches_ours = match check_wallet(&probed).await {
        Ok((identity, _)) => identity == expected_identity,
        Err(_) => false,
    };

    if matches_ours {
        // Our own wallet is already running at cfg.wallet.url — leave it.
        return Some(expected_identity);
    }

    // Mismatch or unreachable: pick a free port and rewrite the URL.
    // The subsequent `try_spawn_wallet(cfg)` will launch `bsv-wallet daemon`
    // on the new port pointing at OUR db, and downstream probes will find it.
    let free_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        drop(listener);
        port
    };

    let new_url = format!("http://localhost:{free_port}");
    eprintln!(
        "  Wallet isolation: port 3322 is taken by another wallet (identity mismatch). \
         Spawning own wallet at {new_url} backed by {db_path}."
    );
    cfg.wallet.url = new_url;

    Some(expected_identity)
}

fn try_spawn_wallet(cfg: &dolphin_milk::config::DmConfig) -> Option<std::process::Child> {
    let db_path = cfg.wallet.db_path.clone().unwrap_or_else(|| {
        cfg.resolved_data_dir()
            .join("wallet.db")
            .to_string_lossy()
            .into_owned()
    });

    if !std::path::Path::new(&db_path).exists() {
        return None;
    }

    // Read root key from .env next to the DB
    let env_path = std::path::Path::new(&db_path)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(".env");

    let env_content = std::fs::read_to_string(&env_path).ok()?;
    let root_key = env_content.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("ROOT_KEY=")
            .or_else(|| line.strip_prefix("SERVER_PRIVATE_KEY="))
            .map(|v| v.trim().trim_matches('"').to_string())
    })?;

    // Extract port from wallet URL
    let wallet_port = cfg
        .wallet
        .url
        .rsplit(':')
        .next()
        .and_then(|p: &str| p.parse::<u16>().ok())
        .unwrap_or(3322);

    eprintln!("  Starting wallet (bsv-wallet daemon --db {db_path} --port {wallet_port})...");

    std::process::Command::new("bsv-wallet")
        .arg("daemon")
        .arg("--db")
        .arg(&db_path)
        .arg("--port")
        .arg(wallet_port.to_string())
        .env("ROOT_KEY", &root_key)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
}

async fn cmd_status() -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    let line = "\u{2500}".repeat(40);

    println!();
    println!("  Dolphin Milk v{VERSION}");
    println!("  {line}");

    // ── Wallet section ──
    let wallet = WalletClient::from_config(&cfg.wallet);
    let wallet_ok = match check_wallet(&wallet).await {
        Ok((identity_key, balance)) => {
            println!("  Wallet:    {} \u{2713}", cfg.wallet.url);
            println!("  Identity:  {identity_key}");
            if let Some(b) = balance {
                println!("  Balance:   {} sats", banner::format_sats_human(b));
            }
            true
        }
        Err(_) => {
            println!("  Wallet:    {} \u{2717}", cfg.wallet.url);
            false
        }
    };

    // ── Config section ──
    println!();
    println!(
        "  Model:     {} ({})",
        cfg.llm.default_model, cfg.llm.default_provider
    );
    println!(
        "  Budget:    {} sats/task, {}",
        banner::format_sats_human(cfg.budget.max_per_task),
        cfg.budget.enforcement
    );
    let scheduler = if cfg.heartbeat.enabled {
        "enabled"
    } else {
        "disabled"
    };
    println!("  Scheduler: {scheduler}");
    let data_dir = cfg.resolved_data_dir();
    println!("  Data dir:  {}", data_dir.display());

    // ── Server section (if running) ──
    if let Ok(resp) = reqwest::Client::new()
        .get("http://localhost:8080/health")
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            let uptime = body
                .get("uptime_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let hours = uptime / 3600;
            let mins = (uptime % 3600) / 60;
            println!();
            println!("  Server:    http://localhost:8080 \u{2713}");
            println!("  Uptime:    {hours}h {mins}m");
        }
    }

    println!("  {line}");

    // ── Next action hint ──
    if !wallet_ok {
        println!();
        banner::print_wallet_error(&cfg.wallet.url);
        return 1;
    }

    println!();
    println!("  Commands:");
    println!("    dolphin-milk start          Start the web UI + daemon");
    println!("    dolphin-milk think MESSAGE  Single LLM call (~200 sats)");
    println!("    dolphin-milk run TASK       Run an autonomous task");
    println!();

    0
}

async fn cmd_run(task: Option<String>, workspace: Option<String>, max_iterations: u32) -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    println!("Dolphin Milk v{VERSION}");

    let wallet = WalletClient::from_config(&cfg.wallet);
    match check_wallet(&wallet).await {
        Ok((identity_key, balance)) => {
            println!("Wallet: {} \u{2713}", cfg.wallet.url);
            println!("Identity: {identity_key}");
            if let Some(b) = balance {
                println!("Balance: {b} sats");
            }
        }
        Err(_) => {
            println!("Wallet: {} \u{2717}", cfg.wallet.url);
            println!("  ERROR: Wallet not reachable. Is bsv-wallet-cli running?");
            println!("  Start with: bsv-wallet daemon --port 3322");
            return 1;
        }
    }

    let task = match task {
        Some(t) if !t.is_empty() => t,
        _ => {
            println!("No task provided. Use: dolphin-milk run 'your task here'");
            return 1;
        }
    };

    let ws = match workspace {
        Some(ref w) => PathBuf::from(w),
        None if cfg.data_dir.is_some() => cfg.workspace_dir(),
        None => PathBuf::from("working"),
    };

    let mut loop_instance = runner::create_loop(cfg, ws, None, None, std::sync::Arc::new(wallet));

    println!(
        "Tools: {}",
        loop_instance.tools.read().await.tool_names().join(", ")
    );
    println!(
        "Loop detection: warn={}, critical={}, breaker={}",
        loop_instance.detector.warn_threshold,
        loop_instance.detector.critical_threshold,
        loop_instance.detector.breaker_threshold,
    );
    println!();
    println!("Task: {task}");
    println!();

    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    loop_instance.run(&task, max_iterations, None, cancel).await;

    if !loop_instance.state.exec.result.is_empty() {
        println!("{}", loop_instance.state.exec.result);
        println!();
    }

    println!("--- session summary ---");
    println!("Iterations:  {}", loop_instance.state.exec.iteration);
    println!("Sats spent:  {}", loop_instance.state.budget.sats_spent);

    let (pt, ct) = loop_instance.transcript.total_tokens();
    if pt > 0 || ct > 0 {
        println!("Tokens:      {} ({} in + {} out)", pt + ct, pt, ct);
    }

    if !loop_instance.state.exec.error.is_empty() {
        println!("Error:       {}", loop_instance.state.exec.error);
        return 1;
    }

    0
}

async fn cmd_think(message: &str, model: &str) -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    let wallet = WalletClient::from_config(&cfg.wallet);
    let wallet_url = cfg.wallet.url.clone();
    let auth = AuthriteClient::new(std::sync::Arc::new(wallet), &wallet_url);

    println!("Dolphin Milk v{VERSION}");
    println!("Thinking via x402... (model={model})");
    println!();

    let messages = vec![serde_json::json!({"role": "user", "content": message})];

    let max_tokens = if think::is_reasoning_model(model) {
        cfg.llm.max_tokens_reasoning
    } else {
        cfg.llm.max_tokens
    };

    match think::think(&auth, &messages, model, max_tokens, None, &cfg).await {
        Ok(result) => {
            println!("{}", result.text);
            println!();
            println!("--- x402 receipt ---");
            println!("Model:       {}", result.model);
            println!(
                "Tokens:      {} ({} in + {} out)",
                result.total_tokens, result.prompt_tokens, result.completion_tokens
            );
            println!("Sats paid:   {}", result.sats_paid);
            println!("Sats effective: {}", result.sats_effective);
            if result.sats_refunded > 0 {
                println!("Sats refunded: {}", result.sats_refunded);
            }
            println!("Latency:     {}ms", result.duration_ms);
            0
        }
        Err(e) => {
            println!("ERROR: {e}");
            if let Some(ctx) = match &e {
                DmError::Payment { context, .. } => Some(context),
                DmError::Wallet { context, .. } => Some(context),
                _ => None,
            } {
                for (k, v) in ctx {
                    println!("  {k}: {v}");
                }
            }
            1
        }
    }
}

async fn cmd_receive(suffix: &str) -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };

    let wallet = WalletClient::from_config(&cfg.wallet);

    println!("Dolphin Milk v{VERSION}");

    match wallet.receive_address(suffix).await {
        Ok((pubkey, script, _suffix)) => {
            // Compute BSV address from the P2PKH script
            let script_bytes = match hex::decode(&script) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Failed to decode script: {e}");
                    return 1;
                }
            };
            // Extract hash160 from P2PKH script: 76 a9 14 <20-bytes> 88 ac
            let pkh = &script_bytes[3..23];

            // Base58Check encode: version(0x00) + hash160 + checksum
            use sha2::{Digest, Sha256};
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(pkh);
            let checksum = Sha256::digest(Sha256::digest(&payload));
            payload.extend_from_slice(&checksum[..4]);

            let address = base58_encode(&payload);

            println!();
            println!("  BSV address: {address}");
            println!("  Public key:  {pubkey}");
            println!("  Suffix:      {suffix}");
            println!();
            println!("Send BSV to the address above, then internalize with:");
            println!("  dolphin-milk fund <TXID> --vout <N> --suffix {suffix}");
            0
        }
        Err(e) => {
            eprintln!("ERROR: {e}");
            1
        }
    }
}

async fn cmd_fund(txid: &str, vout: Option<u32>, suffix: &str) -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    let wallet = WalletClient::from_config(&cfg.wallet);

    println!("Dolphin Milk v{VERSION}");
    match vout {
        Some(v) => println!("Funding from tx {txid} vout {v} (suffix={suffix})..."),
        None => println!("Funding from tx {txid} (auto-detect vout, suffix={suffix})..."),
    }

    match wallet.fund_from_woc(txid, vout, Some(suffix)).await {
        Ok(result) => {
            let accepted = result
                .get("accepted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if accepted {
                println!("Success: funding internalized");
                // Show balance after funding
                if let Ok((_key, Some(bal))) = check_wallet(&wallet).await {
                    println!();
                    println!("  Balance:   {} sats", banner::format_sats_human(bal));
                    println!();
                    println!("  Ready: dolphin-milk start");
                }
                0
            } else {
                println!("Warning: wallet returned {result}");
                1
            }
        }
        Err(e) => {
            println!("ERROR: {e}");
            1
        }
    }
}

async fn cmd_split(count: u32) -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    println!();
    println!("  Dolphin Milk v{VERSION}");
    println!("  Splitting spendable balance into {count} UTXOs...");
    println!();

    #[cfg(feature = "embedded-wallet")]
    {
        let wallet =
            match dolphin_milk::wallet::EmbeddedWalletClient::from_config(&cfg).await {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("  ERROR: Failed to open wallet: {e}");
                    return 1;
                }
            };

        match wallet.split_utxos(count).await {
            Ok((txid, per_output, n)) => {
                println!("  ✓ Split into {n} UTXOs ({per_output} sats each)");
                println!("    TxID:  {txid}");
                println!("    View:  https://whatsonchain.com/tx/{txid}");
                println!();
                println!("  Each output is now independently spendable.");
                0
            }
            Err(e) => {
                eprintln!("  ERROR: split failed: {e}");
                1
            }
        }
    }

    #[cfg(not(feature = "embedded-wallet"))]
    {
        let _ = count;
        let _ = cfg;
        eprintln!("  ERROR: split requires the embedded-wallet feature.");
        eprintln!("  Rebuild with: cargo build --release --features embedded-wallet");
        1
    }
}

async fn cmd_serve(port: u16, workspace: Option<String>) -> i32 {
    let mut cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    println!("Dolphin Milk v{VERSION}");

    // Defend against multi-instance collision: if another wallet is on
    // cfg.wallet.url with a different identity than our .env's ROOT_KEY,
    // rewrite cfg.wallet.url to a free port so try_spawn_wallet launches
    // OUR wallet instead of latching onto someone else's.
    ensure_own_wallet_url(&mut cfg).await;

    // Spawn our own wallet daemon if nothing's at the URL yet.
    let mut _wallet_child: Option<std::process::Child> = None;
    let wallet = WalletClient::from_config(&cfg.wallet);
    if check_wallet(&wallet).await.is_err() {
        _wallet_child = try_spawn_wallet(&cfg);
        if _wallet_child.is_some() {
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if check_wallet(&wallet).await.is_ok() {
                    break;
                }
            }
        }
    }

    match check_wallet(&wallet).await {
        Ok((identity_key, _)) => {
            println!("Wallet: {} \u{2713}", cfg.wallet.url);
            println!("Identity: {identity_key}");
        }
        Err(_) => {
            println!("Wallet: {} \u{2717}", cfg.wallet.url);
            println!("  WARNING: Wallet not reachable. Server will start but tasks will fail.");
        }
    }

    let ws = match workspace {
        Some(ref w) => PathBuf::from(w),
        None if cfg.data_dir.is_some() => cfg.workspace_dir(),
        None => PathBuf::from("working"),
    };

    match server::serve(cfg, ws, port).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Server error: {e}");
            1
        }
    }
}

async fn cmd_mcp() -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };

    match dolphin_milk::mcp::server::run_mcp_server(&cfg).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("MCP server error: {e}");
            1
        }
    }
}

async fn cmd_audit(action: AuditAction) -> i32 {
    let cfg = match load_config(None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {e}");
            return 1;
        }
    };
    logging::setup_logging(&cfg.logging.level, &cfg.logging.format);

    println!("Dolphin Milk v{VERSION}");

    match action {
        AuditAction::RevealLinkage {
            counterparty,
            verifier,
            protocol,
            key_id,
            workspace,
        } => {
            let wallet = WalletClient::from_config(&cfg.wallet);

            // Use verifier or default to counterparty
            let verifier = verifier.unwrap_or_else(|| counterparty.clone());

            let ws = workspace.unwrap_or_else(|| "working".into());

            if let Some(protocol_id) = &protocol {
                // Specific key linkage revelation
                let key = key_id.unwrap_or_else(|| "default".into());
                let proto_val = serde_json::json!([2, protocol_id]);

                println!("Revealing specific key linkage...");
                println!("  Counterparty: {counterparty}");
                println!("  Verifier:     {verifier}");
                println!("  Protocol:     {protocol_id}");
                println!("  Key ID:       {key}");

                match wallet
                    .reveal_specific_key_linkage(&counterparty, &verifier, &proto_val, &key, false)
                    .await
                {
                    Ok(result) => {
                        let revelation = Revelation::specific(
                            &counterparty,
                            &proto_val.to_string(),
                            &key,
                            &verifier,
                            result.clone(),
                        );

                        let log = RevelationLog::new(&ws);
                        if let Err(e) = log.record(&revelation) {
                            eprintln!("Warning: failed to persist revelation: {e}");
                        }

                        println!();
                        println!("Revelation ID:   {}", revelation.id);
                        println!("Revelation hash: {}", revelation.revelation_hash);
                        println!("Timestamp:       {}", revelation.timestamp);
                        println!();
                        println!(
                            "Linkage data: {}",
                            serde_json::to_string_pretty(&result).unwrap_or_default()
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("ERROR: {e}");
                        1
                    }
                }
            } else {
                // Counterparty key linkage revelation
                println!("Revealing counterparty key linkage...");
                println!("  Counterparty: {counterparty}");
                println!("  Verifier:     {verifier}");

                match wallet
                    .reveal_counterparty_key_linkage(&counterparty, &verifier, false)
                    .await
                {
                    Ok(result) => {
                        let revelation =
                            Revelation::counterparty(&counterparty, &verifier, result.clone());

                        let log = RevelationLog::new(&ws);
                        if let Err(e) = log.record(&revelation) {
                            eprintln!("Warning: failed to persist revelation: {e}");
                        }

                        println!();
                        println!("Revelation ID:   {}", revelation.id);
                        println!("Revelation hash: {}", revelation.revelation_hash);
                        println!("Timestamp:       {}", revelation.timestamp);
                        println!();
                        println!(
                            "Linkage data: {}",
                            serde_json::to_string_pretty(&result).unwrap_or_default()
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("ERROR: {e}");
                        1
                    }
                }
            }
        }
        AuditAction::ListRevelations { workspace } => {
            let ws = workspace.unwrap_or_else(|| "working".into());
            let log = RevelationLog::new(&ws);
            let revelations = log.list();

            if revelations.is_empty() {
                println!("No revelations recorded.");
                return 0;
            }

            println!("Recorded revelations ({}):", revelations.len());
            println!();

            for r in &revelations {
                println!("  ID:           {}", r.id);
                println!("  Type:         {}", r.revelation_type);
                println!("  Counterparty: {}", r.counterparty);
                if let Some(proto) = &r.protocol_id {
                    println!("  Protocol:     {proto}");
                }
                if let Some(key) = &r.key_id {
                    println!("  Key ID:       {key}");
                }
                println!("  Requested by: {}", r.requested_by);
                println!("  Timestamp:    {}", r.timestamp);
                println!("  Hash:         {}", r.revelation_hash);
                println!();
            }

            0
        }
    }
}

fn cmd_verify_work(proof_file: &str) -> i32 {
    use dolphin_milk::proofs::{ProofCommitment, ProofType};

    println!("Dolphin Milk v{VERSION}");
    println!("Verifying custody proof: {proof_file}");
    println!();

    // Read the JSON file
    let data = match std::fs::read_to_string(proof_file) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ERROR: Cannot read file: {e}");
            return 1;
        }
    };

    // Parse the JSON — accept either a CustodyProofResponse or a raw ProofCommitment
    let json: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: Invalid JSON: {e}");
            return 1;
        }
    };

    // Try to extract the proof data (handle both nested and flat formats)
    let proof = if let Some(p) = json.get("proof") {
        p.clone()
    } else {
        json.clone()
    };

    let proof_data = match proof.get("data").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => {
            eprintln!("ERROR: Missing 'data' field in proof JSON");
            return 1;
        }
    };

    let timestamp = match proof.get("timestamp").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            eprintln!("ERROR: Missing 'timestamp' field in proof JSON");
            return 1;
        }
    };

    let hash = match proof.get("hash").and_then(|v| v.as_str()) {
        Some(h) => h.to_string(),
        None => {
            eprintln!("ERROR: Missing 'hash' field in proof JSON");
            return 1;
        }
    };

    let prev_hash = proof.get("prev_hash").and_then(|v| v.as_str());

    // Recompute the proof hash
    let recomputed =
        ProofCommitment::with_timestamp(ProofType::Custody, &proof_data, &timestamp, prev_hash);

    let recomputed_hex = recomputed.hash_hex();
    let consistent = recomputed_hex == hash;

    println!("  Proof type:      custody");
    println!("  Timestamp:       {timestamp}");
    println!("  Stored hash:     {hash}");
    println!("  Recomputed hash: {recomputed_hex}");
    if let Some(ph) = prev_hash {
        println!("  Prev hash:       {ph}");
    }
    println!();

    // Parse and display structured fields
    for line in proof_data.lines() {
        if line.contains(':') {
            println!("  {line}");
        }
    }
    println!();

    if consistent {
        println!("RESULT: Proof is internally consistent (hash matches data + timestamp)");
        0
    } else {
        println!("RESULT: PROOF MISMATCH — hash does not match data + timestamp");
        1
    }
}

async fn check_wallet(wallet: &WalletClient) -> Result<(String, Option<u64>), DmError> {
    let _auth = wallet.is_authenticated().await?;
    let identity_key = wallet.get_identity_key().await?;
    let balance = wallet.get_balance().await.ok();
    Ok((identity_key, balance))
}

/// Bitcoin Base58 encoding (used for BSV address display).
fn base58_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

    // Count leading zeros
    let leading_zeros = data.iter().take_while(|&&b| b == 0).count();

    // Convert to base58
    let mut digits: Vec<u8> = Vec::new();
    for &byte in data {
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) * 256;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }

    // Build result: leading '1's + digits in reverse
    let mut result = String::with_capacity(leading_zeros + digits.len());
    for _ in 0..leading_zeros {
        result.push('1');
    }
    for &d in digits.iter().rev() {
        result.push(ALPHABET[d as usize] as char);
    }
    result
}
