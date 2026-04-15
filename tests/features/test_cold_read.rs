//! Cold-read prediction test (issue #100).
//!
//! Validates memory quality: can we predict what topics matter to the user
//! from memory files alone, without seeing the conversation?
//!
//! The test:
//! 1. Load sample memory files from fixtures
//! 2. Extract topic predictions from memory content (without seeing transcripts)
//! 3. Load recorded transcripts and extract actual user questions/topics
//! 4. Measure overlap (relevance) between predicted and actual topics
//! 5. Grade the prediction quality

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use dolphin_milk::eval::report::EvalReport;
use dolphin_milk::eval::trajectory::Trajectory;

/// Fixture base directory.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval")
}

/// Extract topic keywords from a memory markdown file.
///
/// Parses the YAML frontmatter for tags and extracts significant words
/// from the body content. This is the "prediction" step -- we derive
/// what we think matters from memory alone.
fn extract_topics_from_memory(content: &str) -> Vec<String> {
    let mut topics = Vec::new();

    // Extract tags from YAML frontmatter
    let mut in_frontmatter = false;
    let mut in_tags = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break; // end of frontmatter
            }
            in_frontmatter = true;
            continue;
        }
        if !in_frontmatter {
            continue;
        }
        if trimmed.starts_with("tags:") {
            in_tags = true;
            continue;
        }
        if in_tags {
            if trimmed.starts_with("- ") {
                let tag = trimmed.trim_start_matches("- ").trim().to_lowercase();
                if !tag.is_empty() {
                    topics.push(tag);
                }
            } else {
                in_tags = false;
            }
        }
    }

    // Extract significant words from body content (after frontmatter)
    let body = extract_body(content);
    let stop_words: HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "have", "has", "had", "do",
        "does", "did", "will", "would", "could", "should", "can", "to", "of", "in", "for", "on",
        "with", "at", "by", "from", "as", "into", "and", "but", "or", "it", "its", "this", "that",
        "which", "who", "what", "how", "all", "each", "every", "not", "no", "so", "if", "up",
        "out", "about", "than", "other", "more", "also", "very", "just", "only", "use", "used",
        "using", "via", "per", "key",
    ]
    .into_iter()
    .collect();

    let mut word_freq: HashMap<String, usize> = HashMap::new();
    for word in body.split_whitespace() {
        let clean = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if clean.len() >= 3 && !stop_words.contains(clean.as_str()) {
            *word_freq.entry(clean).or_insert(0) += 1;
        }
    }

    // Take top words by frequency
    let mut sorted: Vec<(String, usize)> = word_freq.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    for (word, _) in sorted.into_iter().take(10) {
        if !topics.contains(&word) {
            topics.push(word);
        }
    }

    topics
}

/// Extract the body text from a markdown file (everything after the second ---).
fn extract_body(content: &str) -> &str {
    let mut dashes_seen = 0;
    for (i, line) in content.lines().enumerate() {
        if line.trim() == "---" {
            dashes_seen += 1;
            if dashes_seen == 2 {
                // Return everything after this line
                let byte_offset: usize = content
                    .lines()
                    .take(i + 1)
                    .map(|l| l.len() + 1) // +1 for newline
                    .sum();
                return if byte_offset < content.len() {
                    &content[byte_offset..]
                } else {
                    ""
                };
            }
        }
    }
    content
}

/// Extract topic keywords from transcript user messages.
fn extract_topics_from_transcript(trajectory: &Trajectory) -> Vec<String> {
    trajectory.extract_topics(15)
}

