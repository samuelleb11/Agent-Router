//! Which tree this binary was compiled from, baked in as `AGENT_ROUTER_BUILD` and served
//! as `build` by `/_router/health`.
//!
//! The crate version is 0.1.0 for every build, so health could not say whether what is
//! serving is what was just deployed — on a laptop after a handover, or in a pod after a
//! rollout. Ported from Northstar's `build.rs`, for the same reasons:
//!
//! - It names the COMPILED TREE, not the base commit: two builds of different
//!   uncommitted work on one commit would otherwise share an id.
//! - The tree is `git write-tree` over the working tree on a PRIVATE index, so untracked
//!   files that are compiled in count, and the real index — shared by every session
//!   working in this checkout — is never touched. "Is it HEAD?" is then this id against
//!   `git rev-parse HEAD^{tree}`.
//! - It reruns whenever any file git would put in the tree changes, from a list derived
//!   from `git ls-files` on every run: the binary embeds files from outside `src/`
//!   (`assets/`), and a hand-kept list of watched paths would rot into a stale id.
//! - No git, no id: `unknown`, never a fallback that looks like a real sha.
//!
//! The limit, stated rather than hidden: it names the tree when this script ran. An edit
//! that lands in the middle of a build can make it name a tree no binary was wholly
//! built from.

use std::path::Path;
use std::process::Command;

fn main() {
    let root = std::env::var("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from).ok();
    match root.as_deref().and_then(tree_files) {
        Some(files) if !files.is_empty() => {
            let root = root.as_deref().expect("files came from it");
            for f in files {
                println!("cargo:rerun-if-changed={}", root.join(f).display());
            }
        }
        // No git to ask: the id is `unknown` anyway, and rerunning every time is the only
        // honest option for a script that cannot see what changed.
        _ => println!("cargo:rerun-if-changed=.agent-router-build-id-always-rerun"),
    }
    let id = root.as_deref().and_then(tree_id).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=AGENT_ROUTER_BUILD={id}");
}

fn tree_files(root: &Path) -> Option<Vec<String>> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output()
        .ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .collect()
    })
}

fn tree_id(root: &Path) -> Option<String> {
    let index = Path::new(&std::env::var("OUT_DIR").ok()?).join("agent-router-build-id.index");
    let _ = std::fs::remove_file(&index);

    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_INDEX_FILE", &index)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    git(&["read-tree", "HEAD"])?;
    git(&["add", "-A"])?;
    let tree = git(&["write-tree"])?;
    let _ = std::fs::remove_file(&index);

    (tree.len() == 40 && tree.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("tree:{tree}"))
}
