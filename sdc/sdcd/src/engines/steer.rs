//! Messages sent into a turn that is still running - for **every** engine (0.12.5; every engine since 1.0.9).
//!
//! The reports: *"claude code a build howyar moddai ami jamon onno comamnd dile o seta pore adjust kore nisse
//! amader sdc taw ai rokom hote hobe"* (0.12.5), and *"eta shudhu claude ba codex er jonno na - deepseek api ba
//! je kono api ba cli add kori na keno, ei dhap ta continuously cholte thakbe"* (1.0.9). A message typed while a
//! turn runs goes **into** that turn and changes it; it never waits in a queue for the turn to end.
//!
//! # The rule - do not weaken it
//!
//! Every turn, whatever its engine, runs through [`drive`] (`sdcp::methods::run_turn`). `drive` opens the turn's
//! mailbox for as long as the turn runs, so `engine.steer` accepts the person's words for any engine, and:
//!
//! - an engine that can take words **between its own steps** says so with [`go_live`] and reads them with
//!   [`take`]: SDC's agent loop (every API and local model in a folder) hands them to the model after its next
//!   tool call, and Claude Code gets them on its stdin (`cli::Steering`);
//! - any other engine - Codex, Gemini, a plain API or Ollama chat, and **any engine added later** - is stopped
//!   the moment a message arrives ([`interrupting`] is what its stop check reads; `Engine::cancel` ends a CLI's
//!   process), and the same turn goes on at once with the message and everything said so far;
//! - a message that lands just as an engine finishes is not lost and not queued: the same turn goes on with it.
//!
//! A new engine needs nothing to get this. To take messages between steps itself, it calls `go_live` and `take`;
//! to stop sooner, its stop check also reads `interrupting`. `tests::every_engine_*` pin all of it down - if one
//! of them fails, the change is what is wrong, not the test.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::engines::{Engine, EngineEvent, EventSink, Message, Prompt, ResumeRef, Role};

#[derive(Default)]
struct Mailbox {
    messages: Vec<String>,
    /// The engine reads the messages itself, between its steps.
    live: bool,
    /// The engine is being stopped so the turn can go on with a message.
    interrupting: bool,
}

fn mailboxes() -> &'static Mutex<HashMap<String, Mailbox>> {
    static MAILBOXES: OnceLock<Mutex<HashMap<String, Mailbox>>> = OnceLock::new();

    MAILBOXES.get_or_init(Default::default)
}

fn with<T>(turn_id: &str, read: impl FnOnce(&mut Mailbox) -> T) -> Option<T> {
    mailboxes().lock().ok().and_then(|mut map| map.get_mut(turn_id).map(read))
}

/// The turn takes messages from now on. Dropping the guard closes the mailbox.
pub fn open(turn_id: &str) -> Inbox {
    if let Ok(mut map) = mailboxes().lock() {
        map.insert(turn_id.to_string(), Mailbox::default());
    }

    Inbox { turn_id: turn_id.to_string() }
}

/// Drops a message into a running turn. `false` only when the turn is not running.
pub fn push(turn_id: &str, text: &str) -> bool {
    let text = text.trim();

    !text.is_empty() && with(turn_id, |mailbox| mailbox.messages.push(text.to_string())).is_some()
}

/// Everything that arrived since the last look, oldest first.
pub fn take(turn_id: &str) -> Vec<String> {
    with(turn_id, |mailbox| std::mem::take(&mut mailbox.messages)).unwrap_or_default()
}

/// The engine reads this turn's messages itself, between its steps - it is not stopped for them.
pub fn go_live(turn_id: &str) {
    with(turn_id, |mailbox| mailbox.live = true);
}

/// The engine turned out not to read messages after all (an older `claude` refused the flags): from now on it is
/// stopped for them like any other.
pub fn stand_down(turn_id: &str) {
    with(turn_id, |mailbox| mailbox.live = false);
}

/// Whether the engine of this turn is being stopped so the turn can go on with the person's message. An engine's
/// stop check reads it next to `cancel::requested`.
pub fn interrupting(turn_id: &str) -> bool {
    with(turn_id, |mailbox| mailbox.interrupting).unwrap_or(false)
}

