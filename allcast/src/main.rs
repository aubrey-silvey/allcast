mod config;
#[cfg(feature = "gui")]
mod gui;
mod monitor;
mod platform;
mod receiver;
mod sender;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Parser, Debug)]
#[command(version, about = "Cross-platform screen sharing over RTP/UDP. Run without args to open the config window or start in the configured role.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Open the configuration window even if a config already exists.
    Config,
    /// Run as sender using the saved config (or fail if not configured).
    Send,
    /// Run as receiver using the saved config (or fail if not configured).
    Recv,
    /// Print where the active config file is.
    Where,
    /// Print the monitors the GUI would offer (debug).
    Monitors,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    ctrlc::set_handler(move || stop_clone.store(true, Ordering::Relaxed)).ok();

    let cli = Cli::parse();
    match cli.command {
        None => {
            // No subcommand: launch GUI on first run, otherwise dispatch.
            if !config::exists() {
                run_first_run(stop)
            } else {
                dispatch_saved_role(stop)
            }
        }
        Some(Command::Config) => {
            #[cfg(feature = "gui")]
            {
                let existing = config::load().ok();
                gui::run(existing);
                Ok(())
            }
            #[cfg(not(feature = "gui"))]
            {
                print_headless_config_notice()
            }
        }
        Some(Command::Send) => sender::run(&config::load()?, stop),
        Some(Command::Recv) => receiver::run(&config::load()?, stop),
        Some(Command::Where) => {
            println!("{}", config::config_path()?.display());
            Ok(())
        }
        Some(Command::Monitors) => {
            let s = monitor::enumerate_for_sender()?;
            let r = monitor::enumerate_for_receiver()?;
            println!("sender enum: source={:?} user_selectable={} names={:?}", s.source, s.user_selectable, s.names);
            println!("receiver enum: source={:?} user_selectable={} names={:?}", r.source, r.user_selectable, r.names);
            Ok(())
        }
    }
}

#[cfg(feature = "gui")]
fn run_first_run(stop: Arc<AtomicBool>) -> Result<()> {
    match gui::run(None) {
        Some(cfg) => match cfg.role {
            config::Role::Sender => sender::run(&cfg, stop),
            config::Role::Receiver => receiver::run(&cfg, stop),
        },
        None => Ok(()),
    }
}

/// Headless builds (compiled without the `gui` feature) have no first-run
/// window, so there is nothing to launch — guide the user to a config file.
#[cfg(not(feature = "gui"))]
fn run_first_run(_stop: Arc<AtomicBool>) -> Result<()> {
    print_headless_config_notice()
}

/// Tell the user how to configure a headless build by hand, printing a
/// ready-to-edit default config. Handy for deploying to a Pi receiver.
#[cfg(not(feature = "gui"))]
fn print_headless_config_notice() -> Result<()> {
    let path = config::config_path()?;
    eprintln!("This build was compiled without the GUI (no `gui` feature), so");
    eprintln!("there is no configuration window. Edit the config directly at:");
    eprintln!("  {}", path.display());
    if !config::exists() {
        eprintln!();
        eprintln!("No config exists yet — here is a default to start from:");
        eprintln!();
        println!("{}", toml::to_string_pretty(&config::Config::default())?);
    }
    eprintln!();
    eprintln!("Then run `allcast send` or `allcast recv`.");
    Ok(())
}

fn dispatch_saved_role(stop: Arc<AtomicBool>) -> Result<()> {
    let cfg = config::load()?;
    match cfg.role {
        config::Role::Sender => sender::run(&cfg, stop),
        config::Role::Receiver => receiver::run(&cfg, stop),
    }
}
