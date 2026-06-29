use super::*;

#[test]
fn push_instances_dedups_and_strips_trailing_slash_normal() {
    let mut seen = HashSet::new();
    let mut instances = Vec::new();

    let added = push_instances(
        vec![
            "https://4get.example/".into(),
            "https://4get.example".into(),
            "http://ignored.example".into(),
        ],
        &mut seen,
        &mut instances,
        10,
    );

    assert_eq!(instances, vec!["https://4get.example"]);
    assert_eq!(added, vec!["https://4get.example"]);
}

#[test]
fn push_instances_rejects_non_public_roots_regression() {
    let mut seen = HashSet::new();
    let mut instances = Vec::new();

    let added = push_instances(
        vec![
            "http://4get.example".into(),
            "https://127.0.0.1".into(),
            "https://10.0.0.4".into(),
            "https://metadata.google.internal".into(),
            "https://4get.example/path".into(),
            "https://4get.example?q=x".into(),
            "https://user:pass@4get.example".into(),
            "https://4get.example:8443/".into(),
        ],
        &mut seen,
        &mut instances,
        10,
    );

    assert_eq!(instances, vec!["https://4get.example:8443"]);
    assert_eq!(added, vec!["https://4get.example:8443"]);
}

#[test]
fn next_page_params_keep_original_scraper_normal() {
    let params = page_params("rust", "yandex", Some("yandex_w1.key"));

    assert_eq!(
        params,
        vec![("npt", "yandex_w1.key"), ("scraper", "yandex")]
    );
}
