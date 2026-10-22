//! User-editable settings, persisted as TOML at `settings.toml` in the layout
//! root. Settings are *input* to the system; generated configuration is
//! *output* rendered from the state store, never hand-edited.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::util;

/// Which web server fronts PHP for new sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum WebServer {
    /// nginx fronting php-cgi (Windows) or php-fpm (Linux).
    #[default]
    Nginx,
    /// Apache httpd fronting php-cgi.
    Apache,
}

impl WebServer {
    /// Stable lowercase name, as stored on a site row.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nginx => "nginx",
            Self::Apache => "apache",
        }
    }
}


/// Which model provider backs the conversational agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum AgentProvider {
    /// No provider configured; the agent surface reports as unavailable.
    #[default]
    None,
    /// Anthropic Messages API.
    Anthropic,
    /// Any OpenAI-compatible chat-completions endpoint.
    OpenAiCompatible,
}


/// All user-editable settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Development TLD. `.test` is reserved by RFC 6761 for exactly this use.
    pub tld: String,
    /// Port the local HTTP listener binds.
    pub http_port: u16,
    /// Port the local HTTPS listener binds.
    pub https_port: u16,
    /// Root directory projects live in. Defaults to `<root>/www`.
    pub www_root: Option<std::path::PathBuf>,
    /// Default web server for new sites.
    pub web_server: WebServer,
    /// PHP version used when a site does not pin one.
    pub primary_php: Option<String>,
    /// Override for the `git` executable. Discovered from `PATH` when unset.
    pub git_executable: Option<String>,
    /// Refuse to run operations that would discard uncommitted work unless the
    /// caller passes an explicit override.
    pub git_protect_uncommitted: bool,
    /// Branch name used when initialising a new repository.
    pub git_default_branch: String,
    /// Model provider for the conversational agent.
    pub agent_provider: AgentProvider,
    /// Model identifier for the configured provider.
    pub agent_model: Option<String>,
    /// Whether the agent may propose `write`-and-above capabilities without a
    /// human in the loop. Destructive and admin capabilities are never
    /// auto-approved regardless of this setting.
    pub agent_auto_approve_write: bool,
    /// Folders besides the projects root whose project directories become sites
    /// (`site.park`).
    pub parked: Vec<std::path::PathBuf>,
    /// Extra directories, besides the projects root, in which agents may create
    /// and run projects. Humans are not limited to these.
    pub project_roots: Vec<std::path::PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        let (http_port, https_port) = crate::platform::Os::current().default_ports();
        Self {
            tld: "test".to_string(),
            http_port,
            https_port,
            www_root: None,
            web_server: WebServer::default(),
            primary_php: None,
            git_executable: None,
            git_protect_uncommitted: true,
            git_default_branch: "main".to_string(),
            agent_provider: AgentProvider::default(),
            agent_model: None,
            agent_auto_approve_write: false,
            project_roots: Vec::new(),
            parked: Vec::new(),
        }
    }
}

impl Settings {
    /// Load from `path`, falling back to defaults when the file is absent.
    pub fn load(path: &Path) -> Result<Self> {
        match util::read_to_string_opt(path)? {
            Some(text) => Ok(toml::from_str(&text)?),
            None => Ok(Self::default()),
        }
    }

    /// Persist to `path` atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)?;
        util::atomic_write(path, &text)
    }

    /// Effective projects directory.
    pub fn effective_www_root(&self, fallback: &Path) -> std::path::PathBuf {
        self.www_root
            .clone()
            .unwrap_or_else(|| fallback.to_path_buf())
    }

    /// Validate the settings, returning a descriptive error when unusable.
    pub fn validate(&self) -> Result<()> {
        util::validate_name(&self.tld)?;
        if self.tld == "localhost" || self.tld == "local" {
            return Err(crate::error::DevdError::invalid(
                "`.local` and `localhost` are not supported as the development TLD; use `.test`",
            ));
        }
        if self.http_port == 0 || self.https_port == 0 {
            return Err(crate::error::DevdError::invalid("ports must be non-zero"));
        }
        if self.http_port == self.https_port {
            return Err(crate::error::DevdError::invalid(
                "http_port and https_port must differ",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.tld, "test");
        assert_eq!(
            (s.http_port, s.https_port),
            crate::platform::Os::current().default_ports()
        );
        assert!(s.git_protect_uncommitted);
        s.validate().unwrap();
    }

    #[test]
    fn either_web_server_is_a_valid_choice_and_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("devd-cfg-ws-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("settings.toml");
        for server in [WebServer::Nginx, WebServer::Apache] {
            let mut s = Settings::default();
            s.web_server = server;
            s.validate().unwrap();
            s.save(&f).unwrap();
            assert_eq!(Settings::load(&f).unwrap().web_server, server);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("devd-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("settings.toml");
        let mut s = Settings::default();
        s.tld = "internal".into();
        s.http_port = 8080;
        s.save(&f).unwrap();
        let back = Settings::load(&f).unwrap();
        assert_eq!(back, s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_tld_is_rejected() {
        let mut s = Settings::default();
        s.tld = "local".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let p = std::env::temp_dir().join("devd-does-not-exist-xyz/settings.toml");
        assert_eq!(Settings::load(&p).unwrap(), Settings::default());
    }
}
