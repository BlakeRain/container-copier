use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

use clap::Parser;
use futures_util::StreamExt;
use inotify::{Inotify, WatchDescriptor, WatchMask};
use lazy_static::lazy_static;
use serde::Deserialize;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Enable logging ('-v' for debug logging, '-vv' for tracing).
    #[arg(short = 'v', long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Print version information.
    #[arg(short = 'V', long)]
    version: bool,

    /// Path to configuration file.
    #[arg(long, default_value = "/config/container-copier.toml")]
    config: PathBuf,
}

#[derive(Deserialize)]
struct Config {
    copysets: Vec<Copyset>,
}

impl Config {
    // Load TOML config from the given path.
    fn load<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let path = path.as_ref();
        let mut file = std::fs::File::open(path)?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        toml::from_str(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }

    fn setup(&self) -> std::io::Result<Env> {
        let inotify = Inotify::init()?;
        let mut env = Env {
            notify: inotify,
            targets: HashMap::new(),
        };

        tracing::info!("Setting up inotify watches");
        for copyset in &self.copysets {
            copyset.add_to_watch(&mut env).map_err(|err| {
                tracing::error!(
                    "Failed to add copyset {:?} to inotify: {err:?}",
                    copyset.name
                );
                err
            })?;
        }

        Ok(env)
    }
}

#[derive(Deserialize)]
struct Copyset {
    name: String,
    source: PathBuf,
    target: PathBuf,
    targets: Vec<Target>,
}

lazy_static! {
    static ref WATCH_MASK: WatchMask = WatchMask::CREATE | WatchMask::DELETE | WatchMask::MODIFY;
}

impl Copyset {
    fn add_to_watch(&self, env: &mut Env) -> std::io::Result<()> {
        tracing::info!("Adding watch for copyset {:?}", self.source);
        for target in &self.targets {
            let source = self.source.join(&target.source);
            let target = self.target.join(&target.target);

            tracing::info!("  {:?} -> {:?}", source, target);
            // If the source file exists but the target does not, copy it now.
            if source.is_file() && !target.exists() {
                tracing::info!("  Copying {:?} to {:?}", source, target);
                std::fs::copy(&source, &target)?;
            }

            // Add watch for source file.
            let wd = env.notify.watches().add(&source, *WATCH_MASK)?;
            env.targets.insert(wd, ResolvedTarget::new(source, target));
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct Target {
    source: PathBuf,
    target: PathBuf,
}

struct Env {
    notify: Inotify,
    targets: HashMap<WatchDescriptor, ResolvedTarget>,
}

struct ResolvedTarget {
    source: PathBuf,
    target: PathBuf,
}

impl ResolvedTarget {
    fn new(source: PathBuf, target: PathBuf) -> Self {
        Self { source, target }
    }

    async fn copy(&self) {
        tracing::info!("Copying {:?} to {:?}", self.source, self.target);
    }
}

impl Env {
    async fn run(self) -> std::io::Result<()> {
        let Env { notify, targets } = self;

        let mut buffer = [0; 1024];
        let mut stream = notify.into_event_stream(&mut buffer)?;

        tracing::info!("Processing inotify events");
        while let Some(event_or_error) = stream.next().await {
            let event = event_or_error?;
            if let Some(target) = targets.get(&event.wd) {
                target.copy().await;
            } else {
                tracing::warn!("Unknown watch descriptor {:?}", event.wd);
            }
        }

        tracing::info!("Inotify stream ended");
        Ok(())
    }
}

fn print_version() {
    println!(
        "{} {} {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        // emitted in build.rs
        env!("CARGO_BUILD_INFO")
    );
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();

    if args.version {
        print_version();
        return Ok(());
    }

    {
        let fmt = tracing_subscriber::fmt::layer()
            .with_target(false)
            .without_time();
        let sub = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(match args.verbose {
                0 => std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
                1 => "debug".into(),
                _ => "trace".into(),
            }))
            .with(fmt);
        sub.init();
    }

    tracing::info!(config_path = ?args.config, "Loading configuration");
    let config = Config::load(&args.config)?;

    tracing::info!("Settup up inotify");
    config.setup()?.run().await?;

    Ok(())
}
