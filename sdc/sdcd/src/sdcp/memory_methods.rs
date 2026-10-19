//! Working memory (P1-7): `memory.list` and `memory.update`. The app's memory panel reads the first and writes the second (add, pin,
//! unpin, drop, clear) - the same table the agent's prompt reads, so the person and the model look at one thing.

use super::*;

impl Daemon {
    /// The methods this module owns; `_` passes the envelope on to the next link in the chain.
    pub(super) fn dispatch_memory(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            "memory.list" => {
                let notes = self.store().notes(&envelope.require_str("task")?).map_err(ErrorObject::internal)?;
                let cap = envelope.opt_i64("cap").unwrap_or(4_000).clamp(200, 40_000) as usize;

                Ok(json!({
                    "notes": notes.iter().map(crate::sme::memory::as_json).collect::<Vec<_>>(),
                    "block": crate::sme::memory::render(&notes, cap),
                }))
            }
            "memory.update" => self.memory_update(envelope),
            _ => self.dispatch_fleet(envelope, out),
        }
    }

    fn memory_update(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let task = envelope.require_str("task")?;
        let action = envelope.require_str("action")?;
        let seq = envelope.opt_i64("seq");

        let changed = match action.as_str() {
            "add" => {
                let note = crate::sme::memory::from_tool(&json!({
                    "kind": envelope.opt_str("kind").unwrap_or_else(|| "decision".into()),
                    "text": envelope.opt_str("text").unwrap_or_default(),
                    "source": envelope.opt_str("source").unwrap_or_default(),
                    "quote": envelope.opt_str("quote").unwrap_or_default(),
                    "pin": envelope.opt_bool("pinned"),
                }))
                .map_err(ErrorObject::bad_request)?;

                self.store().add_note(&task, &note.kind, &note.text, &note.source, &note.quote, note.pinned).map_err(ErrorObject::internal)?;

                true
            }
            "pin" | "unpin" => {
                self.store().set_note_pinned(&task, seq.ok_or_else(|| ErrorObject::bad_request("`seq` is required: the note to pin"))?, action == "pin").map_err(ErrorObject::internal)?;

                true
            }
            "drop" => self.store().drop_note(&task, seq.ok_or_else(|| ErrorObject::bad_request("`seq` is required: the note to drop"))?).map_err(ErrorObject::internal)?,
            "clear" => self.store().clear_notes(&task).map_err(ErrorObject::internal)? > 0,
            other => return Err(ErrorObject::bad_request(format!("unknown `action` `{other}`: add, pin, unpin, drop or clear"))),
        };

        let notes = self.store().notes(&task).map_err(ErrorObject::internal)?;

        Ok(json!({ "changed": changed, "notes": notes.iter().map(crate::sme::memory::as_json).collect::<Vec<_>>() }))
    }
}

#[cfg(test)]
#[path = "memory_methods/tests.rs"]
mod tests;
