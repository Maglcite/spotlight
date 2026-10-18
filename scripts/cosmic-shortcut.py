#!/usr/bin/env python3
"""Safely route COSMIC's system Launcher action to Spotlight."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import sys
import tempfile


RELATIVE_CONFIG = Path("cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions")
BACKUP_SUFFIX = ".spotlight-backup"
STATE_SUFFIX = ".spotlight-state.json"


class ConfigError(Exception):
    pass


def skip_space_and_comments(text: str, pos: int) -> int:
    while pos < len(text):
        if text[pos].isspace():
            pos += 1
        elif text.startswith("//", pos):
            end = text.find("\n", pos + 2)
            pos = len(text) if end < 0 else end + 1
        elif text.startswith("/*", pos):
            depth = 1
            pos += 2
            while depth and pos < len(text):
                if text.startswith("/*", pos):
                    depth += 1
                    pos += 2
                elif text.startswith("*/", pos):
                    depth -= 1
                    pos += 2
                else:
                    pos += 1
            if depth:
                raise ConfigError("Unterminated block comment in COSMIC config.")
        else:
            break
    return pos


def quoted_end(text: str, pos: int) -> int:
    quote = text[pos]
    pos += 1
    while pos < len(text):
        if text[pos] == "\\":
            pos += 2
        elif text[pos] == quote:
            return pos + 1
        else:
            pos += 1
    raise ConfigError("Unterminated string in COSMIC config.")


def value_end(text: str, pos: int) -> int:
    """Find a RON value's end, respecting nested values, strings and comments."""
    stack: list[str] = []
    pairs = {"(": ")", "[": "]", "{": "}"}
    while pos < len(text):
        char = text[pos]
        if char in ('"', "'"):
            pos = quoted_end(text, pos)
            continue
        if text.startswith("//", pos):
            end = text.find("\n", pos + 2)
            after = len(text) if end < 0 else end + 1
            if not stack:
                next_token = skip_space_and_comments(text, after)
                if next_token >= len(text) or text[next_token] in ",}":
                    return pos
            pos = after
            continue
        if text.startswith("/*", pos):
            after = skip_space_and_comments(text, pos)
            if not stack:
                next_token = skip_space_and_comments(text, after)
                if next_token >= len(text) or text[next_token] in ",}":
                    return pos
            pos = after
            continue
        if char in pairs:
            stack.append(pairs[char])
        elif char in ")]}" :
            if not stack:
                if char == "}":
                    return pos
                raise ConfigError("Unexpected closing delimiter in COSMIC config.")
            if stack.pop() != char:
                raise ConfigError("Mismatched delimiters in COSMIC config.")
        elif char == "," and not stack:
            return pos
        pos += 1
    if stack:
        raise ConfigError("Unterminated nested value in COSMIC config.")
    return pos


def parse_entries(text: str) -> tuple[list[tuple[str, int, int]], int]:
    """Return (key, value_start, value_end) entries and outer closing brace."""
    pos = skip_space_and_comments(text, 0)
    if pos >= len(text) or text[pos] != "{":
        raise ConfigError("Expected a top-level RON map in system_actions.")
    pos = skip_space_and_comments(text, pos + 1)
    entries: list[tuple[str, int, int]] = []
    closing = -1
    while pos < len(text):
        if text[pos] == "}":
            closing = pos
            pos = skip_space_and_comments(text, pos + 1)
            break
        key_start = pos
        if text[pos] in ('"', "'"):
            key_end = quoted_end(text, pos)
            try:
                key = json.loads(text[key_start:key_end]) if text[pos] == '"' else text[key_start + 1:key_end - 1]
            except (json.JSONDecodeError, TypeError):
                key = ""
        else:
            while pos < len(text) and (text[pos].isalnum() or text[pos] == "_"):
                pos += 1
            key_end = pos
            key = text[key_start:key_end]
        if key_end == key_start:
            raise ConfigError("Could not parse a mapping key in system_actions.")
        pos = skip_space_and_comments(text, key_end)
        if pos >= len(text) or text[pos] != ":":
            raise ConfigError("Expected ':' after a system_actions key.")
        value_start = skip_space_and_comments(text, pos + 1)
        if value_start >= len(text):
            raise ConfigError("Missing value in system_actions.")
        end = value_end(text, value_start)
        if end == value_start:
            raise ConfigError("Empty value in system_actions.")
        entries.append((key, value_start, end))
        if end < len(text) and text[end] == ",":
            pos = skip_space_and_comments(text, end + 1)
        else:
            pos = skip_space_and_comments(text, end)
            if pos >= len(text) or text[pos] != "}":
                raise ConfigError("Expected ',' or '}' after a system_actions value.")
    if closing < 0 or pos != len(text):
        raise ConfigError("Unexpected data after the system_actions map.")
    names = [key for key, _, _ in entries]
    if names.count("Launcher") > 1:
        raise ConfigError("Found multiple Launcher entries; refusing to guess which one to change.")
    return entries, closing


