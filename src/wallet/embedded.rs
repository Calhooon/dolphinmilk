//! Embedded wallet backend — calls `bsv-wallet-toolbox-rs` in-process.
//!
//! Eliminates the need for a separate `bsv-wallet-cli` HTTP process by
//! linking the wallet library directly and calling [`WalletInterface`]
//! methods on an in-process [`Wallet<StorageSqlx, Services>`].
//!
//! # Architecture
//!
//! ```text
//! Current:  WormRunner → HttpWalletClient (HTTP) → bsv-wallet-cli → toolbox → SQLite
//! New:      WormRunner → EmbeddedWalletClient     → toolbox → SQLite  (in-process)
//! ```
//!
//! # Dependency
//!
//! Uses `bsv-rs` from crates.io (pinned to the same version the toolbox
//! links against) for `WalletInterface` argument types and BEEF parsing.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::error::DmError;

use super::types::{CreateActionResult, ANYONE_KEY};
use super::WalletBackend;

use bsv::primitives::{to_hex, PrivateKey, PublicKey};
use bsv::wallet::{
    AbortActionArgs,
    // Certificate arg types
    AcquireCertificateArgs,
    BasketInsertion,
    Counterparty,
    // Action arg types (from types — have serde)
    CreateActionArgs,
    CreateActionInput,
    CreateActionOptions,
    CreateActionOutput,
    // Crypto arg types (from proto_wallet — no serde, manual construction)
    CreateHmacArgs,
    CreateSignatureArgs,
    DecryptArgs,
    DiscoverByAttributesArgs,
    // Discovery arg types
    DiscoverByIdentityKeyArgs,
    EncryptArgs,
    // Header arg types
    GetHeaderArgs,
    GetPublicKeyArgs,
    InternalizeActionArgs,
    InternalizeOutput,
    ListActionsArgs,
    ListCertificatesArgs,
    // Output arg types
    ListOutputsArgs,
    Outpoint,
    // Output include mode
    OutputInclude,
    Protocol,
    ProveCertificateArgs,
    RelinquishCertificateArgs,
    RelinquishOutputArgs,
    SecurityLevel,
    SignActionArgs,
    TrustSelf,
    VerifyHmacArgs,
    VerifySignatureArgs,
    // Wallet trait
    WalletInterface,
    // Payment/insertion types
    WalletPayment,
    // Key linkage (interface args — no serde, manual construction)
    WalletRevealCounterpartyArgs,
    WalletRevealSpecificArgs,
};

// ── Toolbox imports ──────────────────────────────────────────────────
use bsv_wallet_toolbox_rs::{
    services::providers::ArcConfig, Chain, Monitor, Services, ServicesOptions, StorageSqlx, Wallet,
    WalletStorageWriter,
};

const TAAL_ARC_URL: &str = "https://arc.taal.com";

// BEEF parsing uses the git bsv crate — local-only, no cross-crate boundary.
use bsv::script::templates::P2PKH;
use bsv::transaction::Beef;

/// Originator string passed to all WalletInterface calls.
const ORIGINATOR: &str = "dolphin-milk";

/// Embedded wallet client wrapping a toolbox `Wallet` instance.
///
/// Implements [`WalletBackend`] by calling the wallet library directly
/// instead of making HTTP calls. Uses a spending lock to serialize
/// state-modifying operations (same pattern as `bsv-wallet-cli`).
#[derive(Clone)]
pub struct EmbeddedWalletClient {
    wallet: Arc<Wallet<StorageSqlx, Services>>,
    spending_lock: Arc<Mutex<()>>,
    identity_key: String,
    http_client: reqwest::Client,
    /// Optional background Monitor — started only by `open_with_monitor` /
    /// `from_config_with_monitor`. Held via `Arc` so the daemon keeps its
    /// spawned tokio tasks (including `check_for_proofs`, which fills in
    /// merkle proofs/BUMPs after confirmation) alive for the process lifetime.
    _monitor: Option<Arc<Monitor<StorageSqlx, Services>>>,
}

impl EmbeddedWalletClient {
    /// Open an existing wallet database.
    ///
    /// The `root_key_hex` is the 64-char hex private key. The database
    /// at `db_path` must already exist (created by `init()`).
    ///
    /// Does NOT start the background Monitor. Use [`open_with_monitor`] for
    /// long-running daemons that need automatic merkle-proof syncing.
    pub async fn open(db_path: &str, root_key_hex: &str, chain: Chain) -> Result<Self, DmError> {
        Self::open_inner(db_path, root_key_hex, chain, false).await
    }

    /// Open an existing wallet database AND start the background Monitor.
    ///
    /// The Monitor runs 12 tokio background tasks — most importantly
    /// `check_for_proofs` (every 60s), which fetches merkle proofs/BUMPs
    /// from configured block-header sources and stitches them into
    /// `proven_txs`. Without it, transactions confirm on-chain but their
    /// BEEF never gets merkle paths — downstream BEEF consumers will
    /// reject the "raw-tx-only" form.
    ///
    /// Requires a second `StorageSqlx` handle on the same SQLite DB
    /// because `Wallet::with_chain` takes owned values. SQLite handles
    /// the concurrency via `busy_timeout`; `bsv-wallet-cli daemon` uses
    /// the same pattern in production.
    pub async fn open_with_monitor(
        db_path: &str,
        root_key_hex: &str,
        chain: Chain,
    ) -> Result<Self, DmError> {
        Self::open_inner(db_path, root_key_hex, chain, true).await
    }

    async fn open_inner(
        db_path: &str,
        root_key_hex: &str,
        chain: Chain,
        start_monitor: bool,
    ) -> Result<Self, DmError> {
        let root_key = PrivateKey::from_hex(root_key_hex)
            .map_err(|e| DmError::wallet(format!("invalid root key: {e}")))?;
        let identity_key = root_key.public_key().to_hex();

        // Optional Monitor: open a second storage+services pair, wire ChainTracker,
        // and start the daemon before the wallet is constructed. The Monitor keeps
        // its own storage Arc for the lifetime of the process.
        let monitor = if start_monitor {
            let mon_storage = Self::open_storage(db_path).await?;
            let mon_services = Self::build_services(chain)?;
            if let Some(ref ct) = mon_services.chaintracks {
                mon_storage.set_chain_tracker(ct.clone()).await;
            }
            let storage_arc = Arc::new(mon_storage);
            let services_arc = Arc::new(mon_services);
            let m = Arc::new(Monitor::new(storage_arc, services_arc));
            m.start()
                .await
                .map_err(|e| DmError::wallet(format!("monitor start: {e}")))?;
            tracing::info!("Embedded wallet Monitor started (check_for_proofs enabled)");
            Some(m)
        } else {
            None
        };

        // Wallet's own storage+services pair. Wired with ChainTracker so
        // Layer 4 BEEF validation on `create_action` uses the same source
        // as the Monitor. Must be a distinct Services instance — cannot
        // share Arc across ownership boundary.
        let storage = Self::open_storage(db_path).await?;
        let services = Self::build_services(chain)?;
        if let Some(ref ct) = services.chaintracks {
            storage.set_chain_tracker(ct.clone()).await;
        }

        let wallet = Wallet::with_chain(
            Some(root_key),
            storage,
            services,
            bsv_wallet_toolbox_rs::WalletOptions::default(),
            chain,
        )
        .await
        .map_err(|e| DmError::wallet(format!("wallet init: {e}")))?;

        Ok(Self {
            wallet: Arc::new(wallet),
            spending_lock: Arc::new(Mutex::new(())),
            identity_key,
            http_client: reqwest::Client::new(),
            _monitor: monitor,
        })
    }

