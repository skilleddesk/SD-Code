//! **The Hardware Auto-Tuner and the VRAM Arbiter (P0-16, switch `local.autotune`)**: what a local model is allowed to ask
//! of the graphics card, worked out from the card - and a rule that nothing else is allowed to share it.
//!
//! A 9B model at 4-bit is 6.6 GB of weights. On an 8 GB card that leaves a gigabyte or two for the KV cache, which is
//! the memory the *context* lives in; ask for more and Ollama quietly spills layers to the CPU, and the model that
//! answered in a second now takes ten. So the tuner does the arithmetic: weights + a fixed graph overhead + the KV cache
//! of `num_ctx` tokens must fit in the card, and the largest `num_ctx` that does is the proposal.
//!
//! Three things it never does:
//!
//! * **apply anything.** The proposal is shown in the Provider Hub's doctor flow. `OLLAMA_FLASH_ATTENTION` and
//!   `OLLAMA_KV_CACHE_TYPE` are server-wide environment variables, so changing them means restarting Ollama - a thing a
//!   person decides, not a thing a background task does.
//! * **guess a card it cannot read.** No detected GPU is "unknown", and the proposal says what it assumed.
//! * **let a second model onto the GPU.** The [`Arbiter`] admits one main model; rerankers, embedding models and
//!   classifiers are placed on the CPU, always.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

const MIB: u64 = 1024 * 1024;
/// The compute graph and runtime buffers a loaded model needs beside its weights and KV cache.
const GRAPH_OVERHEAD: u64 = 300 * MIB;
/// Never plan to the last byte of a card: the desktop and the driver want some.
const USABLE: f64 = 0.95;
/// A KV cache entry is 2 bytes at f16 and about 1.06 at q8_0 (8.5 bits).
const KV_F16: f64 = 2.0;
const KV_Q8: f64 = 1.0625;

#[derive(Debug, Clone, PartialEq)]
pub struct Hardware {
    pub gpu_name: Option<String>,
    /// Memory the model can live in: a discrete card's VRAM, or on a Mac the share of unified memory the GPU may use.
    pub gpu_bytes: Option<u64>,
    pub ram_bytes: u64,
    /// `true` when the GPU shares system memory (Apple silicon): there is no "spill", only pressure.
    pub unified: bool,
}

/// `name, memory.total` from `nvidia-smi --query-gpu=name,memory.total --format=csv,noheader,nounits` (MiB), the
/// largest card when there are several.
pub fn parse_nvidia(output: &str) -> Option<(String, u64)> {
    output
        .lines()
        .filter_map(|line| {
            let (name, memory) = line.rsplit_once(',')?;

            Some((name.trim().to_string(), memory.trim().parse::<u64>().ok()? * MIB))
        })
        .max_by_key(|(_, bytes)| *bytes)
}

/// Apple's unified memory: the GPU may use about three quarters of it by default (`recommendedMaxWorkingSetSize`).
pub fn mac_gpu_budget(ram_bytes: u64) -> u64 {
    ram_bytes / 4 * 3
}

/// What kind of memory the GPU number is, and how sure we are of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Detected {
    pub hardware: Hardware,
    /// `nvidia`, `apple`, `amd`, `intel`, or `none`.
    pub vendor: &'static str,
    /// `true` when the memory figure was not read from the card itself (AMD/Intel fallbacks, integrated graphics): every number
    /// derived from it is an estimate and is labelled one.
    pub estimate: bool,
}

/// `Card series` and `VRAM Total Memory (B)` from `rocm-smi --showproductname --showmeminfo vram --json`, the largest card.
pub fn parse_rocm(output: &str) -> Option<(String, u64)> {
    let value: Value = serde_json::from_str(output.trim()).ok()?;

    value
        .as_object()?
        .iter()
        .filter(|(key, _)| key.starts_with("card"))
        .filter_map(|(_, card)| {
            let bytes = card["VRAM Total Memory (B)"].as_str()?.trim().parse::<u64>().ok().or_else(|| card["VRAM Total Memory (B)"].as_u64())?;
            let name = card["Card series"].as_str().or_else(|| card["Card Series"].as_str()).unwrap_or("AMD GPU");

            Some((name.to_string(), bytes))
        })
        .max_by_key(|(_, bytes)| *bytes)
}

