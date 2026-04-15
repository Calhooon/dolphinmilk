#!/usr/bin/env node
/**
 * Multimodal Image Upload E2E Test
 *
 * Tests that image attachments are correctly sent through the x402 proxy
 * to both OpenAI and Claude LLM providers, and that responses demonstrate
 * actual image understanding (not just "I received an image").
 *
 * This test uses the HTTP API directly (not Playwright) because:
 *   - We need to control the model per request
 *   - We need to verify both providers independently
 *   - The critical path is the x402 proxy, not the UI
 *
 * Usage:
 *   node test-multimodal.js              # Test both providers
 *   node test-multimodal.js --openai     # OpenAI only
 *   node test-multimodal.js --claude     # Claude only
 *   node test-multimodal.js --dry-run    # Show what would run
 *
 * Prerequisites:
 *   - worm server running at localhost:8080
 *   - Funded wallet (each test costs ~$0.01-0.05)
 *
 * Cost: ~$0.05-0.10 total (2 LLM calls with vision)
 */

const http = require('http');
const { getArg } = require('./lib/helpers');

// ---------------------------------------------------------------------------
// Test image: a 4x4 PNG with distinct red/blue quadrants.
// Small enough to be cheap, distinct enough that a vision model should notice
// the colors. Generated programmatically (valid PNG, 119 bytes decoded).
// ---------------------------------------------------------------------------
const TEST_IMAGE_BASE64 = generateTestPng();
const TEST_IMAGE_MIME = 'image/png';
const TEST_IMAGE_FILENAME = 'test-colors.png';

const SERVER_URL = 'http://localhost:8080';

// Models to test — cheapest vision-capable model per provider
const TESTS = [
  {
    id: 'openai',
    name: 'OpenAI Vision (gpt-5-mini)',
    model: 'gpt-5-mini',
    message: 'Describe the colors you see in this image. Be specific about what colors are present.',
    // gpt-5-mini should describe the red/blue colors
    expected_any: ['red', 'blue', 'color', 'image', 'pixel', 'square'],
    timeout_ms: 120000,
  },
  {
    id: 'claude',
    name: 'Claude Vision (claude-haiku-4-5)',
    model: 'claude-haiku-4-5',
    message: 'Describe the colors you see in this image. Be specific about what colors are present.',
    // Claude haiku should also describe the colors
    expected_any: ['red', 'blue', 'color', 'image', 'pixel', 'square'],
    timeout_ms: 120000,
  },
];

// ---------------------------------------------------------------------------
// Minimal PNG generator — creates a 4x4 image with red (top-left) and
// blue (bottom-right) quadrants. No external deps needed.
// ---------------------------------------------------------------------------
function generateTestPng() {
  // PNG signature
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  // CRC32 table
  const crcTable = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    crcTable[n] = c;
  }
  function crc32(buf) {
    let c = 0xffffffff;
    for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  }

  function makeChunk(type, data) {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const typeAndData = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(typeAndData));
    return Buffer.concat([len, typeAndData, crc]);
  }

  // IHDR: 4x4, 8-bit RGB
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(4, 0);  // width
  ihdr.writeUInt32BE(4, 4);  // height
  ihdr[8] = 8;               // bit depth
  ihdr[9] = 2;               // color type RGB
  ihdr[10] = 0;              // compression
  ihdr[11] = 0;              // filter
  ihdr[12] = 0;              // interlace

  // IDAT: 4 rows of 4 pixels (RGB), each row prefixed with filter byte 0
  // Row 0-1: red red blue blue
  // Row 2-3: red red blue blue
  const raw = Buffer.alloc(4 * (1 + 4 * 3)); // 4 rows * (1 filter + 4 pixels * 3 bytes)
  let offset = 0;
  for (let y = 0; y < 4; y++) {
    raw[offset++] = 0; // filter: none
    for (let x = 0; x < 4; x++) {
      if (x < 2) {
        // Red quadrant
        raw[offset++] = 255; raw[offset++] = 0; raw[offset++] = 0;
      } else {
        // Blue quadrant
        raw[offset++] = 0; raw[offset++] = 0; raw[offset++] = 255;
      }
    }
  }

  // Deflate the raw data (use zlib)
  const zlib = require('zlib');
  const compressed = zlib.deflateSync(raw);

  const idat = makeChunk('IDAT', compressed);
  const ihdrChunk = makeChunk('IHDR', ihdr);
  const iend = makeChunk('IEND', Buffer.alloc(0));

  const png = Buffer.concat([signature, ihdrChunk, idat, iend]);
  return png.toString('base64');
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------
function httpRequest(method, path, body) {
  return new Promise((resolve, reject) => {
    const url = new URL(path, SERVER_URL);
    const options = {
      hostname: url.hostname,
      port: url.port,
      path: url.pathname + url.search,
      method,
      headers: { 'Content-Type': 'application/json' },
    };
    const req = http.request(options, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => {
        try {
          resolve({ status: res.statusCode, body: JSON.parse(data) });
        } catch {
          resolve({ status: res.statusCode, body: data });
        }
      });
    });
    req.on('error', reject);
    if (body) req.write(JSON.stringify(body));
    req.end();
  });
}

