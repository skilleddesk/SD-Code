//! The Universal Intent Engine's SDCP methods (0.12): detect, parse into a Task Spec, confirm, compile,
//! the glossary, and voice.
//!
//! Parsing asks a model, so it answers at once with an `intentId` and pushes `IntentParsed` when the
//! reading is ready - the same pattern as Verify, so a slow model never blocks the window's other calls.
//! With no model to ask (or a model that answers something that is not a spec), the offline reading is
//! used and labelled `heuristic`, which the card shows as a question rather than as an answer.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use super::Daemon;
use crate::engines::{EngineEvent, Prompt, Recorder};
use crate::intent;
use crate::sdcp::envelope::{Envelope, ErrorObject};
use crate::sdcp::events::event;
use crate::sdcp::notifications::Notifier;

/// How long a reading may take before the offline one is used instead.
const PARSE_TIMEOUT: Duration = Duration::from_secs(75);

impl Daemon {
    pub(super) fn dispatch_intent(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        match envelope.method.as_str() {
            "intent.detect" => Ok(intent::detect(&envelope.require_str("text")?).to_json()),
            "intent.parse" => self.intent_parse(envelope, out),
            "intent.confirm" => self.intent_confirm(envelope, &*out),
            "intent.compile" => self.intent_compile(envelope),
            "intent.cancel" => {
                let id = envelope.require_str("intentId")?;

                if let Some(row) = self.store().intent(&id).map_err(ErrorObject::internal)? {
                    let _ = self.store().save_intent(&id, row["sessionId"].as_str(), row["text"].as_str().unwrap_or_default(), &row["spec"], "cancelled", row["corrections"].as_i64().unwrap_or(0));
                }

                Ok(json!({ "cancelled": true }))
            }
            "intent.stats" => self.store().intent_stats().map_err(ErrorObject::internal),
            "glossary.list" => {
                let scope = self.glossary_scope(envelope)?;

                Ok(json!({ "scope": scope, "terms": self.store().glossary(&scope).map_err(ErrorObject::internal)? }))
            }
            "glossary.set" => {
                let scope = envelope.opt_str("scope").map(Ok).unwrap_or_else(|| self.glossary_scope(envelope))?;

                self.store()
                    .set_glossary(&scope, &envelope.require_str("term")?, &envelope.opt_str("meaning").unwrap_or_default())
                    .map_err(ErrorObject::internal)?;

                Ok(json!({ "scope": scope, "terms": self.store().glossary(&scope).map_err(ErrorObject::internal)? }))
            }
            "voice.status" => Ok(intent::voice::status()),
            "voice.transcribe" => self.voice_transcribe(envelope, out),
            _ => self.dispatch_ops(envelope, out),
        }
    }

    /// A chat's glossary lives with its folder (`project:<root>`); a chat with none uses the global one.
    fn glossary_scope(&self, envelope: &Envelope) -> Result<String, ErrorObject> {
        Ok(match self.root_for(envelope)? {
            Some(root) => format!("project:{}", root.display()),
            None => "global".to_string(),
        })
    }

    /// Everything the compiler knows about where a chat works: its folder's rules, memory and map, the
    /// policy, the site bound to it, the glossary, the agency's style guide and how the person wants to be
    /// answered.
    pub(super) fn intent_context(&self, root: Option<&str>, remote: Option<&crate::ssh::Ssh>) -> intent::Context {
        let mut context = match (root, remote) {
            (Some(root), Some(ssh)) => intent::gather_remote(ssh, root),
            (Some(root), None) => intent::gather_local(std::path::Path::new(root)),
            _ => intent::Context { place: "this machine".into(), ..intent::Context::default() },
        };
        let scope = root.map(|root| format!("project:{root}")).unwrap_or_else(|| "global".to_string());

        context.glossary = self
            .store()
            .glossary(&scope)
            .unwrap_or_default()
            .iter()
            .filter_map(|row| Some((row["term"].as_str()?.to_string(), row["meaning"].as_str()?.to_string())))
            .collect();
        context.site = root.and_then(|root| {
            self.store().sites().ok()?.into_iter().find(|site| site["root"].as_str() == Some(root)).map(|site| {
                (site["name"].as_str().unwrap_or_default().to_string(), site["url"].as_str().unwrap_or_default().to_string())
            })
        });
        context.style_guide = self.store().setting("agency.styleGuide").ok().flatten();
        context.reply_style = self.store().setting("intent.replyStyle").ok().flatten().unwrap_or_else(|| "standard".into());

        context
    }