/// `name|bytes;name|bytes` from the Windows display-adapter registry keys (the only place a 64-bit memory size lives - WMI's
/// `AdapterRAM` stops at 4 GB). The largest adapter.
pub fn parse_windows_adapters(output: &str) -> Option<(String, u64)> {
    output
        .trim()
        .split(';')
        .filter_map(|entry| {
            let (name, bytes) = entry.rsplit_once('|')?;

            Some((name.trim().to_string(), bytes.trim().parse::<u64>().ok().filter(|bytes| *bytes > 0)?))
        })
        .max_by_key(|(_, bytes)| *bytes)
}

/// The vendor a display-adapter name belongs to.
pub fn vendor_of(name: &str) -> &'static str {
    let lowered = name.to_lowercase();

    if lowered.contains("nvidia") || lowered.contains("geforce") || lowered.contains("rtx") {
        "nvidia"
    } else if lowered.contains("amd") || lowered.contains("radeon") {
        "amd"
    } else if lowered.contains("intel") || lowered.contains("arc ") || lowered.contains("iris") || lowered.contains("uhd") {
        "intel"
    } else {
        "none"
    }
}

/// An Intel integrated GPU shares system memory; an Arc card has its own.
pub fn intel_is_integrated(name: &str) -> bool {
    let lowered = name.to_lowercase();

    lowered.contains("uhd") || lowered.contains("iris") || lowered.contains("hd graphics") || (lowered.contains("intel") && !lowered.contains("arc"))
}

/// Reads this machine, saying where each figure came from. Every probe is a command that may be missing; a missing one is an
/// unknown, not an error. NVIDIA and Apple silicon are read directly; AMD and Intel are fallbacks and are marked `estimate`.
pub fn detect_detailed() -> Detected {
    let ram = std::process::Command::new("sysctl").args(["-n", "hw.memsize"]).output().ok().and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().ok());
    let ram_bytes = ram.or_else(linux_ram).or_else(windows_ram).unwrap_or(0);

    if let Some(out) = std::process::Command::new("nvidia-smi").args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]).output().ok().filter(|out| out.status.success()) {
        if let Some((name, bytes)) = parse_nvidia(&String::from_utf8_lossy(&out.stdout)) {
            return Detected { hardware: Hardware { gpu_name: Some(name), gpu_bytes: Some(bytes), ram_bytes, unified: false }, vendor: "nvidia", estimate: false };
        }
    }

    if cfg!(all(target_os = "macos", target_arch = "aarch64")) && ram_bytes > 0 {
        return Detected { hardware: Hardware { gpu_name: Some("Apple silicon (unified memory)".into()), gpu_bytes: Some(mac_gpu_budget(ram_bytes)), ram_bytes, unified: true }, vendor: "apple", estimate: false };
    }

    if let Some(out) = std::process::Command::new("rocm-smi").args(["--showproductname", "--showmeminfo", "vram", "--json"]).output().ok().filter(|out| out.status.success()) {
        if let Some((name, bytes)) = parse_rocm(&String::from_utf8_lossy(&out.stdout)) {
            return Detected { hardware: Hardware { gpu_name: Some(name), gpu_bytes: Some(bytes), ram_bytes, unified: false }, vendor: "amd", estimate: true };
        }
    }

    let adapter = linux_drm_adapter().or_else(windows_adapter);

    if let Some((name, bytes)) = adapter {
        let vendor = vendor_of(&name);

        if vendor == "intel" && intel_is_integrated(&name) && ram_bytes > 0 {
            /* shares system memory: about half of it is usable, and that is a guess */
            return Detected { hardware: Hardware { gpu_name: Some(format!("{name} (shared memory)")), gpu_bytes: Some(ram_bytes / 2), ram_bytes, unified: true }, vendor, estimate: true };
        }

        if vendor == "amd" || vendor == "intel" || vendor == "none" {
            return Detected { hardware: Hardware { gpu_name: Some(name), gpu_bytes: Some(bytes), ram_bytes, unified: false }, vendor, estimate: true };
        }
    }

    Detected { hardware: Hardware { gpu_name: None, gpu_bytes: None, ram_bytes, unified: false }, vendor: "none", estimate: false }
}

