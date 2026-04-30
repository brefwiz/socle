//! Typed assertion helpers for [`RfcOk`]-returning handler tests.
//!
//! With `rfc-types` enabled (the default), every handler success arm returns
//! `RfcOk<T>` — a sealed type whose body is a pre-serialized
//! `ApiResponse<T>` JSON blob.  `T` is phantom-only after construction, so
//! test code must deserialize `body_json()["data"]` to recover the payload.
//!
//! This module provides ergonomic wrappers for every `api-bones` payload
//! shape so consumers don't repeat the deserialization boilerplate.
//!
//! # Payload extractors — from `RfcOk<T>` directly
//!
//! | Function | Payload shape | Use with |
//! |---|---|---|
//! | [`payload`] | `T` | `HandlerResponse<T>`, `CreatedResponse<T>`, `CreatedAtResponse<T>`, `EtaggedHandlerResponse<T>` |
//! | [`list_payload`] | `PaginatedResponse<T>` | `HandlerListResponse<T>` |
//! | [`cursor_payload`] | `CursorPaginatedResponse<T>` | handlers returning cursor pages |
//! | [`keyset_payload`] | `KeysetPaginatedResponse<T>` | handlers returning keyset pages |
//! | [`bulk_payload`] | `BulkResponse<T>` | bulk-operation handlers |
//! | [`status`] | `StatusCode` | any `RfcOk<T>` |
//! | [`headers`] | `&HeaderMap` | any `RfcOk<T>` |
//! | [`etag_header`] | `ETag` | `EtaggedHandlerResponse<T>` |
//!
//! # Result unwrappers — from `Result<RfcOk<T>, HandlerError>`
//!
//! These avoid `unwrap_err()` / `expect_err()` which require `T: Debug`
//! (not implemented by `RfcOk<T>`).
//!
//! | Function | Returns |
//! |---|---|
//! | [`unwrap_ok`] | `T` |
//! | [`unwrap_list`] | `PaginatedResponse<T>` |
//! | [`unwrap_cursor`] | `CursorPaginatedResponse<T>` |
//! | [`unwrap_keyset`] | `KeysetPaginatedResponse<T>` |
//! | [`unwrap_bulk`] | `BulkResponse<T>` |
//! | [`unwrap_status`] | `(StatusCode, T)` |
//! | [`unwrap_created`] | `(StatusCode, HeaderMap, T)` |
//! | [`unwrap_err`] | `HandlerError` |
//! | [`unwrap_err_status`] | `StatusCode` (of the error) |
//!
//! # Quick start
//!
//! ```rust,ignore
//! use socle::testing::handler_assert::{unwrap_ok, unwrap_created, unwrap_list, unwrap_err};
//!
//! // Plain payload
//! let trip: Trip = unwrap_ok(create_trip(auth, Json(req)).await);
//!
//! // Created with Location header
//! let (status, headers, trip) = unwrap_created(create_trip(auth, Json(req)).await);
//! assert_eq!(status, StatusCode::CREATED);
//! assert!(headers.contains_key("location"));
//!
//! // Paginated list
//! let page: PaginatedResponse<Trip> = unwrap_list(list_trips(auth, params).await);
//! assert_eq!(page.total_count, 3);
//!
//! // Error path — no Debug bound needed
//! let err = unwrap_err(create_trip(auth, Json(bad_req)).await);
//! assert_eq!(err.0.status_code(), 422);
//! ```

use axum::http::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;

use crate::{
    ETag,
    handler_error::{HandlerError, RfcOk},
};
use api_bones::bulk::BulkResponse;
use api_bones::pagination::{CursorPaginatedResponse, KeysetPaginatedResponse, PaginatedResponse};

// ── Low-level RfcOk accessors ─────────────────────────────────────────────────

/// Deserialize the `data` field of the `ApiResponse<T>` body.
///
/// This is the core primitive all other helpers build on.
///
/// # Panics
///
/// Panics if the body does not contain a deserializable `data` field.
#[must_use]
fn deserialize_data<T: DeserializeOwned>(ok: &RfcOk<T>) -> T {
    serde_json::from_value(ok.body_json()["data"].clone())
        .expect("RfcOk body did not contain a deserializable `data` field")
}

/// Extract the `T` payload from an `RfcOk<T>`.
///
/// Use with `HandlerResponse<T>`, `CreatedResponse<T>`, `CreatedAtResponse<T>`,
/// and `EtaggedHandlerResponse<T>`.
///
/// # Panics
///
/// Panics if the body does not contain a deserializable `data` field.
#[must_use]
pub fn payload<T: DeserializeOwned>(ok: &RfcOk<T>) -> T {
    deserialize_data(ok)
}

/// Extract the `PaginatedResponse<T>` payload from an `RfcOk<PaginatedResponse<T>>`.
///
/// Use with `HandlerListResponse<T>`.
///
/// # Panics
///
/// Panics if the body does not contain a deserializable `data` field.
#[must_use]
pub fn list_payload<T: DeserializeOwned>(ok: &RfcOk<PaginatedResponse<T>>) -> PaginatedResponse<T> {
    deserialize_data(ok)
}

/// Extract the `CursorPaginatedResponse<T>` payload from the response.
///
/// # Panics
///
/// Panics if the body does not contain a deserializable `data` field.
#[must_use]
pub fn cursor_payload<T: DeserializeOwned>(
    ok: &RfcOk<CursorPaginatedResponse<T>>,
) -> CursorPaginatedResponse<T> {
    deserialize_data(ok)
}

