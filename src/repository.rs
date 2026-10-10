//! Which repository a directory is a checkout of, spelt the one way Northstar's ADR 0028
//! spells it, so that a project is its repository on every machine: a clone on a laptop, a
//! clone on a build host and the worktrees beside either are one project, where their paths
//! made them several. The directory itself is never the key where a remote names one.
//!
//! Read on the machine the checkout is on, by asking `git` for the directory's remotes: a
//! hosted router has no checkout, so there it falls to the machine agent (`places`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// One of a directory's git remotes, as `git config` lists it, with any credential taken out
/// of its URL before it goes anywhere (`strip_credentials`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

/// ADR 0028's one spelling of a repository: the host lowercased, then the path, with the
/// scheme, the user, the port, a trailing `.git` and trailing slashes removed. An SSH clone
/// and an HTTPS clone of one repository hold the same work, so `git@gitlab.example.com:acme/
/// api.git` and `https://gitlab.example.com/acme/api` are one repository. A local path, a
/// `file://` URL or anything else with no host names no repository a second machine could
/// share, and gives none.
pub fn canonical(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.is_empty() || !scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b)) {
            return None;
        }
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        (host_of(authority)?, path)
    } else {
        let (authority, path) = scp_like(url)?;
        (host_of(authority)?, path)
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    (!path.is_empty()).then(|| format!("{}/{path}", host.to_ascii_lowercase()))
}

/// The host of `[user[:password]@]host[:port]`, an IPv6 literal's brackets kept.
fn host_of(authority: &str) -> Option<&str> {
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        &host_port[..=host_port.find(']')?]
    } else {
        host_port.split_once(':').map_or(host_port, |(h, _)| h)
    };
    (!host.is_empty()).then_some(host)
}

/// `[user@]host:path`, git's scp-like form, split at the colon. A path with a slash before
/// any colon is a local path, and a single letter before it a Windows drive.
fn scp_like(url: &str) -> Option<(&str, &str)> {
    let (left, path) = url.split_once(':')?;
    let local = left.contains('/') || left.contains('\\') || (left.len() == 1 && left.as_bytes()[0].is_ascii_alphabetic());
    (!local && !left.is_empty()).then_some((left, path))
}

/// The URL with any `user[:password]@` taken out. A remote can carry a token in it
/// (`https://oauth2:<token>@host/…`), and none may leave the machine; a bare user is taken
/// too, since a forge accepts a token in that place as well.
pub fn strip_credentials(url: &str) -> String {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, tail) = rest.find('/').map_or((rest, ""), |i| (&rest[..i], &rest[i..]));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        format!("{scheme}://{host}{tail}")
    } else if let Some((left, path)) = scp_like(url) {
        let host = left.rsplit_once('@').map_or(left, |(_, h)| h);
        format!("{host}:{path}")
    } else {
        url.to_string()
    }
}

/// A directory's repository: `origin`'s, else the first remote's by name that names one.
pub fn key_of(remotes: &[Remote]) -> Option<String> {
    let origin = remotes.iter().find(|r| r.name == "origin").and_then(|r| canonical(&r.url));
    origin.or_else(|| {
        let mut named: Vec<&Remote> = remotes.iter().collect();
        named.sort_by(|a, b| a.name.cmp(&b.name));
        named.into_iter().find_map(|r| canonical(&r.url))
    })
}

/// The remotes `git config --get-regexp '^remote\..*\.url$'` prints, one `remote.<name>.url
/// <url>` a line; a remote's name may itself hold dots.
fn parse_config(out: &str) -> Vec<Remote> {
    out.lines()
        .filter_map(|line| {
            let (key, url) = line.split_once(' ')?;
            let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
            Some(Remote { name: name.to_string(), url: strip_credentials(url) })
        })
        .collect()
}

/// How long `git` may take to list a directory's remotes before it is given up on.
const GIT_WAIT: Duration = Duration::from_secs(2);

