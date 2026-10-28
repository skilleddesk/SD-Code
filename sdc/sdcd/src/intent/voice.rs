//! Voice: speech to text, then the same Intent Engine as typing.
//!
//! Local first, because a person's voice is theirs: **whisper.cpp** (`whisper-cli`) or OpenAI's
//! `whisper` command, when installed, with a model file under `<data>/whisper/` (or `SDC_WHISPER_MODEL`).
//! When there is none, a connected provider that transcribes - Groq, Google (Gemini), Alibaba (Qwen3-ASR), then OpenAI - is used, each tried in turn,, and the
//! answer says which one heard it, so a person always knows whether their voice left the machine.
//!
//! The window records 16 kHz mono WAV (it encodes the PCM itself), which is what whisper.cpp reads
//! without `ffmpeg`.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

/// The model file whisper.cpp should use: `SDC_WHISPER_MODEL`, else an installed `*.bin` under `<data>/whisper/` - for a
/// language the small models handle well (English, German, French, Spanish...) the smallest, otherwise the largest
/// (measured: only the large models read Arabic and Bengali; docs/VOICE.md).
fn whisper_model_for(language: Option<&str>) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SDC_WHISPER_MODEL") {
        let path = PathBuf::from(path);

        if path.is_file() {
            return Some(path);
        }
    }

    let directory = crate::paths::data_dir().ok()?.join("whisper");
    let mut models: Vec<(u64, PathBuf)> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "bin"))
        .map(|path| (path.metadata().map(|meta| meta.len()).unwrap_or(0), path))
        .collect();

    models.sort();

    let small_is_enough = matches!(language, Some("en" | "de" | "fr" | "es" | "pt" | "it" | "nl"));

    if small_is_enough { models.into_iter().next() } else { models.into_iter().next_back() }.map(|(_, path)| path)
}

fn whisper_model() -> Option<PathBuf> {
    whisper_model_for(None)
}

/// What this machine can do: `{local, program, model, online: [providers]}` - the voice button's tooltip
/// and the doctor row.
pub fn status() -> Value {
    let program = ["whisper-cli", "whisper-cpp", "whisper"].into_iter().find(|program| crate::host::program::resolve(program).is_some());
    let model = whisper_model();
    let model_path = model.clone();
    let online: Vec<&str> = CLOUD_ORDER
        .into_iter()
        .filter(|id| crate::auth::keychain::get(&crate::providers::key_ref(id)).is_some())
        .collect();
    let local = match program {
        Some("whisper") => true,
        Some(_) => model.is_some(),
        None => false,
    };

    json!({
        "local": local,
        "program": program,
        "model": model.map(|path| path.display().to_string()),
        "online": online,
        "programMissing": program.is_none(),
        "modelMissing": model_path.is_none() && program != Some("whisper"),
        "installHint": if program.is_none() { json!(program_hint()) } else { Value::Null },
        "models": crate::intent::voice_model::catalogue(),
        "download": crate::intent::voice_model::progress(),
        "available": local || !online.is_empty(),
        "hint": if local || !online.is_empty() {
            Value::Null
        } else {
            json!("Voice needs a transcriber: connect Groq (a free key), Google Gemini (a free key), Alibaba Cloud (Qwen) or OpenAI in the Provider Hub - nothing to install. (Fully offline voice uses whisper.cpp with a model in SDC's data folder under whisper/, when you have it.)")
        },
    })
}

/// How to get the whisper.cpp program on this operating system (SDC downloads the model, not the program).
fn program_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install the program once: `brew install whisper-cpp` (it provides `whisper-cli`). SDC then downloads the speech model itself, after you say yes."
    } else if cfg!(windows) {
        "Install the program once: download `whisper-bin-x64.zip` from github.com/ggml-org/whisper.cpp/releases and put `whisper-cli.exe` on your PATH. SDC then downloads the speech model itself, after you say yes."
    } else {
        "Install the program once: build whisper.cpp (github.com/ggml-org/whisper.cpp) or install your distribution's `whisper-cpp` package so `whisper-cli` is on your PATH. SDC then downloads the speech model itself, after you say yes."
    }
}

/// One connection pool for every transcription (1.0): live captions send a request every second or two while
/// the person speaks, and a fresh agent paid a new TLS handshake for each.
fn shared_agent() -> ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();

    AGENT.get_or_init(|| ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(90)).build()).clone()
}

/// The cloud transcribers in the order they are tried, by provider id (the keychain name is `providers::key_ref(id)`).
/// Groq's Whisper large-v3 has a free tier; Google's Gemini API lists `gemini-3.5-transcribe` as free of charge; Alibaba's
/// Qwen3-ASR and OpenAI's whisper-1 follow, for the keys a person already has. Evidence and dates: `docs/VOICE.md`.
const CLOUD_ORDER: [&str; 4] = ["groq", "google", "qwen", "openai-api"];

