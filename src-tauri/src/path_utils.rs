use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub fn find_binary_on_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .flat_map(|dir| binary_candidates(&dir, binary))
        .find(|candidate| is_executable_file(candidate))
}

pub fn resolve_path_command(path: &Path, path_env: Option<&OsStr>) -> PathBuf {
    if !is_path_command(path) {
        return path.to_path_buf();
    }
    let Some(path_env) = path_env else {
        return path.to_path_buf();
    };
    for dir in std::env::split_paths(path_env) {
        let candidate = dir.join(path);
        if candidate.is_file() {
            return candidate;
        }
    }
    path.to_path_buf()
}

fn is_path_command(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .parent()
            .map(|parent| parent.as_os_str().is_empty())
            .unwrap_or(true)
}

fn binary_candidates(dir: &Path, binary: &str) -> Vec<PathBuf> {
    let base = dir.join(binary);
    #[cfg(windows)]
    {
        if Path::new(binary).extension().is_some() {
            return vec![base];
        }
        let pathext = std::env::var_os("PATHEXT")
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut candidates = vec![base.clone()];
        candidates.extend(
            pathext
                .split(';')
                .filter(|ext| !ext.trim().is_empty())
                .map(|ext| dir.join(format!("{binary}{ext}"))),
        );
        candidates
    }
    #[cfg(not(windows))]
    {
        vec![base]
    }
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}