/// Reads this machine (see [`detect_detailed`] for where the numbers came from).
pub fn detect() -> Hardware {
    detect_detailed().hardware
}

#[cfg(target_os = "linux")]
fn linux_drm_adapter() -> Option<(String, u64)> {
    std::fs::read_dir("/sys/class/drm").ok()?.flatten().filter_map(|entry| {
        let device = entry.path().join("device");
        let vendor = std::fs::read_to_string(device.join("vendor")).ok()?;
        let bytes = std::fs::read_to_string(device.join("mem_info_vram_total")).ok()?.trim().parse::<u64>().ok()?;
        let name = match vendor.trim() {
            "0x1002" => "AMD Radeon",
            "0x8086" => "Intel Arc",
            _ => return None,
        };

        Some((name.to_string(), bytes))
    }).max_by_key(|(_, bytes)| *bytes)
}

#[cfg(not(target_os = "linux"))]
fn linux_drm_adapter() -> Option<(String, u64)> {
    None
}

#[cfg(windows)]
fn windows_adapter() -> Option<(String, u64)> {
    let script = "(Get-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\0*' -ErrorAction SilentlyContinue | ForEach-Object { $_.DriverDesc + '|' + $_.'HardwareInformation.qwMemorySize' }) -join ';'";
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", script]).output().ok().filter(|out| out.status.success())?;

    parse_windows_adapters(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(windows))]
fn windows_adapter() -> Option<(String, u64)> {
    None
}

#[cfg(windows)]
fn windows_ram() -> Option<u64> {
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory"]).output().ok()?;

    String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().ok()
}

#[cfg(not(windows))]
fn windows_ram() -> Option<u64> {
    None
}

fn linux_ram() -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo").ok()?.lines().find_map(|line| line.strip_prefix("MemTotal:")?.split_whitespace().next()?.parse::<u64>().ok().map(|kib| kib * 1024))
}

/// What the KV cache of a model costs per token, from `/api/show`'s `model_info`
/// (`<arch>.block_count`, `<arch>.attention.head_count_kv`, `<arch>.attention.key_length`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub layers: u64,
    pub kv_heads: u64,
    pub head_dim: u64,
    /// The model's own limit, in tokens.
    pub max_context: u64,
}

pub fn shape_from_show(show: &Value) -> Option<Shape> {
    let info = show["model_info"].as_object()?;
    let find = |suffix: &str| info.iter().find(|(key, _)| key.ends_with(suffix)).and_then(|(_, value)| value.as_u64());
    let layers = find(".block_count")?;
    let heads = find(".attention.head_count");
    let kv_heads = find(".attention.head_count_kv").or(heads)?;
    let head_dim = find(".attention.key_length").or_else(|| Some(find(".embedding_length")? / heads?))?;

    Some(Shape { layers, kv_heads, head_dim, max_context: find(".context_length").unwrap_or(32_768) })
}

impl Shape {
    /// Bytes of KV cache for one token: keys and values, in every layer.
    pub fn kv_bytes_per_token(&self, bytes_per_element: f64) -> f64 {
        2.0 * self.layers as f64 * self.kv_heads as f64 * self.head_dim as f64 * bytes_per_element
    }
}

/// A mixture-of-experts model: only `used` of its `experts` run for each token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moe {
    pub experts: u64,
    pub used: u64,
}

impl Moe {
    /// Expert feed-forward layers hold about nine tenths of an MoE's weights (an estimate - the rest is attention and shared layers).
    pub const EXPERT_SHARE: f64 = 0.9;

    /// The share of the weights read for one token.
    pub fn active_fraction(&self) -> f64 {
        if self.experts == 0 {
            return 1.0;
        }

        (1.0 - Self::EXPERT_SHARE) + Self::EXPERT_SHARE * (self.used.min(self.experts) as f64 / self.experts as f64)
    }
}

