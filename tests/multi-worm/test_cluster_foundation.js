#!/usr/bin/env node
/**
 * test_cluster_foundation.js — Phase 1/2 smoke test for lib/cluster.js.
 *
 * Brings up a 2-agent cluster (captain + worker), asserts the contract
 * shape of the ClusterHandle and cluster-state.json (including Phase 2
 * cert issuance fields), then tears it down.
 *
 * NOT run as part of any automated suite yet — Phase 3 integration test
 * (test_poc_23.js) will exercise this module end-to-end. This file is
 * invoked manually:
 *
 *     node tests/multi-worm/test_cluster_foundation.js
 *     node tests/multi-worm/test_cluster_foundation.js --verify-overlay
 *
 * Preconditions:
 *   - cargo build --release (binary at target/release/dolphin-milk)
 *   - Parent wallet running on port 3321 (MetaNet Client)
 *   - Agent wallets running on ports 3322 and 3324
 *
 * CLI flags:
 *   --verify-overlay  Enable step 6 overlay registration verification.
 *                     Requires Agent F's overlay re-registration patch to be
 *                     live (cargo build --release after the patch lands).
 *                     Default off so the test runs independently of Agent F.
 */

'use strict';

const path = require('path');
const fs = require('fs');
const assert = require('assert').strict;

const { startCluster, stopCluster } = require('./lib/cluster');

const PROJECT_ROOT = path.resolve(__dirname, '../../');
const BINARY = path.join(PROJECT_ROOT, 'target/release/dolphin-milk');

function isHex66(s) {
  return typeof s === 'string' && /^[0-9a-fA-F]{66}$/.test(s);
}

