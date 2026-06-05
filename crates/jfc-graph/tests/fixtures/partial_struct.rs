// Expected: Struct BigConfig with 8 fields
// Function uses_two_fields only accesses `name` and `port`
// Used to test partial struct selection

pub struct BigConfig {
    pub name: String,
    pub port: u16,
    pub host: String,
    pub debug: bool,
    pub max_retries: u32,
    pub timeout_ms: u64,
    pub log_level: String,
    pub workers: usize,
}

pub fn uses_two_fields(config: &BigConfig) -> String {
    format!("{}:{}", config.name, config.port)
}

pub fn uses_all_fields(config: &BigConfig) -> String {
    format!(
        "{}:{}@{} debug={} retries={} timeout={}ms log={} workers={}",
        config.name,
        config.port,
        config.host,
        config.debug,
        config.max_retries,
        config.timeout_ms,
        config.log_level,
        config.workers,
    )
}
