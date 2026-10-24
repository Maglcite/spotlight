# Spotlight

Spotlight is an experimental native launcher and read-only AI assistant for Windows and Linux. It is built with Rust and iced, with a compact, centered search window and a translucent purple interface that matches purpwm, including bundled Noto Sans Mono typography. Rendering uses wgpu, with a tiny-skia software fallback; Spotlight does not embed a browser engine.

The interface and integrations are under active development. It is not yet a drop-in replacement for every desktop launcher.

## Features

- Search installed applications, files under your home directory, and the web from one window
- Read-only AI assistant with a choice of providers; a query ending in `?` is sent to the assistant
- Global hotkey (default `Win+Space` on Linux, `Alt+Space` on Windows) on Windows and X11; a desktop shortcut on Wayland
- A single background instance: later invocations forward their command to it over a local socket (a Unix socket on Linux, a named pipe on Windows)
- A warm daemon with fast toggling: graphics pipelines stay initialized, while periodic app/file reindexing runs only while the launcher is visible
- Headless `search` and `ask` subcommands for use from a terminal
- API keys kept in the system credential store

## Install on Windows

Requirements: Windows 10 or 11, x86_64.

### From a release

1. Download `spotlight-windows-x86_64.zip` from the [releases page](https://github.com/Maglcite/spotlight/releases) and unpack it.
2. In the unpacked `spotlight-windows-x86_64` folder, run:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -NoBuild -Autostart
   ```

The script installs `spotlight.exe` to `%LOCALAPPDATA%\Programs\Spotlight`, adds a Start Menu shortcut, and, with `-Autostart`, the same `Run` registry value as Settings > Start at login (`"<install dir>\spotlight.exe" --daemon`, for the current user only). It does not need administrator rights. Any running Spotlight instance from that location is stopped before the binary is replaced.

To remove Spotlight:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -Uninstall
```

Uninstalling removes the binary, the shortcuts, the `Run` autostart value, and any Startup-folder shortcut left by an older install. It keeps your settings in `%APPDATA%\spotlight` and the API key in Windows Credential Manager; delete those manually if you no longer need them.

### From source

Install Rust 1.93 or newer with [rustup](https://rustup.rs) using the default MSVC toolchain, plus the Visual Studio Build Tools "Desktop development with C++" workload (rustup offers to install it). No other native dependencies are needed. Then, from the repository:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -Autostart
```

This runs `cargo build --release --locked` and installs the result. Pass `-NoBuild` to install an existing `target\release\spotlight.exe`.

The Windows binary is built for the GUI subsystem, so it does not open a console window. Output from `spotlight search` and `spotlight ask` is written when the command is run from an existing terminal.

## Install on Linux

Spotlight runs on Wayland and X11 on any desktop. COSMIC is the best-supported desktop.

### Requirements

- Rust 1.93 or newer, with Cargo (for building from source)
- A C toolchain, `pkg-config`, CMake, and xkbcommon and Wayland development files
- Runtime helpers: `gio` or `xdg-open` for opening files and URLs, a terminal emulator for terminal applications, and a Secret Service provider (GNOME Keyring, KWallet, or KeePassXC) for storing the API key

On Debian or Ubuntu:

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev cmake
```

### From source

```sh
cargo build --release --locked
scripts/install.sh --autostart
```

`scripts/install.sh` builds Spotlight and installs the binary to `~/.local/bin/spotlight`, a desktop entry, and the icon under `~/.local/share`. Pass `--autostart` to also install an XDG autostart entry that runs `spotlight --daemon` at login; it works on any desktop that supports XDG autostart. Pass `--no-build` to install an existing `target/release/spotlight` binary. The script does not change keyboard shortcuts.

### From a release

The Linux release archive `spotlight-linux-x86_64.tar.gz` includes a prebuilt binary and the install scripts. After unpacking it, run `scripts/install.sh --no-build` from the unpacked directory to install without Rust or Cargo.

## Hotkeys

| Platform | Default | How it works |
| --- | --- | --- |
| Windows | `Alt+Space` | Registered by Spotlight as a global hotkey |
| Linux, X11 | `Super+Space` | Registered by Spotlight as a global hotkey |
| Linux, Wayland | `Super+Space` (informational) | Bind a desktop shortcut to `spotlight --toggle`; the compositor owns the binding |

On Windows and X11, change the hotkey in Settings or with the `hotkey` key in the config file. The background process must be running (`spotlight --daemon`, or the autostart entry) for the hotkey to work.

Wayland does not allow applications to register global hotkeys, so use your desktop's shortcut settings to run `~/.local/bin/spotlight --toggle`:

- **COSMIC**: run `python3 scripts/cosmic-shortcut.py --install` (see below), or add a custom shortcut in Settings > Input Devices > Keyboard > Keyboard Shortcuts.
- **GNOME**: Settings > Keyboard > View and Customize Shortcuts > Custom Shortcuts. Add a shortcut with the command `~/.local/bin/spotlight --toggle`, using the full path. From a terminal:

  ```sh
  path=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/spotlight/
  gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "['$path']"
  gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$path name 'Spotlight'
  gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$path command "$HOME/.local/bin/spotlight --toggle"
  gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$path binding '<Alt>space'
  ```

  The first command replaces any existing custom shortcut list; add the path to the existing list instead if you have other custom shortcuts.
- **KDE Plasma**: System Settings > Keyboard > Shortcuts > Add New > Command or Script, with the command `~/.local/bin/spotlight --toggle`.

### COSMIC shortcut script

After installing Spotlight, you can route COSMIC's built-in Launcher action to it with:

```sh
python3 scripts/cosmic-shortcut.py --install
```

The script changes only the `Launcher` command in your user `system_actions` config. It keeps the rest of that file and the keyboard mappings intact, including `Alt+Tab`. It saves an exact backup alongside the config. To undo the change, run:

```sh
python3 scripts/cosmic-shortcut.py --restore
```

The script refuses to overwrite an existing Spotlight backup. It requires Spotlight at `~/.local/bin/spotlight`; install Spotlight first. COSMIC may need a settings reload or a new session before the updated action takes effect.

## Command line

The binary is named `spotlight` (`spotlight.exe` on Windows) and uses application ID `io.github.Maglcite.Spotlight`.

```text
spotlight                  Toggle the launcher
spotlight --toggle         Toggle the launcher
spotlight --daemon         Start the persistent background process
spotlight --settings       Open settings
spotlight --query TEXT     Open the launcher and search for TEXT
spotlight search QUERY     Print search results without opening a window
spotlight ask PROMPT       Send PROMPT to the assistant and print the answer
spotlight --version        Print the version
spotlight --help           Show command help
```

Only one Spotlight instance runs per user session. If one is already running, `spotlight`, `--toggle`, `--settings`, and `--query` forward the command to it and exit. If none is running, the command starts a new instance.

## Search and AI

Spotlight's standard search is intended for quick app and file discovery. Applications come from freedesktop desktop entries on Linux and Start Menu shortcuts on Windows. Its file index contains names and paths under your home directory, not file contents; lowercased search keys are precomputed while indexing so queries do not rebuild strings for every file. Hidden directories and common dependency or build directories are excluded. Ordinary web queries open in your default browser using DuckDuckGo. If enabled, the AI `search_files` tool reads indexed text files when called and searches them for matching terms; Spotlight does not keep a separate content index.

AI assistance defaults to OpenCode Go at `https://opencode.ai/zen/go/v1` with the explicitly configured model `deepseek-v4.1-flash`. Store the API key in the system credential store through Settings; `OPENCODE_API_KEY` is also supported as an environment fallback. You can manually select another provider and model in Settings: OpenAI, Anthropic, Gemini, OpenRouter, Ollama, and custom OpenAI-compatible endpoints are supported. AI tools are read-only and limited to searching indexed files, reading files within configured local search roots, web search, and fetching URLs. Web and local file tools have separate switches in Settings. A query ending in `?` is sent to the configured assistant after a 650 ms pause; the pause can be configured.

Local file tools start disabled; enable them in Settings to share selected file text with your AI provider. Web tools start enabled. The harness has no command execution or file modification tools.

## Configuration and credentials

| | Windows | Linux |
| --- | --- | --- |
| Settings | `%APPDATA%\spotlight\` | `$XDG_CONFIG_HOME/spotlight/` (usually `~/.config/spotlight/`) |
| API key | Windows Credential Manager | Secret Service (desktop keyring) |
| Installed binary | `%LOCALAPPDATA%\Programs\Spotlight\spotlight.exe` | `~/.local/bin/spotlight` |
| Autostart | `Run` registry value `Spotlight` (`HKCU`), set by Settings > Start at login | `~/.config/autostart/io.github.Maglcite.Spotlight.desktop` |

## Development

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

To check the Windows build from Linux, install [cargo-xwin](https://github.com/rust-cross/cargo-xwin) and the target, then run Clippy against it:

```sh
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
cargo xwin clippy --target x86_64-pc-windows-msvc --locked --all-targets -- -D warnings
```

Pull requests run formatting, Clippy, and tests in GitHub Actions on Ubuntu and Windows. Tagged releases and manually started release workflows build a Linux x86_64 tarball and a Windows x86_64 zip; tagged builds are published to the GitHub release.

## License

Spotlight is distributed under the MIT License. See [LICENSE](LICENSE).
