use super::*;

const GB: u64 = 1_000_000_000;

/// A Qwen-class 9B at Q4_K_M: 36 layers, 4 KV heads of 128, 262k context, 6.6 GB of weights (the figures the plan quotes).
fn qwen_9b() -> Model {
    Model { name: "qwen3.5:9b".into(), weights_bytes: 6_600_000_000, shape: Shape { layers: 36, kv_heads: 4, head_dim: 128, max_context: 262_144 } }
}

fn card(mib: u64) -> Hardware {
    Hardware { gpu_name: Some("test card".into()), gpu_bytes: Some(mib * MIB), ram_bytes: 32 * GB, unified: false }
}

/// The plan's test: on an 8 GB card the proposal keeps the 9B fully on the GPU, and the context it proposes really fits.
#[test]
fn on_an_8gb_card_the_proposal_keeps_the_9b_on_the_gpu_with_a_context_that_fits() {
    let model = qwen_9b();
    let proposal = propose(&card(8192), &model, &[]);

    assert!(proposal.fits_on_gpu, "{:?}", proposal.notes);
    assert!(proposal.needs_restart, "OLLAMA_FLASH_ATTENTION and OLLAMA_KV_CACHE_TYPE are server-wide: a restart is needed");
    assert_eq!(proposal.env, vec![("OLLAMA_FLASH_ATTENTION".to_string(), "1".to_string()), ("OLLAMA_KV_CACHE_TYPE".to_string(), "q8_0".to_string())]);
    assert!((10_000..=40_000).contains(&proposal.num_ctx), "the plan expects ~10-30k tokens on an 8 GB card, got {}", proposal.num_ctx);
    assert_eq!(proposal.num_ctx % 1024, 0);

    /* The arithmetic, checked independently: weights + graph + the KV cache of the proposed context fit in 95% of the card. */
    let kv = model.shape.kv_bytes_per_token(KV_Q8) * proposal.num_ctx as f64;
    let needed = model.weights_bytes as f64 + GRAPH_OVERHEAD as f64 + kv;

    assert!(needed <= 8192.0 * MIB as f64 * USABLE, "needs {needed} of {}", 8192.0 * MIB as f64);

    /* And one step more would not: the proposal is the largest, not a safe-looking number. */
    let one_more = model.weights_bytes as f64 + GRAPH_OVERHEAD as f64 + model.shape.kv_bytes_per_token(KV_Q8) * (proposal.num_ctx + 1024) as f64;

    assert!(one_more > 8192.0 * MIB as f64 * USABLE);
    assert!(!proposal.to_json()["applied"].as_bool().unwrap(), "a proposal is never applied");
}

#[test]
fn without_the_quantized_cache_the_same_card_holds_about_half_the_context() {
    let model = qwen_9b();
    let q8 = max_context(8192 * MIB, model.weights_bytes, &model.shape, KV_Q8);
    let f16 = max_context(8192 * MIB, model.weights_bytes, &model.shape, KV_F16);

    assert!(q8 > f16 && (q8 as f64 / f16 as f64 - 1.88).abs() < 0.3, "q8 {q8} vs f16 {f16}");
}

#[test]
fn settings_already_in_place_are_not_proposed_again_and_need_no_restart() {
    let in_place = [("OLLAMA_FLASH_ATTENTION".to_string(), "1".to_string()), ("OLLAMA_KV_CACHE_TYPE".to_string(), "q8_0".to_string())];
    let proposal = propose(&card(8192), &qwen_9b(), &in_place);

    assert!(proposal.env.is_empty() && !proposal.needs_restart);
}

#[test]
fn a_card_too_small_for_the_weights_says_it_will_spill_instead_of_promising_a_context() {
    let proposal = propose(&card(6144), &qwen_9b(), &[]);

    assert!(!proposal.fits_on_gpu);
    assert!(proposal.notes[0].contains("spill to the CPU"), "{:?}", proposal.notes);
    assert_eq!(proposal.num_ctx, 4_096);
}

#[test]
fn a_bigger_card_gets_a_bigger_context_up_to_the_models_own_limit() {
    let big = propose(&card(24 * 1024), &qwen_9b(), &[]);
    let small = propose(&card(8192), &qwen_9b(), &[]);

    assert!(big.num_ctx > small.num_ctx * 4);
    assert!(big.num_ctx <= 262_144);

    let limited = Model { shape: Shape { max_context: 32_768, ..qwen_9b().shape }, ..qwen_9b() };

    assert_eq!(propose(&card(48 * 1024), &limited, &[]).num_ctx, 32_768);
}

#[test]
fn no_detected_gpu_is_an_unknown_not_a_guess() {
    let none = Hardware { gpu_name: None, gpu_bytes: None, ram_bytes: 16 * GB, unified: false };
    let proposal = propose(&none, &qwen_9b(), &[]);

    assert!(!proposal.fits_on_gpu && proposal.env.is_empty());
    assert!(proposal.notes[0].contains("No GPU was detected"));
}

