//! The machine agent's installer, which a hosted router serves to whoever can reach it:
//! `GET /agent/install.sh` (macOS and Linux), `GET /agent/install.ps1` (Windows), and the
//! files they fetch, `GET /agent/<platform>/agent-router[.exe]` and
//! `GET /agent/<platform>/agent-router.vsix`. See `docs/hosting-machine-agent.md`.
//!
//! These are open without the password (`guard`). Nothing here is secret: the programs
//! are this repository's own, and a script carries only the address it was fetched from.
//! And a machine being connected has no password to present until the script asks for
//! it. What a script installs is the router's own build, from the pipeline that built
//! the image, so the agent always speaks the tunnel protocol of the router it dials.
//!
//! The files sit in a directory beside the binary, `../share/agent-router/agents` in the
//! image (`--agents-dir` elsewhere), as `scripts/stage-agents.sh` lays them out: one
//! directory per platform holding `agent-router[.exe]` and `agent-router.vsix`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::proxy::AppState;

/// Every platform an agent is built for, and its programs' suffix. Windows on Arm runs
/// the x64 build.
const PLATFORMS: [(&str, &str); 5] = [
    ("darwin-arm64", ""),
    ("darwin-x64", ""),
    ("linux-x64", ""),
    ("linux-arm64", ""),
    ("win32-x64", ".exe"),
];

const INSTALL_SH: &str = include_str!("downloads/install.sh");
const INSTALL_PS1: &str = include_str!("downloads/install.ps1");
/// Where a script takes the router's address, quoted for its language.
const SERVER_MARK: &str = "@SERVER@";

/// The default directory, relative to the running binary: `/usr/local/bin/agent-router`
/// finds `/usr/local/share/agent-router/agents`.
pub fn default_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("share").join("agent-router").join("agents"))
}

/// Whether `path` is one of the installer's own addresses, which `guard` leaves open on a
/// hosted router. Exact: anything else under `/agent/` is asked for the password.
pub fn is_open(path: &str) -> bool {
    match path.strip_prefix("/agent/") {
        Some("install.sh" | "install.ps1") => true,
        Some(rest) => rest.split_once('/').is_some_and(|(platform, name)| served(platform, name)),
        None => false,
    }
}

/// Whether `name` is a file the router serves for `platform`.
fn served(platform: &str, name: &str) -> bool {
    PLATFORMS
        .iter()
        .any(|(p, exe)| *p == platform && (name == format!("agent-router{exe}") || name == "agent-router.vsix"))
}

/// `GET /agent/install.sh`
pub async fn install_sh(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    script(INSTALL_SH.replace(SERVER_MARK, &sh_quote(&address(&state, &headers))))
}

/// `GET /agent/install.ps1`
pub async fn install_ps1(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    script(INSTALL_PS1.replace(SERVER_MARK, &ps_quote(&address(&state, &headers))))
}

/// Plain text, so a browser shows the script rather than saving it: reading what you are
/// about to pipe into a shell is the point of serving it as text.
fn script(body: String) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
        body,
    )
        .into_response()
}

/// The address a script connects the machine to: the deployment's public URL, or else the
/// one this request came in by. The Host was already held to the router's names by the
/// guard, and a forged scheme could only mislead the one who sent it.
fn address(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(url) = &state.config.public_url {
        return url.clone();
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    let https = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| p.trim().eq_ignore_ascii_case("https"));
    format!("{}://{host}", if https { "https" } else { "http" })
}

/// One POSIX shell word: single quotes, each `'` closed, escaped and reopened.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// One PowerShell single-quoted string, which the curly single quotes close as well.
fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// `GET /agent/<platform>/<file>`: one of the files the scripts fetch, as the image holds
/// it. Read whole: the largest is under 20 MB, and a machine is connected once.
pub async fn file(State(state): State<Arc<AppState>>, Path((platform, name)): Path<(String, String)>) -> Response {
    if !served(&platform, &name) {
        return missing(format!("no agent file {name} for {platform}"));
    }
    let Some(dir) = &state.config.agents_dir else {
        return missing("this router serves no agent files".to_string());
    };
    let path = dir.join(&platform).join(&name);
    let read = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)));
    match read {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing(format!(
            "this router carries no {name} for {platform}: its image was built without the agent's packages"
        )),
        Err(e) => {
            tracing::warn!(%platform, %name, "could not read an agent file: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("agent-router: could not read {name}: {e}\n")).into_response()
        }
    }
}

