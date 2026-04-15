#!/usr/bin/env node
/**
 * Report generator for multi-worm graded results (#236).
 *
 * Generates markdown reports from grading results, tracks cost over time,
 * detects regressions against baselines, and produces exportable demo artifacts.
 *
 * Usage:
 *   node report.js                             # Report from latest graded result
 *   node report.js --graded path.json          # Report from specific graded file
 *   node report.js --baseline baselines.json   # Compare against baseline
 *
 * Output: artifacts/report-{timestamp}.md + artifacts/demo-{timestamp}.json
 */

const fs = require('fs');
const path = require('path');

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const RESULTS_DIR = path.join(__dirname, 'results');
const ARTIFACTS_DIR = path.join(__dirname, 'artifacts');
const BASELINES_PATH = path.join(__dirname, 'baselines.json');

// ---------------------------------------------------------------------------
// File loading
// ---------------------------------------------------------------------------

function loadLatestGraded() {
  const files = fs.readdirSync(RESULTS_DIR)
    .filter(f => f.startsWith('graded-') && f.endsWith('.json'))
    .sort()
    .reverse();

  if (files.length === 0) throw new Error(`No graded result files in ${RESULTS_DIR}`);
  return path.join(RESULTS_DIR, files[0]);
}

function loadBaselines(baselinePath) {
  if (!baselinePath) baselinePath = BASELINES_PATH;
  if (!fs.existsSync(baselinePath)) return null;
  return JSON.parse(fs.readFileSync(baselinePath, 'utf8'));
}

// ---------------------------------------------------------------------------
// Regression detection
// ---------------------------------------------------------------------------

function detectRegressions(graded, baselines) {
  if (!baselines || !baselines.scenarios) return [];

  const regressions = [];

  for (const scenario of graded.scenarios) {
    if (scenario.skipped) continue;

    const baseline = baselines.scenarios.find(b => b.id === scenario.id);
    if (!baseline) continue;

    // Check overall verdict regression
    if (baseline.overall_verdict === 'pass' && scenario.overall_verdict !== 'pass') {
      regressions.push({
        scenario_id: scenario.id,
        scenario_name: scenario.name,
        type: 'verdict_regression',
        baseline: baseline.overall_verdict,
        current: scenario.overall_verdict,
        message: `[${scenario.id}] ${scenario.name}: was ${baseline.overall_verdict}, now ${scenario.overall_verdict}`,
      });
    }

    // Check per-criterion regressions
    for (const criterion of scenario.criteria || []) {
      const blCriterion = (baseline.criteria || []).find(c => c.name === criterion.name);
      if (!blCriterion) continue;

      if (blCriterion.verdict === 'pass' && criterion.verdict !== 'pass') {
        regressions.push({
          scenario_id: scenario.id,
          scenario_name: scenario.name,
          criterion: criterion.name,
          type: 'criterion_regression',
          baseline: blCriterion.verdict,
          current: criterion.verdict,
          message: `[${scenario.id}] ${scenario.name} / ${criterion.name}: was ${blCriterion.verdict}, now ${criterion.verdict}`,
        });
      }
    }

    // Check cost regression (>50% increase)
    if (baseline.cost_sats && scenario.criteria) {
      const currentCost = scenario.criteria.reduce((s, c) => s + (c.cost_sats || 0), 0);
      if (currentCost > baseline.cost_sats * 1.5) {
        regressions.push({
          scenario_id: scenario.id,
          scenario_name: scenario.name,
          type: 'cost_regression',
          baseline: baseline.cost_sats,
          current: currentCost,
          message: `[${scenario.id}] ${scenario.name}: grading cost ${currentCost} sats > 1.5x baseline ${baseline.cost_sats}`,
        });
      }
    }
  }

  return regressions;
}

// ---------------------------------------------------------------------------
// Markdown report generation
// ---------------------------------------------------------------------------

