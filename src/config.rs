//! Configuration: the flags `serve` was given, then the environment, then the settings
//! the router stored itself.
//!
//! Two modes share one binary. **Local** is what runs on the developer's own machine: no
//! arguments, or `serve` with a few, exactly as the router has always run. **Server**
//! (`serve --server`) is the hosted router — a container behind an HTTPS ingress, where
//! every caller is on another machine and nothing of the developer's (their `~/.claude`,
//! their Keychain, their processes) is there to read. See
//! `docs/hosting-on-kubernetes.md`.
//!
//! Hosting is switched on only by flags, never by an environment variable, so that a
//! binary too old to know a flag refuses to start rather than quietly ignoring it — the
//! chart passes every setting as an argument for that reason (Northstar does the same).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Where the password a hosted router answers to comes from. Named for the server, so no
/// client variable is ever mistaken for it: a shell that exports a token for some agent
/// must not be able to set the password of a router started from it.
pub const SERVER_PASSWORD_ENV: &str = "AGENT_ROUTER_SERVER_PASSWORD";

/// How long a hosted router waits for open requests after SIGTERM, unless told otherwise.
/// Well inside Kubernetes' default 30 s grace period, which also has to cover the capture
/// flush that follows.
const DEFAULT_DRAIN: Duration = Duration::from_secs(20);

pub struct Config {
    /// Hosted, rather than on the developer's own machine.
    pub server: bool,
    pub bind: SocketAddr,
    pub upstream: String,
    pub db_path: PathBuf,
    /// Largest body slice persisted to the `bodies` table, in bytes.
    pub body_cap: usize,
    /// How many requests get a full header dump at INFO. Diagnostics only.
    pub dump_headers: usize,
    /// Server mode: the names clients reach the router by, lowercase, without a port or
    /// a trailing dot. IP literals and `localhost` are always answered besides these.
    pub allow_hosts: Vec<String>,
    /// Server mode: the URL clients use, when the deployment knows it.
    pub public_url: Option<String>,
    /// Server mode: a read-only directory holding push keys (a mounted Secret).
    pub push_keys_dir: Option<PathBuf>,
    /// Server mode: how long SIGTERM waits for open requests.
    pub drain: Duration,
}

/// What `serve` was given on the command line.
#[derive(Debug, Default, Clone)]
pub struct ServeArgs {
    pub server: bool,
    pub bind: Option<IpAddr>,
    pub port: Option<u16>,
    pub data_dir: Option<PathBuf>,
    pub db: Option<PathBuf>,
    pub allow_hosts: Vec<String>,
    pub public_url: Option<String>,
    pub push_keys_dir: Option<PathBuf>,
    pub drain_seconds: Option<u64>,
}

impl ServeArgs {
    /// Both `--flag value` and `--flag=value`. Anything unknown is an error, so an old
    /// binary handed a new chart's arguments stops instead of serving without them.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut out = Self::default();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            i += 1;
            let (name, inline) = match arg.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
                _ => (arg.as_str(), None),
            };
            let mut value = || -> Result<String, String> {
                if let Some(v) = &inline {
                    return Ok(v.clone());
                }
                let v = args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))?;
                i += 1;
                Ok(v)
            };
            match name {
                "--server" if inline.is_none() => out.server = true,
                "--bind" => {
                    let v = value()?;
                    let ip = v.trim().parse().map_err(|_| format!("--bind wants an IP address, got {v:?}"))?;
                    out.bind = Some(ip);
                }
                "--port" => {
                    let v = value()?;
                    out.port = Some(v.trim().parse().map_err(|_| format!("--port wants a port number, got {v:?}"))?);
                }
                "--data-dir" => out.data_dir = Some(PathBuf::from(value()?)),
                "--db" => out.db = Some(PathBuf::from(value()?)),
                "--allow-host" => out.allow_hosts.push(value()?),
                "--public-url" => out.public_url = Some(value()?),
                "--push-keys-dir" => out.push_keys_dir = Some(PathBuf::from(value()?)),
                "--drain-seconds" => {
                    let v = value()?;
                    out.drain_seconds =
                        Some(v.trim().parse().map_err(|_| format!("--drain-seconds wants a number, got {v:?}"))?);
                }
                other => return Err(format!("unknown option {other:?}")),
            }
        }
        Ok(out)
    }
}

