#!/usr/bin/env node
// Unit tests for lib/proof_verify.js
//
// No live wallet or real txs needed. Uses a test seam:
// verifyProofBatch accepts an optional `_walletLookupFn` that mocks the wallet
// HTTP call. Tests pass synthetic lookup functions that return canned output
// arrays.

'use strict';

const assert = require('assert').strict;
const crypto = require('crypto');
const { execSync } = require('child_process');
const fs = require('fs');
const {
  computeExpectedHash,
  verifyProofBatch,
  extractOpReturnHash,
  hashRecordsFromFile,
} = require('./lib/proof_verify');

let pass = 0;
let fail = 0;
function test(name, fn) {
  return Promise.resolve()
    .then(fn)
    .then(() => {
      pass++;
      console.log(`  ok  ${name}`);
    })
    .catch((err) => {
      fail++;
      console.error(`  FAIL ${name}`);
      console.error(`       ${err.message}`);
      if (err.stack) console.error(err.stack.split('\n').slice(1, 4).join('\n'));
    });
}

// Build a fake wallet output entry: one OP_RETURN output with the given hash,
// optionally prefixed by a spend output to simulate a real tx.
function mockEntry(hash, createdAtMs = Date.now()) {
  return {
    outputs: [
      { satoshis: 500, lockingScript: '76a914' + '00'.repeat(20) + '88ac' }, // p2pkh noise
      { satoshis: 0, lockingScript: '006a20' + hash }, // OP_RETURN with hash
    ],
    createdAt: createdAtMs,
  };
}

function mockLookup(hashByTxid, createdAtByTxid = {}) {
  return async ({ txids }) => {
    const map = new Map();
    for (const txid of txids) {
      const h = hashByTxid[txid.toLowerCase()];
      if (!h) throw new Error(`mock: no hash for txid ${txid}`);
      const ts = createdAtByTxid[txid.toLowerCase()] ?? Date.now();
      map.set(txid.toLowerCase(), mockEntry(h, ts));
    }
    return map;
  };
}

function fakeTxid(seed) {
  return crypto.createHash('sha256').update(String(seed)).digest('hex');
}

