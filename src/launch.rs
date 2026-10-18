//! Opening search results: apps, files and web pages. Launched programs are
//! detached from Spotlight so closing the launcher never takes them down.

use crate::search::{ResultKind, SearchResult};

/// Open a search result with the platform's default handler.
pub fn activate(result: &SearchResult) -> Result<(), String> {
    match &result.kind {
        ResultKind::App { desktop } => platform::open_app(desktop),
        ResultKind::File { path } => platform::open_path(path),
        ResultKind::Web { url } => {
            // Only hand real web URLs to the shell; other schemes can run handlers.
            let parsed = url::Url::parse(url).map_err(|_| "Invalid web address".to_string())?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("Only HTTP(S) web addresses can be opened".into());
            }
            platform::open_url(parsed.as_str())
        }
    }
}

/// Whether `program` is a path to a file, or a name found on `PATH`.
#[cfg(target_os = "linux")]
pub(crate) fn executable_exists(program: &str) -> bool {
    use std::path::Path;
    let path = Path::new(program);
    if path.components().count() > 1 {
        return path.is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

#[cfg(target_os = "linux")]
mod platform {
    use super::executable_exists;
    use freedesktop_desktop_entry::{DesktopEntry, get_languages_from_env};
    use std::ffi::OsStr;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    pub(super) fn open_path(path: &Path) -> Result<(), String> {
        spawn_detached("xdg-open", &[path.as_os_str()])
    }

    pub(super) fn open_url(url: &str) -> Result<(), String> {
        spawn_detached("xdg-open", &[OsStr::new(url)])
    }

    pub(super) fn open_app(desktop: &Path) -> Result<(), String> {
        let locales = get_languages_from_env();
        let entry = DesktopEntry::from_path(desktop.to_path_buf(), Some(&locales))
            .map_err(|error| format!("Could not read application entry: {error}"))?;
        if !entry.terminal() && executable_exists("gio") {
            return spawn_detached("gio", &[OsStr::new("launch"), desktop.as_os_str()]);
        }
        let args = parse_exec(
            entry
                .exec()
                .ok_or_else(|| "Application has no Exec command".to_owned())?,
            entry.name(&locales).as_deref().unwrap_or(""),
            entry.icon().unwrap_or(""),
            desktop,
        )?;
        let Some((program, rest)) = args.split_first() else {
            return Err("Application has an empty Exec command".into());
        };
        let (mut command, label) = if entry.terminal() {
            let mut terminal = terminal_command()?;
            let label = terminal.first().cloned().unwrap_or_default();
            let mut command = Command::new(terminal.remove(0));
            command.args(terminal).arg("-e").args(&args);
            (command, label)
        } else {
            // No gio: run the parsed Exec line directly, still without a shell.
            let mut command = Command::new(program);
            command.args(rest);
            (command, program.clone())
        };
        if let Some(path) = entry.path() {
            command.current_dir(path);
        }
        spawn(command, &label)
    }

    /// cosmic-term first (the original target desktop), then `$TERMINAL`, then
    /// the Debian-style `x-terminal-emulator` alternative.
    fn terminal_command() -> Result<Vec<String>, String> {
        if executable_exists("cosmic-term") {
            return Ok(vec!["cosmic-term".into()]);
        }
        if let Ok(value) = std::env::var("TERMINAL")
            && let Some(words) = shlex::split(&value)
            && words
                .first()
                .is_some_and(|program| executable_exists(program))
        {
            return Ok(words);
        }
        if executable_exists("x-terminal-emulator") {
            return Ok(vec!["x-terminal-emulator".into()]);
        }
        Err("No terminal emulator found. Set $TERMINAL to run terminal apps.".into())
    }

    fn spawn_detached(program: &str, args: &[&OsStr]) -> Result<(), String> {
        let mut command = Command::new(program);
        command.args(args);
        spawn(command, program)
    }

    fn spawn(mut command: Command, program: &str) -> Result<(), String> {
        // A separate process group keeps terminal signals aimed at Spotlight
        // (e.g. Ctrl+C on the CLI) away from the launched app.
        command.stdin(Stdio::null()).process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start {program}: {error}"))?;
        // Reap the launcher helper without blocking the desktop UI or retaining zombies.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    /// Parse the Desktop Entry Exec grammar's quoting and backslash rules, expanding
    /// the file-independent codes safely without invoking a shell.
    fn parse_exec(
        exec: &str,
        name: &str,
        icon: &str,
        desktop: &Path,
    ) -> Result<Vec<String>, String> {
        let mut tokens: Vec<String> = Vec::new();
        let mut token = String::new();
        let mut quoted = false;
        let mut chars = exec.chars().peekable();
        let mut started = false;
        let mut omit_arg = false;
        while let Some(ch) = chars.next() {
            match ch {
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                '\\' => {
                    let next = chars
                        .next()
                        .ok_or_else(|| "Exec ends with an escape".to_owned())?;
                    if matches!(next, '"' | '`' | '$' | '\\') || next.is_whitespace() {
                        token.push(next);
                    } else {
                        token.push('\\');
                        token.push(next);
                    }
                    started = true;
                }
                '%' => {
                    let code = chars
                        .next()
                        .ok_or_else(|| "Exec ends with a field code".to_owned())?;
                    match code {
                        '%' => token.push('%'),
                        'f' | 'F' | 'u' | 'U' => {
                            omit_arg = true;
                        }
                        'i' => {
                            if !icon.is_empty() {
                                if started && !token.is_empty() {
                                    return Err("Exec field %i must be a separate argument".into());
                                }
                                tokens.push("--icon".into());
                                tokens.push(icon.to_owned());
                                omit_arg = true;
                            } else {
                                omit_arg = true;
                            }
                        }
                        'c' => token.push_str(name),
                        'k' => token.push_str(&desktop.to_string_lossy()),
                        _ => return Err(format!("Unknown Exec field %{code}")),
                    }
                    started = true;
                }
                c if c.is_whitespace() && !quoted => {
                    if started && !omit_arg {
                        tokens.push(std::mem::take(&mut token));
                    }
                    token.clear();
                    started = false;
                    omit_arg = false;
                }
                _ => {
                    token.push(ch);
                    started = true;
                }
            }
        }
        if quoted {
            return Err("Exec has an unmatched quote".into());
        }
        if started && !omit_arg {
            tokens.push(token);
        }
        Ok(tokens)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn exec_parser_preserves_quoted_spaces_and_literal_percent() {
            assert_eq!(
                parse_exec(
                    "cosmic-term --title \"hello world\" -e app %%",
                    "App",
                    "",
                    Path::new("/tmp/app.desktop")
                )
                .unwrap(),
                vec!["cosmic-term", "--title", "hello world", "-e", "app", "%"]
            );
            assert_eq!(
                parse_exec("run %f file", "App", "", Path::new("/tmp/app.desktop")).unwrap(),
                vec!["run", "file"]
            );
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    pub(super) fn open_path(path: &Path) -> Result<(), String> {
        shell_open(path.as_os_str())
    }

    pub(super) fn open_url(url: &str) -> Result<(), String> {
        shell_open(OsStr::new(url))
    }

    /// Start Menu entries are shortcuts, so the shell resolves and runs them.
    pub(super) fn open_app(shortcut: &Path) -> Result<(), String> {
        shell_open(shortcut.as_os_str())
    }

    /// `ShellExecuteW` "open" is what Explorer does on double-click: it honours
    /// file associations, `.lnk` targets and the default browser, and the new
    /// process is neither our child nor attached to our console. Using
    /// `cmd /c start` instead would flash a console window.
    fn shell_open(target: &OsStr) -> Result<(), String> {
        let wide = |s: &OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
        let file = wide(target);
        let verb = wide(OsStr::new("open"));
        // Some shell extensions need COM; the docs recommend STA without OLE1 DDE.
        // SAFETY: plain FFI calls with valid NUL-terminated UTF-16 buffers that
        // outlive the call; CoUninitialize is paired with a successful init.
        let code = unsafe {
            let com = CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            );
            let instance = ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
            if com >= 0 {
                CoUninitialize();
            }
            instance as isize
        };
        // Values above 32 mean success; smaller values are legacy error codes.
        if code > 32 {
            Ok(())
        } else {
            Err(format!(
                "Windows could not open {} (error {code})",
                target.to_string_lossy()
            ))
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use std::path::Path;

    pub(super) fn open_path(_: &Path) -> Result<(), String> {
        Err("Opening files is not supported on this platform".into())
    }
    pub(super) fn open_url(_: &str) -> Result<(), String> {
        Err("Opening URLs is not supported on this platform".into())
    }
    pub(super) fn open_app(_: &Path) -> Result<(), String> {
        Err("Launching apps is not supported on this platform".into())
    }
}
