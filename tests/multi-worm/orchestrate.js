#!/usr/bin/env node
/**
 * Multi-Worm Orchestrator
 *
 * Starts multiple Dolphin Milk agent processes, health-checks them, discovers
 * their on-chain identities, and exposes template variables for test scenarios.
 *
 * Can be used as a module (import WormOrchestrator) or run standalone:
 *
 *   node orchestrate.js                          # Start all agents, wait for Ctrl-C
 *   node orchestrate.js --status-only            # Start, print status, stop
 *   node orchestrate.js --agents alice           # Start only alice
 *   node orchestrate.js --config custom.json     # Use custom config file
 *
 * Exports: { WormOrchestrator, httpGet }
 */

const { spawn } = require('child_process');
const http = require('http');
const fs = require('fs');
const path = require('path');
const { authGet } = require('./lib/auth');

// ---------------------------------------------------------------------------
// HTTP helper — promise wrapper around Node.js built-in http.get()
// ---------------------------------------------------------------------------

/**
 * Perform an HTTP GET request and return { status, body }.
 * Body is parsed as JSON if possible, otherwise returned as a raw string.
 * Times out after 5 seconds.
 */
function httpGet(url) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => {
        try { resolve({ status: res.statusCode, body: JSON.parse(data) }); }
        catch { resolve({ status: res.statusCode, body: data }); }
      });
    });
    req.on('error', reject);
    req.setTimeout(5000, () => { req.destroy(); reject(new Error('timeout')); });
  });
}

/**
 * Perform an HTTP POST request and return { status, body }.
 */
