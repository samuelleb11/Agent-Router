//! The forges the Library imports from, GitLab and GitHub, reached over their HTTP APIs: the
//! router's image is distroless and has no git. docs/library.md, "Forges and sources".
//!
//! The model follows Northstar's `crates/northstar-forge`: a forge is a base URL with one
//! token, kept by the router and never read back. What Northstar's crate does not have is
//! added here: a ref's head commit, and an archive of a path at a commit.

use std::sync::OnceLock;
use std::time::Duration;

use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode, Uri};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::bundle::{self, File};
use super::Refused;

/// The most a repository's archive may be, as it arrives and as it unpacks. GitHub's tarball
/// is always the whole repository; a source in a bigger one names a GitLab path, or a smaller
/// repository.
pub const ARCHIVE_MAX: usize = 64 * 1024 * 1024;
pub const UNPACKED_MAX: usize = 256 * 1024 * 1024;

const WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    GitLab,
    GitHub,
}

impl ForgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ForgeKind::GitLab => "gitlab",
            ForgeKind::GitHub => "github",
        }
    }

    pub fn parse(s: &str) -> Option<ForgeKind> {
        [ForgeKind::GitLab, ForgeKind::GitHub].into_iter().find(|k| k.as_str() == s)
    }
}

/// A forge as the router reaches it.
#[derive(Clone, Debug)]
pub struct Forge {
    pub key: String,
    pub kind: ForgeKind,
    /// `https://gitlab.example.com`, `https://github.com`, or a GitHub Enterprise server.
    pub base_url: String,
    pub token: Option<String>,
}

/// Why a forge did not give what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeError {
    /// 401: the token is wrong, expired or revoked — or a private repository needs one.
    Unauthorised,
    /// 403: the token may not read this.
    InsufficientScope,
    /// 404: no such repository, ref or path, as far as this token can see.
    NotFound(String),
    Unreachable(String),
    Unexpected(String),
}

impl std::fmt::Display for ForgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForgeError::Unauthorised => {
                f.write_str("the forge refused the token (401): it is wrong, expired or revoked, or the repository is private and there is none")
            }
            ForgeError::InsufficientScope => f.write_str(
                "the token may not read this (403): GitLab needs read_api, and GitHub read access to the repository's contents",
            ),
            ForgeError::NotFound(what) => write!(f, "the forge has no {what}, as far as its token can see (404)"),
            ForgeError::Unreachable(why) => write!(f, "the forge could not be reached: {why}"),
            ForgeError::Unexpected(why) => f.write_str(why),
        }
    }
}

/// Where a forge's base URL is accepted: https, or plain http on loopback alone (a forge on
/// this machine, a test's). A token is never sent in the clear across a network.
pub fn check_base_url(base_url: &str) -> Result<String, Refused> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let uri: Uri = trimmed.parse().map_err(|_| Refused(format!("{base_url} is not a URL")))?;
    let host = uri.host().unwrap_or_default();
    let loopback = host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    match uri.scheme_str() {
        Some("https") if !host.is_empty() => Ok(trimmed.to_string()),
        Some("http") if loopback => Ok(trimmed.to_string()),
        Some("http") => Err(Refused(format!("{base_url} is plain http: use https, so the forge's token never crosses a network in the clear"))),
        _ => Err(Refused(format!("{base_url} is not an https URL"))),
    }
}

/// A repository's path as its forge names it: `group/subgroup/project` on GitLab, `owner/repo`
/// on GitHub.
pub fn check_repository(kind: ForgeKind, repository: &str) -> Result<(), Refused> {
    let parts: Vec<&str> = repository.split('/').collect();
    // No part may start with a dot: `..` would climb out of the API's path.
    let fits = parts
        .iter()
        .all(|p| !p.is_empty() && !p.starts_with('.') && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
        && match kind {
            ForgeKind::GitHub => parts.len() == 2,
            ForgeKind::GitLab => parts.len() >= 2,
        };
    if fits {
        Ok(())
    } else {
        Err(Refused(match kind {
            ForgeKind::GitHub => format!("{repository} is not a GitHub repository: write it owner/repo"),
            ForgeKind::GitLab => format!("{repository} is not a GitLab project path: write it group/project"),
        }))
    }
}

type Http = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    http_body_util::Full<Bytes>,
>;

fn http() -> &'static Http {
    static CLIENT: OnceLock<Http> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let tls = hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots().https_or_http().enable_http1().build();
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new()).build(tls)
    })
}

