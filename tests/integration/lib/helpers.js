/**
 * CLI argument parsing and formatting utilities.
 */

function getArg(args, flag) {
  const idx = args.indexOf(flag);
  return idx >= 0 && idx + 1 < args.length ? args[idx + 1] : null;
}

function formatCost(sats) {
  return sats > 0 ? `${sats.toLocaleString()} sats` : 'free';
}

function formatDuration(ms) {
  return `${(ms / 1000).toFixed(1)}s`;
}

module.exports = { getArg, formatCost, formatDuration };