/// `<arch>.expert_count` and `<arch>.expert_used_count` from `/api/show`'s `model_info`; `None` for a dense model.
pub fn moe_from_show(show: &Value) -> Option<Moe> {
    let info = show["model_info"].as_object()?;
    let find = |suffix: &str| info.iter().find(|(key, _)| key.ends_with(suffix)).and_then(|(_, value)| value.as_u64());
    let experts = find(".expert_count").filter(|count| *count > 1)?;

    Some(Moe { experts, used: find(".expert_used_count").unwrap_or(2) })
}

/// What the proposal is for: a model's weights and shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub name: String,
    pub weights_bytes: u64,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// `gpu`, `gpu+cpu-experts` (an MoE with its experts in system RAM), `partial-offload` (a dense model with some layers on the
    /// CPU), or `cpu`.
    pub mode: &'static str,
    /// Every number here comes from a fallback (AMD, Intel, shared memory), not from the card.
    pub estimate: bool,
    /// For the offloading modes: `(bytes on the GPU, bytes in system RAM)`.
    pub offload: Option<(u64, u64)>,
    pub model: String,
    /// `NAME=value`, for the person to set on the Ollama server (and restart it).
    pub env: Vec<(String, String)>,
    pub num_ctx: u64,
    /// `-1` keeps the model loaded while SDC works; a person's own setting is never lowered.
    pub keep_alive: String,
    pub fits_on_gpu: bool,
    pub needs_restart: bool,
    pub notes: Vec<String>,
}

/// The largest `num_ctx` (a multiple of 1,024) whose KV cache fits beside the weights, or 0 when even the weights do not.
pub fn max_context(gpu_bytes: u64, weights: u64, shape: &Shape, kv_element_bytes: f64) -> u64 {
    let usable = (gpu_bytes as f64 * USABLE) as u64;
    let free = usable.saturating_sub(weights).saturating_sub(GRAPH_OVERHEAD);
    let tokens = (free as f64 / shape.kv_bytes_per_token(kv_element_bytes)) as u64;

    (tokens.min(shape.max_context) / 1024) * 1024
}

/// What else is known about the machine and the model beyond the bare numbers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Extra {
    pub moe: Option<Moe>,
    /// The GPU figure is a fallback, not read from the card: say so in every answer.
    pub estimate: bool,
}

/// The proposal for a model on this hardware. Pure: `hardware` and `model` in, advice out, nothing applied.
pub fn propose(hardware: &Hardware, model: &Model, current_env: &[(String, String)]) -> Proposal {
    propose_ext(hardware, model, current_env, &Extra::default())
}

