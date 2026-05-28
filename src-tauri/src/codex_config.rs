//! Parsing helpers for the user's `~/.codex/config.toml`.
//!
//! Used in two places:
//!  - `commands::system_commands::get_agent_login_status` — detect whether
//!    Codex is reachable via an API-key provider (e.g. Azure) when the
//!    user hasn't run `codex login`.
//!  - `codex_provider_env` — extend the env-var
//!    inheritance whitelist with whichever `env_key` names the user has
//!    declared, so a Finder-launched Helmor.app sees the same API keys
//!    as a terminal session.

use std::io::Read;
use std::path::{Path, PathBuf};

const WRAPPER_SCAN_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyProvider {
    pub name: String,
    pub env_key: String,
}

/// Resolve `~/.codex/config.toml`, honouring `$CODEX_HOME` if set.
pub fn config_path() -> PathBuf {
    default_codex_home().join("config.toml")
}

pub fn default_codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".codex"))
}

pub fn config_path_for_home(codex_home: &Path) -> PathBuf {
    codex_home.join("config.toml")
}

pub fn auth_path_for_home(codex_home: &Path) -> PathBuf {
    codex_home.join("auth.json")
}

pub fn selected_codex_home() -> PathBuf {
    crate::sidecar::load_codex_executable_path()
        .as_deref()
        .and_then(|path| codex_home_from_executable(Path::new(path)))
        .unwrap_or_else(default_codex_home)
}

