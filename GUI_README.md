# Voice Keyboard GUI

A graphical user interface for the Voice Keyboard application that makes it easy to control voice dictation with Deepgram.

## Features

- **API Key Management**: Securely store your Deepgram API key in a persistent configuration file
- **Global Hotkey**: Toggle dictation on/off using a customizable hotkey (defaults to F13)
- **Audio Feedback**: Audible beeps when starting (high pitch) and stopping (low pitch) dictation
- **Visual Feedback**: Clean interface showing current status and recording state
- **One-Click Control**: Start and stop dictation with a single button click

## Installation & Building

```bash
# Build both the GUI and the voice-keyboard binary
cargo build --release

# The binaries will be available at:
# ./target/release/voice-keyboard-gui  (GUI application)
# ./target/release/voice-keyboard      (Core voice keyboard)
```

## Usage

### First Time Setup

1. **Launch the GUI**:
   ```bash
   ./target/release/voice-keyboard-gui
   ```

2. **Configure Your API Key**:
   - Enter your Deepgram API key in the "Deepgram API Key" field
   - Get your API key from: https://developers.deepgram.com/docs/create-additional-api-keys

3. **Set Your Hotkey** (Optional):
   - The default hotkey is F13
   - You can change it to any key (e.g., F13, F14, etc.)
   - Note: Global hotkey functionality requires the specified key to be available on your keyboard

4. **Save Configuration**:
   - Click "Save Configuration" to persist your settings
   - Settings are saved to: `~/.config/deepgram/voice-keyboard/config.json` (Linux)

### Starting Dictation

**Method 1: Use the GUI Button**
- Click the green "Start Dictation" button
- You'll hear a high-pitched beep indicating dictation has started
- The button will turn red and say "Stop Dictation"

**Method 2: Use the Hotkey**
- Press your configured hotkey (default: F13)
- You'll hear the same high-pitched beep
- Works even when the GUI is minimized or in the background

### Stopping Dictation

**Method 1: Use the GUI Button**
- Click the red "Stop Dictation" button
- You'll hear a low-pitched beep indicating dictation has stopped

**Method 2: Use the Hotkey**
- Press your configured hotkey again (default: F13)
- You'll hear the same low-pitched beep

## Configuration File

The configuration is stored in JSON format at:
- **Linux**: `~/.config/deepgram/voice-keyboard/config.json`

Example configuration:
```json
{
  "api_key": "your-deepgram-api-key-here",
  "hotkey_code": "F13"
}
```

## Permissions

The voice keyboard requires root privileges to create the virtual keyboard device. When you start dictation, you'll be prompted for your password via `pkexec` (PolicyKit).

This is a one-time prompt per dictation session and is required because:
- The virtual keyboard needs to create a `/dev/uinput` device
- This requires root access for security reasons
- After creating the device, privileges are dropped to access your audio system

## Troubleshooting

### "Failed to start" Error

1. **Check your API key**: Ensure your Deepgram API key is correct and saved
2. **Verify permissions**: Make sure you have `pkexec` installed (usually comes with PolicyKit)
3. **Test the core binary**: Try running the voice-keyboard directly:
   ```bash
   export DEEPGRAM_API_KEY="your-key-here"
   sudo -E ./target/release/voice-keyboard --test-stt
   ```

### No Sound / Beeps Not Working

1. **Check audio output**: Ensure your system audio is working and not muted
2. **Volume levels**: The beeps play at 30% volume - increase system volume if needed

### Hotkey Not Working

1. **Key availability**: Ensure your keyboard has the configured key (e.g., F13)
2. **Key conflicts**: Check if another application is using the same hotkey
3. **Try a different key**: Some keys may not be available as global hotkeys

### Build Errors

If you encounter build errors:

```bash
# Update Rust
rustup update

# Install required system dependencies (Ubuntu/Debian)
sudo apt install libasound2-dev

# Install required system dependencies (Fedora/RHEL)
sudo dnf install alsa-lib-devel

# Clean and rebuild
cargo clean
cargo build --release
```

## Technical Details

- **GUI Framework**: iced (native Rust GUI)
- **Global Hotkeys**: global-hotkey crate
- **Audio Playback**: rodio (for beep sounds)
- **Configuration Storage**: serde + JSON
- **Platform**: Linux (tested on Ubuntu with KDE Plasma/Wayland)

## Security Notes

- API keys are stored in plain text in your config file
- The config directory is created with user-only permissions
- Consider restricting permissions on the config file:
  ```bash
  chmod 600 ~/.config/deepgram/voice-keyboard/config.json
  ```

## Contributing

This is a demonstration application for Deepgram's Flux API. For issues or improvements, please check the main project repository.

## License

ISC License - See LICENSE.txt in the project root
