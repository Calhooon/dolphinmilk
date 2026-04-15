#!/usr/bin/env node
/**
 * Fund helper for multi-worm cascade tests.
 *
 * Top up Coordinator (:3323) and Worker (:3324) wallets from Captain's
 * wallet (:3322) by:
 *
 *   1. Calling `dolphin-milk receive` on each recipient wallet to get
 *      their BRC-29 receive address + derived public key.
 *   2. Building a P2PKH lockingScript for each address via @bsv/sdk.
 *   3. POSTing to Captain's wallet /createAction with two outputs —
 *      the wallet signs, broadcasts, and returns the txid.
 *   4. Invoking `dolphin-milk fund <txid> --vout N` on each recipient
 *      via DOLPHIN_MILK_WALLET_URL env override. This internalizes the
 *      payment via the WoC fetch path with the correct BRC-29 suffix.
 *
 * Amounts: edit FUND_COORD_SATS / FUND_WORKER_SATS at the top.
 *
 * Usage:
 *   node fund-agents.js
 *
 * Preconditions:
 *   - Captain's wallet daemon running on 3322
 *   - Coord's wallet daemon running on 3323
 *   - Worker's wallet daemon running on 3324
 *   - dolphin-milk binary built at target/release/dolphin-milk
 */

const { execFileSync } = require('child_process');
const http = require('http');
const path = require('path');
const { P2PKH, PublicKey, Hash } = require('@bsv/sdk');

const PROJECT_ROOT = path.resolve(__dirname, '../../');
const BINARY = path.join(PROJECT_ROOT, 'target/release/dolphin-milk');

const CAPTAIN_WALLET_PORT = 3322;
const COORD_WALLET_PORT = 3323;
const WORKER_WALLET_PORT = 3324;

// Two modes:
//   --probe   tiny amounts, to verify the whole flow end-to-end before
//             moving real money.
//   default   production top-up amounts.
//
// Always run --probe first. Only scale to default after it confirms.
const IS_PROBE = process.argv.includes('--probe');
const FUND_COORD_SATS = IS_PROBE ?   100_000 : 15_000_000;
const FUND_WORKER_SATS = IS_PROBE ?   100_000 :  5_000_000;

// Split the total into N equal outputs per recipient. Creates N parallel
// UTXOs in one createAction instead of one giant UTXO. Parallel UTXOs help
// the wallet avoid serial-chain dependencies when the recipient fires
// multiple txs back-to-back during a cascade run — each tx can spend a
// different fresh UTXO without waiting for a prior tx's change output.
const SPLIT_COUNT = 5;

function httpPost(port, endpoint, body) {
  return new Promise((resolve, reject) => {
    const data = JSON.stringify(body);
    const req = http.request({
      hostname: '127.0.0.1',
      port,
      path: `/${endpoint}`,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Origin': `http://localhost:${port}`,
        'Content-Length': Buffer.byteLength(data),
      },
    }, (res) => {
      let raw = '';
      res.on('data', chunk => raw += chunk);
      res.on('end', () => {
        try {
          const parsed = JSON.parse(raw);
          if (parsed.error) {
            reject(new Error(`HTTP ${res.statusCode} on /${endpoint}: ${JSON.stringify(parsed.error)}`));
          } else {
            resolve(parsed);
          }
        } catch {
          reject(new Error(`Non-JSON response from /${endpoint}: ${raw.substring(0, 300)}`));
        }
      });
    });
    req.on('error', reject);
    req.setTimeout(30000, () => { req.destroy(); reject(new Error(`timeout on /${endpoint}`)); });
    req.write(data);
    req.end();
  });
}

function cliReceive(walletUrl) {
  const out = execFileSync(BINARY, ['receive'], {
    encoding: 'utf8',
    env: { ...process.env, DOLPHIN_MILK_WALLET_URL: walletUrl },
    cwd: PROJECT_ROOT,
  });
  const addr = out.match(/BSV address:\s*(\S+)/)?.[1];
  const pubkey = out.match(/Public key:\s*([0-9a-f]+)/)?.[1];
  if (!addr || !pubkey) throw new Error(`Could not parse receive output from ${walletUrl}: ${out}`);
  return { addr, pubkey };
}