#[test]
fn a_mac_has_no_spill_only_pressure() {
    let mac = Hardware { gpu_name: Some("Apple silicon".into()), gpu_bytes: Some(mac_gpu_budget(16 * GB)), ram_bytes: 16 * GB, unified: true };

    assert_eq!(mac.gpu_bytes, Some(12 * GB));
    assert!(propose(&mac, &qwen_9b(), &[]).notes.iter().any(|note| note.contains("shares memory")));
    assert_eq!(spill_warning(0.5, true), None);
}

#[test]
fn nvidia_smi_output_and_a_models_shape_are_read() {
    assert_eq!(parse_nvidia("NVIDIA GeForce RTX 3070, 8192\n"), Some(("NVIDIA GeForce RTX 3070".to_string(), 8192 * MIB)));
    assert_eq!(parse_nvidia("Card A, 8192\nCard B, 24576\n").unwrap().0, "Card B", "the largest card");
    assert_eq!(parse_nvidia("No devices were found"), None);

    let show = json!({ "model_info": { "qwen3.block_count": 36, "qwen3.attention.head_count": 32, "qwen3.attention.head_count_kv": 4, "qwen3.embedding_length": 4096, "qwen3.context_length": 40960 } });
    let shape = shape_from_show(&show).unwrap();

    assert_eq!((shape.layers, shape.kv_heads, shape.head_dim, shape.max_context), (36, 4, 128, 40_960), "head_dim falls back to embedding / heads");
    assert!(shape_from_show(&json!({})).is_none());
}

#[test]
fn a_model_not_fully_on_the_gpu_is_a_spill_and_the_words_say_how_much() {
    let ps = json!({ "models": [{ "name": "qwen3.5:9b", "size": 8_000_000_000u64, "size_vram": 6_000_000_000u64 }] });
    let share = gpu_share(&ps, "qwen3.5:9b").unwrap();

    assert!((share - 0.75).abs() < 1e-9);
    assert!(spill_warning(share, false).unwrap().contains("75%"));
    assert_eq!(spill_warning(1.0, false), None);
    assert_eq!(gpu_share(&ps, "other"), None);
}

#[test]
fn a_sudden_fall_in_speed_warns_once_and_does_not_teach_the_watch_that_slow_is_normal() {
    let mut watch = SpillWatch::new();

    for rate in [30.0, 31.0, 29.0, 30.5] {
        assert_eq!(watch.observe(rate), None);
    }

    assert!(watch.observe(9.0).unwrap().contains("slowed from about 30 to 9"));
    assert_eq!(watch.observe(8.0), None, "once, not at every slow step");
    assert_eq!(watch.observe(9.5), None);
    assert_eq!(watch.observe(30.0), None, "recovered");
    assert!(watch.observe(8.0).is_some(), "and it can warn again after a recovery");
}

#[test]
fn a_normal_variation_is_not_a_spill() {
    let mut watch = SpillWatch::new();

    for rate in [30.0, 24.0, 33.0, 27.0, 22.0, 35.0, 26.0] {
        assert_eq!(watch.observe(rate), None, "{rate}");
    }
}

/// The plan's test: the arbiter refuses a second GPU model, and sends everything else to the CPU.
#[test]
fn the_arbiter_refuses_a_second_main_model_and_puts_every_auxiliary_model_on_the_cpu() {
    let mut arbiter = Arbiter::default();

    assert_eq!(arbiter.place(Kind::Main, "qwen3.5:9b"), Ok(Placement::Gpu));
    assert_eq!(arbiter.place(Kind::Main, "qwen3.5:9b"), Ok(Placement::Gpu), "the same model again is the same model");

    let refused = arbiter.place(Kind::Main, "llama3.1:8b").unwrap_err();

    assert_eq!((refused.wanted.as_str(), refused.resident.as_str()), ("llama3.1:8b", "qwen3.5:9b"));
    assert!(refused.to_string().contains("cannot go on the GPU"));

    for kind in [Kind::Reranker, Kind::Embedding, Kind::Classifier] {
        assert_eq!(arbiter.place(kind, "bge-small"), Ok(Placement::Cpu), "{kind:?} is never on the GPU");
    }

    arbiter.release_main();

    assert_eq!(arbiter.place(Kind::Main, "llama3.1:8b"), Ok(Placement::Gpu), "after the first is unloaded on purpose");
}

#[test]
fn a_cpu_placement_becomes_the_ollama_option_that_enforces_it() {
    assert_eq!(ollama_options(Placement::Cpu), json!({ "num_gpu": 0 }));
    assert_eq!(ollama_options(Placement::Gpu), json!({}));
}

#[test]
fn this_machine_can_be_read_without_panicking() {
    let hardware = detect();

    assert!(hardware.ram_bytes > 0 || hardware.gpu_bytes.is_none() || hardware.gpu_bytes.is_some());
}

// ---- Phase 2.4: Apple silicon, AMD and Intel fallbacks, mixture-of-experts and CPU offload -----------------------------

