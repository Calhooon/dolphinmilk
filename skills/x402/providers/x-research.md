# X-Research — X/Twitter Search

Synchronous service. Response is immediate after payment.

## Working Example

```
# Search tweets
x402_call({
  "service": "x-research/search",
  "parameters": {
    "query": "BSV micropayments",
    "sort": "likes",
    "pages": 1,
    "limit": 20,
    "format": "json"
  }
})
# Response: { "query": "...", "sort": "...", "tweet_count": 20, "tweets": [...] }

# Get a user profile
x402_call({
  "service": "x-research/profile",
  "parameters": { "username": "satikichain" }
})

# Get a tweet thread
x402_call({
  "service": "x-research/thread",
  "parameters": { "url": "https://x.com/user/status/123456" }
})

# Get trending topics
x402_call({
  "service": "x-research/trending",
  "parameters": {}
})

# Get a single tweet
x402_call({
  "service": "x-research/tweet",
  "parameters": { "id": "123456789" }
})
```

## Endpoints & Costs

| Endpoint | Cost | Description |
|----------|------|-------------|
| `/search` | ~$0.0625/page (~6K sats) | Full-archive search back to 2006 |
| `/profile` | ~$0.0125 (~1.2K sats) | User profile info |
| `/thread` | ~$0.0625 (~6K sats) | Full conversation thread |
| `/trending` | ~$0.00625 (~600 sats) | Current trending topics |
| `/tweet` | ~$0.00625 (~600 sats) | Single tweet by ID |

## Search Parameters

- `query`: X API search syntax (max 1024 chars). Supports operators like `from:user`, `since:2024-01-01`, `#hashtag`, `"exact phrase"`.
- `sort`: `"likes"` (default), `"retweets"`, `"recency"`, `"views"`, `"relevance"`
- `pages`: 1-5 (default 1). Each page is a separate charge.
- `limit`: tweets per page, 1-100 (default 20)
- `since`/`until`: time range. Accepts relative (`"30m"`, `"1h"`, `"7d"`, `"1y"`) or ISO 8601.
- `min_likes`/`min_views`: filter by engagement (optional)
- `exclude_replies`: boolean (default true)
- `format`: `"json"` or `"markdown"` (default json)

## Tweet Fields

Each tweet contains: `id`, `text`, `url`, `author_username`, `author_name`, `likes`, `retweets`, `replies`, `views`, `quotes`, `bookmarks`, `created_at`, `conversation_id`, `urls`, `mentions`, `hashtags`.

## Key Constraints
- Each page is a separate payment (~6K sats). Start with pages: 1.
- query is required for /search. Max 1024 characters.
- username is required for /profile. url is required for /thread. id is required for /tweet.
- sort defaults to "likes". Valid: "likes", "retweets", "recency", "views", "relevance".

## Validation Rules
- pages >= 1 | pages must be at least 1
- pages <= 5 | pages must be at most 5
- limit >= 1 | limit must be at least 1
- limit <= 100 | limit must be at most 100

## Common Errors

- **Empty results**: Refine your query. Use specific keywords, hashtags, or `from:` operators.
- **Pagination costs**: Each `page` is a separate payment. Start with `pages: 1` and only request more if needed.
