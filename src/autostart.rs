//! "Start at login": an XDG autostart entry on Linux and a per-user `Run`
//! registry value on Windows. Both start `spotlight --daemon`.

use std::path::{Path, PathBuf};

fn exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("Could not find the Spotlight binary: {e}"))
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use spotlight::config::APP_ID;

    fn entry_path() -> Option<PathBuf> {
        Some(
            dirs::config_dir()?
                .join("autostart")
                .join(format!("{APP_ID}.desktop")),
        )
    }

    pub fn is_enabled() -> bool {
        entry_path().is_some_and(|path| path.is_file())
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let path = entry_path().ok_or("Could not find your config directory.")?;
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("Could not remove {}: {error}", path.display()))
                }
                _ => Ok(()),
            };
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, desktop_entry(&exe()?))
            .map_err(|e| format!("Could not write {}: {e}", path.display()))
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "Spotlight";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    pub fn is_enabled() -> bool {
        let (key, value) = (wide(RUN_KEY), wide(VALUE));
        // SAFETY: both names are NUL-terminated; a null buffer only queries existence.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let (key, value) = (wide(RUN_KEY), wide(VALUE));
        let status = if enabled {
            let data = wide(&run_command(&exe()?));
            // SAFETY: names are NUL-terminated and `data` is a NUL-terminated
            // UTF-16 string whose byte length is passed alongside it.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    value.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            }
        } else {
            // SAFETY: both names are NUL-terminated.
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                status => status,
            }
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "Could not update the startup entry: {}",
                std::io::Error::from_raw_os_error(status as i32)
            ))
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn set(_enabled: bool) -> Result<(), String> {
        Err("Start at login is not supported on this platform.".into())
    }
}

pub use imp::{is_enabled, set};

/// The `Run` value: the quoted executable path followed by `--daemon`.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_command(exe: &Path) -> String {
    format!("\"{}\" --daemon", exe.display())
}

/// An XDG autostart entry matching the one `scripts/install.sh` writes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Spotlight\nComment=Search apps and files\n\
         Exec={} --daemon\nIcon={}\nTerminal=false\nCategories=Utility;Search;\n\
         StartupWMClass={}\nX-GNOME-Autostart-enabled=true\n",
        desktop_quote(&exe.to_string_lossy()),
        spotlight::config::APP_ID,
        spotlight::config::APP_ID,
    )
}

/// Quote an Exec argument per the Desktop Entry spec: escape `"`, `` ` ``,
/// `$` and `\` inside double quotes, then escape backslashes again for the
/// string value itself.
fn desktop_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_commands_quote_the_binary() {
        assert_eq!(
            run_command(Path::new(r"C:\Users\Ann Lee\Spotlight\spotlight.exe")),
            r#""C:\Users\Ann Lee\Spotlight\spotlight.exe" --daemon"#
        );
        let entry = desktop_entry(Path::new("/home/ann/my apps/spotlight"));
        assert!(entry.contains("Exec=\"/home/ann/my apps/spotlight\" --daemon\n"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert_eq!(desktop_quote("/a$b"), r#""/a\\$b""#);
    }
}
