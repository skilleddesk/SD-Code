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
}
