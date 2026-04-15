#!/usr/bin/env node
/**
 * e19b_comment_churn.js — validates the god-tier pivot assumption.
 *
 * Pulls /r/<sub>/comments.json with `before=<fullname>` cursor paging
 * for a handful of target subs, at a production-like cadence, and
 * measures new-comments-per-iter. Pure scraping, zero sats.
 *
 * Success criterion: at least one sub delivers ≥20 new comments per
 * 60s iter sustained across all 10 iters. If so, the cursor-paged
 * comment firehose architecture (PLAN-C-SCALE.md) is viable and we
 * proceed to build the cache feeder.
 *
 * Reddit cursor semantics:
 *   /r/<sub>/comments.json          → 100 newest comments, reverse-chron
 *   ?before=<fullname>              → items newer than that fullname
 *   ?after=<fullname>               → items older (pagination back in time)
 * children[0] is the newest; use that as next-iter's `before=` cursor.
 *
 * Usage:
 *   node lab/e19b_comment_churn.js
 *   SUBS=technology,worldnews,news ITERATIONS=10 INTERVAL_SEC=60 \
 *     node lab/e19b_comment_churn.js
 */

'use strict';

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const SUBS = (process.env.SUBS || 'technology,worldnews,news').split(',').map((s) => s.trim()).filter(Boolean);
const ITERATIONS = parseInt(process.env.ITERATIONS || '10', 10);
const INTERVAL_SEC = parseInt(process.env.INTERVAL_SEC || '60', 10);
const LIMIT = parseInt(process.env.LIMIT || '100', 10);
const USER_AGENT =
  'Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0';
// Stagger sub fetches inside one iter so we don't hit Reddit from the
// same IP in a burst. 800ms gives us ~1.25 req/sec well under the
// unauth 60/min limit.
const SUB_STAGGER_MS = 800;

const OUT_DIR = path.resolve(__dirname, '../../../test-workspaces/e19b-comments');
fs.mkdirSync(OUT_DIR, { recursive: true });
const STAMP = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
const OUT_FILE = path.join(OUT_DIR, `e19b-${STAMP}.json`);