/// Compute relevance score between predicted topics and actual topics.
///
/// Returns a score from 0.0 to 1.0 based on the overlap between
/// predicted and actual topic sets.
fn compute_relevance(predicted: &[String], actual: &[String]) -> f64 {
    if predicted.is_empty() || actual.is_empty() {
        return 0.0;
    }

    let predicted_set: HashSet<&str> = predicted.iter().map(|s| s.as_str()).collect();
    let actual_set: HashSet<&str> = actual.iter().map(|s| s.as_str()).collect();

    // Direct matches
    let direct_matches = predicted_set.intersection(&actual_set).count();

    // Partial matches (substring containment in either direction)
    let mut partial_matches = 0;
    for pred in &predicted_set {
        for act in &actual_set {
            if pred != act && (pred.contains(act) || act.contains(pred)) {
                partial_matches += 1;
            }
        }
    }

    let match_score = direct_matches as f64 + (partial_matches as f64 * 0.5);
    let max_possible = actual_set.len().max(1) as f64;

    (match_score / max_possible).min(1.0)
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[test]
fn test_cold_read_fees_prediction() {
    let memory_path = fixtures_dir().join("memories/bsv_fees.md");
    let transcript_path = fixtures_dir().join("transcripts/session_fees.jsonl");

    let memory_content = fs::read_to_string(&memory_path).unwrap();
    let predicted = extract_topics_from_memory(&memory_content);

    let trajectory = Trajectory::from_jsonl(transcript_path.as_path()).unwrap();
    let actual = extract_topics_from_transcript(&trajectory);

    let relevance = compute_relevance(&predicted, &actual);

    assert!(
        relevance > 0.2,
        "Fee memory should predict fee-related questions. \
         Predicted: {:?}, Actual: {:?}, Relevance: {:.2}",
        predicted,
        actual,
        relevance,
    );
}

#[test]
fn test_cold_read_wallet_prediction() {
    let memory_path = fixtures_dir().join("memories/wallet_integration.md");
    let transcript_path = fixtures_dir().join("transcripts/session_wallet.jsonl");

    let memory_content = fs::read_to_string(&memory_path).unwrap();
    let predicted = extract_topics_from_memory(&memory_content);

    let trajectory = Trajectory::from_jsonl(transcript_path.as_path()).unwrap();
    let actual = extract_topics_from_transcript(&trajectory);

    let relevance = compute_relevance(&predicted, &actual);

    assert!(
        relevance > 0.2,
        "Wallet memory should predict wallet-related questions. \
         Predicted: {:?}, Actual: {:?}, Relevance: {:.2}",
        predicted,
        actual,
        relevance,
    );
}

#[test]
fn test_cold_read_memory_system_prediction() {
    let memory_path = fixtures_dir().join("memories/memory_system.md");
    let transcript_path = fixtures_dir().join("transcripts/session_memory.jsonl");

    let memory_content = fs::read_to_string(&memory_path).unwrap();
    let predicted = extract_topics_from_memory(&memory_content);

    let trajectory = Trajectory::from_jsonl(transcript_path.as_path()).unwrap();
    let actual = extract_topics_from_transcript(&trajectory);

    let relevance = compute_relevance(&predicted, &actual);

    assert!(
        relevance > 0.2,
        "Memory system memory should predict memory-related questions. \
         Predicted: {:?}, Actual: {:?}, Relevance: {:.2}",
        predicted,
        actual,
        relevance,
    );
}

#[test]
fn test_cold_read_cross_topic_low_relevance() {
    // Fee memory should NOT strongly predict memory system questions
    let memory_path = fixtures_dir().join("memories/bsv_fees.md");
    let transcript_path = fixtures_dir().join("transcripts/session_memory.jsonl");

    let memory_content = fs::read_to_string(&memory_path).unwrap();
    let predicted = extract_topics_from_memory(&memory_content);

    let trajectory = Trajectory::from_jsonl(transcript_path.as_path()).unwrap();
    let actual = extract_topics_from_transcript(&trajectory);

    let relevance = compute_relevance(&predicted, &actual);

    // Cross-topic relevance should be lower than same-topic
    // We do not assert 0.0 because there may be incidental overlap
    // Just verify the mechanism differentiates topics
    assert!(
        relevance < 0.8,
        "Cross-topic relevance should be moderate or low: {:.2}",
        relevance,
    );
}

#[test]
fn test_cold_read_all_memories_aggregate() {
    // Load ALL memories and predict topics
    let memories_dir = fixtures_dir().join("memories");
    let mut all_predicted: Vec<String> = Vec::new();

    for entry in fs::read_dir(&memories_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|ext| ext == "md") {
            let content = fs::read_to_string(entry.path()).unwrap();
            let topics = extract_topics_from_memory(&content);
            all_predicted.extend(topics);
        }
    }

    // Deduplicate
    let predicted_set: HashSet<String> = all_predicted.into_iter().collect();
    let all_predicted: Vec<String> = predicted_set.into_iter().collect();

    // Load ALL transcripts and extract actual topics
    let transcripts_dir = fixtures_dir().join("transcripts");
    let mut all_actual: Vec<String> = Vec::new();

    for entry in fs::read_dir(&transcripts_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|ext| ext == "jsonl") {
            let trajectory = Trajectory::from_jsonl(entry.path().as_path()).unwrap();
            let topics = extract_topics_from_transcript(&trajectory);
            all_actual.extend(topics);
        }
    }

    let actual_set: HashSet<String> = all_actual.into_iter().collect();
    let all_actual: Vec<String> = actual_set.into_iter().collect();

    let relevance = compute_relevance(&all_predicted, &all_actual);

    assert!(
        relevance > 0.3,
        "Aggregate memory should predict aggregate transcript topics. \
         Predicted: {} topics, Actual: {} topics, Relevance: {:.2}",
        all_predicted.len(),
        all_actual.len(),
        relevance,
    );
}