async function main() {
  const verifyOverlay = process.argv.includes('--verify-overlay');

  const outputDir = path.join(PROJECT_ROOT, 'test-workspaces/cluster-foundation-test');
  fs.mkdirSync(outputDir, { recursive: true });

  const config = {
    parentWalletPort: 3321,
    binary: BINARY,
    overlay: {
      url: 'https://rust-overlay.dev-a3e.workers.dev',
      verifyRegistration: verifyOverlay,
      registrationTimeoutMs: 30000,
    },
    outputDir,
    agents: [
      {
        name: 'captain',
        port: 8081,
        walletPort: 3322,
        model: 'claude-haiku-4-5',
        workspace: path.join(PROJECT_ROOT, 'test-workspaces/cluster-foundation-captain'),
        capabilities: ['orchestration', 'intelligence'],
      },
      {
        name: 'worker',
        port: 8083,
        walletPort: 3324,
        model: 'claude-haiku-4-5',
        workspace: path.join(PROJECT_ROOT, 'test-workspaces/cluster-foundation-worker'),
        capabilities: ['scraping', 'web_fetch', 'execute_bash'],
      },
    ],
  };

  console.log('[test] startCluster...');
  const handle = await startCluster(config);

  // ---- ClusterHandle shape assertions ----
  console.log('[test] asserting ClusterHandle shape...');
  assert.ok(handle, 'handle must be returned');
  assert.ok(handle.agents instanceof Map, 'handle.agents must be a Map');
  assert.equal(handle.agents.size, 2, 'handle.agents must have exactly 2 entries');
  assert.ok(handle.agents.has('captain'), 'handle.agents must contain "captain"');
  assert.ok(handle.agents.has('worker'), 'handle.agents must contain "worker"');
  assert.equal(typeof handle.stateFile, 'string', 'handle.stateFile must be a string');
  assert.ok(path.isAbsolute(handle.stateFile), 'handle.stateFile must be absolute');
  assert.ok(fs.existsSync(handle.stateFile), `cluster-state.json must exist at ${handle.stateFile}`);
  assert.ok(isHex66(handle.parentKey), 'handle.parentKey must be 66-char hex');
  assert.equal(handle.overlayUrl, config.overlay.url, 'handle.overlayUrl must match config');
  assert.ok(typeof handle.createdAt === 'string', 'handle.createdAt must be a string');
  assert.ok(!Number.isNaN(Date.parse(handle.createdAt)), 'handle.createdAt must be a parseable ISO date');
  assert.equal(typeof handle.stop, 'function', 'handle.stop must be a function');

  // ---- AgentHandle shape assertions ----
  console.log('[test] asserting AgentHandle shapes...');
  const seenKeys = new Set();
  for (const name of ['captain', 'worker']) {
    const agent = handle.agents.get(name);
    assert.ok(agent, `agent ${name} must exist`);
    assert.equal(agent.name, name);
    assert.equal(typeof agent.port, 'number');
    assert.equal(typeof agent.walletPort, 'number');
    assert.ok(isHex66(agent.identityKey), `${name}.identityKey must be 66-char hex, got: ${agent.identityKey}`);
    assert.ok(!seenKeys.has(agent.identityKey), `${name}.identityKey must be unique across agents`);
    seenKeys.add(agent.identityKey);
    assert.equal(typeof agent.workspace, 'string');
    assert.ok(path.isAbsolute(agent.workspace), `${name}.workspace must be absolute`);
    // Phase 2: cert issuance populated
    assert.ok(agent.certHash, `${name} should have certHash`);
    assert.equal(typeof agent.certHash, 'string', `${name}.certHash must be a string`);
    assert.ok(Array.isArray(agent.certCapabilities), `${name} should have certCapabilities array`);
    assert.ok(agent.certCapabilities.length > 0, `${name} should have non-empty capabilities`);
    // Declared capabilities must all be present on the granted cert.
    for (const declared of (config.agents.find((a) => a.name === name).capabilities || [])) {
      assert.ok(
        agent.certCapabilities.includes(declared),
        `${name}.certCapabilities must include declared '${declared}' (granted: ${agent.certCapabilities.join(',')})`,
      );
    }
    // Phase 2: overlay registration — enforced only when verifyRegistration=true.
    if (config.overlay.verifyRegistration) {
      assert.strictEqual(
        agent.overlayRegistered, true,
        `${name} should be registered on overlay when verifyRegistration=true`,
      );
      assert.ok(
        typeof agent.overlayRegisteredAt === 'string' && agent.overlayRegisteredAt.length > 0,
        `${name} should have overlayRegisteredAt ISO timestamp`,
      );
      assert.ok(
        !Number.isNaN(Date.parse(agent.overlayRegisteredAt)),
        `${name}.overlayRegisteredAt must parse as an ISO date`,
      );
    } else {
      // When skipped, remain in the Phase 1 default state.
      assert.equal(agent.overlayRegistered, false, `${name}.overlayRegistered must be false when verifyRegistration=false`);
      assert.equal(agent.overlayRegisteredAt, null, `${name}.overlayRegisteredAt must be null when verifyRegistration=false`);
    }
    assert.equal(typeof agent.spawnedByUs, 'boolean');
    if (agent.spawnedByUs) {
      assert.ok(agent.proc, `${name}.proc must be non-null when spawnedByUs=true`);
      assert.equal(typeof agent.stdoutLogPath, 'string');
      assert.equal(typeof agent.stderrLogPath, 'string');
      assert.ok(path.isAbsolute(agent.stdoutLogPath));
      assert.ok(path.isAbsolute(agent.stderrLogPath));
    } else {
      assert.equal(agent.proc, null, `${name}.proc must be null when spawnedByUs=false`);
    }
  }

  // ---- cluster-state.json shape assertions ----
  console.log('[test] asserting cluster-state.json contents...');
  const raw = fs.readFileSync(handle.stateFile, 'utf8');
  const parsed = JSON.parse(raw);
  assert.equal(parsed.createdAt, handle.createdAt);
  assert.equal(parsed.parentKey, handle.parentKey);
  assert.equal(parsed.overlayUrl, handle.overlayUrl);
  assert.ok(parsed.agents && typeof parsed.agents === 'object');
  assert.deepEqual(
    Object.keys(parsed.agents).sort(),
    ['captain', 'worker'],
    'state file agents keys must be [captain, worker]',
  );
  for (const name of ['captain', 'worker']) {
    const a = parsed.agents[name];
    assert.equal(a.name, name);
    assert.equal(typeof a.port, 'number');
    assert.equal(typeof a.walletPort, 'number');
    assert.ok(isHex66(a.identityKey));
    assert.ok(path.isAbsolute(a.workspace));
    assert.ok(a.certHash && typeof a.certHash === 'string', `state file ${name}.certHash must be a non-empty string`);
    assert.ok(Array.isArray(a.certCapabilities) && a.certCapabilities.length > 0,
      `state file ${name}.certCapabilities must be a non-empty array`);
    if (config.overlay.verifyRegistration) {
      assert.equal(a.overlayRegistered, true);
      assert.ok(typeof a.overlayRegisteredAt === 'string' && a.overlayRegisteredAt.length > 0);
    } else {
      assert.equal(a.overlayRegistered, false);
      assert.equal(a.overlayRegisteredAt, null);
    }
    assert.equal(typeof a.spawnedByUs, 'boolean');
    // stdoutLogPath / stderrLogPath are absolute strings when spawnedByUs, else null
    if (a.spawnedByUs) {
      assert.ok(path.isAbsolute(a.stdoutLogPath));
      assert.ok(path.isAbsolute(a.stderrLogPath));
    } else {
      assert.equal(a.stdoutLogPath, null);
      assert.equal(a.stderrLogPath, null);
    }
  }

  // ---- stopCluster ----
  console.log('[test] stopCluster...');
  const stopStart = Date.now();
  await stopCluster(handle);
  const stopElapsed = Date.now() - stopStart;
  assert.ok(stopElapsed < 10000, `stopCluster should complete within 10s, took ${stopElapsed}ms`);

  // After stop, every spawned agent's proc should be null (shutdown idempotency).
  for (const agent of handle.agents.values()) {
    if (agent.spawnedByUs) {
      assert.equal(agent.proc, null, `${agent.name}.proc must be null after stopCluster`);
    }
  }

  // Second stop should be a safe no-op.
  await stopCluster(handle);

  console.log('[test] cluster foundation test PASSED');
  process.exit(0);
}

main().catch((e) => {
  console.error('[test] FAILED:', e.message);
  console.error(e.stack);
  process.exit(1);
});
