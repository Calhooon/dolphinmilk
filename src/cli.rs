//! CLI definition using clap derive.
//!
//! Subcommands: run, status, think, fund

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "dolphin-milk",
    about = "Autonomous AI agent with BSV micropayments"
)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the agent loop
    Run {
        /// Task to accomplish
        task: Option<String>,

        /// Workspace directory
        #[arg(long)]
        workspace: Option<String>,

        /// Max loop iterations
        #[arg(long, default_value = "50")]
        max_iterations: u32,
    },

    /// Show wallet connectivity and balance
    Status,

    /// Send a message to an LLM via x402 micropayment
    Think {
        /// The message to send to the LLM
        message: String,

        /// Model name
        #[arg(long, default_value = "gpt-5-mini")]
        model: String,
    },

    /// Show a BSV address to receive funds
    Receive {
        /// Derivation suffix (use different values for different funding rounds)
        #[arg(long, default_value = "1")]
        suffix: String,
    },

    /// Internalize funding from WhatsOnChain
    Fund {
        /// Transaction ID (64-char hex string)
        txid: String,

        /// Output index to claim (auto-detects if omitted)
        #[arg(long)]
        vout: Option<u32>,

        /// Derivation suffix (must match the one used with 'receive')
        #[arg(long, default_value = "1")]
        suffix: String,
    },

    /// First-run setup — checks wallet, creates config, shows funding address
    Init {
        /// Data directory for Dolphin Milk state
        #[arg(long)]
        data_dir: Option<String>,
    },

    /// Split the wallet's spendable balance into N equal-sized UTXOs.
    ///
    /// Useful for parallel-agent funding: spawn N sub-agents, each claiming
    /// one split output. Matches `bsv-wallet-cli split` behavior exactly.
    Split {
        /// Number of output UTXOs to create (>= 2)
        count: u32,
    },

    /// Start the daemon (HTTP server + scheduler + web UI)
    Start {
        /// Port to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// Workspace directory
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Start the HTTP server daemon (alias for start)
    Serve {
        /// Port to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// Workspace directory
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Start MCP server for Claude Code / Codex integration
    Mcp,

    /// Audit tools for enterprise compliance
    Audit(AuditArgs),

    /// Verify a custody proof offline
    VerifyWork {
        /// Path to custody proof JSON file
        proof_file: String,
    },
}

/// Audit subcommands for enterprise compliance and BRC-69 key linkage.
#[derive(Debug, Args)]
pub struct AuditArgs {
    #[command(subcommand)]
    pub action: AuditAction,
}

/// Available audit actions.
#[derive(Debug, Subcommand)]
pub enum AuditAction {
    /// Reveal key linkage with a counterparty (BRC-69/70)
    RevealLinkage {
        /// Counterparty identity key (66-char hex compressed public key)
        counterparty: String,
        /// Verifier identity key (defaults to counterparty if not provided)
        #[arg(long)]
        verifier: Option<String>,
        /// Protocol ID for specific revelation (e.g., "dolphin milk message signature")
        #[arg(long)]
        protocol: Option<String>,
        /// Key ID for specific revelation
        #[arg(long)]
        key_id: Option<String>,
        /// Workspace directory
        #[arg(long)]
        workspace: Option<String>,
    },
    /// List past key linkage revelations
    ListRevelations {
        /// Workspace directory
        #[arg(long)]
        workspace: Option<String>,
    },
}