/// [`propose`] with what Phase 2 adds: a mixture-of-experts model can keep its experts in system RAM and run its attention and KV
/// cache on a small GPU, and a dense model that does not fit says how much of it would run on the CPU. Fallback hardware
/// (AMD, Intel, shared memory) is marked as an estimate.
pub fn propose_ext(hardware: &Hardware, model: &Model, current_env: &[(String, String)], extra: &Extra) -> Proposal {
    let mut proposal = propose_base(hardware, model, current_env, extra);

    proposal.estimate = extra.estimate;

    if extra.estimate {
        proposal.notes.push("Estimate: this computer's GPU memory was not read from the card itself (AMD, Intel or shared memory), so treat these numbers as a starting point and check with `ollama ps`.".to_string());
    }

    let Some(gpu) = hardware.gpu_bytes else {
        return proposal;
    };
    let usable = (gpu as f64 * USABLE) as u64;
    let fits = model.weights_bytes + GRAPH_OVERHEAD <= usable;

    if let (Some(moe), false) = (extra.moe, hardware.unified) {
        /* attention, shared layers and the KV cache on the GPU; the experts in system RAM */
        let experts = (model.weights_bytes as f64 * Moe::EXPERT_SHARE) as u64;
        let on_gpu = model.weights_bytes - experts;
        let ram_needed = experts + 4 * 1024 * MIB;

        if !fits && hardware.ram_bytes >= ram_needed {
            let ctx = max_context(gpu, on_gpu, &model.shape, KV_Q8).max(2_048);

            proposal.mode = "gpu+cpu-experts";
            proposal.offload = Some((on_gpu, experts));
            proposal.num_ctx = ctx;
            proposal.fits_on_gpu = false;
            proposal.notes.push(format!(
                "{}: a mixture-of-experts model ({} experts, {} used per token). Its expert layers (about {:.1} GB) can stay in system RAM while attention and a {}-token KV cache run on the GPU: only about {:.0}% of the weights are read per token, so it is usually much faster than a dense model of the same size on the CPU. Needs about {:.1} GB of free RAM; this computer has {:.1} GB.",
                model.name,
                moe.experts,
                moe.used,
                experts as f64 / 1e9,
                ctx,
                moe.active_fraction() * 100.0,
                ram_needed as f64 / 1e9,
                hardware.ram_bytes as f64 / 1e9
            ));
        } else if !fits {
            proposal.notes.push(format!("{}: a mixture-of-experts model, but this computer's RAM ({:.1} GB) is less than the {:.1} GB its experts need beside the GPU part; a smaller model will run better.", model.name, hardware.ram_bytes as f64 / 1e9, ram_needed as f64 / 1e9));
        } else {
            proposal.notes.push(format!("{}: a mixture-of-experts model that fits on the GPU entirely; only about {:.0}% of its weights are read per token, so it runs faster than its file size suggests.", model.name, moe.active_fraction() * 100.0));
        }
    } else if !fits && !hardware.unified && model.weights_bytes > 0 {
        /* dense: the share of layers the GPU can hold after the graph and a small KV cache */
        let kv_small = (model.shape.kv_bytes_per_token(KV_Q8) * 4_096.0) as u64;
        let for_weights = usable.saturating_sub(GRAPH_OVERHEAD).saturating_sub(kv_small);
        let on_gpu = for_weights.min(model.weights_bytes);
        let on_cpu = model.weights_bytes - on_gpu;

        proposal.mode = "partial-offload";
        proposal.offload = Some((on_gpu, on_cpu));
        proposal.notes.push(format!(
            "About {:.0}% of this model ({:.1} GB) would run on the CPU and {:.1} GB on the GPU (partial offload). It works, but expect it to be several times slower than a model that fits.",
            on_cpu as f64 / model.weights_bytes as f64 * 100.0,
            on_cpu as f64 / 1e9,
            on_gpu as f64 / 1e9
        ));
    }

    proposal
}

fn propose_base(hardware: &Hardware, model: &Model, current_env: &[(String, String)], _extra: &Extra) -> Proposal {
    let mut notes = Vec::new();
    let mut env = Vec::new();
    let has = |name: &str, value: &str| current_env.iter().any(|(n, v)| n == name && v == value);

    /* Flash attention and a quantized KV cache roughly halve the KV memory: both are server-wide. */
    for (name, value) in [("OLLAMA_FLASH_ATTENTION", "1"), ("OLLAMA_KV_CACHE_TYPE", "q8_0")] {
        if !has(name, value) {
            env.push((name.to_string(), value.to_string()));
        }
    }

    let Some(gpu) = hardware.gpu_bytes else {
        notes.push("No GPU was detected, so nothing is assumed about one: the model runs on the CPU, and a smaller context is faster.".to_string());

        return Proposal { mode: "cpu", estimate: false, offload: None, model: model.name.clone(), env: Vec::new(), num_ctx: model.shape.max_context.min(8_192), keep_alive: "-1".into(), fits_on_gpu: false, needs_restart: false, notes };
    };

    if model.weights_bytes + GRAPH_OVERHEAD > (gpu as f64 * USABLE) as u64 {
        notes.push(format!(
            "The weights ({:.1} GB) do not fit in {:.1} GB with room for a context: layers will spill to the CPU and the model will be several times slower. A smaller quantization or a smaller model would stay on the GPU.",
            model.weights_bytes as f64 / 1e9,
            gpu as f64 / 1e9
        ));

        return Proposal { mode: "partial-offload", estimate: false, offload: None, model: model.name.clone(), env, num_ctx: 4_096, keep_alive: "-1".into(), fits_on_gpu: false, needs_restart: false, notes };
    }

    let kv = if has("OLLAMA_KV_CACHE_TYPE", "q8_0") { KV_Q8 } else { KV_F16 };
    let with_q8 = max_context(gpu, model.weights_bytes, &model.shape, KV_Q8);
    let fits_now = max_context(gpu, model.weights_bytes, &model.shape, kv);

    notes.push(format!(
        "{}: {:.1} GB of weights, {:.1} GB card -> at most {} tokens of context stay fully on the GPU with a q8_0 KV cache ({} with the cache as it is now).",
        model.name,
        model.weights_bytes as f64 / 1e9,
        gpu as f64 / 1e9,
        with_q8,
        fits_now
    ));

    if with_q8 < 4_096 {
        notes.push("That is under 4,096 tokens: the model barely fits. SDC will split work into smaller steps, and a smaller model would leave more room.".to_string());
    }

    if hardware.unified {
        notes.push("This Mac shares memory between the CPU and the GPU: there is no spill, but a large context competes with everything else that is open.".to_string());
    }

    Proposal { mode: "gpu", estimate: false, offload: None, model: model.name.clone(), needs_restart: !env.is_empty(), env, num_ctx: with_q8.max(2_048), keep_alive: "-1".into(), fits_on_gpu: with_q8 >= 2_048, notes }
}