/// Every unreserved character as it is, everything else %-escaped: a GitLab project path or a
/// ref with a `/` in it becomes one path segment.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Forge {
    /// The forge's API: GitLab's `/api/v4`; GitHub's own host for github.com, or an
    /// Enterprise server's `/api/v3`.
    fn api(&self) -> String {
        match self.kind {
            ForgeKind::GitLab => format!("{}/api/v4", self.base_url),
            ForgeKind::GitHub if self.base_url.trim_end_matches('/').ends_with("://github.com") => "https://api.github.com".into(),
            ForgeKind::GitHub => format!("{}/api/v3", self.base_url),
        }
    }

    /// GET `url`, following a redirect or three — GitHub sends archives from another host,
    /// which is never shown the token.
    async fn get(&self, url: &str, accept: Option<&str>, most: usize) -> Result<(StatusCode, HeaderMap, Bytes), ForgeError> {
        use http_body_util::BodyExt;
        let origin = url.parse::<Uri>().ok().and_then(|u| u.authority().map(|a| a.to_string()));
        let mut url = url.to_string();
        for _ in 0..4 {
            let here = url.parse::<Uri>().ok().and_then(|u| u.authority().map(|a| a.to_string()));
            let mut request = Request::builder()
                .method(Method::GET)
                .uri(&url)
                .header(header::USER_AGENT, concat!("agent-router/", env!("CARGO_PKG_VERSION")));
            if let Some(accept) = accept {
                request = request.header(header::ACCEPT, accept);
            }
            if let (Some(token), true) = (&self.token, here == origin) {
                let (name, value) = match self.kind {
                    ForgeKind::GitLab => ("private-token", token.clone()),
                    ForgeKind::GitHub => ("authorization", format!("Bearer {token}")),
                };
                let mut value = HeaderValue::from_str(&value).map_err(|_| ForgeError::Unexpected("the token cannot be sent in a header".into()))?;
                value.set_sensitive(true);
                request = request.header(name, value);
            }
            let request = request.body(http_body_util::Full::new(Bytes::new())).map_err(|e| ForgeError::Unexpected(e.to_string()))?;
            let response = tokio::time::timeout(WAIT, http().request(request))
                .await
                .map_err(|_| ForgeError::Unreachable(format!("{} did not answer in time", self.base_url)))?
                .map_err(|e| ForgeError::Unreachable(e.to_string()))?;
            let status = response.status();
            if status.is_redirection() {
                let to = response.headers().get(header::LOCATION).and_then(|l| l.to_str().ok()).map(str::to_string);
                url = to.ok_or_else(|| ForgeError::Unexpected(format!("{} redirected nowhere", self.base_url)))?;
                continue;
            }
            let headers = response.headers().clone();
            let body = http_body_util::Limited::new(response.into_body(), most)
                .collect()
                .await
                .map_err(|_| ForgeError::Unexpected(format!("the forge's answer is over {} MiB", most / (1024 * 1024))))?
                .to_bytes();
            return Ok((status, headers, body));
        }
        Err(ForgeError::Unexpected(format!("{} redirected too many times", self.base_url)))
    }

    fn judge(status: StatusCode, what: &str) -> Result<(), ForgeError> {
        match status.as_u16() {
            200..=299 => Ok(()),
            401 => Err(ForgeError::Unauthorised),
            403 => Err(ForgeError::InsufficientScope),
            404 => Err(ForgeError::NotFound(what.to_string())),
            other => Err(ForgeError::Unexpected(format!("the forge answered {other} for {what}"))),
        }
    }

    /// The commit a ref names now.
    pub async fn head(&self, repository: &str, reference: &str) -> Result<String, ForgeError> {
        let what = format!("ref {reference} in {repository}");
        let commit = match self.kind {
            ForgeKind::GitLab => {
                let url = format!("{}/projects/{}/repository/commits/{}", self.api(), encode(repository), encode(reference));
                let (status, _, body) = self.get(&url, None, 1 << 20).await?;
                Self::judge(status, &what)?;
                #[derive(Deserialize)]
                struct Commit {
                    id: String,
                }
                serde_json::from_slice::<Commit>(&body).map_err(|e| ForgeError::Unexpected(e.to_string()))?.id
            }
            ForgeKind::GitHub => {
                let url = format!("{}/repos/{repository}/commits/{}", self.api(), encode(reference));
                let (status, _, body) = self.get(&url, Some("application/vnd.github.sha"), 1 << 20).await?;
                Self::judge(status, &what)?;
                String::from_utf8_lossy(&body).trim().to_string()
            }
        };
        if commit.len() < 7 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ForgeError::Unexpected(format!("the forge named the head of {what} as {commit:?}, which is not a commit")));
        }
        Ok(commit)
    }

    /// The files under `path` at `commit`, relative to it. GitLab is asked for the path alone;
    /// GitHub only gives the whole repository, which is cut down here.
    pub async fn files(&self, repository: &str, commit: &str, path: &str) -> Result<Vec<File>, ForgeError> {
        let url = match self.kind {
            ForgeKind::GitLab if path.is_empty() => {
                format!("{}/projects/{}/repository/archive.tar.gz?sha={commit}", self.api(), encode(repository))
            }
            ForgeKind::GitLab => {
                format!("{}/projects/{}/repository/archive.tar.gz?sha={commit}&path={}", self.api(), encode(repository), encode(path))
            }
            ForgeKind::GitHub => format!("{}/repos/{repository}/tarball/{commit}", self.api()),
        };
        let (status, _, body) = self.get(&url, None, ARCHIVE_MAX).await?;
        Self::judge(status, &format!("archive of {repository} at {commit}"))?;
        let tar = bundle::gunzip_at_most(&body, UNPACKED_MAX).map_err(|why| ForgeError::Unexpected(why.0))?;
        under(&tar, path).map_err(|why| ForgeError::Unexpected(why.0))
    }

    /// Whether the forge answers, and takes the token for `repository` when one is named.
    pub async fn check(&self, repository: Option<&str>) -> Result<(), ForgeError> {
        let url = match (self.kind, repository) {
            (ForgeKind::GitLab, Some(repository)) => format!("{}/projects/{}", self.api(), encode(repository)),
            (ForgeKind::GitLab, None) => format!("{}/version", self.api()),
            (ForgeKind::GitHub, Some(repository)) => format!("{}/repos/{repository}", self.api()),
            (ForgeKind::GitHub, None) if self.token.is_some() => format!("{}/user", self.api()),
            (ForgeKind::GitHub, None) => format!("{}/rate_limit", self.api()),
        };
        let (status, _, _) = self.get(&url, None, 1 << 20).await?;
        Self::judge(status, repository.unwrap_or("API"))
    }
}