/// The mailbox of a running turn, held by [`drive`].
pub struct Inbox {
    turn_id: String,
}

impl Inbox {
    /// Everything that arrived since the last look, oldest first.
    pub fn take(&self) -> Vec<String> {
        take(&self.turn_id)
    }
}

impl Drop for Inbox {
    fn drop(&mut self) {
        if let Ok(mut map) = mailboxes().lock() {
            map.remove(&self.turn_id);
        }
    }
}

/// The words as the model reads them: said to be the person's, sent while it worked.
pub fn as_message(texts: &[String]) -> String {
    format!(
        "[The person sent this while you were working - take it into account from now on, adjusting or changing course as it asks:]\n{}",
        texts.join("\n\n")
    )
}

/// How often a running turn is looked at for a message its engine does not read itself.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// Runs one turn on `engine`, taking the person's messages into it while it runs - see the module doc.
///
/// The engine's own ending (`Done`/`Failed`) is held until it is clear no message is waiting: when one is, the
/// same turn goes on with it instead, so the window sees one turn that changed course, not two.
pub async fn drive(engine: Arc<dyn Engine>, mut prompt: Prompt, sink: EventSink) {
    let turn_id = prompt.turn_id.clone();
    let _mailbox = open(&turn_id);

    loop {
        with(&turn_id, |mailbox| *mailbox = Mailbox { messages: std::mem::take(&mut mailbox.messages), ..Mailbox::default() });

        let ending: Arc<Mutex<Option<EngineEvent>>> = Arc::default();
        let said: Arc<Mutex<String>> = Arc::default();
        let session: Arc<Mutex<Option<String>>> = Arc::default();
        let inner = {
            let (ending, said, session, sink) = (ending.clone(), said.clone(), session.clone(), sink.clone());

            EventSink::new(move |event| match event {
                event if event.is_terminal() => {
                    if let Ok(mut ending) = ending.lock() {
                        ending.get_or_insert(event);
                    }
                }
                EngineEvent::Delta(text) => {
                    if let Ok(mut said) = said.lock() {
                        said.push_str(&text);
                    }

                    sink.send(EngineEvent::Delta(text));
                }
                EngineEvent::SessionRef(id) => {
                    if let Ok(mut session) = session.lock() {
                        *session = Some(id.clone());
                    }

                    sink.send(EngineEvent::SessionRef(id));
                }
                event => sink.send(event),
            })
        };

        /* An engine that does not read messages itself is stopped the moment one arrives. */
        let watcher = {
            let (engine, turn_id) = (engine.clone(), turn_id.clone());

            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(LOOK_EVERY).await;

                    let stop = with(&turn_id, |mailbox| {
                        let stop = !mailbox.live && !mailbox.interrupting && !mailbox.messages.is_empty();

                        mailbox.interrupting |= stop;
                        stop
                    });

                    match stop {
                        Some(true) => {
                            engine.cancel(&turn_id).await;

                            return;
                        }
                        Some(false) => {}
                        None => return,
                    }
                }
            })
        };

        engine.start(prompt.clone(), &inner).await;
        watcher.abort();

        let ending = ending.lock().ok().and_then(|mut ending| ending.take());
        let more = if crate::engines::cancel::requested(&turn_id) { Vec::new() } else { take(&turn_id) };

        if more.is_empty() {
            if let Some(ending) = ending {
                sink.send(ending);
            }

            return;
        }

        let interrupted = interrupting(&turn_id);
        let said = said.lock().map(|said| said.clone()).unwrap_or_default();
        let session = session.lock().ok().and_then(|session| session.clone());

        for text in &more {
            sink.send(EngineEvent::Steered(text.clone()));
        }

        if !said.trim().is_empty() {
            sink.send(EngineEvent::Delta("\n\n".to_string()));
        }

        prompt = continued(prompt, &said, interrupted, session, &more);
    }
}

