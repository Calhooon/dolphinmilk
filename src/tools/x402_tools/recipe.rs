//! Recipe tools — atomic, single-call tools for common x402 services.
//!
//! `generate_image` handles auto-polling for banana image generation.
//! `upload_to_nanostore` handles the two-step reserve + PUT upload flow.

use serde_json::{json, Value};

use crate::auth::AuthriteClient;
use crate::wallet::WalletClient;
use crate::x402::{discovery, registry};

use super::call::{extract_base_url, find_endpoint, poll_for_result};
use super::do_x402_request;

// ---------------------------------------------------------------------------
// generate_image
// ---------------------------------------------------------------------------

pub(crate) async fn generate_image_impl(params: Value, wallet_url: String) -> String {
    let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("");

    if prompt.is_empty() {
        return "Error: prompt is required".to_string();
    }

    let resolution = params
        .get("resolution")
        .and_then(|v| v.as_str())
        .unwrap_or("1K");

    let aspect_ratio = params
        .get("aspect_ratio")
        .and_then(|v| v.as_str())
        .unwrap_or("1:1");

    let url = match registry::resolve("banana/generate", None).await {
        Ok(u) => u,
        Err(e) => return format!("Error resolving banana/generate: {e}"),
    };

    let wallet = WalletClient::new(&wallet_url, "http://localhost", 30);
    let auth = AuthriteClient::new(std::sync::Arc::new(wallet), &wallet_url);

    let body = json!({
        "prompt": prompt,
        "resolution": resolution,
        "aspect_ratio": aspect_ratio,
    });

    let result = match do_x402_request(&auth, &url, body).await {
        Ok(val) => val,
        Err(e) => return format!("Error: {e}"),
    };

    // Auto-poll — banana uses async-poll delivery
    let polling_config = match registry::resolve_x402_info("banana", None).await {
        Ok(manifest_url) => match discovery::fetch_manifest_from_url(&manifest_url).await {
            Ok(manifest) => find_endpoint(&manifest, &url)
                .map(|ep| ep.polling.clone())
                .unwrap_or(json!({
                    "endpoint": "predictions/{id}",
                    "interval_seconds": 15,
                    "max_wait_seconds": 300,
                    "terminal_states": ["succeeded", "completed", "failed", "canceled"]
                })),
            Err(_) => json!({
                "endpoint": "predictions/{id}",
                "interval_seconds": 15,
                "max_wait_seconds": 300,
                "terminal_states": ["succeeded", "completed", "failed", "canceled"]
            }),
        },
        Err(_) => json!({
            "endpoint": "predictions/{id}",
            "interval_seconds": 15,
            "max_wait_seconds": 300,
            "terminal_states": ["succeeded", "completed", "failed", "canceled"]
        }),
    };

    if polling_config.is_null() {
        return serde_json::to_string(&result)
            .unwrap_or_else(|_| "Error: serialization failed".to_string());
    }

    let base = extract_base_url(&url);
    match poll_for_result(&auth, &base, &result, &polling_config).await {
        Ok(final_val) => {
            let output_url = final_val
                .get("output")
                .and_then(|v| {
                    v.as_str().map(|s| s.to_string()).or_else(|| {
                        v.as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|item| item.as_str().map(|s| s.to_string()))
                    })
                })
                .unwrap_or_default();

            let sats = result
                .get("sats_paid")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let txid = result
                .get("payment_txid")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if !output_url.is_empty() {
                serde_json::to_string(&serde_json::json!({
                    "success": true,
                    "url": output_url,
                    "sats_paid": sats,
                    "payment_txid": txid,
                    "message": format!("Image generated successfully! Cost: {sats} sats")
                }))
                .unwrap_or_else(|_| "Error: serialization failed".to_string())
            } else {
                let mut val = final_val.clone();
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("sats_paid".to_string(), serde_json::json!(sats));
                    obj.insert("payment_txid".to_string(), serde_json::json!(txid));
                }
                serde_json::to_string(&val)
                    .unwrap_or_else(|_| "Error: serialization failed".to_string())
            }
        }
        Err(e) => {
            let partial = serde_json::to_string(&result).unwrap_or_default();
            format!("Error polling for result: {e}\nInitial response: {partial}")
        }
    }
}

// ---------------------------------------------------------------------------
// MIME helpers
// ---------------------------------------------------------------------------

/// Infer MIME type from file extension. Falls back to `application/octet-stream`.
pub(crate) fn mime_from_extension(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" | "md" | "csv" | "log" => "text/plain",
        "wasm" => "application/wasm",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Sniff MIME type from content prefix. Returns `None` if unrecognizable.
fn sniff_content_type(content: &str) -> Option<&'static str> {
    let trimmed = content.trim_start();
    if trimmed.starts_with("<!DOCTYPE html")
        || trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<HTML")
        || trimmed.starts_with("<head")
        || trimmed.starts_with("<body")
    {
        return Some("text/html");
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Some("application/json");
    }
    if trimmed.starts_with("<?xml") || trimmed.starts_with("<rss") {
        return Some("application/xml");
    }
    if trimmed.starts_with("<svg") {
        return Some("image/svg+xml");
    }
    None
}

