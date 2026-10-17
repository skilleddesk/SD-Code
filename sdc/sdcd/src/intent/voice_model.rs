//! The speech model download (Phase 1.3): whisper.cpp needs a `ggml-*.bin` model file next to its program. SDC can fetch one
//! into `<data>/whisper/` - **only after the person said yes** (the request must carry `confirm: true`; the window shows the
//! size and the host first), with progress, and the file is accepted only if its SHA-256 matches the one published by the
//! model's repository (Hugging Face LFS metadata, read 2026-10-18). A wrong size or hash deletes the partial file.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub struct ModelSpec {
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub note: &'static str,
}

/// Where the models come from.
pub const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// The models offered. Measured on generated clips (docs/VOICE.md): `small` is fine for English and German; for Bengali and
/// Arabic only the larger `turbo` model gave usable text, and Bengali needed the language set by hand.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec { id: "small", file: "ggml-small-q5_1.bin", bytes: 190_085_487, sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb", note: "190 MB - fast; good for English and German, weak for Bengali" },
    ModelSpec { id: "turbo", file: "ggml-large-v3-turbo-q5_0.bin", bytes: 574_041_195, sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2", note: "574 MB - best local quality for Arabic and Bengali (choose the language by hand for Bengali)" },
    ModelSpec { id: "medium", file: "ggml-medium-q5_0.bin", bytes: 539_212_467, sha256: "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f", note: "539 MB - between the two" },
];

pub fn dir() -> Option<PathBuf> {
    crate::paths::data_dir().ok().map(|dir| dir.join("whisper"))
}

pub fn spec(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|model| model.id == id)
}

fn progress_slot() -> &'static Mutex<Value> {
    static SLOT: OnceLock<Mutex<Value>> = OnceLock::new();

    SLOT.get_or_init(|| Mutex::new(json!({ "state": "idle" })))
}

fn set_progress(value: Value) {
    if let Ok(mut slot) = progress_slot().lock() {
        *slot = value;
    }
}

/// `{state: idle|downloading|done|failed, id, done, total, error}` - what the window draws as a progress bar.
pub fn progress() -> Value {
    progress_slot().lock().map(|slot| slot.clone()).unwrap_or_else(|_| json!({ "state": "idle" }))
}

/// The catalogue for the window: each model, its size, whether it is already installed.
pub fn catalogue() -> Value {
    let folder = dir();

    json!(MODELS
        .iter()
        .map(|model| json!({
            "id": model.id,
            "file": model.file,
            "bytes": model.bytes,
            "note": model.note,
            "installed": folder.as_ref().is_some_and(|folder| folder.join(model.file).is_file()),
            "source": BASE_URL,
        }))
        .collect::<Vec<_>>())
}

/// Starts a download on its own thread. Refuses without `confirm`, for an unknown model, and while another is running.
pub fn start(id: &str, confirm: bool) -> Result<Value, String> {
    start_from(id, confirm, BASE_URL, dir().ok_or("no data folder")?)
}

pub fn start_from(id: &str, confirm: bool, base: &str, folder: PathBuf) -> Result<Value, String> {
    if !confirm {
        return Err("permission_required: the download needs your yes first (confirm).".into());
    }

    let spec = spec(id).ok_or_else(|| format!("unknown speech model `{id}`"))?;

    if progress()["state"] == "downloading" {
        return Err("a speech model is already downloading".into());
    }

    set_progress(json!({ "state": "downloading", "id": id, "done": 0, "total": spec.bytes }));

    let url = format!("{}/{}", base.trim_end_matches('/'), spec.file);
    let id = id.to_string();

    std::thread::spawn(move || match download(spec, &url, &folder) {
        Ok(()) => set_progress(json!({ "state": "done", "id": id, "done": spec.bytes, "total": spec.bytes })),
        Err(error) => set_progress(json!({ "state": "failed", "id": id, "error": error })),
    });

    Ok(progress())
}

fn download(spec: &ModelSpec, url: &str, folder: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;

    let target = folder.join(spec.file);
    let partial = folder.join(format!("{}.part", spec.file));
    let response = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(60))
        .build()
        .get(url)
        .call()
        .map_err(|error| format!("download failed: {error}"))?;
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&partial).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut done = 0u64;

    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            let _ = std::fs::remove_file(&partial);

            format!("download interrupted: {error}")
        })?;

        if read == 0 {
            break;
        }

        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read]).map_err(|error| error.to_string())?;
        done += read as u64;
        set_progress(json!({ "state": "downloading", "id": spec.id, "done": done, "total": spec.bytes }));
    }

    drop(file);

    let hash: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();

    if done != spec.bytes || hash != spec.sha256 {
        let _ = std::fs::remove_file(&partial);

        return Err(format!("the downloaded file did not match its published checksum (got {done} bytes); it was deleted"));
    }

    std::fs::rename(&partial, &target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn serve(body: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);

            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });

        base
    }

    fn wait() -> Value {
        for _ in 0..200 {
            let now = progress();

            if now["state"] != "downloading" {
                return now;
            }

            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        progress()
    }

    #[test]
    fn nothing_downloads_without_a_yes_and_only_known_models_do() {
        assert!(start_from("small", false, "http://127.0.0.1:9", std::env::temp_dir()).unwrap_err().contains("permission_required"));
        assert!(start_from("no-such", true, "http://127.0.0.1:9", std::env::temp_dir()).unwrap_err().contains("unknown"));
    }

    #[test]
    fn the_catalogue_names_real_sizes_and_checksums() {
        for model in MODELS {
            assert_eq!(model.sha256.len(), 64, "{}", model.id);
            assert!(model.bytes > 100_000_000);
            assert!(model.file.starts_with("ggml-") && model.file.ends_with(".bin"));
        }
    }

    /// One test for both outcomes, because the progress slot is shared by the whole process.
    #[test]
    fn a_file_with_the_right_hash_is_kept_and_a_wrong_one_is_deleted() {
        static SPEC_OK: ModelSpec = ModelSpec { id: "tiny-test", file: "ggml-tiny-test.bin", bytes: 11, sha256: "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9", note: "" };
        let folder = std::env::temp_dir().join(format!("sdc-whisper-dl-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&folder);

        /* sha256("hello world") is b94d27b9... */
        let base = serve(b"hello world".to_vec());

        download(&SPEC_OK, &format!("{base}/ggml-tiny-test.bin"), &folder).unwrap();
        assert_eq!(std::fs::read(folder.join("ggml-tiny-test.bin")).unwrap(), b"hello world");
        assert!(!folder.join("ggml-tiny-test.bin.part").exists());

        let bad = serve(b"HELLO WORLD".to_vec());
        let _ = std::fs::remove_file(folder.join("ggml-tiny-test.bin"));
        let error = download(&SPEC_OK, &format!("{bad}/ggml-tiny-test.bin"), &folder).unwrap_err();

        assert!(error.contains("checksum"), "{error}");
        assert!(!folder.join("ggml-tiny-test.bin").exists() && !folder.join("ggml-tiny-test.bin.part").exists(), "nothing wrong is left behind");

        let _ = wait();
        let _ = std::fs::remove_dir_all(&folder);
    }
}
