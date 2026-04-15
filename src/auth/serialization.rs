//! BRC-104 binary payload serialization for BRC-31 Authrite.
//!
//! Implements Bitcoin-style varint encoding and the binary request/response
//! serialization format used by BRC-31 mutual authentication.
//!
//! Wire format matches TS SDK AuthFetch.serializeRequest() and the Rust SDK
//! cloudflare.rs build_request_payload() exactly.

/// EMPTY_SENTINEL: 9 bytes of 0xFF.
/// Represents "missing/empty" for optional fields (path, query, body).
/// Equivalent to writeVarIntNum(-1) in the TS SDK.
pub const EMPTY_SENTINEL: [u8; 9] = [0xFF; 9];

/// Write a Bitcoin-style variable-length integer.
///
/// Encoding:
/// - n <= 252: 1 byte
/// - n <= 0xFFFF: 0xFD + 2-byte LE
/// - n <= 0xFFFFFFFF: 0xFE + 4-byte LE
/// - else: 0xFF + 8-byte LE
pub fn write_varint(buf: &mut Vec<u8>, n: u64) {
    if n <= 252 {
        buf.push(n as u8);
    } else if n <= 0xFFFF {
        buf.push(0xFD);
        buf.extend_from_slice(&(n as u16).to_le_bytes());
    } else if n <= 0xFFFF_FFFF {
        buf.push(0xFE);
        buf.extend_from_slice(&(n as u32).to_le_bytes());
    } else {
        buf.push(0xFF);
        buf.extend_from_slice(&n.to_le_bytes());
    }
}

/// Read a Bitcoin-style variable-length integer from a byte slice.
/// Returns (value, bytes_consumed).
pub fn read_varint(data: &[u8]) -> Result<(u64, usize), &'static str> {
    if data.is_empty() {
        return Err("unexpected end of data reading varint");
    }
    let first = data[0];
    match first {
        0..=252 => Ok((first as u64, 1)),
        0xFD => {
            if data.len() < 3 {
                return Err("insufficient bytes for 2-byte varint");
            }
            let val = u16::from_le_bytes([data[1], data[2]]);
            Ok((val as u64, 3))
        }
        0xFE => {
            if data.len() < 5 {
                return Err("insufficient bytes for 4-byte varint");
            }
            let val = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
            Ok((val as u64, 5))
        }
        0xFF => {
            if data.len() < 9 {
                return Err("insufficient bytes for 8-byte varint");
            }
            let val = u64::from_le_bytes([
                data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
            ]);
            Ok((val, 9))
        }
    }
}

/// Write varint-prefixed bytes.
fn write_varint_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    write_varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

/// Write an optional string field: varint-prefixed UTF-8 or EMPTY_SENTINEL.
fn write_optional_string(buf: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(s) if !s.is_empty() => write_varint_bytes(buf, s.as_bytes()),
        _ => buf.extend_from_slice(&EMPTY_SENTINEL),
    }
}

/// Write an optional body field: varint-prefixed bytes or EMPTY_SENTINEL.
fn write_optional_bytes(buf: &mut Vec<u8>, data: Option<&[u8]>) {
    match data {
        Some(d) if !d.is_empty() => write_varint_bytes(buf, d),
        _ => buf.extend_from_slice(&EMPTY_SENTINEL),
    }
}

/// Write sorted header pairs: varint(count) then for each (key, value).
fn write_headers(buf: &mut Vec<u8>, headers: &[(String, String)]) {
    write_varint(buf, headers.len() as u64);
    for (key, value) in headers {
        write_varint_bytes(buf, key.as_bytes());
        write_varint_bytes(buf, value.as_bytes());
    }
}

/// Serialize an HTTP request into BRC-104 binary format for signing.
///
/// Format:
/// ```text
/// [request_id:  32 raw bytes, NOT varint-prefixed]
/// [method:      varint(len) + UTF-8 bytes]
/// [path:        varint(len) + UTF-8 bytes, OR EMPTY_SENTINEL]
/// [query:       varint(len) + UTF-8 bytes, OR EMPTY_SENTINEL]
/// [headers:     varint(count) then for each: varint(key_len)+key + varint(val_len)+val]
/// [body:        varint(len) + bytes, OR EMPTY_SENTINEL]
/// ```
pub fn serialize_request(
    request_id: &[u8; 32],
    method: &str,
    path: Option<&str>,
    query: Option<&str>,
    signable_headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    // 1. Request ID — raw 32 bytes, NOT varint-prefixed
    buf.extend_from_slice(request_id);

    // 2. Method — varint-prefixed UTF-8
    write_varint_bytes(&mut buf, method.as_bytes());

    // 3. Path — varint-prefixed or EMPTY_SENTINEL
    write_optional_string(&mut buf, path);

    // 4. Query — varint-prefixed or EMPTY_SENTINEL
    write_optional_string(&mut buf, query);

    // 5. Headers — varint count + sorted pairs
    write_headers(&mut buf, signable_headers);

    // 6. Body — varint-prefixed or EMPTY_SENTINEL
    write_optional_bytes(&mut buf, body);

    buf
}

