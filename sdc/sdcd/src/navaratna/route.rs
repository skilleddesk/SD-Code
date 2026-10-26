//! **N8-1 - the capability tag and the sandwich router** (switch `nr.tag_route`, off).
//!
//! Each planned step gets one tag - `simple`, `code`, `hard` or `architecture` - from deterministic rules first (what the step does, how many
//! files it touches, whether it carries a Blueprint marker, how often it already failed); the local model is only the tie-breaker between `code`
//! and `hard`. The tag decides the route:
//!
//! | tag | route |
//! | --- | --- |
//! | `simple`, `code` | the local model |
//! | `hard` | the local model, with Best-of-N allowed early (the relay is still the last resort) |
//! | `architecture` | the chain's architect row, once - only with the relay consent, a privacy that allows it and no model picked by hand |
//!
//! With a model picked by hand (Manual Pin), `privacy = "local-only"`, or the relay off, nothing is routed anywhere: every step stays local. The
//! tag and the reason are recorded with the step.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Tag {
    Simple,
    Code,
    Hard,
    Architecture,
}

impl Tag {
    pub fn name(self) -> &'static str {
        match self {
            Tag::Simple => "simple",
            Tag::Code => "code",
            Tag::Hard => "hard",
            Tag::Architecture => "architecture",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Edit,
    Create,
    Test,
    Docs,
    Config,
    Plan,
}

#[derive(Debug, Clone)]
pub struct Facts<'a> {
    pub kind: Kind,
    pub files_touched: usize,
    /// The step creates the project's skeleton: folder tree, data models, interface contracts (a Blueprint marker).
    pub blueprint_marker: bool,
    /// Times the step already failed.
    pub failed_attempts: u32,
    pub text: &'a str,
}

/// Words that make a step hard whatever its size.
const HARD_WORDS: &[&str] = &["algorithm", "concurrency", "concurrent", "race condition", "deadlock", "cryptograph", "encrypt", "security", "authentication", "authorization", "optimi", "parser", "scheduler", "distributed", "transaction", "migration", "lock-free", "memory leak"];

/// The tag and the reason in a sentence. `tie` is asked only when the rules cannot choose between `code` and `hard`.
pub fn tag(facts: &Facts<'_>, tie: Option<&mut dyn FnMut() -> Tag>) -> (Tag, String) {
    let lower = facts.text.to_lowercase();

    if facts.blueprint_marker || (facts.kind == Kind::Plan && facts.files_touched >= 3) {
        return (Tag::Architecture, if facts.blueprint_marker { "the step lays out the project's skeleton (Blueprint marker)".into() } else { "a plan over several files".into() });
    }

    if facts.failed_attempts >= 2 {
        return (Tag::Hard, format!("the step already failed {} times", facts.failed_attempts));
    }

    if let Some(word) = HARD_WORDS.iter().find(|word| lower.contains(**word)) {
        return (Tag::Hard, format!("the step mentions `{}`", word.trim_end_matches(['o', 'i'])));
    }

    if facts.files_touched >= 6 {
        return (Tag::Hard, format!("the step touches {} files", facts.files_touched));
    }

    if matches!(facts.kind, Kind::Read | Kind::Docs | Kind::Config) || (facts.files_touched <= 1 && facts.text.len() < 160 && facts.kind != Kind::Create && facts.kind != Kind::Test) {
        return (Tag::Simple, "reading, docs, configuration or a one-file change".into());
    }

    /* between 3 and 5 files with no other signal: the rules cannot say */
    if (3..6).contains(&facts.files_touched) || facts.failed_attempts == 1 {
        if let Some(ask) = tie {
            let answer = ask();

            if matches!(answer, Tag::Code | Tag::Hard) {
                return (answer, "the rules were undecided; the local model broke the tie".into());
            }
        }
    }

    (Tag::Code, "ordinary implementation".into())
}

/// What is allowed to be done outside the local model.
#[derive(Debug, Clone, Copy)]
pub struct Mode {
    /// The person picked a model by hand.
    pub manual_pin: bool,
    /// `privacy = "local-only"` for the project.
    pub local_only: bool,
    /// `relay.auto`: the consent.
    pub relay_auto: bool,
    /// The chain has a row that can be the architect.
    pub architect_row: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The local model does it.
    Local,
    /// The local model, with Best-of-N allowed before the first failure is spent.
    LocalBestOfN,
    /// The architect row of the chain, once.
    Architect,
}

/// The route for a tag in a mode. With the switch off everything is local.
pub fn route(tag: Tag, mode: &Mode) -> Route {
    if !crate::v3::on("nr.tag_route") {
        return Route::Local;
    }

    match tag {
        Tag::Simple | Tag::Code => Route::Local,
        Tag::Hard => Route::LocalBestOfN,
        Tag::Architecture if !mode.manual_pin && !mode.local_only && mode.relay_auto && mode.architect_row => Route::Architect,
        Tag::Architecture => Route::Local,
    }
}

/// What the Step Trace records for a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub tag: Tag,
    pub reason: String,
    pub route: Route,
}

