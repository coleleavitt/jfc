use reqwest::StatusCode;

use super::Auth;

pub(super) const fn auth_label(auth: &Auth) -> &'static str {
    match auth {
        Auth::ApiKey(_) => "api_key",
        Auth::Bearer(_) => "bearer",
    }
}

pub(super) fn trace_request_attempt(label: &'static str, attempt: u32) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [linkscope::TraceField::count(
            "attempt",
            u64::from(attempt.saturating_add(1)),
        )],
    );
}

pub(super) fn trace_request_status(label: &'static str, attempt: u32, status: StatusCode) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("attempt", u64::from(attempt.saturating_add(1))),
            linkscope::TraceField::count("status", u64::from(status.as_u16())),
        ],
    );
}
