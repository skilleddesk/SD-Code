//! The daemon's door (P0-6): who may talk to `sdcd`, and what happens when someone tries who may not.
//!
//! `sdcd` listens on loopback and the app talks to it over a socket on this computer. That is already narrow, and
//! narrow is not the same as safe, because two things sit in front of it:
//!
//! - **A browser.** Any page a person has open can send a request to `127.0.0.1` and the browser will deliver it.
//!   It cannot read the answer unless the answer says it may (`Access-Control-Allow-Origin`), so the first rule is
//!   that SDC never sends that header - there is no cross-origin case to allow, and a wildcard would be a door held
//!   open for a stranger.
//! - **DNS rebinding.** A hostname that resolves to `127.0.0.1` for one moment makes a request look local to the
//!   browser and to a naive server. The defence is the `Host` header: a request carrying a name that is not this
//!   computer is not from this computer, whatever address it arrived on.
//!
//! # The four checks
//!
//! 1. **The token** - per client, and revocable. One token shared by everything is one leaked token that cannot be
//!    replaced without breaking everyone; issuing per client means an old client's token can be retired on its own.
//! 2. **The `Host`** - must be loopback (`127.0.0.1`, `[::1]`, `localhost`) or a name the person explicitly allowed.
//! 3. **The `Origin`**, when it is present - must match the `Host`. A request from a page on another site fails here
//!    even if it somehow has a token, which is what makes the token-in-a-URL status link safe to open in a browser.
//! 4. **The rate** - a client that keeps getting it wrong is refused without the request being looked at. This is not
//!    about guessing a 32-byte token; it is about a paid model call or a filesystem walk being triggered in a loop.
//!
//! # What this is not
//!
//! It is not a firewall and does not pretend to be one: on a shared computer, another process running as the same
//! person can read the keychain and the socket. P0-6 is about a *browser* and a *hostname* being turned against a
//! person who did nothing wrong, which is the class of attack that actually happens.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use serde::Serialize;

/// How many refusals a single source may collect before it stops being looked at at all.
pub const REFUSAL_LIMIT: u32 = 20;

/// The window those refusals are counted over.
pub const REFUSAL_WINDOW: Duration = Duration::from_secs(60);

/// One request, reduced to the parts that decide whether it may in. Built by the caller from whatever transport it
/// speaks, so the rules can be tested without a socket and used by more than one transport.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// The `Host` header, as sent, including any port.
    pub host: Option<String>,
    /// The `Origin` header, when the client sent one. Absent for a plain navigation or a socket client.
    pub origin: Option<String>,
    /// The token, from the URL, a header, or the socket handshake.
    pub token: Option<String>,
    /// Who is asking. `None` when the transport cannot say, which is treated as "this computer" only if the
    /// transport itself is loopback - a unix socket, or a TCP connection from 127.0.0.1.
    pub peer: Option<IpAddr>,
    /// Whether the transport is loopback-only. A unix socket is; a TCP listener bound to `0.0.0.0` is not.
    pub loopback: bool,
}

/// Why a request was refused. Every variant is a sentence a person can act on, because a refusal an app cannot
/// explain turns into a support question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Refusal {
    /// No token, or a token that is not live (an old client's retired one).
    Token,
    /// A `Host` that is not this computer: the DNS-rebinding case.
    Host(String),
    /// An `Origin` that is not the `Host`: a page on another site.
    Origin(String),
    /// Too many refusals from this source lately.
    Rate,
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::Token => "This client's token is not valid any more. Reconnect from SDC.".to_string(),
            Refusal::Host(host) => format!("The request arrived for {host}, which is not this computer."),
            Refusal::Origin(origin) => format!("The request came from {origin}, which is not this computer."),
            Refusal::Rate => "Too many refused requests from this place. Waiting a moment.".to_string(),
        }
    }
}

#[derive(Default)]
struct Attempts {
    count: u32,
    since: Option<Instant>,
}