/// Serialize an HTTP response into BRC-104 binary format for verification.
///
/// Format:
/// ```text
/// [request_id:  32 raw bytes]
/// [status_code: varint]
/// [headers:     varint(count) + pairs]
/// [body:        varint(len) + bytes, OR EMPTY_SENTINEL]
/// ```
pub fn serialize_response(
    request_id: &[u8; 32],
    status_code: u16,
    signable_headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);

    // 1. Request ID — raw 32 bytes
    buf.extend_from_slice(request_id);

    // 2. Status code — varint
    write_varint(&mut buf, status_code as u64);

    // 3. Headers — varint count + sorted pairs
    write_headers(&mut buf, signable_headers);

    // 4. Body — varint-prefixed or EMPTY_SENTINEL
    write_optional_bytes(&mut buf, body);

    buf
}

/// Check if bytes at the given offset are the EMPTY_SENTINEL.
fn is_empty_sentinel(data: &[u8], offset: usize) -> bool {
    if offset + 9 > data.len() {
        return false;
    }
    data[offset..offset + 9] == EMPTY_SENTINEL
}

/// Deserialized request payload.
#[derive(Debug, Clone)]
pub struct DeserializedRequest {
    pub request_id: [u8; 32],
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// Deserialized response payload.
#[derive(Debug, Clone)]
pub struct DeserializedResponse {
    pub request_id: [u8; 32],
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// Deserialize a BRC-104 request payload.
pub fn deserialize_request(payload: &[u8]) -> Result<DeserializedRequest, String> {
    let mut pos = 0;

    // 1. Request ID — raw 32 bytes
    if payload.len() < 32 {
        return Err("payload too short for request_id".into());
    }
    let mut request_id = [0u8; 32];
    request_id.copy_from_slice(&payload[..32]);
    pos += 32;

    // 2. Method
    let (method_len, consumed) =
        read_varint(&payload[pos..]).map_err(|e| format!("method varint: {e}"))?;
    pos += consumed;
    let method_end = pos + method_len as usize;
    if method_end > payload.len() {
        return Err("method extends past payload".into());
    }
    let method = String::from_utf8(payload[pos..method_end].to_vec())
        .map_err(|e| format!("method utf8: {e}"))?;
    pos = method_end;

    // 3. Path
    let path = if is_empty_sentinel(payload, pos) {
        pos += 9;
        String::new()
    } else {
        let (path_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("path varint: {e}"))?;
        pos += consumed;
        let path_end = pos + path_len as usize;
        if path_end > payload.len() {
            return Err("path extends past payload".into());
        }
        let p = String::from_utf8(payload[pos..path_end].to_vec())
            .map_err(|e| format!("path utf8: {e}"))?;
        pos = path_end;
        p
    };

    // 4. Query
    let query = if is_empty_sentinel(payload, pos) {
        pos += 9;
        String::new()
    } else {
        let (query_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("query varint: {e}"))?;
        pos += consumed;
        let query_end = pos + query_len as usize;
        if query_end > payload.len() {
            return Err("query extends past payload".into());
        }
        let q = String::from_utf8(payload[pos..query_end].to_vec())
            .map_err(|e| format!("query utf8: {e}"))?;
        pos = query_end;
        q
    };

    // 5. Headers
    let (header_count, consumed) =
        read_varint(&payload[pos..]).map_err(|e| format!("header count varint: {e}"))?;
    pos += consumed;
    let mut headers = Vec::with_capacity(header_count as usize);
    for _ in 0..header_count {
        let (key_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("header key varint: {e}"))?;
        pos += consumed;
        let key_end = pos + key_len as usize;
        if key_end > payload.len() {
            return Err("header key extends past payload".into());
        }
        let key = String::from_utf8(payload[pos..key_end].to_vec())
            .map_err(|e| format!("header key utf8: {e}"))?;
        pos = key_end;

        let (val_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("header val varint: {e}"))?;
        pos += consumed;
        let val_end = pos + val_len as usize;
        if val_end > payload.len() {
            return Err("header val extends past payload".into());
        }
        let val = String::from_utf8(payload[pos..val_end].to_vec())
            .map_err(|e| format!("header val utf8: {e}"))?;
        pos = val_end;

        headers.push((key, val));
    }

    // 6. Body
    let body = if is_empty_sentinel(payload, pos) {
        None
    } else {
        let (body_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("body varint: {e}"))?;
        pos += consumed;
        let body_end = pos + body_len as usize;
        if body_end > payload.len() {
            return Err("body extends past payload".into());
        }
        Some(payload[pos..body_end].to_vec())
    };

    Ok(DeserializedRequest {
        request_id,
        method,
        path,
        query,
        headers,
        body,
    })
}

/// Deserialize a BRC-104 response payload.
pub fn deserialize_response(payload: &[u8]) -> Result<DeserializedResponse, String> {
    let mut pos = 0;

    // 1. Request ID
    if payload.len() < 32 {
        return Err("payload too short for request_id".into());
    }
    let mut request_id = [0u8; 32];
    request_id.copy_from_slice(&payload[..32]);
    pos += 32;

    // 2. Status code
    let (status_code, consumed) =
        read_varint(&payload[pos..]).map_err(|e| format!("status_code varint: {e}"))?;
    pos += consumed;

    // 3. Headers
    let (header_count, consumed) =
        read_varint(&payload[pos..]).map_err(|e| format!("header count varint: {e}"))?;
    pos += consumed;
    let mut headers = Vec::with_capacity(header_count as usize);
    for _ in 0..header_count {
        let (key_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("header key varint: {e}"))?;
        pos += consumed;
        let key_end = pos + key_len as usize;
        if key_end > payload.len() {
            return Err("header key extends past payload".into());
        }
        let key = String::from_utf8(payload[pos..key_end].to_vec())
            .map_err(|e| format!("header key utf8: {e}"))?;
        pos = key_end;

        let (val_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("header val varint: {e}"))?;
        pos += consumed;
        let val_end = pos + val_len as usize;
        if val_end > payload.len() {
            return Err("header val extends past payload".into());
        }
        let val = String::from_utf8(payload[pos..val_end].to_vec())
            .map_err(|e| format!("header val utf8: {e}"))?;
        pos = val_end;

        headers.push((key, val));
    }

    // 4. Body
    let body = if is_empty_sentinel(payload, pos) {
        None
    } else {
        let (body_len, consumed) =
            read_varint(&payload[pos..]).map_err(|e| format!("body varint: {e}"))?;
        pos += consumed;
        let body_end = pos + body_len as usize;
        if body_end > payload.len() {
            return Err("body extends past payload".into());
        }
        Some(payload[pos..body_end].to_vec())
    };

    Ok(DeserializedResponse {
        request_id,
        status_code: status_code as u16,
        headers,
        body,
    })
}

/// Filter and sort headers for BRC-31 signature.
///
/// Rules:
/// - Include `x-bsv-*` headers EXCEPT `x-bsv-auth-*`
/// - Include `authorization` verbatim
/// - Include `content-type` (media type only — strip `;charset=...` params)
/// - Sort alphabetically by lowercase key
pub fn filter_signable_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut result = Vec::new();

