# Learning — x402

## Lessons Learned
- **[2026-02-24]** The protocol ID `[2, "3241645161d8"]` is not arbitrary — it must match the TS SDK's `AuthFetch.createPaymentContext()` exactly. If you change it, the server derives a different key and payment verification silently fails.
- **[2026-02-24]** `paid_request()` is low-level and does NOT compose BRC-31 Authrite headers. The common pattern (auth + payment) lives in `think.rs`, not here. Calling `paid_request()` directly skips authentication entirely.
- **[2026-02-24]** The wallet's `createAction` can return either raw tx bytes (`01000000...`) or AtomicBEEF. The 4-byte prefix check at `payment.rs:214` auto-detects and wraps raw txs. Removing this check breaks payments against wallets that return raw format.
- **[2026-02-24]** Refund JSON uses camelCase (`derivationPrefix`, `senderIdentityKey`) matching the BRC-29 wire format. The refund parser checks both `excessRefund` and `refund` keys because different servers use different field names for the same concept.

## Debugging Insights
- **[2026-02-24]** If payments fail silently, check the `x-bsv-auth-identity-key` header from the 402 response. When missing, `server_identity_key` defaults to `""` (`payment.rs:269`), and BRC-42 key derivation produces a wrong key — the server rejects payment with another 402.
- **[2026-02-24]** Three consecutive 402s after payment usually means the server identity key is wrong or the protocol ID doesn't match. It is NOT a funding issue — the wallet created valid transactions, the server just can't unlock them.
- **[2026-02-24]** Header-transport vs body-transport: if payment JSON > 6KB and no original body exists, payment moves to the request body with `x-bsv-payment: body` signal. If you're debugging a "payment not found" error, check whether the server expects header or body transport.
- **[2026-02-24]** `paid_request()` returns the response without consuming the body, so callers must handle refund parsing themselves. Forgetting this means leaked refunds (sats the server returned that never get internalized).

## Pattern Notes
- **[2026-02-24]** The module is ~200 lines replacing 2,600 lines of Python because all crypto is delegated to the wallet HTTP API. The pattern: derive key → build script → `createAction` → encode → attach header. Never touch private keys in this module.
- **[2026-02-24]** P2PKH validation (`build_p2pkh_script`) requires exactly 66 hex chars with `02`/`03` prefix. This catches bad keys early before they hit the wallet. The hash160 (RIPEMD160(SHA256)) is done locally since it's just a public key hash, not a signing operation.
- **[2026-02-24]** The `varint()` and `raw_tx_to_beef()`/`raw_tx_to_atomic_beef()` functions implement BRC-62 serialization by hand rather than pulling in a dependency. The BEEF header is `0x0100BEEF`, AtomicBEEF is `0x01010101` followed by reversed txid + BEEF payload.
- **[2026-02-24]** MessageBox is the one exception to header-transport — it uses body-transport as a protocol requirement, not because of size. That logic lives in `src/messagebox/`, not here. Don't try to "fix" x402 to handle MessageBox's case.
