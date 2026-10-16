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
    url.split_once("://").map(|(_, rest)| rest).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or(url).to_ascii_lowercase()
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
    fn acquire_does_not_wait_or_take_a_token_when_the_switch_is_off() {
        let url = "https://acquire-off.example/v1/chat";

        for _ in 0..BURST as usize {
            reserve(&host(url), PER_MINUTE, BURST, Instant::now());
        }

        /* Off on this thread only: `SDC_ABLATE` in the environment also switched it off for the tests beside this one. */
        let started = Instant::now();

        crate::v3::ablating(&["rate_limit"], || acquire(url));

        assert!(started.elapsed() < Duration::from_millis(250), "a switched-off limiter must not sleep");

        /* The skipped call took no token: the debt is still about one token (0.5s), not two (1s). */
        let wait = reserve(&host(url), PER_MINUTE, BURST, Instant::now());

        assert!(wait > Duration::ZERO && wait <= Duration::from_millis(500), "waited {wait:?}");
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
    fn different_paths_and_cases_of_one_host_share_a_bucket() {
        let now = Instant::now();

        assert_eq!(reserve(&host("https://Shared.example/v1/chat"), 60.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve(&host("http://shared.EXAMPLE/v1/models"), 60.0, 1.0, now), Duration::from_secs(1));
    }

    #[test]
    fn the_default_limits_pass_twenty_at_once_then_queue_half_a_second_apart() {
        let now = Instant::now();
        let waits: Vec<Duration> = (0..23).map(|_| reserve("bucket-default.example", PER_MINUTE, BURST, now)).collect();

        assert!(waits[..BURST as usize].iter().all(|wait| wait.is_zero()), "the default burst goes at once");
        assert_eq!(&waits[20..], &[Duration::from_millis(500), Duration::from_secs(1), Duration::from_millis(1500)]);
    }

    #[test]
    fn a_request_that_waited_has_paid_for_its_slot_so_the_next_one_still_queues() {
        let now = Instant::now();

        assert_eq!(reserve("bucket-queue.example", 60.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve("bucket-queue.example", 60.0, 1.0, now), Duration::from_secs(1));

        /* The second request wakes at +1s holding its token; one arriving then finds the bucket empty, not refilled. */
        assert_eq!(reserve("bucket-queue.example", 60.0, 1.0, now + Duration::from_secs(1)), Duration::from_secs(1));
    }

    #[test]
    fn a_poisoned_lock_does_not_stop_the_limiter() {
        /* A thread that panics while holding the buckets poisons the lock; pacing must carry on regardless. */
        let _ = std::thread::spawn(|| {
            let _guard = buckets().lock().unwrap_or_else(|poison| poison.into_inner());

            panic!("poison the buckets on purpose");
        })
        .join();

        let now = Instant::now();

        assert_eq!(reserve("bucket-poison.example", 60.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve("bucket-poison.example", 60.0, 1.0, now), Duration::from_secs(1));
    }

    #[test]
    fn a_slower_rate_makes_each_queued_request_wait_longer() {
        let now = Instant::now();

        /* 30 a minute is one token every 2s. */
        assert_eq!(reserve("bucket-slow.example", 30.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve("bucket-slow.example", 30.0, 1.0, now), Duration::from_secs(2));
        assert_eq!(reserve("bucket-slow.example", 30.0, 1.0, now), Duration::from_secs(4));
    }

    #[test]
    fn a_partial_refill_frees_exactly_the_tokens_earned() {
        let now = Instant::now();

        assert_eq!(reserve("bucket-partial.example", 60.0, 2.0, now), Duration::ZERO);
        assert_eq!(reserve("bucket-partial.example", 60.0, 2.0, now), Duration::ZERO);

        /* One second at 60 a minute earns one token: one request goes at once, the next owes a full second. */
        let later = now + Duration::from_secs(1);

        assert_eq!(reserve("bucket-partial.example", 60.0, 2.0, later), Duration::ZERO);
        assert_eq!(reserve("bucket-partial.example", 60.0, 2.0, later), Duration::from_secs(1));
    }

    #[test]
    fn the_same_host_on_another_port_is_a_separate_bucket() {
        let now = Instant::now();

        assert_eq!(reserve(&host("http://bucket-port.example:8080/v1"), 60.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve(&host("http://bucket-port.example:8081/v1"), 60.0, 1.0, now), Duration::ZERO);
        assert_eq!(reserve(&host("http://bucket-port.example:8080/v2"), 60.0, 1.0, now), Duration::from_secs(1));
    }

    #[test]
    fn a_request_arriving_exactly_when_a_token_is_earned_goes_at_once() {
        let now = Instant::now();

        assert_eq!(reserve("bucket-boundary.example", 60.0, 1.0, now), Duration::ZERO);

        /* The bucket lands on exactly zero tokens, which is not a debt: no wait. The next one right after does wait. */
        let later = now + Duration::from_secs(1);

        assert_eq!(reserve("bucket-boundary.example", 60.0, 1.0, later), Duration::ZERO);
        assert_eq!(reserve("bucket-boundary.example", 60.0, 1.0, later), Duration::from_secs(1));
    }

    #[test]
    fn a_deep_debt_is_cleared_by_waiting_it_out_and_the_bucket_starts_over_full() {
        let now = Instant::now();

        /* One token, then three more requests queue: the bucket is three tokens in debt. */
        let waits: Vec<Duration> = (0..4).map(|_| reserve("bucket-debt.example", 60.0, 1.0, now)).collect();

        assert_eq!(waits, [0, 1, 2, 3].map(Duration::from_secs));

        /* Two seconds on, two of the three are paid off: the next request owes the last second and its own. */
        assert_eq!(reserve("bucket-debt.example", 60.0, 1.0, now + Duration::from_secs(2)), Duration::from_secs(2));

        /* Long after, the debt is gone and the burst is whole again. */
        assert_eq!(reserve("bucket-debt.example", 60.0, 1.0, now + Duration::from_secs(600)), Duration::ZERO);
        assert_eq!(reserve("bucket-debt.example", 60.0, 1.0, now + Duration::from_secs(600)), Duration::from_secs(1));
    }

    #[test]
    fn a_bucket_refills_at_the_rate_of_its_own_call_not_a_shared_clock() {
        let now = Instant::now();

        /* Two hosts drained together, then one asked a second later: only that one has earned a token back. */
        for name in ["bucket-clock-a.example", "bucket-clock-b.example"] {
            assert_eq!(reserve(name, 60.0, 1.0, now), Duration::ZERO);
        }

        assert_eq!(reserve("bucket-clock-a.example", 60.0, 1.0, now + Duration::from_secs(1)), Duration::ZERO);
        assert_eq!(reserve("bucket-clock-b.example", 60.0, 1.0, now), Duration::from_secs(1));
    }

    #[test]
    fn the_host_is_lowercased_and_stripped_of_scheme_and_path() {
        assert_eq!(host("HTTPS://API.Example.COM/v1"), "api.example.com");
        assert_eq!(host("localhost:8080/v1/chat"), "localhost:8080");
        assert_eq!(host("api.example.com"), "api.example.com");
    }

    #[test]
    fn a_query_or_fragment_straight_after_the_host_does_not_make_a_new_bucket() {
        assert_eq!(host("https://api.example.com?key=1"), "api.example.com");
        assert_eq!(host("https://api.example.com#top"), "api.example.com");
        assert_eq!(host("https://api.example.com/v1?key=1"), "api.example.com");
    }
}
