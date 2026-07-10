//! The Brother "pedxml" wire format.
//!
//! This module is a byte-faithful transcription of the protocol captured from
//! Brother's official *Design Database Transfer* application (see the
//! reverse-engineering notes in the reference PoC's PROTOCOL.md). It contains
//! no I/O: it only builds request bodies and parses responses, which keeps it
//! trivially unit-testable.
//!
//! Quirks worth knowing, all deliberate:
//!
//! * The multipart body is built by hand, **not** with a standard multipart
//!   library. The official client emits nonstandard header formatting
//!   (`Content-Disposition:form-data;name=...` — no space after `:` or `;`),
//!   and embedded CGI parsers are strict; we reproduce the capture exactly.
//! * The response XML root element is misspelled by the firmware
//!   (`<respose_info>`). We parse by tag extraction rather than with a strict
//!   XML parser so firmware typos and future fields can't break us.
//! * `req_appver` differs between the status call (`1.2.0`) and the upload
//!   call (`100`) in the capture. Faithfully reproduced.

use super::models::SewingResponse;
use crate::machine::MachineError;

/// Identification endpoint (JSON).
pub const INFO_PATH: &str = "/info";
/// Session/status/upload endpoint (form-encoded or multipart in, XML out).
pub const SEWING_PATH: &str = "/sewing/sewing.cgi";

/// The machine only answers clients that look like the official application.
pub const USER_AGENT: &str = "Design Database Transfer";
/// Sent by the official client; a Windows LCID as a string (1046 = pt-BR in
/// the capture). The machine does not appear to care about the value.
pub const ACCEPT_LANGUAGE: &str = "1046";

/// Application id the official client identifies itself with.
pub const APP_ID: &str = "23";

/// `req_appstate` values.
const APPSTATE_STATUS: &str = "2";
const APPSTATE_UPLOAD: &str = "3";

/// Body of the status/handshake request (`Content-Type:
/// application/x-www-form-urlencoded`).
pub fn status_body() -> String {
    format!("req_sessionid=0&req_appid={APP_ID}&req_appver=1.2.0&req_appstate={APPSTATE_STATUS}")
}

/// Multipart boundary in the exact shape the official client generates:
/// 27 dashes followed by 12 hex characters.
pub fn multipart_boundary(random_hex12: &str) -> String {
    format!("---------------------------{random_hex12}")
}

/// `Content-Type` header value for the upload request.
pub fn upload_content_type(boundary: &str) -> String {
    format!("multipart/form-data;boundary={boundary}")
}

/// Build the multipart upload body, byte-for-byte as captured:
///
/// * part 1 `req_parameter`: the form-encoded command (`req_appstate=3`),
/// * part 2 `myfile`: the raw design bytes.
///
/// The machine ignores the part-2 filename and assigns its own (e.g.
/// `32770.PES`), but we still send a sanitized version of the real name.
pub fn upload_body(boundary: &str, filename: &str, design: &[u8]) -> Vec<u8> {
    const CRLF: &[u8] = b"\r\n";
    let params =
        format!("req_sessionid=0&req_appid={APP_ID}&req_appver=100&req_appstate={APPSTATE_UPLOAD}");
    let safe_name = sanitize_filename(filename);

    let mut body = Vec::with_capacity(design.len() + 512);
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(CRLF);
    body.extend_from_slice(
        b"Content-Disposition:form-data;name=\"req_parameter\";filename=\"req_parameter\"",
    );
    body.extend_from_slice(CRLF);
    body.extend_from_slice(b"Content-Type:application/x-www-form-urlencoded");
    body.extend_from_slice(CRLF);
    body.extend_from_slice(CRLF);
    body.extend_from_slice(params.as_bytes());
    body.extend_from_slice(CRLF);
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(CRLF);
    body.extend_from_slice(b"Content-Disposition:form-data;name=\"myfile\";filename=\"");
    body.extend_from_slice(safe_name.as_bytes());
    body.extend_from_slice(b"\"");
    body.extend_from_slice(CRLF);
    body.extend_from_slice(b"Content-Type:application/octet-stream");
    body.extend_from_slice(CRLF);
    body.extend_from_slice(CRLF);
    body.extend_from_slice(design);
    body.extend_from_slice(CRLF);
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--");
    body.extend_from_slice(CRLF);
    body
}

