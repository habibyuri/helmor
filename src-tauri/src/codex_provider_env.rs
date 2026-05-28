use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

static CODEX_PROVIDER_ENV_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn inherit_keys(env_map: &HashMap<String, String>) -> usize {
    let config_paths = codex_provider_config_paths();

    let mut total_count = 0;
    for config_path in config_paths {
        let Ok(config) = std::fs::read_to_string(&config_path) else {
            // No config file is the common case for users who only use
            // Claude — silently no-op rather than warning.
            continue;
        };

        let (count, declared) = apply_env_keys_from_config(&config, env_map);
        total_count += count;

        tracing::debug!(
            config = %config_path.display(),
            declared,
            inherited = count,
            "Considered Codex provider env_keys for inheritance"
        );
    }

    total_count
}

pub fn codex_provider_env_for_selected_home() -> HashMap<String, String> {
    let config_paths = codex_provider_config_paths();
    let mut values = collect_codex_provider_env(&config_paths, None);
    merge_cached_codex_provider_env(&config_paths, &mut values);
    values
}

fn codex_provider_config_paths() -> Vec<std::path::PathBuf> {
    let mut config_paths = vec![crate::codex_config::config_path()];
    let selected_path =
        crate::codex_config::config_path_for_home(&crate::codex_config::selected_codex_home());
    if !config_paths.contains(&selected_path) {
        config_paths.push(selected_path);
    }
    config_paths
}

fn collect_codex_provider_env(
    config_paths: &[std::path::PathBuf],
    shell_env: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for config_path in config_paths {
        let Ok(config) = std::fs::read_to_string(config_path) else {
            continue;
        };
        for key in crate::codex_config::declared_env_keys(&config) {
            if let Some(value) = current_non_empty_env(&key) {
                values.insert(key, value);
                continue;
            }
            let Some(shell_value) = shell_env
                .and_then(|env| env.get(&key))
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            values.insert(key, shell_value.clone());
        }
    }
    values
}

fn merge_cached_codex_provider_env(
    config_paths: &[std::path::PathBuf],
    values: &mut HashMap<String, String>,
) {
    let cached = CODEX_PROVIDER_ENV_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    merge_codex_provider_env_from_cache(config_paths, values, &cached);
}

fn merge_codex_provider_env_from_cache(
    config_paths: &[std::path::PathBuf],
    values: &mut HashMap<String, String>,
    cached: &HashMap<String, String>,
) {
    let declared_keys = declared_codex_provider_env_keys(config_paths);
    if declared_keys.is_empty() {
        return;
    }
    for (key, value) in cached.iter() {
        if declared_keys.contains(key) && !value.trim().is_empty() {
            values.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn declared_codex_provider_env_keys(config_paths: &[std::path::PathBuf]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for config_path in config_paths {
        let Ok(config) = std::fs::read_to_string(config_path) else {
            continue;
        };
        keys.extend(crate::codex_config::declared_env_keys(&config));
    }
    keys
}

fn current_non_empty_env(key: &str) -> Option<String> {
    std::env::var_os(key)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
}

fn apply_env_keys_from_config(config: &str, env_map: &HashMap<String, String>) -> (usize, usize) {
    let keys = crate::codex_config::declared_env_keys(config);
    let declared = keys.len();
    if declared == 0 {
        return (0, 0);
    }

    let mut count = 0;
    for key in &keys {
        let Some(value) = env_map.get(key) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let before = std::env::var_os(key).is_some_and(|v| !v.is_empty());
        merge_missing_env(key, value);
        let after = std::env::var_os(key).is_some_and(|v| !v.is_empty());
        if !before && after {
            count += 1;
        }
    }
    (count, declared)
}

fn merge_missing_env(var: &str, value: &str) {
    let already_set = std::env::var_os(var).is_some_and(|current| !current.is_empty());
    if already_set || value.is_empty() {
        return;
    }
    // SAFETY: called from startup setup before worker threads are spawned.
    unsafe { std::env::set_var(var, value) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_codex_env_keys_injects_declared_var_from_login_shell() {
        let key = "HELMOR_TEST_CODEX_AZURE_KEY_1";
        unsafe { std::env::remove_var(key) };

        let config = format!(
            r#"
model_provider = "azure"

[model_providers.azure]
env_key = "{key}"
"#
        );
        let mut env_map = HashMap::new();
        env_map.insert(key.to_string(), "abc123".to_string());

        let (count, declared) = apply_env_keys_from_config(&config, &env_map);
        assert_eq!(declared, 1);
        assert_eq!(count, 1);
        assert_eq!(std::env::var(key).as_deref(), Ok("abc123"));

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn apply_codex_env_keys_does_not_clobber_preexisting_value() {
        let key = "HELMOR_TEST_CODEX_AZURE_KEY_2";
        unsafe { std::env::set_var(key, "preexisting") };

        let config = format!(
            r#"
[model_providers.azure]
env_key = "{key}"
"#
        );
        let mut env_map = HashMap::new();
        env_map.insert(key.to_string(), "from-login-shell".to_string());

        let (count, declared) = apply_env_keys_from_config(&config, &env_map);
        assert_eq!(declared, 1);
        assert_eq!(count, 0, "should not count as newly set");
        assert_eq!(std::env::var(key).as_deref(), Ok("preexisting"));

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn apply_codex_env_keys_skips_when_shell_has_no_value() {
        let key = "HELMOR_TEST_CODEX_AZURE_KEY_3";
        unsafe { std::env::remove_var(key) };

        let config = format!(
            r#"
[model_providers.azure]
env_key = "{key}"
"#
        );
        let env_map = HashMap::new();

        let (count, declared) = apply_env_keys_from_config(&config, &env_map);
        assert_eq!(declared, 1);
        assert_eq!(count, 0);
        assert!(std::env::var_os(key).is_none());
    }

    #[test]
    fn collect_codex_provider_env_returns_values_to_hot_push() {
        let key = "HELMOR_TEST_CODEX_AZURE_KEY_4";
        unsafe { std::env::remove_var(key) };
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                r#"
[model_providers.azure]
env_key = "{key}"
"#
            ),
        )
        .unwrap();
        let mut env_map = HashMap::new();
        env_map.insert(key.to_string(), "from-login-shell".to_string());

        let values = collect_codex_provider_env(&[config_path], Some(&env_map));

        assert_eq!(
            values.get(key).map(String::as_str),
            Some("from-login-shell")
        );
        assert!(std::env::var_os(key).is_none());

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn cached_codex_provider_env_is_available_without_global_env_mutation() {
        let key = "HELMOR_TEST_CODEX_AZURE_KEY_5";
        unsafe { std::env::remove_var(key) };
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                r#"
[model_providers.azure]
env_key = "{key}"
"#
            ),
        )
        .unwrap();

        let mut cached = HashMap::new();
        cached.insert(key.to_string(), "from-login-shell".to_string());

        let mut values = collect_codex_provider_env(std::slice::from_ref(&config_path), None);
        merge_codex_provider_env_from_cache(&[config_path], &mut values, &cached);

        assert_eq!(
            values.get(key).map(String::as_str),
            Some("from-login-shell")
        );
        assert!(std::env::var_os(key).is_none());

        unsafe { std::env::remove_var(key) };
    }
}