function httpPost(url, body) {
  return new Promise((resolve, reject) => {
    const urlObj = new URL(url);
    const data = JSON.stringify(body || {});
    const req = http.request({
      hostname: urlObj.hostname,
      port: urlObj.port,
      path: urlObj.pathname,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Origin': `http://localhost:${urlObj.port}`,
        'Content-Length': Buffer.byteLength(data),
      },
    }, (res) => {
      let raw = '';
      res.on('data', chunk => raw += chunk);
      res.on('end', () => {
        try { resolve({ status: res.statusCode, body: JSON.parse(raw) }); }
        catch { resolve({ status: res.statusCode, body: raw }); }
      });
    });
    req.on('error', reject);
    req.setTimeout(5000, () => { req.destroy(); reject(new Error('timeout')); });
    req.write(data);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// WormOrchestrator
// ---------------------------------------------------------------------------

class WormOrchestrator {
  /**
   * @param {object} [config] — parsed config object. If omitted, loads config.json
   *                             from the same directory as this script.
   */
  constructor(config) {
    if (!config) {
      const configPath = path.join(__dirname, 'config.json');
      config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
    }

    this.config = config;

    /** @type {Map<string, { process: ChildProcess, port: number, walletPort: number, workspace: string }>} */
    this.processes = new Map();

    /** @type {Map<string, { identity_key: string, balance: number }>} */
    this.identities = new Map();

    // Project root is two levels up from tests/multi-worm/
    this.projectRoot = path.resolve(__dirname, '../../');

    // Shutdown guard — prevents double stopAll() from signal handlers
    this._shuttingDown = false;
  }

  // -----------------------------------------------------------------------
  // Public API
  // -----------------------------------------------------------------------

  /**
   * Start all configured agents, health-check them, and discover identities.
   * @returns {WormOrchestrator} this — for chaining
   */
  async startAll() {
    const agentNames = Object.keys(this.config.agents);

    for (const name of agentNames) {
      await this.startAgent(name, this.config.agents[name]);
    }

    // Ensure all agents have BRC-52 certificates (required to accept tasks)
    await this.ensureCertificates();

    return this;
  }

  /**
   * Ensure all agents have BRC-52 authorization certificates.
   * Issues a cert via the parent wallet if the agent doesn't have one.
   */
  async ensureCertificates() {
    const parentPort = this.config.parent_wallet_port;
    if (!parentPort) return;

    for (const [name, entry] of this.processes) {
      const { authGet: aGet, authPost: aPost } = require('./lib/auth');
      try {
        const certResp = await aGet(`http://localhost:${entry.port}/certificates`, parentPort);
        if (certResp.status === 200 && certResp.body && certResp.body.valid === true) {
          continue; // Already has a valid cert
        }
      } catch { /* no cert, issue one */ }

      // Issue a certificate
      process.stdout.write(`  Issuing cert for ${name}... `);
      try {
        const issueResp = await aPost(`http://localhost:${entry.port}/certificates/issue`, {
          name: `${name}-test-agent`,
          capabilities: 'messaging,code-analysis,browser,wallet,tools',
        }, parentPort);
        if (issueResp.status === 200 || issueResp.status === 201) {
          console.log('✓');
        } else {
          console.log(`⚠ status ${issueResp.status}`);
        }
      } catch (err) {
        console.log(`⚠ ${err.message}`);
      }
    }
  }

  /**
   * Spawn a single worm agent process.
   *
   * @param {string} name       — agent name (e.g. "alice")
   * @param {object} agentConfig — { worm_port, wallet_port, workspace }
   */
  async startAgent(name, agentConfig) {
    const { worm_port, wallet_port, workspace } = agentConfig;
    const binaryPath = path.resolve(this.projectRoot, this.config.binary);
    const workspacePath = path.resolve(this.projectRoot, workspace);

    // Validate binary exists
    if (!fs.existsSync(binaryPath)) {
      throw new Error(
        `Binary not found at ${binaryPath}. Run \`cargo build --release\` first.`
      );
    }

    // Ensure workspace directory exists
    fs.mkdirSync(workspacePath, { recursive: true });

    // Open log files for stdout/stderr
    const stdoutLog = fs.openSync(path.join(workspacePath, 'worm-stdout.log'), 'w');
    const stderrLog = fs.openSync(path.join(workspacePath, 'worm-stderr.log'), 'w');

    const startTime = Date.now();
    process.stdout.write(`Starting ${name} (port ${worm_port}, wallet ${wallet_port})... `);

    // Spawn the worm process
    const child = spawn(
      binaryPath,
      ['serve', '--port', String(worm_port), '--workspace', workspacePath],
      {
        cwd: this.projectRoot,
        env: {
          ...process.env,
          DOLPHIN_MILK_WALLET_URL: `http://localhost:${wallet_port}`,
          DOLPHIN_MILK_HEARTBEAT_ENABLED: 'false',
        },
        stdio: ['ignore', stdoutLog, stderrLog],
        detached: false,
      }
    );

    // Track an early-exit promise so we can detect crashes during startup
    let earlyExit = null;
    const exitPromise = new Promise((resolve) => {
      child.on('exit', (code, signal) => {
        earlyExit = { code, signal };
        resolve();
      });
    });

    this.processes.set(name, {
      process: child,
      port: worm_port,
      walletPort: wallet_port,
      workspace: workspacePath,
    });

    // Health check — poll until the agent is ready
    try {
      await this.healthCheck(name, worm_port, this.config.health_timeout_ms, exitPromise);
    } catch (err) {
      // If the process exited during health check, include that info
      if (earlyExit) {
        const stderr = this._readLogTail(path.join(workspacePath, 'worm-stderr.log'));
        throw new Error(
          `${name} exited during startup (code=${earlyExit.code}, signal=${earlyExit.signal}).` +
          (stderr ? `\nStderr:\n${stderr}` : '')
        );
      }
      throw err;
    }

    // Discover identity
    await this.discoverIdentity(name, worm_port);

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
    console.log(`\u2713 ${elapsed}s`);
  }

  /**
   * Poll GET /health until status === "ok" and wallet_connected === true.
   *
   * @param {string}  name      — agent name (for error messages)
   * @param {number}  port      — worm HTTP port
   * @param {number}  timeoutMs — max wait time
   * @param {Promise} [earlyExitPromise] — resolves if the child exits early
   */
  async healthCheck(name, port, timeoutMs, earlyExitPromise) {
    const deadline = Date.now() + timeoutMs;
    const pollMs = this.config.health_poll_ms || 1000;
    let lastError = null;

    while (Date.now() < deadline) {
      // Check if the process died while we were waiting
      if (earlyExitPromise) {
        const raced = await Promise.race([
          this._sleep(pollMs).then(() => 'poll'),
          earlyExitPromise.then(() => 'exited'),
        ]);
        if (raced === 'exited') {
          throw new Error(`${name} process exited during health check`);
        }
      } else {
        await this._sleep(pollMs);
      }

      try {
        const { status, body } = await httpGet(`http://localhost:${port}/health`);
        if (
          status === 200 &&
          body &&
          body.status === 'ok' &&
          body.wallet_connected === true
        ) {
          return; // healthy
        }
        lastError = `status=${status}, body=${JSON.stringify(body)}`;
      } catch (err) {
        lastError = err.message;
      }
    }

    throw new Error(
      `Health check timed out for ${name} on port ${port} after ${timeoutMs}ms. ` +
      `Last error: ${lastError}`
    );
  }

  /**
   * Fetch agent identity and balance from GET /agent.
   * Uses BRC-31 Authrite via the parent wallet when parent_wallet_port is configured.
   *
   * @param {string} name — agent name
   * @param {number} port — worm HTTP port
   */
  async discoverIdentity(name, port) {
    // Use BRC-31 authenticated request to /agent via parent wallet.
    const parentPort = this.config.parent_wallet_port;
    if (!parentPort) {
      throw new Error(`parent_wallet_port not configured — required for BRC-31 auth`);
    }

    const { status, body } = await authGet(`http://localhost:${port}/agent`, parentPort);
    if (status !== 200 || !body || !body.identity_key) {
      throw new Error(
        `Failed to discover identity for ${name}: status=${status}, body=${JSON.stringify(body)}`
      );
    }

    this.identities.set(name, {
      identity_key: body.identity_key,
      balance: body.balance || 0,
    });
  }

  /**
   * Stop all running agents gracefully.
   * Sends SIGTERM first, then SIGKILL after 5 seconds if still alive.
   */
  async stopAll() {
    if (this._shuttingDown) return;
    this._shuttingDown = true;

    const names = Array.from(this.processes.keys());
    await Promise.all(names.map(name => this.stopAgent(name)));
  }

  /**
   * Stop a single agent by name.
   * Sends SIGTERM, waits up to 5 seconds, then SIGKILL.
   *
   * @param {string} name — agent name
   */
  async stopAgent(name) {
    const entry = this.processes.get(name);
    if (!entry) return;

    const { process: child } = entry;

    // Already exited — nothing to do
    if (child.exitCode !== null || child.killed) {
      this.processes.delete(name);
      return;
    }

    return new Promise((resolve) => {
      const killTimer = setTimeout(() => {
        try { child.kill('SIGKILL'); } catch { /* already dead */ }
      }, 5000);

      child.on('exit', () => {
        clearTimeout(killTimer);
        this.processes.delete(name);
        resolve();
      });

      try { child.kill('SIGTERM'); } catch { /* already dead */ }
    });
  }

  /**
   * Build template variable map for use in test scenarios.
   *
   * Returns an object like:
   *   {
   *     "alice.identity_key": "034aa44...",
   *     "alice.worm_url": "http://localhost:8080",
   *     "bob.identity_key": "03def...",
   *     "bob.worm_url": "http://localhost:8081",
   *   }
   */
  getTemplateVars() {
    const vars = {};

    for (const [name, entry] of this.processes) {
      const identity = this.identities.get(name);
      vars[`${name}.worm_url`] = `http://localhost:${entry.port}`;
      if (identity) {
        vars[`${name}.identity_key`] = identity.identity_key;
      }
    }

    return vars;
  }

  /**
   * Replace {{variable}} placeholders in a string with template vars.
   *
   * @param {string} str — string with {{alice.identity_key}} etc.
   * @returns {string} — resolved string
   */
  resolveTemplate(str) {
    const vars = this.getTemplateVars();
    return str.replace(/\{\{(\w+\.\w+)\}\}/g, (match, key) => {
      return vars[key] !== undefined ? vars[key] : match;
    });
  }

  // -----------------------------------------------------------------------
  // Private helpers
  // -----------------------------------------------------------------------

  /** Sleep for ms milliseconds. */
  _sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  /** Read the last ~2KB of a log file for error reporting. */
  _readLogTail(filepath) {
    try {
      const content = fs.readFileSync(filepath, 'utf8');
      return content.slice(-2048);
    } catch {
      return '';
    }
  }
}

// ---------------------------------------------------------------------------
// Standalone CLI
// ---------------------------------------------------------------------------

async function main() {
  const args = process.argv.slice(2);

  // --config PATH
  let config = null;
  const configIdx = args.indexOf('--config');
  if (configIdx !== -1 && args[configIdx + 1]) {
    const configPath = path.resolve(args[configIdx + 1]);
    config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
  }

  const statusOnly = args.includes('--status-only');

  // --agents alice,bob (subset filter)
  let agentFilter = null;
  const agentsIdx = args.indexOf('--agents');
  if (agentsIdx !== -1 && args[agentsIdx + 1]) {
    agentFilter = args[agentsIdx + 1].split(',').map(s => s.trim());
  }

  const orchestrator = new WormOrchestrator(config);

  // If --agents filter is set, remove agents not in the list
  if (agentFilter) {
    for (const name of Object.keys(orchestrator.config.agents)) {
      if (!agentFilter.includes(name)) {
        delete orchestrator.config.agents[name];
      }
    }
    // Validate all requested agents exist
    for (const name of agentFilter) {
      if (!orchestrator.config.agents[name]) {
        console.error(`Error: agent "${name}" not found in config.`);
        process.exit(1);
      }
    }
  }

  const agentNames = Object.keys(orchestrator.config.agents);

  // Banner
  console.log(`\n\uD83D\uDC1B Multi-Worm Orchestrator`);
  console.log(`   Config: ${agentNames.length} agents (${agentNames.join(', ')})`);
  console.log(`   Binary: ${orchestrator.config.binary}\n`);

  // Register signal handlers for graceful shutdown
  let signalReceived = false;
  const shutdown = async (sig) => {
    if (signalReceived) return;
    signalReceived = true;
    console.log(`\n\nReceived ${sig}. Stopping all agents...`);
    await orchestrator.stopAll();
    console.log('All agents stopped.');
    process.exit(0);
  };
  process.on('SIGINT', () => shutdown('SIGINT'));
  process.on('SIGTERM', () => shutdown('SIGTERM'));

  // Start all agents
  try {
    await orchestrator.startAll();
  } catch (err) {
    console.log(`\u2717 FAILED`);
    console.error(`\nError: ${err.message}`);
    await orchestrator.stopAll();
    process.exit(1);
  }

  // Agent discovery summary
  console.log(`\nAgent Discovery:`);
  for (const [name, identity] of orchestrator.identities) {
    const truncKey = identity.identity_key.substring(0, 9) + '...';
    const balStr = identity.balance.toLocaleString();
    console.log(`  ${name}: ${truncKey} (${balStr} sats)`);
  }

  // Low balance warnings
  for (const [name, identity] of orchestrator.identities) {
    if (identity.balance < 100000) {
      console.log(
        `\n\u26A0 ${name} balance low: ${identity.balance.toLocaleString()} sats ` +
        `\u2014 fund before running scenarios`
      );
    }
  }

  // Template variables
  const vars = orchestrator.getTemplateVars();
  console.log(`\nTemplate Variables:`);
  for (const [key, value] of Object.entries(vars)) {
    // Pad the key for alignment
    const padded = key.padEnd(22);
    console.log(`  ${padded} = ${value}`);
  }

  if (statusOnly) {
    console.log(`\nStatus check complete. Stopping agents...`);
    await orchestrator.stopAll();
    console.log('Done.');
    process.exit(0);
  }

  console.log(`\nReady. Press Ctrl-C to stop all agents.`);
}

// ---------------------------------------------------------------------------
// Run standalone if executed directly
// ---------------------------------------------------------------------------

if (require.main === module) {
  main().catch(async (err) => {
    console.error(`\nFatal: ${err.message}`);
    process.exit(1);
  });
}

// ---------------------------------------------------------------------------
// Module exports
// ---------------------------------------------------------------------------

module.exports = { WormOrchestrator, httpGet };