/// Restrict a filename to ASCII that cannot break the multipart framing.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Parse the XML answer of `POST /sewing/sewing.cgi`.
///
/// Tolerant by design: extracts known tags and ignores everything else, so
/// the firmware's misspelled root element and any future additions pass
/// through harmlessly.
pub fn parse_sewing_response(xml: &str) -> Result<SewingResponse, MachineError> {
    let error_code = extract_tag(xml, "error_code")
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(|| {
            MachineError::Protocol(format!(
                "machine response has no parsable <error_code>: {}",
                truncate(xml, 200)
            ))
        })?;

    Ok(SewingResponse {
        error_code,
        session_id: extract_tag(xml, "session_id"),
        upload_path: extract_tag(xml, "upload_path"),
        upload_size: extract_tag(xml, "upload_size").and_then(|v| v.parse().ok()),
        upload_freesize: extract_tag(xml, "upload_freesize").and_then(|v| v.parse().ok()),
        files: extract_all_tags(xml, "file_name"),
    })
}

/// First occurrence of `<tag>...</tag>`, trimmed.
fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

/// Every occurrence of `<tag>...</tag>`, trimmed.
fn extract_all_tags(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find(&open) {
        let start = pos + open.len();
        let Some(end_rel) = rest[start..].find(&close) else {
            break;
        };
        out.push(rest[start..start + end_rel].trim().to_string());
        rest = &rest[start + end_rel + close.len()..];
    }
    out
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Response captured from an Innov-is BP1530L (note the firmware's
    /// misspelled root element).
    const CAPTURED_STATUS: &str = r#"<respose_info>
  <respose_status><error_code>0</error_code></respose_status>
  <term_info><session_id>0</session_id></term_info>
  <data_path>
    <upload_path>/sewing/dataupl.cgi</upload_path>
    <upload_size>4194304</upload_size>
    <upload_freesize>4133845</upload_freesize>
  </data_path>
  <files>
    <file_name>32769.PES</file_name>
    <file_name>32770.PES</file_name>
  </files>
</respose_info>"#;

    #[test]
    fn parses_captured_status_response() {
        let parsed = parse_sewing_response(CAPTURED_STATUS).unwrap();
        assert_eq!(
            parsed,
            SewingResponse {
                error_code: 0,
                session_id: Some("0".into()),
                upload_path: Some("/sewing/dataupl.cgi".into()),
                upload_size: Some(4_194_304),
                upload_freesize: Some(4_133_845),
                files: vec!["32769.PES".into(), "32770.PES".into()],
            }
        );
    }

    #[test]
    fn missing_error_code_is_a_protocol_error() {
        assert!(matches!(
            parse_sewing_response("<html>gateway error</html>"),
            Err(MachineError::Protocol(_))
        ));
    }

    #[test]
    fn status_body_matches_capture() {
        assert_eq!(
            status_body(),
            "req_sessionid=0&req_appid=23&req_appver=1.2.0&req_appstate=2"
        );
    }

    #[test]
    fn upload_body_matches_captured_layout() {
        let boundary = multipart_boundary("0123456789ab");
        let body = upload_body(&boundary, "rose.pes", b"#PES0001");
        let text = String::from_utf8_lossy(&body);

        let expected = "-----------------------------0123456789ab\r\n\
Content-Disposition:form-data;name=\"req_parameter\";filename=\"req_parameter\"\r\n\
Content-Type:application/x-www-form-urlencoded\r\n\
\r\n\
req_sessionid=0&req_appid=23&req_appver=100&req_appstate=3\r\n\
-----------------------------0123456789ab\r\n\
Content-Disposition:form-data;name=\"myfile\";filename=\"rose.pes\"\r\n\
Content-Type:application/octet-stream\r\n\
\r\n\
#PES0001\r\n\
-----------------------------0123456789ab--\r\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn filenames_are_sanitized() {
        let body = upload_body("b", "wéird name\";.pes", b"x");
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("filename=\"w_ird_name__.pes\""));
    }
}