/// An archive's files under `path`, relative to it. A forge's archive holds one directory
/// around everything (`project-<sha>/`, `owner-repo-<sha>/`); within it, the path's files
/// sit under the path, or — when the forge cut the archive to the path — at the top.
fn under(tar: &[u8], path: &str) -> Result<Vec<File>, Refused> {
    let path = path.trim_matches('/');
    let inside = |name: &str| name.split_once('/').map(|(_, rest)| rest.to_string()).filter(|rest| !rest.is_empty());
    let prefix = format!("{path}/");
    let names = std::cell::RefCell::new(Vec::new());
    bundle::read_tar_where(tar, |name| {
        names.borrow_mut().extend(inside(name));
        None
    })?;
    let rooted = path.is_empty() || !names.borrow().iter().any(|n| n.starts_with(&prefix));
    bundle::read_tar_where(tar, |name| {
        let rest = inside(name)?;
        if rooted {
            Some(rest)
        } else {
            rest.strip_prefix(&prefix).filter(|r| !r.is_empty()).map(str::to_string)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forge_is_reached_over_https_or_on_loopback() {
        assert_eq!(check_base_url("https://gitlab.example.com/").unwrap(), "https://gitlab.example.com");
        assert!(check_base_url("http://127.0.0.1:9000").is_ok());
        assert!(check_base_url("http://gitlab.example.com").unwrap_err().0.contains("plain http"));
        assert!(check_base_url("gitlab.example.com").is_err());
        assert!(check_repository(ForgeKind::GitHub, "anthropics/skills").is_ok());
        assert!(check_repository(ForgeKind::GitHub, "anthropics/skills/x").is_err());
        assert!(check_repository(ForgeKind::GitLab, "whnet/tools/skills").is_ok());
        assert!(check_repository(ForgeKind::GitLab, "../etc").is_err());
        assert_eq!(encode("whnet/tools"), "whnet%2Ftools");
        let github = Forge { key: "gh".into(), kind: ForgeKind::GitHub, base_url: "https://github.com".into(), token: None };
        assert_eq!(github.api(), "https://api.github.com");
        let enterprise = Forge { base_url: "https://ghe.example.com".into(), ..github };
        assert_eq!(enterprise.api(), "https://ghe.example.com/api/v3");
    }
}
