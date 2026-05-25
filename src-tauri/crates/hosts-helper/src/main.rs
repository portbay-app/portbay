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
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let manager = HostsManager::new(args.hosts_file);
    match serve(&args.socket, manager) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("portbay-hosts-helper: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