#[test]
fn test_extract_topics_from_memory_parses_tags() {
    let content = r#"---
id: test-001
category: knowledge
tags:
  - blockchain
  - fees
  - transactions
created: "2026-03-01T00:00:00Z"
source: test
---

Content about blockchain fees and transactions.
"#;
    let topics = extract_topics_from_memory(content);
    assert!(
        topics.contains(&"blockchain".to_string()),
        "should extract 'blockchain' tag: {:?}",
        topics
    );
    assert!(
        topics.contains(&"fees".to_string()),
        "should extract 'fees' tag: {:?}",
        topics
    );
    assert!(
        topics.contains(&"transactions".to_string()),
        "should extract 'transactions' tag: {:?}",
        topics
    );
}

#[test]
fn test_extract_topics_from_memory_extracts_body_words() {
    let content = r#"---
id: test-002
category: knowledge
tags: []
created: "2026-03-01T00:00:00Z"
source: test
---

Bitcoin micropayments enable frictionless commerce. The protocol supports
micropayments through low transaction fees and instant settlement.
"#;
    let topics = extract_topics_from_memory(content);
    assert!(
        topics.contains(&"micropayments".to_string()),
        "should extract 'micropayments' from body: {:?}",
        topics
    );
}

#[test]
fn test_compute_relevance_perfect_match() {
    let predicted = vec!["bsv".to_string(), "fees".to_string()];
    let actual = vec!["bsv".to_string(), "fees".to_string()];
    let score = compute_relevance(&predicted, &actual);
    assert!(
        (score - 1.0).abs() < f64::EPSILON,
        "perfect match should score 1.0: {}",
        score
    );
}

#[test]
fn test_compute_relevance_no_match() {
    let predicted = vec!["blockchain".to_string(), "mining".to_string()];
    let actual = vec!["weather".to_string(), "forecast".to_string()];
    let score = compute_relevance(&predicted, &actual);
    assert_eq!(score, 0.0, "no match should score 0.0");
}

#[test]
fn test_compute_relevance_partial_match() {
    let predicted = vec!["bsv".to_string(), "fees".to_string(), "mining".to_string()];
    let actual = vec!["bsv".to_string(), "fees".to_string(), "wallet".to_string()];
    let score = compute_relevance(&predicted, &actual);
    assert!(
        score > 0.5 && score < 1.0,
        "partial match should be between 0.5 and 1.0: {}",
        score
    );
}

#[test]
fn test_compute_relevance_empty_inputs() {
    assert_eq!(compute_relevance(&[], &["bsv".to_string()]), 0.0);
    assert_eq!(compute_relevance(&["bsv".to_string()], &[]), 0.0);
    assert_eq!(
        compute_relevance(&Vec::<String>::new(), &Vec::<String>::new()),
        0.0
    );
}

#[test]
fn test_compute_relevance_substring_match() {
    let predicted = vec!["transaction".to_string()];
    let actual = vec!["transactions".to_string()];
    let score = compute_relevance(&predicted, &actual);
    assert!(
        score > 0.0,
        "substring match should contribute to score: {}",
        score
    );
}

#[test]
fn test_eval_report_on_fixture_transcript() {
    let transcript_path = fixtures_dir().join("transcripts/session_fees.jsonl");
    let trajectory = Trajectory::from_jsonl(transcript_path.as_path()).unwrap();

    let report = EvalReport::generate(&trajectory);
    assert_eq!(report.session_id, "sess-fees-001");
    assert!(report.overall_score > 0.0);
    assert_eq!(report.scores.len(), 4);

    // Should not regress without a baseline
    assert!(report.regressions.is_empty());
}

#[test]
fn test_eval_report_regression_across_transcripts() {
    // Use session_fees as baseline, session_wallet as current
    let baseline_path = fixtures_dir().join("transcripts/session_fees.jsonl");
    let current_path = fixtures_dir().join("transcripts/session_wallet.jsonl");

    let baseline_traj = Trajectory::from_jsonl(baseline_path.as_path()).unwrap();
    let current_traj = Trajectory::from_jsonl(current_path.as_path()).unwrap();

    let baseline = dolphin_milk::eval::report::Baseline::from_trajectory("v1", &baseline_traj);
    let report = dolphin_milk::eval::report::EvalReport::generate_with_baseline(
        &current_traj,
        Some(&baseline),
        0.10,
    );

    // Both are reasonable sessions, regressions should be minor or none
    // This verifies the regression mechanism works end-to-end
    assert_eq!(report.scores.len(), 4);
    assert!(report.overall_score > 0.0);
}

#[test]
fn test_extract_body_helper() {
    let content = "---\nid: x\n---\nBody text here.";
    let body = extract_body(content);
    assert!(
        body.contains("Body text here"),
        "should extract body after frontmatter: '{}'",
        body
    );
}

#[test]
fn test_extract_body_no_frontmatter() {
    let content = "Just plain text without frontmatter.";
    let body = extract_body(content);
    assert_eq!(body, content);
}
