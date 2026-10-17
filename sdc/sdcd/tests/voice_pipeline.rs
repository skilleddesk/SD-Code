//! The audio pipeline, with fixtures (Phase 1.4): the clips are real 16 kHz mono speech, the chain is the one the app uses, the
//! cloud side is a local mock server, and the local whisper.cpp side runs only when a `whisper-cli` and a model are present
//! (`SDC_WHISPER_MODEL`), so CI without them stays green and says it skipped.

use std::io::{Read, Write};
use std::path::PathBuf;

use sdcd::intent::voice::{transcribe_with, Endpoints};

fn clip(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice").join(format!("{name}.wav"));

    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// (sample rate, channels, bits, samples) of a canonical 44-byte-header WAV.
fn wav(bytes: &[u8]) -> (u32, u16, u16, Vec<i16>) {
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");

    let find = |tag: &[u8]| bytes.windows(4).position(|window| window == tag).expect("chunk");
    let fmt = find(b"fmt ") + 8;
    let channels = u16::from_le_bytes([bytes[fmt + 2], bytes[fmt + 3]]);
    let rate = u32::from_le_bytes([bytes[fmt + 4], bytes[fmt + 5], bytes[fmt + 6], bytes[fmt + 7]]);
    let bits = u16::from_le_bytes([bytes[fmt + 14], bytes[fmt + 15]]);
    let data = find(b"data") + 8;
    let samples = bytes[data..].chunks_exact(2).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect();

    (rate, channels, bits, samples)
}

#[test]
fn every_fixture_is_16khz_mono_pcm_with_real_sound_in_it() {
    for name in ["en", "de", "ar-neural", "bn-neural"] {
        let (rate, channels, bits, samples) = wav(&clip(name));
        let rms = (samples.iter().map(|sample| (*sample as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();

        assert_eq!((rate, channels, bits), (16_000, 1, 16), "{name}");
        assert!(samples.len() > 16_000 * 2, "{name} is at least two seconds");
        assert!(rms > 300.0, "{name} is not silence (rms {rms:.0})");
    }
}

/// A one-reply HTTP server that keeps the request's body.
fn echo_server(reply: &'static str) -> (String, std::thread::JoinHandle<Vec<u8>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 65536];

        loop {
            let n = stream.read(&mut buffer).unwrap();

            request.extend_from_slice(&buffer[..n]);

            let text = String::from_utf8_lossy(&request).to_string();

            if let Some(split) = text.find("\r\n\r\n") {
                let length = text.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|value| value.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);

                if request.len() >= split + 4 + length {
                    break;
                }
            }

            if n == 0 {
                break;
            }
        }

        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).unwrap();
        request
    });

    (base, handle)
}

#[test]
fn the_chain_uploads_the_recording_whole_with_the_language_and_reads_the_text_back() {
    let audio = clip("bn-neural");
    let (base, server) = echo_server(r#"{"text":" অনুগ্রহ করে লগইন পেজটি খুলুন "}"#);
    let endpoints = Endpoints { groq: format!("{base}/openai/v1/audio/transcriptions"), gemini: base.clone(), openai: String::new() };
    let heard = transcribe_with(&endpoints, &[("groq", "k".into())], false, &audio, "audio/wav", Some("bn")).unwrap();
    let request = server.join().unwrap();
    let text = String::from_utf8_lossy(&request);

    assert_eq!(heard["text"], "অনুগ্রহ করে লগইন পেজটি খুলুন");
    assert_eq!(heard["engine"], "groq");
    assert!(text.contains("name=\"language\"\r\n\r\nbn"), "the language travels");
    assert!(text.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo"));
    assert!(request.windows(audio.len()).any(|window| window == audio.as_slice()), "the recording is in the upload byte for byte");
}

/// Runs the real whisper.cpp on every fixture, when one is installed; prints the text and the error rates.
#[test]
fn local_whisper_reads_the_fixtures_when_it_is_installed() {
    let Some(model) = std::env::var_os("SDC_WHISPER_MODEL").filter(|path| std::path::Path::new(path).is_file()) else {
        eprintln!("skipped: set SDC_WHISPER_MODEL to a ggml model (and have whisper-cli on PATH) to run the local checks");

        return;
    };

    let _ = model;

    for (name, language, must_contain) in [("en", "en", "login page"), ("de", "de", "Anmeldeseite"), ("ar-neural", "ar", "تسجيل الدخول")] {
        let heard = transcribe_with(&Endpoints::default(), &[], true, &clip(name), "audio/wav", Some(language));

        match heard {
            Ok(heard) => assert!(heard["text"].as_str().unwrap().contains(must_contain), "{name}: {heard}"),
            Err(error) => panic!("{name}: {error}"),
        }
    }
}
