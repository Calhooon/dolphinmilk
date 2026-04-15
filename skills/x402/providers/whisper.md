# Whisper — Speech-to-Text Transcription

Synchronous service. Send audio, get text back immediately.

## Working Example

```
# Transcribe audio (base64-encoded)
x402_call({
  "service": "whisper/transcribe",
  "parameters": {
    "audio": "<base64-encoded audio bytes>",
    "language": "en"
  }
})
# Response: { "text": "transcribed text here...", "segments": [...] }
```

## Critical Details

- `audio` must be base64-encoded audio bytes. Supported formats: mp3, wav, m4a, webm, ogg, flac.
- `language` is optional but improves accuracy. ISO 639-1 codes (e.g. `"en"`, `"es"`, `"ja"`).
- Large audio files increase cost proportionally. ~1.3K sats per minute of audio.
- Response includes `text` (full transcript) and `segments` (timestamped chunks).

## Cost

- ~$0.0006 per minute of audio (~1.3K sats/min at typical rates)
- Short clips (< 10 seconds) still cost a minimum.

## Key Constraints
- audio must be base64-encoded audio bytes, not a file path or URL.
- Supported formats: mp3, wav, m4a, webm, ogg, flac.
- Large files increase cost proportionally (~1.3K sats/min of audio).

## Validation Rules
- audio required | audio (base64-encoded) is required for transcription

## Getting Audio as Base64

If you have a file path, use `file_read` to read it and then base64-encode:
```
execute_bash({ "command": "base64 -i audio.mp3" })
```

If you have a URL, download and encode:
```
execute_bash({ "command": "curl -s 'https://example.com/audio.mp3' | base64" })
```
