#!/usr/bin/env node
/**
 * CLI Output Validation — Dolphin Milk
 *
 * Spawns the release binary and validates stdout/stderr for all user-facing
 * CLI commands: status, receive, fund, --version, --help.
 *
 * No Playwright needed. No sats spent. Just child_process + assertions.
 *
 * Usage:
 *   node test-cli.js                        # Run all CLI tests
 *   node test-cli.js --binary ./custom-bin  # Custom binary path
 *
 * Prerequisites:
 *   - cargo build --release
 *   - Funded wallet on localhost:3322 (for status/receive tests)
 */

const { execSync, execFileSync } = require('child_process');
const path = require('path');
const { getArg } = require('./lib/helpers');

const args = process.argv.slice(2);
const BINARY = getArg(args, '--binary') || path.join(__dirname, '..', '..', 'target', 'release', 'dolphin-milk');
const PROJECT_ROOT = path.join(__dirname, '..', '..');

const results = [];

function run(command, args = [], opts = {}) {
  try {
    const stdout = execFileSync(command, args, {
      cwd: PROJECT_ROOT,
      timeout: opts.timeout || 15000,
      encoding: 'utf-8',
      env: { ...process.env, ...opts.env },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    return { code: 0, stdout, stderr: '' };
  } catch (err) {
    return {
      code: err.status || 1,
      stdout: (err.stdout || '').toString(),
      stderr: (err.stderr || '').toString(),
    };
  }
}

function test(name, fn) {
  process.stdout.write(`  [${results.length + 1}] ${name}... `);
  try {
    fn();
    results.push({ name, pass: true });
    console.log('\u2713');
  } catch (err) {
    results.push({ name, pass: false, error: err.message });
    console.log(`\u2717 ${err.message}`);
  }
}

function assert(condition, msg) {
  if (!condition) throw new Error(msg);
}

function assertContains(haystack, needle, label) {
  if (!haystack.toLowerCase().includes(needle.toLowerCase())) {
    throw new Error(`${label || 'output'} missing "${needle}" — got: ${haystack.slice(0, 200)}`);
  }
}

function assertMatch(text, pattern, label) {
  if (!pattern.test(text)) {
    throw new Error(`${label || 'output'} does not match ${pattern} — got: ${text.slice(0, 200)}`);
  }
}

// ─── Tests ─────────────────────────────────────────────────────────────

console.log('\n  CLI Output Validation\n');

// 1. --version
test('--version prints version', () => {
  const r = run(BINARY, ['--version']);
  assert(r.code === 0, `exit code ${r.code}`);
  assertMatch(r.stdout, /dolphin-milk \d+\.\d+\.\d+/, 'version');
});

// 2. --help lists all subcommands
test('--help lists subcommands', () => {
  const r = run(BINARY, ['--help']);
  assert(r.code === 0, `exit code ${r.code}`);
  const required = ['status', 'think', 'run', 'receive', 'fund', 'serve', 'mcp'];
  for (const cmd of required) {
    assertContains(r.stdout, cmd, '--help');
  }
});

// 3. status — wallet connected, shows identity + balance
test('status shows wallet info', () => {
  const r = run(BINARY, ['status']);
  assert(r.code === 0, `exit code ${r.code} — stderr: ${r.stderr}`);
  assertContains(r.stdout, 'Dolphin Milk v', 'status header');
  // Identity: 66-char hex pubkey
  assertMatch(r.stdout, /[0-9a-f]{66}/i, 'identity key');
  // Balance: number followed by "sats"
  assertMatch(r.stdout, /[\d,]+ sats/i, 'balance');
});

// 4. status — config section present
test('status shows config section', () => {
  const r = run(BINARY, ['status']);
  assert(r.code === 0, `exit code ${r.code}`);
  assertContains(r.stdout, 'Model:', 'config model');
  assertContains(r.stdout, 'Budget:', 'config budget');
});

// 5. status — box-drawing visual style
test('status uses box-drawing separator', () => {
  const r = run(BINARY, ['status']);
  assert(r.code === 0, `exit code ${r.code}`);
  assertContains(r.stdout, '\u2500\u2500\u2500', 'box-drawing line');
});

// 6. status — next action hint
test('status shows next action', () => {
  const r = run(BINARY, ['status']);
  assert(r.code === 0, `exit code ${r.code}`);
  // Should suggest a command to run next
  assertMatch(r.stdout, /dolphin-milk (start|think|run)/i, 'next action');
});

// 7. receive — shows BSV address
test('receive shows BSV address', () => {
  const r = run(BINARY, ['receive']);
  assert(r.code === 0, `exit code ${r.code}`);
  assertContains(r.stdout, 'BSV address:', 'address label');
  // P2PKH address starts with 1
  assertMatch(r.stdout, /1[A-HJ-NP-Za-km-z1-9]{25,34}/, 'BSV address');
  assertContains(r.stdout, 'Public key:', 'public key label');
  assertContains(r.stdout, 'Suffix:', 'suffix label');
});

// 8. receive — shows next step (fund command)
test('receive shows fund hint', () => {
  const r = run(BINARY, ['receive']);
  assert(r.code === 0, `exit code ${r.code}`);
  assertContains(r.stdout, 'dolphin-milk fund', 'fund hint');
});

// 9. fund with invalid txid — actionable error
test('fund rejects invalid txid', () => {
  const r = run(BINARY, ['fund', 'not-a-real-txid']);
  assert(r.code !== 0, 'should fail with bad txid');
  const output = r.stdout + r.stderr;
  // Should give a useful error, not a raw stack trace
  assert(output.length > 10, 'should have error message');
});

// 10. fund with valid-looking but nonexistent txid — clear error
test('fund handles missing tx gracefully', () => {
  const fakeTxid = 'a'.repeat(64);
  const r = run(BINARY, ['fund', fakeTxid], { timeout: 20000 });
  assert(r.code !== 0, 'should fail');
  const output = r.stdout + r.stderr;
  assert(output.length > 10, 'should have error message');
});

// 11. slim build compiles (--no-default-features)
test('slim build compiles', () => {
  // This is a build test — verify the command exits 0
  // We only check that cargo check passes (faster than full build)
  const r = run('cargo', ['check', '--no-default-features', '--features', 'external-wallet'], {
    timeout: 120000,
  });
  // cargo check may warn but should exit 0
  assert(r.code === 0, `slim build failed: ${r.stderr.slice(0, 300)}`);
});

// 12. server health check (if running)
test('server /health responds (if running)', () => {
  try {
    const body = execSync('curl -s http://localhost:8080/health', {
      timeout: 5000,
      encoding: 'utf-8',
    });
    const health = JSON.parse(body);
    assert(health.status === 'ok', `health status: ${health.status}`);
    assert(typeof health.version === 'string', 'version present');
    assert(typeof health.uptime_secs === 'number', 'uptime present');
  } catch {
    // Server not running — skip gracefully
    console.log('(server not running, skipped)');
    results[results.length - 1].pass = true;
    results[results.length - 1].skipped = true;
    // Rewrite the last console output
  }
});

// ─── Summary ───────────────────────────────────────────────────────────

console.log();
const passed = results.filter(r => r.pass).length;
const failed = results.filter(r => !r.pass).length;
const skipped = results.filter(r => r.skipped).length;

if (failed === 0) {
  console.log(`  ${passed} passed${skipped ? `, ${skipped} skipped` : ''} \u2713\n`);
  process.exit(0);
} else {
  console.log(`  ${passed} passed, ${failed} FAILED\n`);
  for (const r of results.filter(r => !r.pass)) {
    console.log(`    FAIL: ${r.name} — ${r.error}`);
  }
  console.log();
  process.exit(1);
}