/// Where each cloud transcriber is asked. A struct so a test can point it at a local mock server.
#[derive(Clone)]
pub struct Endpoints {
    pub groq: String,
    pub gemini: String,
    pub openai: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            groq: "https://api.groq.com/openai/v1/audio/transcriptions".into(),
            gemini: "https://generativelanguage.googleapis.com".into(),
            openai: "https://api.openai.com/v1/audio/transcriptions".into(),
        }
    }
}

/// One step of the chain, as the person is told about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt {
    pub engine: String,
    pub local: bool,
    /// `None`: it heard something. `Some(why)`: it could not, in words.
    pub error: Option<String>,
}

/// Transcribes one recording. `language` is a hint (`bn`, `ar`, …) or `None` for automatic. `engine` is the person's
/// choice in Settings: `None` or `auto` runs the whole chain; `local` or a provider id (`groq`, `google`, `qwen`,
/// `openai-api`) uses only that one.
///
/// 1.0.3, the report "it hears me for a short while and then stops": a live caption asked again every second or two
/// used up a free key's requests per minute (Groq, Gemini) within the first minute, and from then on every request -
/// the phrases that are kept included - came back `429`. Now a provider that says "too many requests" **rests** for
/// as long as it asked (`Retry-After`, else 20 s) and is not asked meanwhile; a live caption (`interim`) asks one
/// rested service only, never the whole chain; and a refusal because everything is resting says how long to wait
/// (`retry_after`), so the window waits and asks again instead of losing what was said.
pub fn transcribe(audio: &[u8], mime: &str, language: Option<&str>, engine: Option<&str>, interim: bool) -> Result<Value, String> {
    let keys: Vec<(&str, String)> = CLOUD_ORDER
        .iter()
        .filter_map(|id| crate::auth::keychain::get(&crate::providers::key_ref(id)).map(|key| (*id, key)))
        .collect();
    let ready: Vec<(&str, String)> = keys.iter().filter(|(id, _)| resting_for(id).is_none()).cloned().collect();
    let chosen = engine.map(str::trim).filter(|engine| !engine.is_empty() && *engine != "auto" && *engine != "local");

    /* The one service the person chose is resting: say so, with the wait, rather than "it has no key". */
    if let Some(chosen) = chosen {
        if let Some(left) = resting_for(chosen) {
            return Err(resting_sentence(&[chosen], left));
        }
    }

    /* Nothing local, and every cloud service with a key is resting. */
    if chosen.is_none() && ready.is_empty() && !keys.is_empty() && !has_local() {
        let names: Vec<&str> = keys.iter().map(|(id, _)| *id).collect();

        return Err(resting_sentence(&names, retry_after().unwrap_or(Duration::from_secs(5))));
    }

    let asked: Vec<(&str, String)> = if interim && chosen.is_none() { ready.into_iter().take(1).collect() } else { ready };

    transcribe_choosing(&Endpoints::default(), &asked, engine, audio, mime, language)
}

type RestMap = std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>;

/// The services that asked to be left alone, and until when (1.0.3).
fn resting() -> &'static RestMap {
    static RESTING: std::sync::OnceLock<RestMap> = std::sync::OnceLock::new();

    RESTING.get_or_init(Default::default)
}

/// A `429` from `provider`: it rests for `retry_after` (its `Retry-After` header, in seconds), else 20 s.
pub fn note_too_many(provider: &str, retry_after: Option<&str>) {
    let seconds = retry_after.and_then(|value| value.trim().parse::<f64>().ok()).filter(|seconds| seconds.is_finite() && *seconds > 0.0).unwrap_or(20.0).min(300.0);

    if let Ok(mut map) = resting().lock() {
        map.insert(provider.to_string(), std::time::Instant::now() + Duration::from_secs_f64(seconds));
    }
}

/// How long `provider` still rests, if it does.
pub fn resting_for(provider: &str) -> Option<Duration> {
    let map = resting().lock().ok()?;
    let until = *map.get(provider)?;

    until.checked_duration_since(std::time::Instant::now()).filter(|left| !left.is_zero())
}

/// The shortest rest left among the services that are resting: when the window may ask again.
pub fn retry_after() -> Option<Duration> {
    let now = std::time::Instant::now();

    resting().lock().ok()?.values().filter_map(|until| until.checked_duration_since(now)).filter(|left| !left.is_zero()).min()
}

fn resting_sentence(names: &[&str], left: Duration) -> String {
    format!("{} asked SDC to slow down (too many requests); SDC asks again in {} s.", names.join(", "), left.as_secs().max(1))
}

