mod backend;
mod http;
#[cfg(test)]
mod live_tests;
mod store;
use clap::{Parser, ValueEnum};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Backend {
    Android,
    Macos,
    Windows,
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Rust anisette service with Android, macOS and Windows ADI backends"
)]
struct Args {
    #[arg(long, value_enum, default_value = "android")]
    backend: Backend,
    #[arg(long, env = "ANISETTE_LISTEN", default_value = "127.0.0.1:6969")]
    listen: SocketAddr,
    /// Override the backend's bundled binary path under /assets; never downloaded at startup.
    #[arg(long)]
    binary: Option<PathBuf>,
    #[arg(long, env = "ANISETTE_STATE_DIR", default_value = "data")]
    state_dir: PathBuf,
    /// Verify the backend binary files and exit without network or state access.
    #[arg(long)]
    check: bool,
}

impl Args {
    fn binary_path(&self) -> &Path {
        self.binary.as_deref().unwrap_or_else(|| {
            Path::new(match self.backend {
                Backend::Android => "/assets",
                Backend::Macos => "/assets/CoreADI-26.x86_64",
                Backend::Windows => "/assets/CoreADI.dll",
            })
        })
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = args.binary_path();
    let assets = Arc::new(match args.backend {
        Backend::Macos => backend::Assets::Macos(adi_macos::Binary::open(path)?),
        Backend::Android => backend::Assets::Android(adi_android::Libraries::open(path)?),
        Backend::Windows => backend::Assets::Windows(adi_windows::Binary::open(path)?),
    });
    if args.check {
        println!("{} backend binaries verified", assets.name());
        return Ok(());
    }
    let worker = backend::V1Worker::start(assets.clone(), &args.state_dir)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(args.listen).await?;
        let (stop, stopping) = tokio::sync::watch::channel(false);
        let app = http::App::new(assets, worker.sender.clone(), stopping);
        eprintln!(
            "Listening on {}; macOS/Windows V3 require routing extension; Android standard V3 defaults to routing 17106176",
            listener.local_addr()?
        );
        let server = http::serve(listener, app, stop.clone());
        tokio::pin!(server);
        tokio::select! {
            result = &mut server => result,
            _ = shutdown_signal() => {
                let _ = stop.send(true);
                server.await
            }
        }
    });
    drop(worker.sender);
    worker.join.join().map_err(|_| "V1 worker panicked")?;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::{Args, Backend, Parser, Path, SocketAddr};

    #[test]
    fn backend_selects_bundled_binary_unless_overridden() {
        let args = Args::try_parse_from(["server"]).unwrap();
        assert_eq!(args.backend, Backend::Android);
        assert_eq!(args.binary_path(), Path::new("/assets"));
        for (backend, binary) in [
            ("android", "/assets"),
            ("macos", "/assets/CoreADI-26.x86_64"),
            ("windows", "/assets/CoreADI.dll"),
        ] {
            let args = Args::try_parse_from(["server", "--backend", backend]).unwrap();
            assert_eq!(args.binary_path(), Path::new(binary));
            let args = Args::try_parse_from([
                "server",
                "--backend",
                backend,
                "--binary",
                "/custom/binary",
                "--listen",
                "127.0.0.1:7000",
                "--state-dir",
                "/custom/state",
            ])
            .unwrap();
            assert_eq!(args.binary_path(), Path::new("/custom/binary"));
            assert_eq!(args.listen, "127.0.0.1:7000".parse::<SocketAddr>().unwrap());
            assert_eq!(args.state_dir, Path::new("/custom/state"));
        }
    }

    #[test]
    fn cli_defaults_and_validation() {
        assert!(
            Args::try_parse_from(["server"])
                .unwrap()
                .listen
                .ip()
                .is_loopback()
        );
        for name in ["android", "macos", "windows"] {
            assert!(Args::try_parse_from(["server", "--backend", name]).is_ok());
        }
        assert!(Args::try_parse_from(["server", "--check"]).unwrap().check);
        assert!(!Args::try_parse_from(["server"]).unwrap().check);
        assert!(Args::try_parse_from(["server", "--backend", "fake"]).is_err());
        assert!(Args::try_parse_from(["server", "--listen", "bad-address"]).is_err());
    }
}
