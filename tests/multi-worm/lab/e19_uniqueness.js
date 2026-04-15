#!/usr/bin/env node
/**
 * e19_uniqueness.js — Reddit uniqueness ceiling measurement.
 *
 * Scrapes r/technology/hot.json at a production-like cadence and measures
 * how many UNIQUE posts appear per scrape after a strict post-ID dedup.
 *
 * This is the E19 gate from PLAN-C-SCALE.md: if unique-per-cycle drops below
 * 30 after 3 cycles, we pivot to comment scraping. If it stays >=60 we can
 * ride on posts alone for the 1.5M target.
 *
 * Design: pure scraping, zero sats, does not touch test_cycle_v2.js.
 * Uses the same URL + User-Agent + curl invocation as the harness so the
 * signal is a faithful proxy for what production cycles will see.
 *
 * Usage:
 *   node lab/e19_uniqueness.js
 *   INTERVAL_SEC=200 ITERATIONS=10 SUBREDDIT=technology node lab/e19_uniqueness.js
 */

'use strict';

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const SUBREDDIT = process.env.SUBREDDIT || 'technology';
const ITERATIONS = parseInt(process.env.ITERATIONS || '10', 10);
const INTERVAL_SEC = parseInt(process.env.INTERVAL_SEC || '180', 10); // 3 min matches projected worker-only cycle wall
const LIMIT = parseInt(process.env.LIMIT || '100', 10);
const USER_AGENT =
  'Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0';

const OUT_DIR = path.resolve(__dirname, '../../../test-workspaces/e19-uniqueness');
fs.mkdirSync(OUT_DIR, { recursive: true });
const STAMP = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
const OUT_FILE = path.join(OUT_DIR, `e19-${SUBREDDIT}-${STAMP}.json`);