def write_atomic(path: Path, data: bytes, mode: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as temp_file:
            temp_file.write(data)
            temp_file.flush()
            os.fsync(temp_file.fileno())
        os.chmod(temp_name, mode)
        os.replace(temp_name, path)
    except BaseException:
        try:
            os.unlink(temp_name)
        except FileNotFoundError:
            pass
        raise


def config_path() -> Path:
    config_home = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    if not config_home.is_absolute():
        raise ConfigError("XDG_CONFIG_HOME must be an absolute path.")
    return config_home / RELATIVE_CONFIG


def command_value() -> str:
    binary = Path.home() / ".local/bin/spotlight"
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ConfigError(f"{binary} is missing or not executable; install Spotlight first.")
    return shlex.join([str(binary), "--toggle"])


def install(path: Path) -> None:
    backup = Path(str(path) + BACKUP_SUFFIX)
    state_path = Path(str(path) + STATE_SUFFIX)
    command = command_value()
    if path.is_symlink():
        raise ConfigError("system_actions is a symlink; refusing to replace it.")
    existed = path.exists()
    original = path.read_bytes() if existed else b"{}\n"
    try:
        text = original.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ConfigError("system_actions is not UTF-8; refusing to modify it.") from exc
    entries, closing = parse_entries(text)
    matches = [(start, end) for key, start, end in entries if key == "Launcher"]

    if matches and text[matches[0][0]:matches[0][1]].strip() == json.dumps(command):
        print("Spotlight is already configured as the COSMIC Launcher action.")
        return
    if backup.exists() or state_path.exists():
        raise ConfigError(f"A Spotlight backup already exists ({backup}); restore it before installing again.")

    # Create the exact backup before changing system_actions. Exclusive create
    # avoids replacing a backup made by an earlier or interrupted operation.
    backup.parent.mkdir(parents=True, exist_ok=True)
    backup_created = False
    state_created = False
    try:
        with backup.open("xb") as file:
            file.write(original)
        backup_created = True
        state = {"existed": existed, "mode": path.stat().st_mode & 0o777 if existed else 0o644}
        with state_path.open("x", encoding="utf-8") as file:
            state_created = True
            json.dump(state, file)
            file.write("\n")
    except FileExistsError as exc:
        if backup_created:
            backup.unlink(missing_ok=True)
        if state_created:
            state_path.unlink(missing_ok=True)
        raise ConfigError("A Spotlight backup already exists; refusing to overwrite it.") from exc
    except OSError:
        if backup_created:
            backup.unlink(missing_ok=True)
        if state_created:
            state_path.unlink(missing_ok=True)
        raise

    if matches:
        start, end = matches[0]
        updated = text[:start] + json.dumps(command) + text[end:]
    else:
        if entries:
            last_end = entries[-1][2]
            if last_end >= len(text) or text[last_end] != ",":
                text = text[:last_end] + "," + text[last_end:]
                closing += 1
        updated = text[:closing] + f'\n    Launcher: {json.dumps(command)},\n' + text[closing:]
    try:
        write_atomic(path, updated.encode("utf-8"), state["mode"])
    except BaseException:
        backup.unlink(missing_ok=True)
        state_path.unlink(missing_ok=True)
        raise
    print(f"COSMIC Launcher now runs: {command}")
    print(f"Original config saved at: {backup}")
    print("Other system actions and keyboard mappings, including Alt+Tab, are unchanged.")


def restore(path: Path) -> None:
    backup = Path(str(path) + BACKUP_SUFFIX)
    state_path = Path(str(path) + STATE_SUFFIX)
    if not backup.is_file() or not state_path.is_file():
        raise ConfigError("No Spotlight backup was found; nothing was restored.")
    try:
        state = json.loads(state_path.read_text(encoding="utf-8"))
        if state.get("existed"):
            write_atomic(path, backup.read_bytes(), int(state["mode"]))
        else:
            path.unlink(missing_ok=True)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        raise ConfigError(f"Could not restore the COSMIC config: {exc}") from exc
    backup.unlink()
    state_path.unlink()
    print("Restored the exact COSMIC system_actions file from the Spotlight backup.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    operation = parser.add_mutually_exclusive_group(required=True)
    operation.add_argument("--install", action="store_true", help="route COSMIC's Launcher action to Spotlight")
    operation.add_argument("--restore", action="store_true", help="restore the exact system_actions backup")
    args = parser.parse_args()
    try:
        path = config_path()
        if args.install:
            install(path)
        else:
            restore(path)
    except (ConfigError, OSError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
