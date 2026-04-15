// lib/proof_verify.js
//
// Bijective proof verification for DolphinMilkShake #23 quality gate.
//
// Given a batch of ground-truth records and the txids a Worker claims it
// published, independently:
//   1. Hash the records (byte-compatible with proof_records.sh via `jq -c`)
//   2. Look up each tx via the worker wallet's HTTP API and/or sqlite fallback
//   3. Extract the SHA-256 payload from each OP_RETURN output
//   4. Assert a 1:1 mapping (no orphans, no misses, no duplicates)
//   5. Check createdAt falls within the test's time window
//   6. (Optional) Check each tx has an attached merkle proof
//
// Exports:
//   verifyProofBatch(input): Promise<ProofVerdict>
//   computeExpectedHash(record): string
//
// See lib/CONTRACTS.md §lib/proof_verify.js for the interface spec and
// lib/INVESTIGATION-AGENT-B.md §Task 3 for the hash compatibility rationale.

'use strict';

const crypto = require('crypto');
const http = require('http');
const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const LOG = '[proof]';

// -----------------------------------------------------------------------------
// computeExpectedHash — byte-identical to proof_records.sh
// -----------------------------------------------------------------------------

/**
 * Hash a single record the same way `proof_records.sh` does:
 *
 *   echo -n "$record" | shasum -a 256 | cut -d' ' -f1
 *
 * where `$record` is a line from `jq -c '.[]' <file>`.
 *
 * IMPORTANT — float suffix caveat:
 * Agent B Task 3 confirmed that jq preserves float suffixes (e.g.
 * `created_utc":1776088267.0`) that Node's JSON.parse/JSON.stringify round-trip
 * destroys. That means if the input is a plain JS object that was parsed from
 * a Reddit JSON payload, `computeExpectedHash(obj)` will NOT match the shell
 * script's hash — the `.0` is already gone.
 *
 * Two accepted shapes:
 *   - `string`  — treated as the raw, pre-canonicalized JSON line for one
 *                 record. Piped directly through `jq -c .` to produce
 *                 byte-identical output to the shell script.
 *   - `object`  — JSON.stringified first. Only safe for records with no
 *                 floats-that-are-whole-numbers. Primarily for tests.
 *
 * For live Reddit records the caller should use `hashRecordsFromFile()` which
 * streams `jq -c <filter> <file>` and hashes each output line — that path
 * never JSON.parses, so float info is preserved byte-for-byte.
 *
 * @param {object|string} record
 * @returns {string} 64-char lowercase hex sha256
 */
function computeExpectedHash(record) {
  const input = typeof record === 'string' ? record : JSON.stringify(record);
  let canonical;
  try {
    canonical = execSync('jq -c .', {
      input,
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
    });
  } catch (err) {
    throw new Error(`${LOG} jq canonicalization failed: ${err.message}`);
  }
  // jq -c appends a trailing newline; `echo -n` in the shell script does not.
  canonical = canonical.replace(/\n$/, '');
  return crypto.createHash('sha256').update(canonical).digest('hex');
}

/**
 * Read a JSON file, run `jq -c <filter>` against it, and hash each emitted
 * line with sha256. This is the byte-accurate path for Reddit snapshots — it
 * never parses floats through JS, so `1776088267.0` survives.
 *
 * @param {string} filePath absolute path to the raw JSON file
 * @param {string} jqFilter e.g. `.data.children[]`
 * @returns {Array<{rawLine: string, hash: string}>}
 */
function hashRecordsFromFile(filePath, jqFilter) {
  let out;
  try {
    out = execSync(`jq -c ${JSON.stringify(jqFilter)} ${JSON.stringify(filePath)}`, {
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
      maxBuffer: 64 * 1024 * 1024,
    });
  } catch (err) {
    throw new Error(`${LOG} jq file extraction failed: ${err.message}`);
  }
  const lines = out.split('\n').filter((l) => l.length > 0);
  return lines.map((rawLine) => ({
    rawLine,
    hash: crypto.createHash('sha256').update(rawLine).digest('hex'),
  }));
}

