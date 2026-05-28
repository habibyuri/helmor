use anyhow::Context;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::{LazyLock, Mutex},
};
use tauri::State;

use crate::{
    agents::{ActionKind, SlashCommandCache},
    db,
    rate_limits::throttle::Throttle,
    settings,
    sidecar::ManagedSidecar,
};

use super::common::{run_blocking, CmdResult};

/// 30 s belt-and-suspenders gate for rate-limit fetchers. Independent
/// of the frontend's 2 min `refetchInterval` and hover-triggered
/// refetches: even if the UI somehow hammers the command (event-loop
/// bug, runaway hover handler), the upstream HTTP call still fires at
/// most once per provider per 30 s. Within the cooldown window the
/// caller gets the cached body verbatim.
const RATE_LIMITS_THROTTLE_SECONDS: i64 = 30;
static CLAUDE_RATE_LIMITS_THROTTLE: Throttle = Throttle::new(RATE_LIMITS_THROTTLE_SECONDS);
static CODEX_RATE_LIMITS_THROTTLES: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[tauri::command]
pub async fn get_app_settings() -> CmdResult<std::collections::HashMap<String, String>> {
    run_blocking(|| {
        let conn = db::read_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT key, value FROM settings WHERE key LIKE 'app.%' OR key LIKE 'branch_prefix_%'",
            )
            .context("Failed to query app settings")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to iterate app settings")?;

        let mut map = std::collections::HashMap::new();
        for row in rows.flatten() {
            map.insert(row.0, row.1);
        }
        Ok(map)
    })
    .await
}

#[tauri::command]
pub async fn update_app_settings(
    sidecar: State<'_, ManagedSidecar>,
    slash_cache: State<'_, SlashCommandCache>,
    settings_map: std::collections::HashMap<String, String>,
) -> CmdResult<()> {
    let touched_cursor_key = settings_map.contains_key("app.cursor_provider");
    let touched_codex_binary_key = settings_map.contains_key("app.codex_executable_path");
    run_blocking(move || {
        for (key, value) in &settings_map {
            if !key.starts_with("app.") && !key.starts_with("branch_prefix_") {
                continue;
            }
            settings::upsert_setting_value(key, value)?;
        }
        Ok(())
    })
    .await?;

    // Hot-push the key — restart would interrupt other providers.
    if touched_cursor_key {
        sidecar.push_cursor_api_key(crate::sidecar::load_cursor_api_key());
    }
    if touched_codex_binary_key {
        slash_cache.clear_provider("codex");
        let codex_env = crate::codex_provider_env::codex_provider_env_for_selected_home();
        sidecar.push_codex_binary_config(crate::sidecar::load_codex_executable_path(), codex_env);
    }
    Ok(())
}

/// Read the account-global Codex rate-limit snapshot. Each call attempts
/// a live `wham/usage` fetch via the selected Codex home
/// (`CODEX_HOME` from the configured executable wrapper, otherwise
/// `~/.codex`) and falls back to the cached body on failure. API-key
/// providers from that Codex config (Azure, custom OpenAI endpoints)
/// do not have ChatGPT usage windows, so this clears cached ChatGPT
/// usage and returns `None` for them.
/// `app.codex_rate_limits` stores the raw response — no shape mapping —
/// so downstream parsing lives entirely in the frontend, mirroring the
/// Claude pipeline.
///
/// Frontend `useQuery` already caches the returned body and gates
/// repeat calls via `staleTime` / `refetchInterval`. We deliberately do
/// NOT publish a `*RateLimitsChanged` UI-sync event from this command
/// — that would invalidate the same query key the frontend just
/// resolved and trigger an immediate refetch, looping into HTTP 429.
#[tauri::command]
pub async fn get_codex_rate_limits(
    codex_executable_path: Option<String>,
) -> CmdResult<Option<String>> {
    run_blocking(move || {
        let codex_home = codex_home_for_executable_path(codex_executable_path.as_deref());
        let cache_key = codex_rate_limits_cache_key(&codex_home);
        if codex_uses_metered_api_key(&codex_home) {
            settings::delete_setting_value(&cache_key)?;
            return Ok(None);
        }
        let cached = settings::load_setting_value(&cache_key)?;
        if !claim_codex_rate_limits_fetch(&cache_key) {
            return Ok(cached);
        }
        match crate::rate_limits::codex::fetch_codex_rate_limits_for_home(&codex_home) {
            Ok(body) => {
                settings::upsert_setting_value(&cache_key, &body)?;
                Ok(Some(body))
            }
            Err(error) => {
                tracing::warn!("Failed to refresh Codex rate limits: {error}");
                Ok(cached)
            }
        }
    })
    .await
}

