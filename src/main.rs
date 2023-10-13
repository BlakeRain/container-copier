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

    async fn setup(&self) -> std::io::Result<Env> {
        tracing::info!("Creating inotify");
        let inotify = Inotify::init()?;
        let mut env = Env {
            notify: inotify,
            targets: HashMap::new(),
        };

        tracing::info!("Setting up inotify watches");
        for copyset in &self.copysets {
            copyset.add_to_watch(&mut env).await.map_err(|err| {
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
    target: Option<PathBuf>,
    targets: Vec<Target>,
}

lazy_static! {
    static ref WATCH_MASK: WatchMask = WatchMask::CREATE | WatchMask::DELETE | WatchMask::MODIFY;
}

impl Copyset {
    async fn add_to_watch(&self, env: &mut Env) -> std::io::Result<()> {
        tracing::info!("Adding watch for copyset {:?}", self.name);
        tracing::info!("  Source: {:?}", self.source);
        tracing::info!("  Target: {:?}", self.target);

        for target in &self.targets {
            tracing::info!("  {:?} -> {:?}", target.source, target.target);
            let source = self.source.join(&target.source);
            let target = self
                .target
                .as_ref()
                .unwrap_or(&self.source)
                .join(&target.target);

            let target_exists = target
                .try_exists()
                .map_err(|err| {
                    tracing::error!("  Failed to check if target exists: {err:?}");
                    err
                })
                .unwrap_or(false);

            let target = ResolvedTarget::new(source.clone(), target);
            if source.is_file() && !target_exists {
                tracing::info!("  Target does not exist; copying");
                target.copy().await.map_err(|err| {
                    tracing::error!("  Failed to copy: {err:?}");
                    err
                })?;
            }

            let wd = env
                .notify
                .watches()
                .add(&source, *WATCH_MASK)
                .map_err(|err| {
                    tracing::error!("  Failed to add watch: {err:?}");
                    err
                })?;

            env.targets.insert(wd, target);
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

    async fn copy(&self) -> std::io::Result<()> {
        tracing::info!("Copying {:?} to {:?}", self.source, self.target);

        // Make sure that the parent directory of the target exists. If it does not exist, then
        // create it.
        let parent = self.target.parent().unwrap();
        if !parent.exists() {
            tracing::info!("Creating parent directory {:?}", parent);
            std::fs::create_dir_all(parent).map_err(|err| {
                tracing::error!(parent = ?parent, "Failed to create directory: {err:?}");
                err
            })?;
        }

        // Copy the source to the target.
        std::fs::copy(&self.source, &self.target).map_err(|err| {
            tracing::error!(source = ?self.source, target = ?self.target,
                          "Failed to copy from source to target: {err:?}");
            err
        })?;

        Ok(())
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
                target.copy().await.map_err(|err| {
                    tracing::error!("Failed to copy target: {err:?}");
                    err
                })?;
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

    config.setup().await?.run().await?;

    Ok(())
}
