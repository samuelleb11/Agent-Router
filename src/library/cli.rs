//! `agent-router library`: the Library on a machine whose agent is installed, from the
//! command line. It reads the router's address and password from `agent.json`, as the agent
//! does, so it asks for nothing.

use std::sync::Arc;

use anyhow::Result;

use super::sync::{self, Places, Router};

pub const USAGE: &str = "\
agent-router library — the hosted router's Library on this machine

USAGE:
  agent-router library status    each item, and where it stands here: in step, behind,
                                 changed here, not the Library's, failed or withdrawn
  agent-router library sync      bring this machine in step now, and say what changed

The Library wins: a sync puts back an item edited here, and keeps the copy it replaced in
<data-dir>/library/replaced/. A machine installed with --no-library takes no part.
";

pub async fn main(args: &[String]) -> Result<()> {
    let outcome = match args.first().map(String::as_str) {
        Some("status") if args.len() == 1 => status().await,
        Some("sync") if args.len() == 1 => sync_now().await,
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        _ => {
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = outcome {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

fn connect() -> Result<(Places, Arc<Router>, Option<bool>)> {
    rustls::crypto::ring::default_provider().install_default().ok();
    let data = crate::config::settle_data_dir(None)?;
    let saved = crate::agent::connection()?;
    let router = Arc::new(Router::new(&saved.server, &saved.password)?);
    Ok((Places::system(data), router, saved.library))
}

async fn status() -> Result<()> {
    let (places, router, library) = connect()?;
    if library == Some(false) {
        println!("this machine takes no part in the Library (it was installed with --no-library)");
    }
    let wanted = router.manifest().await?;
    let status = tokio::task::spawn_blocking(move || sync::status(&places, &wanted)).await?;
    for line in sync::status_lines(&status) {
        println!("{line}");
    }
    Ok(())
}

async fn sync_now() -> Result<()> {
    let (places, router, library) = connect()?;
    match sync::run_once(places, router, library).await? {
        Some(report) => {
            for line in sync::report_lines(&report) {
                println!("{line}");
            }
        }
        None => println!("nothing synced: this machine takes no part in the Library (it was installed with --no-library)"),
    }
    Ok(())
}