/// A local transcriber is installed and has what it needs (the same test as `status().local`).
fn has_local() -> bool {
    match ["whisper-cli", "whisper-cpp", "whisper"].into_iter().find(|program| crate::host::program::resolve(program).is_some()) {
        Some("whisper") => true,
        Some(_) => whisper_model().is_some(),
        None => false,
    }
}

/// The chain narrowed to the engine the person chose. A chosen engine is the only one tried: picking Gemini must
/// mean Gemini heard it, and a failure says so instead of quietly being answered by another service.
pub fn transcribe_choosing(endpoints: &Endpoints, keys: &[(&str, String)], engine: Option<&str>, audio: &[u8], mime: &str, language: Option<&str>) -> Result<Value, String> {
    match engine.map(str::trim).filter(|engine| !engine.is_empty() && *engine != "auto") {
        None => transcribe_with(endpoints, keys, true, audio, mime, language),
        Some("local") => {
            if audio.len() < 1000 {
                return Err("The recording is too short to hear anything in it.".to_string());
            }

            match local(audio, mime, language) {
                Some(Ok(heard)) if !heard["text"].as_str().unwrap_or_default().trim().is_empty() => Ok(heard),
                Some(Ok(_)) => Err("You chose the local transcriber (whisper.cpp), and it heard no words in the recording.".to_string()),
                Some(Err(error)) => Err(format!("You chose the local transcriber (whisper.cpp), and it failed: {error}")),
                None => Err("You chose the local transcriber, but whisper.cpp is not installed or has no model file. Pick another engine in Settings → Language, or install it.".to_string()),
            }
        }
        Some(chosen) => {
            let Some(id) = CLOUD_ORDER.iter().find(|id| **id == chosen) else {
                return Err(format!("`{chosen}` is not a transcriber SDC knows. Pick one in Settings → Language."));
            };
            let only: Vec<(&str, String)> = keys.iter().filter(|(provider, _)| provider == id).cloned().collect();

            if only.is_empty() {
                return Err(format!("You chose {chosen} for voice, but it has no key. Add it in the Provider Hub, or pick another engine in Settings → Language."));
            }

            transcribe_with(endpoints, &only, false, audio, mime, language)
        }
    }
}

/// The provider chain (Phase 1.3): local whisper.cpp first, then every cloud transcriber the person has a key for, **each
/// tried in turn** - a refused key (403), a missing model file or a broken network moves on to the next instead of ending
/// the dictation. The answer says which engine heard it and whether it left the machine; when nothing could, the error
/// lists every attempt and why, never a bare "failed".
pub fn transcribe_with(endpoints: &Endpoints, keys: &[(&str, String)], try_local: bool, audio: &[u8], mime: &str, language: Option<&str>) -> Result<Value, String> {
    if audio.len() < 1000 {
        return Err("The recording is too short to hear anything in it.".to_string());
    }

    let mut tried: Vec<Attempt> = Vec::new();

    if try_local {
        if let Some(result) = local(audio, mime, language) {
            match result {
                Ok(heard) if !heard["text"].as_str().unwrap_or_default().trim().is_empty() => return Ok(with_tried(heard, &tried)),
                Ok(_) => tried.push(Attempt { engine: heard_engine(&result), local: true, error: Some("it heard no words in the recording".into()) }),
                Err(error) => tried.push(Attempt { engine: "whisper".into(), local: true, error: Some(error) }),
            }
        }
    }

    for (provider, key) in keys {
        let heard = match *provider {
            "qwen" => qwen_online(key, audio, mime, language),
            "google" => gemini_online(&endpoints.gemini, key, audio, mime, language),
            "groq" => online("groq", &endpoints.groq, key, "whisper-large-v3-turbo", audio, mime, language),
            "openai-api" => online("openai-api", &endpoints.openai, key, "whisper-1", audio, mime, language),
            _ => continue,
        };

        match heard {
            Ok(text) if !text.trim().is_empty() => return Ok(with_tried(json!({ "text": text, "engine": provider, "local": false }), &tried)),
            Ok(_) => tried.push(Attempt { engine: (*provider).to_string(), local: false, error: Some("it heard no words in the recording".into()) }),
            Err(error) => tried.push(Attempt { engine: (*provider).to_string(), local: false, error: Some(error) }),
        }
    }

    if tried.is_empty() {
        return Err(status()["hint"].as_str().unwrap_or("No transcriber is available.").to_string());
    }

    Err(format!(
        "No transcriber could hear this. Tried: {}.",
        tried.iter().map(|attempt| format!("{} ({})", attempt.engine, attempt.error.as_deref().unwrap_or("no words"))).collect::<Vec<_>>().join("; ")
    ))
}