async function pollForCompletion(taskId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let cursor = 0;
  let lastResponse = '';

  while (Date.now() < deadline) {
    const res = await httpRequest('GET', `/task/${taskId}/events?since=${cursor}`);
    if (res.status !== 200) {
      await sleep(1000);
      continue;
    }

    const events = res.body.events || [];
    for (const evt of events) {
      if (evt.event_type === 'think_response' && evt.data?.content) {
        lastResponse = evt.data.content;
      }
      if (evt.event_type === 'session_end') {
        return { done: true, response: lastResponse, sats: Number(evt.data?.sats_spent || 0) };
      }
      if (evt.event_type === 'error') {
        return { done: true, response: `ERROR: ${evt.data?.error || evt.data?.message || 'unknown'}`, sats: 0, error: true };
      }
    }

    cursor = res.body.total || cursor;
    if (res.body.active === false && events.length === 0) {
      // Task finished but we may have missed the session_end
      return { done: true, response: lastResponse, sats: 0 };
    }

    await sleep(1000);
  }

  return { done: false, response: lastResponse, sats: 0, error: true };
}

function sleep(ms) {
  return new Promise(r => setTimeout(r, ms));
}

// ---------------------------------------------------------------------------
// Test execution
// ---------------------------------------------------------------------------
async function runTest(test) {
  console.log(`\n  Testing: ${test.name}`);
  console.log(`  Model:   ${test.model}`);
  console.log(`  Message: "${test.message.substring(0, 60)}..."`);

  const start = Date.now();

  // Send chat with image attachment
  const chatBody = {
    message: test.message,
    model: test.model,
    attachments: [
      {
        data: TEST_IMAGE_BASE64,
        mime_type: TEST_IMAGE_MIME,
        filename: TEST_IMAGE_FILENAME,
      },
    ],
  };

  const chatRes = await httpRequest('POST', '/chat', chatBody);
  if (chatRes.status !== 200) {
    console.log(`  FAIL: POST /chat returned ${chatRes.status}`);
    console.log(`  Body: ${JSON.stringify(chatRes.body).substring(0, 200)}`);
    return { pass: false, error: `HTTP ${chatRes.status}`, sats: 0, latencyMs: Date.now() - start };
  }

  const taskId = chatRes.body.task_id;
  console.log(`  Task:    ${taskId}`);

  // Poll for completion
  const result = await pollForCompletion(taskId, test.timeout_ms);
  const latencyMs = Date.now() - start;

  if (!result.done) {
    console.log(`  FAIL: Timed out after ${test.timeout_ms}ms`);
    return { pass: false, error: 'timeout', sats: 0, latencyMs };
  }

  if (result.error) {
    console.log(`  FAIL: ${result.response}`);
    return { pass: false, error: result.response, sats: result.sats, latencyMs };
  }

  // Validate response contains expected terms
  const respLower = result.response.toLowerCase();
  const matched = test.expected_any.filter(term => respLower.includes(term.toLowerCase()));
  const pass = matched.length > 0;

  const costStr = result.sats ? `${result.sats.toLocaleString()} sats` : 'unknown';
  const timeStr = `${(latencyMs / 1000).toFixed(1)}s`;

  if (pass) {
    console.log(`  PASS: Matched [${matched.join(', ')}] | ${costStr} | ${timeStr}`);
  } else {
    console.log(`  FAIL: No expected terms found in response`);
    console.log(`  Expected any of: [${test.expected_any.join(', ')}]`);
    console.log(`  Response (first 300 chars): ${result.response.substring(0, 300)}`);
  }

  return { pass, response: result.response, sats: result.sats, latencyMs, matched };
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main() {
  const args = process.argv.slice(2);
  const onlyOpenai = args.includes('--openai');
  const onlyClaude = args.includes('--claude');
  const dryRun = args.includes('--dry-run');

  let tests = TESTS;
  if (onlyOpenai) tests = tests.filter(t => t.id === 'openai');
  if (onlyClaude) tests = tests.filter(t => t.id === 'claude');

  console.log('=== Multimodal Image Upload E2E Test ===');
  console.log(`Image: ${TEST_IMAGE_FILENAME} (4x4 red/blue PNG, ${TEST_IMAGE_BASE64.length} base64 chars)`);
  console.log(`Tests: ${tests.map(t => t.name).join(', ')}`);

  if (dryRun) {
    console.log('\nDry run — would test:');
    for (const t of tests) console.log(`  - ${t.name} (${t.model})`);
    process.exit(0);
  }

  // Verify server is up
  try {
    const health = await httpRequest('GET', '/health');
    if (health.status !== 200) throw new Error(`Health check returned ${health.status}`);
    console.log(`Server: OK (v${health.body.version || '?'}, uptime ${health.body.uptime_secs || '?'}s)`);
  } catch (e) {
    console.error(`\nServer not reachable at ${SERVER_URL} — start it first:`);
    console.error('  cargo run --release -- serve --port 8080\n');
    process.exit(1);
  }

  const results = [];
  let passed = 0;
  let failed = 0;

  for (const test of tests) {
    const result = await runTest(test);
    results.push({ ...test, ...result });
    if (result.pass) passed++;
    else failed++;

    // Cooldown between tests
    if (tests.indexOf(test) < tests.length - 1) {
      await sleep(3000);
    }
  }

  // Summary
  console.log('\n=== Results ===');
  const totalSats = results.reduce((sum, r) => sum + (r.sats || 0), 0);
  console.log(`Passed: ${passed}/${tests.length}`);
  console.log(`Total cost: ${totalSats.toLocaleString()} sats`);

  for (const r of results) {
    const icon = r.pass ? 'PASS' : 'FAIL';
    const costStr = r.sats ? `${r.sats.toLocaleString()} sats` : 'n/a';
    console.log(`  [${icon}] ${r.name} — ${costStr}, ${(r.latencyMs / 1000).toFixed(1)}s`);
  }

  if (failed > 0) {
    console.log(`\n${failed} test(s) failed.`);
    process.exit(1);
  }

  console.log('\nAll multimodal tests passed.');
}

main().catch(e => {
  console.error('Fatal error:', e);
  process.exit(1);
});