fn moe_30b() -> Model {
    /* a 30B-class MoE at 4 bit: ~18 GB of weights, 48 layers, 4 KV heads of 128 */
    Model { name: "qwen3-moe:30b".into(), weights_bytes: 18 * GB, shape: Shape { layers: 48, kv_heads: 4, head_dim: 128, max_context: 262_144 } }
}

#[test]
fn an_moe_that_does_not_fit_keeps_its_experts_in_ram_and_runs_attention_on_the_gpu() {
    let proposal = propose_ext(&card(8192), &moe_30b(), &[], &Extra { moe: Some(Moe { experts: 128, used: 8 }), estimate: false });

    assert_eq!(proposal.mode, "gpu+cpu-experts");
    assert!(proposal.offload.is_some_and(|(gpu, cpu)| gpu < 3 * GB && cpu > 15 * GB), "{:?}", proposal.offload);
    assert!(proposal.num_ctx >= 2_048);
    assert!(proposal.notes.iter().any(|note| note.contains("mixture-of-experts") && note.contains("system RAM")), "{:?}", proposal.notes);
    assert!(Moe { experts: 128, used: 8 }.active_fraction() < 0.2, "only a fraction of the weights is read per token");
}

#[test]
fn an_moe_with_too_little_ram_says_so_instead_of_promising_a_speed() {
    let small_ram = Hardware { ram_bytes: 16 * GB, ..card(8192) };
    let proposal = propose_ext(&small_ram, &moe_30b(), &[], &Extra { moe: Some(Moe { experts: 128, used: 8 }), estimate: false });

    assert_ne!(proposal.mode, "gpu+cpu-experts");
    assert!(proposal.notes.iter().any(|note| note.contains("RAM") && note.contains("smaller model")), "{:?}", proposal.notes);
}

#[test]
fn a_dense_model_that_does_not_fit_reports_how_much_would_run_on_the_cpu() {
    let proposal = propose_ext(&card(6144), &qwen_9b(), &[], &Extra::default());

    assert_eq!(proposal.mode, "partial-offload");
    assert!(proposal.offload.is_some_and(|(gpu, cpu)| gpu > 0 && cpu > 0 && gpu + cpu == qwen_9b().weights_bytes));
    assert!(proposal.notes.iter().any(|note| note.contains("partial offload")));
}

#[test]
fn fallback_hardware_is_always_labelled_an_estimate() {
    let proposal = propose_ext(&card(8192), &qwen_9b(), &[], &Extra { moe: None, estimate: true });

    assert!(proposal.estimate);
    assert!(proposal.notes.iter().any(|note| note.starts_with("Estimate:")), "{:?}", proposal.notes);
    assert_eq!(proposal.to_json()["estimate"], true);
    assert!(!propose(&card(8192), &qwen_9b(), &[]).estimate, "a card read directly is not an estimate");
}

#[test]
fn amd_and_windows_adapters_are_read_and_vendors_told_apart() {
    let rocm = r#"{"card0": {"Card series": "Radeon RX 7900 XTX", "VRAM Total Memory (B)": "25753026560"}, "card1": {"VRAM Total Memory (B)": "536870912"}}"#;

    assert_eq!(parse_rocm(rocm), Some(("Radeon RX 7900 XTX".to_string(), 25_753_026_560)));
    assert_eq!(parse_rocm("not json"), None);
    assert_eq!(parse_windows_adapters("Intel(R) UHD Graphics|134217728;AMD Radeon RX 6700 XT|12868124672"), Some(("AMD Radeon RX 6700 XT".to_string(), 12_868_124_672)));
    assert_eq!(parse_windows_adapters(""), None);
    assert_eq!(vendor_of("NVIDIA GeForce RTX 4060"), "nvidia");
    assert_eq!(vendor_of("AMD Radeon RX 6700 XT"), "amd");
    assert_eq!(vendor_of("Intel(R) Arc(TM) A770"), "intel");
    assert!(intel_is_integrated("Intel(R) UHD Graphics 770") && !intel_is_integrated("Intel(R) Arc(TM) A770"));
}

#[test]
fn an_moe_is_recognised_from_the_architecture_a_dense_model_is_not() {
    let moe = json!({ "model_info": { "qwen3moe.expert_count": 128, "qwen3moe.expert_used_count": 8 } });
    let dense = json!({ "model_info": { "qwen3.block_count": 36 } });

    assert_eq!(moe_from_show(&moe), Some(Moe { experts: 128, used: 8 }));
    assert_eq!(moe_from_show(&dense), None);
}

#[test]
fn this_computers_hardware_names_where_its_numbers_came_from() {
    let detected = detect_detailed();

    assert!(["nvidia", "apple", "amd", "intel", "none"].contains(&detected.vendor));
    assert_eq!(detected.hardware, detect());

    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert_eq!(detected.vendor, "apple");
        assert!(detected.hardware.unified && !detected.estimate, "Apple silicon is read from the machine, not guessed");
    }
}
