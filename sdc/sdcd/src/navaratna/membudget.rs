//! **N1-3 - the memory budgeter** (switch `nr.mem_budget`, off).
//!
//! Two 9B calls at once double the KV memory; on a small machine that is slow or fails. The budgeter does the sum -
//! `kv_budget = free memory - weights - overhead`, `tokens_total = kv_budget / bytes per token` - and allows **two** slots only if each
//! still gets at least the probed usable window; otherwise one. It reads and advises; it never sets `OLLAMA_NUM_PARALLEL`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub kv_budget: u64,
    pub tokens_total: u64,
    pub slots: usize,
    /// Even one slot cannot hold the whole usable window (the model will be squeezed): the UI says so.
    pub tight: bool,
}

/// The budget. Memory in bytes, `bytes_per_token` is the KV cache's size for one token of this model.
pub fn budget(free_memory: u64, weights: u64, overhead: u64, bytes_per_token: u64, usable_window: u64) -> Budget {
    let kv_budget = free_memory.saturating_sub(weights).saturating_sub(overhead);
    let tokens_total = kv_budget / bytes_per_token.max(1);
    let slots = if usable_window > 0 && tokens_total / 2 >= usable_window { 2 } else { 1 };

    Budget { kv_budget, tokens_total, slots, tight: tokens_total < usable_window }
}

/// A guess of the KV bytes per token when the runtime does not say: 2 (K and V) x layers x kv heads x head dim x 2 bytes (f16). The caller
/// passes the architecture when it knows it; this default is for a 9B-class model (36 layers, 8 KV heads, 128 dims).
pub fn default_bytes_per_token() -> u64 {
    2 * 36 * 8 * 128 * 2
}

/// The size of a model's weights in bytes, estimated from the parameter count in its name (`qwen3:9b`, `llama3.1:8b-q8_0`) and its
/// quantization (4.5 bits when it says nothing). `0` when the name carries no size - the budgeter then trusts the free memory alone.
pub fn weights_estimate(model: &str, quantization: Option<&str>) -> u64 {
    let lower = model.to_ascii_lowercase();
    let billions = lower
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == 'b' || c == 'm'))
        .find_map(|part| part.strip_suffix('b').and_then(|n| n.parse::<f64>().ok()))
        .unwrap_or(0.0);
    let bits = quantization.and_then(super::advice::bits_of).or_else(|| super::advice::quant_in_tag(model).as_deref().and_then(super::advice::bits_of)).unwrap_or(4.5);

    (billions * 1e9 * f64::from(bits) / 8.0) as u64
}

/// Memory the machine could give to a model right now, in bytes; `None` where it cannot be read (the budgeter then says one slot).
pub fn free_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: u64 = text.lines().find(|line| line.starts_with("MemAvailable:"))?.split_whitespace().nth(1)?.parse().ok()?;

        return Some(kb * 1024);
    }

    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("vm_stat").output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let page: u64 = text.lines().next()?.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok()?;
        let pages = |label: &str| -> u64 { text.lines().find(|line| line.starts_with(label)).and_then(|line| line.split(':').nth(1)).and_then(|n| n.trim().trim_end_matches('.').parse().ok()).unwrap_or(0) };

        return Some((pages("Pages free") + pages("Pages inactive") + pages("Pages speculative")) * page);
    }

    #[allow(unreachable_code)]
    None
}

/// How many local-model calls may run at once right now: 1 unless the switch is on and the budget says 2.
pub fn slots_now(weights: u64, usable_window: u64) -> usize {
    if !crate::v3::on("nr.mem_budget") {
        return usize::MAX;
    }

    free_memory().map_or(1, |free| budget(free, weights, 1 << 30, default_bytes_per_token(), usable_window).slots)
}

/// The scheduler's gate: at most `limit` holders at once. A second 9B call waits (or is refused by `try_take`) when the limit is one.
#[derive(Clone)]
pub struct Slots {
    inner: Arc<(Mutex<usize>, Condvar)>,
    limit: Arc<AtomicUsize>,
}

pub struct Held {
    inner: Arc<(Mutex<usize>, Condvar)>,
}

impl Drop for Held {
    fn drop(&mut self) {
        let (count, wake) = &*self.inner;

        if let Ok(mut count) = count.lock() {
            *count = count.saturating_sub(1);
        }

        wake.notify_one();
    }
}

