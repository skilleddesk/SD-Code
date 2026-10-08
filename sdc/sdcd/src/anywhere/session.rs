//! What a connected browser may do, and for how long (plan section 6.1).
//!
//! Four levels, opened step by step so that looking is easy, changing needs more, and dangerous work
//! needs a fresh passkey every time:
//!
//! | level      | what it allows                                   | how it opens                          | how long |
//! | ---------- | ------------------------------------------------ | ------------------------------------- | -------- |
//! | `Locked`   | a bare "approval needed" count; Kill             | a trusted device's signed hello       | -        |
//! | `View`     | stream, lists, diffs, logs                       | a passkey assertion on the daemon's nonce | until 15 minutes of silence |
//! | `Operate`  | edits, uploads, low and medium approvals        | a passkey assertion → a 5-minute window | 5 minutes |
//! | `Critical` | high-risk commands, delete, deploy, rewind       | a passkey assertion over that very action | one action |
//!
//! `Critical` is never a state a session sits in: it is what one action asks for, and the passkey
//! assertion that answers it is spent on that action's hash alone.
//!
//! Everything here is a pure function of the clock that is passed in, so every rule is testable.

use std::time::Duration;

/// Methods no remote session may call at any level. Plan principle 6, as a constant in code: a project
/// file or a setting cannot shorten this list (DESIGN.md OQ-4 and OQ-12).
pub const REMOTE_FORBIDDEN_EXACT: &[&str] = &[
    "policy.edit",
    "policy.set",
    "remote.pair",
    "remote.toggle",
    "keychain.read",
    "audit.erase",
    "host.password",
    "host.shutdown",
    "ssh.key",
    "team.set",
    "reset.erase",
];

/// Whole families that are never remote: the Anywhere settings themselves, key material, provider keys.
pub const REMOTE_FORBIDDEN_PREFIX: &[&str] = &["anywhere.", "keychain.", "provider.key", "secrets.", "policy."];

/// The levels, in order. `Ord` is the order of power.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Locked,
    View,
    Operate,
    Critical,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::View => "view",
            Self::Operate => "operate",
            Self::Critical => "critical",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "locked" => Some(Self::Locked),
            "view" => Some(Self::View),
            "operate" => Some(Self::Operate),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// What a method needs, or that it is not available remotely at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Callable once the session has at least this level.
    Level(Level),
    /// Never callable remotely, at any level.
    Never,
}

/// The level a tunnelled SDCP method needs. **An allow-list**: a method that is not named is `Never`,
/// so a method added to the daemon later is not reachable from a browser until someone lists it here.
pub fn need_for(method: &str) -> Need {
    if REMOTE_FORBIDDEN_EXACT.contains(&method) || REMOTE_FORBIDDEN_PREFIX.iter().any(|prefix| method.starts_with(prefix)) {
        return Need::Never;
    }

    match method {
        /* Stopping things is always easy (plan 5.8): no step-up, available even while locked. */
        "kill.all" | "kill.list" => Need::Level(Level::Locked),
        /* Looking. */
        "host.status" | "session.list" | "event.list" | "audit.verify" | "checkpoint.list" | "trust.score" => Need::Level(Level::View),
        /* The file gateway (0.18): looking is View, starting a chat is Operate. Reading a protected file also
           needs a fresh passkey for that file; `core` asks for it. */
        "hosts.list" | "fs.list" | "fs.read" | "fs.search" | "fs.git" | "chat.options" => Need::Level(Level::View),
        "chat.send" => Need::Level(Level::Operate),
        _ => Need::Never,
    }
}

/// The timers a session runs on. Defaults are the plan's; the daemon's settings may only make them
/// stricter than `Limits::default()` (see `clamp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub view_idle_ms: i64,
    pub operate_window_ms: i64,
    pub guest_max_ms: i64,
    /// How long an unlock challenge stays answerable.
    pub challenge_ms: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            view_idle_ms: Duration::from_secs(15 * 60).as_millis() as i64,
            operate_window_ms: Duration::from_secs(5 * 60).as_millis() as i64,
            guest_max_ms: Duration::from_secs(120 * 60).as_millis() as i64,
            challenge_ms: Duration::from_secs(60).as_millis() as i64,
        }
    }
}