function cliFund(walletUrl, txid, vout, suffix = '1') {
  const out = execFileSync(BINARY,
    ['fund', txid, '--vout', String(vout), '--suffix', String(suffix)],
    {
      encoding: 'utf8',
      env: { ...process.env, DOLPHIN_MILK_WALLET_URL: walletUrl },
      cwd: PROJECT_ROOT,
    }
  );
  return out;
}

function p2pkhScriptHex(address) {
  // Use @bsv/sdk's LockingScript builder via P2PKH template.
  const lock = new P2PKH().lock(address);
  return lock.toHex();
}

/**
 * Round-trip verify that a pubkey hex string hashes to the same address the
 * CLI returned. If the CLI output is tampered with (or the wallet daemon is
 * lying about which key it holds), this catches it before we send sats.
 *
 * Path: pubkey hex → DER bytes → hash160 → address via @bsv/sdk PublicKey.
 */
function verifyAddressMatchesPubkey(addr, pubkeyHex) {
  const pk = PublicKey.fromString(pubkeyHex);
  const recomputed = pk.toAddress();
  if (recomputed !== addr) {
    throw new Error(
      `Address/pubkey mismatch!\n  pubkey ${pubkeyHex}\n  CLI addr ${addr}\n  recomputed ${recomputed}\n` +
      `  This means the wallet daemon is returning a pubkey that does NOT derive to the address it claims. ` +
      `Refusing to fund.`
    );
  }
}

