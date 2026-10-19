use super::*;
use serde_json::json;

fn call(daemon: &Daemon, method: &str, params: Value) -> Result<Value, ErrorObject> {
    let state = daemon.state.clone();
    let out: Arc<dyn Notifier> = Arc::new(crate::sdcp::notifications::ChannelNotifier::new(state.events.clone(), state.fanout.clone()));
    let envelope = Envelope { v: crate::SDCP_VERSION.to_string(), id: "t-1".to_string(), method: method.to_string(), params: params.as_object().cloned().unwrap_or_default(), host_id: None };

    daemon.dispatch(&envelope, out)
}

/// P1-7: the app's memory panel and the agent's prompt read and write one table.
#[test]
fn the_memory_methods_list_add_pin_drop_and_clear_a_tasks_notes() {
    let state = crate::DaemonState::bootstrap(Some(std::path::PathBuf::from(":memory:"))).expect("a daemon");
    let daemon = Daemon::new(state);

    let added = call(&daemon, "memory.update", json!({ "task": "chat-1", "action": "add", "kind": "decision", "text": "the cache is in `store/cache.rs`", "source": "store/cache.rs", "quote": "pub struct Cache" })).unwrap();

    assert_eq!(added["changed"], true);
    assert_eq!(added["notes"][0]["text"], "the cache is in `store/cache.rs`");

    let listed = call(&daemon, "memory.list", json!({ "task": "chat-1" })).unwrap();

    assert_eq!(listed["notes"].as_array().unwrap().len(), 1);
    assert!(listed["block"].as_str().unwrap().contains("## This task, so far"), "{}", listed["block"]);

    assert_eq!(call(&daemon, "memory.update", json!({ "task": "chat-1", "action": "pin", "seq": 1 })).unwrap()["notes"][0]["pinned"], true);
    assert_eq!(call(&daemon, "memory.update", json!({ "task": "chat-1", "action": "unpin", "seq": 1 })).unwrap()["notes"][0]["pinned"], false);
    assert_eq!(call(&daemon, "memory.update", json!({ "task": "chat-1", "action": "drop", "seq": 1 })).unwrap()["changed"], true);
    assert_eq!(call(&daemon, "memory.list", json!({ "task": "chat-1" })).unwrap()["notes"], json!([]));

    /* a bad kind, a missing seq and an unknown action are refused with a sentence, not a panic */
    assert!(call(&daemon, "memory.update", json!({ "task": "chat-1", "action": "add", "kind": "poem", "text": "x" })).is_err());
    assert!(call(&daemon, "memory.update", json!({ "task": "chat-1", "action": "pin" })).unwrap_err().message.contains("`seq` is required"));
    assert!(call(&daemon, "memory.update", json!({ "task": "chat-1", "action": "erase" })).unwrap_err().message.contains("add, pin, unpin, drop or clear"));
    assert_eq!(call(&daemon, "memory.update", json!({ "task": "chat-2", "action": "clear" })).unwrap()["changed"], false);
}
