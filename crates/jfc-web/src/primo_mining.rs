//! Mining and analysis of Primo/HOLLIS institutional profiles.
//!
//! This module extracts institutional metadata from ExLibris Primo and HOLLIS
//! discovery system JavaScript bundles, including VIDs, search scopes, endpoints,
//! and query field patterns.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An institutional profile extracted from Primo/HOLLIS JS analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstitutionProfile {
    pub domain: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub institution_code: Option<String>,
    #[serde(default)]
    pub endpoints: HashMap<String, u32>,
    #[serde(default)]
    pub query_fields: Vec<String>,
    #[serde(default)]
    pub js_files: Vec<String>,
}

/// Extracted search endpoint patterns from Primo bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEndpoints {
    pub discovery: Vec<String>,
    pub api: Vec<String>,
    pub auth: Vec<String>,
}

/// Configuration extracted from minified Primo JavaScript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimoConfiguration {
    pub vids: Vec<String>,
    pub institutions: Vec<String>,
    pub search_endpoints: SearchEndpoints,
    pub window_vars: Vec<String>,
}

impl Default for PrimoConfiguration {
    fn default() -> Self {
        Self {
            vids: Vec::new(),
            institutions: Vec::new(),
            search_endpoints: SearchEndpoints {
                discovery: Vec::new(),
                api: Vec::new(),
                auth: Vec::new(),
            },
            window_vars: Vec::new(),
        }
    }
}

/// Extract VID patterns from JavaScript code.
/// VIDs match patterns like `01CMU_INST:01CMU` or `01HVD_INST:HVD2`.
pub fn extract_vids(code: &str) -> Vec<String> {
    let mut vids = std::collections::HashSet::new();
    
    // Look for quoted strings containing _INST pattern
    for chunk in code.split('"') {
        if chunk.contains("_INST") && chunk.len() < 50 {
            vids.insert(chunk.to_string());
        }
    }
    
    for chunk in code.split('\'') {
        if chunk.contains("_INST") && chunk.len() < 50 {
            vids.insert(chunk.to_string());
        }
    }
    
    let mut result: Vec<_> = vids.into_iter().collect();
    result.sort();
    result
}

/// Extract search scope configurations.
pub fn extract_search_scopes(code: &str) -> Vec<String> {
    let mut scopes = std::collections::HashSet::new();
    
    // Look for search_scope= or search_scope: patterns
    for line in code.lines() {
        if line.contains("search_scope") {
            if let Some(start) = line.find('"') {
                if let Some(end) = line[start + 1..].find('"') {
                    let scope = &line[start + 1..start + 1 + end];
                    if scope.len() < 100 {
                        scopes.insert(scope.to_string());
                    }
                }
            }
        }
    }
    
    let mut result: Vec<_> = scopes.into_iter().collect();
    result.sort();
    result
}

/// Extract Primo/HOLLIS discovery endpoints.
pub fn extract_endpoints(code: &str) -> SearchEndpoints {
    let mut discovery = std::collections::HashSet::new();
    let mut api = std::collections::HashSet::new();
    let mut auth = std::collections::HashSet::new();
    
    // Look for quoted strings with discovery paths
    for chunk in code.split('"') {
        if chunk.contains("/discovery") && chunk.len() < 100 {
            discovery.insert(chunk.to_string());
        }
        if chunk.starts_with("https://") && chunk.contains("/api") && chunk.len() < 200 {
            api.insert(chunk.to_string());
        }
        if (chunk.contains("/auth") || chunk.contains("/login") || chunk.contains("/sso")) && chunk.starts_with('/') {
            auth.insert(chunk.to_string());
        }
    }
    
    for chunk in code.split('\'') {
        if chunk.contains("/discovery") && chunk.len() < 100 {
            discovery.insert(chunk.to_string());
        }
        if chunk.starts_with("https://") && chunk.contains("/api") && chunk.len() < 200 {
            api.insert(chunk.to_string());
        }
        if (chunk.contains("/auth") || chunk.contains("/login") || chunk.contains("/sso")) && chunk.starts_with('/') {
            auth.insert(chunk.to_string());
        }
    }
    
    SearchEndpoints {
        discovery: discovery.into_iter().collect(),
        api: api.into_iter().collect(),
        auth: auth.into_iter().collect(),
    }
}

/// Extract query field types supported by Primo.
pub fn extract_query_fields(code: &str) -> Vec<String> {
    let mut fields = std::collections::HashSet::new();
    
    let common = ["any", "title", "author", "subject", "isbn", "issn", "publisher", "keyword"];
    for field in &common {
        if code.to_lowercase().contains(field) {
            fields.insert(field.to_string());
        }
    }
    
    let mut result: Vec<_> = fields.into_iter().collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_vids() {
        let code = r#"vid="01CMU_INST:01CMU" other=test"#;
        let vids = extract_vids(code);
        assert!(vids.iter().any(|v| v.contains("01CMU_INST")));
    }

    #[test]
    fn test_extract_search_scopes() {
        let code = r#"search_scope="MyInst_and_CI""#;
        let scopes = extract_search_scopes(code);
        assert!(scopes.iter().any(|s| s.contains("MyInst")));
    }

    #[test]
    fn test_extract_query_fields() {
        let code = r#""title" "author" "subject" configuration"#;
        let fields = extract_query_fields(code);
        assert!(fields.contains(&"title".to_string()));
        assert!(fields.contains(&"author".to_string()));
    }
}