impl Limits {
    /// A setting can shorten a limit but never lengthen it beyond the plan's maximum.
    pub fn clamp(self) -> Self {
        let max = Self::default();

        Self {
            view_idle_ms: self.view_idle_ms.clamp(60_000, max.view_idle_ms),
            operate_window_ms: self.operate_window_ms.clamp(30_000, max.operate_window_ms),
            guest_max_ms: self.guest_max_ms.clamp(60_000, max.guest_max_ms),
            challenge_ms: self.challenge_ms.clamp(10_000, max.challenge_ms),
        }
    }
}

/// One connection's capability state.
#[derive(Debug, Clone)]
pub struct Capability {
    limits: Limits,
    guest: bool,
    started_at: i64,
    last_activity: i64,
    /// Set once a passkey (or, for a guest, the device key) has unlocked the session.
    viewing: bool,
    operate_until: i64,
}

impl Capability {
    pub fn new(now_ms: i64, guest: bool, limits: Limits) -> Self {
        /* A guest has no passkey on a borrowed laptop, so the device key it just paired with is the
           whole proof, and the session opens at View. Its life is capped. */
        Self { limits: limits.clamp(), guest, started_at: now_ms, last_activity: now_ms, viewing: guest, operate_until: 0 }
    }

    pub fn is_guest(&self) -> bool {
        self.guest
    }

    /// Whether the session as a whole has run out (a guest's two hours).
    pub fn is_over(&self, now_ms: i64) -> bool {
        self.guest && now_ms - self.started_at >= self.limits.guest_max_ms
    }

    /// Records that the person did something. Reading a screen counts; a heartbeat does not.
    pub fn touch(&mut self, now_ms: i64) {
        if self.level(now_ms) != Level::Locked {
            self.last_activity = now_ms;
        }
    }

    /// The level right now.
    pub fn level(&self, now_ms: i64) -> Level {
        if self.is_over(now_ms) || !self.viewing {
            return Level::Locked;
        }

        if now_ms - self.last_activity >= self.limits.view_idle_ms {
            return Level::Locked;
        }

        if !self.guest && now_ms < self.operate_until {
            return Level::Operate;
        }

        Level::View
    }

    /// A passkey assertion on the daemon's nonce was verified: the session may view.
    pub fn unlock_view(&mut self, now_ms: i64) {
        self.viewing = true;
        self.last_activity = now_ms;
    }

    /// A passkey assertion opened the Operate window. A guest cannot.
    pub fn unlock_operate(&mut self, now_ms: i64) -> Result<i64, &'static str> {
        if self.guest {
            return Err("a guest session is view-only");
        }

        self.viewing = true;
        self.last_activity = now_ms;
        self.operate_until = now_ms + self.limits.operate_window_ms;

