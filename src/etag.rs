//! ETag / conditional-request helpers.

pub use api_bones::etag::{ETag, IfMatch, IfNoneMatch, ParseETagError};

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};

use crate::ApiError;

/// Derive a weak [`ETag`] from an `updated_at` timestamp.
pub fn etag_from_updated_at(updated_at: DateTime<Utc>) -> ETag {
    let millis = updated_at.timestamp_millis();
    ETag::weak(format!("{millis:x}"))
}

/// Validate the `If-Match` request header against a current [`ETag`].
pub fn check_if_match(headers: &HeaderMap, current_etag: &ETag) -> Result<(), ApiError> {
    let raw = match headers.get(axum::http::header::IF_MATCH) {
        None => {
            let mut err = ApiError::new(
                api_bones::error::ErrorCode::BadRequest,
                "If-Match header is required",
            );
            err.status = 428;
            err.title = "Precondition Required".to_owned();
            return Err(err);
        }
        Some(v) => v
            .to_str()
            .map_err(|_| ApiError::bad_request("If-Match header is not valid ASCII"))?,
    };

    let trimmed = raw.trim();
    let matched = if trimmed == "*" {
        true
    } else {
        let tags = ETag::parse_list(trimmed)
            .map_err(|e| ApiError::bad_request(format!("If-Match header is malformed: {e}")))?;
        tags.iter().any(|t| t.matches_weak(current_etag))
    };

    if matched {
        Ok(())
    } else {
        let mut err = ApiError::new(
            api_bones::error::ErrorCode::BadRequest,
            "ETag does not match; the resource has been modified",
        );
        err.status = 412;
        err.title = "Precondition Failed".to_owned();
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use chrono::TimeZone as _;

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            axum::http::HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn etag_from_updated_at_is_deterministic() {
        let ts = chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let a = etag_from_updated_at(ts);
        let b = etag_from_updated_at(ts);
        assert_eq!(a.to_string(), b.to_string());
        assert!(a.weak);
    }

    #[test]
    fn etag_from_updated_at_different_times_differ() {
        let t1 = chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t2 = chrono::Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        assert_ne!(
            etag_from_updated_at(t1).to_string(),
            etag_from_updated_at(t2).to_string()
        );
    }

    #[test]
    fn check_if_match_missing_header_returns_428() {
        let current = ETag::strong("abc".to_string());
        let err = check_if_match(&HeaderMap::new(), &current).unwrap_err();
        assert_eq!(err.status, 428);
    }

    #[test]
    fn check_if_match_wildcard_always_matches() {
        let current = ETag::strong("abc".to_string());
        assert!(check_if_match(&headers_with("if-match", "*"), &current).is_ok());
    }

    #[test]
    fn check_if_match_matching_etag_succeeds() {
        let ts = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
        let current = etag_from_updated_at(ts);
        let tag_str = current.to_string();
        assert!(check_if_match(&headers_with("if-match", &tag_str), &current).is_ok());
    }

    #[test]
    fn check_if_match_mismatched_etag_returns_412() {
        let current = ETag::strong("current".to_string());
        let err = check_if_match(&headers_with("if-match", r#""other""#), &current).unwrap_err();
        assert_eq!(err.status, 412);
    }

    #[test]
    fn check_if_match_invalid_ascii_returns_400() {
        let current = ETag::strong("x".to_string());
        let mut h = HeaderMap::new();
        h.insert(
            "if-match",
            axum::http::HeaderValue::from_bytes(b"\xff\xfe").unwrap(),
        );
        let err = check_if_match(&h, &current).unwrap_err();
        assert_eq!(err.status, 400);
    }
}