impl Proposal {
    pub fn to_json(&self) -> Value {
        json!({
            "model": self.model,
            "env": self.env.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect::<Vec<_>>(),
            "numCtx": self.num_ctx,
            "keepAlive": self.keep_alive,
            "fitsOnGpu": self.fits_on_gpu,
            "mode": self.mode,
            "estimate": self.estimate,
            "offload": self.offload.map(|(gpu, cpu)| json!({ "gpuBytes": gpu, "cpuBytes": cpu })),
            "needsRestart": self.needs_restart,
            "notes": self.notes,
            "applied": false,
        })
    }
}

/// The proposal for each installed Ollama model (at most three), read from the running server: `/api/tags` for the
/// weights' size, `/api/show` for the architecture. A model whose shape the server does not report is skipped - it is
/// never given an invented one. Everything here is a read; the answer says `applied: false`.
pub fn tuning_for_ollama(models: &[String]) -> Value {
    let detected = detect_detailed();
    let hardware = detected.hardware.clone();
    let tags: Value = crate::engines::ollama::call("/api/tags", None).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
    let mut proposals = Vec::new();

    for name in models.iter().take(3) {
        let size = tags["models"].as_array().into_iter().flatten().find(|entry| entry["name"].as_str() == Some(name.as_str())).and_then(|entry| entry["size"].as_u64());
        let show: Value = crate::engines::ollama::call("/api/show", Some(&json!({ "model": name }).to_string())).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);

        if let (Some(weights_bytes), Some(shape)) = (size, shape_from_show(&show)) {
            proposals.push(propose_ext(&hardware, &Model { name: name.clone(), weights_bytes, shape }, &[], &Extra { moe: moe_from_show(&show), estimate: detected.estimate }).to_json());
        }
    }

    json!({
        "hardware": { "gpu": hardware.gpu_name, "gpuBytes": hardware.gpu_bytes, "ramBytes": hardware.ram_bytes, "unified": hardware.unified, "vendor": detected.vendor, "estimate": detected.estimate },
        "proposals": proposals,
        "note": "A proposal only: the two OLLAMA_ variables are set on the Ollama server and need it restarted, so SDC never applies them itself.",
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Spill detection
// ---------------------------------------------------------------------------------------------------------------

/// The share of a loaded model that is on the GPU, from `/api/ps` (`size_vram` over `size`); `None` when it is not loaded.
pub fn gpu_share(ps: &Value, model: &str) -> Option<f64> {
    let entry = ps["models"].as_array()?.iter().find(|entry| entry["name"].as_str() == Some(model) || entry["model"].as_str() == Some(model))?;
    let (size, vram) = (entry["size"].as_u64()?, entry["size_vram"].as_u64()?);

    Some(if size == 0 { 0.0 } else { vram as f64 / size as f64 })
}

/// A loaded model not entirely on the GPU, said in words; `None` when it is fully resident (or unified memory).
pub fn spill_warning(share: f64, unified: bool) -> Option<String> {
    if unified || share >= 0.98 {
        return None;
    }

    Some(format!("Only {:.0}% of this model is on the GPU; the rest runs on the CPU and every answer will be slower. A smaller context or a smaller model would keep it all on the GPU.", share * 100.0))
}

/// Watches tokens per second for a sudden fall - the other sign of a spill (the model reloaded half on the CPU).
pub struct SpillWatch {
    history: VecDeque<f64>,
    warned: bool,
}

impl SpillWatch {
    pub const WINDOW: usize = 6;

    pub fn new() -> Self {
        Self { history: VecDeque::new(), warned: false }
    }

    /// Feeds one step's rate. Returns a warning the first time the rate falls below half of the recent median - and not
    /// again until it has recovered.
    pub fn observe(&mut self, tokens_per_second: f64) -> Option<String> {
        if tokens_per_second <= 0.0 {
            return None;
        }

        let verdict = if self.history.len() >= 3 {
            let mut sorted: Vec<f64> = self.history.iter().copied().collect();

            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            let median = sorted[sorted.len() / 2];

            if tokens_per_second < median * 0.5 {
                if self.warned {
                    None
                } else {
                    self.warned = true;

                    Some(format!("This model just slowed from about {median:.0} to {tokens_per_second:.0} tokens per second. That usually means part of it moved from the GPU to the CPU (memory pressure from another program, or a larger context)."))
                }
            } else {
                self.warned = false;

                None
            }
        } else {
            None
        };

        /* The slow value stays out of the baseline, or one spill would teach the watch that slow is normal. */
        if verdict.is_none() && !self.warned {
            self.history.push_back(tokens_per_second);

            if self.history.len() > Self::WINDOW {
                self.history.pop_front();
            }
        }

        verdict
    }
}

impl Default for SpillWatch {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide watch the agent feeds after every local reply.
pub fn watch() -> &'static Mutex<SpillWatch> {
    static WATCH: OnceLock<Mutex<SpillWatch>> = OnceLock::new();

    WATCH.get_or_init(|| Mutex::new(SpillWatch::new()))
}

// ---------------------------------------------------------------------------------------------------------------
// The VRAM Arbiter
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The model the person chose to talk to.
    Main,
    Reranker,
    Embedding,
    Classifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Gpu,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub wanted: String,
    pub resident: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} cannot go on the GPU: {} already holds it, and a second model would push both to the CPU", self.wanted, self.resident)
    }
}

