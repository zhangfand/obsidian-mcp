use std::process::ExitCode;

use tools::Tools;
use vault::{TmpPlacement, Vault};

const USAGE: &str = "usage: obsidian-mcp --vault <dir> --listen <addr:port> [--allowed-host <host>]... [--tmp-dir <dir>]";

struct Args {
    vault: String,
    listen: String,
    allowed_hosts: Vec<String>,
    tmp_dir: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut vault = None;
    let mut listen = None;
    let mut allowed_hosts = Vec::new();
    let mut tmp_dir = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| it.next().ok_or_else(|| format!("{flag} needs a value"));
        match arg.as_str() {
            "--vault" => vault = Some(value("--vault")?),
            "--listen" => listen = Some(value("--listen")?),
            "--allowed-host" => allowed_hosts.push(value("--allowed-host")?),
            "--tmp-dir" => tmp_dir = Some(value("--tmp-dir")?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Args {
        vault: vault.ok_or("--vault is required")?,
        listen: listen.ok_or("--listen is required")?,
        allowed_hosts,
        tmp_dir,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("obsidian-mcp: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let vault = match Vault::open(&args.vault) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("obsidian-mcp: cannot open vault {}: {e}", args.vault);
            return ExitCode::FAILURE;
        }
    };
    let vault = match args.tmp_dir.as_deref().map(|d| vault.clone().with_tmp_dir(d)) {
        None => vault,
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            eprintln!("obsidian-mcp: cannot use the temporary folder: {e}");
            return ExitCode::FAILURE;
        }
    };
    match vault.prepare_tmp_dir() {
        Ok(TmpPlacement::TmpDir(dir)) => eprintln!("obsidian-mcp: temporary files go to {}", dir.display()),
        Ok(TmpPlacement::BesideTarget) => eprintln!(
            "obsidian-mcp: warning: {} is on another filesystem than the vault, so a rename from it cannot work; \
             temporary files will be written as hidden files next to each changed note instead",
            vault.tmp_dir().display()
        ),
        Err(e) => {
            eprintln!("obsidian-mcp: cannot prepare the temporary folder {}: {e}", vault.tmp_dir().display());
            return ExitCode::FAILURE;
        }
    }
    let listener = match tokio::net::TcpListener::bind(&args.listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("obsidian-mcp: cannot listen on {}: {e}", args.listen);
            return ExitCode::FAILURE;
        }
    };
    eprintln!("obsidian-mcp: serving {} at http://{}/mcp", vault.root().display(), args.listen);
    let app = transport::router(Tools::new(vault), &args.allowed_hosts);
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match axum::serve(listener, app).with_graceful_shutdown(shutdown).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("obsidian-mcp: server stopped: {e}");
            ExitCode::FAILURE
        }
    }
}