    async fn open_storage(db_path: &str) -> Result<StorageSqlx, DmError> {
        // Toolbox's StorageSqlx::open sets `busy_timeout=5000` internally,
        // which is what keeps the Monitor + Wallet handles from deadlocking
        // on the same SQLite DB.
        let storage = StorageSqlx::open(db_path)
            .await
            .map_err(|e| DmError::wallet(format!("open storage: {e}")))?;
        storage
            .make_available()
            .await
            .map_err(|e| DmError::wallet(format!("make_available: {e}")))?;
        Ok(storage)
    }

    fn build_services(chain: Chain) -> Result<Services, DmError> {
        let mut opts = match chain {
            Chain::Main => ServicesOptions::mainnet(),
            Chain::Test => ServicesOptions::testnet(),
        };
        if let Ok(url) = std::env::var("CHAINTRACKS_URL") {
            opts = opts.with_chaintracks_url(url);
        }
        // Optionally authenticate TAAL broadcasts via MAIN_TAAL_API_KEY
        // env. TAAL accepts `Authorization: <key>` without the "Bearer "
        // prefix that ArcConfig.api_key would produce, so we pass the
        // header directly via additional_headers. TAAL is the first
        // post_beef provider in bsv-wallet-toolbox-rs 0.3.37+.
        if let Ok(api_key) = std::env::var("MAIN_TAAL_API_KEY") {
            if !api_key.is_empty() {
                let mut headers = std::collections::HashMap::new();
                headers.insert("Authorization".to_string(), api_key);
                opts = opts.with_arc(
                    TAAL_ARC_URL,
                    Some(ArcConfig {
                        headers: Some(headers),
                        ..Default::default()
                    }),
                );
            }
        }
        Services::with_options(chain, opts)
            .map_err(|e| DmError::wallet(format!("services init: {e}")))
    }

    /// Create from config — resolves db_path and root key, opens existing wallet.
    ///
    /// Root key is read from `SERVER_PRIVATE_KEY` env var or `{db_dir}/.env` file.
    /// The database must already exist (created by `dolphin-milk init`).
    ///
    /// Does NOT start the Monitor. Use [`from_config_with_monitor`] for the
    /// `serve` path.
    pub async fn from_config(cfg: &crate::config::DmConfig) -> Result<Self, DmError> {
        let (db_path, root_key) = Self::resolve_db_and_key(cfg)?;
        Self::open(&db_path, &root_key, Chain::Main).await
    }

    /// Like [`from_config`], but also starts the background Monitor so the
    /// embedded wallet automatically fills in merkle proofs (BUMPs) for
    /// confirmed transactions. Call this from the `serve` path only.
    pub async fn from_config_with_monitor(cfg: &crate::config::DmConfig) -> Result<Self, DmError> {
        let (db_path, root_key) = Self::resolve_db_and_key(cfg)?;
        Self::open_with_monitor(&db_path, &root_key, Chain::Main).await
    }

    fn resolve_db_and_key(cfg: &crate::config::DmConfig) -> Result<(String, String), DmError> {
        let db_path = cfg.wallet.db_path.clone().unwrap_or_else(|| {
            cfg.resolved_data_dir()
                .join("wallet.db")
                .to_string_lossy()
                .into_owned()
        });

        let root_key = if let Ok(key) = std::env::var("ROOT_KEY") {
            key
        } else if let Ok(key) = std::env::var("SERVER_PRIVATE_KEY") {
            key
        } else {
            let env_path = std::path::Path::new(&db_path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(".env");
            read_root_key_from_env_file(&env_path)?
        };

        if !std::path::Path::new(&db_path).exists() {
            return Err(DmError::wallet(format!(
                "Wallet database not found at {db_path}\n\n  Run `dolphin-milk init` to create one."
            )));
        }

        Ok((db_path, root_key))
    }

    /// Initialize a brand-new wallet (creates SQLite DB + runs migrations).
    pub async fn init(db_path: &str, root_key_hex: &str, chain: Chain) -> Result<Self, DmError> {
        // Validate key early.
        let root_key = PrivateKey::from_hex(root_key_hex)
            .map_err(|e| DmError::wallet(format!("invalid root key: {e}")))?;
        let identity_key = root_key.public_key().to_hex();

        // Create storage and run migrations for new DB.
        let storage = StorageSqlx::open(db_path)
            .await
            .map_err(|e| DmError::wallet(format!("create storage: {e}")))?;
        storage
            .migrate(ORIGINATOR, &identity_key)
            .await
            .map_err(|e| DmError::wallet(format!("migrate: {e}")))?;
        storage
            .make_available()
            .await
            .map_err(|e| DmError::wallet(format!("make_available: {e}")))?;

        // Delegate to open() now that the DB exists.
        drop(storage);
        Self::open(db_path, root_key_hex, chain).await
    }

    // ── Private helpers ──────────────────────────────────────────

    /// Funding derivation protocol — same as `HttpWalletClient::funding_protocol()`.
    fn funding_protocol() -> Value {
        json!([2, "3241645161d8"])
    }

    /// Default derivation prefix for receiving funds.
    const FUNDING_PREFIX: &'static str = "dolphin-milk-fund";
}

// ── Type-adapter helpers ─────────────────────────────────────────────

/// Parse `[level, "name"]` JSON array into [`Protocol`].
fn parse_protocol_id(v: &Value) -> Result<Protocol, DmError> {
    let arr = v
        .as_array()
        .ok_or_else(|| DmError::wallet("protocol_id must be a JSON array"))?;
    let level = arr
        .first()
        .and_then(|v| v.as_u64())
        .ok_or_else(|| DmError::wallet("protocol_id[0] must be an integer"))?;
    let name = arr
        .get(1)
        .and_then(|v| v.as_str())
        .ok_or_else(|| DmError::wallet("protocol_id[1] must be a string"))?;
    let security_level = match level {
        0 => SecurityLevel::Silent,
        1 => SecurityLevel::App,
        2 => SecurityLevel::Counterparty,
        _ => return Err(DmError::wallet(format!("invalid security level: {level}"))),
    };
    Ok(Protocol::new(security_level, name))
}

/// Parse counterparty string: `"self"`, `"anyone"`, or a 66-char hex public key.
fn parse_counterparty(s: &str) -> Result<Counterparty, DmError> {
    match s {
        "self" => Ok(Counterparty::Self_),
        "anyone" => Ok(Counterparty::Anyone),
        hex => Counterparty::from_hex(hex)
            .map_err(|e| DmError::wallet(format!("invalid counterparty key: {e}"))),
    }
}

/// Parse a single output JSON object into [`CreateActionOutput`].
fn parse_output(v: &Value) -> Result<CreateActionOutput, DmError> {
    let script_hex = v["lockingScript"]
        .as_str()
        .ok_or_else(|| DmError::wallet("output missing lockingScript"))?;
    let locking_script = hex::decode(script_hex)
        .map_err(|e| DmError::wallet(format!("invalid lockingScript hex: {e}")))?;
    let satoshis = v["satoshis"].as_u64().unwrap_or(0);
    let output_description = v["outputDescription"]
        .as_str()
        .unwrap_or("output")
        .to_string();
    let basket = v["basket"].as_str().map(String::from);
    let custom_instructions = v["customInstructions"].as_str().map(String::from);
    let tags = v["tags"].as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    });

    Ok(CreateActionOutput {
        locking_script,
        satoshis,
        output_description,
        basket,
        custom_instructions,
        tags,
    })
}

