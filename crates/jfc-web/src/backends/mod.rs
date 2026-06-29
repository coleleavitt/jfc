//! Concrete backend implementations.
//!
//! Each backend wraps the existing search functions and implements the
//! `SearchBackend` trait for unified dispatch.

mod arxiv;
mod brave;
mod dblp;
mod duckduckgo;
mod fourget;
mod google;
mod millionshort;
mod openalex;
mod primo;
mod searxng;
mod semantic_scholar;
mod trace;
mod wikipedia;

pub use arxiv::ArXivBackend;
pub use brave::BraveBackend;
pub use dblp::DBLPBackend;
pub use duckduckgo::DuckDuckGoBackend;
pub use fourget::{FourGetBackend, search_fourget_structured};
pub use google::GoogleBackend;
pub use millionshort::{MillionShortBackend, search_millionshort_structured};
pub use openalex::OpenAlexBackend;
pub use primo::PrimoBackend;
pub use searxng::SearXNGBackend;
pub use semantic_scholar::SemanticScholarBackend;
pub use wikipedia::WikipediaBackend;

use crate::backend::{BackendId, SearchBackend};

/// Get all available backends (those with required API keys configured).
pub fn available_backends() -> Vec<Box<dyn SearchBackend>> {
    let all: Vec<Box<dyn SearchBackend>> = vec![
        Box::new(GoogleBackend),
        Box::new(BraveBackend),
        Box::new(MillionShortBackend),
        Box::new(SearXNGBackend),
        Box::new(DuckDuckGoBackend),
        Box::new(FourGetBackend),
        Box::new(ArXivBackend),
        Box::new(SemanticScholarBackend),
        Box::new(OpenAlexBackend),
        Box::new(PrimoBackend),
        Box::new(DBLPBackend),
        Box::new(WikipediaBackend),
    ];

    all.into_iter().filter(|b| b.is_available()).collect()
}

/// Get backends for a specific set of IDs, filtering to available only.
pub fn backends_for_ids(ids: &[BackendId]) -> Vec<Box<dyn SearchBackend>> {
    let mut backends: Vec<Box<dyn SearchBackend>> = Vec::new();

    for id in ids {
        let backend: Option<Box<dyn SearchBackend>> = match id {
            BackendId::Google => Some(Box::new(GoogleBackend)),
            BackendId::Brave => Some(Box::new(BraveBackend)),
            BackendId::MillionShort => Some(Box::new(MillionShortBackend)),
            BackendId::SearXNG => Some(Box::new(SearXNGBackend)),
            BackendId::DuckDuckGo => Some(Box::new(DuckDuckGoBackend)),
            BackendId::FourGet => Some(Box::new(FourGetBackend)),
            BackendId::ArXiv => Some(Box::new(ArXivBackend)),
            BackendId::SemanticScholar => Some(Box::new(SemanticScholarBackend)),
            BackendId::OpenAlex => Some(Box::new(OpenAlexBackend)),
            BackendId::Primo => Some(Box::new(PrimoBackend)),
            BackendId::DBLP => Some(Box::new(DBLPBackend)),
            BackendId::Wikipedia => Some(Box::new(WikipediaBackend)),
            // Not yet implemented
            BackendId::Tavily
            | BackendId::Exa
            | BackendId::Crossref
            | BackendId::PubMed
            | BackendId::DOAJ
            | BackendId::CORE
            | BackendId::Unpaywall => None,
        };

        if let Some(b) = backend {
            if b.is_available() {
                backends.push(b);
            }
        }
    }

    backends
}