function log(msg) {
  const ts = new Date().toISOString().slice(11, 19);
  console.log(`[e19 ${ts}] ${msg}`);
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function curlGet(url) {
  return execFileSync(
    'curl',
    ['-sS', '-A', USER_AGENT, '-L', '--max-time', '30', url],
    { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
  );
}

function scrapeHot() {
  const url = `https://www.reddit.com/r/${SUBREDDIT}/hot.json?limit=${LIMIT}`;
  const t0 = Date.now();
  const body = curlGet(url);
  const ms = Date.now() - t0;
  const parsed = JSON.parse(body);
  const children = (parsed.data && parsed.data.children) || [];
  const ids = children
    .map((c) => c && c.data && c.data.id)
    .filter((id) => typeof id === 'string' && id.length > 0);
  return { ms, total: children.length, ids };
}

function decisionLabel(unique, iter) {
  // Per PLAN-C:
  //   >=60 unique/cycle after cycle 3 → POSTS-ONLY VIABLE
  //   30-59 after cycle 3            → BORDERLINE (needs more data)
  //   <30 after cycle 3              → PIVOT to comments
  if (iter < 3) return null;
  if (unique >= 60) return 'PASS_POSTS_ONLY';
  if (unique >= 30) return 'BORDERLINE';
  return 'PIVOT_TO_COMMENTS';
}

async function main() {
  log('='.repeat(70));
  log(`E19 uniqueness ceiling: r/${SUBREDDIT}, ${ITERATIONS} iters, ${INTERVAL_SEC}s apart, limit=${LIMIT}`);
  log(`out: ${OUT_FILE}`);
  log('='.repeat(70));

  const cumulative = new Set();
  const iters = [];
  let earlyDecision = null;

  for (let i = 0; i < ITERATIONS; i++) {
    const iterStart = Date.now();
    let scrape;
    try {
      scrape = scrapeHot();
    } catch (e) {
      log(`iter ${i + 1} SCRAPE FAILED: ${e.message}`);
      iters.push({ iter: i + 1, error: String(e.message) });
      // Continue — we want data from other iterations
      if (i < ITERATIONS - 1) {
        log(`sleeping ${INTERVAL_SEC}s before next iter...`);
        await sleep(INTERVAL_SEC * 1000);
      }
      continue;
    }

    const newThisIter = [];
    const dupeThisIter = [];
    for (const id of scrape.ids) {
      if (cumulative.has(id)) {
        dupeThisIter.push(id);
      } else {
        cumulative.add(id);
        newThisIter.push(id);
      }
    }

    const summary = {
      iter: i + 1,
      ts: new Date().toISOString(),
      wallMs: scrape.ms,
      totalFetched: scrape.total,
      newUnique: newThisIter.length,
      dupes: dupeThisIter.length,
      cumulativeUnique: cumulative.size,
      sampleNewIds: newThisIter.slice(0, 5),
    };
    iters.push(summary);

    log(
      `iter ${String(i + 1).padStart(2)}/${ITERATIONS} ` +
        `fetched=${summary.totalFetched} ` +
        `new=${summary.newUnique} ` +
        `dupe=${summary.dupes} ` +
        `cumulative=${summary.cumulativeUnique} ` +
        `(scrape ${summary.wallMs}ms)`,
    );

    // Persist incrementally so we can inspect mid-run
    fs.writeFileSync(
      OUT_FILE,
      JSON.stringify(
        {
          subreddit: SUBREDDIT,
          iterations: ITERATIONS,
          intervalSec: INTERVAL_SEC,
          limit: LIMIT,
          startedAt: STAMP,
          iters,
          cumulativeUnique: cumulative.size,
        },
        null,
        2,
      ),
    );

    // Early pivot gate: if cycle 3 comes in <30 new-unique, stop and shout.
    if (i + 1 === 3 && summary.newUnique < 30) {
      earlyDecision = 'PIVOT_TO_COMMENTS';
      log('>>> EARLY DECISION: new-unique at cycle 3 < 30 → PIVOT TO COMMENTS');
      log('>>> Stopping early. Flag to operator.');
      break;
    }

    if (i < ITERATIONS - 1) {
      const elapsed = Date.now() - iterStart;
      const wait = Math.max(0, INTERVAL_SEC * 1000 - elapsed);
      log(`sleeping ${Math.round(wait / 1000)}s before next iter...`);
      await sleep(wait);
    }
  }

  // Decision
  const successfulIters = iters.filter((it) => !it.error);
  const lastIter = successfulIters[successfulIters.length - 1];
  const lastUnique = lastIter ? lastIter.newUnique : 0;
  const decision =
    earlyDecision ||
    decisionLabel(lastUnique, successfulIters.length) ||
    'INSUFFICIENT_DATA';

  // Compute avg new-unique after cycle 2 (steady-state indicator)
  const steadyState = successfulIters.slice(2);
  const steadyAvg =
    steadyState.length > 0
      ? Math.round(
          steadyState.reduce((acc, it) => acc + it.newUnique, 0) / steadyState.length,
        )
      : null;

  const aggregate = {
    subreddit: SUBREDDIT,
    iterations: ITERATIONS,
    intervalSec: INTERVAL_SEC,
    limit: LIMIT,
    startedAt: STAMP,
    cumulativeUnique: cumulative.size,
    steadyStateAvgNewUnique: steadyAvg,
    decision,
    iters,
  };
  fs.writeFileSync(OUT_FILE, JSON.stringify(aggregate, null, 2));

  log('='.repeat(70));
  log('AGGREGATE');
  log('='.repeat(70));
  log(`cumulative unique post IDs: ${cumulative.size}`);
  log(`steady-state avg new-unique (iter 3+): ${steadyAvg}`);
  log(`DECISION: ${decision}`);
  log(`out: ${OUT_FILE}`);
  log('');
  log('per-iter:');
  log('  iter  fetched  new   dupe  cumulative');
  for (const it of iters) {
    if (it.error) {
      log(`  ${String(it.iter).padStart(4)}  ERROR: ${it.error}`);
      continue;
    }
    log(
      `  ${String(it.iter).padStart(4)}  ${String(it.totalFetched).padStart(7)}  ${String(it.newUnique).padStart(3)}  ${String(it.dupes).padStart(4)}  ${String(it.cumulativeUnique).padStart(10)}`,
    );
  }

  // Exit code conveys decision for pipeline use
  if (decision === 'PASS_POSTS_ONLY') process.exitCode = 0;
  else if (decision === 'BORDERLINE') process.exitCode = 10;
  else if (decision === 'PIVOT_TO_COMMENTS') process.exitCode = 20;
  else process.exitCode = 30;
}

main().catch((e) => {
  console.error('[e19] FATAL:', e);
  process.exitCode = 99;
});
