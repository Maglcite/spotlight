//! API keys live in the OS credential store: Windows Credential Manager or the
//! freedesktop Secret Service (GNOME Keyring, KWallet, ...) on Linux. Keys are
//! never written to the JSON config.

use crate::config::APP_ID;

fn entry(provider: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(APP_ID, provider).map_err(|e| format!("Keyring is unavailable: {e}"))
}

/// Look up the stored key for a provider ID. `Ok(None)` means nothing is stored.
pub fn get(provider: &str) -> Result<Option<String>, String> {
    let stored = match entry(provider)?.get_password() {
        Ok(value) => Some(value),
        Err(keyring::Error::NoEntry) => None,
        Err(error) => {
            // A locked or missing keyring should not hide a legacy entry.
            tracing::debug!("keyring lookup failed: {error}");
            None
        }
    };
    if let Some(value) = stored
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
    {
        return Ok(Some(value));
    }
    Ok(legacy_lookup(provider))
}

/// Store (or replace) the key for a provider ID.
pub fn set(provider: &str, key: &str) -> Result<(), String> {
    entry(provider)?
        .set_password(key.trim())
        .map_err(|error| match error {
            keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
                "The system keyring could not save your key. Unlock it and try again.".to_string()
            }
            other => format!("The system keyring could not save your key: {other}"),
        })
}

/// Remove a stored key. Missing entries are not an error.
pub fn delete(provider: &str) -> Result<(), String> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "The system keyring could not remove your key: {error}"
        )),
    }
}

/// Earlier Linux builds stored keys with libsecret's `secret-tool` under the
/// attributes `application=<APP_ID> provider=<id>`. Read those if present.
#[cfg(target_os = "linux")]
fn legacy_lookup(provider: &str) -> Option<String> {
    let output = std::process::Command::new("secret-tool")
        .args(["lookup", "application", APP_ID, "provider", provider])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(not(target_os = "linux"))]
fn legacy_lookup(_provider: &str) -> Option<String> {
    None
}
