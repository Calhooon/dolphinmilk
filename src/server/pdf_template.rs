//! HTML template for PDF export of audit trails and budget reports.
//!
//! Generates styled HTML from audit data that can be rendered to PDF via
//! chromiumoxide's Page::pdf() CDP command. All user-generated content is
//! HTML-escaped to prevent XSS.

use serde_json::Value;

// =============================================================================
// HTML escaping — XSS-safe
// =============================================================================

/// Escape a string for safe inclusion in HTML content.
/// Handles the 5 critical characters: & < > " '
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

// =============================================================================
// Audit trail HTML template
// =============================================================================

/// Render audit events, proofs, and summary as a styled HTML document.
///
/// Parameters:
/// - `task_id`: the task identifier
/// - `events`: flattened audit event objects (same as JSON export)
/// - `proofs`: proof objects with txid, proof_type, hash, etc.
/// - `summary`: summary object with total_events, iterations, sats_spent, etc.
pub fn render_audit_html(
    task_id: &str,
    events: &[Value],
    proofs: &[Value],
    summary: &Value,
) -> String {
    let total_events = summary
        .get("total_events")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let iterations = summary
        .get("iterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let sats_spent = summary
        .get("sats_spent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let proof_count = summary
        .get("proof_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let tool_calls = summary
        .get("tool_calls")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let duration_secs = summary
        .get("duration_secs")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let task_id_escaped = html_escape(task_id);

    let mut html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Audit Trail — Task {task_id_escaped}</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; margin: 40px; color: #1a1a1a; font-size: 12px; line-height: 1.5; }}
  h1 {{ font-size: 22px; margin-bottom: 4px; }}
  h2 {{ font-size: 16px; margin-top: 28px; margin-bottom: 8px; border-bottom: 1px solid #ddd; padding-bottom: 4px; }}
  .meta {{ color: #666; font-size: 11px; margin-bottom: 20px; }}
  .summary {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin-bottom: 24px; }}
  .stat {{ background: #f5f5f5; padding: 12px; border-radius: 6px; }}
  .stat .label {{ font-size: 10px; text-transform: uppercase; color: #888; }}
  .stat .value {{ font-size: 18px; font-weight: 600; }}
  table {{ width: 100%; border-collapse: collapse; font-size: 11px; margin-bottom: 20px; }}
  th {{ text-align: left; background: #f0f0f0; padding: 6px 8px; border-bottom: 2px solid #ccc; }}
  td {{ padding: 5px 8px; border-bottom: 1px solid #eee; vertical-align: top; }}
  tr:nth-child(even) {{ background: #fafafa; }}
  .event-type {{ font-weight: 600; white-space: nowrap; }}
  .event-think_request, .event-think_response {{ color: #2563eb; }}
  .event-tool_call, .event-tool_result {{ color: #059669; }}
  .event-proof_created, .event-checkpoint_created {{ color: #7c3aed; }}
  .event-error {{ color: #dc2626; }}
  .txid {{ font-family: monospace; font-size: 10px; word-break: break-all; }}
  .proof-card {{ background: #f8f0ff; border: 1px solid #e0d0f0; border-radius: 6px; padding: 10px; margin-bottom: 8px; }}
  .proof-card .type {{ font-weight: 600; color: #7c3aed; }}
  .proof-card .hash {{ font-family: monospace; font-size: 10px; color: #555; word-break: break-all; }}
  .footer {{ margin-top: 30px; padding-top: 10px; border-top: 1px solid #ddd; color: #999; font-size: 10px; }}
  .detail {{ max-width: 300px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
</style>
</head>
<body>
<h1>Audit Trail</h1>
<p class="meta">Task: <code>{task_id_escaped}</code></p>

<h2>Summary</h2>
<div class="summary">
  <div class="stat"><div class="label">Iterations</div><div class="value">{iterations}</div></div>
  <div class="stat"><div class="label">Total Events</div><div class="value">{total_events}</div></div>
  <div class="stat"><div class="label">Sats Spent</div><div class="value">{sats_spent}</div></div>
  <div class="stat"><div class="label">Tool Calls</div><div class="value">{tool_calls}</div></div>
  <div class="stat"><div class="label">Proofs</div><div class="value">{proof_count}</div></div>
  <div class="stat"><div class="label">Duration</div><div class="value">{duration_secs:.1}s</div></div>
</div>
"#
    );

    // Proofs section
    if !proofs.is_empty() {
        html.push_str("<h2>On-Chain Proofs</h2>\n");
        for proof in proofs {
            let txid = proof.get("txid").and_then(|v| v.as_str()).unwrap_or("");
            let proof_type = proof
                .get("proof_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let hash = proof.get("hash").and_then(|v| v.as_str()).unwrap_or("");
            let iteration = proof.get("iteration").and_then(|v| v.as_u64());

            let iter_str = match iteration {
                Some(i) => format!(" (iteration {i})"),
                None => String::new(),
            };

            html.push_str(&format!(
                r#"<div class="proof-card">
  <div class="type">{}{}</div>
  <div>TxID: <span class="txid">{}</span></div>
  <div>Hash: <span class="hash">{}</span></div>
</div>
"#,
                html_escape(proof_type),
                html_escape(&iter_str),
                html_escape(txid),
                html_escape(hash),
            ));
        }
    }

    // Events timeline table
    html.push_str(r#"<h2>Event Timeline</h2>
<table>
<thead>
  <tr><th>#</th><th>Iter</th><th>Type</th><th>Model / Tool</th><th>Sats</th><th>Proof TxID</th><th>Detail</th></tr>
</thead>
<tbody>
"#);

    for (i, ev) in events.iter().enumerate() {
        let etype = ev.get("event_type").and_then(|v| v.as_str()).unwrap_or("");
        let iteration = ev.get("iteration").and_then(|v| v.as_u64()).unwrap_or(0);
        let model = ev.get("model").and_then(|v| v.as_str()).unwrap_or("");
        let tool_name = ev.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let sats = ev.get("sats_spent").and_then(|v| v.as_u64()).unwrap_or(0);
        let proof_txid = ev.get("proof_txid").and_then(|v| v.as_str()).unwrap_or("");
        let detail = ev.get("detail").and_then(|v| v.as_str()).unwrap_or("");

        let model_or_tool = if !tool_name.is_empty() {
            tool_name
        } else {
            model
        };

        let sats_str = if sats > 0 {
            format!("{sats}")
        } else {
            String::new()
        };

        let txid_short = if proof_txid.len() > 12 {
            format!("{}...", &proof_txid[..12])
        } else {
            proof_txid.to_string()
        };

        html.push_str(&format!(
            r#"  <tr><td>{}</td><td>{}</td><td class="event-type event-{}">{}</td><td>{}</td><td>{}</td><td class="txid" title="{}">{}</td><td class="detail" title="{}">{}</td></tr>
"#,
            i + 1,
            iteration,
            html_escape(etype),
            html_escape(etype),
            html_escape(model_or_tool),
            html_escape(&sats_str),
            html_escape(proof_txid),
            html_escape(&txid_short),
            html_escape(detail),
            html_escape(detail),
        ));
    }

    html.push_str("</tbody>\n</table>\n");

    // Footer
    html.push_str(
        r#"<div class="footer">
  Generated by Dolphin Milk audit export. All events are backed by BRC-18 on-chain proofs.
</div>
</body>
</html>"#,
    );

    html
}

// =============================================================================
// Budget report HTML template
// =============================================================================

/// Render a budget report as styled HTML for PDF export.
///
/// `detail` should be a `BudgetDetailResponse`-shaped JSON value with
/// `entries`, `by_service`, and `total_sats` fields.
pub fn render_budget_html(detail: &Value) -> String {
    let total_sats = detail
        .get("total_sats")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let mut html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Budget Report</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; margin: 40px; color: #1a1a1a; font-size: 12px; line-height: 1.5; }}
  h1 {{ font-size: 22px; margin-bottom: 4px; }}
  h2 {{ font-size: 16px; margin-top: 28px; margin-bottom: 8px; border-bottom: 1px solid #ddd; padding-bottom: 4px; }}
  .total {{ font-size: 28px; font-weight: 700; color: #2563eb; margin: 16px 0; }}
  table {{ width: 100%; border-collapse: collapse; font-size: 11px; margin-bottom: 20px; }}
  th {{ text-align: left; background: #f0f0f0; padding: 6px 8px; border-bottom: 2px solid #ccc; }}
  td {{ padding: 5px 8px; border-bottom: 1px solid #eee; vertical-align: top; }}
  tr:nth-child(even) {{ background: #fafafa; }}
  .service-header {{ background: #e8f0fe; font-weight: 600; }}
  .footer {{ margin-top: 30px; padding-top: 10px; border-top: 1px solid #ddd; color: #999; font-size: 10px; }}
</style>
</head>
<body>
<h1>Budget Report</h1>
<div class="total">{total_sats} sats</div>
"#
    );

    // By-service breakdown
    if let Some(by_service) = detail.get("by_service").and_then(|v| v.as_object()) {
        html.push_str("<h2>Spending by Service</h2>\n<table>\n<thead><tr><th>Service</th><th>Operation</th><th>Count</th><th>Total Sats</th><th>Avg Sats</th></tr></thead>\n<tbody>\n");

        for (service, breakdown) in by_service {
            let svc_total = breakdown
                .get("total_sats")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            html.push_str(&format!(
                r#"  <tr class="service-header"><td colspan="2">{}</td><td></td><td>{}</td><td></td></tr>
"#,
                html_escape(service),
                svc_total,
            ));

            if let Some(ops) = breakdown.get("operations").and_then(|v| v.as_object()) {
                for (op, stats) in ops {
                    let count = stats.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let op_total = stats
                        .get("total_sats")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let avg = stats.get("avg_sats").and_then(|v| v.as_u64()).unwrap_or(0);
                    html.push_str(&format!(
                        "  <tr><td></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                        html_escape(op),
                        count,
                        op_total,
                        avg,
                    ));
                }
            }
        }

        html.push_str("</tbody>\n</table>\n");
    }

    // Recent entries table
    if let Some(entries) = detail.get("entries").and_then(|v| v.as_array()) {
        html.push_str("<h2>Recent Spending Entries</h2>\n<table>\n<thead><tr><th>Timestamp</th><th>Service</th><th>Operation</th><th>Sats</th></tr></thead>\n<tbody>\n");

        // Show last 100 entries at most
        let start = if entries.len() > 100 {
            entries.len() - 100
        } else {
            0
        };
        for entry in &entries[start..] {
            let ts = entry
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let service = entry.get("service").and_then(|v| v.as_str()).unwrap_or("");
            let operation = entry
                .get("operation")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sats = entry.get("sats").and_then(|v| v.as_u64()).unwrap_or(0);
            html.push_str(&format!(
                "  <tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                html_escape(ts),
                html_escape(service),
                html_escape(operation),
                sats,
            ));
        }

        html.push_str("</tbody>\n</table>\n");
    }

    html.push_str(
        r#"<div class="footer">
  Generated by Dolphin Milk budget export.
</div>
</body>
</html>"#,
    );

    html
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_html_escape_basic() {
        assert_eq!(html_escape("hello"), "hello");
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a&b"), "a&amp;b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(html_escape("it's"), "it&#x27;s");
    }

    #[test]
    fn test_html_escape_all_chars() {
        let input = "<script>alert('xss')&\"test\"</script>";
        let escaped = html_escape(input);
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('>'));
        assert!(!escaped.contains('\''));
        assert!(escaped.contains("&lt;"));
        assert!(escaped.contains("&gt;"));
        assert!(escaped.contains("&#x27;"));
        assert!(escaped.contains("&amp;"));
        assert!(escaped.contains("&quot;"));
    }

    #[test]
    fn test_html_escape_empty() {
        assert_eq!(html_escape(""), "");
    }

    #[test]
    fn test_render_audit_html_basic() {
        let events = vec![json!({
            "timestamp": 1700000000.0,
            "iteration": 1,
            "event_type": "think_request",
            "model": "gpt-5-mini",
            "tool_name": "",
            "sats_spent": 0,
            "proof_txid": "",
            "detail": "model=gpt-5-mini"
        })];
        let proofs = vec![json!({
            "txid": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
            "proof_type": "TaskCompletion",
            "hash": "deadbeef",
            "iteration": 2,
        })];
        let summary = json!({
            "total_events": 5,
            "iterations": 2,
            "sats_spent": 300,
            "proof_count": 1,
            "tool_calls": 1,
            "duration_secs": 10.5,
        });

        let html = render_audit_html("test-task-001", &events, &proofs, &summary);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("test-task-001"));
        assert!(html.contains("300")); // sats_spent
        assert!(html.contains("TaskCompletion"));
        assert!(html.contains("abcd1234abcd")); // truncated txid
        assert!(html.contains("deadbeef")); // hash
        assert!(html.contains("think_request"));
        assert!(html.contains("gpt-5-mini"));
    }

    #[test]
    fn test_render_audit_html_escapes_user_content() {
        let events = vec![json!({
            "timestamp": 1700000000.0,
            "iteration": 1,
            "event_type": "tool_call",
            "model": "",
            "tool_name": "execute_bash",
            "sats_spent": 0,
            "proof_txid": "",
            "detail": "<script>alert('xss')</script>"
        })];
        let summary = json!({
            "total_events": 1, "iterations": 0, "sats_spent": 0,
            "proof_count": 0, "tool_calls": 1, "duration_secs": 0.0,
        });

        let html = render_audit_html("<script>bad</script>", &events, &[], &summary);

        // Task ID should be escaped
        assert!(html.contains("&lt;script&gt;bad&lt;/script&gt;"));
        // Detail should be escaped
        assert!(html.contains("&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"));
        // No raw script tags
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn test_render_budget_html_basic() {
        let detail = json!({
            "total_sats": 5000,
            "by_service": {
                "llm": {
                    "total_sats": 4000,
                    "operations": {
                        "inference": { "count": 10, "total_sats": 4000, "avg_sats": 400 }
                    }
                }
            },
            "entries": [
                { "timestamp": "2026-03-01T12:00:00Z", "service": "llm", "operation": "inference", "sats": 400 }
            ]
        });

        let html = render_budget_html(&detail);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("5000 sats"));
        assert!(html.contains("llm"));
        assert!(html.contains("inference"));
        assert!(html.contains("400"));
    }
}
