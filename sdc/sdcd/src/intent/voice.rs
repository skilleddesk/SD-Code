//! Voice: speech to text, then the same Intent Engine as typing.
//!
//! Local first, because a person's voice is theirs: **whisper.cpp** (`whisper-cli`) or OpenAI's
//! `whisper` command, when installed, with a model file under `<data>/whisper/` (or `SDC_WHISPER_MODEL`).
//! When there is none, a connected provider that transcribes - Groq, then OpenAI - is used, and the
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
    let online: Vec<&str> = ["groq", "openai-api"]
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
            json!("Voice needs a transcriber: install whisper.cpp and put a model (for example ggml-small.bin) in SDC's data folder under whisper/, or connect Groq or OpenAI in the Provider Hub.")
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
        ("openai-api", "https://api.openai.com/v1/audio/transcriptions", "whisper-1"),
    ] {
        let Some(key) = crate::auth::keychain::get(&crate::providers::key_ref(provider)) else {
            continue;
        };

        return online(url, &key, model, audio, mime, language).map(|text| json!({ "text": text, "engine": provider, "local": false }));
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
    fn the_status_always_says_whether_voice_can_work() {
        let status = status();

        assert!(status["available"].is_boolean());
        assert!(status["available"].as_bool().unwrap() || status["hint"].as_str().unwrap().contains("whisper"));
    }
}
