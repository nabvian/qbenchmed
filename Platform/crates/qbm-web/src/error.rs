use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use qbm_app::AppError;
use serde::Serialize;

/// Stable HTTP/API failure returned by the local browser service.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    /// Safe, human-readable description, for callers outside the HTTP layer.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

impl ApiError {
    pub(crate) fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn unprocessable(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn too_large(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "source_too_large",
            message: message.into(),
        }
    }

    pub(crate) fn upstream(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: message.into(),
        }
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        let message = error.to_string();
        if message.starts_with("stale approval:") {
            return Self::conflict("stale_output", message);
        }
        if message.contains("not waiting for approval") {
            return Self::conflict("stage_already_decided", message);
        }
        if message.contains("must be approved") {
            return Self::conflict("approval_required", message);
        }
        match error {
            AppError::Domain(_) | AppError::InvalidSource(_) | AppError::InvalidArtifact(_) => {
                Self::bad_request("invalid_request", message)
            }
            AppError::Intake(_) | AppError::Audit(_) | AppError::Adapter(_) => {
                Self::unprocessable("source_cannot_be_audited", message)
            }
            AppError::Store(_) => Self::conflict("platform_state_conflict", message),
            AppError::Io(_) => Self::internal("The local service could not access managed data."),
            AppError::AdapterPackageTooLarge { .. } => Self::too_large(message),
            AppError::AdapterConformanceFailed { .. }
            | AppError::InventoryRunMismatch { .. }
            | AppError::ReservedStage(_)
            | AppError::InvalidDerivedTerminalStage(_) => {
                Self::unprocessable("invalid_workflow", message)
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status;
        let body = Json(ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                message: &self.message,
            },
        });
        (status, body).into_response()
    }
}