/// The prompt that carries a turn on after the person's messages: what was asked and said so far, then the words.
fn continued(mut prompt: Prompt, said: &str, interrupted: bool, session: Option<String>, more: &[String]) -> Prompt {
    let mut so_far = vec![Message::user(prompt.text.clone())];
    let answer = match (said.trim().is_empty(), interrupted) {
        (true, true) => "[Stopped before answering - the person sent a message.]".to_string(),
        (true, false) => String::new(),
        (false, true) => format!("{said}\n\n[Stopped here - the person sent a message.]"),
        (false, false) => said.to_string(),
    };

    if !answer.is_empty() {
        so_far.push(Message { role: Role::Assistant, text: answer });
    }

    /* A CLI that named its conversation remembers this run itself; otherwise it is told what it missed. */
    prompt.resume = match (session, prompt.resume.take()) {
        (Some(id), _) => Some(ResumeRef { id, unseen: Vec::new() }),
        (None, Some(mut resume)) => {
            resume.unseen.extend(so_far.iter().cloned());
            Some(resume)
        }
        (None, None) => None,
    };
    prompt.history.extend(so_far);
    prompt.text = as_message(more);
    prompt.images.clear();
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::{EngineStatus, Recorder};
    use async_trait::async_trait;

    #[test]
    fn a_message_reaches_an_open_turn_only() {
        assert!(!push("turn-steer-1", "stop"));

        let inbox = open("turn-steer-1");

        assert!(push("turn-steer-1", "use tabs, not spaces"));
        assert!(push("turn-steer-1", "and name it utils.ts"));
        assert!(!push("turn-steer-1", "   "));
        assert_eq!(inbox.take(), vec!["use tabs, not spaces".to_string(), "and name it utils.ts".to_string()]);
        assert!(inbox.take().is_empty());

        drop(inbox);

        assert!(!push("turn-steer-1", "too late"));
    }

    /// An engine that answers slowly and knows nothing about messages - the shape of Codex, Gemini, a plain API
    /// chat, Ollama, or an engine nobody has written yet. It stops when asked, the way a killed CLI does.
    struct Plain {
        prompts: Mutex<Vec<Prompt>>,
        steps: usize,
    }

    #[async_trait]
    impl Engine for Plain {
        fn id(&self) -> &'static str {
            "plain"
        }

        async fn start(&self, prompt: Prompt, sink: &EventSink) {
            self.prompts.lock().unwrap().push(prompt.clone());

            for step in 0..self.steps {
                if interrupting(&prompt.turn_id) {
                    sink.send(EngineEvent::Failed("stopped".into()));
                    return;
                }

                sink.send(EngineEvent::Delta(format!("part {step}. ")));
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            sink.send(EngineEvent::Done { summary: "Done".into(), meta: String::new(), pass: Some(true) });
        }

        async fn cancel(&self, _turn_id: &str) -> bool {
            true
        }

        fn status(&self, _turn_id: &str) -> EngineStatus {
            EngineStatus::Idle
        }
    }

    fn prompt(turn_id: &str) -> Prompt {
        Prompt {
            session_id: "s1".into(),
            turn_id: turn_id.into(),
            text: "write the page".into(),
            model: "any".into(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            autonomy: Default::default(),
            resume: None,
            images: Vec::new(),
            effort: None,
        }
    }

    fn endings(events: &[EngineEvent]) -> Vec<&EngineEvent> {
        events.iter().filter(|event| event.is_terminal()).collect()
    }

    /// 1.0.9, the rule itself: an engine that knows nothing about messages is stopped when one arrives, and the
    /// **same** turn goes on with it at once - not after the turn, not as a queued next turn.
    #[tokio::test]
    async fn every_engine_takes_a_message_mid_turn_even_one_that_knows_nothing_of_it() {
        let engine = Arc::new(Plain { prompts: Mutex::default(), steps: 40 });
        let recorder = Recorder::new();
        let turn = tokio::spawn(drive(engine.clone(), prompt("turn-steer-plain"), recorder.sink()));

        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        assert!(push("turn-steer-plain", "make it blue"), "a running turn of any engine takes a message");

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while engine.prompts.lock().unwrap().len() < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the turn did not go on with the message while it ran");

        /* It went on long before the first run's 4 seconds were up. */
        let second = engine.prompts.lock().unwrap()[1].clone();

        assert!(second.text.contains("make it blue"));
        assert!(second.history.iter().any(|message| message.text == "write the page"), "the continuation knows what was asked");
        assert!(second.history.iter().any(|message| message.role == Role::Assistant && message.text.contains("part 0.")), "and what was said");

        turn.abort();
    }

    /// A message that lands as the engine finishes is not queued for later: the same turn goes on with it, and
    /// the turn ends once.
    #[tokio::test]
    async fn every_engine_goes_on_with_a_message_that_arrived_as_it_finished() {
        let engine = Arc::new(Plain { prompts: Mutex::default(), steps: 1 });
        let recorder = Recorder::new();
        let mailbox_ready = tokio::spawn(drive(engine.clone(), prompt("turn-steer-late"), recorder.sink()));

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(push("turn-steer-late", "and a footer"));
        tokio::time::timeout(std::time::Duration::from_secs(5), mailbox_ready).await.expect("the turn hung").unwrap();

        let events = recorder.events();

        assert_eq!(engine.prompts.lock().unwrap().len(), 2);
        assert!(events.contains(&EngineEvent::Steered("and a footer".into())));
        assert_eq!(endings(&events).len(), 1, "one turn, one ending: {events:?}");
        assert!(!push("turn-steer-late", "after the end"), "a finished turn takes nothing more");
    }

    /// An engine that reads the messages itself (the agent loop, Claude Code) is never stopped for them.
    #[tokio::test]
    async fn every_engine_that_reads_messages_itself_is_left_running() {
        struct Live;

        #[async_trait]
        impl Engine for Live {
            fn id(&self) -> &'static str {
                "live"
            }

            async fn start(&self, prompt: Prompt, sink: &EventSink) {
                go_live(&prompt.turn_id);

                for _ in 0..10 {
                    for text in take(&prompt.turn_id) {
                        sink.send(EngineEvent::Steered(text));
                    }

                    assert!(!interrupting(&prompt.turn_id), "a live engine is not stopped");
                    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                }

                sink.send(EngineEvent::Done { summary: "Done".into(), meta: String::new(), pass: Some(true) });
            }

            async fn cancel(&self, _turn_id: &str) -> bool {
                panic!("a live engine must not be cancelled for a message")
            }

            fn status(&self, _turn_id: &str) -> EngineStatus {
                EngineStatus::Idle
            }
        }

        let recorder = Recorder::new();
        let turn = tokio::spawn(drive(Arc::new(Live), prompt("turn-steer-live"), recorder.sink()));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(push("turn-steer-live", "rename it"));
        tokio::time::timeout(std::time::Duration::from_secs(5), turn).await.expect("the turn hung").unwrap();

        let events = recorder.events();

        assert_eq!(events.iter().filter(|event| **event == EngineEvent::Steered("rename it".into())).count(), 1);
        assert_eq!(endings(&events).len(), 1);
    }

    /// The guard on the rule: every turn of every engine is run by `drive`. A dispatcher that calls `engine.start`
    /// directly again puts every engine but the live ones back to queueing the person's messages.
    #[test]
    fn every_turn_of_every_engine_goes_through_drive() {
        let dispatcher = include_str!("../sdcp/methods.rs");

        assert!(
            dispatcher.contains("crate::engines::steer::drive(engine, prompt, sink).await"),
            "sdcp::methods::run_turn must start the engine through engines::steer::drive - see that module's doc"
        );
    }

    #[test]
    fn a_continued_cli_turn_resumes_its_own_conversation() {
        let next = continued(prompt("turn-steer-resume"), "half", true, Some("thread-9".into()), &["stop".into()]);

        assert_eq!(next.resume, Some(ResumeRef { id: "thread-9".into(), unseen: Vec::new() }));
        assert!(next.text.contains("stop"));
    }
}
