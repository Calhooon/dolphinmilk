/**
 * Wallet authentication via BRC-31 AuthFetch.
 *
 * Reads wallet URL from the server's /health endpoint (sourced from dolphin-milk.toml),
 * connects to the wallet via HTTPWalletJSON substrate, and uses AuthFetch to sign
 * every request with x-bsv-auth-* headers.
 */

import { WalletClient, AuthFetch, HTTPWalletJSON } from '@bsv/sdk';

export interface AuthState {
  authenticated: boolean;
  identityKey: string | null;
  error: string | null;
  /** Authenticated fetch function — AuthFetch.fetch, signs every request. */
  fetchFn: typeof fetch;
}

/**
 * Authenticate via the wallet configured in dolphin-milk.toml.
 * Returns auth state with fetchFn that signs every request with BRC-31.
 */
export async function authenticate(baseUrl: string = ''): Promise<AuthState> {
  try {
    // Read wallet URL from the server (sourced from dolphin-milk.toml).
    // /health is the one unauthenticated endpoint.
    let walletUrl = 'http://localhost:3322';
    try {
      const healthRes = await fetch(`${baseUrl}/health`);
      if (healthRes.ok) {
        const health = await healthRes.json();
        if (health.wallet_url) walletUrl = health.wallet_url;
      }
    } catch { /* server unreachable, use default */ }

    // Connect to the wallet directly via HTTP substrate
    const substrate = new HTTPWalletJSON('dolphin-milk', walletUrl);
    const wallet = new WalletClient(substrate);

    // Get identity key from the wallet
    const { publicKey: identityKey } = await wallet.getPublicKey({
      identityKey: true,
    });

    // Create AuthFetch — handles handshake + signing automatically
    const authFetch = new AuthFetch(wallet);
    const rawFetch = authFetch.fetch.bind(authFetch);

    // Wrap to resolve relative URLs — AuthFetch needs absolute URLs for
    // BRC-31 signature computation.
    const fetchFn = ((input: RequestInfo | URL, init?: RequestInit) => {
      if (typeof input === 'string' && input.startsWith('/')) {
        input = `${window.location.origin}${input}`;
      }
      return (rawFetch as any)(input, init);
    }) as typeof fetch;

    return {
      authenticated: true,
      identityKey,
      error: null,
      fetchFn,
    };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return {
      authenticated: false,
      identityKey: null,
      error: `Connect your wallet (bsv-wallet-cli). ${msg}`,
      fetchFn: fetch.bind(window),
    };
  }
}
