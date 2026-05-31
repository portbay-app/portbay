//! Privileged hosts helper daemon.
//!
//! Production builds install this executable as a macOS LaunchDaemon via
//! SMAppService. In development it can be run manually with `sudo` and a
//! temporary hosts file for validation.

use std::path::PathBuf;

use clap::Parser;
use portbay_lib::hosts::HostsManager;
use portbay_lib::hosts_helper::{serve, SOCKET_PATH};

#[derive(Debug, Parser)]
#[command(name = "portbay-hosts-helper")]
#[command(about = "PortBay privileged hosts-file helper")]
struct Args {
    /// Unix socket path to listen on.
    #[arg(long, default_value = SOCKET_PATH)]
    socket: PathBuf,

    /// Hosts file path. Defaults to /etc/hosts in production.
    #[arg(long, default_value = "/etc/hosts")]
    hosts_file: PathBuf,

    /// UID permitted to drive the daemon. The LaunchDaemon plist sets this to
    /// the installing user's UID; the daemon then rejects socket connections
    /// from any other (non-root) process. Omitted only for manual `sudo` dev
    /// runs, where the socket stays world-connectable.
    #[arg(long)]
    allow_uid: Option<u32>,
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let manager = HostsManager::new(args.hosts_file);
    match serve(&args.socket, manager, args.allow_uid) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("portbay-hosts-helper: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
