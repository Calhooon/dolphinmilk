#!/bin/bash
# E11 — comment expansion POC (zero LLM cost)
#
# Measures:
#   - Fresh reddit scrape: N posts at limit=100
#   - Per-post comment fetch: K hashable records per post
#   - Total unique hashes per scrape
#   - Wall clock for the full batch at serial pace
#   - On-chain proof creation rate (via worker wallet)
#
# Run with: bash e11_comment_expand.sh
#
# This script is NOT called from the LLM flow — it's a local lab harness
# to prove the content-multiplier gate BEFORE paying for an LLM cycle.

set -u

WALLET_URL="${WALLET_URL:-http://localhost:3324}"
SUBREDDIT="${SUBREDDIT:-technology}"
LIMIT="${LIMIT:-50}"                 # posts to pull
COMMENT_LIMIT="${COMMENT_LIMIT:-100}" # comments per post
TMPDIR="${TMPDIR:-/tmp}/e11-$(date +%s)"
USER_AGENT="Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0"

mkdir -p "$TMPDIR"
echo "[e11] workdir: $TMPDIR"
echo "[e11] wallet:  $WALLET_URL"
echo "[e11] source:  r/${SUBREDDIT} limit=${LIMIT} (+${COMMENT_LIMIT} comments/post)"

# ------------------------------------------------------------------
# Phase 1 — scrape posts
# ------------------------------------------------------------------
T0=$(python3 -c 'import time; print(time.time())')
echo "[e11] phase 1: scraping posts..."
curl -sS -A "$USER_AGENT" -L \
  "https://www.reddit.com/r/${SUBREDDIT}/hot.json?limit=${LIMIT}" \
  -o "$TMPDIR/posts.json" \
  -w "  posts HTTP:%{http_code} time:%{time_total}s size:%{size_download}B\n"

POST_COUNT=$(jq '.data.children | length' "$TMPDIR/posts.json")
echo "[e11] scraped $POST_COUNT posts"

# Write post hashes (jq -c of each child, matching proof_verify.js canon)
jq -c '.data.children[]' "$TMPDIR/posts.json" > "$TMPDIR/records-posts.jsonl"
wc -l < "$TMPDIR/records-posts.jsonl" | awk '{print "[e11] post records: " $1}'

# ------------------------------------------------------------------
# Phase 2 — fetch comments per post (serial for now)
# ------------------------------------------------------------------
echo "[e11] phase 2: fetching comments..."
T1=$(python3 -c 'import time; print(time.time())')
: > "$TMPDIR/records-comments.jsonl"

POST_IDS=$(jq -r '.data.children[].data.permalink' "$TMPDIR/posts.json")
FETCH_COUNT=0
while IFS= read -r permalink; do
  [ -z "$permalink" ] && continue
  FETCH_COUNT=$((FETCH_COUNT + 1))
  curl -sS -A "$USER_AGENT" -L \
    "https://www.reddit.com${permalink}.json?limit=${COMMENT_LIMIT}" \
    -o "$TMPDIR/p${FETCH_COUNT}.json" 2>/dev/null
  # recursive flatten of ALL body-bearing comments (top-level + replies)
  jq -c '.[1] | [.. | objects | select(.body != null and .id != null) | {id,body,author,score,parent_id,created_utc}] | .[]' \
    "$TMPDIR/p${FETCH_COUNT}.json" 2>/dev/null >> "$TMPDIR/records-comments.jsonl" || true
  # light rate limit courtesy
  sleep 0.15
done <<< "$POST_IDS"

T2=$(python3 -c 'import time; print(time.time())')
COMMENT_RECORDS=$(wc -l < "$TMPDIR/records-comments.jsonl")
FETCH_TIME=$(python3 -c "print(f'{$T2 - $T1:.2f}')")
echo "[e11] comment records: $COMMENT_RECORDS (fetch took ${FETCH_TIME}s)"

