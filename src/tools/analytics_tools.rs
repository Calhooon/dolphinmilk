//! Cost analysis tool — lets the agent analyze its own spending patterns.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::tools::registry::ToolDef;

pub fn all_analytics_tools(workspace: PathBuf) -> Vec<ToolDef> {
    let ws = workspace;

    vec![ToolDef {
        name: "cost_analysis".to_string(),
        description: "Analyze spending patterns: efficiency metrics, ROI report, provider benchmarks, and cost comparison across models. Returns structured data about task costs, completion rates, and provider performance.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Analysis action: 'summary' (all metrics), 'roi' (ROI report), 'benchmarks' (provider benchmarks), 'compare' (cost comparison for a task)",
                    "enum": ["summary", "roi", "benchmarks", "compare"]
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID for cost comparison (required when action='compare')"
                },
                "alt_model": {
                    "type": "string",
                    "description": "Alternative model for cost comparison (required when action='compare')"
                }
            }
        }),
        execute: Box::new(move |params: Value| {
            let ws = ws.clone();
            Box::pin(async move {
                let action = params
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("summary");

                match action {
                    "summary" => {
                        let analysis = crate::analytics::compute_cost_analysis(&ws);
                        match serde_json::to_string_pretty(&analysis) {
                            Ok(json_str) => json_str,
                            Err(e) => format!("Error serializing analysis: {e}"),
                        }
                    }
                    "roi" => {
                        let roi = crate::analytics::compute_roi(&ws);
                        match serde_json::to_string_pretty(&roi) {
                            Ok(json_str) => json_str,
                            Err(e) => format!("Error serializing ROI: {e}"),
                        }
                    }
                    "benchmarks" => {
                        let mut entries = crate::analytics::load_benchmarks(&ws);
                        if entries.is_empty() {
                            entries =
                                crate::analytics::backfill_benchmarks_from_transcripts(&ws);
                            for entry in &entries {
                                crate::analytics::record_benchmark(&ws, entry);
                            }
                        }
                        let benchmarks = crate::analytics::aggregate_benchmarks(&entries);
                        match serde_json::to_string_pretty(&benchmarks) {
                            Ok(json_str) => json_str,
                            Err(e) => format!("Error serializing benchmarks: {e}"),
                        }
                    }
                    "compare" => {
                        let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => return "Error: task_id is required for compare action".to_string(),
                        };
                        let alt_model = match params.get("alt_model").and_then(|v| v.as_str()) {
                            Some(m) => m,
                            None => return "Error: alt_model is required for compare action".to_string(),
                        };

                        // Build default pricing table
                        let mut pricing = std::collections::HashMap::new();
                        pricing.insert("gpt-5".to_string(), 150.0);
                        pricing.insert("gpt-5-mini".to_string(), 15.0);
                        pricing.insert("gpt-4.1".to_string(), 100.0);
                        pricing.insert("gpt-4.1-mini".to_string(), 20.0);
                        pricing.insert("gpt-4.1-nano".to_string(), 5.0);
                        pricing.insert("o4-mini".to_string(), 60.0);
                        pricing.insert("claude-sonnet-4-20250514".to_string(), 120.0);
                        pricing.insert("claude-haiku-4-5-20251001".to_string(), 30.0);
                        pricing.insert("claude-opus-4-20250514".to_string(), 400.0);

                        match crate::analytics::cost_replay(&ws, task_id, alt_model, &pricing) {
                            Some(comparison) => {
                                match serde_json::to_string_pretty(&comparison) {
                                    Ok(json_str) => json_str,
                                    Err(e) => format!("Error serializing comparison: {e}"),
                                }
                            }
                            None => format!("Error: Task '{task_id}' not found or has no LLM calls"),
                        }
                    }
                    other => format!("Error: unknown action '{other}'. Use 'summary', 'roi', 'benchmarks', or 'compare'"),
                }
            })
        }),
        category: "analytics".to_string(),
        cleanup: None,
    deferred: true,
    always_load: false,
    search_hint: Some("Spending analysis: efficiency, ROI, benchmarks, cost comparison".to_string()),
    }]
}
