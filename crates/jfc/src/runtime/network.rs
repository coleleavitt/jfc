use crate::app::{App, NetworkRecoveryProvider, NetworkRecoveryReason, NetworkRecoveryStatus};

fn parse_retry_status_code(message: &str) -> Option<u16> {
    message
        .split(|ch: char| !ch.is_ascii_digit())
        .find_map(|part| {
            if part.len() == 3 {
                part.parse::<u16>()
                    .ok()
                    .filter(|code| (100..=599).contains(code))
            } else {
                None
            }
        })
}

fn classify_network_recovery(status_code: Option<u16>, message: &str) -> NetworkRecoveryReason {
    let lower = message.to_ascii_lowercase();
    if matches!(status_code, Some(529)) || lower.contains("overloaded") {
        NetworkRecoveryReason::Overloaded
    } else if matches!(status_code, Some(429))
        || lower.contains("rate limit")
        || lower.contains("rate-limit")
        || lower.contains("too many requests")
    {
        NetworkRecoveryReason::RateLimited
    } else if matches!(status_code, Some(500..=599)) {
        NetworkRecoveryReason::ServerError
    } else {
        NetworkRecoveryReason::Transient
    }
}

pub(crate) fn record_network_recovery(
    app: &mut App,
    provider: NetworkRecoveryProvider,
    message: &str,
) {
    let status_code = parse_retry_status_code(message);
    let reason = classify_network_recovery(status_code, message);
    app.network_recovery_attempts = app.network_recovery_attempts.saturating_add(1);
    app.network_recovery_status = Some(NetworkRecoveryStatus {
        provider,
        reason,
        status_code,
        attempts: app.network_recovery_attempts,
        updated_at: std::time::Instant::now(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overloaded_wins_for_529_and_text() {
        assert_eq!(
            classify_network_recovery(Some(529), "status 529"),
            NetworkRecoveryReason::Overloaded
        );
        assert_eq!(
            classify_network_recovery(None, "Error: Overloaded"),
            NetworkRecoveryReason::Overloaded
        );
    }

    #[test]
    fn rate_limit_and_server_errors_are_classified() {
        assert_eq!(
            classify_network_recovery(Some(429), "too many requests"),
            NetworkRecoveryReason::RateLimited
        );
        assert_eq!(
            classify_network_recovery(Some(500), "internal server error"),
            NetworkRecoveryReason::ServerError
        );
    }
}