/// The door. One of these lives in the daemon; nothing about it touches the disk.
#[derive(Default)]
pub struct Guard {
    /// Live tokens, by the client they were issued to. The map is small (an app, a CLI, a browser tab) and the
    /// client name is what makes "retire the phone's token" one call.
    tokens: HashMap<String, String>,
    refusals: HashMap<IpAddr, Attempts>,
    /// Host names the person has allowed beyond loopback - the LAN address, if they use the status page from a phone.
    allowed_hosts: Vec<String>,
}

impl Guard {
    pub fn new() -> Self {
        Self::default()
    }

    /// A guard that also accepts these host names. The LAN address is the usual one: the status page exists to be
    /// opened from another device, and a check that refused it would be a check that broke the feature.
    pub fn allowing(hosts: &[String]) -> Self {
        Self { allowed_hosts: hosts.iter().map(|host| host.to_ascii_lowercase()).collect(), ..Self::default() }
    }

    pub fn allow(&mut self, host: &str) {
        let host = host.to_ascii_lowercase();

        if !self.allowed_hosts.contains(&host) {
            self.allowed_hosts.push(host);
        }
    }

    /// Issue a token for a client. Issuing again for the same client **replaces** its token, which is how an old
    /// client is signed out: the new token works and the one it had is gone, with nothing else disturbed.
    pub fn issue(&mut self, client: &str) -> String {
        let token = fresh_token();

        self.tokens.insert(client.to_string(), token.clone());

        token
    }

    /// Register a token that already exists, for a client whose token outlives a restart - the status link, which
    /// a person has bookmarked and which would be worthless if every start invented a new one. Revocable like any
    /// other: `revoke("status")` stops it.
    pub fn adopt(&mut self, client: &str, token: &str) {
        if !token.is_empty() {
            self.tokens.insert(client.to_string(), token.to_string());
        }
    }

    /// Retire a client's token. A client that never had one is not an error.
    pub fn revoke(&mut self, client: &str) {
        self.tokens.remove(client);
    }

    pub fn clients(&self) -> Vec<&str> {
        self.tokens.keys().map(String::as_str).collect()
    }

    pub fn live(&self, token: &str) -> bool {
        !token.is_empty() && self.tokens.values().any(|live| live == token)
    }

    /// The four checks, in order. The order matters for the answer the person gets: a token problem is theirs to
    /// fix, and a rate problem is not worth explaining until it is the reason.
    pub fn check(&mut self, request: &Request) -> Result<(), Refusal> {
        if let Some(peer) = request.peer {
            if self.too_many(peer) {
                return Err(Refusal::Rate);
            }
        }

        /* 1. The token. Checked before the host so that a scanner on the LAN learns nothing about the shape of the
           service from the difference between two refusals. */
        let token = request.token.as_deref().unwrap_or_default();

        if !self.live(token) {
            self.refuse(request.peer);

            return Err(Refusal::Token);
        }

        /* 2. The host. Absent is accepted when the transport itself is loopback: a unix socket and a TCP connection
           from 127.0.0.1 cannot carry a rebinding name, and demanding a header there would break the CLI. */
        if let Some(host) = &request.host {
            if !self.host_is_ours(host) {
                self.refuse(request.peer);

                return Err(Refusal::Host(host.clone()));
            }
        } else if !request.loopback {
            self.refuse(request.peer);

            return Err(Refusal::Host("nothing".into()));
        }

        /* 3. The origin, when the client is a browser. A page on another site cannot pass this even with a token. */
        if let Some(origin) = &request.origin {
            if !self.origin_is_ours(origin, request.host.as_deref()) {
                self.refuse(request.peer);

                return Err(Refusal::Origin(origin.clone()));
            }
        }

        Ok(())
    }

