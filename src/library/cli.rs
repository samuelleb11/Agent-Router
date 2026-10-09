//! `agent-router library`: the Library on a machine whose agent is installed, from the
//! command line. It reads the router's address and password from `agent.json`, as the agent
//! does, so it asks for nothing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};

use super::bundle::Bundle;
use super::sync::{self, Places, Router};
use super::Kind;

pub const USAGE: &str = "\
agent-router library — the hosted router's Library on this machine

USAGE:
  agent-router library status    each item, and where it stands here: in step, behind,
                                 changed here, not the Library's, failed or withdrawn
  agent-router library sync      bring this machine in step now, and say what changed
  agent-router library publish <dir | file.md> [--kind skill|command|subagent] [--name <name>]
                                 publish a version from this machine: a skill's directory,
                                 or a command's or subagent's file. It is signed with this
                                 machine and, run by Claude Code, its session; it waits for
                                 approval unless the owner trusts publishes from machines.
                                 An item from a repository is changed in that repository.

The Library wins: a sync puts back an item edited here, and keeps the copy it replaced in
<data-dir>/library/replaced/. A machine installed with --no-library takes no part.
";

pub async fn main(args: &[String]) -> Result<()> {
    let outcome = match args.first().map(String::as_str) {
        Some("status") if args.len() == 1 => status().await,
        Some("sync") if args.len() == 1 => sync_now().await,
        Some("publish") => publish(&args[1..]).await,
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

/// What a publish sends: an item's kind and name, and where its files are.
#[derive(Debug, PartialEq, Eq)]
pub struct ToPublish {
    pub path: PathBuf,
    pub kind: Kind,
    pub name: String,
}

fn parse_publish(args: &[String]) -> Result<ToPublish> {
    let (mut path, mut kind, mut name) = (None, None, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--kind" => {
                let given = rest.next().ok_or_else(|| anyhow!("--kind needs skill, command or subagent"))?;
                kind = Some(Kind::parse(given).ok_or_else(|| anyhow!("no kind is called {given}: skill, command or subagent"))?);
            }
            "--name" => name = Some(rest.next().ok_or_else(|| anyhow!("--name needs a name"))?.clone()),
            other if other.starts_with("--") => bail!("unknown option {other:?}"),
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => bail!("one thing at a time: {other:?} is a second path"),
        }
    }
    let path = path.ok_or_else(|| anyhow!("publish what? Name a skill's directory, or a command's or subagent's file"))?;
    let kind = match kind {
        Some(kind) => kind,
        None if path.is_dir() => Kind::Skill,
        // A file says what it is by the directory Claude Code reads it from.
        None => match path.parent().and_then(Path::file_name).and_then(|p| p.to_str()) {
            Some("agents") => Kind::Subagent,
            Some("commands") => Kind::Command,
            _ => bail!("{} is a file: say what it is with --kind command, or --kind subagent", path.display()),
        },
    };
    let name = match name {
        Some(name) => name,
        None => {
            let stem = if kind == Kind::Skill { path.file_name() } else { path.file_stem() };
            stem.and_then(|s| s.to_str()).ok_or_else(|| anyhow!("{} has no name to publish it under: give one with --name", path.display()))?.to_string()
        }
    };
    Ok(ToPublish { path, kind, name })
}

/// Publish from this machine, as `agent-router library publish` does once it has read
/// `agent.json`: the bundle is checked here first, as the router will check it.
pub async fn publish_from_here(
    router: &Router,
    what: &ToPublish,
    machine: &str,
    label: &str,
    session: Option<&str>,
) -> Result<serde_json::Value> {
    let bundle = Bundle::from_path(what.kind, &what.name, &what.path).map_err(|why| anyhow!("{why}"))?;
    router.publish(&bundle, machine, label, session).await
}

async fn publish(args: &[String]) -> Result<()> {
    let what = parse_publish(args)?;
    rustls::crypto::ring::default_provider().install_default().ok();
    crate::config::settle_data_dir(None)?;
    let saved = crate::agent::connection()?;
    let machine = saved.machine.ok_or_else(|| anyhow!("this machine has no id yet: start its agent once, or run `agent-router agent install` again"))?;
    let router = Router::new(&saved.server, &saved.password)?;
    // Claude Code names its session to every command a session runs.
    let session = std::env::var("CLAUDE_CODE_SESSION_ID").ok().filter(|s| !s.trim().is_empty());
    let answer = publish_from_here(&router, &what, &machine, &saved.label, session.as_deref()).await?;
    let version = &answer["version"];
    let number = version["number"].as_i64().unwrap_or_default();
    let (kind, name) = (what.kind, &what.name);
    if answer["unchanged"] == true {
        println!("the {kind} {name} is version {number} already: nothing new to publish");
    } else if version["state"] == "live" {
        println!("published the {kind} {name} as version {number}: live, and the machines are told now");
    } else {
        println!("published the {kind} {name} as version {number}: it waits for approval in the console's Library");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::*;
    use crate::api_refusals::Lab;

    #[tokio::test]
    async fn publish_from_a_machine_records_machine_and_session() {
        rustls::crypto::ring::default_provider().install_default().ok();
        let lab = Lab::hosted("publish-machine").await;
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab.app.clone().into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });

        // A skill an agent improved on this machine, in Claude Code's own skills directory.
        let root = std::env::temp_dir().join(format!("ar-publish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(".claude/skills/bench");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: bench\ndescription: Drives the bench, better.\n---\n").unwrap();
        let what = parse_publish(&[dir.display().to_string()]).unwrap();
        assert_eq!((what.kind, what.name.as_str()), (Kind::Skill, "bench"), "a directory is a skill, named by it");

        // What `publish` does once it has read agent.json: nothing is asked of anyone.
        let router = Router::new(&format!("http://{addr}"), lab.password.unwrap()).unwrap();
        let session = "4d5e6f70-0000-4000-8000-00000000000d";
        let answer = publish_from_here(&router, &what, "machine-1", "bench mac", Some(session)).await.unwrap();
        assert_eq!(answer["version"]["number"], 1);
        assert_eq!(
            answer["version"]["provenance"],
            serde_json::json!({ "from": "machine", "machine": "machine-1", "label": "bench mac", "session": session })
        );

        // Without a session — run by hand, not by Claude Code — the version names none.
        std::fs::write(dir.join("SKILL.md"), "---\nname: bench\ndescription: Drives the bench, by hand.\n---\n").unwrap();
        let answer = publish_from_here(&router, &what, "machine-1", "bench mac", None).await.unwrap();
        assert!(answer["version"]["provenance"].get("session").is_none(), "{answer}");

        // A file says what it is by the directory Claude Code reads it from.
        let agents = root.join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("reviewer.md"), "x").unwrap();
        let what = parse_publish(&[agents.join("reviewer.md").display().to_string()]).unwrap();
        assert_eq!((what.kind, what.name.as_str()), (Kind::Subagent, "reviewer"));
        std::fs::write(root.join("notes.md"), "x").unwrap();
        assert!(parse_publish(&[root.join("notes.md").display().to_string()]).unwrap_err().to_string().contains("--kind"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
