use rmcp::ServiceError;

#[derive(Debug)]
pub enum RequestError {
    Disconnected,
    Timeout,
    AuthHeaderRejected,
    Transport { code: &'static str, message: String },
    BadArguments,
    Service(String),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("MCP server disconnected"),
            Self::Timeout => f.write_str("MCP request timed out"),
            Self::AuthHeaderRejected => f.write_str(
                "MCP server rejected the configured Authorization header; \
                 check the token for this endpoint",
            ),
            Self::Transport { code, message } => {
                write!(f, "MCP transport error ({code}): {message}")
            }
            Self::BadArguments => f.write_str("MCP tool arguments must be a JSON object"),
            Self::Service(message) => write!(f, "MCP service error: {message}"),
        }
    }
}

impl std::error::Error for RequestError {}

impl From<ServiceError> for RequestError {
    fn from(error: ServiceError) -> Self {
        map_service_error(error, false)
    }
}

pub(super) fn map_service_error(error: ServiceError, has_auth_header: bool) -> RequestError {
    match error {
        ServiceError::TransportClosed | ServiceError::Cancelled { .. } => {
            RequestError::Disconnected
        }
        ServiceError::Timeout { .. } => RequestError::Timeout,
        ServiceError::TransportSend(error) => {
            classify_transport_error(error.error.as_ref(), has_auth_header)
        }
        other => RequestError::Service(other.to_string()),
    }
}

fn classify_transport_error(
    error: &(dyn std::error::Error + Send + Sync + 'static),
    has_auth_header: bool,
) -> RequestError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    if has_auth_header
        && (lower.contains("401")
            || lower.contains("403")
            || lower.contains("unauthorized")
            || lower.contains("forbidden"))
    {
        return RequestError::AuthHeaderRejected;
    }
    let code = if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("connection refused") || lower.contains("econnrefused") {
        "connection_refused"
    } else if lower.contains("connection reset") || lower.contains("econnreset") {
        "connection_reset"
    } else if lower.contains("dns") || lower.contains("enotfound") || lower.contains("eai_again") {
        "dns"
    } else if lower.contains("closed") || lower.contains("terminated") {
        "connection_closed"
    } else {
        "transport"
    };
    RequestError::Transport { code, message }
}

pub(super) fn service_error_suggests_auth_rejection(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("401")
        || message.contains("403")
        || message.contains("unauthorized")
        || message.contains("forbidden")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[derive(Debug)]
    struct StringError(&'static str);

    impl std::fmt::Display for StringError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for StringError {}

    #[test]
    fn request_error_maps_transport_closed_to_disconnected() {
        let mapped = RequestError::from(ServiceError::TransportClosed);
        assert!(matches!(mapped, RequestError::Disconnected));
    }

    #[test]
    fn request_error_maps_timeout() {
        let mapped = RequestError::from(ServiceError::Timeout {
            timeout: Duration::from_secs(1),
        });
        assert!(matches!(mapped, RequestError::Timeout));
    }

    #[test]
    fn auth_rejection_requires_configured_auth_header_robust() {
        let err = StringError("server returned 401 unauthorized");
        assert!(matches!(
            classify_transport_error(&err, true),
            RequestError::AuthHeaderRejected
        ));
        assert!(matches!(
            classify_transport_error(&err, false),
            RequestError::Transport {
                code: "transport",
                ..
            }
        ));
    }
}