// ---------------------------------------------------------------------------
// upload_to_nanostore
// ---------------------------------------------------------------------------

pub(crate) async fn upload_to_nanostore_impl(params: Value, wallet_url: String) -> String {
    let file_path = params.get("file_path").and_then(|v| v.as_str());
    let content_param = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let explicit_ct = params
        .get("content_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let content: String = if let Some(path) = file_path {
        match tokio::fs::read_to_string(path).await {
            Ok(data) => data,
            Err(e) => return format!("Error reading file '{}': {}", path, e),
        }
    } else if !content_param.is_empty() {
        content_param.to_string()
    } else {
        return "Error: either 'content' or 'file_path' is required".to_string();
    };

    let retention_minutes = params
        .get("retention_minutes")
        .and_then(|v| v.as_u64())
        .unwrap_or(525600);

    if retention_minutes < 180 {
        return "Error: retention_minutes must be >= 180 (NanoStore minimum)".to_string();
    }

    let file_size = content.len() as u64;

    let url = match registry::resolve("nanostore/upload", None).await {
        Ok(u) => u,
        Err(e) => return format!("Error resolving nanostore/upload: {e}"),
    };

    let wallet = WalletClient::new(&wallet_url, "http://localhost", 30);
    let auth = AuthriteClient::new(std::sync::Arc::new(wallet), &wallet_url);

    // Step 1: Reserve upload slot
    let body = json!({
        "fileSize": file_size,
        "retentionPeriod": retention_minutes,
    });

    let result = match do_x402_request(&auth, &url, body).await {
        Ok(val) => val,
        Err(e) => return format!("Error reserving upload slot: {e}"),
    };

    // Step 2: Extract upload URL and required headers
    let upload_url = result
        .get("uploadURL")
        .or_else(|| result.get("presignedUrl"))
        .or_else(|| result.get("upload_url"))
        .and_then(|v| v.as_str());

    let upload_url = match upload_url {
        Some(u) => u.to_string(),
        None => {
            return format!(
                "Error: reservation succeeded but no upload URL found in response: {}",
                serde_json::to_string(&result).unwrap_or_default()
            );
        }
    };

    // Build curl headers from requiredHeaders
    let mut header_args = Vec::new();
    if let Some(req_headers) = result.get("requiredHeaders").and_then(|v| v.as_object()) {
        for (key, value) in req_headers {
            let val_str = match value.as_str() {
                Some(s) => s.to_string(),
                None => value.to_string(),
            };
            header_args.push(format!("-H '{}: {}'", key, val_str));
        }
    }

    let has_ct = header_args
        .iter()
        .any(|h| h.to_lowercase().contains("content-type"));
    let resolved_ct = if let Some(ref ct) = explicit_ct {
        ct.as_str()
    } else if let Some(path) = file_path {
        mime_from_extension(path)
    } else {
        sniff_content_type(&content).unwrap_or("application/octet-stream")
    };
    if !has_ct {
        header_args.push(format!("-H 'Content-Type: {}'", resolved_ct));
    }

    let headers_str = header_args.join(" ");

    // Step 2b: Execute the PUT upload directly
    let mut cmd = tokio::process::Command::new("curl");
    cmd.arg("-s").arg("-X").arg("PUT");

    if let Some(req_headers) = result.get("requiredHeaders").and_then(|v| v.as_object()) {
        for (key, value) in req_headers {
            let val_str = match value.as_str() {
                Some(s) => s.to_string(),
                None => value.to_string(),
            };
            cmd.arg("-H").arg(format!("{}: {}", key, val_str));
        }
    }

    if !has_ct {
        cmd.arg("-H").arg(format!("Content-Type: {}", resolved_ct));
    }

    if let Some(path) = file_path {
        cmd.arg("--data-binary").arg(format!("@{}", path));
    } else {
        cmd.arg("--data-binary").arg(&content);
    }
    cmd.arg(&upload_url);

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            if !output.status.success() {
                return format!(
                    "Error: upload PUT failed (exit {})\nURL: {}\nHeaders: {}\nstdout: {}\nstderr: {}",
                    output.status.code().unwrap_or(-1),
                    upload_url,
                    headers_str,
                    stdout,
                    stderr
                );
            }

            let sats = result
                .get("sats_paid")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let txid = result
                .get("payment_txid")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let public_url = upload_url.split('?').next().unwrap_or(&upload_url);

            use sha2::{Digest, Sha256};
            let content_hash = hex::encode(Sha256::digest(content.as_bytes()));

            serde_json::to_string(&serde_json::json!({
                "success": true,
                "public_url": public_url,
                "content_sha256": content_hash,
                "sats_paid": sats,
                "payment_txid": txid,
                "retention_minutes": retention_minutes,
                "file_size": file_size,
                "message": format!("Upload complete. File stored on NanoStore. Cost: {sats} sats")
            }))
            .unwrap_or_else(|_| "Error: serialization failed".to_string())
        }
        Err(e) => {
            format!(
                "Error executing upload: {e}\n\n\
                 Manual fallback — run this command:\n\
                 curl -s -X PUT {headers_str} --data-binary 'YOUR_CONTENT' '{upload_url}'"
            )
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_generate_image_validates_prompt() {
        let result = generate_image_impl(json!({}), "http://localhost:3322".into()).await;
        assert!(
            result.contains("prompt is required"),
            "should reject empty prompt, got: {result}"
        );

        let result =
            generate_image_impl(json!({"prompt": ""}), "http://localhost:3322".into()).await;
        assert!(
            result.contains("prompt is required"),
            "should reject empty string prompt, got: {result}"
        );
    }

    #[tokio::test]
    async fn test_upload_to_nanostore_validates_content() {
        let result = upload_to_nanostore_impl(json!({}), "http://localhost:3322".into()).await;
        assert!(
            result.contains("'content' or 'file_path' is required"),
            "should reject no content or file_path, got: {result}"
        );

        let result =
            upload_to_nanostore_impl(json!({"content": ""}), "http://localhost:3322".into()).await;
        assert!(
            result.contains("'content' or 'file_path' is required"),
            "should reject empty string content, got: {result}"
        );

        let result = upload_to_nanostore_impl(
            json!({"file_path": "/nonexistent/file.txt"}),
            "http://localhost:3322".into(),
        )
        .await;
        assert!(
            result.contains("Error reading file"),
            "should reject bad file_path, got: {result}"
        );
    }

    #[tokio::test]
    async fn test_upload_to_nanostore_validates_retention() {
        let result = upload_to_nanostore_impl(
            json!({"content": "hello", "retention_minutes": 60}),
            "http://localhost:3322".into(),
        )
        .await;
        assert!(
            result.contains("retention_minutes must be >= 180"),
            "should reject low retention, got: {result}"
        );
    }

    #[test]
    fn test_mime_from_extension() {
        assert_eq!(mime_from_extension("page.html"), "text/html");
        assert_eq!(mime_from_extension("page.htm"), "text/html");
        assert_eq!(mime_from_extension("style.css"), "text/css");
        assert_eq!(mime_from_extension("app.js"), "application/javascript");
        assert_eq!(mime_from_extension("data.json"), "application/json");
        assert_eq!(mime_from_extension("feed.xml"), "application/xml");
        assert_eq!(mime_from_extension("icon.svg"), "image/svg+xml");

        assert_eq!(mime_from_extension("photo.png"), "image/png");
        assert_eq!(mime_from_extension("photo.jpg"), "image/jpeg");
        assert_eq!(mime_from_extension("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_from_extension("anim.gif"), "image/gif");
        assert_eq!(mime_from_extension("pic.webp"), "image/webp");

        assert_eq!(mime_from_extension("doc.pdf"), "application/pdf");
        assert_eq!(mime_from_extension("readme.txt"), "text/plain");
        assert_eq!(mime_from_extension("notes.md"), "text/plain");
        assert_eq!(mime_from_extension("data.csv"), "text/plain");

        assert_eq!(mime_from_extension("song.mp3"), "audio/mpeg");
        assert_eq!(mime_from_extension("video.mp4"), "video/mp4");
        assert_eq!(mime_from_extension("clip.webm"), "video/webm");

        assert_eq!(mime_from_extension("PAGE.HTML"), "text/html");
        assert_eq!(mime_from_extension("Data.JSON"), "application/json");

        assert_eq!(mime_from_extension("/some/path/to/file.html"), "text/html");
        assert_eq!(
            mime_from_extension("working/tasks/abc/article.html"),
            "text/html"
        );

        assert_eq!(mime_from_extension("file.xyz"), "application/octet-stream");
        assert_eq!(
            mime_from_extension("noextension"),
            "application/octet-stream"
        );
        assert_eq!(mime_from_extension(""), "application/octet-stream");
    }

    #[test]
    fn test_sniff_content_type() {
        assert_eq!(
            sniff_content_type("<!DOCTYPE html><html>"),
            Some("text/html")
        );
        assert_eq!(sniff_content_type("<!doctype html>"), Some("text/html"));
        assert_eq!(sniff_content_type("<html lang=\"en\">"), Some("text/html"));
        assert_eq!(sniff_content_type("<HTML>"), Some("text/html"));
        assert_eq!(sniff_content_type("<head><title>"), Some("text/html"));
        assert_eq!(sniff_content_type("<body>hello"), Some("text/html"));

        assert_eq!(
            sniff_content_type("  \n  <!DOCTYPE html>"),
            Some("text/html")
        );

        assert_eq!(
            sniff_content_type("{\"key\": \"value\"}"),
            Some("application/json")
        );
        assert_eq!(sniff_content_type("[1, 2, 3]"), Some("application/json"));

        assert_eq!(
            sniff_content_type("<?xml version=\"1.0\"?>"),
            Some("application/xml")
        );
        assert_eq!(
            sniff_content_type("<rss version=\"2.0\">"),
            Some("application/xml")
        );

        assert_eq!(sniff_content_type("<svg xmlns="), Some("image/svg+xml"));

        assert_eq!(sniff_content_type("Hello world"), None);
        assert_eq!(sniff_content_type(""), None);
        assert_eq!(sniff_content_type("fn main() {}"), None);
    }
}
