//! Slash-command cache.
//!
//! Two-tier cache:
//! 1. **Workspace tier** — keyed by `(provider, cwd, linked-dir-signature)`.
//!    Primary cache — an exact hit returns instantly and we revalidate in the
//!    background.
//! 2. **Repo tier** — keyed by `(provider, repo_id)`. Fallback used when the
//!    workspace tier misses. Different workspaces on the same repo usually
//!    share the same `~/.claude/skills/` and `.claude/commands/`, so we can
//!    show stale-but-plausible commands while the real scan runs.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use super::queries::SlashCommandEntry;

pub type WorkspaceKey = (String, String, String); // (provider, cwd, linked-dir-signature)
pub type RepoKey = (String, String); // (provider, repo_id)

pub fn workspace_key(
    provider: &str,
    working_directory: Option<&str>,
    additional_directories: &[String],
) -> WorkspaceKey {
    (
        provider.to_string(),
        working_directory.unwrap_or_default().to_string(),
        additional_directories.join("\u{1f}"),
    )
}

pub fn repo_key(provider: &str, repo_id: &str) -> RepoKey {
    (provider.to_string(), repo_id.to_string())
}

pub struct SlashCommandCache {
    workspaces: RwLock<HashMap<WorkspaceKey, Vec<SlashCommandEntry>>>,
    repos: RwLock<HashMap<RepoKey, Vec<SlashCommandEntry>>>,
    refresh: Mutex<RefreshState>,
}

#[derive(Default)]
struct RefreshState {
    /// Prevents duplicate background refreshes for the same workspace key.
    in_flight: HashMap<WorkspaceKey, u64>,
    /// Bumped when a provider's external command context changes, so stale
    /// in-flight refreshes cannot repopulate caches after a clear.
    generations: HashMap<String, u64>,
}

impl RefreshState {
    fn generation(&self, provider: &str) -> u64 {
        self.generations.get(provider).copied().unwrap_or_default()
    }
}

impl Default for SlashCommandCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SlashCommandCache {
    pub fn new() -> Self {
        Self {
            workspaces: RwLock::new(HashMap::new()),
            repos: RwLock::new(HashMap::new()),
            refresh: Mutex::new(RefreshState::default()),
        }
    }

    /// Exact workspace-level lookup.
    pub fn get_workspace(&self, key: &WorkspaceKey) -> Option<Vec<SlashCommandEntry>> {
        let map = self.workspaces.read().ok()?;
        let commands = map.get(key)?.clone();
        tracing::debug!(
            provider = %key.0,
            cwd = %key.1,
            linked_dir_count = key.2.split('\u{1f}').filter(|s| !s.is_empty()).count(),
            count = commands.len(),
            "Slash-command workspace cache hit"
        );
        Some(commands)
    }

    /// Repo-level fallback lookup — used only when the workspace tier misses.
    pub fn get_repo(&self, key: &RepoKey) -> Option<Vec<SlashCommandEntry>> {
        let map = self.repos.read().ok()?;
        let commands = map.get(key)?.clone();
        tracing::debug!(
            provider = %key.0,
            repo_id = %key.1,
            count = commands.len(),
            "Slash-command repo-level fallback hit"
        );
        Some(commands)
    }

    /// Write a result into the workspace tier, and also mirror it to the repo
    /// tier (latest-wins) so future workspaces on the same repo get a good
    /// fallback on first access.
    pub fn set(
        &self,
        workspace_key: WorkspaceKey,
        repo_id: Option<&str>,
        commands: Vec<SlashCommandEntry>,
    ) {
        let generation = self.provider_generation(&workspace_key.0);
        let _ = self.set_if_generation(workspace_key, repo_id, commands, generation);
    }

    pub fn set_if_generation(
        &self,
        workspace_key: WorkspaceKey,
        repo_id: Option<&str>,
        commands: Vec<SlashCommandEntry>,
        generation: u64,
    ) -> bool {
        let Ok(refresh) = self.refresh.lock() else {
            return false;
        };
        if refresh.generation(&workspace_key.0) != generation {
            return false;
        }
        if let Ok(mut map) = self.workspaces.write() {
            map.insert(workspace_key.clone(), commands.clone());
        }
        if let Some(repo_id) = repo_id.filter(|id| !id.is_empty()) {
            let rkey = repo_key(&workspace_key.0, repo_id);
            if let Ok(mut map) = self.repos.write() {
                map.insert(rkey, commands);
            }
        }
        true
    }

