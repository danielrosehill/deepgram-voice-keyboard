use anyhow::{Context, Result};
use directories::ProjectDirs;
use global_hotkey::{
    hotkey::{Code, HotKey},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use iced::{
    widget::{button, column, container, text, text_input},
    Element, Length, Task, Theme,
};
use rodio::{source::SineWave, OutputStream, Sink, Source};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    api_key: String,
    hotkey_code: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            hotkey_code: "F13".to_string(),
        }
    }
}

impl Config {
    fn config_path() -> Result<PathBuf> {
        let project_dirs = ProjectDirs::from("com", "deepgram", "voice-keyboard")
            .context("Failed to get project directories")?;
        let config_dir = project_dirs.config_dir();
        fs::create_dir_all(config_dir)?;
        Ok(config_dir.join("config.json"))
    }

    fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            let contents = fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&contents)?)
        } else {
            Ok(Self::default())
        }
    }

    fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
enum Message {
    ApiKeyChanged(String),
    HotkeyChanged(String),
    SaveConfig,
    ToggleDictation,
}

struct VoiceKeyboardGui {
    config: Config,
    api_key_input: String,
    hotkey_input: String,
    is_recording: bool,
    status_message: String,
    voice_keyboard_process: Arc<Mutex<Option<Child>>>,
    _hotkey_manager: GlobalHotKeyManager,
    _audio_output_stream: OutputStream,
    audio_sink: Arc<Mutex<Sink>>,
}

impl VoiceKeyboardGui {
    fn new() -> (Self, Task<Message>) {
        let config = Config::load().unwrap_or_default();
        let api_key_input = config.api_key.clone();
        let hotkey_input = config.hotkey_code.clone();

        // Initialize audio system
        let (stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();

        // Initialize hotkey manager
        let hotkey_manager = GlobalHotKeyManager::new().unwrap();

        // Register F13 hotkey
        let hotkey = HotKey::new(None, Code::F13);
        hotkey_manager.register(hotkey).ok();

        let gui = Self {
            config,
            api_key_input,
            hotkey_input,
            is_recording: false,
            status_message: "Ready".to_string(),
            voice_keyboard_process: Arc::new(Mutex::new(None)),
            _hotkey_manager: hotkey_manager,
            _audio_output_stream: stream,
            audio_sink: Arc::new(Mutex::new(sink)),
        };

        // Start hotkey listener
        let process_clone = gui.voice_keyboard_process.clone();
        let audio_sink_clone = gui.audio_sink.clone();
        std::thread::spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            loop {
                if let Ok(_event) = receiver.recv() {
                    // Toggle dictation
                    let mut process_lock = process_clone.lock().unwrap();
                    let is_running = process_lock.is_some();

                    if is_running {
                        // Stop recording - play lower beep
                        if let Ok(sink) = audio_sink_clone.lock() {
                            let source = SineWave::new(400.0)
                                .take_duration(Duration::from_millis(100))
                                .amplify(0.3);
                            sink.append(source);
                        }

                        if let Some(mut child) = process_lock.take() {
                            let _ = child.kill();
                        }
                    } else {
                        // Start recording - play higher beep
                        if let Ok(sink) = audio_sink_clone.lock() {
                            let source = SineWave::new(800.0)
                                .take_duration(Duration::from_millis(100))
                                .amplify(0.3);
                            sink.append(source);
                        }

                        // Start the voice keyboard process
                        if let Ok(api_key) = std::env::var("DEEPGRAM_API_KEY") {
                            if !api_key.is_empty() {
                                if let Ok(child) = Command::new("pkexec")
                                    .arg(std::env::current_exe().unwrap().parent().unwrap().join("voice-keyboard"))
                                    .arg("--test-stt")
                                    .env("DEEPGRAM_API_KEY", api_key)
                                    .spawn()
                                {
                                    *process_lock = Some(child);
                                }
                            }
                        }
                    }
                }
            }
        });

