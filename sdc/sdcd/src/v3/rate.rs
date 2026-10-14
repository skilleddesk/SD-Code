//! **Per-provider pacing** (v3 W9, switch `v3.rate_limit`): a token bucket per endpoint host, so a turn's parallel
//! sub-agents and a bench run do not set off a provider's 429 storm - the requests wait their turn on this side.
//!
//! The default is generous (120 requests a minute, bursts of 20): it shapes floods, not normal turns.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const PER_MINUTE: f64 = 120.0;
pub const BURST: f64 = 20.0;

struct Bucket {
    tokens: f64,
    at: Instant,
}

fn buckets() -> &'static Mutex<HashMap<String, Bucket>> {
    static BUCKETS: OnceLock<Mutex<HashMap<String, Bucket>>> = OnceLock::new();

    BUCKETS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The host of an endpoint URL (the bucket's name).
pub fn host(url: &str) -> String {
    url.split_once("://").map(|(_, rest)| rest).unwrap_or(url).split('/').next().unwrap_or(url).to_ascii_lowercase()
}

/// How long the next request to `host` must wait (and takes its token).
fn reserve(host: &str, per_minute: f64, burst: f64, now: Instant) -> Duration {
    let mut buckets = buckets().lock().unwrap_or_else(|poison| poison.into_inner());
    let bucket = buckets.entry(host.to_string()).or_insert(Bucket { tokens: burst, at: now });
    let rate = per_minute / 60.0;

    bucket.tokens = (bucket.tokens + now.saturating_duration_since(bucket.at).as_secs_f64() * rate).min(burst);
    bucket.at = now;
    bucket.tokens -= 1.0;

    if bucket.tokens >= 0.0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(-bucket.tokens / rate)
    }
}

/// Waits for a request slot to the endpoint (switch on), or not at all.
pub fn acquire(url: &str) {
    if !crate::v3::on("rate_limit") {
        return;
    }

    let wait = reserve(&host(url), PER_MINUTE, BURST, Instant::now());

    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_passes_and_the_rest_waits_its_turn() {
        let now = Instant::now();
        let waits: Vec<Duration> = (0..5).map(|_| reserve("bucket-test.example", 60.0, 3.0, now)).collect();

        assert_eq!(&waits[..3], &[Duration::ZERO; 3], "the burst goes at once");
        assert_eq!(waits[3], Duration::from_secs(1));
        assert_eq!(waits[4], Duration::from_secs(2));

        /* Ten seconds later the bucket is full again (capped at the burst). */
        assert_eq!(reserve("bucket-test.example", 60.0, 3.0, now + Duration::from_secs(10)), Duration::ZERO);
        assert_eq!(host("https://api.deepseek.com/v1/chat/completions"), "api.deepseek.com");
    }

    #[test]
    fn each_host_has_its_own_bucket() {
        let now = Instant::now();

        assert_eq!(reserve("bucket-a.example", 60.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve("bucket-a.example", 60.0, 1.0, now), Duration::from_secs(1));
        assert_eq!(reserve("bucket-b.example", 60.0, 1.0, now), Duration::ZERO, "a busy host does not slow another");
    }

    #[test]
    fn the_bucket_refills_with_time_and_a_debt_is_paid_off() {
        let now = Instant::now();

        assert_eq!(reserve("bucket-refill.example", 60.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve("bucket-refill.example", 60.0, 1.0, now), Duration::from_secs(1));

        /* Half a second on, the bucket is still 0.5 tokens in debt: one more request waits 1.5s in all. */
        assert_eq!(reserve("bucket-refill.example", 60.0, 1.0, now + Duration::from_millis(500)), Duration::from_millis(1500));

        /* A clock that runs backwards neither refills nor panics. */
        assert!(reserve("bucket-refill.example", 60.0, 1.0, now) > Duration::ZERO);
    }

    #[test]
    fn acquire_lets_the_default_burst_through_without_sleeping_then_the_next_request_waits() {
        let url = "https://acquire-test.example/v1/chat";
        let started = Instant::now();

        for _ in 0..BURST as usize {
            acquire(url);
        }

        assert!(started.elapsed() < Duration::from_millis(250), "the burst must not sleep");

        /* The bucket is drained: the next request owes about one token's time (60s / 120 = 0.5s). */
        let wait = reserve(&host(url), PER_MINUTE, BURST, Instant::now());

        assert!(wait > Duration::ZERO && wait <= Duration::from_millis(500), "waited {wait:?}");
    }

    #[test]
    fn acquire_sleeps_out_the_wait_once_the_bucket_is_drained() {
        let url = "https://acquire-sleep.example/v1/chat";

        crate::v3::forcing(&["rate_limit"], || {
            for _ in 0..BURST as usize {
                reserve(&host(url), PER_MINUTE, BURST, Instant::now());
            }

            let started = Instant::now();

            acquire(url);

            /* One token's time is 60s / 120 = 0.5s; allow a little for the refill between the calls. */
            let slept = started.elapsed();

            assert!(slept >= Duration::from_millis(400) && slept < Duration::from_millis(1500), "slept {slept:?}");
        });
    }

    #[test]
    fn parallel_requests_to_one_host_each_take_a_distinct_slot() {
        let now = Instant::now();
        let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(move || reserve("bucket-parallel.example", 60.0, 2.0, now))).collect();
        let mut waits: Vec<Duration> = threads.into_iter().map(|thread| thread.join().unwrap()).collect();

        waits.sort();

        /* Two ride the burst; the other six queue one second apart, none handed the same slot twice. */
        let expected: Vec<Duration> = [0, 0, 1, 2, 3, 4, 5, 6].iter().map(|secs| Duration::from_secs(*secs)).collect();

        assert_eq!(waits, expected);
    }

    #[test]
    fn a_long_idle_banks_no_more_than_the_burst() {
        let now = Instant::now();
        let later = now + Duration::from_secs(3600);

        assert_eq!(reserve("bucket-idle.example", 60.0, 2.0, now), Duration::ZERO);

        /* An hour of quiet refills to the burst and no further: two go at once, the third waits a full second. */
        assert_eq!(reserve("bucket-idle.example", 60.0, 2.0, later), Duration::ZERO);
        assert_eq!(reserve("bucket-idle.example", 60.0, 2.0, later), Duration::ZERO);
        assert_eq!(reserve("bucket-idle.example", 60.0, 2.0, later), Duration::from_secs(1));
    }

    #[test]
    fn the_host_is_lowercased_and_stripped_of_scheme_and_path() {
        assert_eq!(host("HTTPS://API.Example.COM/v1"), "api.example.com");
        assert_eq!(host("localhost:8080/v1/chat"), "localhost:8080");
        assert_eq!(host("api.example.com"), "api.example.com");
    }
}
