//! **Recovery Ladder (P0-4, switch `padma.recovery`)**: an empty or contentless reply never ends the task. The rungs, in order:
//!
//! - the same request with a one-line nudge;
//! - thinking off;
//! - a tighter context (masking `k=1`);
//! - Best-of-N (only when `nr.best_of_n` is on);
//! - the next row of the rotation chain (only when `relay.auto` is on);
//! - the step is `blocked(reason)`.
//!
//! At most six rungs per step; each rung is written to the step trace with its reason.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    Nudge,
    ThinkingOff,
    TightContext,
    BestOfN,
    NextModel,
    Blocked,
}

impl Rung {
    pub fn name(self) -> &'static str {
        match self {
            Rung::Nudge => "nudge",
            Rung::ThinkingOff => "thinking off",
            Rung::TightContext => "tighter context",
            Rung::BestOfN => "best-of-n",
            Rung::NextModel => "next model",
            Rung::Blocked => "blocked",
        }
    }

    const ORDER: [Rung; 6] = [Rung::Nudge, Rung::ThinkingOff, Rung::TightContext, Rung::BestOfN, Rung::NextModel, Rung::Blocked];
}

/// What the ladder may use this turn.
#[derive(Debug, Clone, Copy, Default)]
pub struct Caps {
    pub best_of_n: bool,
    pub rotation: bool,
}

/// One rung taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub rung: Rung,
    pub number: u32,
    pub reason: String,
}

#[derive(Debug)]
pub struct Ladder {
    caps: Caps,
    at: usize,
    taken: Vec<Taken>,
}

pub const MAX_RUNGS: u32 = 6;

impl Ladder {
    pub fn new(caps: Caps) -> Self {
        Self { caps, at: 0, taken: Vec::new() }
    }

    /// The next rung for a step that failed to produce anything useful, and why. Rungs a turn cannot use are skipped; after the last
    /// one the answer is always `Blocked`, so the ladder cannot loop.
    pub fn next(&mut self, reason: &str) -> Rung {
        let rung = loop {
            let candidate = Rung::ORDER[self.at.min(Rung::ORDER.len() - 1)];

            if self.at < Rung::ORDER.len() - 1 {
                self.at += 1;
            }

            match candidate {
                Rung::BestOfN if !self.caps.best_of_n => continue,
                Rung::NextModel if !self.caps.rotation => continue,
                other => break other,
            }
        };

        self.taken.push(Taken { rung, number: self.taken.len() as u32 + 1, reason: reason.to_string() });

        rung
    }

    /// The step produced a tool call: the next failure starts from the first rung again.
    pub fn reset(&mut self) {
        self.at = 0;
    }

    /// Every rung taken in the turn, for Trust and the receipt.
    pub fn taken(&self) -> &[Taken] {
        &self.taken
    }

    pub fn used(&self) -> u32 {
        self.taken.len() as u32
    }

    pub fn last(&self) -> Option<&Taken> {
        self.taken.last()
    }
}

/// Writes one rung to the step trace.
pub fn trace_rung(turn_id: &str, session_id: &str, step: usize, model_key: &str, taken: &Taken) {
    crate::trace::record(crate::trace::Step {
        task_id: turn_id.to_string(),
        step_id: format!("{turn_id}.s{step}.r{}", taken.number),
        session_id: session_id.to_string(),
        step: step as u32,
        model_key: model_key.to_string(),
        mode: "recovery".to_string(),
        purpose: "recovery".to_string(),
        escalation_reason: Some(format!("rung {} {}: {}", taken.number, taken.rung.name(), taken.reason)),
        ..crate::trace::Step::default()
    });
}

/// The one-line nudge of rung 1 and the notes of the others, in the words the model is told.
pub fn note(rung: Rung) -> &'static str {
    match rung {
        Rung::Nudge => "[SDC: your last reply had no words and no tool call. Answer in plain words, or make the tool call that is needed.]",
        Rung::ThinkingOff => "[SDC: answer now in plain words, or make the tool call that is needed. Do not think first.]",
        Rung::TightContext => "[SDC: the earlier tool outputs were shortened to fit. Answer in plain words, or make the tool call that is needed.]",
        Rung::BestOfN | Rung::NextModel | Rung::Blocked => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rungs_come_in_order_and_the_unusable_ones_are_skipped() {
        let mut ladder = Ladder::new(Caps::default());

        assert_eq!(ladder.next("empty"), Rung::Nudge);
        assert_eq!(ladder.next("empty"), Rung::ThinkingOff);
        assert_eq!(ladder.next("empty"), Rung::TightContext);
        assert_eq!(ladder.next("empty"), Rung::Blocked);
    }

    #[test]
    fn all_six_rungs_are_used_when_the_turn_allows_them() {
        let mut ladder = Ladder::new(Caps { best_of_n: true, rotation: true });
        let rungs: Vec<Rung> = (0..6).map(|_| ladder.next("empty")).collect();

        assert_eq!(rungs, Rung::ORDER.to_vec());
        assert_eq!(ladder.used(), 6);
    }

    #[test]
    fn it_never_climbs_past_blocked() {
        let mut ladder = Ladder::new(Caps::default());

        for _ in 0..20 {
            ladder.next("empty");
        }

        assert_eq!(ladder.next("empty"), Rung::Blocked);
    }

    #[test]
    fn a_tool_call_starts_the_ladder_again_but_keeps_the_log() {
        let mut ladder = Ladder::new(Caps::default());

        ladder.next("empty");
        ladder.next("empty");
        ladder.reset();

        assert_eq!(ladder.next("empty"), Rung::Nudge);
        assert_eq!(ladder.taken().len(), 3);
        assert_eq!(ladder.last().map(|taken| taken.reason.as_str()), Some("empty"));
    }
}
