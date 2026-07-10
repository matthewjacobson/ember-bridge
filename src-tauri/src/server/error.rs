//! Structured JSON errors for the localhost API.
//!
//! Every failure leaves the API as
//! `{"error": {"code": "<stable_snake_case>", "message": "<human text>"}}`
//! with an appropriate HTTP status, so Ember can branch on `code` without
//! parsing prose.

use crate::machine::MachineError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "missing or invalid API token".into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: message.into(),
        }
    }
}

impl From<MachineError> for ApiError {
    fn from(e: MachineError) -> Self {
        let status = match &e {
            // The bridge is fine; the machine could not be reached or
            // answered nonsense — a gateway-style failure.
            MachineError::Unreachable(_)
            | MachineError::Protocol(_)
            | MachineError::Rejected { .. }
            | MachineError::UploadFailed(_) => StatusCode::BAD_GATEWAY,
            MachineError::Timeout => StatusCode::GATEWAY_TIMEOUT,
            // The request itself is unacceptable for this machine.
            MachineError::FileTooLarge { .. }
            | MachineError::InsufficientStorage { .. }
            | MachineError::UnsupportedFormat { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Self {
            status,
            code: e.code(),
            message: e.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": { "code": self.code, "message": self.message }
            })),
        )
            .into_response()
    }
}
