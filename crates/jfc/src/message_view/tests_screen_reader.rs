use super::*;

#[test]
#[serial_test::serial]
fn reasoning_header_uses_text_label_in_screen_reader_mode() {
    // Force screen-reader mode via env so we don't need a real config file.
    unsafe { std::env::set_var("JFC_SCREEN_READER", "1") };

    let t = Theme::dark();
    let mut items: Vec<super::core::RenderItem<'_>> = Vec::new();
    // Expanded=true path exercises the header rendering directly.
    super::assistant_parts::push_reasoning_lines(
        &mut items,
        "internal thoughts",
        true,
        true,
        None,
        &t,
    );

    // Find the first TextLine and flatten to string.
    fn line_text(l: &ratatui::text::Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }
    let header = items
        .iter()
        .filter_map(|it| match it {
            super::core::RenderItem::TextLine(l) => Some(line_text(l)),
            _ => None,
        })
        .next()
        .unwrap_or_default();

    assert!(header.contains("Thinking"));
    assert!(
        !header.contains("∴"),
        "should avoid decorative reasoning glyph in screen-reader mode"
    );

    unsafe { std::env::remove_var("JFC_SCREEN_READER") };
}