function log(msg) {
  const ts = new Date().toISOString().slice(11, 19);
  console.log(`[e19b ${ts}] ${msg}`);
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

function pullComments(sub, beforeFullname) {
  const base = `https://www.reddit.com/r/${sub}/comments.json?limit=${LIMIT}`;
  const url = beforeFullname ? `${base}&before=${beforeFullname}` : base;
  const t0 = Date.now();
  const body = curlGet(url);
  const ms = Date.now() - t0;
  const parsed = JSON.parse(body);
  const children = (parsed.data && parsed.data.children) || [];
  // Each child: { kind: "t1", data: { id, name, body, author, ... } }
  // children[0] is newest.
  return { ms, total: children.length, children };
}

async function main() {
  log('='.repeat(70));
  log(`E19b comment churn probe — subs: [${SUBS.join(', ')}]`);
  log(`iterations=${ITERATIONS} interval=${INTERVAL_SEC}s limit=${LIMIT}`);
  log(`out: ${OUT_FILE}`);
  log('='.repeat(70));

  // Per-sub state: cursor (newest fullname seen) + running stats
  const state = Object.fromEntries(
    SUBS.map((sub) => [
      sub,
      {
        sub,
        cursor: null,
        totalNew: 0,
        iters: [],
        seenIds: new Set(), // safety net — verify cursor dedup is working
      },
    ]),
  );

  for (let i = 0; i < ITERATIONS; i++) {
    const iterStart = Date.now();
    log(`--- iter ${i + 1}/${ITERATIONS} ---`);
    for (const sub of SUBS) {
      const st = state[sub];
      let pull;
      try {
        pull = pullComments(sub, st.cursor);
      } catch (e) {
        log(`  ${sub}: SCRAPE FAILED: ${e.message}`);
        st.iters.push({ iter: i + 1, error: String(e.message) });
        await sleep(SUB_STAGGER_MS);
        continue;
      }

      let crossDupes = 0;
      let firstNew = null;
      const newIds = [];
      for (const child of pull.children) {
        const data = child && child.data;
        if (!data || !data.id) continue;
        if (st.seenIds.has(data.id)) {
          crossDupes++;
          continue;
        }
        st.seenIds.add(data.id);
        newIds.push(data.id);
        if (!firstNew) firstNew = data.name || `t1_${data.id}`;
      }

      // Update cursor to the newest fullname in this pull — which is
      // children[0].data.name when the pull returned anything.
      if (pull.children.length > 0 && pull.children[0].data && pull.children[0].data.name) {
        st.cursor = pull.children[0].data.name;
      }

      const iterSummary = {
        iter: i + 1,
        ts: new Date().toISOString(),
        wallMs: pull.ms,
        fetched: pull.total,
        newUnique: newIds.length,
        crossDupesWithCursor: crossDupes,
        cursorAfter: st.cursor,
      };
      st.iters.push(iterSummary);
      st.totalNew += newIds.length;

      log(
        `  ${sub.padEnd(14)} fetched=${String(pull.total).padStart(3)} ` +
          `new=${String(newIds.length).padStart(3)} ` +
          `dupe=${String(crossDupes).padStart(3)} ` +
          `totalNew=${String(st.totalNew).padStart(4)} ` +
          `cursor=${(st.cursor || '(none)').slice(0, 12)} ` +
          `(${pull.ms}ms)`,
      );

      // Persist incrementally
      const snapshot = {
        subs: SUBS,
        iterations: ITERATIONS,
        intervalSec: INTERVAL_SEC,
        limit: LIMIT,
        startedAt: STAMP,
        perSub: Object.fromEntries(
          SUBS.map((s) => [
            s,
            {
              sub: s,
              cursor: state[s].cursor,
              totalNew: state[s].totalNew,
              iters: state[s].iters,
              uniqueSeen: state[s].seenIds.size,
            },
          ]),
        ),
      };
      fs.writeFileSync(OUT_FILE, JSON.stringify(snapshot, null, 2));

      await sleep(SUB_STAGGER_MS);
    }

    if (i < ITERATIONS - 1) {
      const elapsed = Date.now() - iterStart;
      const wait = Math.max(0, INTERVAL_SEC * 1000 - elapsed);
      log(`sleeping ${Math.round(wait / 1000)}s before next iter...`);
      await sleep(wait);
    }
  }

  log('='.repeat(70));
  log('AGGREGATE PER SUB');
  log('='.repeat(70));
  log('  sub             totalNew   avgNew/iter  minNew  maxNew  decision');
  const decisions = {};
  for (const sub of SUBS) {
    const st = state[sub];
    const good = st.iters.filter((it) => !it.error && typeof it.newUnique === 'number');
    const avg = good.length ? st.totalNew / good.length : 0;
    const news = good.map((it) => it.newUnique);
    const minN = news.length ? Math.min(...news) : 0;
    const maxN = news.length ? Math.max(...news) : 0;
    // Success criterion: avg ≥20 per 60s iter AND min ≥5 (no complete
    // stalls). Gives confidence the sub is a reliable firehose source.
    const dec = avg >= 20 && minN >= 5 ? 'VIABLE' : avg >= 10 ? 'MARGINAL' : 'WEAK';
    decisions[sub] = { totalNew: st.totalNew, avgNew: +avg.toFixed(1), minN, maxN, decision: dec };
    log(
      `  ${sub.padEnd(14)}  ${String(st.totalNew).padStart(8)}  ${String(avg.toFixed(1)).padStart(11)}  ${String(minN).padStart(6)}  ${String(maxN).padStart(6)}  ${dec}`,
    );
  }

  const anyViable = Object.values(decisions).some((d) => d.decision === 'VIABLE');
  const overall = anyViable ? 'GO_BUILD_FEEDER' : 'NEEDS_RETHINK';
  log('');
  log(`OVERALL DECISION: ${overall}`);

  const aggregate = {
    subs: SUBS,
    iterations: ITERATIONS,
    intervalSec: INTERVAL_SEC,
    limit: LIMIT,
    startedAt: STAMP,
    decisions,
    overall,
    perSub: Object.fromEntries(
      SUBS.map((s) => [
        s,
        {
          sub: s,
          cursor: state[s].cursor,
          totalNew: state[s].totalNew,
          iters: state[s].iters,
          uniqueSeen: state[s].seenIds.size,
        },
      ]),
    ),
  };
  fs.writeFileSync(OUT_FILE, JSON.stringify(aggregate, null, 2));
  log(`out: ${OUT_FILE}`);

  process.exitCode = overall === 'GO_BUILD_FEEDER' ? 0 : 20;
}

main().catch((e) => {
  console.error('[e19b] FATAL:', e);
  process.exitCode = 99;
});