/// Decides who may be on the GPU. One main model; everything else on the CPU.
#[derive(Debug, Default)]
pub struct Arbiter {
    main: Option<String>,
}

impl Arbiter {
    /// Where `name` of this `kind` runs. A main model gets the GPU unless another main model already has it
    /// ([`Refused`]); an auxiliary model is placed on the CPU, always - it is never refused, because the CPU is where it
    /// is meant to be.
    pub fn place(&mut self, kind: Kind, name: &str) -> Result<Placement, Refused> {
        match kind {
            Kind::Main => match &self.main {
                Some(resident) if resident != name => Err(Refused { wanted: name.to_string(), resident: resident.clone() }),
                _ => {
                    self.main = Some(name.to_string());

                    Ok(Placement::Gpu)
                }
            },
            _ => Ok(Placement::Cpu),
        }
    }

    /// The main model was unloaded or replaced on purpose.
    pub fn release_main(&mut self) {
        self.main = None;
    }
}

/// The Ollama request options that enforce a placement: `num_gpu: 0` keeps every layer off the GPU.
pub fn ollama_options(placement: Placement) -> Value {
    match placement {
        Placement::Cpu => json!({ "num_gpu": 0 }),
        Placement::Gpu => json!({}),
    }
}

#[cfg(test)]
mod tests;