/// Convert SDK [`bsv::wallet::CreateActionResult`] to our [`CreateActionResult`].
fn convert_create_action_result(
    result: bsv::wallet::CreateActionResult,
) -> Result<CreateActionResult, DmError> {
    let txid_hex = result.txid.as_ref().map(|t| to_hex(t)).unwrap_or_default();

    // Build AtomicBEEF from result.beef (with ancestors) + txid.
    let tx_bytes = match (result.beef.as_ref(), result.txid.as_ref()) {
        (Some(beef_bytes), Some(txid_bytes)) => {
            let hex_id = to_hex(txid_bytes);
            match Beef::from_binary(beef_bytes) {
                Ok(mut beef) => match beef.to_binary_atomic(&hex_id) {
                    Ok(ab) => ab,
                    Err(e) => {
                        warn!(error = %e, "Failed to build AtomicBEEF, falling back to raw tx");
                        result.tx.clone().unwrap_or_default()
                    }
                },
                Err(e) => {
                    warn!(error = %e, "Failed to parse BEEF, falling back to raw tx");
                    result.tx.clone().unwrap_or_default()
                }
            }
        }
        _ => result.tx.clone().unwrap_or_default(),
    };

    let raw = json!({
        "txid": txid_hex,
        "tx": tx_bytes,
        "sendWithResults": result.send_with_results.as_ref()
            .and_then(|r| serde_json::to_value(r).ok()),
    });

    Ok(CreateActionResult {
        txid: txid_hex,
        tx: tx_bytes,
        raw,
    })
}

/// Parse WOC BEEF bytes (may be hex string or raw bytes).
fn parse_woc_beef(raw: &[u8]) -> Result<Vec<u8>, DmError> {
    let data = if raw.starts_with(b"\"") && raw.ends_with(b"\"") {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };

    if let Ok(hex_str) = std::str::from_utf8(data) {
        let trimmed = hex_str.trim();
        if trimmed.len() >= 8 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(bytes) = hex::decode(trimmed) {
                return Ok(bytes);
            }
        }
    }

    Ok(data.to_vec())
}

// ── Inherent methods ─────────────────────────────────────────────────

impl EmbeddedWalletClient {
    // ── Key Management ───────────────────────────────────────────

    pub async fn get_identity_key(&self) -> Result<String, DmError> {
        Ok(self.identity_key.clone())
    }

