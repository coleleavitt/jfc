use std::time::Duration;

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(60);
const HTTP_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const HTTP_TCP_KEEPALIVE: Duration = Duration::from_secs(30);

pub fn streaming_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        // Streaming responses have no known total duration. `read_timeout`
        // catches stalled reads without imposing a hard deadline on the body.
        .read_timeout(HTTP_READ_TIMEOUT)
        .pool_idle_timeout(HTTP_POOL_IDLE_TIMEOUT)
        .tcp_keepalive(HTTP_TCP_KEEPALIVE)
        .build()
        .expect("provider HTTP client configuration is valid")
}