fn codex_home_for_executable_path(codex_executable_path: Option<&str>) -> std::path::PathBuf {
    codex_executable_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .and_then(|path| crate::codex_config::codex_home_from_executable(Path::new(path)))
        .unwrap_or_else(crate::codex_config::default_codex_home)
}

fn codex_rate_limits_cache_key(codex_home: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(codex_home.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    format!(
        "{}.{}",
        settings::CODEX_RATE_LIMITS_KEY,
        hex::encode(&digest[..8])
    )
}

fn claim_codex_rate_limits_fetch(cache_key: &str) -> bool {
    let now = Utc::now().timestamp();
    let mut attempts = CODEX_RATE_LIMITS_THROTTLES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let last = attempts.get(cache_key).copied().unwrap_or_default();
    if now.saturating_sub(last) < RATE_LIMITS_THROTTLE_SECONDS {
        return false;
    }
    attempts.insert(cache_key.to_string(), now);
    true
}

fn codex_uses_metered_api_key(codex_home: &std::path::Path) -> bool {
    if crate::rate_limits::codex::credential_kind_for_home(codex_home)
        == Some(crate::rate_limits::codex::CodexCredentialKind::ApiKey)
    {
        return true;
    }
    let config_path = crate::codex_config::config_path_for_home(codex_home);
    let Ok(config) = std::fs::read_to_string(config_path) else {
        return false;
    };
    crate::codex_config::active_api_key_provider(&config).is_some()
}

/// Read the account-global Claude rate-limit snapshot. Each call
/// attempts a live fetch and falls back to the cached body on failure.
/// `app.claude_rate_limits` stores the raw Anthropic response — no
/// shape mapping — so downstream parsing lives entirely in the frontend.
///
/// See `get_codex_rate_limits` for why this command does not publish a
/// `*RateLimitsChanged` UI-sync event.
#[tauri::command]
pub async fn get_claude_rate_limits() -> CmdResult<Option<String>> {
    run_blocking(|| {
        let cached = settings::load_setting_value(settings::CLAUDE_RATE_LIMITS_KEY)?;
        if !CLAUDE_RATE_LIMITS_THROTTLE.should_fetch() {
            return Ok(cached);
        }
        CLAUDE_RATE_LIMITS_THROTTLE.record_attempt();
        match crate::rate_limits::claude::fetch_claude_rate_limits() {
            Ok(body) => {
                settings::upsert_setting_value(settings::CLAUDE_RATE_LIMITS_KEY, &body)?;
                Ok(Some(body))
            }
            Err(error) => {
                tracing::warn!("Failed to refresh Claude rate limits: {error}");
                Ok(cached)
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn load_auto_close_action_kinds() -> CmdResult<Vec<ActionKind>> {
    run_blocking(settings::load_auto_close_action_kinds).await
}

#[tauri::command]
pub async fn save_auto_close_action_kinds(kinds: Vec<ActionKind>) -> CmdResult<()> {
    run_blocking(move || settings::save_auto_close_action_kinds(&kinds)).await
}

#[tauri::command]
pub async fn load_auto_close_opt_in_asked() -> CmdResult<Vec<ActionKind>> {
    run_blocking(settings::load_auto_close_opt_in_asked).await
}

#[tauri::command]
pub async fn save_auto_close_opt_in_asked(kinds: Vec<ActionKind>) -> CmdResult<()> {
    run_blocking(move || settings::save_auto_close_opt_in_asked(&kinds)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_rate_limit_cache_keys_are_scoped_by_home() {
        let first = codex_rate_limits_cache_key(Path::new("/tmp/codex-a"));
        let second = codex_rate_limits_cache_key(Path::new("/tmp/codex-b"));

        assert!(first.starts_with(settings::CODEX_RATE_LIMITS_KEY));
        assert!(second.starts_with(settings::CODEX_RATE_LIMITS_KEY));
        assert_ne!(first, second);
    }

    #[test]
    fn codex_home_for_empty_executable_uses_default_home() {
        assert_eq!(
            codex_home_for_executable_path(Some("")),
            crate::codex_config::default_codex_home()
        );
        assert_eq!(
            codex_home_for_executable_path(None),
            crate::codex_config::default_codex_home()
        );
    }

    #[test]
    fn codex_metered_usage_detects_auth_json_api_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("auth.json"),
            br#"{ "OPENAI_API_KEY": "sk-test" }"#,
        )
        .unwrap();

        assert!(codex_uses_metered_api_key(dir.path()));
    }

    #[test]
    fn codex_metered_usage_ignores_chatgpt_login_tokens() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("auth.json"),
            br#"{ "tokens": { "access_token": "access" } }"#,
        )
        .unwrap();

        assert!(!codex_uses_metered_api_key(dir.path()));
    }
}
