use arboard::Clipboard;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rdev::{Event, EventType, Key, listen};
use reqwest::header::AUTHORIZATION;
use reqwest::multipart;
use serde_json::json;
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

// Global states
static ALT_PRESSED: AtomicBool = AtomicBool::new(false);
static IS_RECORDING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
enum Task {
    Fix,
    Translate,
    Voice,
}

struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}

struct AppState {
    processing: AtomicBool,
    // Use the wrapper here
    audio_stream: Arc<Mutex<Option<SendStream>>>,
    audio_buffer: Arc<Mutex<Vec<i16>>>,
}

#[tokio::main]
async fn main() {
    println!("🚀 Polyglot Tool Pro Active!");
    println!("Hotkeys:");
    println!("  [Option + R] -> Fix Russian Clipboard");
    println!("  [Option + T] -> Translate Clipboard to Russian");
    println!("  [Option + L] -> Toggle Voice to Russian Text");

    let state = Arc::new(AppState {
        processing: AtomicBool::new(false),
        audio_stream: Arc::new(Mutex::new(None)),
        audio_buffer: Arc::new(Mutex::new(Vec::new())),
    });

    let state_clone = Arc::clone(&state);
    // listen blocks the thread, which is fine here
    if let Err(error) = listen(move |event| {
        let task_state = Arc::clone(&state_clone);

        // This will now work because AppState is Send
        tokio::spawn(async move {
            handle_event(event, task_state).await;
        });
    }) {
        eprintln!("Error: {:?}", error);
    }
}

async fn handle_event(event: Event, state: Arc<AppState>) {
    match event.event_type {
        EventType::KeyPress(Key::Alt) | EventType::KeyPress(Key::AltGr) => {
            ALT_PRESSED.store(true, Ordering::SeqCst);
        }
        EventType::KeyRelease(Key::Alt) | EventType::KeyRelease(Key::AltGr) => {
            ALT_PRESSED.store(false, Ordering::SeqCst);
        }
        EventType::KeyPress(key) if ALT_PRESSED.load(Ordering::SeqCst) => match key {
            Key::KeyR => trigger_task(Task::Fix, state).await,
            Key::KeyT => trigger_task(Task::Translate, state).await,
            Key::KeyL => trigger_task(Task::Voice, state).await,
            _ => {}
        },
        _ => {}
    }
}

async fn trigger_task(task: Task, state: Arc<AppState>) {
    if let Task::Voice = task {
        let is_recording = IS_RECORDING.load(Ordering::SeqCst);
        if is_recording {
            stop_and_process_voice(state).await;
        } else {
            start_voice_recording(state);
        }
        return;
    }

    // Atomic "Compare and Swap" logic:
    // If it's false, set it to true and return false. If it's already true, return true.
    if state.processing.swap(true, Ordering::SeqCst) {
        return; // Already busy
    }

    // Now we are safe to await because no MutexGuard is held
    let _ = execute_clipboard_task(task).await;

    // Reset the flag
    state.processing.store(false, Ordering::SeqCst);
}

fn start_voice_recording(state: Arc<AppState>) {
    println!("🔴 Recording started...");
    IS_RECORDING.store(true, Ordering::SeqCst);
    play_sound("Hero");

    let host = cpal::default_host();
    let device = host.default_input_device().expect("No input device");
    let config: cpal::StreamConfig = device.default_input_config().unwrap().into();

    let buffer_clone = Arc::clone(&state.audio_buffer);
    buffer_clone.lock().unwrap().clear();

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut buffer = buffer_clone.lock().unwrap();
                for &sample in data {
                    buffer.push((sample * i16::MAX as f32) as i16);
                }
            },
            |err| eprintln!("Stream error: {}", err),
            None,
        )
        .unwrap();

    stream.play().unwrap();
    *state.audio_stream.lock().unwrap() = Some(SendStream(stream));
}

async fn stop_and_process_voice(state: Arc<AppState>) {
    println!("📤 Stopping and processing...");
    IS_RECORDING.store(false, Ordering::SeqCst);

    // Stop the stream by dropping it
    let _ = state.audio_stream.lock().unwrap().take();

    let state_clone = Arc::clone(&state);
    if let Err(e) = process_audio_to_whisper(state_clone).await {
        eprintln!("❌ Voice Error: {}", e);
    }
}

async fn process_audio_to_whisper(state: Arc<AppState>) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Save buffer to Temp WAV
    let spec = hound::WavSpec {
        channels: 1, // simplified
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let path = "temp_voice.wav";
    {
        let mut writer = hound::WavWriter::create(path, spec)?;
        let buffer = state.audio_buffer.lock().unwrap();
        for &sample in buffer.iter() {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
    }

    // 2. Send to Whisper
    let api_key = std::env::var("OPENAI_API_KEY")?;
    let client = reqwest::Client::new();
    let file_content = tokio::fs::read(path).await?;
    let part = multipart::Part::bytes(file_content)
        .file_name("audio.wav")
        .mime_str("audio/wav")?;

    let form = multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        .text("language", "ru");

    let res = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    if let Some(text) = res["text"].as_str() {
        let mut clipboard = Clipboard::new()?;
        clipboard.set_text(text.to_string())?;
        println!("✅ Transcribed: {}", text);
        play_sound("Glass");
    }

    Ok(())
}

async fn execute_clipboard_task(task: Task) -> Result<(), Box<dyn std::error::Error>> {
    let mut clipboard = Clipboard::new()?;
    let input = clipboard.get_text()?;
    if input.trim().is_empty() {
        return Ok(());
    }

    let prompt = match task {
        Task::Fix => format!(
            "Correct this Russian text. Do not add anything extra, just corrext the text.: {}",
            input
        ),
        Task::Translate => format!(
            "Translate this to English. Do not add anything extra, just translate the text.: {}",
            input
        ),
        _ => unreachable!(),
    };

    let result = call_gpt(prompt).await?;
    clipboard.set_text(result)?;
    play_sound("Glass");
    Ok(())
}

async fn call_gpt(prompt: String) -> Result<String, Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY")?;
    let client = reqwest::Client::new();
    let res = client
        .post("https://api.openai.com/v1/chat/completions")
        .header(AUTHORIZATION, format!("Bearer {}", api_key))
        .json(&json!({
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": prompt}]
        }))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    Ok(res["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

fn play_sound(name: &str) {
    let _ = Command::new("afplay")
        .arg(format!("/System/Library/Sounds/{}.aiff", name))
        .spawn();
}