/// A directory's remotes, from `git`. None for a directory that is not a checkout, does not
/// exist here, or when `git` is not installed or does not answer in time: such a directory is
/// its own key, as every directory was before. Blocking: call it off the async runtime.
pub fn remotes_of(dir: &Path) -> Vec<Remote> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut git = std::process::Command::new("git");
    git.arg("-C").arg(dir).args(["config", "--get-regexp", r"^remote\..*\.url$"]);
    git.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        git.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = git.spawn() else { return Vec::new() };
    let deadline = Instant::now() + GIT_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    }
    match child.wait_with_output() {
        Ok(out) => parse_config(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

/// How long a directory's remotes are taken as read. A remote is renamed or added rarely, and
/// every session listed asks for its directory's.
const REMEMBERED: Duration = Duration::from_secs(5 * 60);
/// Past this many directories, the ones read longest ago are let go.
const REMEMBERED_MOST: usize = 512;

/// `remotes_of`, remembered per directory for a while. Blocking, as `remotes_of` is.
pub fn remotes_cached(dir: &str) -> Vec<Remote> {
    static SEEN: OnceLock<Mutex<HashMap<String, (Instant, Vec<Remote>)>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some((at, remotes)) = seen.lock().unwrap_or_else(|e| e.into_inner()).get(dir) {
        if at.elapsed() < REMEMBERED {
            return remotes.clone();
        }
    }
    let remotes = remotes_of(Path::new(dir));
    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() >= REMEMBERED_MOST {
        seen.retain(|_, (at, _)| at.elapsed() < REMEMBERED);
    }
    seen.insert(dir.to_string(), (Instant::now(), remotes.clone()));
    remotes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0028's examples, and the shapes git writes a remote in.
    #[test]
    fn a_remote_is_spelt_one_way() {
        let one = Some("gitlab.example.com/acme/api".to_string());
        for url in [
            "git@gitlab.example.com:acme/api.git",
            "https://gitlab.example.com/acme/api",
            "https://gitlab.example.com/acme/api.git/",
            "ssh://git@gitlab.example.com:2222/acme/api.git",
            "https://oauth2:secret@GitLab.Example.com:8443/acme/api",
            "git://gitlab.example.com/acme/api.git",
            "git+ssh://git@gitlab.example.com/acme/api",
            "  gitlab.example.com:acme/api.git  ",
            "git@gitlab.example.com:/acme/api.git",
        ] {
            assert_eq!(canonical(url), one, "{url}");
        }
        assert_eq!(canonical("https://gitlab.example.com/Acme/API"), Some("gitlab.example.com/Acme/API".into()), "the path keeps its case");
        assert_eq!(canonical("ssh://git@[::1]:22/acme/api.git"), Some("[::1]/acme/api".into()));
        for none in ["/srv/git/api.git", "../api", "file:///srv/git/api.git", "C:\\src\\api", "C:/src/api", "", "https://host/", "git@host:"] {
            assert_eq!(canonical(none), None, "{none:?} names no repository");
        }
    }

    /// No credential leaves the machine, whatever shape the remote is written in.
    #[test]
    fn a_remotes_credentials_are_taken_out() {
        assert_eq!(strip_credentials("https://oauth2:glpat-x@gitlab.example.com/acme/api.git"), "https://gitlab.example.com/acme/api.git");
        assert_eq!(strip_credentials("https://ghp_x@github.com/o/r"), "https://github.com/o/r");
        assert_eq!(strip_credentials("git@gitlab.example.com:acme/api.git"), "gitlab.example.com:acme/api.git");
        assert_eq!(strip_credentials("/srv/git/api.git"), "/srv/git/api.git");
        let parsed = parse_config("remote.origin.url https://u:p@h.example/a/b.git\nremote.my.fork.url git@h.example:me/b.git\nnot a remote\n");
        assert_eq!(
            parsed,
            [
                Remote { name: "origin".into(), url: "https://h.example/a/b.git".into() },
                Remote { name: "my.fork".into(), url: "h.example:me/b.git".into() },
            ]
        );
    }

    /// `origin` names the repository; without it, the first remote by name that names one.
    #[test]
    fn a_directorys_repository_is_its_origins() {
        let r = |name: &str, url: &str| Remote { name: name.into(), url: url.into() };
        assert_eq!(key_of(&[r("upstream", "https://h/up/x"), r("origin", "https://h/me/x")]), Some("h/me/x".into()));
        assert_eq!(key_of(&[r("zeta", "https://h/z/x"), r("alpha", "/local/x"), r("beta", "https://h/b/x")]), Some("h/b/x".into()));
        assert_eq!(key_of(&[r("origin", "/local/x")]), None);
        assert_eq!(key_of(&[]), None);
    }
}
