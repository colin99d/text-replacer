use arboard::Clipboard;
use rdev::{Event, EventType, Key, listen};
use reqwest::header::AUTHORIZATION;
use serde_json::json;
use std::process::Command;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

// Atomic flag to track if the Alt/Option key is held down
static ALT_PRESSED: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() {
    println!("🚀 Russian Fixer is active!");
    println!("Hotkey: [Option + R]");
    println!("Press Ctrl+C to stop.");

    // Shared state to prevent overlapping API calls
    let processing = Arc::new(Mutex::new(false));

    // Start the keyboard listener.
    // listen() blocks the current thread, so we don't need a loop at the end.
    if let Err(error) = listen(move |event| {
        handle_event(event, Arc::clone(&processing));
    }) {
        eprintln!("Error: Could not start keyboard listener: {:?}", error);
    }
}

fn handle_event(event: Event, processing: Arc<Mutex<bool>>) {
    match event.event_type {
        // Update Alt key state
        EventType::KeyPress(Key::Alt) | EventType::KeyPress(Key::AltGr) => {
            ALT_PRESSED.store(true, Ordering::SeqCst);
        }
        EventType::KeyRelease(Key::Alt) | EventType::KeyRelease(Key::AltGr) => {
            ALT_PRESSED.store(false, Ordering::SeqCst);
        }

        // Check for 'R' key
        EventType::KeyPress(Key::KeyR) => {
            if ALT_PRESSED.load(Ordering::SeqCst) {
                // Check if we are already processing a request
                let should_start = {
                    let mut is_busy = processing.lock().unwrap();
                    if !*is_busy {
                        *is_busy = true;
                        true
                    } else {
                        false
                    }
                };

                if should_start {
                    let processing_clone = Arc::clone(&processing);
                    tokio::spawn(async move {
                        println!("✨ Fixing Russian grammar...");
                        if let Err(e) = fix_clipboard().await {
                            eprintln!("❌ Error: {}", e);
                        }

                        // Release the lock so we can trigger again
                        let mut is_busy = processing_clone.lock().unwrap();
                        *is_busy = false;
                        println!("✅ Done!");
                    });
                }
            }
        }
        _ => {}
    }
}

async fn fix_clipboard() -> Result<(), Box<dyn std::error::Error>> {
    let mut clipboard = Clipboard::new()?;
    let text = clipboard.get_text()?;

    if text.trim().is_empty() {
        return Ok(());
    }

    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY environment variable not set");

    let client = reqwest::Client::new();

    // Minimalist prompt for quick, accurate corrections
    let prompt = format!(
        "Correct the following Russian text. Keep it natural and change as little as possible. \
        Output ONLY the corrected text. NEVER ADD QUOTATIONS\n\n\"{}\"",
        text
    );

    let res = client.post("https://api.openai.com/v1/chat/completions")
        .header(AUTHORIZATION, format!("Bearer {}", api_key))
        .json(&json!({
            "model": "gpt-5.4-mini",
            "messages": [
                {"role": "system", "content": "You are a direct text-replacement tool for Russian grammar."},
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.2
        }))
        .send()
        .await?;

    let response_json: serde_json::Value = res.json().await?;
    if let Some(corrected) = response_json["choices"][0]["message"]["content"].as_str() {
        clipboard.set_text(corrected.trim().to_string())?;

        // macOS notification sound
        let _ = Command::new("afplay")
            .arg("/System/Library/Sounds/Glass.aiff")
            .spawn();
    }

    Ok(())
}
