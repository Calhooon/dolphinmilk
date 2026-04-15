/**
 * Multi-Worm Scenario Validation
 *
 * Validates scenario files against the multi-worm schema without external
 * dependencies. Uses Node.js built-ins only.
 *
 * Can be used as a module or run standalone:
 *
 *   node -e "require('./lib/validate').validateScenariosFile('./scenarios.json')"
 *   node lib/validate.js [path/to/scenarios.json]
 *
 * Exports: { validateScenario, validateScenarios, validateScenariosFile }
 */

const fs = require('fs');
const path = require('path');

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const VALID_TIERS = ['canary', 'basic', 'security', 'economic', 'full'];
const VALID_ACTIONS = ['task', 'wait', 'assert', 'api'];
const VALID_EVENTS = ['task_complete', 'message_received', 'proof_created', 'budget_updated'];
const VALID_METHODS = ['GET', 'POST', 'PUT', 'DELETE'];
const VALID_ASSERTION_TYPES = [
  'response_contains',
  'response_not_contains',
  'response_matches',
  'status_code',
  'field_equals',
  'field_exists',
  'cost_within',
  'agents_different_keys',
  'session_no_error',
  'tool_succeeded',
  'tool_result_contains',
  'no_tool_errors',
];
const SNAKE_CASE_RE = /^[a-z][a-z0-9_]*$/;

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/**
 * Collect validation errors for a value. Returns an array of error strings.
 * An empty array means the value is valid.
 *
 * @param {*} value — the value to check
 * @param {string} path — dot-separated path for error messages (e.g. "scenarios[0].name")
 * @param {object} rules — validation rules
 * @param {string}  [rules.type]       — expected typeof (or "array")
 * @param {boolean} [rules.required]   — value must be present and non-undefined
 * @param {*[]}     [rules.enum]       — value must be one of these
 * @param {RegExp}  [rules.pattern]    — string must match
 * @param {number}  [rules.minItems]   — array min length
 * @param {number}  [rules.minimum]    — number minimum
 * @returns {string[]}
 */
function check(value, path, rules) {
  const errors = [];

  if (rules.required && (value === undefined || value === null)) {
    errors.push(`${path}: required field is missing`);
    return errors; // no point checking further
  }

  if (value === undefined || value === null) {
    return errors; // optional and absent — OK
  }

  if (rules.type) {
    const actualType = Array.isArray(value) ? 'array' : typeof value;
    if (actualType !== rules.type) {
      errors.push(`${path}: expected type '${rules.type}', got '${actualType}'`);
      return errors;
    }
  }

  if (rules.enum && !rules.enum.includes(value)) {
    errors.push(`${path}: '${value}' is not one of [${rules.enum.join(', ')}]`);
  }

  if (rules.pattern && typeof value === 'string' && !rules.pattern.test(value)) {
    errors.push(`${path}: '${value}' does not match pattern ${rules.pattern}`);
  }

  if (rules.minItems !== undefined && Array.isArray(value) && value.length < rules.minItems) {
    errors.push(`${path}: expected at least ${rules.minItems} items, got ${value.length}`);
  }

  if (rules.minimum !== undefined && typeof value === 'number' && value < rules.minimum) {
    errors.push(`${path}: ${value} is below minimum ${rules.minimum}`);
  }

  return errors;
}

// ---------------------------------------------------------------------------
// Assertion validation
// ---------------------------------------------------------------------------

/**
 * Validate an assertion object.
 *
 * @param {object} assertion — the assertion to validate
 * @param {string} prefix — path prefix for error messages
 * @returns {string[]} — list of errors
 */