fn heard_engine(result: &Result<Value, String>) -> String {
    result.as_ref().ok().and_then(|heard| heard["engine"].as_str()).unwrap_or("whisper").to_string()
}

/// The success answer, with the engines that failed before it (so the window can say "Groq refused, Gemini heard").
fn with_tried(mut heard: Value, tried: &[Attempt]) -> Value {
    heard["tried"] = json!(tried.iter().map(|attempt| json!({ "engine": attempt.engine, "local": attempt.local, "error": attempt.error })).collect::<Vec<_>>());
    heard
}

/// Google's Gemini API: the recording goes in as inline audio of a `generateContent` call, and the transcript comes back as
/// the model's text. `gemini-3.5-transcribe` first (free of charge on the Gemini API pricing page, 2026-10-18); a model the
/// key cannot use (HTTP 404) falls back to `gemini-2.5-flash`, which is free too.
fn gemini_online(base: &str, key: &str, audio: &[u8], mime: &str, language: Option<&str>) -> Result<String, String> {
    use base64::Engine;

    let mime = if mime.contains("wav") { "audio/wav" } else if mime.contains("webm") { "audio/webm" } else { "audio/ogg" };
    let hint = language.filter(|language| *language != "auto" && !language.is_empty()).map(|language| format!(" The language is {language}.")).unwrap_or_default();
    let body = json!({
        "contents": [{ "parts": [
            { "text": format!("Transcribe this audio exactly as spoken, in the language spoken. Do not translate. Do not add comments. Answer with the transcript only.{hint}") },
            { "inline_data": { "mime_type": mime, "data": base64::engine::general_purpose::STANDARD.encode(audio) } },
        ] }],
    });
    let mut last = String::new();

    for model in ["gemini-3.5-transcribe", "gemini-3.8-flash", "gemini-2.5-flash"] {
        let url = format!("{}/v1beta/models/{model}:generateContent", base.trim_end_matches('/'));

        match shared_agent().post(&url).set("x-goog-api-key", key).set("content-type", "application/json").send_string(&body.to_string()) {
            Ok(response) => {
                let mut text = String::new();

                response.into_reader().take(1_000_000).read_to_string(&mut text).map_err(|error| error.to_string())?;

                let answer: Value = serde_json::from_str(&text).map_err(|_| format!("Gemini answered something that is not JSON: {}", text.chars().take(200).collect::<String>()))?;

                let heard = gemini_text(&answer);

                /* A model that answers with nothing is not an answer: the next one is asked. */
                if heard.is_empty() {
                    last = format!("{model} heard nothing in the recording");
                    continue;
                }

                return Ok(heard);
            }
            Err(ureq::Error::Status(404, response)) => last = format!("{model} is not available to this key (HTTP 404): {}", response.into_string().unwrap_or_default().chars().take(160).collect::<String>()),
            Err(ureq::Error::Status(code, response)) => {
                if code == 429 {
                    note_too_many("google", response.header("retry-after"));
                }

                return Err(format!("Gemini refused the recording (HTTP {code}): {}", response.into_string().unwrap_or_default().chars().take(300).collect::<String>()));
            }
            Err(error) => return Err(format!("Gemini could not be reached: {error}")),
        }
    }

    Err(last)
}