pub fn codex_home_from_executable(path: &Path) -> Option<PathBuf> {
    let path = resolve_executable_path(path);
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(WRAPPER_SCAN_BYTES as usize);
    file.take(WRAPPER_SCAN_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines().take(200) {
        if let Some(value) = parse_codex_home_assignment(line) {
            return Some(expand_codex_home_value(&value));
        }
    }
    None
}

fn resolve_executable_path(path: &Path) -> PathBuf {
    crate::path_utils::resolve_path_command(path, std::env::var_os("PATH").as_deref())
}

fn parse_codex_home_assignment(line: &str) -> Option<String> {
    let line = line.trim();
    let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
    let value = line.strip_prefix("CODEX_HOME=")?;
    let value = value.trim_start();
    if value.is_empty() {
        return None;
    }
    if let Some(rest) = value.strip_prefix('"') {
        return rest.split('"').next().map(str::to_string);
    }
    if let Some(rest) = value.strip_prefix('\'') {
        return rest.split('\'').next().map(str::to_string);
    }
    value
        .split_whitespace()
        .next()
        .map(|part| part.trim_end_matches(';').to_string())
}

fn expand_codex_home_value(value: &str) -> PathBuf {
    let home = home_dir();
    let home_str = home.to_string_lossy();
    let expanded = value
        .replace("${HOME}", &home_str)
        .replace("$HOME", &home_str);
    if expanded == "~" {
        return home;
    }
    if let Some(rest) = expanded.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(expanded)
}

/// The provider currently selected by `model_provider`, if it declares an
/// `env_key`. Returns `None` when no provider is active, the active
/// provider isn't an API-key flavour, or the config can't be parsed.
pub fn active_api_key_provider(config: &str) -> Option<ApiKeyProvider> {
    let value = toml::from_str::<toml::Value>(config).ok()?;
    let provider = value
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|provider| !provider.is_empty())?;
    let env_key = value
        .get("model_providers")
        .and_then(|providers| providers.get(provider))
        .and_then(|provider_config| provider_config.get("env_key"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|env_key| !env_key.is_empty())?;

    Some(ApiKeyProvider {
        name: provider.to_string(),
        env_key: env_key.to_string(),
    })
}

/// Every `env_key` declared under `[model_providers.*]` — regardless of
/// which provider is currently active. We surface them all so the user
/// can switch providers in `config.toml` without restarting Helmor.
///
/// Returns an empty vec on parse failure or when no providers declare an
/// `env_key`.
pub fn declared_env_keys(config: &str) -> Vec<String> {
    let Ok(value) = toml::from_str::<toml::Value>(config) else {
        return Vec::new();
    };
    let Some(providers) = value.get("model_providers").and_then(toml::Value::as_table) else {
        return Vec::new();
    };

    let mut keys = Vec::new();
    for provider in providers.values() {
        let Some(env_key) = provider
            .get("env_key")
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
        else {
            continue;
        };
        let env_key = env_key.to_string();
        if !keys.contains(&env_key) {
            keys.push(env_key);
        }
    }
    keys
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_provider_reads_azure_env_key() {
        let provider = active_api_key_provider(
            r#"
model = "gpt-5.5"
model_provider = "azure"

[model_providers.azure]
name = "Azure"
base_url = "https://example.openai.azure.com/openai/v1"
env_key = "AZURE_OPENAI_API_KEY"
wire_api = "responses"
"#,
        );

        assert_eq!(
            provider,
            Some(ApiKeyProvider {
                name: "azure".to_string(),
                env_key: "AZURE_OPENAI_API_KEY".to_string(),
            })
        );
    }

    #[test]
    fn active_provider_returns_none_when_section_missing() {
        let provider = active_api_key_provider(
            r#"
model_provider = "azure"

[model_providers.openai]
env_key = "OPENAI_API_KEY"
"#,
        );

        assert_eq!(provider, None);
    }

    #[test]
    fn active_provider_returns_none_without_env_key() {
        let provider = active_api_key_provider(
            r#"
model_provider = "azure"

[model_providers.azure]
base_url = "https://example.openai.azure.com/openai/v1"
"#,
        );

        assert_eq!(provider, None);
    }

    #[test]
    fn declared_env_keys_collects_every_provider() {
        let keys = declared_env_keys(
            r#"
model_provider = "azure"

[model_providers.azure]
env_key = "AZURE_OPENAI_API_KEY"

[model_providers.openai]
env_key = "OPENAI_API_KEY"

[model_providers.no_key]
base_url = "https://example.com"
"#,
        );

        assert_eq!(keys, vec!["AZURE_OPENAI_API_KEY", "OPENAI_API_KEY"]);
    }

    #[test]
    fn declared_env_keys_deduplicates() {
        let keys = declared_env_keys(
            r#"
[model_providers.a]
env_key = "SHARED_KEY"

[model_providers.b]
env_key = "SHARED_KEY"
"#,
        );

        assert_eq!(keys, vec!["SHARED_KEY"]);
    }

    #[test]
    fn declared_env_keys_handles_empty_config() {
        assert!(declared_env_keys("").is_empty());
        assert!(declared_env_keys("not valid toml = [").is_empty());
        assert!(declared_env_keys("[other]\nfoo = 1").is_empty());
    }

    #[test]
    fn parses_codex_home_from_shell_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = dir.path().join("codexaz");
        std::fs::write(
            &wrapper,
            r#"#!/usr/bin/env bash
export CODEX_HOME="$HOME/.codexaz"
exec /usr/local/bin/codex "$@"
"#,
        )
        .unwrap();

        assert_eq!(
            codex_home_from_executable(&wrapper),
            Some(home_dir().join(".codexaz"))
        );
    }

    #[test]
    fn resolves_path_command_before_scanning_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = dir.path().join("codexaz");
        std::fs::write(
            &wrapper,
            r#"#!/usr/bin/env bash
CODEX_HOME="$HOME/.codexaz" exec /usr/local/bin/codex "$@"
"#,
        )
        .unwrap();

        assert_eq!(
            crate::path_utils::resolve_path_command(
                Path::new("codexaz"),
                Some(dir.path().as_os_str()),
            ),
            wrapper
        );
    }

    #[test]
    fn reads_only_wrapper_prefix_when_detecting_codex_home() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = dir.path().join("codexaz");
        let mut bytes = br#"#!/usr/bin/env bash
export CODEX_HOME="$HOME/.codexaz"
"#
        .to_vec();
        bytes.extend(std::iter::repeat_n(
            b'x',
            (WRAPPER_SCAN_BYTES as usize) + 1024,
        ));
        std::fs::write(&wrapper, bytes).unwrap();

        assert_eq!(
            codex_home_from_executable(&wrapper),
            Some(home_dir().join(".codexaz"))
        );
    }
}