    pub async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, DmError> {
        let args = GetPublicKeyArgs {
            identity_key: false,
            protocol_id: Some(parse_protocol_id(protocol_id)?),
            key_id: Some(key_id.to_string()),
            counterparty: Some(parse_counterparty(counterparty)?),
            for_self: Some(for_self),
        };
        let result = self
            .wallet
            .get_public_key(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("get_public_key: {e}")))?;
        Ok(result.public_key)
    }

    pub async fn raw_call(&self, method: &str, params: Option<Value>) -> Result<Value, DmError> {
        match method {
            "getPublicKey" => {
                let p = params.unwrap_or(json!({}));
                if p.get("identityKey")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    let key = self.get_identity_key().await?;
                    return Ok(json!({"publicKey": key}));
                }
                let protocol_id = p
                    .get("protocolID")
                    .ok_or_else(|| DmError::wallet("raw_call getPublicKey: missing protocolID"))?;
                let key_id = p["keyID"].as_str().unwrap_or("");
                let counterparty = p["counterparty"].as_str().unwrap_or("self");
                let for_self = p["forSelf"].as_bool().unwrap_or(false);
                let key = self
                    .get_public_key(protocol_id, key_id, counterparty, for_self)
                    .await?;
                Ok(json!({"publicKey": key}))
            }
            "isAuthenticated" => self.is_authenticated().await,
            "getHeight" => {
                let h = self.get_height().await?;
                Ok(json!({"height": h}))
            }
            "getNetwork" => {
                let n = self.get_network().await?;
                Ok(json!({"network": n}))
            }
            "getVersion" => {
                let v = self.get_version().await?;
                Ok(json!({"version": v}))
            }
            _ => Err(DmError::wallet(format!(
                "raw_call: unsupported method '{method}' for embedded wallet"
            ))),
        }
    }

    // ── Transaction Creation ─────────────────────────────────────

    pub async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        randomize_outputs: bool,
    ) -> Result<CreateActionResult, DmError> {
        let parsed_outputs: Vec<CreateActionOutput> =
            outputs.iter().map(parse_output).collect::<Result<_, _>>()?;

        let args = CreateActionArgs {
            description: description.to_string(),
            input_beef: None,
            inputs: Some(vec![]),
            outputs: Some(parsed_outputs),
            lock_time: None,
            version: None,
            labels: None,
            options: Some(CreateActionOptions {
                accept_delayed_broadcast: Some(accept_delayed_broadcast),
                randomize_outputs: Some(randomize_outputs),
                sign_and_process: Some(true),
                ..Default::default()
            }),
        };

        let _guard = self.spending_lock.lock().await;
        let result = self
            .wallet
            .create_action(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("create_action: {e}")))?;
        drop(_guard);

        convert_create_action_result(result)
    }

    pub async fn spend_output(
        &self,
        _basket: &str,
        txid: &str,
        vout: u32,
        description: &str,
    ) -> Result<CreateActionResult, DmError> {
        let outpoint = Outpoint::from_string(&format!("{txid}.{vout}"))
            .map_err(|e| DmError::wallet(format!("spend_output outpoint: {e}")))?;

        let args = CreateActionArgs {
            description: description.to_string(),
            input_beef: None,
            inputs: Some(vec![CreateActionInput {
                outpoint,
                input_description: description.to_string(),
                unlocking_script: None,
                unlocking_script_length: Some(73),
                sequence_number: None,
            }]),
            outputs: Some(vec![]),
            lock_time: None,
            version: None,
            labels: None,
            options: Some(CreateActionOptions {
                accept_delayed_broadcast: Some(false),
                randomize_outputs: Some(false),
                sign_and_process: Some(true),
                trust_self: Some(TrustSelf::Known),
                ..Default::default()
            }),
        };

        let _guard = self.spending_lock.lock().await;
        let result = self
            .wallet
            .create_action(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("spend_output: {e}")))?;
        drop(_guard);

        convert_create_action_result(result)
    }

    pub async fn internalize_action(
        &self,
        tx_bytes: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, DmError> {
        let parsed_outputs: Vec<InternalizeOutput> = outputs
            .iter()
            .map(|o| {
                let output_index = o["outputIndex"].as_u64().unwrap_or(0) as u32;
                let protocol = o["protocol"]
                    .as_str()
                    .unwrap_or("wallet payment")
                    .to_string();

                let payment_remittance = o.get("paymentRemittance").and_then(|p| {
                    if p.is_null() {
                        return None;
                    }
                    Some(WalletPayment {
                        derivation_prefix: p["derivationPrefix"].as_str().unwrap_or("").to_string(),
                        derivation_suffix: p["derivationSuffix"].as_str().unwrap_or("").to_string(),
                        sender_identity_key: p["senderIdentityKey"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                    })
                });

                let insertion_remittance = o.get("insertionRemittance").and_then(|i| {
                    if i.is_null() {
                        return None;
                    }
                    Some(BasketInsertion {
                        basket: i["basket"].as_str().unwrap_or("").to_string(),
                        custom_instructions: i["customInstructions"].as_str().map(String::from),
                        tags: i["tags"].as_array().map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        }),
                    })
                });

                InternalizeOutput {
                    output_index,
                    protocol,
                    payment_remittance,
                    insertion_remittance,
                }
            })
            .collect();

        let args = InternalizeActionArgs {
            tx: tx_bytes.to_vec(),
            outputs: parsed_outputs,
            description: description.to_string(),
            labels: None,
            seek_permission: None,
        };

        let result = self
            .wallet
            .internalize_action(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("internalize_action: {e}")))?;

        Ok(json!({"accepted": result.accepted}))
    }

    // ── Output Management ────────────────────────────────────────

    pub async fn get_balance(&self) -> Result<u64, DmError> {
        let (balance, _count) = self.get_balance_and_count().await?;
        Ok(balance)
    }

    /// List spendable outputs and return (total_balance_sats, spendable_output_count).
    pub async fn get_balance_and_count(&self) -> Result<(u64, u64), DmError> {
        let mut total: u64 = 0;
        let mut count: u64 = 0;
        let mut offset: i32 = 0;
        let limit: u32 = 100;
        loop {
            let args = ListOutputsArgs {
                basket: "default".to_string(),
                tags: None,
                tag_query_mode: None,
                include: Some(OutputInclude::LockingScripts),
                include_custom_instructions: None,
                include_tags: None,
                include_labels: None,
                limit: Some(limit),
                offset: Some(offset),
                seek_permission: None,
            };
            let result = self
                .wallet
                .list_outputs(args, ORIGINATOR)
                .await
                .map_err(|e| DmError::wallet(format!("get_balance: {e}")))?;

            if result.outputs.is_empty() {
                break;
            }
            for output in &result.outputs {
                if output.spendable {
                    total += output.satoshis;
                    count += 1;
                }
            }
            if (result.outputs.len() as u32) < limit {
                break;
            }
            offset += limit as i32;
        }
        Ok((total, count))
    }

    pub async fn list_outputs(
        &self,
        basket: &str,
        include: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        let include_mode = match include {
            "locking scripts" => Some(OutputInclude::LockingScripts),
            "entire transactions" => Some(OutputInclude::EntireTransactions),
            _ => Some(OutputInclude::LockingScripts),
        };
        let args = ListOutputsArgs {
            basket: basket.to_string(),
            tags: None,
            tag_query_mode: None,
            include: include_mode,
            include_custom_instructions: None,
            include_tags: None,
            include_labels: None,
            limit: Some(limit as u32),
            offset: Some(offset as i32),
            seek_permission: None,
        };
        let result = self
            .wallet
            .list_outputs(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("list_outputs: {e}")))?;

        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("list_outputs serialize: {e}")))
    }

    pub async fn relinquish_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
    ) -> Result<Value, DmError> {
        let outpoint = Outpoint::from_string(&format!("{txid}.{vout}"))
            .map_err(|e| DmError::wallet(format!("relinquish_output outpoint: {e}")))?;

        let args = RelinquishOutputArgs {
            basket: basket.to_string(),
            output: outpoint,
        };

        let _guard = self.spending_lock.lock().await;
        let result = self
            .wallet
            .relinquish_output(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("relinquish_output: {e}")))?;
        drop(_guard);

        Ok(json!({"relinquished": result.relinquished}))
    }

    // ── Cryptographic Operations ─────────────────────────────────

    pub async fn create_signature(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let args = CreateSignatureArgs {
            data: Some(data.to_vec()),
            hash_to_directly_sign: None,
            protocol_id: parse_protocol_id(protocol_id)?,
            key_id: key_id.to_string(),
            counterparty: Some(parse_counterparty(counterparty)?),
        };
        let result = self
            .wallet
            .create_signature(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("create_signature: {e}")))?;
        Ok(result.signature)
    }

    pub async fn verify_signature(
        &self,
        data: &[u8],
        signature: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        let args = VerifySignatureArgs {
            data: Some(data.to_vec()),
            hash_to_directly_verify: None,
            signature: signature.to_vec(),
            protocol_id: parse_protocol_id(protocol_id)?,
            key_id: key_id.to_string(),
            counterparty: Some(parse_counterparty(counterparty)?),
            for_self: None,
        };
        let result = self
            .wallet
            .verify_signature(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("verify_signature: {e}")))?;
        Ok(result.valid)
    }

    pub async fn encrypt(
        &self,
        plaintext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let args = EncryptArgs {
            plaintext: plaintext.to_vec(),
            protocol_id: parse_protocol_id(protocol_id)?,
            key_id: key_id.to_string(),
            counterparty: Some(parse_counterparty(counterparty)?),
        };
        let result = self
            .wallet
            .encrypt(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("encrypt: {e}")))?;
        Ok(result.ciphertext)
    }

    pub async fn decrypt(
        &self,
        ciphertext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let args = DecryptArgs {
            ciphertext: ciphertext.to_vec(),
            protocol_id: parse_protocol_id(protocol_id)?,
            key_id: key_id.to_string(),
            counterparty: Some(parse_counterparty(counterparty)?),
        };
        let result = self
            .wallet
            .decrypt(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("decrypt: {e}")))?;
        Ok(result.plaintext)
    }

    pub async fn create_hmac(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        let args = CreateHmacArgs {
            data: data.to_vec(),
            protocol_id: parse_protocol_id(protocol_id)?,
            key_id: key_id.to_string(),
            counterparty: Some(parse_counterparty(counterparty)?),
        };
        let result = self
            .wallet
            .create_hmac(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("create_hmac: {e}")))?;
        Ok(result.hmac.to_vec())
    }

    pub async fn verify_hmac(
        &self,
        data: &[u8],
        hmac: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        let hmac_array: [u8; 32] = hmac.try_into().map_err(|_| {
            DmError::wallet(format!(
                "verify_hmac: hmac must be exactly 32 bytes, got {}",
                hmac.len()
            ))
        })?;
        let args = VerifyHmacArgs {
            data: data.to_vec(),
            hmac: hmac_array,
            protocol_id: parse_protocol_id(protocol_id)?,
            key_id: key_id.to_string(),
            counterparty: Some(parse_counterparty(counterparty)?),
        };
        let result = self
            .wallet
            .verify_hmac(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("verify_hmac: {e}")))?;
        Ok(result.valid)
    }

    // ── Certificate Operations ───────────────────────────────────

    pub async fn acquire_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        let args: AcquireCertificateArgs = serde_json::from_value(certificate.clone())
            .map_err(|e| DmError::wallet(format!("acquire_certificate parse: {e}")))?;
        let result = self
            .wallet
            .acquire_certificate(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("acquire_certificate: {e}")))?;
        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("acquire_certificate serialize: {e}")))
    }

    pub async fn list_certificates(
        &self,
        certifiers: &[&str],
        types: &[&str],
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        let args = ListCertificatesArgs {
            certifiers: certifiers.iter().map(|s| s.to_string()).collect(),
            types: types.iter().map(|s| s.to_string()).collect(),
            limit: Some(limit as u32),
            offset: Some(offset as u32),
            privileged: None,
            privileged_reason: None,
        };
        let result = self
            .wallet
            .list_certificates(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("list_certificates: {e}")))?;
        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("list_certificates serialize: {e}")))
    }

    pub async fn prove_certificate(
        &self,
        certificate: &Value,
        fields_to_reveal: &[&str],
    ) -> Result<Value, DmError> {
        let args_json = json!({
            "certificate": certificate,
            "fieldsToReveal": fields_to_reveal,
            "verifier": certificate.get("verifier")
                .and_then(|v| v.as_str())
                .unwrap_or(&self.identity_key),
        });
        let args: ProveCertificateArgs = serde_json::from_value(args_json)
            .map_err(|e| DmError::wallet(format!("prove_certificate parse: {e}")))?;
        let result = self
            .wallet
            .prove_certificate(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("prove_certificate: {e}")))?;
        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("prove_certificate serialize: {e}")))
    }

    pub async fn relinquish_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        let cert_type = certificate
            .get("type")
            .or_else(|| certificate.get("certificateType"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let serial = certificate
            .get("serialNumber")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let certifier = certificate
            .get("certifier")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let args = RelinquishCertificateArgs {
            certificate_type: cert_type.to_string(),
            serial_number: serial.to_string(),
            certifier: certifier.to_string(),
        };
        let result = self
            .wallet
            .relinquish_certificate(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("relinquish_certificate: {e}")))?;
        Ok(json!({"relinquished": result.relinquished}))
    }

    // ── Discovery Operations ─────────────────────────────────────

    pub async fn discover_by_identity_key(
        &self,
        identity_key: &str,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        let args = DiscoverByIdentityKeyArgs {
            identity_key: identity_key.to_string(),
            limit: Some(limit as u32),
            offset: None,
            seek_permission: None,
        };
        let result = self
            .wallet
            .discover_by_identity_key(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("discover_by_identity_key: {e}")))?;
        let mut val = serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("discover_by_identity_key serialize: {e}")))?;
        if let Some(t) = cert_type {
            val["type"] = json!(t);
        }
        Ok(val)
    }

    pub async fn discover_by_attributes(
        &self,
        attributes: &Value,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        let attr_map: HashMap<String, String> = if let Some(obj) = attributes.as_object() {
            obj.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        } else {
            HashMap::new()
        };
        let args = DiscoverByAttributesArgs {
            attributes: attr_map,
            limit: Some(limit as u32),
            offset: None,
            seek_permission: None,
        };
        let result = self
            .wallet
            .discover_by_attributes(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("discover_by_attributes: {e}")))?;
        let mut val = serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("discover_by_attributes serialize: {e}")))?;
        if let Some(t) = cert_type {
            val["type"] = json!(t);
        }
        Ok(val)
    }

    // ── Key Linkage Operations ───────────────────────────────────

    pub async fn reveal_counterparty_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        let args = WalletRevealCounterpartyArgs {
            counterparty: PublicKey::from_hex(counterparty)
                .map_err(|e| DmError::wallet(format!("invalid counterparty key: {e}")))?,
            verifier: PublicKey::from_hex(verifier)
                .map_err(|e| DmError::wallet(format!("invalid verifier key: {e}")))?,
            privileged: Some(privileged),
            privileged_reason: None,
        };
        let result = self
            .wallet
            .reveal_counterparty_key_linkage(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("reveal_counterparty_key_linkage: {e}")))?;
        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("reveal_counterparty_key_linkage serialize: {e}")))
    }

    pub async fn reveal_specific_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        protocol_id: &Value,
        key_id: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        let args = WalletRevealSpecificArgs {
            counterparty: parse_counterparty(counterparty)?,
            verifier: PublicKey::from_hex(verifier)
                .map_err(|e| DmError::wallet(format!("invalid verifier key: {e}")))?,
            protocol_id: parse_protocol_id(protocol_id)?,
            key_id: key_id.to_string(),
            privileged: Some(privileged),
            privileged_reason: None,
        };
        let result = self
            .wallet
            .reveal_specific_key_linkage(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("reveal_specific_key_linkage: {e}")))?;
        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("reveal_specific_key_linkage serialize: {e}")))
    }

    // ── Status / Metadata ────────────────────────────────────────

    pub async fn is_authenticated(&self) -> Result<Value, DmError> {
        let result = self
            .wallet
            .is_authenticated(ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("is_authenticated: {e}")))?;
        Ok(json!({"authenticated": result.authenticated}))
    }

    pub async fn get_height(&self) -> Result<u64, DmError> {
        let result = self
            .wallet
            .get_height(ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("get_height: {e}")))?;
        Ok(result.height as u64)
    }

    pub async fn get_network(&self) -> Result<String, DmError> {
        let result = self
            .wallet
            .get_network(ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("get_network: {e}")))?;
        let val = serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("get_network serialize: {e}")))?;
        Ok(val["network"].as_str().unwrap_or("mainnet").to_string())
    }

    pub async fn get_version(&self) -> Result<String, DmError> {
        let result = self
            .wallet
            .get_version(ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("get_version: {e}")))?;
        Ok(result.version)
    }

    pub async fn wait_for_authentication(&self) -> Result<Value, DmError> {
        let result = self
            .wallet
            .wait_for_authentication(ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("wait_for_authentication: {e}")))?;
        Ok(json!({"authenticated": result.authenticated}))
    }

    // ── Chain Operations ─────────────────────────────────────────

    pub async fn get_header_for_height(&self, height: u64) -> Result<String, DmError> {
        let args = GetHeaderArgs {
            height: height as u32,
        };
        let result = self
            .wallet
            .get_header_for_height(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("get_header_for_height: {e}")))?;
        Ok(result.header)
    }

    // ── Action Management ────────────────────────────────────────

    pub async fn sign_action(&self, reference: &str) -> Result<Value, DmError> {
        let args = SignActionArgs {
            spends: HashMap::new(),
            reference: reference.to_string(),
            options: None,
        };
        let result = self
            .wallet
            .sign_action(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("sign_action: {e}")))?;
        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("sign_action serialize: {e}")))
    }

    pub async fn abort_action(&self, reference: &str) -> Result<Value, DmError> {
        let args = AbortActionArgs {
            reference: reference.to_string(),
        };
        let result = self
            .wallet
            .abort_action(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("abort_action: {e}")))?;
        Ok(json!({"aborted": result.aborted}))
    }

    // ── Transaction History ──────────────────────────────────────

    pub async fn list_actions(
        &self,
        labels: &[&str],
        include_labels: bool,
        include_inputs: bool,
        include_outputs: bool,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        let args = ListActionsArgs {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            label_query_mode: None,
            include_labels: Some(include_labels),
            include_inputs: Some(include_inputs),
            include_input_source_locking_scripts: None,
            include_input_unlocking_scripts: None,
            include_outputs: Some(include_outputs),
            include_output_locking_scripts: None,
            limit: Some(limit as u32),
            offset: Some(offset as u32),
            seek_permission: None,
        };
        let result = self
            .wallet
            .list_actions(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("list_actions: {e}")))?;
        serde_json::to_value(&result)
            .map_err(|e| DmError::wallet(format!("list_actions serialize: {e}")))
    }

    // ── Funding Operations ───────────────────────────────────────

    pub async fn receive_address(&self, suffix: &str) -> Result<(String, String, String), DmError> {
        let protocol_id = Self::funding_protocol();
        let key_id = format!("{} {suffix}", Self::FUNDING_PREFIX);

        let pubkey = self
            .get_public_key(&protocol_id, &key_id, ANYONE_KEY, true)
            .await?;

        let script = crate::x402::payment::build_p2pkh_script(&pubkey)?;

        Ok((pubkey, script, suffix.to_string()))
    }

    pub async fn fund_from_woc(
        &self,
        txid: &str,
        vout: Option<u32>,
        derivation_suffix: Option<&str>,
    ) -> Result<Value, DmError> {
        // Embedded wallet delegates to the same WOC flow — for now just require explicit vout
        let vout = vout.unwrap_or(0);
        let suffix = derivation_suffix.unwrap_or("1");

        // Validate txid
        if txid.len() != 64 {
            return Err(DmError::wallet(format!(
                "Invalid txid: expected 64 hex chars, got {}",
                txid.len()
            )));
        }
        hex::decode(txid).map_err(|_| DmError::wallet(format!("Invalid hex in txid: {txid}")))?;

        // Step 1: Derive the expected payment key and script
        let protocol_id = Self::funding_protocol();
        let key_id = format!("{} {suffix}", Self::FUNDING_PREFIX);

        let derived_pubkey = self
            .get_public_key(&protocol_id, &key_id, ANYONE_KEY, true)
            .await?;

        let expected_script = crate::x402::payment::build_p2pkh_script(&derived_pubkey)?;

        debug!(
            "Expected P2PKH script for suffix '{}': {}...{}",
            suffix,
            &expected_script[..10.min(expected_script.len())],
            &expected_script[expected_script.len().saturating_sub(4)..],
        );

        // Step 2: Fetch the actual output from WOC and verify the script matches
        let tx_url = format!("https://api.whatsonchain.com/v1/bsv/main/tx/{txid}");
        let tx_resp = self
            .http_client
            .get(&tx_url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to fetch tx from WOC: {e}")))?;

        if !tx_resp.status().is_success() {
            return Err(DmError::wallet(format!(
                "WOC returned HTTP {} for tx {txid}",
                tx_resp.status()
            )));
        }

        let tx_json: Value = tx_resp
            .json()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to parse tx JSON: {e}")))?;

        let vout_array = tx_json
            .get("vout")
            .and_then(|v| v.as_array())
            .ok_or_else(|| DmError::wallet("Missing 'vout' in tx JSON"))?;

        let output = vout_array.get(vout as usize).ok_or_else(|| {
            DmError::wallet(format!(
                "Output index {} out of range (tx has {} outputs)",
                vout,
                vout_array.len()
            ))
        })?;

        let actual_script = output
            .get("scriptPubKey")
            .and_then(|sp| sp.get("hex"))
            .and_then(|h| h.as_str())
            .unwrap_or("");

        if actual_script.to_lowercase() != expected_script.to_lowercase() {
            let actual_sats = output
                .get("value")
                .and_then(|v| v.as_f64())
                .map(|v| (v * 1e8) as u64)
                .unwrap_or(0);

            return Err(DmError::wallet(format!(
                "Script mismatch for tx {txid} vout {vout} ({actual_sats} sats).\n\
                 Expected: {expected_script}\n\
                 Actual:   {actual_script}\n\
                 This output was NOT sent to a wallet-derived address.\n\
                 Use 'dolphin-milk receive' to generate a proper receiving address first,\n\
                 then send BSV to that address."
            )));
        }

        let actual_sats = output
            .get("value")
            .and_then(|v| v.as_f64())
            .map(|v| (v * 1e8) as u64)
            .unwrap_or(0);

        debug!(
            "Script verified: output {vout} ({actual_sats} sats) matches derived key with suffix '{suffix}'"
        );

        // Step 3: Fetch BEEF from WhatsOnChain
        let woc_url = format!("https://api.whatsonchain.com/v1/bsv/main/tx/{txid}/beef");
        debug!("Fetching BEEF from WOC: {}", woc_url);

        let resp = self
            .http_client
            .get(&woc_url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to fetch BEEF from WOC: {e}")))?;

        if !resp.status().is_success() {
            return Err(DmError::wallet(format!(
                "WOC returned HTTP {} for BEEF of tx {txid}",
                resp.status()
            )));
        }

        let raw = resp
            .bytes()
            .await
            .map_err(|e| DmError::wallet(format!("Failed to read WOC response: {e}")))?;

        let beef_bytes = parse_woc_beef(&raw)?;

        if beef_bytes.len() < 4 {
            return Err(DmError::wallet(format!(
                "BEEF data too short ({} bytes) for tx {txid}",
                beef_bytes.len()
            )));
        }

        debug!("Fetched BEEF: {} bytes for tx {}", beef_bytes.len(), txid);

        // Step 4: Build AtomicBEEF: [0x01, 0x01, 0x01, 0x01] + reversed_txid(32) + beef_bytes
        let mut atomic_beef = vec![0x01u8, 0x01, 0x01, 0x01];
        let txid_bytes =
            hex::decode(txid).map_err(|e| DmError::wallet(format!("txid decode: {e}")))?;
        let reversed: Vec<u8> = txid_bytes.into_iter().rev().collect();
        atomic_beef.extend_from_slice(&reversed);
        atomic_beef.extend_from_slice(&beef_bytes);

        // Step 5: Internalize with verified derivation parameters
        let outputs = vec![json!({
            "outputIndex": vout,
            "protocol": "wallet payment",
            "paymentRemittance": {
                "derivationPrefix": Self::FUNDING_PREFIX,
                "derivationSuffix": suffix,
                "senderIdentityKey": ANYONE_KEY,
            },
        })];

        debug!(
            "Internalizing tx {} vout {} ({} byte AtomicBEEF, {} sats)",
            txid,
            vout,
            atomic_beef.len(),
            actual_sats,
        );

        let result = self
            .internalize_action(
                &atomic_beef,
                &outputs,
                &format!("Fund from external tx {}", &txid[..16]),
            )
            .await?;

        let accepted = result
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !accepted {
            return Err(DmError::wallet(format!(
                "Wallet rejected internalization of tx {txid}: {result}"
            )));
        }

        Ok(result)
    }

    // ── Split ────────────────────────────────────────────────────
    //
    // UTXO splitting — take all spendable outputs and atomically
    // re-emit them as N equal-sized outputs. Used for parallel-agent
    // funding (e.g. DolphinMilkShake fleet provisioning) where one
    // large deposit needs to become N spendable chunks.
    //
    // MUST match bsv-wallet-cli/src/commands/split.rs exactly. Any
    // drift between the two implementations will produce different
    // on-chain UTXO layouts for the same input state. The hardcoded
    // constants, fee math, basket/tag/label semantics, and the
    // create_action → internalize_action sequence all come from there.
    //
    // Cross-reference: bsv-wallet-cli commit at 2026-04-15 fixed a
    // hardcoded 200-sat fee reserve that failed on 50-output splits.
    // We inherit the fixed dynamic estimator below.

    /// Mirrors `bsv-wallet-cli` split constants. The toolbox's
    /// "wallet payment" internalization protocol expects the
    /// derivation pair to match these exact strings for the outputs
    /// to be promoted to `change=1` and become spendable.
    const SPLIT_DERIVATION_PREFIX: &'static str = "SfKxPIJNgdI=";
    const SPLIT_DERIVATION_SUFFIX: &'static str = "NaGLC6fMH50=";
    const SPLIT_BRC29_PROTOCOL: &'static str = "3241645161d8";

    /// Split the wallet's spendable balance into `count` equal-sized
    /// outputs. Returns `(txid, sats_per_output, count)`.
    ///
    /// Behavior mirrors `bsv-wallet-cli split <count>` exactly:
    /// - Uses `basket: None` on `create_action` (CRITICAL — putting
    ///   the outputs into a basket breaks the `internalize_action`
    ///   change-promotion path).
    /// - Tags each output `["relinquish"]` and labels the tx `["split"]`.
    /// - Self-internalizes via "wallet payment" protocol with the
    ///   fixed base64 derivation pair so the outputs become spendable
    ///   by future `create_action` calls.
    /// - Dynamic fee reserve: `max(inputs*148 + outputs*34 + 10, 500)`
    ///   bytes at 1 sat/byte.
    pub async fn split_utxos(&self, count: u32) -> Result<(String, u64, u32), DmError> {
        if count < 2 {
            return Err(DmError::wallet("Split count must be at least 2"));
        }

        // Step 1: enumerate current spendable outputs.
        let list_args = ListOutputsArgs {
            basket: "default".to_string(),
            tags: None,
            tag_query_mode: None,
            include: None,
            include_custom_instructions: None,
            include_tags: None,
            include_labels: None,
            limit: None,
            offset: None,
            seek_permission: None,
        };
        let list = self
            .wallet
            .list_outputs(list_args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("split list_outputs: {e}")))?;

        let total_sats: u64 = list.outputs.iter().map(|o| o.satoshis).sum();
        let utxo_count = list.outputs.len();

        if utxo_count == 0 || total_sats == 0 {
            return Err(DmError::wallet(format!(
                "No UTXOs to split (balance: {total_sats} sats, {utxo_count} UTXOs)"
            )));
        }

        // Step 2: dynamic fee reserve. See bsv-wallet-cli commit notes
        // (2026-04-15) for why a hardcoded 200 sats is insufficient.
        //   inputs:  ~148 bytes each (sig + pubkey + outpoint)
        //   outputs:  ~34 bytes each (value + P2PKH script)
        //   overhead: ~10 bytes (version + locktime + count varints)
        let estimated_tx_bytes: u64 = (utxo_count as u64 * 148) + (count as u64 * 34) + 10;
        let fee_reserve: u64 = estimated_tx_bytes.max(500);
        if total_sats <= fee_reserve {
            return Err(DmError::wallet(format!(
                "Balance too low to split ({total_sats} sats, need > {fee_reserve} for fees)"
            )));
        }
        let available = total_sats - fee_reserve;
        let per_output = available / count as u64;
        if per_output < 1 {
            return Err(DmError::wallet(format!(
                "Cannot create {count} outputs from {available} available sats"
            )));
        }

        // Step 3: derive the wallet's own P2PKH locking script via
        // the SAME derivation path bsv-wallet-cli uses, so the
        // subsequent internalize_action can sign for these outputs.
        let protocol_id = json!([2, Self::SPLIT_BRC29_PROTOCOL]);
        let key_id = format!(
            "{} {}",
            Self::SPLIT_DERIVATION_PREFIX,
            Self::SPLIT_DERIVATION_SUFFIX
        );
        let derived_pubkey_hex = self
            .get_public_key(&protocol_id, &key_id, ANYONE_KEY, true)
            .await?;
        let derived_pubkey = PublicKey::from_hex(&derived_pubkey_hex)
            .map_err(|e| DmError::wallet(format!("split decode pubkey: {e}")))?;
        let address = derived_pubkey.to_address();
        let lock = P2PKH::lock_from_address(&address)
            .map_err(|e| DmError::wallet(format!("split build P2PKH: {e}")))?;
        let lock_bytes = lock.to_binary();

        // Step 4: build N outputs with basket: None (CRITICAL — see
        // module-level comment for why a basket breaks the internalize
        // merge path).
        let outputs: Vec<CreateActionOutput> = (0..count)
            .map(|_| CreateActionOutput {
                locking_script: lock_bytes.clone(),
                satoshis: per_output,
                output_description: "split output".to_string(),
                basket: None,
                custom_instructions: None,
                tags: Some(vec!["relinquish".to_string()]),
            })
            .collect();

        let args = CreateActionArgs {
            description: format!("Split into {count} UTXOs ({per_output} sats each)"),
            input_beef: None,
            inputs: Some(vec![]),
            outputs: Some(outputs),
            lock_time: None,
            version: None,
            labels: Some(vec!["split".to_string()]),
            options: Some(CreateActionOptions {
                randomize_outputs: Some(false),
                sign_and_process: Some(true),
                no_send: Some(false),
                ..Default::default()
            }),
        };

        // Serialize with spending_lock to avoid UTXO contention with
        // other in-flight wallet ops.
        let _guard = self.spending_lock.lock().await;
        let result = self
            .wallet
            .create_action(args, ORIGINATOR)
            .await
            .map_err(|e| DmError::wallet(format!("split create_action: {e}")))?;
        drop(_guard);

        let txid = result
            .txid
            .ok_or_else(|| DmError::wallet("split: create_action returned no txid"))?;
        let txid_hex = to_hex(&txid);

        // Step 5: self-internalize with "wallet payment" protocol so
        // the new outputs become change=1 (spendable by the coin
        // selector on subsequent create_action calls).
        let (_, anyone_pubkey) = bsv::wallet::KeyDeriver::anyone_key();
        let sender_identity_key = anyone_pubkey.to_hex();

        if let Some(beef_bytes) = &result.beef {
            if let Ok(mut beef) = Beef::from_binary(beef_bytes) {
                if let Ok(atomic_bytes) = beef.to_binary_atomic(&txid_hex) {
                    let internalize_outputs: Vec<InternalizeOutput> = (0..count)
                        .map(|i| InternalizeOutput {
                            output_index: i,
                            protocol: "wallet payment".to_string(),
                            payment_remittance: Some(WalletPayment {
                                derivation_prefix: Self::SPLIT_DERIVATION_PREFIX.to_string(),
                                derivation_suffix: Self::SPLIT_DERIVATION_SUFFIX.to_string(),
                                sender_identity_key: sender_identity_key.clone(),
                            }),
                            insertion_remittance: None,
                        })
                        .collect();

                    self.wallet
                        .internalize_action(
                            InternalizeActionArgs {
                                tx: atomic_bytes,
                                outputs: internalize_outputs,
                                description: "Self-internalize split outputs".to_string(),
                                labels: Some(vec!["split".to_string()]),
                                seek_permission: None,
                            },
                            ORIGINATOR,
                        )
                        .await
                        .map_err(|e| DmError::wallet(format!("split internalize_action: {e}")))?;
                }
            }
        }

        Ok((txid_hex, per_output, count))
    }
}