impl Config {
    pub fn load(args: &ServeArgs) -> anyhow::Result<Self> {
        let port: u16 = match args.port {
            Some(p) => p,
            None => std::env::var("AGENT_ROUTER_PORT")
                .unwrap_or_else(|_| "8787".into())
                .parse()?,
        };

        if !args.server {
            for (set, flag) in [
                (!args.allow_hosts.is_empty(), "--allow-host"),
                (args.public_url.is_some(), "--public-url"),
                (args.push_keys_dir.is_some(), "--push-keys-dir"),
                (args.drain_seconds.is_some(), "--drain-seconds"),
            ] {
                // A name the router answers to is only safe where every caller, loopback
                // included, must present the password — which is server mode. Locally,
                // loopback is never asked, so an allowed name would reopen the DNS
                // rebinding hole the Host check exists to close.
                anyhow::ensure!(!set, "{flag} needs --server");
            }
        }

        // Only once the flags are known to be good: a refusal should leave nothing
        // behind, not an empty data directory.
        let data_dir = resolve_data_dir(args.data_dir.as_deref())?;
        let db_path = match &args.db {
            Some(p) => p.clone(),
            None => db_path_in(&data_dir),
        };

        let bind = if args.server {
            let ip = args
                .bind
                .ok_or_else(|| anyhow::anyhow!("--server needs --bind (0.0.0.0 in a container)"))?;
            SocketAddr::new(ip, port)
        } else {
            // The bind address defaults to loopback and should stay there unless you mean
            // otherwise. Agent Router holds a live subscription credential and signs
            // whatever it forwards, so a routable bind hands that credential to everyone
            // on the network: `proxy` will sign for any caller that presents any
            // credential at all. It is widened deliberately, for following sessions from
            // another device on a network you trust, and then the password in `auth` is
            // what stands in the way.
            //
            // Three ways to say so, in this order: `--bind`, then AGENT_ROUTER_BIND, both
            // explicit overrides for a hand-launch; then the stored setting the dashboard
            // writes, which is the one that survives a reboot — launchd hands a login
            // item an almost empty environment, so an answer that lives only in a
            // variable is an answer that lasts until the next restart. See `network`.
            match args.bind {
                Some(ip) => SocketAddr::new(ip, port),
                None => match std::env::var("AGENT_ROUTER_BIND") {
                    Ok(addr) if !addr.trim().is_empty() => {
                        let ip: IpAddr = addr.trim().parse().map_err(|_| {
                            anyhow::anyhow!("AGENT_ROUTER_BIND must be an IP address, got {addr:?}")
                        })?;
                        SocketAddr::new(ip, port)
                    }
                    _ => match crate::network::stored(&db_path) {
                        Some(ip) => SocketAddr::new(ip, port),
                        None => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
                    },
                },
            }
        };

        let public_url = match &args.public_url {
            Some(url) => Some(parse_public_url(url)?.0),
            None => None,
        };
        let mut allow_hosts = Vec::new();
        for raw in &args.allow_hosts {
            let host = normalize_host_name(raw)
                .ok_or_else(|| anyhow::anyhow!("--allow-host wants a host name, got {raw:?}"))?;
            if !allow_hosts.contains(&host) {
                allow_hosts.push(host);
            }
        }
        if let Some(url) = &args.public_url {
            let host = parse_public_url(url)?.1;
            if !allow_hosts.contains(&host) {
                allow_hosts.push(host);
            }
        }

        let config = Self {
            server: args.server,
            bind,
            upstream: std::env::var("AGENT_ROUTER_UPSTREAM")
                .unwrap_or_else(|_| "https://api.anthropic.com".into()),
            db_path,
            body_cap: std::env::var("AGENT_ROUTER_BODY_CAP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(64 * 1024),
            dump_headers: std::env::var("AGENT_ROUTER_DUMP_HEADERS")
                .unwrap_or_else(|_| "1".into())
                .parse()
                .unwrap_or(1),
            allow_hosts,
            public_url,
            push_keys_dir: args.push_keys_dir.clone(),
            drain: args.drain_seconds.map(Duration::from_secs).unwrap_or(DEFAULT_DRAIN),
        };

        // The modules that write beside the store, or behave differently on a server,
        // read these rather than being handed a `Config` each. Set once, for the life of
        // the process; a second `load` (tests) keeps the first answer.
        let _ = SERVER.set(config.server);
        let _ = DATA_DIR.set(data_dir);
        let _ = DB_PATH.set(config.db_path.clone());
        Ok(config)
    }
}

static SERVER: OnceLock<bool> = OnceLock::new();
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static DB_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Whether this process is a hosted router. False until `Config::load` has run, which is
/// right for the subcommands (`install`, `status`) that never serve.
pub fn is_server() -> bool {
    SERVER.get().copied().unwrap_or(false)
}

/// Where everything the router writes lives: the store (unless `--db` or
/// `AGENT_ROUTER_DB` moved it), the secrets file, the archive shelf, the profile
/// directories, the installed binaries and the logs.
///
/// `--data-dir`, then `AGENT_ROUTER_DATA_DIR`, then `~/.agent-router`. Before there was
/// one answer there were five, each working out `~/.agent-router` on its own, so a store
/// moved with `AGENT_ROUTER_DB` left the secrets behind in `$HOME` — in a container,
/// on a disk that does not survive a restart.
pub fn data_dir() -> anyhow::Result<PathBuf> {
    match DATA_DIR.get() {
        Some(dir) => Ok(dir.clone()),
        None => resolve_data_dir(None),
    }
}

/// Settle the data directory for a process that never builds a `Config` (the machine
/// agent): `--data-dir`, else the usual resolution, kept for the rest of the process.
pub fn settle_data_dir(flag: Option<&Path>) -> anyhow::Result<PathBuf> {
    let dir = resolve_data_dir(flag)?;
    let _ = DATA_DIR.set(dir);
    data_dir()
}

/// Where the store lives, without building a whole `Config` around it: `install` needs
/// to write a setting into it before there is a router to ask.
pub fn db_path() -> anyhow::Result<PathBuf> {
    match DB_PATH.get() {
        Some(path) => Ok(path.clone()),
        None => Ok(db_path_in(&resolve_data_dir(None)?)),
    }
}

/// `AGENT_ROUTER_DB` moves only the store, as it always has; the default is inside the
/// data directory.
fn db_path_in(data_dir: &Path) -> PathBuf {
    match std::env::var("AGENT_ROUTER_DB") {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => data_dir.join("agent-router.db"),
    }
}

fn resolve_data_dir(flag: Option<&Path>) -> anyhow::Result<PathBuf> {
    let dir = match flag {
        Some(dir) => dir.to_path_buf(),
        None => match std::env::var("AGENT_ROUTER_DATA_DIR") {
            Ok(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => crate::setup::home_dir()?.join(".agent-router"),
        },
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("cannot create the data directory {}: {e}", dir.display()))?;
    Ok(dir)
}

/// The password a hosted router pins at every start. Required in server mode, and held
/// to the same minimum as one typed into the dashboard.
pub fn server_password() -> anyhow::Result<String> {
    let value = std::env::var(SERVER_PASSWORD_ENV).unwrap_or_default();
    anyhow::ensure!(
        !value.is_empty(),
        "--server needs the password in {SERVER_PASSWORD_ENV}: every caller of a hosted router presents it"
    );
    anyhow::ensure!(
        value.chars().count() >= crate::auth::MIN_LEN,
        "{SERVER_PASSWORD_ENV} is shorter than {} characters",
        crate::auth::MIN_LEN
    );
    Ok(value)
}

/// A host name as the Host check compares it: lowercase, one trailing dot dropped. `None`
/// for anything that is not a plain name — a scheme, a port, a path or a pattern would
/// otherwise be stored and then match nothing, which looks like a working configuration.
pub fn normalize_host_name(raw: &str) -> Option<String> {
    let name = raw.trim().to_ascii_lowercase();
    let name = name.strip_suffix('.').unwrap_or(&name).to_string();
    let plausible = !name.is_empty()
        && name.len() <= 253
        && !name.starts_with('.')
        && !name.starts_with('-')
        && !name.contains("..")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    plausible.then_some(name)
}

/// `(url without a trailing slash, its host)` for `--public-url`, which must be an
/// `http(s)://host[:port][/path]` URL whose host is a name.
fn parse_public_url(raw: &str) -> anyhow::Result<(String, String)> {
    let url = raw.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| anyhow::anyhow!("--public-url wants an https:// URL, got {raw:?}"))?;
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !port.is_empty() => h,
        _ => authority,
    };
    let host = normalize_host_name(host)
        .ok_or_else(|| anyhow::anyhow!("--public-url wants a URL with a host name, got {raw:?}"))?;
    Ok((url.to_string(), host))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_both_flag_forms() {
        let a = ServeArgs::parse(&args(&[
            "--server",
            "--bind=0.0.0.0",
            "--port",
            "9000",
            "--allow-host",
            "Router.Example.com.",
            "--allow-host=other.example.com",
            "--public-url=https://router.example.com/",
            "--drain-seconds=15",
            "--data-dir",
            "/data",
        ]))
        .unwrap();
        assert!(a.server);
        assert_eq!(a.bind, Some("0.0.0.0".parse().unwrap()));
        assert_eq!(a.port, Some(9000));
        assert_eq!(a.allow_hosts, vec!["Router.Example.com.", "other.example.com"]);
        assert_eq!(a.public_url.as_deref(), Some("https://router.example.com/"));
        assert_eq!(a.drain_seconds, Some(15));
        assert_eq!(a.data_dir, Some(PathBuf::from("/data")));
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        assert!(ServeArgs::parse(&args(&["--severs"])).is_err());
        assert!(ServeArgs::parse(&args(&["--bind"])).is_err());
        assert!(ServeArgs::parse(&args(&["--bind", "example.com"])).is_err());
        assert!(ServeArgs::parse(&args(&["--server=yes"])).is_err());
        assert!(ServeArgs::parse(&args(&["serve"])).is_err());
    }

    #[test]
    fn host_names_are_normalized_or_refused() {
        assert_eq!(normalize_host_name("Router.Example.COM."), Some("router.example.com".into()));
        assert_eq!(normalize_host_name("agent-router.ns.svc.cluster.local"), Some("agent-router.ns.svc.cluster.local".into()));
        for bad in ["", ".", "https://x.example.com", "x.example.com:443", "*.example.com", "a..b", "x/y", "-x"] {
            assert_eq!(normalize_host_name(bad), None, "{bad:?} should be refused");
        }
    }

    #[test]
    fn public_url_gives_its_host() {
        assert_eq!(
            parse_public_url("https://Router.Example.com/").unwrap(),
            ("https://Router.Example.com".to_string(), "router.example.com".to_string())
        );
        assert_eq!(parse_public_url("https://router.example.com:8443/x").unwrap().1, "router.example.com");
        assert!(parse_public_url("router.example.com").is_err());
        assert!(parse_public_url("ftp://router.example.com").is_err());
    }
}
