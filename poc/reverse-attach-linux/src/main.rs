// reverse-attach: makes a Linux node a lendable "hands" node for openab-pty
// reverse-attach. Single self-contained binary: a minimal HTTP/1.1 control
// server (POST/GET /attach, GET/DELETE /attach/{id}) plus a WebSocket dialer that
// connects outbound to a runtime and serves an MCP tool surface.
//
// Synchronous, std threads only. Deps: serde_json + tungstenite (which
// re-exports `http`). Target: aarch64 Debian 13, Rust 1.98.

mod attach;
mod auth;
mod cli;
mod http;
mod mcp;
mod platform;
mod switchboard;
mod tools;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        Ok(cli::Invocation::Run(flags)) => flags.apply(),
        Ok(cli::Invocation::Version) => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            return;
        }
        // Same exit code as the macOS daemon's usage().
        Ok(cli::Invocation::Help) => {
            println!("{}", cli::USAGE);
            std::process::exit(64);
        }
        Err(e) => {
            eprintln!("{e}\n\n{}", cli::USAGE);
            std::process::exit(64);
        }
    }
    platform::warm_up();
    #[cfg(windows)]
    if cli::options().menu_bar {
        platform::windows_start_tray();
    }
    if let Some((url, secret_file, profile)) = cli::options().switchboard.clone() {
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        std::thread::spawn(move || switchboard::run(url, secret_file, profile, cancelled));
    }
    http::serve();
}