// ── WalletBackend trait impl (delegates to inherent methods) ─────────

#[async_trait]
impl WalletBackend for EmbeddedWalletClient {
    async fn get_identity_key(&self) -> Result<String, DmError> {
        EmbeddedWalletClient::get_identity_key(self).await
    }

    async fn get_public_key(
        &self,
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
        for_self: bool,
    ) -> Result<String, DmError> {
        EmbeddedWalletClient::get_public_key(self, protocol_id, key_id, counterparty, for_self)
            .await
    }

    async fn raw_call(&self, method: &str, params: Option<Value>) -> Result<Value, DmError> {
        EmbeddedWalletClient::raw_call(self, method, params).await
    }

    async fn create_action(
        &self,
        outputs: &[Value],
        description: &str,
        accept_delayed_broadcast: bool,
        randomize_outputs: bool,
    ) -> Result<CreateActionResult, DmError> {
        EmbeddedWalletClient::create_action(
            self,
            outputs,
            description,
            accept_delayed_broadcast,
            randomize_outputs,
        )
        .await
    }

    async fn spend_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
        description: &str,
    ) -> Result<CreateActionResult, DmError> {
        EmbeddedWalletClient::spend_output(self, basket, txid, vout, description).await
    }

    async fn internalize_action(
        &self,
        tx_bytes: &[u8],
        outputs: &[Value],
        description: &str,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::internalize_action(self, tx_bytes, outputs, description).await
    }

    async fn split_utxos(&self, count: u32) -> Result<(String, u64, u32), DmError> {
        EmbeddedWalletClient::split_utxos(self, count).await
    }

    async fn get_balance(&self) -> Result<u64, DmError> {
        EmbeddedWalletClient::get_balance(self).await
    }

    async fn get_balance_and_count(&self) -> Result<(u64, u64), DmError> {
        EmbeddedWalletClient::get_balance_and_count(self).await
    }

    async fn list_outputs(
        &self,
        basket: &str,
        include: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::list_outputs(self, basket, include, limit, offset).await
    }

    async fn relinquish_output(
        &self,
        basket: &str,
        txid: &str,
        vout: u32,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::relinquish_output(self, basket, txid, vout).await
    }

    async fn create_signature(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        EmbeddedWalletClient::create_signature(self, data, protocol_id, key_id, counterparty).await
    }

    async fn verify_signature(
        &self,
        data: &[u8],
        signature: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        EmbeddedWalletClient::verify_signature(
            self,
            data,
            signature,
            protocol_id,
            key_id,
            counterparty,
        )
        .await
    }

    async fn encrypt(
        &self,
        plaintext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        EmbeddedWalletClient::encrypt(self, plaintext, protocol_id, key_id, counterparty).await
    }

    async fn decrypt(
        &self,
        ciphertext: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        EmbeddedWalletClient::decrypt(self, ciphertext, protocol_id, key_id, counterparty).await
    }

    async fn create_hmac(
        &self,
        data: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<Vec<u8>, DmError> {
        EmbeddedWalletClient::create_hmac(self, data, protocol_id, key_id, counterparty).await
    }

    async fn verify_hmac(
        &self,
        data: &[u8],
        hmac: &[u8],
        protocol_id: &Value,
        key_id: &str,
        counterparty: &str,
    ) -> Result<bool, DmError> {
        EmbeddedWalletClient::verify_hmac(self, data, hmac, protocol_id, key_id, counterparty).await
    }

    async fn acquire_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        EmbeddedWalletClient::acquire_certificate(self, certificate).await
    }

    async fn list_certificates(
        &self,
        certifiers: &[&str],
        types: &[&str],
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::list_certificates(self, certifiers, types, limit, offset).await
    }

    async fn prove_certificate(
        &self,
        certificate: &Value,
        fields_to_reveal: &[&str],
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::prove_certificate(self, certificate, fields_to_reveal).await
    }

    async fn relinquish_certificate(&self, certificate: &Value) -> Result<Value, DmError> {
        EmbeddedWalletClient::relinquish_certificate(self, certificate).await
    }

    async fn discover_by_identity_key(
        &self,
        identity_key: &str,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::discover_by_identity_key(self, identity_key, cert_type, limit).await
    }

    async fn discover_by_attributes(
        &self,
        attributes: &Value,
        cert_type: Option<&str>,
        limit: u64,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::discover_by_attributes(self, attributes, cert_type, limit).await
    }

    async fn reveal_counterparty_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::reveal_counterparty_key_linkage(
            self,
            counterparty,
            verifier,
            privileged,
        )
        .await
    }

    async fn reveal_specific_key_linkage(
        &self,
        counterparty: &str,
        verifier: &str,
        protocol_id: &Value,
        key_id: &str,
        privileged: bool,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::reveal_specific_key_linkage(
            self,
            counterparty,
            verifier,
            protocol_id,
            key_id,
            privileged,
        )
        .await
    }

    async fn is_authenticated(&self) -> Result<Value, DmError> {
        EmbeddedWalletClient::is_authenticated(self).await
    }

    async fn get_height(&self) -> Result<u64, DmError> {
        EmbeddedWalletClient::get_height(self).await
    }

    async fn get_network(&self) -> Result<String, DmError> {
        EmbeddedWalletClient::get_network(self).await
    }

    async fn get_version(&self) -> Result<String, DmError> {
        EmbeddedWalletClient::get_version(self).await
    }

    async fn wait_for_authentication(&self) -> Result<Value, DmError> {
        EmbeddedWalletClient::wait_for_authentication(self).await
    }

    async fn get_header_for_height(&self, height: u64) -> Result<String, DmError> {
        EmbeddedWalletClient::get_header_for_height(self, height).await
    }

    async fn sign_action(&self, reference: &str) -> Result<Value, DmError> {
        EmbeddedWalletClient::sign_action(self, reference).await
    }

    async fn abort_action(&self, reference: &str) -> Result<Value, DmError> {
        EmbeddedWalletClient::abort_action(self, reference).await
    }

    async fn list_actions(
        &self,
        labels: &[&str],
        include_labels: bool,
        include_inputs: bool,
        include_outputs: bool,
        limit: u64,
        offset: u64,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::list_actions(
            self,
            labels,
            include_labels,
            include_inputs,
            include_outputs,
            limit,
            offset,
        )
        .await
    }

    async fn receive_address(&self, suffix: &str) -> Result<(String, String, String), DmError> {
        EmbeddedWalletClient::receive_address(self, suffix).await
    }

    async fn fund_from_woc(
        &self,
        txid: &str,
        vout: Option<u32>,
        derivation_suffix: Option<&str>,
    ) -> Result<Value, DmError> {
        EmbeddedWalletClient::fund_from_woc(self, txid, vout, derivation_suffix).await
    }
}

// ── Helper: read root key from .env file ────────────────────────────

/// Read `SERVER_PRIVATE_KEY=<hex>` from a `.env` file.
fn read_root_key_from_env_file(path: &std::path::Path) -> Result<String, DmError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        DmError::wallet(format!(
            "Cannot read root key from {}: {e}\n\n  \
             Set SERVER_PRIVATE_KEY env var or run `dolphin-milk init`.",
            path.display()
        ))
    })?;

    for line in content.lines() {
        let line = line.trim();
        let val = line
            .strip_prefix("ROOT_KEY=")
            .or_else(|| line.strip_prefix("SERVER_PRIVATE_KEY="));
        if let Some(val) = val {
            let key = val.trim().trim_matches('"');
            if !key.is_empty() {
                return Ok(key.to_string());
            }
        }
    }

    Err(DmError::wallet(format!(
        "No SERVER_PRIVATE_KEY found in {}\n\n  Run `dolphin-milk init` to generate one.",
        path.display()
    )))
}
