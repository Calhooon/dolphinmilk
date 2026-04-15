/**
 * Hash-based route parser for dm-app.
 * Extracts the route name and parameters from window.location.hash.
 */

export type Route =
  | 'chat'
  | 'tasks'
  | 'audit'
  | 'audit-global'
  | 'proofs'
  | 'replay'
  | 'agent'
  | 'conversations'
  | 'conversation-detail'
  | 'dashboard'
  | 'certificates'
  | 'automations'
  | 'budget'
  | 'reports'
  | 'compliance'
  | 'services'
  | 'settings'
  | 'wallet'
  | 'artifacts'
  | 'demo';

export interface RouteInfo {
  route: Route;
  params: Record<string, string>;
}

/**
 * Parse a hash string (without the leading '#') into a RouteInfo.
 * Handles all known routes and falls back to 'chat'.
 */
export function parseRoute(hash: string): RouteInfo {
  if (hash === 'tasks' || hash === 'activity') {
    return { route: 'tasks', params: {} };
  } else if (hash === 'automations') {
    return { route: 'automations', params: {} };
  } else if (hash === 'agent/tools') {
    return { route: 'agent', params: { tab: 'tools' } };
  } else if (hash === 'agent/memory') {
    return { route: 'agent', params: { tab: 'memory' } };
  } else if (hash === 'agent/telemetry') {
    return { route: 'agent', params: { tab: 'telemetry' } };
  } else if (hash === 'agent') {
    return { route: 'agent', params: { tab: 'identity' } };
  } else if (hash === 'conversations') {
    return { route: 'conversations', params: {} };
  } else if (hash === 'dashboard') {
    return { route: 'conversations', params: {} };
  } else if (hash === 'knowledge/schedules' || hash === 'schedules') {
    return { route: 'automations', params: {} };
  } else if (hash === 'knowledge' || hash === 'memory') {
    return { route: 'agent', params: { tab: 'memory' } };
  } else if (hash === 'budget') {
    return { route: 'budget', params: {} };
  } else if (hash === 'reports') {
    return { route: 'reports', params: {} };
  } else if (hash === 'audit-trail') {
    return { route: 'audit-global', params: {} };
  } else if (hash === 'compliance') {
    return { route: 'compliance', params: {} };
  } else if (hash === 'telemetry') {
    return { route: 'agent', params: { tab: 'telemetry' } };
  } else if (hash === 'certificates') {
    return { route: 'certificates', params: {} };
  } else if (hash === 'services') {
    return { route: 'services', params: {} };
  } else if (hash === 'settings') {
    return { route: 'settings', params: {} };
  } else if (hash === 'wallet') {
    return { route: 'wallet', params: {} };
  } else if (hash === 'artifacts') {
    return { route: 'artifacts', params: {} };
  } else if (hash.startsWith('demo')) {
    const parts = hash.split('/');
    return { route: 'demo', params: { pillar: parts[1] || 'stats' } };
  } else if (hash.startsWith('chat/')) {
    const parts = hash.split('/');
    return { route: 'chat', params: { sessionId: parts[1] } };
  } else if (hash.startsWith('conversation/')) {
    const parts = hash.split('/');
    const id = parts[1];
    // Sub-routes for conversation detail tabs
    if (parts.length > 2) {
      const tab = parts[2]; // artifacts, audit, chain
      return { route: 'conversation-detail', params: { id, tab } };
    }
    return { route: 'conversation-detail', params: { id } };
  } else if (hash.startsWith('task/')) {
    const parts = hash.split('/');
    const route: Route = hash.endsWith('/proofs') ? 'proofs' : hash.endsWith('/replay') ? 'replay' : 'audit';
    return { route, params: { id: parts[1] } };
  } else {
    return { route: 'chat', params: {} };
  }
}
