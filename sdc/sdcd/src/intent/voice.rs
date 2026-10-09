//! Voice: speech to text, then the same Intent Engine as typing.
//!
//! Local first, because a person's voice is theirs: **whisper.cpp** (`whisper-cli`) or OpenAI's
//! `whisper` command, when installed, with a model file under `<data>/whisper/` (or `SDC_WHISPER_MODEL`).
//! When there is none, a connected provider that transcribes - Groq, Alibaba (Qwen3-ASR), then OpenAI - is used, and the
//! answer says which one heard it, so a person always knows whether their voice left the machine.
//!
//! The window records 16 kHz mono WAV (it encodes the PCM itself), which is what whisper.cpp reads
//! without `ffmpeg`.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

/// The model file whisper.cpp should use: `SDC_WHISPER_MODEL`, else the first `*.bin` under
/// `<data>/whisper/`.
fn whisper_model() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SDC_WHISPER_MODEL") {
        let path = PathBuf::from(path);

        if path.is_file() {
            return Some(path);
        }
    }

    let directory = crate::paths::data_dir().ok()?.join("whisper");
    let mut models: Vec<PathBuf> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "bin"))
        .collect();

    models.sort();
    models.into_iter().next()
}

/// What this machine can do: `{local, program, model, online: [providers]}` - the voice button's tooltip
/// and the doctor row.
pub fn status() -> Value {
    let program = ["whisper-cli", "whisper-cpp", "whisper"].into_iter().find(|program| crate::host::program::resolve(program).is_some());
    let model = whisper_model();
    let online: Vec<&str> = ["groq", "qwen", "openai-api"]
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
        "available": local || !online.is_empty(),
        "hint": if local || !online.is_empty() {
            Value::Null
        } else {
            json!("Voice needs a transcriber: connect Alibaba Cloud (Qwen), Groq (a free key) or OpenAI in the Provider Hub - nothing to install. (Fully offline voice uses whisper.cpp with a model in SDC's data folder under whisper/, when you have it.)")
        },
    })
}

/// Transcribes one recording. `language` is a hint (`bn`, `ar`, …) or `None` for automatic.
pub fn transcribe(audio: &[u8], mime: &str, language: Option<&str>) -> Result<Value, String> {
    if audio.len() < 1000 {
        return Err("The recording is too short to hear anything in it.".to_string());
    }

    if let Some(result) = local(audio, mime, language) {
        return result;
    }

    for (provider, url, model) in [
        ("groq", "https://api.groq.com/openai/v1/audio/transcriptions", "whisper-large-v3-turbo"),
        /* 0.22: Alibaba's Qwen3-ASR - the key a person already has for DeepSeek, Qwen and Kimi through Model Studio.
           DeepSeek's own API has no speech-to-text, so this is the way to use that key for voice. It is a chat
           call with the audio inside it, not an `audio/transcriptions` upload. */
        ("qwen", "", "qwen3-asr-flash"),
        ("openai-api", "https://api.openai.com/v1/audio/transcriptions", "whisper-1"),
    ] {
        let Some(key) = crate::auth::keychain::get(&crate::providers::key_ref(provider)) else {
            continue;
        };

        let heard = if provider == "qwen" { qwen_online(&key, audio, mime, language) } else { online(url, &key, model, audio, mime, language) };

        return heard.map(|text| json!({ "text": text, "engine": provider, "local": false }));
    }

    Err(status()["hint"].as_str().unwrap_or("No transcriber is available.").to_string())
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
        let model = whisper_model()?;

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
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(90)).build();
    let post = |url: &str, body: &Value| -> Result<Value, (u16, String)> {
        match agent.post(url).set("authorization", &format!("Bearer {key}")).set("content-type", "application/json").send_string(&body.to_string()) {
            Ok(response) => {
                let mut text = String::new();

                response.into_reader().take(1_000_000).read_to_string(&mut text).map_err(|error| (0, error.to_string()))?;

                serde_json::from_str(&text).map_err(|_| (0, format!("the transcriber answered something that is not JSON: {}", text.chars().take(200).collect::<String>())))
            }
            Err(ureq::Error::Status(code, response)) => Err((code, response.into_string().unwrap_or_default().chars().take(300).collect())),
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
fn online(url: &str, key: &str, model: &str, audio: &[u8], mime: &str, language: Option<&str>) -> Result<String, String> {
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

    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(90)).build();
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
            let body = response.into_string().unwrap_or_default();

            Err(format!("The transcriber refused the recording (HTTP {code}): {}", body.chars().take(300).collect::<String>()))
        }
        Err(error) => Err(format!("The transcriber could not be reached: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_that_is_too_short_is_refused_with_a_sentence() {
        assert!(transcribe(&[0u8; 10], "audio/wav", None).unwrap_err().contains("too short"));
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

    #[test]
    fn the_status_always_says_whether_voice_can_work() {
        let status = status();

        assert!(status["available"].is_boolean());
        assert!(status["available"].as_bool().unwrap() || status["hint"].as_str().unwrap().contains("whisper"));
    }
}
