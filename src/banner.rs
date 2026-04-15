//! Shared CLI banner, startup display, and error helpers.

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Format sats with comma separators: 45230 → "45,230"
pub fn format_sats_human(sats: u64) -> String {
    let s = sats.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

/// Truncate a hex key for display: "02a1b2c3...f6"
pub fn truncate_key(key: &str) -> String {
    if key.len() > 16 {
        format!("{}...{}", &key[..10], &key[key.len() - 4..])
    } else {
        key.to_string()
    }
}

/// Print the startup banner shown by `dolphin-milk start`.
pub fn print_startup_banner(
    port: u16,
    identity: &str,
    balance: Option<u64>,
    model: &str,
    wallet_url: &str,
) {
    println!();
    println!("  Dolphin Milk v{VERSION}");
    println!("  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!("  Wallet:    {wallet_url} (connected)");
    println!("  Identity:  {identity}");
    if let Some(b) = balance {
        println!("  Balance:   {} sats", format_sats_human(b));
    }
    println!("  Model:     {model}");
    println!("  Port:      {port}");
    println!();
    println!("  Web UI:    http://localhost:{port}/ui/");
    println!("  API:       http://localhost:{port}/health");
    println!("  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!();
}

/// Print the getting-started message shown when no subcommand is given.
pub fn print_getting_started() {
    println!();
    println!("  Dolphin Milk v{VERSION}");
    println!("  The AI agent that pays for its own inference.");
    println!();
    println!("  Get started:");
    println!("    dolphin-milk init           Set up wallet + config");
    println!("    dolphin-milk think MESSAGE  Single LLM call (~200 sats)");
    println!("    dolphin-milk start          Start the web UI + daemon");
    println!();
    println!("  Full reference: dolphin-milk --help");
    println!();
}

/// Print getting-started with wallet status (after init has been run).
pub fn print_status_guide(identity: &str, balance: Option<u64>) {
    let funded = balance.is_some_and(|b| b > 0);

    println!();
    println!("  Dolphin Milk v{VERSION}");
    println!();
    println!("  Identity:  {}", truncate_key(identity));
    if let Some(b) = balance {
        println!("  Balance:   {} sats", format_sats_human(b));
    }
    println!();

    if funded {
        println!("  Commands:");
        println!("    dolphin-milk start          Start the web UI + daemon");
        println!("    dolphin-milk think MESSAGE  Single LLM call (~200 sats)");
        println!("    dolphin-milk run TASK       Run an autonomous task");
    } else {
        println!("  Your wallet is empty. Fund it to get started:");
        println!("    dolphin-milk init           Show funding address");
        println!("    dolphin-milk start          Start the web UI (browse while you wait)");
    }
    println!();
}

/// Print an actionable wallet-not-reachable error.
pub fn print_wallet_error(wallet_url: &str) {
    eprintln!();
    eprintln!("  ERROR: Wallet not reachable at {wallet_url}");
    eprintln!();
    eprintln!("  The agent needs bsv-wallet-cli running to function.");
    eprintln!("  Start it with:");
    eprintln!();
    eprintln!("    bsv-wallet daemon");
    eprintln!();
    eprintln!("  Or check your wallet URL in dolphin-milk.toml:");
    eprintln!("    [wallet]");
    eprintln!("    url = \"{wallet_url}\"");
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_sats_human() {
        assert_eq!(format_sats_human(0), "0");
        assert_eq!(format_sats_human(999), "999");
        assert_eq!(format_sats_human(1000), "1,000");
        assert_eq!(format_sats_human(1234567), "1,234,567");
        assert_eq!(format_sats_human(45230), "45,230");
    }

    #[test]
    fn test_truncate_key() {
        let key = "02a1b2c3d4e5f67890abcdef1234567890abcdef1234567890abcdef12345678ab";
        assert_eq!(truncate_key(key), "02a1b2c3d4...78ab");
        assert_eq!(truncate_key("short"), "short");
    }
}