    pub fn provider_generation(&self, provider: &str) -> u64 {
        self.refresh
            .lock()
            .map(|refresh| refresh.generation(provider))
            .unwrap_or_default()
    }

    /// Try to claim the refresh lock for a workspace key. Returns `true` if
    /// this caller won.
    pub fn try_start_refresh(&self, key: &WorkspaceKey) -> bool {
        self.try_start_refresh_with_generation(key).is_some()
    }

    pub fn try_start_refresh_with_generation(&self, key: &WorkspaceKey) -> Option<u64> {
        let Ok(mut refresh) = self.refresh.lock() else {
            return None;
        };
        if refresh.in_flight.contains_key(key) {
            return None;
        }
        let generation = refresh.generation(&key.0);
        refresh.in_flight.insert(key.clone(), generation);
        Some(generation)
    }

    pub fn finish_refresh(&self, key: &WorkspaceKey) {
        if let Ok(mut refresh) = self.refresh.lock() {
            refresh.in_flight.remove(key);
        }
    }

    pub fn finish_refresh_generation(&self, key: &WorkspaceKey, generation: u64) {
        if let Ok(mut refresh) = self.refresh.lock() {
            if refresh.in_flight.get(key).copied() == Some(generation) {
                refresh.in_flight.remove(key);
            }
        }
    }

    pub fn clear_provider(&self, provider: &str) {
        if let Ok(mut refresh) = self.refresh.lock() {
            let generation = refresh.generations.entry(provider.to_string()).or_default();
            *generation = generation.wrapping_add(1);
            refresh.in_flight.retain(|key, _| key.0 != provider);
        }
        if let Ok(mut map) = self.workspaces.write() {
            map.retain(|key, _| key.0 != provider);
        }
        if let Ok(mut map) = self.repos.write() {
            map.retain(|key, _| key.0 != provider);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> SlashCommandEntry {
        SlashCommandEntry {
            name: name.to_string(),
            description: format!("desc for {name}"),
            argument_hint: None,
            source: "skill".to_string(),
        }
    }

    #[test]
    fn workspace_key_distinguishes_provider() {
        let claude = workspace_key("claude", Some("/repo"), &["/linked".to_string()]);
        let codex = workspace_key("codex", Some("/repo"), &["/linked".to_string()]);
        assert_ne!(claude, codex);
    }

    #[test]
    fn workspace_key_distinguishes_linked_dir_signature() {
        let none = workspace_key("claude", Some("/repo"), &[]);
        let one = workspace_key("claude", Some("/repo"), &["/linked".to_string()]);
        let two = workspace_key(
            "claude",
            Some("/repo"),
            &["/linked".to_string(), "/other".to_string()],
        );
        assert_ne!(none, one);
        assert_ne!(one, two);
    }

    #[test]
    fn workspace_key_treats_none_cwd_as_empty_string() {
        let none = workspace_key("claude", None, &[]);
        let empty = workspace_key("claude", Some(""), &[]);
        assert_eq!(none, empty);
    }

    #[test]
    fn cache_starts_empty_and_returns_none() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("claude", Some("/repo"), &[]);
        assert!(cache.get_workspace(&key).is_none());
        assert!(cache
            .get_repo(&("claude".into(), "repo-1".into()))
            .is_none());
    }

    #[test]
    fn set_writes_workspace_and_mirrors_repo_when_repo_id_present() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("claude", Some("/repo"), &[]);
        cache.set(key.clone(), Some("repo-1"), vec![entry("a"), entry("b")]);

        assert_eq!(cache.get_workspace(&key).unwrap().len(), 2);
        let repo_hit = cache.get_repo(&repo_key("claude", "repo-1")).unwrap();
        assert_eq!(repo_hit.len(), 2);
        assert_eq!(repo_hit[0].name, "a");
    }

    #[test]
    fn set_skips_repo_mirror_when_repo_id_is_empty_or_missing() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("claude", Some("/repo"), &[]);
        cache.set(key.clone(), None, vec![entry("a")]);
        cache.set(key.clone(), Some(""), vec![entry("a")]);

