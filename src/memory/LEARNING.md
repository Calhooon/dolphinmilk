# Learning — memory

## Lessons Learned
- **[2026-02-24]** `MemoryCategory::Session` displays as `"session"` but its directory is `sessions/`. The `FromStr` impl accepts both, but if you're building file paths manually, use `store.category_dir()` — never construct paths from `Display` output.
- **[2026-02-24]** `encrypt_and_store` encrypts only `entry.content` bytes, not the full markdown-with-frontmatter. The `.enc` file is not a valid markdown file — metadata lives in the `UhrpIndex` sidecar, not in the encrypted blob.
- **[2026-02-24]** Legacy encryption (`0x42421033` magic) derives AES keys by signing a fixed message then SHA-256 hashing the signature. This round-trips through the wallet's `createSignature` endpoint. Don't replicate this pattern — wallet-native encrypt/decrypt is the correct path for new code.
- **[2026-02-24]** `MemorySync::load_index` silently returns `Ok(())` when `index.enc` doesn't exist, starting with an empty index. This is intentional — first boot has no index file. Don't treat a missing index as an error.

## Debugging Insights
- **[2026-02-24]** Tantivy `IndexWriter` allocates a 50MB heap arena per call to `writer()`. Each `add_entry`/`delete_entry` creates a fresh writer. For bulk operations, use `rebuild()` which shares one writer — otherwise you'll spike memory with many individual inserts.
- **[2026-02-24]** If frontmatter parsing fails on a valid-looking markdown file, check for UTF-8 BOM. `from_markdown` strips `\u{feff}` but only at the start — a BOM after any leading whitespace will cause a "missing YAML frontmatter delimiter" error.
- **[2026-02-24]** Search queries match against both `content` and `tags` fields. Tags are stored space-joined (e.g., `"bsv payment x402"`), so searching for a tag name will also match content text containing that word. This is by design but can produce unexpected results.
- **[2026-02-24]** When `decrypt_entry` fails, check `is_legacy_format()` first. If the blob starts with `0x42421033`, it needs the legacy key derivation path (wallet `createSignature` + SHA-256). Otherwise it's wallet-native ciphertext. Mixing up the two paths gives opaque decryption errors.

## Pattern Notes
- **[2026-02-24]** The store/index split is deliberate: `MemoryStore` handles files (source of truth), `MemoryIndex` handles tantivy (ephemeral, rebuilt from files). Always write to the store first, then update the index. Never rely on the index surviving a restart.
- **[2026-02-24]** `extract_first_sentence` has a 10-character minimum before recognizing sentence boundaries. Sentences like `"Done."` or `"OK."` won't be extracted as findings — they'll be returned as-is via `truncate_str`. This prevents noisy short fragments in session summaries.
- **[2026-02-24]** UHRP upload/list/find endpoints have `// TODO: Add X-Authrite-* headers` — they won't work against real NanoStore without Authrite session headers. Downloads work because CDN access is public (content is encrypted at rest). Don't attempt live UHRP uploads until Authrite is wired in.
- **[2026-02-24]** The `UhrpIndex` is itself encrypted and stored as `encrypted/index.enc`. It's a separate artifact from memory `.enc` files — `save_index`/`load_index` manage it independently. If you delete `.enc` files without updating the index, the index will reference ghosts.
