#!/usr/bin/env node
/**
 * Recovery sweep: spend the wrong-address UTXO at
 *   2d6ad71e9ca73c964bec6bf7c2560c9358a4dcd682fcd8adf858cc3e92470210:0
 * (1,050,000 sats P2PKH at the worker wallet's identity-key address)
 * and pay the proceeds to the worker's BRC-29 funding address so it can
 * be internalized into the spendable balance via `bsv-wallet fund`.
 *
 * The worker wallet does NOT track this UTXO, so we sign with the
 * identity root private key directly via @bsv/sdk's P2PKH.unlock template.
 */

const fs = require('fs');
const path = require('path');
const { Transaction, P2PKH, PrivateKey, Beef } = require('@bsv/sdk');

const STUCK_TXID = '2d6ad71e9ca73c964bec6bf7c2560c9358a4dcd682fcd8adf858cc3e92470210';
const STUCK_VOUT = 0;
const SOURCE_BEEF_PATH = '/tmp/worker-fund.beef';
const ROOT_KEY_ENV = path.join(process.env.HOME, 'bsv/wallets/worker-3324.env');
const BRC29_FUNDING_ADDR = '1GDzcxf7McziLpu4sFZGEvKJo6bXKrVty5';
const OUT_ATOMIC_HEX_PATH = '/tmp/worker-recover.atomic.hex';

async function main() {
  // 1. Load root key
  const envText = fs.readFileSync(ROOT_KEY_ENV, 'utf8');
  const m = envText.match(/^ROOT_KEY=([0-9a-f]+)/m);
  if (!m) throw new Error('ROOT_KEY not found in env file');
  const privKey = PrivateKey.fromHex(m[1]);
  console.log('worker identity addr (from privKey):', privKey.toAddress());

  // 2. Load source BEEF
  const beefHex = fs.readFileSync(SOURCE_BEEF_PATH, 'utf8').trim();
  const sourceBeef = Beef.fromString(beefHex);
  const sourceTx = sourceBeef.findAtomicTransaction(STUCK_TXID);
  if (!sourceTx) throw new Error(`source tx ${STUCK_TXID} not found in BEEF`);
  const sourceOutput = sourceTx.outputs[STUCK_VOUT];
  console.log('source output sats:', sourceOutput.satoshis);
  console.log('source lock asm:', sourceOutput.lockingScript.toASM());

  // 3. Build sweep tx
  const tx = new Transaction();
  tx.addInput({
    sourceTransaction: sourceTx,
    sourceOutputIndex: STUCK_VOUT,
    unlockingScriptTemplate: new P2PKH().unlock(privKey),
  });
  // Single output paying ALL remaining sats (after fee) to BRC-29 funding addr.
  // Using addOutput with `change: true` lets the sdk compute the value during fee().
  tx.addOutput({
    lockingScript: new P2PKH().lock(BRC29_FUNDING_ADDR),
    change: true,
  });

  // 4. Fee
  await tx.fee();
  const computedFee = tx.getFee();
  console.log('computed fee sats:', computedFee);
  if (computedFee > 1000) {
    throw new Error(`fee too high: ${computedFee} sats - aborting`);
  }
  console.log('output sats (after fee):', tx.outputs[0].satoshis);

  // 5. Sign
  await tx.sign();
  console.log('signed. tx size bytes:', tx.toBinary().length);

  // 6. Broadcast
  console.log('broadcasting...');
  const broadcastResult = await tx.broadcast();
  console.log('broadcast result:', JSON.stringify(broadcastResult, null, 2));

  // 7. Save AtomicBEEF for internalize
  const atomicHex = Buffer.from(tx.toAtomicBEEF()).toString('hex');
  fs.writeFileSync(OUT_ATOMIC_HEX_PATH, atomicHex);
  console.log('atomic beef written:', OUT_ATOMIC_HEX_PATH);
  console.log('new txid:', tx.id('hex'));
}

main().catch(e => {
  console.error('FATAL:', e.message);
  if (e.stack) console.error(e.stack);
  process.exit(1);
});