        (gui, Task::none())
    }

    fn play_beep(&self, frequency: f32) {
        if let Ok(sink) = self.audio_sink.lock() {
            let source = SineWave::new(frequency)
                .take_duration(Duration::from_millis(100))
                .amplify(0.3);
            sink.append(source);
        }
    }

    fn start_dictation(&mut self) {
        // Set the API key environment variable
        std::env::set_var("DEEPGRAM_API_KEY", &self.config.api_key);

        // Play start beep (higher pitch)
        self.play_beep(800.0);

        // Get the path to the voice-keyboard binary
        let exe_path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("voice-keyboard");

        // Start the voice-keyboard process with pkexec for sudo privileges
        match Command::new("pkexec")
            .arg(&exe_path)
            .arg("--test-stt")
            .env("DEEPGRAM_API_KEY", &self.config.api_key)
            .spawn()
        {
            Ok(child) => {
                *self.voice_keyboard_process.lock().unwrap() = Some(child);
                self.is_recording = true;
                self.status_message = "Recording...".to_string();
            }
            Err(e) => {
                self.status_message = format!("Failed to start: {}", e);
            }
        }
    }

    fn stop_dictation(&mut self) {
        // Play stop beep (lower pitch)
        self.play_beep(400.0);

        if let Some(mut child) = self.voice_keyboard_process.lock().unwrap().take() {
            let _ = child.kill();
            self.is_recording = false;
            self.status_message = "Stopped".to_string();
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ApiKeyChanged(value) => {
                self.api_key_input = value;
            }
            Message::HotkeyChanged(value) => {
                self.hotkey_input = value;
            }
            Message::SaveConfig => {
                self.config.api_key = self.api_key_input.clone();
                self.config.hotkey_code = self.hotkey_input.clone();
                match self.config.save() {
                    Ok(_) => {
                        self.status_message = "Configuration saved!".to_string();
                    }
                    Err(e) => {
                        self.status_message = format!("Failed to save config: {}", e);
                    }
                }
            }
            Message::ToggleDictation => {
                if self.is_recording {
                    self.stop_dictation();
                } else {
                    self.start_dictation();
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<Message> {
        let title = text("Voice Keyboard Control").size(32);

        let api_key_label = text("Deepgram API Key:");
        let api_key_field = text_input("Enter your Deepgram API key", &self.api_key_input)
            .on_input(Message::ApiKeyChanged)
            .padding(10)
            .size(20);

        let hotkey_label = text("Hotkey (e.g., F13):");
        let hotkey_field = text_input("Enter hotkey", &self.hotkey_input)
            .on_input(Message::HotkeyChanged)
            .padding(10)
            .size(20);

        let save_button = button("Save Configuration")
            .on_press(Message::SaveConfig)
            .padding(10);

        let toggle_button = if self.is_recording {
            button("Stop Dictation")
                .on_press(Message::ToggleDictation)
                .padding(15)
                .style(|theme: &Theme, _status| {
                    let palette = theme.extended_palette();
                    button::Style {
                        background: Some(iced::Background::Color(palette.danger.strong.color)),
                        text_color: palette.danger.strong.text,
                        ..button::Style::default()
                    }
                })
        } else {
            button("Start Dictation")
                .on_press(Message::ToggleDictation)
                .padding(15)
                .style(|theme: &Theme, _status| {
                    let palette = theme.extended_palette();
                    button::Style {
                        background: Some(iced::Background::Color(palette.success.strong.color)),
                        text_color: palette.success.strong.text,
                        ..button::Style::default()
                    }
                })
        };

        let status = text(&self.status_message).size(18);

        let content: Element<_> = column![
            title,
            text("").size(10),
            api_key_label,
            api_key_field,
            text("").size(10),
            hotkey_label,
            hotkey_field,
            text("").size(10),
            save_button,
            text("").size(20),
            toggle_button,
            text("").size(20),
            status,
        ]
        .padding(20)
        .spacing(5)
        .into();

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center(Length::Fill)
            .into()
    }
}

fn main() -> iced::Result {
    iced::application("Voice Keyboard", VoiceKeyboardGui::update, VoiceKeyboardGui::view)
        .window_size((500.0, 600.0))
        .centered()
        .run_with(VoiceKeyboardGui::new)
}