function validateAssertion(assertion, prefix) {
  const errors = [];

  if (typeof assertion !== 'object' || assertion === null) {
    errors.push(`${prefix}: assertion must be an object`);
    return errors;
  }

  errors.push(...check(assertion.type, `${prefix}.type`, {
    required: true,
    type: 'string',
    enum: VALID_ASSERTION_TYPES,
  }));

  // Type-specific requirements
  if (assertion.type === 'response_contains' || assertion.type === 'response_not_contains') {
    errors.push(...check(assertion.value, `${prefix}.value`, { required: true, type: 'string' }));
  }
  if (assertion.type === 'response_matches') {
    errors.push(...check(assertion.value, `${prefix}.value`, { required: true, type: 'string' }));
  }
  if (assertion.type === 'status_code') {
    errors.push(...check(assertion.value, `${prefix}.value`, { required: true, type: 'number' }));
  }
  if (assertion.type === 'field_exists' || assertion.type === 'field_equals') {
    errors.push(...check(assertion.field, `${prefix}.field`, { required: true, type: 'string' }));
  }
  if (assertion.type === 'cost_within') {
    errors.push(...check(assertion.value, `${prefix}.value`, { required: true, type: 'number', minimum: 0 }));
  }
  if (assertion.type === 'session_no_error' || assertion.type === 'no_tool_errors') {
    errors.push(...check(assertion.step, `${prefix}.step`, { required: true, type: 'string' }));
  }
  if (assertion.type === 'tool_succeeded' || assertion.type === 'tool_result_contains') {
    errors.push(...check(assertion.step, `${prefix}.step`, { required: true, type: 'string' }));
    errors.push(...check(assertion.tool_name, `${prefix}.tool_name`, { required: true, type: 'string' }));
  }
  if (assertion.type === 'tool_result_contains') {
    errors.push(...check(assertion.value, `${prefix}.value`, { required: true, type: 'string' }));
  }

  return errors;
}

// ---------------------------------------------------------------------------
// Step validation
// ---------------------------------------------------------------------------

/**
 * Validate a single step within a scenario.
 *
 * @param {object} step — the step to validate
 * @param {string} prefix — path prefix for error messages
 * @param {string[]} agents — list of valid agent names for this scenario
 * @returns {string[]} — list of errors
 */
function validateStep(step, prefix, agents) {
  const errors = [];

  if (typeof step !== 'object' || step === null) {
    errors.push(`${prefix}: step must be an object`);
    return errors;
  }

  errors.push(...check(step.agent, `${prefix}.agent`, { required: true, type: 'string' }));
  errors.push(...check(step.action, `${prefix}.action`, {
    required: true,
    type: 'string',
    enum: VALID_ACTIONS,
  }));

  // Agent must be in the scenario's agents list
  if (step.agent && agents.length > 0 && !agents.includes(step.agent)) {
    errors.push(`${prefix}.agent: '${step.agent}' is not in scenario agents [${agents.join(', ')}]`);
  }

  // Action-specific required fields
  if (step.action === 'task') {
    errors.push(...check(step.message, `${prefix}.message`, { required: true, type: 'string' }));
  }

  if (step.action === 'wait') {
    errors.push(...check(step.event, `${prefix}.event`, {
      required: true,
      type: 'string',
      enum: VALID_EVENTS,
    }));
  }

  if (step.action === 'api') {
    errors.push(...check(step.path, `${prefix}.path`, { required: true, type: 'string' }));
    if (step.method !== undefined) {
      errors.push(...check(step.method, `${prefix}.method`, { type: 'string', enum: VALID_METHODS }));
    }
  }

  if (step.action === 'assert') {
    if (step.condition === undefined || step.condition === null) {
      errors.push(`${prefix}.condition: required for 'assert' action`);
    } else {
      errors.push(...validateAssertion(step.condition, `${prefix}.condition`));
    }
  }

  // Optional field types
  if (step.timeout_seconds !== undefined) {
    errors.push(...check(step.timeout_seconds, `${prefix}.timeout_seconds`, { type: 'number', minimum: 1 }));
  }
  if (step.max_cost_sats !== undefined) {
    errors.push(...check(step.max_cost_sats, `${prefix}.max_cost_sats`, { type: 'number', minimum: 0 }));
  }
  if (step.max_iterations !== undefined) {
    errors.push(...check(step.max_iterations, `${prefix}.max_iterations`, { type: 'number', minimum: 1 }));
  }
  if (step.store_as !== undefined) {
    errors.push(...check(step.store_as, `${prefix}.store_as`, { type: 'string', pattern: SNAKE_CASE_RE }));
  }

  return errors;
}

// ---------------------------------------------------------------------------
// Scenario validation
// ---------------------------------------------------------------------------