fn missing(why: String) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("agent-router: {why}\n"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_installers_own_addresses_are_open() {
        for path in [
            "/agent/install.sh",
            "/agent/install.ps1",
            "/agent/darwin-arm64/agent-router",
            "/agent/linux-x64/agent-router.vsix",
            "/agent/win32-x64/agent-router.exe",
            "/agent/win32-x64/agent-router.vsix",
        ] {
            assert!(is_open(path), "{path}");
        }
        for path in [
            "/agent/",
            "/agent",
            "/agent/install.bat",
            "/agent/win32-x64/agent-router",
            "/agent/darwin-arm64/agent-router.exe",
            "/agent/darwin-arm64/claude-wrapper",
            "/agent/darwin-arm64/../agent-router",
            "/agent/darwin-arm64/agent-router/x",
            "/agent/freebsd-x64/agent-router",
            "/agent/waiting",
            "/api/agent/install.sh",
            "/_router/agent",
        ] {
            assert!(!is_open(path), "{path}");
        }
    }

    #[test]
    fn an_address_is_one_word_in_either_language() {
        assert_eq!(sh_quote("https://r.example.com"), "'https://r.example.com'");
        assert_eq!(sh_quote("https://r.example.com/a'b"), r"'https://r.example.com/a'\''b'");
        assert_eq!(ps_quote("https://r.example.com/a'b"), "'https://r.example.com/a''b'");
        assert_eq!(ps_quote("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
    }

    #[test]
    fn each_script_takes_the_address_once() {
        for (name, text) in [("install.sh", INSTALL_SH), ("install.ps1", INSTALL_PS1)] {
            assert_eq!(text.matches(SERVER_MARK).count(), 1, "{name}");
        }
    }

    /// The shell script is checked by the shell that will run it: a syntax error would
    /// otherwise surface only on someone's machine, mid-`curl | sh`.
    #[cfg(unix)]
    #[test]
    fn the_shell_script_parses() {
        let text = INSTALL_SH.replace(SERVER_MARK, &sh_quote("https://r.example.com"));
        let path = std::env::temp_dir().join(format!("ar-install-{}.sh", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let out = std::process::Command::new("sh").arg("-n").arg(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    /// A machine opened from another computer through Remote - SSH has no `code` command,
    /// only the VS Code Server, and that is where Claude Code runs for such a window. A Mac's
    /// VS Code has its `code` inside the app. The script runs on a PATH of stand-ins alone,
    /// `uname` among them, so no VS Code on the machine running the test can be reached.
    #[cfg(unix)]
    #[test]
    fn the_shell_script_installs_the_extension_where_vs_code_runs() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;
        let root = std::env::temp_dir().join(format!("ar-install-vscode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let calls = root.join("calls");
        let stub = |path: &Path, body: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for tool in ["mktemp", "chmod", "rm"] {
            let real = ["/usr/bin", "/bin"].iter().map(|d| Path::new(d).join(tool)).find(|p| p.exists()).unwrap();
            std::os::unix::fs::symlink(real, bin.join(tool)).unwrap();
        }
        let uname = |os: &str| stub(&bin.join("uname"), &format!("case $1 in -s) echo {os} ;; *) echo arm64 ;; esac"));
        // Every download is a program that succeeds, which serves as the agent too.
        stub(&bin.join("curl"), r#"while [ "$1" != -o ]; do shift; done; printf '#!/bin/sh\nexit 0\n' > "$2""#);
        let server = root.join("home/.vscode-server/cli/servers/Stable-0123/server/bin");
        stub(&server.join("code-server"), &format!("echo \"code-server $*\" >> '{}'", calls.display()));
        stub(&server.join("remote-cli/code"), &format!("echo \"remote-cli $*\" >> '{}'", calls.display()));
        let script = root.join("install.sh");
        std::fs::write(&script, INSTALL_SH.replace(SERVER_MARK, &sh_quote("https://r.example.com"))).unwrap();

        let run = |path: String| {
            let _ = std::fs::remove_file(&calls);
            let out = std::process::Command::new("/bin/sh")
                .arg(&script)
                .env_clear()
                .env("PATH", path)
                .env("HOME", root.join("home"))
                .env("TMPDIR", &root)
                .env("AGENT_ROUTER_PASSWORD", "pw")
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(out.status.success(), "{said}");
            (std::fs::read_to_string(&calls).unwrap_or_default(), said)
        };
        let installed_once = |calls: &str, by: &str| {
            let lines: Vec<&str> = calls.lines().collect();
            lines.len() == 1
                && lines[0].starts_with(&format!("{by} --install-extension "))
                && lines[0].ends_with("/agent-router.vsix --force")
        };

        // On Linux, where no app is looked for.
        uname("Linux");

        // Headless, over ssh: no `code` anywhere, only the server.
        let (got, said) = run(bin.display().to_string());
        assert!(installed_once(&got, "code-server"), "{got}\n{said}");
        assert!(said.contains("Reload the VS Code windows"), "{said}");

        // In the server's own terminal `code` is its shim: the server takes it, once.
        let (got, said) = run(format!("{}:{}", server.join("remote-cli").display(), bin.display()));
        assert!(installed_once(&got, "code-server"), "{got}\n{said}");

        // Neither: nothing is installed, and the script says how to do it by hand.
        std::fs::remove_dir_all(root.join("home/.vscode-server")).unwrap();
        let (got, said) = run(bin.display().to_string());
        assert_eq!(got, "", "{said}");
        assert!(said.contains("nor a VS Code Server was found"), "{said}");

        // A Mac where VS Code's "Install 'code' command in PATH" was never run: the app's own
        // `code` takes it, once. ~/Applications is looked in first, so a VS Code in the
        // /Applications of a Mac running the test is never reached.
        uname("Darwin");
        let app = root.join("home/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code");
        stub(&app, &format!("echo \"app $*\" >> '{}'", calls.display()));
        let (got, said) = run(bin.display().to_string());
        assert!(installed_once(&got, "app"), "{got}\n{said}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
