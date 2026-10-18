use crate::secrets;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const APP_ID: &str = "io.github.Maglcite.Spotlight";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    OpenCodeGo,
    OpenAI,
    Anthropic,
    Gemini,
    OpenRouter,
    Ollama,
    Custom,
}

impl Provider {
    pub const ALL: [Self; 7] = [
        Self::OpenCodeGo,
        Self::OpenAI,
        Self::Anthropic,
        Self::Gemini,
        Self::OpenRouter,
        Self::Ollama,
        Self::Custom,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::OpenCodeGo => "opencode-go",
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Self::OpenCodeGo => "https://opencode.ai/zen/go/v1",
            Self::OpenAI => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::Ollama => "http://localhost:11434/v1",
            Self::Custom => "",
        }
    }

    fn env_name(self) -> &'static str {
        match self {
            Self::OpenCodeGo => "OPENCODE_API_KEY",
            Self::OpenAI => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::Gemini => "GEMINI_API_KEY",
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::Ollama => "OLLAMA_API_KEY",
            Self::Custom => "SPOTLIGHT_API_KEY",
        }
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OpenCodeGo => "OpenCode Go",
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
            Self::OpenRouter => "OpenRouter",
            Self::Ollama => "Ollama",
            Self::Custom => "Custom",
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    pub web_engine: String,
    pub web_tools: bool,
    pub local_tools: bool,
    pub search_roots: Vec<PathBuf>,
    pub max_tool_rounds: u8,
    pub debounce_ms: u64,
    pub opacity: f32,
    /// Global shortcut shown in Settings. On Wayland the compositor owns the
    /// binding, so this is informational there.
    pub hotkey: String,
    /// Start Spotlight in the background at login.
    pub autostart: bool,
}

/// The platform's conventional launcher shortcut.
pub fn default_hotkey() -> &'static str {
    if cfg!(windows) {
        "Alt+Space"
    } else {
        "Super+Space"
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: Provider::OpenCodeGo,
            model: "deepseek-v4.1-flash".into(),
            base_url: Provider::OpenCodeGo.endpoint().into(),
            web_engine: "duckduckgo".into(),
            web_tools: true,
            local_tools: false,
            search_roots: vec![dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))],
            max_tool_rounds: 4,
            debounce_ms: 650,
            opacity: 0.88,
            hotkey: default_hotkey().into(),
            autostart: false,
        }
    }
}

impl Config {
    /// `~/.config/spotlight/config.json` on Linux (unchanged from earlier
    /// releases) and `%APPDATA%\spotlight\config.json` on Windows.
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("spotlight")
            .join("config.json")
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut value: Self =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Settings could not be read: {e}"))?;
        value.debounce_ms = value.debounce_ms.clamp(250, 3000);
        value.max_tool_rounds = value.max_tool_rounds.clamp(1, 6);
        value.opacity = value.opacity.clamp(0.65, 1.0);
        if value.hotkey.trim().is_empty() {
            value.hotkey = default_hotkey().into();
        }
        Ok(value)
    }

    pub fn save(&self) -> Result<(), String> {
        self.validate()?;
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.model.trim().is_empty() {
            return Err("Enter the model ID for your provider.".into());
        }
        let url = url::Url::parse(&self.base_url)
            .map_err(|_| "Enter a valid API base URL.".to_string())?;
        let local = matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        );
        if url.scheme() != "https" && !(local && url.scheme() == "http") {
            return Err("API endpoints must use HTTPS, or HTTP on localhost.".into());
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Put your API key in the API key field.".into());
        }
        if self
            .search_roots
            .iter()
            .any(|p| !p.is_absolute() || !p.is_dir())
        {
            return Err("Search folders must be existing absolute paths.".into());
        }
        Ok(())
    }

    /// Resolve the API key: provider environment variable first, then the OS
    /// keyring (including the legacy `secret-tool` entry on Linux). This may
    /// block briefly on the keyring, so call it off the UI thread.
    pub fn api_key(&self) -> Result<String, String> {
        if let Ok(value) = std::env::var(self.provider.env_name())
            && !value.trim().is_empty()
        {
            return Ok(value.trim().to_string());
        }
        match secrets::get(self.provider.id()) {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(error) => tracing::debug!("{error}"),
        }
        if self.provider == Provider::Ollama {
            return Ok("ollama".into());
        }
        Err(format!(
            "Add your {} API key in Settings, or set {}.",
            self.provider,
            self.provider.env_name()
        ))
    }

    /// Save a key for the current provider in the OS keyring. Blank keys are ignored.
    pub fn store_key(&self, key: &str) -> Result<(), String> {
        if key.trim().is_empty() {
            return Ok(());
        }
        secrets::set(self.provider.id(), key)
    }

    /// Forget the stored key for the current provider.
    pub fn clear_key(&self) -> Result<(), String> {
        secrets::delete(self.provider.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_never_serialize() {
        let config = Config::default();
        let value = serde_json::to_string(&config).unwrap();
        assert!(!value.contains("api_key"));
        assert!(!value.to_lowercase().contains("secret"));
        assert!(serde_json::from_str::<Config>(&value).is_ok());
    }

    #[test]
    fn configs_from_older_releases_still_load() {
        let old = r#"{"provider":"OpenAI","model":"gpt","base_url":"https://api.openai.com/v1","opacity":0.9}"#;
        let config: Config = serde_json::from_str(old).unwrap();
        assert_eq!(config.provider, Provider::OpenAI);
        assert_eq!(config.hotkey, default_hotkey());
        assert!(!config.autostart);
        assert_eq!(config.debounce_ms, 650);
    }
    #[test]
    fn endpoint_validation() {
        let mut config = Config::default();
        for endpoint in ["http://localhost:11434/v1", "https://api.openai.com/v1"] {
            config.base_url = endpoint.into();
            assert!(config.validate().is_ok());
        }
        for endpoint in [
            "http://example.com/v1",
            "file:///tmp/key",
            "https://user:pass@example.com/v1",
        ] {
            config.base_url = endpoint.into();
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn defaults_match_requested_provider_and_model() {
        let config = Config::default();
        assert_eq!(config.provider, Provider::OpenCodeGo);
        assert_eq!(config.model, "deepseek-v4.1-flash");
        assert_eq!(config.base_url, "https://opencode.ai/zen/go/v1");
        assert_eq!(config.debounce_ms, 650);
    }
}