/**
 * Validate a single multi-worm test scenario.
 *
 * @param {object} scenario — the scenario object
 * @param {number} [index] — array index (for error messages)
 * @returns {{ valid: boolean, errors: string[] }}
 */
function validateScenario(scenario, index) {
  const prefix = index !== undefined ? `scenarios[${index}]` : 'scenario';
  const errors = [];

  if (typeof scenario !== 'object' || scenario === null) {
    return { valid: false, errors: [`${prefix}: must be an object`] };
  }

  // Required fields
  errors.push(...check(scenario.id, `${prefix}.id`, { required: true, type: 'number', minimum: 1 }));
  errors.push(...check(scenario.tier, `${prefix}.tier`, {
    required: true,
    type: 'string',
    enum: VALID_TIERS,
  }));
  errors.push(...check(scenario.name, `${prefix}.name`, {
    required: true,
    type: 'string',
    pattern: SNAKE_CASE_RE,
  }));
  errors.push(...check(scenario.description, `${prefix}.description`, { required: true, type: 'string' }));
  errors.push(...check(scenario.agents, `${prefix}.agents`, {
    required: true,
    type: 'array',
    minItems: 2,
  }));
  errors.push(...check(scenario.steps, `${prefix}.steps`, {
    required: true,
    type: 'array',
    minItems: 1,
  }));

  // Agent names must be snake_case
  const agents = Array.isArray(scenario.agents) ? scenario.agents : [];
  for (let i = 0; i < agents.length; i++) {
    errors.push(...check(agents[i], `${prefix}.agents[${i}]`, { type: 'string', pattern: SNAKE_CASE_RE }));
  }

  // Duplicate agent check
  const uniqueAgents = new Set(agents);
  if (uniqueAgents.size !== agents.length) {
    errors.push(`${prefix}.agents: contains duplicate agent names`);
  }

  // Steps
  const steps = Array.isArray(scenario.steps) ? scenario.steps : [];
  for (let i = 0; i < steps.length; i++) {
    errors.push(...validateStep(steps[i], `${prefix}.steps[${i}]`, agents));
  }

  // Duplicate store_as keys
  const storeKeys = steps.map(s => s.store_as).filter(Boolean);
  const uniqueStoreKeys = new Set(storeKeys);
  if (uniqueStoreKeys.size !== storeKeys.length) {
    errors.push(`${prefix}.steps: duplicate store_as keys found`);
  }

  // Post-scenario assertions
  if (scenario.assertions !== undefined) {
    errors.push(...check(scenario.assertions, `${prefix}.assertions`, { type: 'array' }));
    if (Array.isArray(scenario.assertions)) {
      for (let i = 0; i < scenario.assertions.length; i++) {
        errors.push(...validateAssertion(scenario.assertions[i], `${prefix}.assertions[${i}]`));
      }
    }
  }

  // Budget
  if (scenario.budget !== undefined) {
    if (typeof scenario.budget !== 'object' || scenario.budget === null) {
      errors.push(`${prefix}.budget: must be an object`);
    } else {
      if (scenario.budget.max_total_sats !== undefined) {
        errors.push(...check(scenario.budget.max_total_sats, `${prefix}.budget.max_total_sats`, {
          type: 'number',
          minimum: 0,
        }));
      }
      if (scenario.budget.max_per_agent_sats !== undefined) {
        errors.push(...check(scenario.budget.max_per_agent_sats, `${prefix}.budget.max_per_agent_sats`, {
          type: 'number',
          minimum: 0,
        }));
      }
    }
  }

  // Optional fields
  if (scenario.skip !== undefined) {
    errors.push(...check(scenario.skip, `${prefix}.skip`, { type: 'boolean' }));
  }
  if (scenario.timeout_seconds !== undefined) {
    errors.push(...check(scenario.timeout_seconds, `${prefix}.timeout_seconds`, { type: 'number', minimum: 1 }));
  }
  if (scenario.tags !== undefined) {
    errors.push(...check(scenario.tags, `${prefix}.tags`, { type: 'array' }));
  }
  if (scenario.note !== undefined) {
    errors.push(...check(scenario.note, `${prefix}.note`, { type: 'string' }));
  }

  return { valid: errors.length === 0, errors };
}