# ------------------------------------------------------------------
# Phase 3 — combine + dedupe by SHA-256
# ------------------------------------------------------------------
cat "$TMPDIR/records-posts.jsonl" "$TMPDIR/records-comments.jsonl" > "$TMPDIR/records-all.jsonl"
TOTAL=$(wc -l < "$TMPDIR/records-all.jsonl")
echo "[e11] total records: $TOTAL"

# Compute hashes (one per line → shasum)
while IFS= read -r rec; do
  printf '%s' "$rec" | shasum -a 256 | cut -d' ' -f1
done < "$TMPDIR/records-all.jsonl" > "$TMPDIR/hashes.txt"

UNIQUE=$(sort -u "$TMPDIR/hashes.txt" | wc -l)
echo "[e11] unique hashes: $UNIQUE (of $TOTAL)"

# ------------------------------------------------------------------
# Phase 4 — on-chain proof rate sample (no full batch — first 20 only)
# ------------------------------------------------------------------
echo "[e11] phase 4: sampling on-chain proof rate (first 20 unique hashes)..."
SAMPLE=20
T3=$(python3 -c 'import time; print(time.time())')
CREATED=0
head -n "$SAMPLE" "$TMPDIR/hashes.txt" | while IFS= read -r HASH; do
  [ -z "$HASH" ] && continue
  LOCKING="006a20${HASH}"
  RESULT=$(curl -sS -X POST "${WALLET_URL}/createAction" \
    -H "Origin: ${WALLET_URL}" \
    -H 'Content-Type: application/json' \
    -d "{\"description\":\"e11 probe\",\"outputs\":[{\"lockingScript\":\"${LOCKING}\",\"satoshis\":0,\"outputDescription\":\"e11 probe record\"}]}" 2>/dev/null)
  TXID=$(printf '%s' "$RESULT" | jq -r '.txid // empty' 2>/dev/null)
  if [ -n "$TXID" ] && [ "$TXID" != "null" ]; then
    CREATED=$((CREATED + 1))
    echo "  tx: $TXID"
  else
    echo "  FAIL: $RESULT" >&2
  fi
done
T4=$(python3 -c 'import time; print(time.time())')
PROOF_TIME=$(python3 -c "print(f'{$T4 - $T3:.2f}')")
echo "[e11] sample proofs: wall=${PROOF_TIME}s for ${SAMPLE} serial createAction"

# ------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------
SCRAPE_TIME=$(python3 -c "print(f'{$T2 - $T0:.2f}')")
PROJECTED_BATCH_500=$(python3 -c "print(f'{(500 / $SAMPLE) * $PROOF_TIME:.1f}')")
PROJECTED_BATCH_ALL=$(python3 -c "print(f'{($UNIQUE / $SAMPLE) * $PROOF_TIME:.1f}')")

cat <<SUMMARY

================================================================
E11 SUMMARY
================================================================
subreddit:              r/${SUBREDDIT}
posts scraped:          $POST_COUNT
comment records:        $COMMENT_RECORDS
total records:          $TOTAL
unique hashes:          $UNIQUE
content multiplier:     $(python3 -c "print(f'{$UNIQUE / $POST_COUNT:.1f}')")×  (baseline = $POST_COUNT)
scrape+fetch wall time: ${SCRAPE_TIME}s
sample proof rate:      ${SAMPLE} txs in ${PROOF_TIME}s
  → projected batch=500:  ${PROJECTED_BATCH_500}s wall clock
  → projected batch=${UNIQUE}:  ${PROJECTED_BATCH_ALL}s wall clock
================================================================

files in $TMPDIR:
  posts.json            raw post listing
  records-posts.jsonl   post records (one per line)
  records-comments.jsonl comment records
  hashes.txt            all hashes
  p{N}.json             per-post comment trees

NEXT:
  - If unique > 500 and projected batch fits in 600s bash timeout → gate 1 PASS
  - If not → need parallel bash (xargs -P) or smaller batch
SUMMARY
