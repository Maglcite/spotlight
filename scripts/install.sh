#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/install.sh [--autostart] [--no-build] [--help]

Build and install Spotlight in the current user's ~/.local directories.
Pass --autostart to start the background daemon at COSMIC session login.
Pass --no-build to install an existing target/release/spotlight binary.
USAGE
}

autostart=false
no_build=false
while (($#)); do
  case "$1" in
    --autostart) autostart=true ;;
    --no-build) no_build=true ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'Unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
binary="$repo_root/target/release/spotlight"
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
bin_dir="$HOME/.local/bin"
applications_dir="$data_home/applications"
icon_dir="$data_home/icons/hicolor/scalable/apps"
autostart_file="$config_home/autostart/io.github.Maglcite.Spotlight.desktop"

desktop_quote() {
  local value=$1 escaped='' char index
  for ((index = 0; index < ${#value}; index++)); do
    char=${value:index:1}
    case "$char" in
      '\'|'"'|'$'|'`') escaped+='\' ;;
    esac
    escaped+="$char"
  done
  printf '"%s"' "$escaped"
}

write_desktop_entry() {
  local path=$1 action=$2
  cat > "$path" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Spotlight
Comment=Search apps and files
Exec=$(desktop_quote "$bin_dir/spotlight") --$action
Icon=io.github.Maglcite.Spotlight
Terminal=false
Categories=Utility;Search;
StartupWMClass=io.github.Maglcite.Spotlight
DESKTOP
}

if [[ "$autostart" == true && -e "$autostart_file" ]]; then
  printf 'Refusing to overwrite existing autostart entry: %s\n' "$autostart_file" >&2
  exit 1
fi

if [[ "$no_build" == false ]]; then
  if command -v cargo >/dev/null 2>&1; then
    cargo_bin=$(command -v cargo)
  elif [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  else
    printf 'Cargo was not found on PATH or at %s/.cargo/bin/cargo\n' "$HOME" >&2
    exit 1
  fi
  "$cargo_bin" build --release --locked --manifest-path "$repo_root/Cargo.toml"
elif [[ ! -x "$binary" ]]; then
  printf 'No executable release binary found at %s; build it first or omit --no-build.\n' "$binary" >&2
  exit 1
fi

install -Dm755 "$binary" "$bin_dir/spotlight"
install -Dm644 "$repo_root/assets/io.github.Maglcite.Spotlight.svg" \
  "$icon_dir/io.github.Maglcite.Spotlight.svg"

mkdir -p "$applications_dir"
write_desktop_entry "$applications_dir/io.github.Maglcite.Spotlight.desktop" toggle
chmod 644 "$applications_dir/io.github.Maglcite.Spotlight.desktop"

if [[ "$autostart" == true ]]; then
  install -d "$(dirname -- "$autostart_file")"
  write_desktop_entry "$autostart_file" daemon
  chmod 644 "$autostart_file"
fi

printf 'Installed Spotlight. Run: %s --toggle\n' "$bin_dir/spotlight"
