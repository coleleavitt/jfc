use jfc_config::Config;

#[test]
fn screen_reader_mode_defaults_false() {
    let cfg = Config::default();
    assert!(
        !cfg.screen_reader_mode,
        "default screen_reader_mode should be false"
    );
}

#[test]
fn screen_reader_mode_parses_both_casings() {
    let toml_snake = r#"screen_reader_mode = true"#;
    let cfg1: Config = toml::from_str(toml_snake).expect("parse snake_case");
    assert!(cfg1.screen_reader_mode);

    let toml_camel = r#"screenReaderMode = true"#;
    let cfg2: Config = toml::from_str(toml_camel).expect("parse camelCase alias");
    assert!(cfg2.screen_reader_mode);
}