    fn intent_parse(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let text = envelope.require_str("text")?;
        let session = envelope.opt_str("sessionId");
        let detection = intent::detect(&text);
        let intent_id = format!("intent-{}", uuid::Uuid::new_v4().simple());
        let root = self.root_for(envelope)?.map(|root| root.display().to_string());
        let remote = self.remote_for(envelope)?;
        let context = self.intent_context(root.as_deref(), remote.as_ref());
        let fallback = intent::heuristic(&text, &detection, &context);

        self.store().save_intent(&intent_id, session.as_deref(), &text, &fallback, "parsing", 0).map_err(ErrorObject::internal)?;

        let engine_id = envelope.opt_str("engine").unwrap_or_else(|| "native_api".into());
        let model = envelope.opt_str("model").unwrap_or_default();
        let provider = envelope.opt_str("provider").filter(|id| !id.is_empty());
        let engine = self.state.engines.get(&engine_id);
        let state = self.state.clone();
        let prompt_text = intent::parse_prompt(&text, &detection, &context);
        let (id, words) = (intent_id.clone(), text.clone());
        /* A CLI engine answers where the chat lives, so a VPS chat whose CLI is only on the VPS can still
           read its own requests; an API model is a network call from here either way. */
        let cli_remote = if matches!(engine_id.as_str(), "claude_code" | "codex" | "gemini") { remote.clone() } else { None };

        tokio::spawn(async move {
            let (spec, note) = match engine {
                None => (fallback.clone(), Some(format!("`{engine_id}` is not an engine here; this is SDC's offline reading."))),
                Some(engine) => {
                    let recorder = Recorder::new();
                    let prompt = Prompt {
                        session_id: session.clone().unwrap_or_else(|| "intent".into()),
                        turn_id: format!("{id}-parse"),
                        text: prompt_text,
                        model,
                        provider,
                        history: Vec::new(),
                        project_root: None,
                        remote: cli_remote,
                        autonomy: crate::agent::gate::Autonomy::Ask,
                        resume: None,
                        images: Vec::new(),
                        effort: None,
                    };
                    let finished = tokio::time::timeout(PARSE_TIMEOUT, engine.start(prompt, &recorder.sink())).await;
                    let events = recorder.events();
                    let answer: String = events
                        .iter()
                        .filter_map(|event| match event {
                            EngineEvent::Delta(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();

                    match intent::read_spec(&answer, &detection) {
                        Some(spec) => (spec, None),
                        None => {
                            let reason = events
                                .iter()
                                .find_map(|event| match event {
                                    EngineEvent::Failed(reason) => Some(reason.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| {
                                    if finished.is_err() {
                                        "the model took too long".to_string()
                                    } else {
                                        "the model did not answer in the requested shape".to_string()
                                    }
                                });

                            (fallback.clone(), Some(format!("SDC's offline reading ({reason}).")))
                        }
                    }
                }
            };

            let _ = state.store.save_intent(&id, session.as_deref(), &words, &spec, "parsed", 0);

            out.push(
                json!({
                    "type": "IntentParsed",
                    "intentId": id,
                    "sessionId": session,
                    "text": words,
                    "detection": detection.to_json(),
                    "spec": spec,
                    "note": note,
                }),
                session.clone(),
                None,
            );
        });

        Ok(json!({ "intentId": intent_id, "detection": intent::detect(&text).to_json() }))
    }

    /// The person's answer to the card: the spec as they left it. A term they explained ("ghor means page")
    /// goes into the glossary, so the next reading does not ask again.
    fn intent_confirm(&self, envelope: &Envelope, out: &dyn Notifier) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("intentId")?;
        let row = self.store().intent(&id).map_err(ErrorObject::internal)?.ok_or_else(|| ErrorObject::not_found(format!("no intent `{id}`")))?;
        let text = row["text"].as_str().unwrap_or_default().to_string();
        let detection = intent::detect(&text);
        let before = row["spec"].clone();
        let edited = envelope.params.get("spec").cloned().unwrap_or_else(|| before.clone());
        let spec = intent::normalise(&edited, &detection, before["source"].as_str().unwrap_or("model"));
        let changed = |pointer: &str| before.pointer(pointer) != spec.pointer(pointer);
        let terms: Vec<(String, String)> = envelope
            .params
            .get("glossary")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|term| Some((term["term"].as_str()?.trim().to_string(), term["meaning"].as_str()?.trim().to_string())))
            .filter(|(term, meaning)| !term.is_empty() && !meaning.is_empty())
            .collect();
        let corrections = ["/goal/value", "/target/value", "/acceptance"].iter().filter(|pointer| changed(pointer)).count() + terms.len();
        let scope = self.glossary_scope(envelope)?;

        for (term, meaning) in &terms {
            let _ = self.store().set_glossary(&scope, term, meaning);
        }

        self.store()
            .save_intent(&id, row["sessionId"].as_str(), &text, &spec, "confirmed", corrections as i64)
            .map_err(ErrorObject::internal)?;
        out.push(event::intent_confirmed(row["sessionId"].as_str(), &id, corrections as i64), row["sessionId"].as_str().map(str::to_string), None);

        Ok(json!({ "intentId": id, "spec": spec, "corrections": corrections, "glossaryScope": scope }))
    }

    /// The prompt a confirmed intent becomes for one engine - what "see the compiled prompt" shows.
    fn intent_compile(&self, envelope: &Envelope) -> Result<Value, ErrorObject> {
        let id = envelope.require_str("intentId")?;
        let engine = envelope.opt_str("engine").unwrap_or_else(|| "claude_code".into());

        Ok(json!({ "engine": engine, "prompt": self.compiled_prompt(&id, &engine, envelope)? }))
    }

    /// A confirmed intent, compiled for `engine` in the chat's own context. `engine.start { intentId }` sends
    /// exactly this.
    pub(super) fn compiled_prompt(&self, intent_id: &str, engine: &str, envelope: &Envelope) -> Result<String, ErrorObject> {
        let row = self
            .store()
            .intent(intent_id)
            .map_err(ErrorObject::internal)?
            .ok_or_else(|| ErrorObject::not_found(format!("no intent `{intent_id}`")))?;
        let root = self.root_for(envelope)?.map(|root| root.display().to_string());
        let remote = self.remote_for(envelope)?;
        let context = self.intent_context(root.as_deref(), remote.as_ref());

        Ok(intent::compile(engine, &row["spec"], row["text"].as_str().unwrap_or_default(), &context))
    }

    fn voice_transcribe(&self, envelope: &Envelope, out: Arc<dyn Notifier>) -> Result<Value, ErrorObject> {
        let audio = base64::engine::general_purpose::STANDARD
            .decode(envelope.require_str("audio")?.trim())
            .map_err(|error| ErrorObject::bad_request(format!("the recording is not base64: {error}")))?;
        let mime = envelope.opt_str("mime").unwrap_or_else(|| "audio/wav".into());
        let language = envelope.opt_str("language").filter(|language| !language.is_empty());
        let request = format!("voice-{}", uuid::Uuid::new_v4().simple());
        let id = request.clone();
        let session = envelope.opt_str("sessionId");

        tokio::task::spawn_blocking(move || {
            let result = intent::voice::transcribe(&audio, &mime, language.as_deref());
            let payload = match result {
                Ok(value) => json!({
                    "type": "VoiceTranscribed",
                    "requestId": id,
                    "text": value["text"],
                    "engine": value["engine"],
                    "local": value["local"],
                    "detection": intent::detect(value["text"].as_str().unwrap_or_default()).to_json(),
                    "error": Value::Null,
                }),
                Err(error) => json!({ "type": "VoiceTranscribed", "requestId": id, "text": "", "engine": Value::Null, "local": false, "error": error }),
            };

            out.push(payload, session, None);
        });

        Ok(json!({ "requestId": request }))
    }
}