/// Extract the `KeysetPaginatedResponse<T>` payload from the response.
///
/// # Panics
///
/// Panics if the body does not contain a deserializable `data` field.
#[must_use]
pub fn keyset_payload<T: DeserializeOwned>(
    ok: &RfcOk<KeysetPaginatedResponse<T>>,
) -> KeysetPaginatedResponse<T> {
    deserialize_data(ok)
}

/// Extract the `BulkResponse<T>` payload from the response.
///
/// # Panics
///
/// Panics if the body does not contain a deserializable `data` field.
#[must_use]
pub fn bulk_payload<T: DeserializeOwned>(ok: &RfcOk<BulkResponse<T>>) -> BulkResponse<T> {
    deserialize_data(ok)
}

/// Extract the HTTP status code from an `RfcOk<T>`.
#[must_use]
pub fn status<T>(ok: &RfcOk<T>) -> StatusCode {
    ok.status()
}

/// Extract the response headers from an `RfcOk<T>`.
#[must_use]
pub fn headers<T>(ok: &RfcOk<T>) -> &HeaderMap {
    ok.headers()
}

/// Extract the `ETag` header value from an `EtaggedHandlerResponse<T>` success.
///
/// # Panics
///
/// Panics if the `ETag` header is absent or malformed.
#[must_use]
pub fn etag_header<T>(ok: &RfcOk<T>) -> ETag {
    let value = ok
        .headers()
        .get(axum::http::header::ETAG)
        .expect("RfcOk did not contain an ETag header")
        .to_str()
        .expect("ETag header was not valid UTF-8");
    value
        .parse::<ETag>()
        .expect("ETag header value could not be parsed as an ETag")
}

// ── Result unwrappers ─────────────────────────────────────────────────────────

/// Unwrap a `Result<RfcOk<T>, HandlerError>` and return the `T` payload.
///
/// # Panics
///
/// Panics with a clear message on `Err`.
#[must_use]
pub fn unwrap_ok<T: DeserializeOwned>(result: Result<RfcOk<T>, HandlerError>) -> T {
    payload(&result.expect("handler returned Err, expected Ok"))
}

/// Unwrap a `Result<RfcOk<PaginatedResponse<T>>, HandlerError>` and return the page.
///
/// # Panics
///
/// Panics if the result is `Err`.
#[must_use]
pub fn unwrap_list<T: DeserializeOwned>(
    result: Result<RfcOk<PaginatedResponse<T>>, HandlerError>,
) -> PaginatedResponse<T> {
    list_payload(&result.expect("handler returned Err, expected Ok"))
}

/// Unwrap a cursor-paginated handler result.
///
/// # Panics
///
/// Panics if the result is `Err`.
#[must_use]
pub fn unwrap_cursor<T: DeserializeOwned>(
    result: Result<RfcOk<CursorPaginatedResponse<T>>, HandlerError>,
) -> CursorPaginatedResponse<T> {
    cursor_payload(&result.expect("handler returned Err, expected Ok"))
}

/// Unwrap a keyset-paginated handler result.
///
/// # Panics
///
/// Panics if the result is `Err`.
#[must_use]
pub fn unwrap_keyset<T: DeserializeOwned>(
    result: Result<RfcOk<KeysetPaginatedResponse<T>>, HandlerError>,
) -> KeysetPaginatedResponse<T> {
    keyset_payload(&result.expect("handler returned Err, expected Ok"))
}

/// Unwrap a bulk-operation handler result.
///
/// # Panics
///
/// Panics if the result is `Err`.
#[must_use]
pub fn unwrap_bulk<T: DeserializeOwned>(
    result: Result<RfcOk<BulkResponse<T>>, HandlerError>,
) -> BulkResponse<T> {
    bulk_payload(&result.expect("handler returned Err, expected Ok"))
}

/// Unwrap a handler result and return `(status, payload)`.
///
/// Useful when the test cares about both the status code and the response body
/// (e.g. `200 OK` vs `201 Created` on the same payload type).
///
/// # Panics
///
/// Panics if the result is `Err`.
#[must_use]
pub fn unwrap_status<T: DeserializeOwned>(
    result: Result<RfcOk<T>, HandlerError>,
) -> (StatusCode, T) {
    let ok = result.expect("handler returned Err, expected Ok");
    let s = ok.status();
    (s, payload(&ok))
}

/// Unwrap a `CreatedAtResponse` and return `(status, headers, payload)`.
///
/// Captures the `Location` and any other headers alongside the payload so
/// tests can assert on the `Location` value without extra boilerplate.
///
/// # Panics
///
/// Panics if the result is `Err`.
#[must_use]
pub fn unwrap_created<T: DeserializeOwned>(
    result: Result<RfcOk<T>, HandlerError>,
) -> (StatusCode, HeaderMap, T) {
    let ok = result.expect("handler returned Err, expected Ok");
    let s = ok.status();
    let h = ok.headers().clone();
    (s, h, payload(&ok))
}

/// Unwrap a handler result that is expected to be an error.
///
/// Does **not** require `T: Debug` (unlike `Result::unwrap_err`).
///
/// # Panics
///
/// Panics if the result is `Ok`.
#[must_use]
pub fn unwrap_err<T>(result: Result<RfcOk<T>, HandlerError>) -> HandlerError {
    match result {
        Err(e) => e,
        Ok(_) => panic!("handler returned Ok, expected Err"),
    }
}

/// Unwrap a handler error and return its HTTP status code as a `u16`.
///
/// Equivalent to `unwrap_err(result).0.status_code()` but more readable.
///
/// # Panics
///
/// Panics if the result is `Ok`.
#[must_use]
pub fn unwrap_err_status<T>(result: Result<RfcOk<T>, HandlerError>) -> u16 {
    unwrap_err(result).0.status_code()
}