impl Slots {
    pub fn new(limit: usize) -> Self {
        Self { inner: Arc::new((Mutex::new(0), Condvar::new())), limit: Arc::new(AtomicUsize::new(limit.max(1))) }
    }

    pub fn set_limit(&self, limit: usize) {
        self.limit.store(limit.max(1), Ordering::SeqCst);
        self.inner.1.notify_all();
    }

    /// Takes a slot if one is free right now.
    pub fn try_take(&self) -> Option<Held> {
        let mut count = self.inner.0.lock().ok()?;

        if *count >= self.limit.load(Ordering::SeqCst) {
            return None;
        }

        *count += 1;
        Some(Held { inner: self.inner.clone() })
    }

    /// Waits for a slot.
    pub fn take(&self) -> Held {
        let (count, wake) = &*self.inner;
        let mut guard = count.lock().unwrap_or_else(|poison| poison.into_inner());

        while *guard >= self.limit.load(Ordering::SeqCst) {
            guard = wake.wait(guard).unwrap_or_else(|poison| poison.into_inner());
        }

        *guard += 1;
        Held { inner: self.inner.clone() }
    }
}

/// The process's one gate for local-model calls.
pub fn gate() -> &'static Slots {
    static GATE: std::sync::OnceLock<Slots> = std::sync::OnceLock::new();

    GATE.get_or_init(|| Slots::new(usize::MAX / 2))
}

/// Holds the gate for the length of one local call - nothing when the switch is off.
pub fn hold_for_local_call(weights: u64, usable_window: u64) -> Option<Held> {
    if !crate::v3::on("nr.mem_budget") {
        return None;
    }

    gate().set_limit(slots_now(weights, usable_window));
    Some(gate().take())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn table_8_16_24_gb() {
        /* weights ~5.5 GB for a 9B at Q4/Q5, overhead 1 GB, 144 KB per token, a 16k usable window */
        let per_token = 147_456;
        let eight = budget(8 * GB, 5 * GB + GB / 2, GB, per_token, 16_384);
        let sixteen = budget(16 * GB, 5 * GB + GB / 2, GB, per_token, 16_384);
        let twenty_four = budget(24 * GB, 5 * GB + GB / 2, GB, per_token, 16_384);

        assert_eq!((eight.slots, eight.tight), (1, true), "{eight:?}");
        assert_eq!((sixteen.slots, sixteen.tight), (2, false), "{sixteen:?}");
        assert_eq!(twenty_four.slots, 2);
        assert!(twenty_four.tokens_total > sixteen.tokens_total && sixteen.tokens_total > eight.tokens_total);
        assert_eq!(budget(2 * GB, 5 * GB, GB, per_token, 16_384).kv_budget, 0, "never negative");
    }

    #[test]
    fn the_weights_are_estimated_from_the_name() {
        assert_eq!(weights_estimate("qwen3:9b", None), 5_062_500_000);
        assert_eq!(weights_estimate("llama3.1:8b-q8_0", None), 8_000_000_000);
        assert_eq!(weights_estimate("mystery", None), 0);
    }

    #[test]
    fn two_slots_need_the_window_twice_over() {
        assert_eq!(budget(10_000, 0, 0, 1, 5_000).slots, 2);
        assert_eq!(budget(9_999, 0, 0, 1, 5_000).slots, 1);
        assert_eq!(budget(10_000, 0, 0, 1, 0).slots, 1, "no probed window, no second slot");
    }

    #[test]
    fn a_second_call_does_not_start_while_the_limit_is_one() {
        let slots = Slots::new(1);
        let first = slots.try_take();

        assert!(first.is_some());
        assert!(slots.try_take().is_none(), "the scheduler never starts a second call");
        drop(first);
        assert!(slots.try_take().is_some());

        slots.set_limit(2);

        let (a, b) = (slots.try_take(), slots.try_take());

        assert!(a.is_some() && b.is_some());
        assert!(slots.try_take().is_none());
    }

    #[test]
    fn a_waiting_call_starts_when_the_first_ends() {
        let slots = Slots::new(1);
        let first = slots.take();
        let other = slots.clone();
        let waiter = std::thread::spawn(move || {
            let _held = other.take();
            true
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!waiter.is_finished());
        drop(first);
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn with_the_switch_off_there_is_no_limit_and_nothing_is_held() {
        crate::v3::ablating(&["nr.mem_budget"], || {
            assert!(hold_for_local_call(0, 1_000).is_none());
            assert_eq!(slots_now(0, 1_000), usize::MAX);
        });
    }
}