// -----------------------------------------------------------------------------
// OP_RETURN extraction — from Agent B Task 2
// -----------------------------------------------------------------------------

/**
 * Extract the 32-byte SHA-256 payload from a BSV OP_FALSE OP_RETURN output.
 *
 * Expected script layout: 006a20<64 hex chars>
 *   00 = OP_FALSE, 6a = OP_RETURN, 20 = PUSH_32
 *
 * @param {string} lockingScriptHex
 * @returns {string|null} 64-char lowercase hex, or null if not an OP_RETURN-32
 */
function extractOpReturnHash(lockingScriptHex) {
  if (typeof lockingScriptHex !== 'string') return null;
  const lc = lockingScriptHex.toLowerCase();
  const prefix = '006a20';
  if (!lc.startsWith(prefix)) return null;
  const payload = lc.substring(prefix.length, prefix.length + 64);
  if (!/^[0-9a-f]{64}$/.test(payload)) return null;
  return payload;
}

// -----------------------------------------------------------------------------
// HTTP helper — plain POST, no auth (the worker wallet is local/unsecured)
// -----------------------------------------------------------------------------

function httpPostJson(url, body, timeoutMs = 15000) {
  return new Promise((resolve, reject) => {
    let u;
    try {
      u = new URL(url);
    } catch (err) {
      return reject(new Error(`${LOG} invalid wallet URL ${url}: ${err.message}`));
    }
    const payload = Buffer.from(JSON.stringify(body), 'utf8');
    const req = http.request(
      {
        hostname: u.hostname,
        port: u.port || 80,
        path: u.pathname + (u.search || ''),
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': payload.length,
          Origin: `${u.protocol}//${u.host}`,
        },
        timeout: timeoutMs,
      },
      (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          let parsed;
          try {
            parsed = text ? JSON.parse(text) : null;
          } catch (err) {
            return reject(
              new Error(
                `${LOG} wallet returned non-JSON from ${url}: ${err.message} (body=${text.slice(0, 200)})`
              )
            );
          }
          resolve({ status: res.statusCode, body: parsed });
        });
      }
    );
    req.on('timeout', () => {
      req.destroy(new Error(`${LOG} wallet request timed out after ${timeoutMs}ms: ${url}`));
    });
    req.on('error', reject);
    req.write(payload);
    req.end();
  });
}

// -----------------------------------------------------------------------------
// Default wallet lookup: hit POST /listActions with txid filter
// -----------------------------------------------------------------------------

/**
 * Fetch a batch of actions/txids from the worker wallet and return a map of
 * txid -> { outputs: [{ satoshis, lockingScript }], createdAt: number|null }.
 *
 * Strategy:
 *   1. POST /listActions with large limit + includeOutputs=true
 *   2. Scan the returned actions for matching txids
 *   3. For any txid we still haven't found, fall back to SQLite if a db path
 *      was supplied.
 *
 * @param {{ txids: string[], workerWalletUrl: string, workerWalletDbPath?: string }} opts
 * @returns {Promise<Map<string, {outputs: Array<{satoshis:number, lockingScript:string}>, createdAt: number|null}>>}
 */
