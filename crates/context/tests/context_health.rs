use jfc_context::{
    ContextContributor, ContextDoctorReport, ContextHealth, ContextHealthEventKind,
    ContextHealthService, ContextHealthStatus, ContextHealthUpdate, ContextLayout, ContributorId,
    InMemoryContextHealthService,
};
use serde_json::json;

#[test]
fn context_health_serializes_round_trip_persistent_dto_normal() {
    let layout = ContextLayout::destination_skeleton();
    let contributor = ContextContributor::new(
        ContributorId::new("builtin.embeddings").expect("valid contributor id"),
        "embedding cache",
    );
    let mut health = ContextHealth::new(layout, ContextHealthStatus::Healthy, vec![contributor])
        .expect("complete health dto");

    health
        .apply_update(ContextHealthUpdate::embedding_failure("indexer timed out"))
        .expect("valid embedding failure cause");
    health
        .apply_update(ContextHealthUpdate::cache_bust("layout hash changed"))
        .expect("valid cache bust cause");

    let encoded = serde_json::to_value(&health).expect("serializable health dto");
    assert_eq!(encoded["status"], json!("degraded"));
    assert_eq!(encoded["revision"], json!(2));
    assert_eq!(encoded["events"][0]["kind"], json!("embedding_failure"));
    assert_eq!(encoded["events"][0]["cause"], json!("indexer timed out"));
    assert_eq!(encoded["events"][1]["kind"], json!("cache_bust"));
    assert_eq!(encoded["events"][1]["cause"], json!("layout hash changed"));

    let decoded: ContextHealth =
        serde_json::from_value(encoded).expect("deserializable health dto");
    assert_eq!(decoded.status(), ContextHealthStatus::Degraded);
    assert_eq!(decoded.revision(), 2);
    assert_eq!(decoded.events().len(), 2);
    assert_eq!(decoded.events()[0].cause(), "indexer timed out");
    assert_eq!(
        decoded.events()[1].kind(),
        ContextHealthEventKind::CacheBust
    );
}

#[test]
fn context_health_service_applies_updates_observable_normal() {
    let layout = ContextLayout::destination_skeleton();
    let health = ContextHealth::new(layout, ContextHealthStatus::Healthy, Vec::new())
        .expect("complete health dto");
    let mut service = InMemoryContextHealthService::new(health);

    let updated = service
        .update_health(ContextHealthUpdate::cache_bust(
            "manual compact invalidated cache",
        ))
        .expect("valid cache bust update");

    assert_eq!(updated.status(), ContextHealthStatus::Degraded);
    assert_eq!(updated.revision(), 1);
    assert_eq!(updated.events().len(), 1);
    assert_eq!(
        updated.events()[0].kind(),
        ContextHealthEventKind::CacheBust
    );
    assert_eq!(service.current_health().revision(), 1);
}

#[test]
fn context_health_rejects_empty_update_cause_malformed() {
    let layout = ContextLayout::destination_skeleton();
    let mut health = ContextHealth::new(layout, ContextHealthStatus::Healthy, Vec::new())
        .expect("complete health dto");

    let error = health
        .apply_update(ContextHealthUpdate::embedding_failure("   "))
        .expect_err("empty causes are not observable health evidence");

    assert_eq!(
        error.to_string(),
        "context health update cause cannot be empty"
    );
}

#[test]
fn context_doctor_report_serializes_context_health_visibility_normal() {
    let layout = ContextLayout::destination_skeleton();
    let contributor = ContextContributor::new(
        ContributorId::new("builtin.memory").expect("valid contributor id"),
        "memory recall",
    );
    let mut health = ContextHealth::new(layout, ContextHealthStatus::Healthy, vec![contributor])
        .expect("complete health dto");
    health
        .apply_update(ContextHealthUpdate::cache_bust(
            "manual compact invalidated cache",
        ))
        .expect("valid cache bust cause");

    let report = ContextDoctorReport::from_health(&health);
    let encoded = serde_json::to_value(&report).expect("serializable doctor report");

    assert_eq!(encoded["context_health"]["status"], json!("degraded"));
    assert_eq!(encoded["context_health"]["revision"], json!(1));
    assert_eq!(encoded["context_health"]["contributors"], json!(1));
    assert_eq!(encoded["context_health"]["events"], json!(1));
    assert_eq!(
        encoded["context_health"]["latest_event"]["kind"],
        json!("cache_bust")
    );
    assert_eq!(
        encoded["context_health"]["latest_event"]["cause"],
        json!("manual compact invalidated cache")
    );
}
