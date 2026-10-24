//! **N1-2 - the quant and KV advisor** (switch `nr.quant_advice`, advice only).
//!
//! A 4-bit model loses far more on long work than on a short chat, and a 4-bit KV cache loses more again. This reads what `ollama show`
//! (or the model's tag) says about the quantization, and the KV cache type Ollama is configured with, and answers a short code the UI turns
//! into one line of advice in both languages. It changes nothing and downloads nothing; the only thing it influences is the step target -
//! the share of the window a step may fill - which falls from 80% to 60% for a 4-bit model.

/// How many bits per weight a quantization label means, `None` for a label this does not know.
pub fn bits_of(quant: &str) -> Option<f32> {
    let q = quant.trim().to_ascii_uppercase();

    if q.starts_with("F32") {
        return Some(32.0);
    }

    if q.starts_with("F16") || q.starts_with("BF16") {
        return Some(16.0);
    }

    let digits: String = q.trim_start_matches("IQ").trim_start_matches('Q').chars().take_while(char::is_ascii_digit).collect();

    if !(q.starts_with('Q') || q.starts_with("IQ")) || digits.is_empty() {
        return None;
    }

    digits.parse().ok()
}

/// The quantization named in a model's tag (`qwen3:9b-q4_K_M`), when it names one.
pub fn quant_in_tag(tag: &str) -> Option<String> {
    tag.split(['-', ':', '/']).find(|part| bits_of(part).is_some() && part.len() >= 2 && !part.chars().all(|c| c.is_ascii_digit())).map(|part| part.to_ascii_uppercase())
}

/// What is worth saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advice {
    /// The weights are 4-bit (or fewer): prefer Q5_K_M or higher when memory allows.
    FourBitWeights,
    /// The KV cache is 4-bit: never for long tasks.
    FourBitKv,
    /// Both.
    Both,
}

impl Advice {
    pub fn code(self) -> &'static str {
        match self {
            Advice::FourBitWeights => "quant4",
            Advice::FourBitKv => "kv4",
            Advice::Both => "quant4kv4",
        }
    }
}

/// Is a KV cache type 4-bit (`q4_0`, `q4_1`, `iq4_nl`)?
pub fn kv_is_four_bit(kv: &str) -> bool {
    let kv = kv.trim().to_ascii_lowercase();

    kv.starts_with("q4") || kv.starts_with("iq4") || kv.starts_with("q3") || kv.starts_with("q2")
}

/// The advice for a model, or `None` when its setup is fine (Q5 and up, KV f16 or q8).
pub fn advise(quantization: Option<&str>, kv_cache_type: Option<&str>) -> Option<Advice> {
    let weights = quantization.and_then(bits_of).is_some_and(|bits| bits < 5.0);
    let kv = kv_cache_type.is_some_and(kv_is_four_bit);

    match (weights, kv) {
        (true, true) => Some(Advice::Both),
        (true, false) => Some(Advice::FourBitWeights),
        (false, true) => Some(Advice::FourBitKv),
        (false, false) => None,
    }
}

/// The KV cache type Ollama is configured with (`OLLAMA_KV_CACHE_TYPE`), read, never set.
pub fn kv_cache_type_from_env() -> Option<String> {
    std::env::var("OLLAMA_KV_CACHE_TYPE").ok().filter(|value| !value.trim().is_empty())
}

/// The share of the window one step may fill, in percent: 80, or 60 when the weights are 4-bit.
pub fn step_target_percent(quantization: Option<&str>) -> u64 {
    if quantization.and_then(bits_of).is_some_and(|bits| bits < 5.0) {
        60
    } else {
        80
    }
}

/// The step target in tokens for a window and a quantization.
pub fn step_target(window: u64, quantization: Option<&str>) -> u64 {
    window * step_target_percent(quantization) / 100
}

/// The advice code a model row carries to the UI: from the stored quantization, else the tag; `None` when off or nothing to say.
pub fn code_for(model: &str, stored_quantization: Option<&str>) -> Option<&'static str> {
    if !crate::v3::on("nr.quant_advice") {
        return None;
    }

    let from_tag = quant_in_tag(model);
    let quantization = stored_quantization.filter(|q| !q.trim().is_empty()).or(from_tag.as_deref());

    advise(quantization, kv_cache_type_from_env().as_deref()).map(Advice::code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_read_as_bits() {
        for (label, bits) in [("Q4_K_M", 4.0), ("Q5_K_M", 5.0), ("Q8_0", 8.0), ("F16", 16.0), ("BF16", 16.0), ("IQ3_XS", 3.0), ("q4_0", 4.0)] {
            assert_eq!(bits_of(label), Some(bits), "{label}");
        }

        assert_eq!(bits_of("mystery"), None);
        assert_eq!(bits_of(""), None);
    }

    #[test]
    fn advice_appears_for_a_q4_fixture_and_not_for_a_q8_fixture() {
        assert_eq!(advise(Some("Q4_K_M"), None), Some(Advice::FourBitWeights));
        assert_eq!(advise(Some("Q8_0"), None), None);
        assert_eq!(advise(Some("Q5_K_M"), Some("f16")), None);
        assert_eq!(advise(Some("Q8_0"), Some("q4_0")), Some(Advice::FourBitKv));
        assert_eq!(advise(Some("Q4_K_M"), Some("q4_0")), Some(Advice::Both));
        assert_eq!(advise(None, None), None, "an unknown setup is not scolded");
        assert_eq!(Advice::Both.code(), "quant4kv4");
    }

    #[test]
    fn the_step_target_falls_from_eighty_to_sixty_percent_for_q4() {
        assert_eq!(step_target(10_000, Some("Q8_0")), 8_000);
        assert_eq!(step_target(10_000, Some("Q4_K_M")), 6_000);
        assert_eq!(step_target(10_000, None), 8_000);
    }

    #[test]
    fn the_tag_can_name_the_quantization() {
        assert_eq!(quant_in_tag("qwen3:9b-q4_K_M").as_deref(), Some("Q4_K_M"));
        assert_eq!(quant_in_tag("llama3.1:8b-instruct-q8_0").as_deref(), Some("Q8_0"));
        assert_eq!(quant_in_tag("qwen3:9b"), None);
    }

    #[test]
    fn the_row_code_follows_the_switch() {
        crate::v3::forcing(&["nr.quant_advice"], || {
            assert_eq!(code_for("m:9b", Some("Q4_K_M")), Some("quant4"));
            assert_eq!(code_for("m:9b", Some("Q8_0")), None);
            assert_eq!(code_for("m:9b-q4_0", None), Some("quant4"));
        });
        crate::v3::ablating(&["nr.quant_advice"], || assert_eq!(code_for("m:9b", Some("Q4_K_M")), None));
    }
}