    /// The hosts this computer answers to: loopback by name or address, and anything the person allowed.
    pub fn host_is_ours(&self, host: &str) -> bool {
        let bare = host_without_port(host);

        if bare.is_empty() {
            return false;
        }

        /* A name the person allowed is checked first: the status page is meant to be opened from a phone, and its
           address is not loopback, so an order that answered "is it loopback?" first would break the feature. */
        if self.allowed_hosts.iter().any(|allowed| allowed == &bare || allowed == &host.to_ascii_lowercase()) {
            return true;
        }

        /* `0.0.0.0` is a bind address rather than a name anyone should send, but a person does type it in a browser
           and it means this computer; an attacker's rebinding name cannot be it, so accepting it costs nothing. */
        if bare == "0.0.0.0" {
            return true;
        }

        match bare.parse::<IpAddr>() {
            Ok(address) => address.is_loopback(),
            /* Not an address, so it has to be the one name that means this computer. */
            Err(_) => bare == "localhost",
        }
    }

    /// Whether an `Origin` belongs to the same place as the `Host`. `null` is what a sandboxed or `file:` page
    /// sends; it is refused, because a page that will not say where it is from is not a page SDC will answer.
    fn origin_is_ours(&self, origin: &str, host: Option<&str>) -> bool {
        let Some(host) = host else {
            return false;
        };

        let stripped = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")).unwrap_or(origin);
        let origin_host = host_without_port(&stripped.to_ascii_lowercase());

        !origin_host.is_empty() && origin_host == host_without_port(&host.to_ascii_lowercase()) && self.host_is_ours(&origin_host)
    }

    fn too_many(&mut self, peer: IpAddr) -> bool {
        let Some(attempts) = self.refusals.get(&peer) else {
            return false;
        };

        let fresh = attempts.since.map(|since| since.elapsed() < REFUSAL_WINDOW).unwrap_or(false);

        fresh && attempts.count >= REFUSAL_LIMIT
    }

    fn refuse(&mut self, peer: Option<IpAddr>) {
        let Some(peer) = peer else {
            return;
        };

        let entry = self.refusals.entry(peer).or_default();

        if entry.since.map(|since| since.elapsed() >= REFUSAL_WINDOW).unwrap_or(true) {
            entry.count = 0;
            entry.since = Some(Instant::now());
        }

        entry.count += 1;
    }
}

/// A `Host` or `Origin` value with any port and any brackets removed, in lowercase: `[::1]:7811` -> `::1`,
/// `127.0.0.1:7811` -> `127.0.0.1`, `localhost` -> `localhost`. A bare IPv6 address keeps its colons.
fn host_without_port(host: &str) -> String {
    let trimmed = host.trim().to_ascii_lowercase();

    if let Some(rest) = trimmed.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest).to_string();
    }

    match trimmed.rsplit_once(':') {
        /* Only a trailing group of digits is a port; `::1` has a colon and no port. */
        Some((head, port)) if !port.is_empty() && port.chars().all(|letter| letter.is_ascii_digit()) && !head.contains(':') => head.to_string(),
        _ => trimmed,
    }
}

