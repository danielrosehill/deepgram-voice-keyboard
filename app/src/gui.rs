use anyhow::{Context, Result};
use directories::ProjectDirs;
use global_hotkey::{
    hotkey::{Code, HotKey},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use iced::{
    widget::{button, column, container, text, text_input},
    window, Element, Length, Task, Theme,
};
use rodio::{source::SineWave, OutputStream, Sink, Source};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;
// Tray icon disabled - requires GTK which is incompatible with KDE/Wayland
// use tray_icon::{
//     menu::{Menu, MenuItem},
//     TrayIcon, TrayIconBuilder,
// };
use reqwest::Client;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    api_key: String,
    hotkey_code: String,
    project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BillingBalance {
    balance_id: String,
    amount: f64,
    units: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BillingResponse {
    balances: Vec<BillingBalance>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            hotkey_code: "F13".to_string(),
            project_id: String::new(),
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
    ProjectIdChanged(String),
    SaveConfig,
    ToggleDictation,
    CheckBalance,
    BalanceReceived(Result<BillingResponse, String>),
    TrayEvent,
    ShowWindow,
    HideWindow,
}

struct VoiceKeyboardGui {
    config: Config,
    api_key_input: String,
    hotkey_input: String,
    project_id_input: String,
    is_recording: bool,
    status_message: String,
    balance_info: String,
    voice_keyboard_process: Arc<Mutex<Option<Child>>>,
    _hotkey_manager: GlobalHotKeyManager,
    _audio_output_stream: OutputStream,
    audio_sink: Arc<Mutex<Sink>>,
    // _tray_icon: Option<TrayIcon>,  // Disabled for KDE compatibility
    http_client: Client,
}

impl Drop for VoiceKeyboardGui {
    fn drop(&mut self) {
        // Ensure child process is terminated when GUI is closed
        if let Some(mut child) = self.voice_keyboard_process.lock().unwrap().take() {
            let _ = child.kill();
            // Wait up to 1 second for clean shutdown
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_secs(1) {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => break,
                }
            }
            // Force kill if still running
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl VoiceKeyboardGui {
    fn new() -> (Self, Task<Message>) {
        let config = Config::load().unwrap_or_default();
        let api_key_input = config.api_key.clone();
        let hotkey_input = config.hotkey_code.clone();
        let project_id_input = config.project_id.clone();

        // Initialize audio system
        let (stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();

        // Initialize hotkey manager
        let hotkey_manager = GlobalHotKeyManager::new().unwrap();

        // Register F13 hotkey
        let hotkey = HotKey::new(None, Code::F13);
        hotkey_manager.register(hotkey).ok();

        // System tray disabled for KDE/Wayland compatibility
        // The tray-icon crate requires GTK initialization which conflicts with KDE

        let gui = Self {
            config,
            api_key_input,
            hotkey_input,
            project_id_input,
            is_recording: false,
            status_message: "Ready".to_string(),
            balance_info: "Click 'Check Balance' to view billing info".to_string(),
            voice_keyboard_process: Arc::new(Mutex::new(None)),
            _hotkey_manager: hotkey_manager,
            _audio_output_stream: stream,
            audio_sink: Arc::new(Mutex::new(sink)),
            // _tray_icon: tray_icon,  // Disabled for KDE compatibility
            http_client: Client::new(),
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
                            // Send SIGTERM first for graceful shutdown
                            let _ = child.kill();
                            // Wait up to 2 seconds for process to terminate
                            let start = std::time::Instant::now();
                            while start.elapsed() < Duration::from_secs(2) {
                                match child.try_wait() {
                                    Ok(Some(_)) => break, // Process exited
                                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                                    Err(_) => break,
                                }
                            }
                            // Force kill if still running
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    } else {
                        // Start recording - play distinctive double beep
                        if let Ok(sink) = audio_sink_clone.lock() {
                            // First beep - high pitch
                            let beep1 = SineWave::new(1000.0)
                                .take_duration(Duration::from_millis(80))
                                .amplify(0.35);
                            sink.append(beep1);

                            // Short pause
                            std::thread::sleep(Duration::from_millis(50));

                            // Second beep - even higher pitch for brightness
                            let beep2 = SineWave::new(1200.0)
                                .take_duration(Duration::from_millis(80))
                                .amplify(0.35);
                            sink.append(beep2);
                        }

                        // Start the voice keyboard process
                        if let Ok(api_key) = std::env::var("DEEPGRAM_API_KEY") {
                            if !api_key.is_empty() {
                                let exe_path = std::env::current_exe()
                                    .unwrap()
                                    .parent()
                                    .unwrap()
                                    .join("voice-keyboard");

                                let mut cmd = Command::new("pkexec");
                                cmd.arg("env")
                                    .arg(format!("DEEPGRAM_API_KEY={}", api_key));

                                // Preserve audio session environment variables
                                if let Ok(val) = std::env::var("PULSE_RUNTIME_PATH") {
                                    cmd.arg(format!("PULSE_RUNTIME_PATH={}", val));
                                }
                                if let Ok(val) = std::env::var("XDG_RUNTIME_DIR") {
                                    cmd.arg(format!("XDG_RUNTIME_DIR={}", val));
                                }
                                if let Ok(val) = std::env::var("DISPLAY") {
                                    cmd.arg(format!("DISPLAY={}", val));
                                }
                                if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
                                    cmd.arg(format!("WAYLAND_DISPLAY={}", val));
                                }
                                if let Ok(val) = std::env::var("HOME") {
                                    cmd.arg(format!("HOME={}", val));
                                }
                                if let Ok(val) = std::env::var("USER") {
                                    cmd.arg(format!("USER={}", val));
                                }

                                cmd.arg(&exe_path).arg("--test-stt");

                                if let Ok(child) = cmd.spawn() {
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

    fn play_start_beep(&self) {
        // Double beep for starting - bright and distinctive
        if let Ok(sink) = self.audio_sink.lock() {
            // First beep - high pitch
            let beep1 = SineWave::new(1000.0)
                .take_duration(Duration::from_millis(80))
                .amplify(0.35);
            sink.append(beep1);

            // Short pause
            std::thread::sleep(Duration::from_millis(50));

            // Second beep - even higher pitch for brightness
            let beep2 = SineWave::new(1200.0)
                .take_duration(Duration::from_millis(80))
                .amplify(0.35);
            sink.append(beep2);
        }
    }

    fn start_dictation(&mut self) {
        // Set the API key environment variable
        std::env::set_var("DEEPGRAM_API_KEY", &self.config.api_key);

        // Play distinctive double beep for start
        self.play_start_beep();

        // Get the path to the voice-keyboard binary
        let exe_path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("voice-keyboard");

        // Start the voice-keyboard process with pkexec for sudo privileges
        // Pass through necessary environment variables for audio access
        let mut cmd = Command::new("pkexec");
        cmd.arg("env")
            .arg(format!("DEEPGRAM_API_KEY={}", &self.config.api_key));

        // Preserve audio session environment variables
        if let Ok(val) = std::env::var("PULSE_RUNTIME_PATH") {
            cmd.arg(format!("PULSE_RUNTIME_PATH={}", val));
        }
        if let Ok(val) = std::env::var("XDG_RUNTIME_DIR") {
            cmd.arg(format!("XDG_RUNTIME_DIR={}", val));
        }
        if let Ok(val) = std::env::var("DISPLAY") {
            cmd.arg(format!("DISPLAY={}", val));
        }
        if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
            cmd.arg(format!("WAYLAND_DISPLAY={}", val));
        }
        if let Ok(val) = std::env::var("HOME") {
            cmd.arg(format!("HOME={}", val));
        }
        if let Ok(val) = std::env::var("USER") {
            cmd.arg(format!("USER={}", val));
        }

        cmd.arg(&exe_path).arg("--test-stt");

        match cmd.spawn()
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
            // Send SIGTERM first for graceful shutdown
            let _ = child.kill();
            // Wait up to 2 seconds for process to terminate
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_secs(2) {
                match child.try_wait() {
                    Ok(Some(_)) => break, // Process exited
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => break,
                }
            }
            // Force kill if still running
            let _ = child.kill();
            let _ = child.wait();
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
            Message::ProjectIdChanged(value) => {
                self.project_id_input = value;
            }
            Message::SaveConfig => {
                self.config.api_key = self.api_key_input.clone();
                self.config.hotkey_code = self.hotkey_input.clone();
                self.config.project_id = self.project_id_input.clone();
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
            Message::CheckBalance => {
                let api_key = self.config.api_key.clone();
                let project_id = self.config.project_id.clone();
                let client = self.http_client.clone();

                return Task::future(async move {
                    let url = format!("https://api.deepgram.com/v1/projects/{}/balances", project_id);
                    let result = client
                        .get(&url)
                        .header("Authorization", format!("Token {}", api_key))
                        .send()
                        .await;

                    match result {
                        Ok(response) => {
                            if response.status().is_success() {
                                match response.json::<BillingResponse>().await {
                                    Ok(billing) => Message::BalanceReceived(Ok(billing)),
                                    Err(e) => Message::BalanceReceived(Err(format!("Parse error: {}", e))),
                                }
                            } else {
                                Message::BalanceReceived(Err(format!("API error: {}", response.status())))
                            }
                        }
                        Err(e) => Message::BalanceReceived(Err(format!("Request failed: {}", e))),
                    }
                });
            }
            Message::BalanceReceived(result) => {
                match result {
                    Ok(billing) => {
                        if billing.balances.is_empty() {
                            self.balance_info = "No balance information available".to_string();
                        } else {
                            let balance_text = billing.balances.iter()
                                .map(|b| format!("{}: ${:.2}", b.units, b.amount))
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.balance_info = format!("Account Balance:\n{}", balance_text);
                        }
                    }
                    Err(e) => {
                        self.balance_info = format!("Error: {}", e);
                    }
                }
            }
            Message::TrayEvent => {
                // Handle tray events
            }
            Message::ShowWindow => {
                return window::get_latest().and_then(|id| window::gain_focus(id));
            }
            Message::HideWindow => {
                return window::get_latest().and_then(|id| window::minimize(id, true));
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

        let project_id_label = text("Deepgram Project ID:");
        let project_id_field = text_input("Enter your project ID", &self.project_id_input)
            .on_input(Message::ProjectIdChanged)
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

        // Billing panel
        let billing_title = text("Billing Information").size(24);
        let check_balance_button = button("Check Balance")
            .on_press(Message::CheckBalance)
            .padding(10);
        let balance_display = text(&self.balance_info).size(16);

        let content: Element<_> = column![
            title,
            text("").size(10),
            api_key_label,
            api_key_field,
            text("").size(10),
            project_id_label,
            project_id_field,
            text("").size(10),
            hotkey_label,
            hotkey_field,
            text("").size(10),
            save_button,
            text("").size(20),
            toggle_button,
            text("").size(20),
            status,
            text("").size(30),
            billing_title,
            text("").size(10),
            check_balance_button,
            text("").size(10),
            balance_display,
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