    for (key, value) in headers {
        let lower = key.to_lowercase();

        // Exclude x-bsv-auth-* (carries the signature itself)
        if lower.starts_with("x-bsv-auth-") {
            continue;
        }

        // Include other x-bsv-* headers
        if lower.starts_with("x-bsv-") {
            result.push((lower, value.clone()));
            continue;
        }

        // Include authorization verbatim
        if lower == "authorization" {
            result.push((lower, value.clone()));
            continue;
        }

        // Include content-type, media type only (strip params)
        if lower == "content-type" {
            let media_type = value.split(';').next().unwrap_or(value).trim().to_string();
            result.push((lower, media_type));
            continue;
        }

        // All other headers silently excluded
    }

    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

/// Filter and sort response headers for BRC-31 signature verification.
///
/// Response signing uses different rules than request signing:
/// - Include `x-bsv-*` headers EXCEPT `x-bsv-auth-*`
/// - Include `authorization` verbatim
/// - **NO content-type** (unlike request signing)
/// - Sort alphabetically by lowercase key
pub fn filter_response_signable_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut result = Vec::new();

    for (key, value) in headers {
        let lower = key.to_lowercase();

        // Exclude x-bsv-auth-* (carries the signature itself)
        if lower.starts_with("x-bsv-auth-") {
            continue;
        }

        // Include other x-bsv-* headers
        if lower.starts_with("x-bsv-") {
            result.push((lower, value.clone()));
            continue;
        }

        // Include authorization verbatim
        if lower == "authorization" {
            result.push((lower, value.clone()));
            continue;
        }

        // All other headers silently excluded (including content-type)
    }

    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

/// Build BRC-104 auth headers for a general authenticated request.
pub fn build_auth_headers(
    identity_key: &str,
    nonce_b64: &str,
    your_nonce_b64: &str,
    signature_hex: &str,
    request_id_b64: &str,
) -> Vec<(String, String)> {
    vec![
        ("x-bsv-auth-version".into(), "0.1".into()),
        ("x-bsv-auth-identity-key".into(), identity_key.into()),
        ("x-bsv-auth-message-type".into(), "general".into()),
        ("x-bsv-auth-nonce".into(), nonce_b64.into()),
        ("x-bsv-auth-your-nonce".into(), your_nonce_b64.into()),
        ("x-bsv-auth-signature".into(), signature_hex.into()),
        ("x-bsv-auth-request-id".into(), request_id_b64.into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_single_byte() {
        for n in [0u64, 1, 100, 252] {
            let mut buf = Vec::new();
            write_varint(&mut buf, n);
            assert_eq!(buf.len(), 1);
            let (val, consumed) = read_varint(&buf).unwrap();
            assert_eq!(val, n);
            assert_eq!(consumed, 1);
        }
    }

    #[test]
    fn test_varint_two_byte() {
        for n in [253u64, 0xFFFF] {
            let mut buf = Vec::new();
            write_varint(&mut buf, n);
            assert_eq!(buf.len(), 3);
            assert_eq!(buf[0], 0xFD);
            let (val, consumed) = read_varint(&buf).unwrap();
            assert_eq!(val, n);
            assert_eq!(consumed, 3);
        }
    }

    #[test]
    fn test_varint_four_byte() {
        for n in [0x10000u64, 0xFFFF_FFFF] {
            let mut buf = Vec::new();
            write_varint(&mut buf, n);
            assert_eq!(buf.len(), 5);
            assert_eq!(buf[0], 0xFE);
            let (val, consumed) = read_varint(&buf).unwrap();
            assert_eq!(val, n);
            assert_eq!(consumed, 5);
        }
    }

    #[test]
    fn test_varint_eight_byte() {
        let n = 0x1_0000_0000u64;
        let mut buf = Vec::new();
        write_varint(&mut buf, n);
        assert_eq!(buf.len(), 9);
        assert_eq!(buf[0], 0xFF);
        let (val, consumed) = read_varint(&buf).unwrap();
        assert_eq!(val, n);
        assert_eq!(consumed, 9);
    }

    #[test]
    fn test_empty_sentinel() {
        assert_eq!(EMPTY_SENTINEL.len(), 9);
        assert!(EMPTY_SENTINEL.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn test_empty_sentinel_detection() {
        let data = [0xFF; 9];
        assert!(is_empty_sentinel(&data, 0));
        assert!(!is_empty_sentinel(&data, 1)); // not enough bytes from offset 1

        let mut data2 = vec![0x00; 5];
        data2.extend_from_slice(&EMPTY_SENTINEL);
        assert!(!is_empty_sentinel(&data2, 0));
        assert!(is_empty_sentinel(&data2, 5));
    }

    #[test]
    fn test_request_serialization_roundtrip() {
        let request_id: [u8; 32] = [42u8; 32];
        let method = "POST";
        let path = Some("/sendMessage");
        let query = Some("?foo=bar");
        let headers = vec![
            ("content-type".into(), "application/json".into()),
            ("x-bsv-payment".into(), "some-payment-data".into()),
        ];
        let body = Some(b"{\"hello\":\"world\"}" as &[u8]);

        let serialized = serialize_request(&request_id, method, path, query, &headers, body);
        let deserialized = deserialize_request(&serialized).unwrap();

        assert_eq!(deserialized.request_id, request_id);
        assert_eq!(deserialized.method, "POST");
        assert_eq!(deserialized.path, "/sendMessage");
        assert_eq!(deserialized.query, "?foo=bar");
        assert_eq!(deserialized.headers.len(), 2);
        assert_eq!(
            deserialized.headers[0],
            ("content-type".into(), "application/json".into())
        );
        assert_eq!(
            deserialized.body.as_deref(),
            Some(b"{\"hello\":\"world\"}" as &[u8])
        );
    }

    #[test]
    fn test_request_serialization_empty_optionals() {
        let request_id: [u8; 32] = [1u8; 32];
        let serialized = serialize_request(&request_id, "GET", None, None, &[], None);
        let deserialized = deserialize_request(&serialized).unwrap();

        assert_eq!(deserialized.method, "GET");
        assert_eq!(deserialized.path, "");
        assert_eq!(deserialized.query, "");
        assert_eq!(deserialized.headers.len(), 0);
        assert!(deserialized.body.is_none());
    }

    #[test]
    fn test_response_serialization_roundtrip() {
        let request_id: [u8; 32] = [7u8; 32];
        let headers = vec![("x-bsv-result".into(), "ok".into())];
        let body = Some(b"response body" as &[u8]);

        let serialized = serialize_response(&request_id, 200, &headers, body);
        let deserialized = deserialize_response(&serialized).unwrap();

        assert_eq!(deserialized.request_id, request_id);
        assert_eq!(deserialized.status_code, 200);
        assert_eq!(deserialized.headers.len(), 1);
        assert_eq!(
            deserialized.body.as_deref(),
            Some(b"response body" as &[u8])
        );
    }

    #[test]
    fn test_response_serialization_no_body() {
        let request_id: [u8; 32] = [0u8; 32];
        let serialized = serialize_response(&request_id, 404, &[], None);
        let deserialized = deserialize_response(&serialized).unwrap();

        assert_eq!(deserialized.status_code, 404);
        assert!(deserialized.body.is_none());
    }

    #[test]
    fn test_filter_signable_headers_includes_x_bsv() {
        let headers = vec![
            ("X-BSV-Payment".into(), "some-payment".into()),
            ("X-BSV-Auth-Nonce".into(), "should-be-excluded".into()),
            ("Authorization".into(), "Bearer token".into()),
            (
                "Content-Type".into(),
                "application/json; charset=utf-8".into(),
            ),
            ("Accept".into(), "text/html".into()),
        ];

        let result = filter_signable_headers(&headers);

        assert_eq!(result.len(), 3);
        // Sorted alphabetically
        assert_eq!(result[0].0, "authorization");
        assert_eq!(result[0].1, "Bearer token");
        assert_eq!(result[1].0, "content-type");
        assert_eq!(result[1].1, "application/json"); // params stripped
        assert_eq!(result[2].0, "x-bsv-payment");
        assert_eq!(result[2].1, "some-payment");
    }

    #[test]
    fn test_filter_signable_headers_excludes_auth() {
        let headers = vec![
            ("x-bsv-auth-version".into(), "0.1".into()),
            ("x-bsv-auth-signature".into(), "sig".into()),
            ("x-bsv-auth-identity-key".into(), "key".into()),
        ];
        let result = filter_signable_headers(&headers);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_auth_headers() {
        let headers = build_auth_headers(
            "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
            "msg_nonce_b64",
            "server_nonce_b64",
            "deadbeef",
            "request_id_b64",
        );
        assert_eq!(headers.len(), 7);
        assert_eq!(headers[0], ("x-bsv-auth-version".into(), "0.1".into()));
        assert_eq!(
            headers[2],
            ("x-bsv-auth-message-type".into(), "general".into())
        );
        assert_eq!(
            headers[5],
            ("x-bsv-auth-signature".into(), "deadbeef".into())
        );
    }

    #[test]
    fn test_filter_response_signable_headers_excludes_content_type() {
        let headers = vec![
            ("X-BSV-Payment".into(), "some-payment".into()),
            ("Authorization".into(), "Bearer token".into()),
            (
                "Content-Type".into(),
                "application/json; charset=utf-8".into(),
            ),
            ("Accept".into(), "text/html".into()),
        ];

        let result = filter_response_signable_headers(&headers);

        // content-type should NOT be included (unlike request signing)
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "authorization");
        assert_eq!(result[0].1, "Bearer token");
        assert_eq!(result[1].0, "x-bsv-payment");
        assert_eq!(result[1].1, "some-payment");
    }

    #[test]
    fn test_filter_response_signable_headers_includes_x_bsv() {
        let headers = vec![
            ("x-bsv-payment".into(), "pay-data".into()),
            ("x-bsv-custom".into(), "custom-val".into()),
        ];

        let result = filter_response_signable_headers(&headers);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "x-bsv-custom");
        assert_eq!(result[1].0, "x-bsv-payment");
    }

    #[test]
    fn test_filter_response_signable_headers_excludes_x_bsv_auth() {
        let headers = vec![
            ("x-bsv-auth-version".into(), "0.1".into()),
            ("x-bsv-auth-signature".into(), "sig".into()),
            ("x-bsv-auth-identity-key".into(), "key".into()),
        ];
        let result = filter_response_signable_headers(&headers);
        assert!(result.is_empty());
    }
}
