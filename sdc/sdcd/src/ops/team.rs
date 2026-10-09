//! **Agency Mode's roles** (1.0 in the plan): Owner, Developer, Reviewer, Client.
//!
//! | role      | may |
//! | --------- | --- |
//! | owner     | everything, including the team, the policy, the budgets |
//! | developer | work: chats, files, commands, staging, deploys that do not need an approval |
//! | reviewer  | read everything, run Verify, approve or decline |
//! | client    | read only: sites, health, reports, proof - and answer an approval meant for them |
//!
//! The kill switch is allowed to everyone: stopping is always safe.
//!
//! This build's team is **who is at this desk**: the person picks their name in Settings → Team, every
//! ledger row carries it, and the daemon refuses what the role may not do. Several people on several
//! machines sharing one agency needs a shared server, which SDC does not run (P1); the roles here are the
//! same rules that server would enforce.

use serde_json::{json, Value};

use crate::store::sqlite::Store;

pub const ROLES: &[&str] = &["owner", "developer", "reviewer", "client"];

/// The team as stored: `[{name, role}]`.
pub fn members(store: &Store) -> Vec<Value> {
    store
        .setting("team.members")
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
        .unwrap_or_default()
}

/// The role of the person at the desk. No team at all is one person, who owns everything.
pub fn current_role(store: &Store) -> String {
    let members = members(store);

    if members.is_empty() {
        return "owner".to_string();
    }

    let current = store.setting("team.current").ok().flatten().unwrap_or_default();

    members
        .iter()
        .find(|member| member["name"].as_str() == Some(current.as_str()))
        .and_then(|member| member["role"].as_str())
        .unwrap_or("client")
        .to_string()
}

/// Methods that only read.
fn reads(method: &str) -> bool {
    matches!(
        method,
        "host.status" | "host.doctor" | "session.list" | "project.list" | "event.list" | "event.subscribe" | "provider.list" | "models.list"
            | "checkpoint.list" | "checkpoint.files" | "checkpoint.fileDiff" | "audit.list" | "audit.verify" | "policy.get" | "cost.summary"
            | "cost.estimate" | "kill.list" | "fs.read" | "fs.list" | "fs.stat" | "fs.search" | "git.status" | "git.diff" | "engine.status"
            | "site.list" | "deploy.list" | "deploy.get" | "deploy.preview" | "health.history" | "approval.list" | "xray.get" | "proof.export"
            | "intent.detect" | "intent.stats" | "glossary.list" | "team.get" | "trust.score" | "voice.status" | "update.check" | "crash.list"
            | "cli.recipes" | "provider.registry.list" | "provider.local.doctor" | "timeline.branches" | "playbook.list" | "pty.output"
            | "tool.list" | "tool.status" | "provider.warm"
    )
}

/// Whether the role may call the method - or the sentence that says why not.
pub fn allowed(role: &str, method: &str) -> Result<(), String> {
    if method == "kill.all" || method == "host.shutdown" || reads(method) {
        return Ok(());
    }

    let denied = match role {
        "owner" => false,
        "developer" => matches!(method, "team.set" | "policy.set" | "cost.budget.set" | "approval.decide"),
        "reviewer" => !matches!(method, "verify.run" | "approval.decide" | "approval.poll" | "health.check" | "intent.parse"),
        "client" => method != "approval.decide",
        _ => true,
    };

    if denied {
        Err(format!(
            "Your role ({role}) cannot do this ({method}). Ask the owner, or switch to your own name in Settings → Team."
        ))
    } else {
        Ok(())
    }
}

/// Whether this person may decide this approval: a client only their own sign-off, a production deploy
/// never by the person who asked for it (four eyes) - unless the team is one person.
pub fn may_decide(store: &Store, approval: &Value) -> Result<(), String> {
    let role = current_role(store);
    let solo = members(store).len() <= 1;
    let me = store.setting("team.current").ok().flatten().unwrap_or_default();

    match (role.as_str(), approval["kind"].as_str().unwrap_or_default()) {
        ("client", "client") => Ok(()),
        ("client", _) => Err("A client can answer only a sign-off that was sent to them.".into()),
        ("developer", _) => Err("A developer cannot approve; an owner or a reviewer can.".into()),
        (_, "deploy") if !solo && approval["requestedBy"].as_str() == Some(me.as_str()) => {
            Err("A production deploy needs a second person: the one who asked cannot also approve it.".into())
        }
        _ => Ok(()),
    }
}

pub fn to_json(store: &Store) -> Value {
    json!({
        "members": members(store),
        "current": store.setting("team.current").ok().flatten(),
        "role": current_role(store),
        "roles": ROLES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_allow_what_the_table_says() {
        assert!(allowed("owner", "team.set").is_ok());
        assert!(allowed("developer", "engine.start").is_ok());
        assert!(allowed("developer", "policy.set").is_err());
        assert!(allowed("reviewer", "engine.start").is_err());
        assert!(allowed("reviewer", "verify.run").is_ok());
        assert!(allowed("client", "fs.write").is_err());
        assert!(allowed("client", "site.list").is_ok());
        assert!(allowed("client", "kill.all").is_ok(), "anyone may stop everything");
    }

    #[test]
    fn a_production_deploy_needs_four_eyes_in_a_team() {
        let store = Store::in_memory().unwrap();

        store.set_setting("team.members", r#"[{"name":"Owner","role":"owner"},{"name":"Rina","role":"reviewer"}]"#).unwrap();
        store.set_setting("team.current", "Owner").unwrap();

        let mine = json!({ "kind": "deploy", "requestedBy": "Owner" });
        let theirs = json!({ "kind": "deploy", "requestedBy": "Rina" });

        assert!(may_decide(&store, &mine).is_err());
        assert!(may_decide(&store, &theirs).is_ok());

        store.set_setting("team.current", "Nobody").unwrap();
        assert_eq!(current_role(&store), "client", "an unknown name gets the least");
    }

    #[test]
    fn no_team_is_one_owner() {
        let store = Store::in_memory().unwrap();

        assert_eq!(current_role(&store), "owner");
        assert!(may_decide(&store, &json!({ "kind": "deploy", "requestedBy": "" })).is_ok());
    }
}