        // Workspace tier was written.
        assert!(cache.get_workspace(&key).is_some());
        // Repo tier was not — the empty repo_id falls in the same bucket as None.
        assert!(cache.get_repo(&repo_key("claude", "")).is_none());
    }

    #[test]
    fn try_start_refresh_is_exclusive_until_finish() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("claude", Some("/repo"), &[]);

        assert!(cache.try_start_refresh(&key), "first call wins the lock");
        assert!(!cache.try_start_refresh(&key), "second call loses");

        cache.finish_refresh(&key);
        assert!(
            cache.try_start_refresh(&key),
            "after finish, the lock can be reclaimed"
        );
    }

    #[test]
    fn try_start_refresh_independent_keys_dont_block_each_other() {
        let cache = SlashCommandCache::new();
        let a = workspace_key("claude", Some("/a"), &[]);
        let b = workspace_key("claude", Some("/b"), &[]);
        assert!(cache.try_start_refresh(&a));
        assert!(cache.try_start_refresh(&b));
    }

    #[test]
    fn finish_refresh_on_unheld_key_is_a_noop() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("claude", Some("/repo"), &[]);
        cache.finish_refresh(&key);
        // Subsequent claim still works.
        assert!(cache.try_start_refresh(&key));
    }

    #[test]
    fn clear_provider_removes_only_matching_provider_entries() {
        let cache = SlashCommandCache::new();
        let codex_key = workspace_key("codex", Some("/repo"), &[]);
        let claude_key = workspace_key("claude", Some("/repo"), &[]);
        cache.set(codex_key.clone(), Some("repo-1"), vec![entry("codex")]);
        cache.set(claude_key.clone(), Some("repo-1"), vec![entry("claude")]);
        assert!(cache.try_start_refresh(&codex_key));
        assert!(cache.try_start_refresh(&claude_key));

        cache.clear_provider("codex");

        assert!(cache.get_workspace(&codex_key).is_none());
        assert!(cache.get_repo(&repo_key("codex", "repo-1")).is_none());
        assert!(cache.try_start_refresh(&codex_key));
        assert!(cache.get_workspace(&claude_key).is_some());
        assert!(cache.get_repo(&repo_key("claude", "repo-1")).is_some());
        assert!(!cache.try_start_refresh(&claude_key));
    }

    #[test]
    fn set_if_generation_rejects_stale_provider_writes_after_clear() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("codex", Some("/repo"), &[]);
        let generation = cache.provider_generation("codex");

        cache.clear_provider("codex");

        assert!(!cache.set_if_generation(
            key.clone(),
            Some("repo-1"),
            vec![entry("stale")],
            generation,
        ));
        assert!(cache.get_workspace(&key).is_none());
        assert!(cache.get_repo(&repo_key("codex", "repo-1")).is_none());
    }

    #[test]
    fn stale_finish_does_not_clear_new_refresh_marker() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("codex", Some("/repo"), &[]);
        let old_generation = cache.try_start_refresh_with_generation(&key).unwrap();

        cache.clear_provider("codex");
        let new_generation = cache.try_start_refresh_with_generation(&key).unwrap();
        cache.finish_refresh_generation(&key, old_generation);

        assert!(!cache.try_start_refresh(&key));
        cache.finish_refresh_generation(&key, new_generation);
        assert!(cache.try_start_refresh(&key));
    }

    #[test]
    fn cache_set_overwrites_previous_value_for_same_key() {
        let cache = SlashCommandCache::new();
        let key = workspace_key("claude", Some("/repo"), &[]);
        cache.set(key.clone(), Some("repo-1"), vec![entry("old")]);
        cache.set(
            key.clone(),
            Some("repo-1"),
            vec![entry("new"), entry("two")],
        );

        let entries = cache.get_workspace(&key).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "new");
    }

    #[test]
    fn cache_concurrent_get_and_set_does_not_panic() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(SlashCommandCache::new());
        let key = workspace_key("claude", Some("/repo"), &[]);
        cache.set(key.clone(), Some("repo-1"), vec![entry("init")]);

        let mut handles = Vec::new();
        for i in 0..8 {
            let cache = cache.clone();
            let key = key.clone();
            handles.push(thread::spawn(move || {
                if i % 2 == 0 {
                    let _ = cache.get_workspace(&key);
                } else {
                    cache.set(key.clone(), Some("repo-1"), vec![entry(&format!("e{i}"))]);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(cache.get_workspace(&key).is_some());
    }
}

// ---------------------------------------------------------------------------
// Local skill/command scanner
