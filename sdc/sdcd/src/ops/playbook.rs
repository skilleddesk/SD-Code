//! **Playbooks** (1.0 in the plan): the same steps on many sites - update the plugins everywhere, clear
//! every cache, rotate a header - each site with its own backup, its own health check and its own
//! rollback, because a playbook run on a site *is* a Safe Deploy whose steps are the playbook's.
//!
//! A playbook step is a command (run on each site) or a prompt (a chat the window opens on each site, so
//! an AI step still goes through the Intent Engine, the checkpoints and Verify).

use serde_json::{json, Value};

use crate::store::sqlite::Store;

pub fn all(store: &Store) -> Vec<Value> {
    store
        .setting("playbooks")
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
        .unwrap_or_else(defaults)
}

/// The playbooks a new install starts with - common agency chores, all editable.
pub fn defaults() -> Vec<Value> {
    vec![
        json!({
            "id": "wp-updates",
            "name": "WordPress: update plugins and themes",
            "steps": [
                { "kind": "command", "text": "wp plugin update --all --path=." },
                { "kind": "command", "text": "wp theme update --all --path=." },
                { "kind": "command", "text": "wp cache flush --path=." }
            ]
        }),
        json!({
            "id": "wp-core",
            "name": "WordPress: update core (minor versions)",
            "steps": [
                { "kind": "command", "text": "wp core update --minor --path=." },
                { "kind": "command", "text": "wp core update-db --path=." }
            ]
        }),
        json!({
            "id": "security-review",
            "name": "Ask an AI for a security review",
            "steps": [
                { "kind": "prompt", "text": "Review this site's code for security problems (injection, XSS, secrets, outdated dependencies). List them with file:line and a fix. Do not change anything yet." }
            ]
        }),
    ]
}

pub fn save(store: &Store, playbook: &Value) -> anyhow::Result<Vec<Value>> {
    let mut list = all(store);
    let id = playbook["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("pb-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let mut clean = json!({
        "id": id,
        "name": playbook["name"].as_str().unwrap_or("Playbook"),
        "steps": playbook["steps"].as_array().cloned().unwrap_or_default().into_iter().filter(|step| {
            matches!(step["kind"].as_str(), Some("command" | "prompt")) && step["text"].as_str().is_some_and(|text| !text.trim().is_empty())
        }).collect::<Vec<_>>(),
    });

    if clean["steps"].as_array().map(Vec::is_empty).unwrap_or(true) {
        clean["steps"] = json!([]);
    }

    list.retain(|existing| existing["id"] != clean["id"]);
    list.push(clean);
    store.set_setting("playbooks", &serde_json::to_string(&list)?)?;

    Ok(list)
}

pub fn remove(store: &Store, id: &str) -> anyhow::Result<Vec<Value>> {
    let mut list = all(store);

    list.retain(|existing| existing["id"] != id);
    store.set_setting("playbooks", &serde_json::to_string(&list)?)?;

    Ok(list)
}

/// A playbook's command steps (what each site's deploy runs) and its prompt steps (what the window opens).
pub fn split(playbook: &Value) -> (Vec<String>, Vec<String>) {
    let mut commands = Vec::new();
    let mut prompts = Vec::new();

    for step in playbook["steps"].as_array().cloned().unwrap_or_default() {
        let text = step["text"].as_str().unwrap_or_default().to_string();

        match step["kind"].as_str() {
            Some("command") => commands.push(text),
            Some("prompt") => prompts.push(text),
            _ => {}
        }
    }

    (commands, prompts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playbooks_are_saved_and_split_into_commands_and_prompts() {
        let store = Store::in_memory().unwrap();

        assert_eq!(all(&store).len(), 3, "the defaults");

        let list = save(&store, &json!({ "name": "Clear caches", "steps": [{ "kind": "command", "text": "wp cache flush" }, { "kind": "prompt", "text": "check the homepage" }, { "kind": "nonsense", "text": "x" }] })).unwrap();
        let saved = list.iter().find(|playbook| playbook["name"] == "Clear caches").unwrap();
        let (commands, prompts) = split(saved);

        assert_eq!(commands, ["wp cache flush"]);
        assert_eq!(prompts, ["check the homepage"]);
        assert_eq!(remove(&store, saved["id"].as_str().unwrap()).unwrap().len(), 3);
    }
}
