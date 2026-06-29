pub(crate) fn estimate_thinking_text_tokens(text: &str) -> u32 {
    (text.encode_utf16().count() / 4).min(u32::MAX as usize) as u32
}

pub(crate) fn estimate_signature_thinking_tokens(signature: &str) -> u32 {
    let base64_decoded_bytes = signature.len().saturating_mul(3).saturating_add(2) / 4;
    (base64_decoded_bytes / 4).min(u32::MAX as usize) as u32
}
