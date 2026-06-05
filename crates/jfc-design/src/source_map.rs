//! Minimal Source Map v3 support for direct-edit source rewrites.
//!
//! This intentionally implements only the lookup path JFC needs: generated
//! `(line, column)` -> original `(source, line, column)`. The patch itself stays
//! in `api.rs`, where the same previous-text safety checks used by non-bundled
//! source hints are applied.

use serde::Deserialize;

use crate::project::DesignProject;
use crate::{DesignError, Result as DesignResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSourceLocation {
    pub source_path: String,
    pub line: usize,
    pub column: usize,
    pub source_content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSourceMap {
    sources: Vec<String>,
    #[serde(default)]
    sources_content: Vec<Option<String>>,
    mappings: String,
    #[serde(default)]
    source_root: Option<String>,
}

#[derive(Debug, Clone)]
struct Mapping {
    source_index: usize,
    original_line: usize,
    original_column: usize,
}

pub fn map_generated_position(
    project: &DesignProject,
    generated_path: &str,
    source_map_path: Option<&str>,
    generated_line: usize,
    generated_column: usize,
) -> DesignResult<Option<MappedSourceLocation>> {
    let Some((raw_map, map_path)) = load_source_map(project, generated_path, source_map_path)?
    else {
        return Ok(None);
    };
    let parsed: RawSourceMap = serde_json::from_str(&raw_map)?;
    let Some(mapping) = find_mapping(&parsed.mappings, generated_line, generated_column)? else {
        return Ok(None);
    };
    let Some(source) = parsed.sources.get(mapping.source_index) else {
        return Ok(None);
    };
    let source_content = parsed
        .sources_content
        .get(mapping.source_index)
        .and_then(Clone::clone);
    let Some(source_path) =
        resolve_source_path(project, &map_path, parsed.source_root.as_deref(), source)
    else {
        return Ok(None);
    };
    Ok(Some(MappedSourceLocation {
        source_path,
        line: mapping.original_line.saturating_add(1),
        column: mapping.original_column,
        source_content,
    }))
}

fn load_source_map(
    project: &DesignProject,
    generated_path: &str,
    source_map_path: Option<&str>,
) -> DesignResult<Option<(String, String)>> {
    if let Some(path) = source_map_path.filter(|p| !p.trim().is_empty()) {
        return match project.read_to_string(path) {
            Ok(raw) => Ok(Some((raw, path.to_owned()))),
            Err(DesignError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        };
    }

    let generated = match project.read_to_string(generated_path) {
        Ok(raw) => raw,
        Err(DesignError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    if let Some(raw) = inline_source_map(&generated) {
        return Ok(Some((raw, generated_path.to_owned())));
    }
    if let Some(path) = source_mapping_url_path(generated_path, &generated) {
        return match project.read_to_string(&path) {
            Ok(raw) => Ok(Some((raw, path))),
            Err(DesignError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        };
    }
    let adjacent = format!("{generated_path}.map");
    match project.read_to_string(&adjacent) {
        Ok(raw) => Ok(Some((raw, adjacent))),
        Err(DesignError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

fn inline_source_map(generated: &str) -> Option<String> {
    let url = source_mapping_url(generated)?;
    let prefix = "data:application/json";
    if !url.starts_with(prefix) {
        return None;
    }
    let comma = url.find(',')?;
    let (meta, data) = url.split_at(comma);
    let data = &data[1..];
    if meta.contains(";base64") {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .ok()?;
        String::from_utf8(bytes).ok()
    } else {
        Some(percent_decode(data))
    }
}

fn source_mapping_url_path(generated_path: &str, generated: &str) -> Option<String> {
    let url = source_mapping_url(generated)?;
    if url.starts_with("data:") || url.starts_with("http://") || url.starts_with("https://") {
        return None;
    }
    let clean = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .replace('\\', "/");
    if clean.starts_with('/') {
        return Some(clean.trim_start_matches('/').to_owned());
    }
    let dir = generated_path
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("");
    if dir.is_empty() {
        Some(clean)
    } else {
        Some(format!("{dir}/{clean}"))
    }
}

fn source_mapping_url(generated: &str) -> Option<&str> {
    generated.lines().rev().take(12).find_map(|line| {
        line.split_once("sourceMappingURL=")
            .map(|(_, url)| url.trim())
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn find_mapping(
    mappings: &str,
    generated_line_one_based: usize,
    generated_column: usize,
) -> DesignResult<Option<Mapping>> {
    if generated_line_one_based == 0 {
        return Ok(None);
    }
    let target_line = generated_line_one_based - 1;
    let mut source_index = 0i64;
    let mut original_line = 0i64;
    let mut original_column = 0i64;
    let mut best = None;

    for (line_index, line) in mappings.split(';').enumerate() {
        let mut generated_column_acc = 0i64;
        for segment in line.split(',').filter(|s| !s.is_empty()) {
            let values = decode_segment(segment)?;
            if values.is_empty() {
                continue;
            }
            generated_column_acc += values[0];
            if values.len() >= 4 {
                source_index += values[1];
                original_line += values[2];
                original_column += values[3];
                if line_index == target_line {
                    let gen_col = usize::try_from(generated_column_acc).unwrap_or(usize::MAX);
                    if gen_col <= generated_column {
                        if source_index >= 0 && original_line >= 0 && original_column >= 0 {
                            best = Some(Mapping {
                                source_index: source_index as usize,
                                original_line: original_line as usize,
                                original_column: original_column as usize,
                            });
                        }
                    } else {
                        break;
                    }
                }
            }
        }
        if line_index > target_line {
            break;
        }
    }
    Ok(best)
}

fn decode_segment(segment: &str) -> DesignResult<Vec<i64>> {
    let mut out = Vec::new();
    let mut value = 0i64;
    let mut shift = 0u32;
    for byte in segment.bytes() {
        let Some(mut digit) = base64_value(byte) else {
            return Err(DesignError::Bundle(format!(
                "invalid source-map base64 digit: {}",
                byte as char
            )));
        };
        let continuation = (digit & 32) != 0;
        digit &= 31;
        value += i64::from(digit) << shift;
        if continuation {
            shift += 5;
            continue;
        }
        let negative = (value & 1) == 1;
        value >>= 1;
        out.push(if negative { -value } else { value });
        value = 0;
        shift = 0;
    }
    if shift != 0 {
        return Err(DesignError::Bundle(
            "unterminated source-map VLQ segment".to_owned(),
        ));
    }
    Ok(out)
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

fn resolve_source_path(
    project: &DesignProject,
    map_path: &str,
    source_root: Option<&str>,
    source: &str,
) -> Option<String> {
    let mut candidates = Vec::new();
    let combined = match source_root.filter(|s| !s.is_empty()) {
        Some(root) => format!("{}/{}", root.trim_end_matches('/'), source),
        None => source.to_owned(),
    };
    push_source_candidates(&mut candidates, &combined);
    push_source_candidates(&mut candidates, source);
    if let Some((dir, _)) = map_path.rsplit_once('/') {
        push_source_candidates(&mut candidates, &format!("{dir}/{source}"));
    }

    let files = project.list_files();
    for candidate in candidates {
        if files.iter().any(|file| file == &candidate) {
            return Some(candidate);
        }
        if let Some(found) = unique_file_suffix(&files, &candidate) {
            return Some(found);
        }
    }
    None
}

fn push_source_candidates(out: &mut Vec<String>, raw: &str) {
    let clean = normalize_source(raw);
    if clean.is_empty() {
        return;
    }
    push_unique(out, clean.clone());
    if let Some(pos) = clean.find("/src/") {
        push_unique(out, clean[pos + 1..].to_owned());
    }
    if let Some(pos) = clean.find("src/") {
        push_unique(out, clean[pos..].to_owned());
    }
    if let Some(pos) = clean.find("/app/") {
        push_unique(out, clean[pos + 1..].to_owned());
    }
    if let Some(name) = clean.rsplit('/').next() {
        push_unique(out, name.to_owned());
    }
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if !out.iter().any(|existing| existing == &value) {
        out.push(value);
    }
}

fn normalize_source(raw: &str) -> String {
    let mut value = raw
        .split(['?', '#'])
        .next()
        .unwrap_or(raw)
        .replace('\\', "/");
    if let Some(rest) = value.strip_prefix("webpack://") {
        value = rest.to_owned();
    }
    if let Some(rest) = value.strip_prefix("vite://") {
        value = rest.to_owned();
    }
    if let Some(rest) = value.strip_prefix("file://") {
        value = rest.to_owned();
    }
    if let Some(pos) = value.find("/./") {
        value = value[pos + 3..].to_owned();
    }
    while let Some(rest) = value.strip_prefix("./") {
        value = rest.to_owned();
    }
    while let Some(rest) = value.strip_prefix("../") {
        value = rest.to_owned();
    }
    value.trim_start_matches('/').to_owned()
}

fn unique_file_suffix(files: &[String], candidate: &str) -> Option<String> {
    let suffix = candidate.trim_start_matches('/');
    let matches = files
        .iter()
        .filter(|file| file.ends_with(suffix))
        .cloned()
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlq_segment_decodes_normal() {
        assert_eq!(decode_segment("AAAA").unwrap(), vec![0, 0, 0, 0]);
        assert_eq!(decode_segment("KAAA").unwrap(), vec![5, 0, 0, 0]);
    }

    #[test]
    fn source_map_finds_previous_segment_normal() {
        let map = RawSourceMap {
            sources: vec!["src/App.jsx".to_owned()],
            sources_content: vec![],
            mappings: "KAAA".to_owned(),
            source_root: None,
        };
        let mapping = find_mapping(&map.mappings, 1, 8).unwrap().unwrap();
        assert_eq!(mapping.source_index, 0);
        assert_eq!(mapping.original_line, 0);
        assert_eq!(mapping.original_column, 0);
    }
}