async function defaultWalletLookup(opts) {
  const { txids, workerWalletUrl, workerWalletDbPath } = opts;
  const result = new Map();

  if (!txids.length) return result;

  // Step 1: listActions
  //
  // CRITICAL: BRC-100 listActions needs `includeOutputLockingScripts: true` to
  // return the actual lockingScript hex bytes per output. `includeOutputs: true`
  // alone returns output satoshis + vout metadata but with empty lockingScript
  // strings, which causes extractOpReturnHash to return null for every output
  // and verifyProofBatch to throw "no OP_RETURN-32 output found". Observed in
  // the first #23 run (2026-04-13): 10 real 006a20<hash> outputs existed in
  // the wallet but listActions returned them with lockingScript="". Fix: pass
  // includeOutputLockingScripts: true.
  try {
    const resp = await httpPostJson(`${workerWalletUrl}/listActions`, {
      labels: [],
      limit: 1000,
      includeOutputs: true,
      includeOutputLockingScripts: true,
      includeRawTx: false,
    });
    if (resp.status !== 200 || !resp.body) {
      throw new Error(`listActions returned status ${resp.status}`);
    }
    const actions = Array.isArray(resp.body.actions) ? resp.body.actions : [];
    const wanted = new Set(txids.map((t) => t.toLowerCase()));
    for (const action of actions) {
      const txid = (action.txid || action.txID || '').toLowerCase();
      if (!wanted.has(txid)) continue;
      const outputs = Array.isArray(action.outputs)
        ? action.outputs.map((o) => ({
            satoshis: Number(o.satoshis ?? o.satoshis_value ?? 0),
            lockingScript: String(o.lockingScript || o.locking_script || o.script || ''),
          }))
        : [];
      // createdAt can be unix seconds, ms, or ISO string; normalize to ms.
      let createdAtMs = null;
      const raw = action.createdAt ?? action.created_at ?? action.timestamp;
      if (typeof raw === 'number') {
        createdAtMs = raw > 1e12 ? raw : raw * 1000;
      } else if (typeof raw === 'string') {
        const parsed = Date.parse(raw);
        if (!Number.isNaN(parsed)) createdAtMs = parsed;
      }
      result.set(txid, { outputs, createdAt: createdAtMs });
    }
  } catch (err) {
    // Fall through to sqlite if provided; otherwise re-throw below.
    console.error(`${LOG} listActions failed: ${err.message}`);
  }

  // Step 2: sqlite fallback for missing txids
  const missing = txids.filter((t) => !result.has(t.toLowerCase()));
  if (missing.length && workerWalletDbPath && fs.existsSync(workerWalletDbPath)) {
    for (const txid of missing) {
      try {
        // shell out to sqlite3 for simplicity
        const sql = `SELECT hex(o.locking_script), o.satoshis, o.vout, t.created_at FROM transactions t JOIN outputs o ON t.transaction_id = o.transaction_id WHERE lower(t.txid) = lower('${txid.replace(/'/g, "''")}');`;
        const raw = execSync(`sqlite3 -separator '|' ${JSON.stringify(workerWalletDbPath)} ${JSON.stringify(sql)}`, {
          encoding: 'utf8',
          stdio: ['pipe', 'pipe', 'pipe'],
        });
        const lines = raw.split('\n').filter(Boolean);
        if (!lines.length) continue;
        const outputs = [];
        let createdAtMs = null;
        for (const line of lines) {
          const [script, sats, _vout, ts] = line.split('|');
          outputs.push({ satoshis: Number(sats || 0), lockingScript: (script || '').toLowerCase() });
          if (!createdAtMs && ts) {
            const parsed = Date.parse(ts);
            if (!Number.isNaN(parsed)) createdAtMs = parsed;
          }
        }
        result.set(txid.toLowerCase(), { outputs, createdAt: createdAtMs });
      } catch (err) {
        console.error(`${LOG} sqlite fallback failed for ${txid}: ${err.message}`);
      }
    }
  }

  const stillMissing = txids.filter((t) => !result.has(t.toLowerCase()));
  if (stillMissing.length) {
    throw new Error(
      `${LOG} could not locate ${stillMissing.length}/${txids.length} txids in worker wallet: ${stillMissing.slice(0, 3).join(', ')}${stillMissing.length > 3 ? '…' : ''}`
    );
  }

  return result;
}

// -----------------------------------------------------------------------------
// verifyProofBatch — the main entry point
// -----------------------------------------------------------------------------

/**
 * @param {import('./CONTRACTS.md').ProofVerifyInput & { _walletLookupFn?: Function }} input
 * @returns {Promise<import('./CONTRACTS.md').ProofVerdict>}
 */