impl Decision {
    pub fn trace_line(&self) -> String {
        format!("tag {} ({}) -> {:?}", self.tag.name(), self.reason, self.route)
    }
}

pub fn decide(facts: &Facts<'_>, mode: &Mode, tie: Option<&mut dyn FnMut() -> Tag>) -> Decision {
    let (tag, reason) = tag(facts, tie);

    Decision { tag, reason, route: route(tag, mode) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(kind: Kind, files: usize, text: &'a str) -> Facts<'a> {
        Facts { kind, files_touched: files, blueprint_marker: false, failed_attempts: 0, text }
    }

    #[test]
    fn deterministic_rules_tag_the_four_kinds() {
        assert_eq!(tag(&facts(Kind::Read, 1, "look at the config"), None).0, Tag::Simple);
        assert_eq!(tag(&facts(Kind::Edit, 1, "rename a variable"), None).0, Tag::Simple);
        assert_eq!(tag(&facts(Kind::Edit, 2, &"implement the cart total with tax and rounding rules for each item ".repeat(4)), None).0, Tag::Code);
        assert_eq!(tag(&facts(Kind::Edit, 2, "implement a lock-free queue"), None).0, Tag::Hard);
        assert_eq!(tag(&facts(Kind::Edit, 7, "update callers"), None).0, Tag::Hard);
        assert_eq!(tag(&Facts { failed_attempts: 2, ..facts(Kind::Edit, 1, "fix it") }, None).0, Tag::Hard);
        assert_eq!(tag(&Facts { blueprint_marker: true, ..facts(Kind::Create, 5, "lay out the folders and data models") }, None).0, Tag::Architecture);
        assert_eq!(tag(&facts(Kind::Plan, 4, "plan the modules"), None).0, Tag::Architecture);
    }

    #[test]
    fn the_model_is_only_a_tie_breaker_for_undecided_cases() {
        let mut asked = 0;
        let mut ask = || {
            asked += 1;
            Tag::Hard
        };
        let long_text = "change the order flow across several files carefully and keep every test green ".repeat(3);
        let undecided = facts(Kind::Edit, 4, &long_text);

        assert_eq!(tag(&undecided, Some(&mut ask)).0, Tag::Hard);
        assert_eq!(asked, 1);

        let mut never = || -> Tag { panic!("the rules decided, the model must not be asked") };

        assert_eq!(tag(&facts(Kind::Read, 1, "read"), Some(&mut never)).0, Tag::Simple);
        assert_eq!(tag(&facts(Kind::Edit, 8, "touch many"), Some(&mut never)).0, Tag::Hard);
        assert_eq!(tag(&undecided, None).0, Tag::Code, "no model, the ordinary default");

        let mut wild = || Tag::Architecture;

        assert_eq!(tag(&undecided, Some(&mut wild)).0, Tag::Code, "the model may only choose between code and hard");
    }

    /// The routing table for every mode combination (the plan's work-split section).
    #[test]
    fn the_routing_table_matches_the_work_split_for_all_modes() {
        crate::v3::forcing(&["nr.tag_route"], || {
            for bits in 0..16u8 {
                let mode = Mode { manual_pin: bits & 1 != 0, local_only: bits & 2 != 0, relay_auto: bits & 4 != 0, architect_row: bits & 8 != 0 };
                let may_leave = !mode.manual_pin && !mode.local_only && mode.relay_auto && mode.architect_row;

                assert_eq!(route(Tag::Simple, &mode), Route::Local, "{mode:?}");
                assert_eq!(route(Tag::Code, &mode), Route::Local, "{mode:?}");
                assert_eq!(route(Tag::Hard, &mode), Route::LocalBestOfN, "{mode:?}: hard never leaves by itself, it only enables Best-of-N earlier");
                assert_eq!(route(Tag::Architecture, &mode), if may_leave { Route::Architect } else { Route::Local }, "{mode:?}");
            }
        });
    }

    /// 3.2: the switch is on by default now, so "off" has to be said rather than assumed.
    #[test]
    fn nothing_is_routed_with_the_switch_off() {
        crate::v3::ablating(&["nr.tag_route"], || {
            let mode = Mode { manual_pin: false, local_only: false, relay_auto: true, architect_row: true };

            for tag in [Tag::Simple, Tag::Code, Tag::Hard, Tag::Architecture] {
                assert_eq!(route(tag, &mode), Route::Local);
            }
        });
    }

    #[test]
    fn the_decision_records_the_tag_and_the_reason_for_the_step_trace() {
        crate::v3::forcing(&["nr.tag_route"], || {
            let mode = Mode { manual_pin: false, local_only: false, relay_auto: true, architect_row: true };
            let decision = decide(&Facts { blueprint_marker: true, ..facts(Kind::Create, 5, "lay out the project") }, &mode, None);

            assert_eq!(decision.route, Route::Architect);
            assert!(decision.trace_line().starts_with("tag architecture (the step lays out the project's skeleton"), "{}", decision.trace_line());
        });
    }
}