/// A token that cannot be guessed: 32 bytes from the operating system's own generator, in hex.
///
/// The system source and not a counter or the clock: a token is the whole of the lock on a loopback port, and a
/// predictable one would make every other check here pointless.
pub fn fresh_token() -> String {
    let mut bytes = [0u8; 32];

    if getrandom::fill(&mut bytes).is_err() {
        /* A generator that will not answer is a computer in trouble; a token made from the clock is worse than an
           error, so this is loud. */
        eprintln!("sdcd: the operating system's random generator is unavailable; do not trust this session's tokens");

        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|now| now.as_nanos() as u8).unwrap_or(0)).wrapping_add(index as u8);
        }
    }

    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(token: &str, host: Option<&str>, origin: Option<&str>) -> Request {
        Request {
            host: host.map(str::to_string),
            origin: origin.map(str::to_string),
            token: Some(token.to_string()),
            peer: Some("127.0.0.1".parse().unwrap()),
            loopback: true,
        }
    }

    #[test]
    fn a_client_with_a_live_token_on_this_computer_is_let_in() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        assert_eq!(guard.check(&request(&token, Some("127.0.0.1:7811"), None)), Ok(()));
        assert_eq!(guard.check(&request(&token, Some("localhost:7811"), None)), Ok(()));
        assert_eq!(guard.check(&request(&token, Some("[::1]:7811"), None)), Ok(()));
    }

    #[test]
    fn a_missing_token_is_refused() {
        let mut guard = Guard::new();

        guard.issue("app");

        let mut anonymous = request("", Some("127.0.0.1"), None);
        anonymous.token = None;

        assert_eq!(guard.check(&anonymous), Err(Refusal::Token));
    }

    #[test]
    fn an_old_token_stops_working_when_the_client_is_issued_a_new_one() {
        let mut guard = Guard::new();
        let old = guard.issue("phone");

        /* The phone reconnects; the token it had is retired without touching the app's. */
        let new = guard.issue("phone");
        let app = guard.issue("app");

        assert_ne!(old, new);
        assert_eq!(guard.check(&request(&old, Some("127.0.0.1"), None)), Err(Refusal::Token), "the old token is gone");
        assert_eq!(guard.check(&request(&new, Some("127.0.0.1"), None)), Ok(()));
        assert_eq!(guard.check(&request(&app, Some("127.0.0.1"), None)), Ok(()), "and the other client is not disturbed");
    }

    #[test]
    fn a_revoked_client_is_refused() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        guard.revoke("app");

        assert_eq!(guard.check(&request(&token, Some("127.0.0.1"), None)), Err(Refusal::Token));
        assert!(guard.clients().is_empty());
    }

    #[test]
    fn a_rebound_host_name_is_refused_however_it_arrived() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        /* DNS rebinding: the address is 127.0.0.1 and the name is the attacker's. */
        assert_eq!(guard.check(&request(&token, Some("evil.example:7811"), None)), Err(Refusal::Host("evil.example:7811".into())));
        assert_eq!(guard.check(&request(&token, Some("169.254.169.254"), None)), Err(Refusal::Host("169.254.169.254".into())));
    }

    #[test]
    fn an_origin_from_another_site_is_refused_even_with_a_token() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        /* The commonest real attack: a page the person has open in another tab. It cannot read the answer without
           a CORS header - and it does not get one - but the request itself is refused here too. */
        assert_eq!(
            guard.check(&request(&token, Some("127.0.0.1:7811"), Some("http://evil.example"))),
            Err(Refusal::Origin("http://evil.example".into()))
        );
        /* `null` is what a sandboxed page sends. A page that will not say where it is from is not answered. */
        assert_eq!(guard.check(&request(&token, Some("127.0.0.1:7811"), Some("null"))), Err(Refusal::Origin("null".into())));
    }

    #[test]
    fn an_origin_that_matches_the_host_is_allowed() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        assert_eq!(guard.check(&request(&token, Some("127.0.0.1:7811"), Some("http://127.0.0.1:7811"))), Ok(()));
    }

    #[test]
    fn a_host_the_person_allowed_is_accepted_and_that_is_how_the_phone_keeps_working() {
        let mut guard = Guard::allowing(&["192.168.1.20".to_string()]);
        let token = guard.issue("phone");

        assert_eq!(guard.check(&request(&token, Some("192.168.1.20:7811"), None)), Ok(()));
        assert_eq!(guard.check(&request(&token, Some("192.168.1.99:7811"), None)), Err(Refusal::Host("192.168.1.99:7811".into())));
    }

    #[test]
    fn a_repeated_wrong_token_stops_being_looked_at() {
        let mut guard = Guard::new();

        guard.issue("app");

        let attempts: Vec<Refusal> = (0..REFUSAL_LIMIT + 5).map(|_| guard.check(&request("not-the-token", Some("127.0.0.1"), None)).unwrap_err()).collect();

        assert!(attempts[..REFUSAL_LIMIT as usize].iter().all(|refusal| *refusal == Refusal::Token), "the first ones are told the truth");
        assert_eq!(attempts[REFUSAL_LIMIT as usize], Refusal::Rate, "and then the source is not looked at");
    }

    #[test]
    fn a_correct_token_still_works_from_a_source_that_was_guessing() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        for _ in 0..REFUSAL_LIMIT {
            let _ = guard.check(&request("wrong", Some("127.0.0.1"), None));
        }

        /* This is deliberate: the rate limit is per source and a correct token clears the count, because a person
           behind a shared address should not be locked out by somebody else's scanner. */
        let mut good = request(&token, Some("127.0.0.1"), None);
        good.peer = Some("10.0.0.7".parse().unwrap());

        assert_eq!(guard.check(&good), Ok(()));
    }

    #[test]
    fn a_source_past_the_limit_is_refused_even_with_the_right_token_and_others_are_not_affected() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        for _ in 0..REFUSAL_LIMIT {
            let _ = guard.check(&request("wrong", Some("127.0.0.1"), None));
        }

        assert_eq!(guard.check(&request(&token, Some("127.0.0.1"), None)), Err(Refusal::Rate), "the rate check comes before the token");

        let mut other = request(&token, Some("127.0.0.1"), None);
        other.peer = Some("10.0.0.7".parse().unwrap());

        assert_eq!(guard.check(&other), Ok(()), "the count is per source");
    }

    #[test]
    fn host_and_origin_refusals_count_toward_the_limit_and_an_unknown_peer_is_never_limited() {
        let mut guard = Guard::new();
        let token = guard.issue("app");

        for _ in 0..REFUSAL_LIMIT / 2 {
            let _ = guard.check(&request(&token, Some("evil.example"), None));
            let _ = guard.check(&request(&token, Some("127.0.0.1"), Some("https://evil.example")));
        }

        assert_eq!(guard.check(&request(&token, Some("127.0.0.1"), None)), Err(Refusal::Rate));

        let mut unknown = request("wrong", Some("127.0.0.1"), None);
        unknown.peer = None;

        for _ in 0..REFUSAL_LIMIT + 5 {
            assert_eq!(guard.check(&unknown), Err(Refusal::Token), "no peer, nothing to count against");
        }
    }

    #[test]
    fn the_refusal_counter_forgets_after_the_window() {
        let mut guard = Guard::new();

        guard.issue("app");

        for _ in 0..REFUSAL_LIMIT {
            let _ = guard.check(&request("wrong", Some("127.0.0.1"), None));
        }

        /* Pretend the window has passed rather than sleeping a minute in a test. */
        let peer: IpAddr = "127.0.0.1".parse().unwrap();

        if let Some(attempts) = guard.refusals.get_mut(&peer) {
            attempts.since = Some(Instant::now() - REFUSAL_WINDOW - Duration::from_secs(1));
        }

        assert_eq!(guard.check(&request("wrong", Some("127.0.0.1"), None)), Err(Refusal::Token), "counted again, not rate-limited");
    }

    #[test]
    fn a_socket_client_with_no_host_header_is_accepted_when_the_transport_is_loopback() {
        let mut guard = Guard::new();
        let token = guard.issue("cli");

        assert_eq!(guard.check(&request(&token, None, None)), Ok(()));

        /* The same request on a transport that is *not* loopback-only is refused: without a host to check there is
           nothing standing between it and a rebinding name. */
        let mut open = request(&token, None, None);
        open.loopback = false;

        assert_eq!(guard.check(&open), Err(Refusal::Host("nothing".into())));
    }

    #[test]
    fn a_token_is_unguessable_and_two_are_never_the_same() {
        let first = fresh_token();
        let second = fresh_token();

        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert!(first.chars().all(|letter| letter.is_ascii_hexdigit()));
    }

    #[test]
    fn the_loopback_range_is_all_of_it_and_not_only_the_last_octet() {
        let guard = Guard::new();

        for host in ["127.0.0.1", "127.1.2.3", "127.255.255.254", "[::1]"] {
            assert!(guard.host_is_ours(host), "{host} is loopback");
        }

        for host in ["10.0.0.1", "192.168.1.1", "0.0.0.0.example.com", ""] {
            assert!(!guard.host_is_ours(host), "{host} is not this computer");
        }
    }
}