async function verifyProofBatch(input) {
  const started = Date.now();

  if (!input || typeof input !== 'object') {
    throw new Error(`${LOG} verifyProofBatch requires an input object`);
  }
  const {
    mode = 'snapshot',
    records,
    reportedTxids,
    workerWalletUrl,
    workerWalletDbPath,
    timeWindow,
    hashFn = computeExpectedHash,
    chainCheckTimeoutMs = 120000,
    skipOnChain = false,
    _walletLookupFn, // test seam — undocumented
  } = input;

  if (mode === 'live') {
    throw new Error(`${LOG} live mode not yet implemented in Phase 2`);
  }
  if (mode !== 'snapshot') {
    throw new Error(`${LOG} unknown mode: ${mode}`);
  }
  if (!Array.isArray(records)) {
    throw new Error(`${LOG} records must be an array`);
  }
  if (!Array.isArray(reportedTxids)) {
    throw new Error(`${LOG} reportedTxids must be an array`);
  }
  if (records.length !== reportedTxids.length) {
    throw new Error(
      `${LOG} length mismatch: records=${records.length} reportedTxids=${reportedTxids.length}`
    );
  }
  const hexRe = /^[0-9a-fA-F]{64}$/;
  for (const t of reportedTxids) {
    if (typeof t !== 'string' || !hexRe.test(t)) {
      throw new Error(`${LOG} invalid txid (not 64 hex chars): ${JSON.stringify(t)}`);
    }
  }
  if (!timeWindow || typeof timeWindow.startMs !== 'number' || typeof timeWindow.endMs !== 'number') {
    throw new Error(`${LOG} timeWindow must be { startMs, endMs }`);
  }

  // --- Step 2: expected hashes ------------------------------------------------
  const expectedHashes = [];
  const expectedSet = new Set();
  const hashToRecordIndex = new Map();
  for (let i = 0; i < records.length; i++) {
    const h = hashFn(records[i]).toLowerCase();
    expectedHashes.push(h);
    expectedSet.add(h);
    if (!hashToRecordIndex.has(h)) hashToRecordIndex.set(h, i);
  }

  // --- Step 3: actual on-chain hashes ----------------------------------------
  const lookupFn = typeof _walletLookupFn === 'function' ? _walletLookupFn : defaultWalletLookup;
  const txidMap = await lookupFn({
    txids: reportedTxids,
    workerWalletUrl,
    workerWalletDbPath,
  });

  const actualSet = new Set();
  const txidToHash = new Map(); // txid -> hash
  const hashCount = new Map(); // hash -> count (for dupe detection)
  const txidCreatedAt = new Map(); // txid -> createdAt ms
  const pairs = []; // evidence rows

  for (const txid of reportedTxids) {
    const lower = txid.toLowerCase();
    const entry = txidMap.get(lower);
    if (!entry) {
      throw new Error(`${LOG} wallet lookup missing txid ${txid}`);
    }
    // Find the first OP_RETURN output with a valid 32-byte payload.
    let hash = null;
    for (const out of entry.outputs || []) {
      if (Number(out.satoshis) !== 0) continue;
      const h = extractOpReturnHash(out.lockingScript || '');
      if (h) {
        hash = h;
        break;
      }
    }
    if (!hash) {
      throw new Error(`${LOG} no OP_RETURN-32 output found in tx ${txid}`);
    }
    actualSet.add(hash);
    txidToHash.set(lower, hash);
    hashCount.set(hash, (hashCount.get(hash) || 0) + 1);
    txidCreatedAt.set(lower, entry.createdAt);
    pairs.push({
      txid: lower,
      hash,
      recordId: (() => {
        const idx = hashToRecordIndex.get(hash);
        if (idx == null) return undefined;
        const r = records[idx];
        if (r && typeof r === 'object') {
          if (typeof r.id === 'string') return r.id;
          if (r.data && typeof r.data.id === 'string') return r.data.id;
        }
        return undefined;
      })(),
      matched: expectedSet.has(hash),
    });
  }

  // --- Step 4: bijection ------------------------------------------------------
  const orphans = [];
  for (const h of actualSet) {
    if (!expectedSet.has(h)) orphans.push(h);
  }
  const misses = [];
  for (const h of expectedSet) {
    if (!actualSet.has(h)) misses.push(h);
  }
  const duplicates = [];
  for (const [h, count] of hashCount) {
    if (count > 1) duplicates.push(h);
  }
  const bijection = {
    status: orphans.length === 0 && misses.length === 0 && duplicates.length === 0 ? 'PASS' : 'FAIL',
    orphans,
    misses,
    duplicates,
  };

  // --- Step 5: temporal ------------------------------------------------------
  const outOfWindow = [];
  const lo = timeWindow.startMs;
  const hi = timeWindow.endMs + 60000; // 1-minute grace
  for (const [txid, createdAt] of txidCreatedAt) {
    if (createdAt == null) continue; // unknown timestamps don't fail the check
    if (createdAt < lo || createdAt > hi) {
      outOfWindow.push({ txid, timestamp: new Date(createdAt).toISOString() });
    }
  }
  const temporal = {
    status: outOfWindow.length === 0 ? 'PASS' : 'FAIL',
    windowStart: new Date(timeWindow.startMs).toISOString(),
    windowEnd: new Date(timeWindow.endMs).toISOString(),
    outOfWindow,
  };

  // --- Step 6: on-chain broadcast check --------------------------------------
  // The worker wallet doesn't expose a clean "is this tx proven?" endpoint via
  // HTTP — per Agent B, proven_txs is a sqlite-only concept. Skip this step
  // in the first pass unless we have a db path AND !skipOnChain.
  let onChain;
  if (skipOnChain) {
    onChain = {
      status: 'SKIPPED',
      verifiedCount: 0,
      pendingCount: reportedTxids.length,
      failed: [],
    };
  } else if (workerWalletDbPath && fs.existsSync(workerWalletDbPath)) {
    const verified = [];
    const pending = [];
    const failed = [];
    for (const txid of reportedTxids) {
      try {
        const sql = `SELECT count(*) FROM proven_txs WHERE lower(txid) = lower('${txid.replace(/'/g, "''")}');`;
        const out = execSync(
          `sqlite3 ${JSON.stringify(workerWalletDbPath)} ${JSON.stringify(sql)}`,
          { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }
        ).trim();
        if (Number(out) > 0) verified.push(txid);
        else pending.push(txid);
      } catch (err) {
        failed.push({ txid, reason: err.message });
      }
    }
    onChain = {
      status:
        failed.length > 0
          ? 'FAIL'
          : pending.length === 0
            ? 'PASS'
            : 'PENDING',
      verifiedCount: verified.length,
      pendingCount: pending.length,
      failed,
    };
  } else {
    onChain = {
      status: 'SKIPPED',
      verifiedCount: 0,
      pendingCount: reportedTxids.length,
      failed: [
        {
          txid: '(all)',
          reason: 'workerWalletDbPath not provided; cannot check proven_txs',
        },
      ],
    };
  }

  // --- Verdict ---------------------------------------------------------------
  const verifiedCount = pairs.filter((p) => p.matched).length;
  const verdict =
    bijection.status === 'PASS' && temporal.status === 'PASS' && onChain.status !== 'FAIL'
      ? 'PASS'
      : 'FAIL';

  return {
    verdict,
    mode,
    expectedCount: expectedHashes.length,
    actualCount: actualSet.size,
    verifiedCount,
    bijection,
    onChain,
    temporal,
    evidence: {
      txidHashPairs: pairs,
      hashFnDescription: 'sha256(jq -c canonicalized JSON) — matches proof_records.sh byte-for-byte',
    },
    totals: {
      satsSpentOnProofs: 0, // not computed in Phase 2; wallet doesn't expose fee per tx cleanly
      wallClockMs: Date.now() - started,
    },
  };
}

module.exports = {
  verifyProofBatch,
  computeExpectedHash,
  hashRecordsFromFile,
  // exposed for reuse / testing
  extractOpReturnHash,
  defaultWalletLookup,
};
