use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use jfc_design::api::{self, DesignServerState};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args).await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("jfc-design-server: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(args: Vec<String>) -> std::io::Result<()> {
    let cwd = flag(&args, "--cwd")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let port: u16 = flag(&args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(4322);
    let host = flag(&args, "--host").unwrap_or("127.0.0.1");
    let ip: IpAddr = host.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let addr = SocketAddr::new(ip, port);
    let state =
        DesignServerState::default_in(&cwd).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("jfc-design API: http://{addr}");
    println!("  projects: http://{addr}/design/projects");
    api::serve(addr, state).await
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}