async function main() {
  console.log('lib/proof_verify.js — unit tests');

  // ---------------------------------------------------------------------------
  // Test 1: computeExpectedHash matches bash jq | shasum byte-for-byte
  // ---------------------------------------------------------------------------
  await test('computeExpectedHash matches bash jq|shasum for a simple object', () => {
    const record = { a: 1, b: 2, c: 'hello' };
    const js = computeExpectedHash(record);
    const bash = execSync(
      `printf '%s' '${JSON.stringify(record).replace(/'/g, "'\\''")}' | jq -c . | tr -d '\\n' | shasum -a 256 | cut -d' ' -f1`,
      { encoding: 'utf8' }
    ).trim();
    assert.equal(js, bash, `js=${js} bash=${bash}`);
  });

  await test('hashRecordsFromFile matches bash jq|shasum on real Reddit snapshot', () => {
    const fp = '/tmp/reddit-sample.json';
    if (!fs.existsSync(fp)) {
      console.log('    (skipped — /tmp/reddit-sample.json not present)');
      return;
    }
    const rows = hashRecordsFromFile(fp, '.data.children[]');
    assert.ok(rows.length > 0, 'expected at least one record');
    // Compare each row to the bash equivalent
    for (let i = 0; i < rows.length; i++) {
      const bash = execSync(
        `jq -c '.data.children[${i}]' ${fp} | tr -d '\\n' | shasum -a 256 | cut -d' ' -f1`,
        { encoding: 'utf8' }
      ).trim();
      assert.equal(rows[i].hash, bash, `row ${i}: js=${rows[i].hash} bash=${bash}`);
    }
  });

  await test('computeExpectedHash handles nested structures', () => {
    const record = { id: 't3_abc', data: { title: 'Foo', score: 42, flags: [true, null, 3] } };
    const js = computeExpectedHash(record);
    const bash = execSync(
      `printf '%s' '${JSON.stringify(record).replace(/'/g, "'\\''")}' | jq -c . | tr -d '\\n' | shasum -a 256 | cut -d' ' -f1`,
      { encoding: 'utf8' }
    ).trim();
    assert.equal(js, bash);
  });

  // ---------------------------------------------------------------------------
  // Test 2: extractOpReturnHash
  // ---------------------------------------------------------------------------
  await test('extractOpReturnHash returns payload for valid OP_RETURN-32', () => {
    const h = 'a'.repeat(64);
    assert.equal(extractOpReturnHash('006a20' + h), h);
  });
  await test('extractOpReturnHash returns null for p2pkh', () => {
    assert.equal(extractOpReturnHash('76a91400000000000000000000000000000000000000000088ac'), null);
  });
  await test('extractOpReturnHash returns null for truncated script', () => {
    assert.equal(extractOpReturnHash('006a20'), null);
  });

  // ---------------------------------------------------------------------------
  // Test 3: verdict PASS on clean case
  // ---------------------------------------------------------------------------
  await test('verdict PASS when records, hashes, and time align', async () => {
    const records = [{ i: 1 }, { i: 2 }, { i: 3 }];
    const hashes = records.map(computeExpectedHash);
    const txids = [fakeTxid('a'), fakeTxid('b'), fakeTxid('c')];
    const now = Date.now();
    const hashByTxid = {};
    txids.forEach((t, i) => (hashByTxid[t] = hashes[i]));
    const verdict = await verifyProofBatch({
      mode: 'snapshot',
      records,
      reportedTxids: txids,
      workerWalletUrl: 'http://mock',
      timeWindow: { startMs: now - 1000, endMs: now + 1000 },
      skipOnChain: true,
      _walletLookupFn: mockLookup(hashByTxid),
    });
    assert.equal(verdict.verdict, 'PASS', JSON.stringify(verdict, null, 2));
    assert.equal(verdict.bijection.status, 'PASS');
    assert.equal(verdict.temporal.status, 'PASS');
    assert.equal(verdict.expectedCount, 3);
    assert.equal(verdict.actualCount, 3);
    assert.equal(verdict.verifiedCount, 3);
  });

  // ---------------------------------------------------------------------------
  // Test 4: bijection detects orphans (extra on-chain hash)
  // ---------------------------------------------------------------------------
  await test('bijection detects orphans (on-chain hash not in expected)', async () => {
    const records = [{ i: 1 }, { i: 2 }, { i: 3 }];
    const hashes = records.map(computeExpectedHash);
    // Corrupt one hash so chain returns something records don't contain
    const garbage = 'f'.repeat(64);
    const txids = [fakeTxid('x'), fakeTxid('y'), fakeTxid('z')];
    const hashByTxid = { [txids[0]]: hashes[0], [txids[1]]: garbage, [txids[2]]: hashes[2] };
    const now = Date.now();
    const verdict = await verifyProofBatch({
      mode: 'snapshot',
      records,
      reportedTxids: txids,
      workerWalletUrl: 'http://mock',
      timeWindow: { startMs: now - 1000, endMs: now + 1000 },
      skipOnChain: true,
      _walletLookupFn: mockLookup(hashByTxid),
    });
    assert.equal(verdict.verdict, 'FAIL');
    assert.equal(verdict.bijection.status, 'FAIL');
    assert.ok(verdict.bijection.orphans.includes(garbage));
    assert.equal(verdict.bijection.misses.length, 1); // hashes[1]
    assert.equal(verdict.bijection.misses[0], hashes[1]);
  });

  // ---------------------------------------------------------------------------
  // Test 5: bijection detects misses (wallet returned fewer unique hashes)
  // ---------------------------------------------------------------------------
  await test('bijection detects duplicates (same hash used twice)', async () => {
    const records = [{ i: 1 }, { i: 2 }, { i: 3 }];
    const hashes = records.map(computeExpectedHash);
    // Two txids decode to the same hash → duplicate + one missed record
    const txids = [fakeTxid('p'), fakeTxid('q'), fakeTxid('r')];
    const hashByTxid = {
      [txids[0]]: hashes[0],
      [txids[1]]: hashes[0], // duplicate
      [txids[2]]: hashes[2],
    };
    const now = Date.now();
    const verdict = await verifyProofBatch({
      mode: 'snapshot',
      records,
      reportedTxids: txids,
      workerWalletUrl: 'http://mock',
      timeWindow: { startMs: now - 1000, endMs: now + 1000 },
      skipOnChain: true,
      _walletLookupFn: mockLookup(hashByTxid),
    });
    assert.equal(verdict.verdict, 'FAIL');
    assert.equal(verdict.bijection.status, 'FAIL');
    assert.deepEqual(verdict.bijection.duplicates, [hashes[0]]);
    assert.ok(verdict.bijection.misses.includes(hashes[1]));
  });

  // ---------------------------------------------------------------------------
  // Test 6: temporal check catches out-of-window tx
  // ---------------------------------------------------------------------------
  await test('temporal check FAILS when tx created before window start', async () => {
    const records = [{ i: 1 }, { i: 2 }];
    const hashes = records.map(computeExpectedHash);
    const txids = [fakeTxid('t1'), fakeTxid('t2')];
    const hashByTxid = { [txids[0]]: hashes[0], [txids[1]]: hashes[1] };
    const now = Date.now();
    const createdAt = { [txids[0]]: now - 1000 * 60 * 60, [txids[1]]: now }; // 1 hour too early
    const verdict = await verifyProofBatch({
      mode: 'snapshot',
      records,
      reportedTxids: txids,
      workerWalletUrl: 'http://mock',
      timeWindow: { startMs: now - 1000, endMs: now + 1000 },
      skipOnChain: true,
      _walletLookupFn: mockLookup(hashByTxid, createdAt),
    });
    assert.equal(verdict.verdict, 'FAIL');
    assert.equal(verdict.bijection.status, 'PASS');
    assert.equal(verdict.temporal.status, 'FAIL');
    assert.equal(verdict.temporal.outOfWindow.length, 1);
    assert.equal(verdict.temporal.outOfWindow[0].txid, txids[0]);
  });

  // ---------------------------------------------------------------------------
  // Test 7: input validation
  // ---------------------------------------------------------------------------
  await test('throws on records/txids length mismatch', async () => {
    const now = Date.now();
    await assert.rejects(
      () =>
        verifyProofBatch({
          mode: 'snapshot',
          records: [{ i: 1 }],
          reportedTxids: [],
          workerWalletUrl: 'http://mock',
          timeWindow: { startMs: now - 1, endMs: now + 1 },
          skipOnChain: true,
          _walletLookupFn: mockLookup({}),
        }),
      /length mismatch/
    );
  });

  await test('throws on invalid txid format', async () => {
    const now = Date.now();
    await assert.rejects(
      () =>
        verifyProofBatch({
          mode: 'snapshot',
          records: [{ i: 1 }],
          reportedTxids: ['not-a-txid'],
          workerWalletUrl: 'http://mock',
          timeWindow: { startMs: now - 1, endMs: now + 1 },
          skipOnChain: true,
          _walletLookupFn: mockLookup({}),
        }),
      /invalid txid/
    );
  });

  await test('throws on live mode (not implemented)', async () => {
    const now = Date.now();
    await assert.rejects(
      () =>
        verifyProofBatch({
          mode: 'live',
          records: [],
          reportedTxids: [],
          workerWalletUrl: 'http://mock',
          timeWindow: { startMs: now, endMs: now },
          skipOnChain: true,
        }),
      /live mode not yet implemented/
    );
  });

  // ---------------------------------------------------------------------------
  console.log('');
  console.log(`${pass} passed, ${fail} failed`);
  process.exit(fail === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error('FATAL', err);
  process.exit(1);
});