/// The transcript in a `generateContent` answer: every part of the first candidate. `gemini-3.5-transcribe` puts it in
/// `audioTranscription.text` (seen live, 2026-10-28: an empty transcript for every recording until this was read); a chat model
/// puts it in `text`.
pub fn gemini_text(answer: &Value) -> String {
    answer["candidates"][0]["content"]["parts"]
        .as_array()
        .map(|parts| parts.iter().filter_map(|part| part["audioTranscription"]["text"].as_str().or_else(|| part["text"].as_str())).map(str::trim).filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

fn local(audio: &[u8], mime: &str, language: Option<&str>) -> Option<Result<Value, String>> {
    let program = ["whisper-cli", "whisper-cpp", "whisper"].into_iter().find(|program| crate::host::program::resolve(program).is_some())?;
    let directory = std::env::temp_dir().join(format!("sdc-voice-{}", uuid::Uuid::new_v4().simple()));

    if std::fs::create_dir_all(&directory).is_err() {
        return None;
    }

    let extension = if mime.contains("wav") { "wav" } else if mime.contains("webm") { "webm" } else { "ogg" };
    let file = directory.join(format!("speech.{extension}"));

    if std::fs::write(&file, audio).is_err() {
        return None;
    }

    let language = language.unwrap_or("auto");
    let output = if program == "whisper" {
        crate::host::program::command(program).map(|mut command| {
            command
                .arg(&file)
                .args(["--model", "small", "--output_format", "txt", "--output_dir"])
                .arg(&directory)
                .args(if language == "auto" { vec![] } else { vec!["--language".to_string(), language.to_string()] })
                .output()
        })
    } else {
        let model = whisper_model_for(Some(language))?;

        crate::host::program::command(program).map(|mut command| {
            command.arg("-m").arg(&model).arg("-f").arg(&file).args(["-l", language, "-nt", "-np"]).output()
        })
    };

    let result = match output {
        Some(Ok(output)) if output.status.success() => {
            let text = if program == "whisper" {
                std::fs::read_to_string(directory.join("speech.txt")).unwrap_or_default()
            } else {
                String::from_utf8_lossy(&output.stdout).to_string()
            };

            Ok(json!({ "text": text.trim(), "engine": program, "local": true }))
        }
        Some(Ok(output)) => Err(format!("{program} could not read the recording: {}", String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or("no reason given"))),
        Some(Err(error)) => Err(format!("{program} did not start: {error}")),
        None => return None,
    };

    let _ = std::fs::remove_dir_all(&directory);

    Some(result)
}

/// Qwen3-ASR through Model Studio's OpenAI-compatible chat endpoint: the recording goes in as a `data:` URL in an
/// `input_audio` part, and the transcript comes back as the message. The endpoint is the one the person's Alibaba key
/// was placed on (a workspace URL or a region, `providers::place_key`), so a US key is asked in the US.
fn qwen_online(key: &str, audio: &[u8], mime: &str, language: Option<&str>) -> Result<String, String> {
    use base64::Engine;

    let endpoint = crate::engines::native_api::endpoint_for("qwen3-asr-flash", Some("qwen"));
    let mime = if mime.contains("wav") { "audio/wav" } else if mime.contains("webm") { "audio/webm" } else { "audio/ogg" };
    let data = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(audio));
    let agent = shared_agent();
    let post = |url: &str, body: &Value| -> Result<Value, (u16, String)> {
        match agent.post(url).set("authorization", &format!("Bearer {key}")).set("content-type", "application/json").send_string(&body.to_string()) {
            Ok(response) => {
                let mut text = String::new();

                response.into_reader().take(1_000_000).read_to_string(&mut text).map_err(|error| (0, error.to_string()))?;

                serde_json::from_str(&text).map_err(|_| (0, format!("the transcriber answered something that is not JSON: {}", text.chars().take(200).collect::<String>())))
            }
            Err(ureq::Error::Status(code, response)) => {
                if code == 429 {
                    note_too_many("qwen", response.header("retry-after"));
                }

                Err((code, response.into_string().unwrap_or_default().chars().take(300).collect()))
            }
            Err(error) => Err((0, format!("The transcriber could not be reached: {error}"))),
        }
    };

    /* Model Studio's own (DashScope) endpoint first: on some regions - the US one, measured 2026-10-09 - Qwen3-ASR is
       not offered through the OpenAI-compatible path at all ("Unsupported model for OpenAI compatibility mode"). */
    let native = qwen_native_url(&endpoint.url);
    let first = match native.as_deref() {
        Some(url) => post(url, &qwen_native_body(&data, language)).map(|answer| qwen_native_text(&answer)),
        None => Err((404, String::new())),
    };

    match first {
        Ok(text) => Ok(text),
        Err((401 | 403, body)) => Err(format!("Alibaba refused the key for speech (HTTP 401/403): {body}")),
        Err(native_error) => {
            let mut body = qwen_body(&data, language);

            body["stream"] = json!(false);

            post(&endpoint.url, &body).map(|answer| qwen_text(&answer)).map_err(|(code, body)| {
                format!("The transcriber refused the recording (HTTP {code}): {body} (and the native endpoint said: HTTP {} {})", native_error.0, native_error.1)
            })
        }
    }
}

/// The DashScope-native multimodal URL on the same host as a compatible-mode chat URL:
/// `https://dashscope-us.aliyuncs.com/compatible-mode/v1/chat/completions` →
/// `https://dashscope-us.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation`.
pub fn qwen_native_url(chat_url: &str) -> Option<String> {
    let host = chat_url.split("/compatible-mode/").next().filter(|host| *host != chat_url)?;

    Some(format!("{host}/api/v1/services/aigc/multimodal-generation/generation"))
}

/// The native request: the recording as the user's message, the language (when known) as an option.
pub fn qwen_native_body(data_url: &str, language: Option<&str>) -> Value {
    let mut body = json!({
        "model": "qwen3-asr-flash",
        "input": { "messages": [{ "role": "user", "content": [{ "audio": data_url }] }] },
        "parameters": { "asr_options": { "enable_itn": false } },
    });

    if let Some(language) = language.filter(|language| *language != "auto" && !language.is_empty()) {
        body["parameters"]["asr_options"]["language"] = json!(language);
    }

    body
}

/// The transcript in a native answer: `output.choices[0].message.content[0].text`.
pub fn qwen_native_text(answer: &Value) -> String {
    answer["output"]["choices"][0]["message"]["content"]
        .as_array()
        .map(|parts| parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The request body of one Qwen3-ASR call. `language` is a hint (`bn`, `zh`…); without one the model detects it.
pub fn qwen_body(data_url: &str, language: Option<&str>) -> Value {
    let mut body = json!({
        "model": "qwen3-asr-flash",
        "messages": [{ "role": "user", "content": [{ "type": "input_audio", "input_audio": { "data": data_url } }] }],
    });

    if let Some(language) = language.filter(|language| *language != "auto" && !language.is_empty()) {
        body["asr_options"] = json!({ "language": language, "enable_itn": false });
    }

    body
}

/// The transcript in a Qwen3-ASR answer: `choices[0].message.content`, a string (or parts with a `text`).
pub fn qwen_text(answer: &Value) -> String {
    let content = &answer["choices"][0]["message"]["content"];

    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(parts) => parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join(" ").trim().to_string(),
        _ => String::new(),
    }
}

/// An OpenAI-compatible `audio/transcriptions` call, multipart by hand (the one form upload SDC makes).
fn online(provider: &str, url: &str, key: &str, model: &str, audio: &[u8], mime: &str, language: Option<&str>) -> Result<String, String> {
    let boundary = format!("sdc{}", uuid::Uuid::new_v4().simple());
    let extension = if mime.contains("wav") { "wav" } else if mime.contains("webm") { "webm" } else { "ogg" };
    let mut body = Vec::new();
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    };

    field("model", model);
    field("response_format", "json");

    if let Some(language) = language.filter(|language| *language != "auto") {
        field("language", language);
    }

    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.{extension}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(audio);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let agent = shared_agent();
    let response = agent
        .post(url)
        .set("authorization", &format!("Bearer {key}"))
        .set("content-type", &format!("multipart/form-data; boundary={boundary}"))
        .send_bytes(&body);

    match response {
        Ok(response) => {
            let mut text = String::new();

            response.into_reader().take(1_000_000).read_to_string(&mut text).map_err(|error| error.to_string())?;

            let value: Value = serde_json::from_str(&text).map_err(|_| format!("the transcriber answered something that is not JSON: {}", text.chars().take(200).collect::<String>()))?;

            Ok(value["text"].as_str().unwrap_or_default().trim().to_string())
        }
        Err(ureq::Error::Status(code, response)) => {
            if code == 429 {
                note_too_many(provider, response.header("retry-after"));
            }

            let body = response.into_string().unwrap_or_default();

            Err(format!("The transcriber refused the recording (HTTP {code}): {}", body.chars().take(300).collect::<String>()))
        }
        Err(error) => Err(format!("The transcriber could not be reached: {error}")),
    }
}

/// `sdcd voice status` and `sdcd voice FILE.wav [--lang bn] [--no-local]`: the same chain the app uses, from a terminal - how a person
/// (or a test) finds out why dictation does not work, with the engine, whether it left the machine, and every failed attempt.
/// A key is never printed.
pub fn main(args: &[String]) -> i32 {
    if args.first().map(String::as_str) == Some("status") || args.is_empty() {
        println!("{}", serde_json::to_string_pretty(&status()).unwrap_or_default());

        return 0;
    }

    let file = &args[0];
    let language = args.iter().position(|arg| arg == "--lang").and_then(|at| args.get(at + 1)).map(String::as_str);
    let try_local = !args.iter().any(|arg| arg == "--no-local");
    let Ok(audio) = std::fs::read(file) else {
        eprintln!("cannot read {file}");

        return 2;
    };
    let keys: Vec<(&str, String)> = CLOUD_ORDER.iter().filter_map(|id| crate::auth::keychain::get(&crate::providers::key_ref(id)).map(|key| (*id, key))).collect();
    let started = std::time::Instant::now();

    let engine = args.iter().position(|arg| arg == "--engine").and_then(|at| args.get(at + 1)).map(String::as_str);
    let result = match engine {
        Some(_) => transcribe_choosing(&Endpoints::default(), &keys, engine, &audio, "audio/wav", language),
        None => transcribe_with(&Endpoints::default(), &keys, try_local, &audio, "audio/wav", language),
    };

    match result {
        Ok(heard) => {
            println!("engine: {} (local: {})", heard["engine"].as_str().unwrap_or("?"), heard["local"]);

            for attempt in heard["tried"].as_array().into_iter().flatten() {
                println!("  failed first: {} - {}", attempt["engine"].as_str().unwrap_or("?"), attempt["error"].as_str().unwrap_or("?"));
            }

            println!("seconds: {:.1}", started.elapsed().as_secs_f32());
            println!("text: {}", heard["text"].as_str().unwrap_or_default());

            0
        }
        Err(error) => {
            println!("FAILED: {error}");

            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_that_said_too_many_requests_rests_for_as_long_as_it_asked() {
        note_too_many("test-rest-a", Some("12"));
        note_too_many("test-rest-b", None);

        let a = resting_for("test-rest-a").unwrap().as_secs();
        let b = resting_for("test-rest-b").unwrap().as_secs();

        assert!((10..=12).contains(&a), "{a}");
        assert!((18..=20).contains(&b), "{b}");
        assert!(resting_for("test-rest-none").is_none());
        assert!(retry_after().is_some());
        assert!(resting_sentence(&["groq"], Duration::from_secs(7)).contains("7 s"));
    }

    #[test]
    fn a_recording_that_is_too_short_is_refused_with_a_sentence() {
        assert!(transcribe(&[0u8; 10], "audio/wav", None, None, false).unwrap_err().contains("too short"));
    }

    #[test]
    fn a_qwen_request_carries_the_audio_in_the_message_and_the_transcript_is_read_back() {
        let body = qwen_body("data:audio/wav;base64,AAAA", None);

        assert_eq!(body["model"], "qwen3-asr-flash");
        assert_eq!(body["messages"][0]["content"][0]["type"], "input_audio");
        assert_eq!(body["messages"][0]["content"][0]["input_audio"]["data"], "data:audio/wav;base64,AAAA");
        assert!(body.get("asr_options").is_none(), "no language: the model detects it");
        assert_eq!(qwen_body("x", Some("bn"))["asr_options"]["language"], "bn");
        assert!(qwen_body("x", Some("auto")).get("asr_options").is_none());

        assert_eq!(qwen_text(&json!({ "choices": [{ "message": { "content": " hello there " } }] })), "hello there");
        assert_eq!(qwen_text(&json!({ "choices": [{ "message": { "content": [{ "type": "text", "text": "a" }, { "text": "b" }] } }] })), "a b");
        assert_eq!(qwen_text(&json!({})), "");

        assert_eq!(
            qwen_native_url("https://dashscope-us.aliyuncs.com/compatible-mode/v1/chat/completions").as_deref(),
            Some("https://dashscope-us.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation")
        );
        assert_eq!(qwen_native_url("https://api.example.com/v1/chat/completions"), None);
        assert_eq!(qwen_native_body("d", None)["input"]["messages"][0]["content"][0]["audio"], "d");
        assert_eq!(qwen_native_body("d", Some("bn"))["parameters"]["asr_options"]["language"], "bn");
        assert_eq!(qwen_native_text(&json!({ "output": { "choices": [{ "message": { "content": [{ "text": " hi " }] } }] } })), "hi");
    }

    /// A one-shot HTTP server: answers the given `(status, body)` pairs in order and records each request's first line.
    fn mock(replies: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();

            for (status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = vec![0u8; 65536];
                let mut total = 0;

                /* read until the headers are complete and the declared body has arrived */
                loop {
                    let n = stream.read(&mut buffer[total..]).unwrap();
                    total += n;
                    let text = String::from_utf8_lossy(&buffer[..total]).to_string();

                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);

                        if total >= split + 4 + length || n == 0 {
                            break;
                        }
                    } else if n == 0 {
                        break;
                    }
                }

                seen.push(String::from_utf8_lossy(&buffer[..total]).lines().next().unwrap_or_default().to_string());
                let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).unwrap();
            }

            seen
        });

        (base, handle)
    }

    fn audio() -> Vec<u8> {
        vec![1u8; 4000]
    }

    #[test]
    fn a_refused_key_moves_on_to_the_next_transcriber_and_the_answer_says_who_failed() {
        let (base, server) = mock(vec![(403, r#"{"error":"not allowed"}"#), (200, r#"{"candidates":[{"content":{"parts":[{"text":" লগইন পেজ খুলুন "}]}}]}"#)]);
        let endpoints = Endpoints { groq: format!("{base}/groq"), gemini: base.clone(), openai: format!("{base}/openai") };
        let keys = vec![("groq", "k1".to_string()), ("google", "k2".to_string())];
        let heard = transcribe_with(&endpoints, &keys, false, &audio(), "audio/wav", Some("bn")).unwrap();

        assert_eq!(heard["text"], "লগইন পেজ খুলুন");
        assert_eq!(heard["engine"], "google");
        assert_eq!(heard["local"], false);
        assert_eq!(heard["tried"][0]["engine"], "groq");
        assert!(heard["tried"][0]["error"].as_str().unwrap().contains("403"));

        let seen = server.join().unwrap();

        assert!(seen[0].contains("/groq") && seen[1].contains("gemini-3.5-transcribe:generateContent"), "{seen:?}");
    }

    #[test]
    fn when_every_transcriber_fails_the_error_lists_each_attempt() {
        let (base, server) = mock(vec![(401, "{}"), (404, "{}"), (404, "{}"), (404, "{}")]);
        let endpoints = Endpoints { groq: format!("{base}/groq"), gemini: base.clone(), openai: String::new() };
        let keys = vec![("groq", "k1".to_string()), ("google", "k2".to_string())];
        let error = transcribe_with(&endpoints, &keys, false, &audio(), "audio/wav", None).unwrap_err();

        assert!(error.contains("groq (") && error.contains("401") && error.contains("google ("), "{error}");

        let seen = server.join().unwrap();

        assert!(seen[3].contains("gemini-2.5-flash"), "an unavailable model falls back to the next: {seen:?}");
    }

    #[test]
    fn with_no_transcriber_the_hint_says_what_to_connect() {
        let endpoints = Endpoints::default();
        let error = transcribe_with(&endpoints, &[], false, &audio(), "audio/wav", None).unwrap_err();

        assert!(error.contains("Voice needs a transcriber") || error.contains("whisper"), "{error}");
    }

    #[test]
    fn a_chosen_engine_is_the_only_one_asked() {
        /* Groq would answer 200 first in the chain; choosing Google must skip it, and Google's refusal is not hidden by Groq. */
        let (base, server) = mock(vec![(403, r#"{"error":"no"}"#)]);
        let endpoints = Endpoints { groq: format!("{base}/groq"), gemini: base.clone(), openai: format!("{base}/openai") };
        let keys = vec![("groq", "k1".to_string()), ("google", "k2".to_string())];
        let error = transcribe_choosing(&endpoints, &keys, Some("google"), &audio(), "audio/wav", None).unwrap_err();

        assert!(error.contains("google (") && !error.contains("groq"), "{error}");

        let seen = server.join().unwrap();

        assert!(seen.iter().all(|request| !request.contains("/groq")), "{seen:?}");
    }

    #[test]
    fn a_chosen_engine_without_a_key_or_unknown_says_so() {
        let endpoints = Endpoints::default();
        let keys = vec![("groq", "k1".to_string())];

        assert!(transcribe_choosing(&endpoints, &keys, Some("qwen"), &audio(), "audio/wav", None).unwrap_err().contains("has no key"));
        assert!(transcribe_choosing(&endpoints, &keys, Some("nope"), &audio(), "audio/wav", None).unwrap_err().contains("not a transcriber"));
    }

    /// The real Gemini API, with a real key in `SDC_LIVE_GEMINI_KEY`: `cargo test --lib live_gemini -- --ignored`.
    #[test]
    #[ignore]
    fn live_gemini_hears_the_fixtures() {
        let key = std::env::var("SDC_LIVE_GEMINI_KEY").expect("SDC_LIVE_GEMINI_KEY");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice");

        for (file, word) in [("en.wav", "login"), ("bn-neural.wav", "লগইন")] {
            let audio = std::fs::read(dir.join(file)).unwrap();
            let heard = transcribe_choosing(&Endpoints::default(), &[("google", key.clone())], Some("google"), &audio, "audio/wav", None).unwrap();

            assert!(heard["text"].as_str().unwrap_or_default().contains(word), "{file}: {heard}");
        }
    }

    #[test]
    fn a_gemini_answer_is_read_from_its_text_parts() {
        assert_eq!(gemini_text(&json!({ "candidates": [{ "content": { "parts": [{ "text": "a" }, { "text": " b " }] } }] })), "a b");
        assert_eq!(gemini_text(&json!({})), "");
        /* gemini-3.5-transcribe: the transcript is in `audioTranscription.text`, not `text`. */
        assert_eq!(gemini_text(&json!({ "candidates": [{ "content": { "parts": [{ "audioTranscription": { "text": " hello " } }] } }] })), "hello");
    }

    #[test]
    fn the_status_always_says_whether_voice_can_work() {
        let status = status();

        assert!(status["available"].is_boolean());
        assert!(status["available"].as_bool().unwrap() || status["hint"].as_str().unwrap().contains("whisper"));
    }
}