async function main() {
  console.log('='.repeat(70));
  console.log('  multi-worm funding helper');
  console.log('='.repeat(70));
  console.log();

  console.log(`  Mode: ${IS_PROBE ? 'PROBE (tiny amounts)' : 'PRODUCTION top-up'}`);
  console.log(`  Coord amount:  ${FUND_COORD_SATS.toLocaleString()} sats`);
  console.log(`  Worker amount: ${FUND_WORKER_SATS.toLocaleString()} sats`);
  console.log();

  // ---- 1. Get receive addresses from Coord + Worker wallets ----------------
  console.log('[1] Getting receive addresses from Coord + Worker...');
  const coord = cliReceive(`http://localhost:${COORD_WALLET_PORT}`);
  const worker = cliReceive(`http://localhost:${WORKER_WALLET_PORT}`);
  console.log(`    Coord  : ${coord.addr}  (pubkey ${coord.pubkey.substring(0, 16)}...)`);
  console.log(`    Worker : ${worker.addr}  (pubkey ${worker.pubkey.substring(0, 16)}...)`);
  console.log();

  // ---- 1.5. Verify the CLI-returned address actually derives from the
  //           CLI-returned pubkey. Guards against daemon lies / parse bugs.
  console.log('[1.5] Verifying addresses round-trip from pubkeys (local check)...');
  verifyAddressMatchesPubkey(coord.addr, coord.pubkey);
  verifyAddressMatchesPubkey(worker.addr, worker.pubkey);
  console.log('    OK — both addresses hash correctly from their pubkeys.');
  console.log();

  // ---- 2. Build P2PKH lockingScripts + verify round-trip ------------------
  const coordLock = p2pkhScriptHex(coord.addr);
  const workerLock = p2pkhScriptHex(worker.addr);
  console.log(`[2] P2PKH lockingScripts:`);
  console.log(`    Coord  : ${coordLock}`);
  console.log(`    Worker : ${workerLock}`);
  // Sanity: standard P2PKH is exactly 50 hex chars (25 bytes):
  //   76 a9 14 <20-byte hash> 88 ac
  if (coordLock.length !== 50 || !/^76a914[0-9a-f]{40}88ac$/.test(coordLock)) {
    throw new Error(`Coord lockingScript is not standard P2PKH: ${coordLock}`);
  }
  if (workerLock.length !== 50 || !/^76a914[0-9a-f]{40}88ac$/.test(workerLock)) {
    throw new Error(`Worker lockingScript is not standard P2PKH: ${workerLock}`);
  }
  console.log('    Both scripts are well-formed standard P2PKH.');
  console.log();

  // ---- 3. Captain createAction with SPLIT_COUNT outputs per recipient ----
  const coordPerOutput = Math.floor(FUND_COORD_SATS / SPLIT_COUNT);
  const workerPerOutput = Math.floor(FUND_WORKER_SATS / SPLIT_COUNT);
  console.log(`[3] Calling Captain /createAction with ${SPLIT_COUNT * 2} outputs (${SPLIT_COUNT} per recipient)...`);
  console.log(`    Coord:  ${SPLIT_COUNT} × ${coordPerOutput.toLocaleString()} sats`);
  console.log(`    Worker: ${SPLIT_COUNT} × ${workerPerOutput.toLocaleString()} sats`);

  const outputs = [];
  // Interleave: coord0, worker0, coord1, worker1, ... — so vout indices are
  // predictable: coord at even indices (0,2,4,...), worker at odd (1,3,5,...).
  for (let i = 0; i < SPLIT_COUNT; i++) {
    outputs.push({
      satoshis: coordPerOutput,
      lockingScript: coordLock,
      outputDescription: `fund cascade Coordinator (split ${i + 1}/${SPLIT_COUNT})`,
    });
    outputs.push({
      satoshis: workerPerOutput,
      lockingScript: workerLock,
      outputDescription: `fund cascade Worker (split ${i + 1}/${SPLIT_COUNT})`,
    });
  }

  const createResult = await httpPost(CAPTAIN_WALLET_PORT, 'createAction', {
    description: 'multi-worm cascade test funding (split)',
    outputs,
    options: {
      acceptDelayedBroadcast: false,
    },
  });

  const txid = createResult.txid;
  if (!txid) {
    console.error('FAIL: no txid in createAction response:', JSON.stringify(createResult).substring(0, 500));
    process.exit(1);
  }
  console.log(`    txid: ${txid}`);
  console.log();

  // ---- 4. Wait briefly so WoC indexes the tx -----------------------------
  console.log('[4] Waiting 5s for the tx to be indexed on WhatsOnChain...');
  await new Promise(r => setTimeout(r, 5000));

  // ---- 5. Fund each vout on each recipient (SPLIT_COUNT vouts per side) -
  console.log('[5] Internalizing each split output on Coord + Worker...');
  let fundedOk = 0;
  let fundedFail = 0;
  for (let i = 0; i < SPLIT_COUNT; i++) {
    const coordVout = i * 2;
    const workerVout = i * 2 + 1;
    try {
      const out = cliFund(`http://localhost:${COORD_WALLET_PORT}`, txid, coordVout, '1');
      const bal = out.match(/Balance:\s+([\d,]+)\s+sats/)?.[1] || '?';
      console.log(`    Coord  vout=${coordVout}: balance ${bal} sats`);
      fundedOk++;
    } catch (e) {
      console.error(`    Coord  vout=${coordVout}: FAIL ${e.message}`);
      fundedFail++;
    }
    try {
      const out = cliFund(`http://localhost:${WORKER_WALLET_PORT}`, txid, workerVout, '1');
      const bal = out.match(/Balance:\s+([\d,]+)\s+sats/)?.[1] || '?';
      console.log(`    Worker vout=${workerVout}: balance ${bal} sats`);
      fundedOk++;
    } catch (e) {
      console.error(`    Worker vout=${workerVout}: FAIL ${e.message}`);
      fundedFail++;
    }
  }
  console.log(`    ${fundedOk} internalizations succeeded, ${fundedFail} failed.`);
  console.log();

  console.log('='.repeat(70));
  console.log(`  DONE — funded Coord +${FUND_COORD_SATS.toLocaleString()} sats, Worker +${FUND_WORKER_SATS.toLocaleString()} sats`);
  console.log(`  txid: ${txid}`);
  console.log('='.repeat(70));
}

main().catch((e) => {
  console.error('\nFATAL:', e.message);
  if (e.stack) console.error(e.stack);
  process.exit(1);
});
