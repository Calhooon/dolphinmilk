/**
 * Proof hash computation using Web Crypto API.
 * Matches Rust's compute_proof_hash() in proofs.rs.
 */

/**
 * Compute SHA-256 of a plain string. Returns hex-encoded hash.
 */
export async function sha256(input: string): Promise<string> {
  const data = new TextEncoder().encode(input);
  const hashBuffer = await crypto.subtle.digest('SHA-256', data);
  return Array.from(new Uint8Array(hashBuffer)).map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Compute SHA-256(prev_hash_bytes || data || timestamp).
 * prevHash is optional hex-encoded bytes; when present, decoded to raw bytes first.
 * Returns the hex-encoded hash string.
 */
export async function computeProofHash(data: string, timestamp: string, prevHash?: string): Promise<string> {
  const encoder = new TextEncoder();
  const parts: Uint8Array[] = [];
  if (prevHash) {
    const bytes = new Uint8Array(prevHash.length / 2);
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = parseInt(prevHash.substr(i * 2, 2), 16);
    }
    parts.push(bytes);
  }
  parts.push(encoder.encode(data));
  parts.push(encoder.encode(timestamp));
  const totalLen = parts.reduce((sum, p) => sum + p.length, 0);
  const combined = new Uint8Array(totalLen);
  let offset = 0;
  for (const p of parts) {
    combined.set(p, offset);
    offset += p.length;
  }
  const hashBuffer = await crypto.subtle.digest('SHA-256', combined);
  return Array.from(new Uint8Array(hashBuffer)).map(b => b.toString(16).padStart(2, '0')).join('');
}
