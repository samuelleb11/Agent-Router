//! Agent Router's own skill, the Library's first item: `skills/agent-router/` in this
//! repository, built into the binary and published from it when a hosted router starts.
//! docs/library.md, "The router's own skill".
//!
//! It describes the agent's commands, and a machine's agent comes from the router's own
//! image, so the two always agree: a router rolled back publishes the older skill, which is
//! still the right one for it. Its versions come from the build alone (`store::publish`
//! refuses every other write to it, naming the repository path).

use std::path::Path;

use anyhow::anyhow;
use rust_embed::RustEmbed;

use super::bundle::{Bundle, File};
use super::store::{self, Provenance, Published};
use super::{Kind, Refused};

/// What the skill is called, and where it lives in this repository.
pub const NAME: &str = "agent-router";

#[derive(RustEmbed)]
#[folder = "skills/agent-router/"]
struct Files;

/// The skill as this build holds it, checked as any other.
pub fn bundle() -> Result<Bundle, Refused> {
    let files = Files::iter()
        .filter_map(|path| Files::get(&path).map(|file| File { path: path.to_string(), bytes: file.data.into_owned(), executable: false }))
        .collect();
    Bundle::check(Kind::Skill, NAME, files)
}

/// Publish this build's skill: live, signed with the build, and nothing new when the
/// Library holds it already.
pub fn publish(db: &Path) -> anyhow::Result<Published> {
    let bundle = bundle().map_err(|why| anyhow!("the router's own skill does not pass the Library's checks: {why}"))?;
    let conn = store::writer(db)?;
    let build = Provenance::Router { build: env!("AGENT_ROUTER_BUILD").to_string() };
    store::publish(&conn, &bundle, &build, true, crate::db::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::bundle;

    #[test]
    fn the_routers_own_skill_passes_the_librarys_checks() {
        let built = bundle().expect("the skill this build carries");
        assert_eq!(built.name(), NAME);
        assert!(built.files().iter().any(|f| f.path == "SKILL.md"));
        // What is built in is what the repository holds.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills").join(NAME);
        let on_disk = Bundle::from_path(Kind::Skill, NAME, &dir).expect("skills/agent-router as the Library reads a directory");
        assert_eq!(bundle::digest(&built.pack()), bundle::digest(&on_disk.pack()));
    }

    #[tokio::test]
    async fn the_router_publishes_its_own_skill_at_start() {
        let lab = crate::api_refusals::Lab::hosted("builtin-start").await;
        let db = &lab.state.config.db_path;
        let first = publish(db).unwrap();
        let Published::New(version) = first else { panic!("a first start publishes it: {first:?}") };
        assert_eq!((version.number, version.state), (1, store::State::Live));
        assert_eq!(version.provenance, Provenance::Router { build: env!("AGENT_ROUTER_BUILD").into() });
        // Started again with the same build: nothing new.
        assert!(matches!(publish(db).unwrap(), Published::Unchanged(v) if v.number == 1));
        let conn = store::writer(db).unwrap();
        let live = store::live(&conn).unwrap();
        assert_eq!(live.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), [NAME], "the first item every machine gets");
    }

    #[tokio::test]
    async fn the_routers_own_skill_refuses_other_writes() {
        let lab = crate::api_refusals::Lab::hosted("builtin-owns").await;
        let db = lab.state.config.db_path.clone();
        publish(&db).unwrap();
        let conn = store::writer(&db).unwrap();
        let mine = Bundle::check(
            Kind::Skill,
            NAME,
            vec![File { path: "SKILL.md".into(), bytes: b"---\nname: agent-router\ndescription: Mine.\n---\n".to_vec(), executable: false }],
        )
        .unwrap();
        for from in [
            Provenance::Upload,
            Provenance::Token { token: "ci".into() },
            Provenance::Machine { machine: "m".into(), label: None, session: None },
            Provenance::Forge { source: "s".into(), forge: "f".into(), repository: "r/r".into(), commit: "c".into(), path: "p".into() },
        ] {
            let Published::Refused(why) = store::publish(&conn, &mine, &from, true, 2).unwrap() else { panic!("{from:?} was let in") };
            assert!(why.contains("comes with the router's build: change skills/agent-router in Agent Router's repository"), "{why}");
        }
        assert_eq!(store::versions(&conn, Kind::Skill, NAME).unwrap().len(), 1);
    }
}