        Ok(self.operate_until)
    }

    pub fn lock(&mut self) {
        self.viewing = false;
        self.operate_until = 0;
    }

    pub fn operate_until(&self, now_ms: i64) -> Option<i64> {
        (self.level(now_ms) == Level::Operate).then_some(self.operate_until)
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_000_000;
    const MIN: i64 = 60_000;

    fn session() -> Capability {
        Capability::new(T0, false, Limits::default())
    }

    #[test]
    fn a_new_session_is_locked_until_a_passkey_unlocks_it() {
        let session = session();

        assert_eq!(session.level(T0), Level::Locked);
    }

    #[test]
    fn view_locks_again_after_fifteen_minutes_of_silence() {
        let mut session = session();

        session.unlock_view(T0);

        assert_eq!(session.level(T0 + 14 * MIN), Level::View);
        assert_eq!(session.level(T0 + 15 * MIN), Level::Locked);
    }

    #[test]
    fn activity_keeps_view_open() {
        let mut session = session();

        session.unlock_view(T0);
        session.touch(T0 + 10 * MIN);

        assert_eq!(session.level(T0 + 20 * MIN), Level::View, "10 + 10 minutes of silence is under the limit");
        assert_eq!(session.level(T0 + 25 * MIN), Level::Locked);
    }

    #[test]
    fn touching_a_locked_session_does_not_unlock_it() {
        let mut session = session();

        session.touch(T0 + MIN);

        assert_eq!(session.level(T0 + MIN), Level::Locked);
    }

    #[test]
    fn the_operate_window_is_five_minutes_and_then_it_is_view_again() {
        let mut session = session();

        session.unlock_view(T0);

        assert_eq!(session.unlock_operate(T0 + MIN), Ok(T0 + 6 * MIN));
        assert_eq!(session.level(T0 + 5 * MIN), Level::Operate);
        assert_eq!(session.level(T0 + 6 * MIN), Level::View);
        assert_eq!(session.operate_until(T0 + 6 * MIN), None);
    }

    #[test]
    fn operate_can_be_reopened_but_never_stretched_past_the_limit() {
        let mut session = session();

        session.unlock_operate(T0).unwrap();
        session.touch(T0 + 4 * MIN);

        assert_eq!(session.level(T0 + 5 * MIN), Level::View, "activity does not extend the window");
    }

    #[test]
    fn a_guest_opens_at_view_and_cannot_operate() {
        let mut guest = Capability::new(T0, true, Limits::default());

        assert_eq!(guest.level(T0), Level::View);
        assert!(guest.unlock_operate(T0).is_err());
        assert_eq!(guest.level(T0 + MIN), Level::View);
    }

    #[test]
    fn a_guest_session_ends_after_two_hours_whatever_it_does() {
        let mut guest = Capability::new(T0, true, Limits::default());

        for minute in (10..120).step_by(10) {
            guest.touch(T0 + minute * MIN);
        }

        assert_eq!(guest.level(T0 + 119 * MIN), Level::View);
        assert_eq!(guest.level(T0 + 120 * MIN), Level::Locked);
        assert!(guest.is_over(T0 + 120 * MIN));
    }

    #[test]
    fn locking_closes_everything() {
        let mut session = session();

        session.unlock_operate(T0).unwrap();
        session.lock();

        assert_eq!(session.level(T0 + 1), Level::Locked);
    }

    #[test]
    fn settings_can_shorten_limits_but_not_lengthen_them() {
        let longer = Limits { view_idle_ms: 10 * 60 * MIN, operate_window_ms: 60 * MIN, guest_max_ms: 24 * 60 * MIN, challenge_ms: 60 * MIN };
        let clamped = longer.clamp();

        assert_eq!(clamped, Limits::default());

        let shorter = Limits { view_idle_ms: 2 * MIN, ..Limits::default() }.clamp();

        assert_eq!(shorter.view_idle_ms, 2 * MIN);
    }

    #[test]
    fn levels_are_ordered_by_power() {
        assert!(Level::Locked < Level::View);
        assert!(Level::View < Level::Operate);
        assert!(Level::Operate < Level::Critical);
        assert_eq!(Level::parse("operate"), Some(Level::Operate));
        assert_eq!(Level::parse("root"), None);
    }

    #[test]
    fn the_forbidden_methods_are_never_callable() {
        for method in ["policy.edit", "remote.pair", "remote.toggle", "keychain.read", "audit.erase", "anywhere.enable", "anywhere.pair.begin", "policy.anything", "host.password", "provider.key.set"] {
            assert_eq!(need_for(method), Need::Never, "{method}");
        }
    }

    #[test]
    fn an_unlisted_method_is_never_callable() {
        assert_eq!(need_for("fs.write"), Need::Never, "not until a later phase lists it with its own checks");
        assert_eq!(need_for("shell.run"), Need::Never);
        assert_eq!(need_for("some.method.added.next.year"), Need::Never);
    }

    #[test]
    fn kill_needs_no_step_up_and_looking_needs_view() {
        assert_eq!(need_for("kill.all"), Need::Level(Level::Locked));
        assert_eq!(need_for("host.status"), Need::Level(Level::View));
        assert_eq!(need_for("event.list"), Need::Level(Level::View));
    }
}