// ---------------------------------------------------------------------------
// File-level validation
// ---------------------------------------------------------------------------

/**
 * Validate an array of scenarios (the parsed contents of a scenarios file).
 *
 * @param {object} data — parsed JSON with { schema_version, scenarios, ... }
 * @returns {{ valid: boolean, errors: string[], scenarioCount: number }}
 */
function validateScenarios(data) {
  const errors = [];

  if (typeof data !== 'object' || data === null) {
    return { valid: false, errors: ['Root: must be an object'], scenarioCount: 0 };
  }

  // schema_version
  errors.push(...check(data.schema_version, 'schema_version', {
    required: true,
    type: 'string',
  }));
  if (data.schema_version !== '2.0') {
    errors.push(`schema_version: expected '2.0', got '${data.schema_version}'`);
  }

  // default_timeout_seconds
  if (data.default_timeout_seconds !== undefined) {
    errors.push(...check(data.default_timeout_seconds, 'default_timeout_seconds', {
      type: 'number',
      minimum: 1,
    }));
  }

  // scenarios array
  errors.push(...check(data.scenarios, 'scenarios', {
    required: true,
    type: 'array',
    minItems: 1,
  }));

  const scenarios = Array.isArray(data.scenarios) ? data.scenarios : [];

  // Validate each scenario
  for (let i = 0; i < scenarios.length; i++) {
    const result = validateScenario(scenarios[i], i);
    errors.push(...result.errors);
  }

  // Check for duplicate IDs
  const ids = scenarios.map(s => s.id).filter(id => id !== undefined);
  const uniqueIds = new Set(ids);
  if (uniqueIds.size !== ids.length) {
    const counts = {};
    for (const id of ids) {
      counts[id] = (counts[id] || 0) + 1;
    }
    const dupes = Object.entries(counts).filter(([, c]) => c > 1).map(([id]) => id);
    errors.push(`scenarios: duplicate IDs found: [${dupes.join(', ')}]`);
  }

  // Check for duplicate names
  const names = scenarios.map(s => s.name).filter(Boolean);
  const uniqueNames = new Set(names);
  if (uniqueNames.size !== names.length) {
    const counts = {};
    for (const name of names) {
      counts[name] = (counts[name] || 0) + 1;
    }
    const dupes = Object.entries(counts).filter(([, c]) => c > 1).map(([name]) => name);
    errors.push(`scenarios: duplicate names found: [${dupes.join(', ')}]`);
  }

  return {
    valid: errors.length === 0,
    errors,
    scenarioCount: scenarios.length,
  };
}

/**
 * Load and validate a scenarios JSON file.
 *
 * @param {string} filePath — path to the scenarios JSON file
 * @returns {{ valid: boolean, errors: string[], scenarioCount: number }}
 */
function validateScenariosFile(filePath) {
  const resolved = path.resolve(filePath);

  let raw;
  try {
    raw = fs.readFileSync(resolved, 'utf8');
  } catch (err) {
    return { valid: false, errors: [`Failed to read file: ${err.message}`], scenarioCount: 0 };
  }

  let data;
  try {
    data = JSON.parse(raw);
  } catch (err) {
    return { valid: false, errors: [`Invalid JSON: ${err.message}`], scenarioCount: 0 };
  }

  const result = validateScenarios(data);

  // Print summary
  if (result.valid) {
    console.log(`OK: ${result.scenarioCount} scenarios validated successfully.`);
  } else {
    console.error(`FAIL: ${result.errors.length} error(s) in ${result.scenarioCount} scenario(s):\n`);
    for (const err of result.errors) {
      console.error(`  - ${err}`);
    }
    process.exitCode = 1;
  }

  return result;
}

// ---------------------------------------------------------------------------
// Standalone CLI
// ---------------------------------------------------------------------------

if (require.main === module) {
  const filePath = process.argv[2] || path.join(__dirname, '..', 'scenarios.json');
  validateScenariosFile(filePath);
}

// ---------------------------------------------------------------------------
// Module exports
// ---------------------------------------------------------------------------

module.exports = { validateScenario, validateScenarios, validateScenariosFile };
