# Spotlight

Spotlight is an experimental native launcher for COSMIC on Linux. It is built with Rust and libcosmic, with a compact, centered search window and a translucent purple interface. The renderer uses tiny-skia; Spotlight does not embed a browser engine.

The project is aimed first at Linux systems running the COSMIC desktop on Wayland. It is not yet a drop-in replacement for every desktop launcher. The interface and integrations are under active development.

## Requirements

- Rust 1.93 or newer, with Cargo
- Linux and a Wayland session running COSMIC
- The native libraries required by libcosmic and its Rust dependencies (including a C toolchain, `pkg-config`, Wayland, and xkbcommon development files)
- Runtime helpers: `gio`, `xdg-open`, `cosmic-term` for terminal applications, and `secret-tool` (libsecret) for the desktop keyring

On Debian or Ubuntu, the commonly needed build packages are:

```sh
sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libudev-dev libfontconfig1-dev cmake
```

## Build and run

```sh
cargo build --release --locked
./target/release/spotlight --help
```

The binary is named `spotlight` and uses application ID `io.github.Maglcite.Spotlight`.

```text
spotlight                 Toggle the launcher
spotlight --toggle        Toggle the launcher
spotlight --daemon        Start the persistent background process
spotlight --settings      Open settings
spotlight --query TEXT    Search for TEXT
spotlight --version       Print the version
spotlight --help          Show command help
```

Use `scripts/install.sh` to build and install the binary, desktop entry, and icon under `~/.local`. Pass `--autostart` to also install a COSMIC session autostart entry that runs the daemon. Pass `--no-build` to install an existing `target/release/spotlight` binary. The script does not change desktop shortcuts.

The Linux release archive includes a prebuilt binary and the install scripts. After unpacking it, run `scripts/install.sh --no-build` to install without Rust or Cargo.

## COSMIC shortcut

After installing Spotlight, you can route COSMIC's built-in Launcher action to it with:

```sh
python3 scripts/cosmic-shortcut.py --install
```

The script changes only the `Launcher` command in your user `system_actions` config. It keeps the rest of that file and the keyboard mappings intact, including `Alt+Tab`. It saves an exact backup alongside the config. To undo the change, run:

```sh
python3 scripts/cosmic-shortcut.py --restore
```

The script refuses to overwrite an existing Spotlight backup. It requires Spotlight at `~/.local/bin/spotlight`; install Spotlight first. COSMIC may need a settings reload or a new session before the updated action takes effect.

## Search and AI

Spotlight's standard search is intended for quick app and file discovery. Its file index contains names and paths under your home directory, not file contents. Hidden directories and common dependency or build directories are excluded. Ordinary web queries open in your default browser using DuckDuckGo. If enabled, the AI `search_files` tool reads indexed text files when called and searches them for matching terms; Spotlight does not keep a separate content index.

AI assistance defaults to OpenCode Go at `https://opencode.ai/zen/go/v1` with the explicitly configured model `deepseek-v4.1-flash`. Store the API key in the desktop keyring through Settings; `OPENCODE_API_KEY` is also supported as an environment fallback. You can manually select another provider and model in Settings: OpenAI, Anthropic, Gemini, OpenRouter, Ollama, and custom OpenAI-compatible endpoints are supported. AI tools are read-only and limited to searching indexed files, reading files within configured local search roots, web search, and fetching URLs. Web and local file tools have separate switches in Settings. A query ending in `?` is sent to the configured assistant after a 650 ms pause; the pause can be configured.

Local file tools start disabled; enable them in Settings to share selected file text with your AI provider. Web tools start enabled. The harness has no command execution or file modification tools.

## Development

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Pull requests run these checks in GitHub Actions on Ubuntu. Tagged releases and manually started release workflows build a Linux x86_64 tarball.

## License

Spotlight is distributed under the MIT License. See [LICENSE](LICENSE).