function generateReport(graded, baselines) {
  const regressions = detectRegressions(graded, baselines);
  const lines = [];

  // Header
  lines.push(`# Multi-Worm Integration Report — ${graded.timestamp}`);
  lines.push('');

  // Summary table
  lines.push('## Summary');
  lines.push('');
  lines.push('| Metric | Value |');
  lines.push('|--------|-------|');
  lines.push(`| Scenarios | ${graded.summary.total_scenarios} |`);
  lines.push(`| Passed | ${graded.summary.passed} |`);
  lines.push(`| Failed | ${graded.summary.failed} |`);
  lines.push(`| Partial | ${graded.summary.partial} |`);
  lines.push(`| Skipped | ${graded.summary.skipped} |`);
  lines.push(`| Criteria evaluated | ${graded.summary.total_criteria || graded.summary.total_criteria_evaluated || 0} |`);
  lines.push(`| Criteria passed | ${graded.summary.criteria_passed} |`);
  const needsReview = graded.summary?.needs_review || 0;
  if (needsReview > 0) lines.push(`| Needs review | ${needsReview} scenario(s) |`);
  lines.push(`| Source | ${graded.source_result} |`);
  lines.push('');

  // Per-scenario results
  lines.push('## Per-Scenario Results');
  lines.push('');
  lines.push('| ID | Name | Tier | Assertions | AI Grade | Criteria |');
  lines.push('|----|------|------|------------|----------|----------|');

  for (const s of graded.scenarios) {
    const assertIcon = s.pass === true ? 'PASS' : s.pass === false ? 'FAIL' : '-';
    const gradeIcon = s.overall_verdict === 'pass' ? 'PASS'
      : s.overall_verdict === 'fail' ? 'FAIL'
      : s.overall_verdict === 'partial' ? 'PARTIAL'
      : 'SKIP';

    const criteriaList = (s.criteria || [])
      .map(c => {
        const icon = c.verdict === 'pass' ? '+' : c.verdict === 'fail' ? 'x' : '~';
        return `${icon}${c.name}`;
      })
      .join(', ') || s.reason || '-';

    lines.push(`| ${s.id} | ${s.name} | ${s.tier} | ${assertIcon} | ${gradeIcon} | ${criteriaList} |`);
  }
  lines.push('');

  // Failed scenarios detail
  const failed = graded.scenarios.filter(s => s.overall_verdict === 'fail');
  if (failed.length > 0) {
    lines.push('## Failed Scenarios');
    lines.push('');

    for (const s of failed) {
      lines.push(`### [${s.id}] ${s.name}`);
      lines.push('');
      for (const c of s.criteria.filter(c => c.verdict !== 'pass')) {
        lines.push(`**${c.name}** — ${c.verdict.toUpperCase()}`);
        lines.push(`> ${c.reasoning}`);
        if (c.evidence && c.evidence.length > 0) {
          lines.push('');
          lines.push('Evidence:');
          for (const e of c.evidence) {
            lines.push(`- \`${e}\``);
          }
        }
        lines.push('');
      }
    }
  }

  // Regressions
  if (regressions.length > 0) {
    lines.push('## Regressions Detected');
    lines.push('');
    for (const r of regressions) {
      lines.push(`- **${r.type}**: ${r.message}`);
    }
    lines.push('');
  } else if (baselines) {
    lines.push('## Regressions');
    lines.push('');
    lines.push('No regressions detected against baseline.');
    lines.push('');
  }

  // Needs review section
  const reviewScenarios = graded.scenarios.filter(s => s.needs_review);
  if (reviewScenarios.length > 0) {
    lines.push('## Needs Review (Claude Code)');
    lines.push('');
    lines.push('These criteria require semantic evaluation during the quality loop:');
    lines.push('');
    for (const s of reviewScenarios) {
      const reviewCriteria = (s.criteria || []).filter(c => c.needs_review);
      for (const c of reviewCriteria) {
        lines.push(`### [${s.id}] ${s.name} / ${c.name}`);
        lines.push(`> ${c.reasoning}`);
        if (c.evidence.length > 0) {
          lines.push('');
          for (const e of c.evidence) {
            lines.push(`- \`${e}\``);
          }
        }
        lines.push('');
      }
    }
  }

  // Footer
  lines.push('---');
  lines.push(`Generated by Dolphin Milk multi-worm grader at ${graded.timestamp}`);

  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// Demo artifact extraction
// ---------------------------------------------------------------------------

function extractDemoArtifacts(graded) {
  const artifacts = {
    timestamp: graded.timestamp,
    summary: graded.summary,

    // Sanitized transcripts showing multi-agent conversation highlights
    conversation_highlights: [],

    // Per-criterion verdict roll-up
    criteria_roll_up: {},

    // Cost data showing real economic activity
    cost_data: {
      grading_cost_sats: (graded.grading_cost_sats || 0),
      model: graded.model,
      per_scenario: [],
    },
  };

  // Build criteria roll-up across all scenarios
  for (const s of graded.scenarios) {
    for (const c of s.criteria || []) {
      if (!artifacts.criteria_roll_up[c.name]) {
        artifacts.criteria_roll_up[c.name] = { pass: 0, fail: 0, partial: 0, error: 0 };
      }
      artifacts.criteria_roll_up[c.name][c.verdict] =
        (artifacts.criteria_roll_up[c.name][c.verdict] || 0) + 1;
    }

    // Cost per scenario
    if (!s.skipped) {
      const scenarioSats = (s.criteria || []).reduce((sum, c) => sum + (c.cost_sats || 0), 0);
      artifacts.cost_data.per_scenario.push({
        id: s.id,
        name: s.name,
        tier: s.tier,
        criteria_count: (s.criteria || []).length,
        cost_sats: scenarioSats,
        overall_verdict: s.overall_verdict,
      });
    }

    // Conversation highlights (scenarios with interesting results)
    if (s.overall_verdict === 'fail' || s.overall_verdict === 'partial') {
      artifacts.conversation_highlights.push({
        scenario: { id: s.id, name: s.name, tier: s.tier },
        failing_criteria: (s.criteria || [])
          .filter(c => c.verdict !== 'pass')
          .map(c => ({
            name: c.name,
            verdict: c.verdict,
            reasoning: c.reasoning,
            evidence: c.evidence,
          })),
      });
    }
  }

  return artifacts;
}

// ---------------------------------------------------------------------------
// Baseline management
// ---------------------------------------------------------------------------

function saveAsBaseline(graded, baselinePath) {
  const baseline = {
    timestamp: graded.timestamp,
    model: graded.model,
    scenarios: graded.scenarios.map(s => ({
      id: s.id,
      name: s.name,
      overall_verdict: s.overall_verdict,
      cost_sats: (s.criteria || []).reduce((sum, c) => sum + (c.cost_sats || 0), 0),
      criteria: (s.criteria || []).map(c => ({
        name: c.name,
        verdict: c.verdict,
      })),
    })),
  };

  fs.writeFileSync(baselinePath, JSON.stringify(baseline, null, 2));
  return baselinePath;
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

async function main() {
  const args = process.argv.slice(2);

  let gradedPath = null;
  let baselinePath = null;
  let saveBaseline = false;

  for (let i = 0; i < args.length; i++) {
    switch (args[i]) {
      case '--graded':
        gradedPath = args[++i];
        break;
      case '--baseline':
        baselinePath = args[++i];
        break;
      case '--save-baseline':
        saveBaseline = true;
        break;
      case '--help':
        console.log('Usage: node report.js [--graded path] [--baseline path] [--save-baseline]');
        process.exit(0);
    }
  }

  // Load graded results
  if (!gradedPath) gradedPath = loadLatestGraded();
  console.log(`Loading: ${path.basename(gradedPath)}`);

  const graded = JSON.parse(fs.readFileSync(gradedPath, 'utf8'));
  const baselines = loadBaselines(baselinePath);

  // Ensure artifacts directory exists
  if (!fs.existsSync(ARTIFACTS_DIR)) fs.mkdirSync(ARTIFACTS_DIR, { recursive: true });

  // Generate markdown report
  const report = generateReport(graded, baselines);
  const ts = new Date().toISOString().replace(/[:.]/g, '-');
  const reportPath = path.join(ARTIFACTS_DIR, `report-${ts}.md`);
  fs.writeFileSync(reportPath, report);
  console.log(`Report:    ${reportPath}`);

  // Generate demo artifacts
  const artifacts = extractDemoArtifacts(graded);
  const artifactsPath = path.join(ARTIFACTS_DIR, `demo-${ts}.json`);
  fs.writeFileSync(artifactsPath, JSON.stringify(artifacts, null, 2));
  console.log(`Artifacts: ${artifactsPath}`);

  // Save as baseline if requested
  if (saveBaseline) {
    const blPath = baselinePath || BASELINES_PATH;
    saveAsBaseline(graded, blPath);
    console.log(`Baseline:  ${blPath}`);
  }

  // Check for regressions
  const regressions = detectRegressions(graded, baselines);
  if (regressions.length > 0) {
    console.log(`\nREGRESSIONS DETECTED (${regressions.length}):`);
    for (const r of regressions) {
      console.log(`  [!] ${r.message}`);
    }
    process.exitCode = 1;
  } else if (baselines) {
    console.log('\nNo regressions detected.');
  }

  // Print summary
  console.log(`\nSummary: ${graded.summary.passed}/${graded.summary.total_scenarios} passed`);
}

main().catch(err => {
  console.error(`Fatal: ${err.message}`);
  process.exitCode = 1;
});
