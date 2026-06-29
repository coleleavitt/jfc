const DEFAULT_INSTANCES: &[&str] = &[
    "https://4get.dcs0.hu",
    "https://4get.edmateo.site",
    "https://4get.ca",
    "https://search.fischbytes.de",
    "https://4get.sudovanilla.org",
];
const DEFAULT_SCRAPERS: &[&str] = &[
    "yandex",
    "startpage",
    "qwant",
    "google_cse",
    "yep",
    "mwmbl",
    "mojeek",
    "marginalia",
    "wiby",
    "solofield",
];
const WEB_SCRAPERS: &[&str] = &[
    "ddg",
    "brave",
    "yandex",
    "google",
    "google_api",
    "google_cse",
    "yahoo_japan",
    "startpage",
    "qwant",
    "yep",
    "mwmbl",
    "mojeek",
    "naver",
    "baidu",
    "coccoc",
    "solofield",
    "marginalia",
    "wiby",
];

pub fn configured_instances() -> Vec<String> {
    configured_list("FOURGET_INSTANCES", "FOURGET_INSTANCE", DEFAULT_INSTANCES)
}

pub fn configured_scrapers() -> Vec<String> {
    let configured = configured_list("FOURGET_SCRAPERS", "FOURGET_SCRAPER", DEFAULT_SCRAPERS);
    configured
        .into_iter()
        .filter(|scraper| WEB_SCRAPERS.contains(&scraper.as_str()))
        .collect()
}

pub fn instance_limit() -> usize {
    std::env::var("FOURGET_INSTANCE_LIMIT")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(32)
        .clamp(1, 256)
}

pub fn discovery_enabled() -> bool {
    std::env::var("FOURGET_DISCOVER_INSTANCES")
        .map(|raw| matches!(raw.trim(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

pub fn page_fanout_limit() -> usize {
    std::env::var("FOURGET_PAGE_FANOUT_LIMIT")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(8)
        .clamp(1, 64)
}

fn configured_list(multi_env: &str, single_env: &str, default: &[&str]) -> Vec<String> {
    std::env::var(multi_env)
        .ok()
        .or_else(|| std::env::var(single_env).ok())
        .map(|raw| split_csv(&raw))
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| default.iter().map(|v| (*v).to_owned()).collect())
}

fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub struct FourGetRequest<'a> {
    pub query: &'a str,
    pub scraper: Option<&'a str>,
}

impl<'a> FourGetRequest<'a> {
    pub fn parse(query: &'a str) -> Self {
        let Some((candidate, rest)) = query.split_once(':') else {
            return Self {
                query: query.trim(),
                scraper: None,
            };
        };
        let scraper = candidate.trim();
        if WEB_SCRAPERS.contains(&scraper) {
            Self {
                query: rest.trim(),
                scraper: Some(scraper),
            }
        } else {
            Self {
                query: query.trim(),
                scraper: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_parse_accepts_known_scraper_normal() {
        let request = FourGetRequest::parse("yandex: rust async traits");

        assert_eq!(request.scraper, Some("yandex"));
        assert_eq!(request.query, "rust async traits");
    }

    #[test]
    fn request_parse_keeps_unknown_prefix_as_query_robust() {
        let request = FourGetRequest::parse("unknown: rust async traits");

        assert_eq!(request.scraper, None);
        assert_eq!(request.query, "unknown: rust async traits");
    }

    #[test]
    fn default_scrapers_exclude_brave_and_ddg_normal() {
        assert!(!DEFAULT_SCRAPERS.contains(&"brave"));
        assert!(!DEFAULT_SCRAPERS.contains(&"ddg"));
        assert!(DEFAULT_SCRAPERS.contains(&"yandex"));
        assert!(DEFAULT_SCRAPERS.contains(&"google_cse"));
        assert!(DEFAULT_SCRAPERS.contains(&"yep"));
    }
}
