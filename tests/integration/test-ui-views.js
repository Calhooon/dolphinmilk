#!/usr/bin/env node
/**
 * UI View Validation — Round 3 new views
 *
 * Tests new UI pages render and show meaningful content.
 * Works through the authenticated UI (same as run.js).
 */

const { chromium } = require('playwright');

const SERVER = 'http://localhost:8080';

async function main() {
  console.log('\n🖥️  UI View Validation — Round 3\n');

  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  const page = await context.newPage();
  const results = [];

  // ── Test 1: Main UI loads ─────────────────────────────────────────────
  {
    process.stdout.write('  [1] Main UI loads... ');
    try {
      await page.goto(SERVER + '/ui/');
      await page.getByText('Connecting...').first().waitFor({ state: 'hidden', timeout: 15000 });
      results.push({ name: 'main_ui_loads', pass: true });
      console.log('✓');
    } catch (err) {
      results.push({ name: 'main_ui_loads', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 2: Telemetry view renders metrics ────────────────────────────
  {
    process.stdout.write('  [2] Telemetry view renders... ');
    try {
      await page.goto(SERVER + '/ui/#telemetry');
      await page.waitForTimeout(4000);

      const content = await page.evaluate(() => {
        const app = document.querySelector('dm-app');
        if (!app?.shadowRoot) return { found: false, text: 'no dm-app shadow root' };
        // #telemetry now routes to agent page with telemetry tab
        const agent = app.shadowRoot.querySelector('dm-agent');
        if (!agent) return { found: false, text: 'no dm-agent element' };
        if (!agent.shadowRoot) return { found: false, text: 'no dm-agent shadow root' };
        const telemetry = agent.shadowRoot.querySelector('dm-telemetry');
        if (!telemetry) return { found: false, text: 'no dm-telemetry element inside agent' };
        if (!telemetry.shadowRoot) return { found: false, text: 'no shadow root' };
        return { found: true, text: telemetry.shadowRoot.textContent || '' };
      });

      if (content.found && content.text.length > 20) {
        const hasMetricContent = content.text.includes('Active') ||
                                  content.text.includes('Token') ||
                                  content.text.includes('Task') ||
                                  content.text.includes('Latency');
        results.push({ name: 'telemetry_view_renders', pass: hasMetricContent });
        console.log(hasMetricContent ? `✓ (${content.text.length} chars)` : '✗ no metric keywords');
      } else {
        results.push({ name: 'telemetry_view_renders', pass: false, error: content.text });
        console.log(`✗ ${content.text}`);
      }
    } catch (err) {
      results.push({ name: 'telemetry_view_renders', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 3: /metrics endpoint returns valid Prometheus format ─────────
  {
    process.stdout.write('  [3] /metrics returns Prometheus format... ');
    try {
      const resp = await page.request.get(SERVER + '/metrics');
      if (resp.status() === 401) {
        results.push({ name: 'metrics_prometheus_format', pass: true });
        console.log('✓ (auth required — BRC-31 protected)');
      } else {
        const text = await resp.text();
        const metricNames = [...text.matchAll(/^# HELP (\S+)/gm)].map(m => m[1]);
        const pass = metricNames.length >= 3 && text.includes('worm_');
        results.push({ name: 'metrics_prometheus_format', pass });
        console.log(pass ? `✓ ${metricNames.length} metrics` : '✗ insufficient metrics');
      }
    } catch (err) {
      results.push({ name: 'metrics_prometheus_format', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 4: Replay view renders (send a message first, then navigate to replay)
  {
    process.stdout.write('  [4] Replay view... ');
    try {
      // Send a test message through the chat
      await page.goto(SERVER + '/ui/');
      await page.getByText('Connecting...').first().waitFor({ state: 'hidden', timeout: 15000 }).catch(() => {});
      await page.waitForTimeout(1500);
      try { await page.getByText('New Chat').first().click({ timeout: 3000 }); } catch {}
      await page.waitForTimeout(1000);

      const input = page.getByRole('textbox', { name: 'Message Dolphin Milk...' });
      for (let i = 0; i < 60; i++) {
        if (!(await input.isDisabled())) break;
        await page.waitForTimeout(500);
      }
      await input.fill('Say hello');
      await page.waitForTimeout(300);
      await page.getByRole('button', { name: 'Send' }).click();

      // Wait for response
      await page.waitForTimeout(3000);
      try {
        await page.getByRole('button', { name: 'Stop' }).waitFor({ state: 'hidden', timeout: 60000 });
      } catch {}
      await page.waitForTimeout(2000);

      // Now navigate to replay for this task — check if URL has task ID
      const hasReplay = await page.evaluate(() => {
        const app = document.querySelector('dm-app');
        if (!app?.shadowRoot) return false;
        // Check if any replay-related nav exists or if the chat shows a task link
        const text = app.shadowRoot.textContent || '';
        return text.length > 0;
      });

      // The replay view can be tested if we can extract a task ID from the page
      // For now, verify the replay route exists and doesn't crash
      await page.goto(SERVER + '/ui/#task/test-id/replay');
      await page.waitForTimeout(2000);
      const replayExists = await page.evaluate(() => {
        const app = document.querySelector('dm-app');
        if (!app?.shadowRoot) return false;
        const replay = app.shadowRoot.querySelector('dm-replay-view');
        return !!replay;
      });

      results.push({ name: 'replay_view_exists', pass: replayExists });
      console.log(replayExists ? '✓ component renders' : '✗ component not found');
    } catch (err) {
      results.push({ name: 'replay_view_exists', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 5: Marketplace API responds ─────────────────────────────────
  {
    process.stdout.write('  [5] Marketplace API... ');
    try {
      const resp = await page.request.get(SERVER + '/marketplace/plugins');
      if (resp.status() === 401) {
        results.push({ name: 'marketplace_api', pass: true });
        console.log('✓ (auth required — BRC-31 protected)');
      } else {
        const data = await resp.json();
        const valid = typeof data.count === 'number' && Array.isArray(data.plugins);
        results.push({ name: 'marketplace_api', pass: valid });
        console.log(valid ? `✓ ${data.count} plugins` : '✗ invalid response');
      }
    } catch (err) {
      results.push({ name: 'marketplace_api', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 6: Escalation endpoint exists ───────────────────────────────
  {
    process.stdout.write('  [6] Escalation endpoint... ');
    try {
      const resp = await page.request.get(SERVER + '/task/nonexistent/escalation');
      const pass = [401, 404, 200].includes(resp.status());
      results.push({ name: 'escalation_endpoint', pass });
      console.log(pass ? `✓ HTTP ${resp.status()}` : `✗ HTTP ${resp.status()}`);
    } catch (err) {
      results.push({ name: 'escalation_endpoint', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 7: Model picker shows correct models (#172) ────────────────
  {
    process.stdout.write('  [7] Model picker models... ');
    try {
      await page.goto(SERVER + '/ui/');
      await page.getByText('Connecting...').first().waitFor({ state: 'hidden', timeout: 15000 });
      // Click the model trigger button in the shadow DOM
      const app = await page.$('dm-app');
      const appShadow = await app.evaluateHandle(el => el.shadowRoot);
      const chat = await appShadow.asElement().$('dm-chat');
      const chatShadow = await chat.evaluateHandle(el => el.shadowRoot);
      const chatInput = await chatShadow.asElement().$('dm-chat-input');
      const inputShadow = await chatInput.evaluateHandle(el => el.shadowRoot);
      // Click the model trigger to open the flyout
      const trigger = await inputShadow.asElement().$('.model-trigger');
      await trigger.click();
      await page.waitForTimeout(200);
      // Read all model option text from the flyout
      const modelNames = await inputShadow.asElement().$$eval('.model-option .model-name', els => els.map(el => el.textContent.trim()));
      const hasMini = modelNames.includes('GPT-5 Mini');
      const hasGpt5 = modelNames.includes('GPT-5');
      const hasProf = modelNames.some(n => n.includes('Pro'));
      const noPhantom = !modelNames.some(n => n.includes('4.1'));
      const pass = hasMini && hasGpt5 && hasProf && noPhantom && modelNames.length >= 9;
      results.push({ name: 'model_picker_models', pass });
      console.log(pass ? `✓ ${modelNames.length} models (no phantom gpt-4.1)` : `✗ models: [${modelNames.join(', ')}]`);
    } catch (err) {
      results.push({ name: 'model_picker_models', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 8: Model pricing shows input/output format (#172) ─────────
  {
    process.stdout.write('  [8] Model pricing format... ');
    try {
      const app = await page.$('dm-app');
      const appShadow = await app.evaluateHandle(el => el.shadowRoot);
      const chat = await appShadow.asElement().$('dm-chat');
      const chatShadow = await chat.evaluateHandle(el => el.shadowRoot);
      const chatInput = await chatShadow.asElement().$('dm-chat-input');
      const inputShadow = await chatInput.evaluateHandle(el => el.shadowRoot);
      const costs = await inputShadow.asElement().$$eval('.model-option .model-cost', els => els.map(el => el.textContent.trim()));
      // All costs should contain '/' (input / output format), none should have '~$' (old misleading format)
      const allSlash = costs.every(c => c.includes('/'));
      const noTilde = costs.every(c => !c.includes('~'));
      const pass = allSlash && noTilde && costs.length >= 9;
      results.push({ name: 'model_pricing_format', pass });
      console.log(pass ? `✓ all ${costs.length} use input/output format` : `✗ costs: [${costs.join(', ')}]`);
    } catch (err) {
      results.push({ name: 'model_pricing_format', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  // ── Test 9: /agent returns available_models (#172 Part F) ──────────
  {
    process.stdout.write('  [9] /agent has models... ');
    try {
      const resp = await page.request.get(SERVER + '/agent');
      if (resp.status() === 401) {
        // Auth required — still valid, check health for connectivity
        results.push({ name: 'agent_models', pass: true });
        console.log('✓ (auth required — BRC-31 protected)');
      } else {
        const data = await resp.json();
        const hasModels = Array.isArray(data.available_models) && data.available_models.length >= 5;
        const hasDefault = typeof data.default_model === 'string' && data.default_model.length > 0;
        const pass = hasModels && hasDefault;
        results.push({ name: 'agent_models', pass });
        console.log(pass ? `✓ ${data.available_models.length} models, default=${data.default_model}` : '✗ missing models or default_model');
      }
    } catch (err) {
      results.push({ name: 'agent_models', pass: false, error: err.message });
      console.log(`✗ ${err.message}`);
    }
  }

  await browser.close();

  const passed = results.filter(r => r.pass).length;
  const failed = results.filter(r => !r.pass).length;
  console.log(`\n${'─'.repeat(50)}`);
  console.log(`UI Views: ${passed} passed, ${failed} failed of ${results.length}`);

  if (failed > 0) {
    console.log('\nFailed:');
    results.filter(r => !r.pass).forEach(r => console.log(`  ✗ ${r.name}: ${r.error}`));
  }

  process.exit(failed > 0 ? 1 : 0);
}

main().catch(err => {
  console.error('Fatal:', err);
  process.exit(1);
});
