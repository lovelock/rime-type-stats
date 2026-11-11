//! 跨平台路径解析：Rime 统计库目录与本地数据目录。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 用户主目录；`HOME` 缺失或为空时退回当前目录。
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 读取路径类环境变量；空值视为未设置。
fn env_path(env: &dyn Fn(&str) -> Option<OsString>, key: &str) -> Option<PathBuf> {
    env(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// XDG 数据目录：`$XDG_DATA_HOME`，未设置或为空时用 `~/.local/share`。
fn xdg_data_home(env: &dyn Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf {
    env_path(env, "XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"))
}

/// 当前平台的 Rime 统计库目录（`stats.userdb` 本身）。
pub fn rime_dir() -> PathBuf {
    let env = |key: &str| std::env::var_os(key);
    let home = home_dir();
    #[cfg(target_os = "macos")]
    {
        macos_rime_dir(&env, &home)
    }
    #[cfg(not(target_os = "macos"))]
    {
        linux_rime_dir(&env, &home, &|path: &Path| path.is_dir())
    }
}

/// 当前平台的本地数据目录。
pub fn data_dir() -> PathBuf {
    let env = |key: &str| std::env::var_os(key);
    let home = home_dir();
    #[cfg(target_os = "macos")]
    {
        macos_data_dir(&env, &home)
    }
    #[cfg(not(target_os = "macos"))]
    {
        linux_data_dir(&env, &home)
    }
}

/// Linux：按 `RIME_TYPE_STATS_RIME_DIR` → fcitx5 → ibus → fcitx4 → fcitx5（配置目录）探测。
pub fn linux_rime_dir(
    env: &dyn Fn(&str) -> Option<OsString>,
    home: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(dir) = env_path(env, "RIME_TYPE_STATS_RIME_DIR") {
        return dir;
    }

    let candidates = [
        xdg_data_home(env, home).join("fcitx5/rime/stats.userdb"),
        home.join(".config/ibus/rime/stats.userdb"),
        home.join(".config/fcitx/rime/stats.userdb"),
        home.join(".config/fcitx5/rime/stats.userdb"),
    ];

    candidates
        .iter()
        .find(|path| exists(path))
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

/// Linux：`RIME_TYPE_STATS_DATA_DIR` → `$XDG_DATA_HOME/rime-type-stats` → `~/.local/share/rime-type-stats`。
pub fn linux_data_dir(env: &dyn Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf {
    env_path(env, "RIME_TYPE_STATS_DATA_DIR")
        .unwrap_or_else(|| xdg_data_home(env, home).join("rime-type-stats"))
}

/// macOS：`RIME_TYPE_STATS_RIME_DIR` → `~/Library/Rime/stats.userdb`。
pub fn macos_rime_dir(env: &dyn Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf {
    env_path(env, "RIME_TYPE_STATS_RIME_DIR")
        .unwrap_or_else(|| home.join("Library/Rime/stats.userdb"))
}

/// macOS：`RIME_TYPE_STATS_DATA_DIR` → `~/Library/Application Support/rime-type-stats`。
pub fn macos_data_dir(env: &dyn Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf {
    env_path(env, "RIME_TYPE_STATS_DATA_DIR")
        .unwrap_or_else(|| home.join("Library/Application Support/rime-type-stats"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(v)))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn exists_in(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p| set.iter().any(|candidate| candidate == p)
    }

    #[test]
    fn linux_data_dir_prefers_env_override() {
        let env = env_of(&[
            ("RIME_TYPE_STATS_DATA_DIR", "/custom/data"),
            ("XDG_DATA_HOME", "/xdg"),
        ]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/custom/data")
        );
    }

    #[test]
    fn linux_data_dir_uses_xdg_data_home() {
        let env = env_of(&[("XDG_DATA_HOME", "/xdg")]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/xdg/rime-type-stats")
        );
    }

    #[test]
    fn linux_data_dir_falls_back_to_local_share() {
        let env = env_of(&[]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/home/u/.local/share/rime-type-stats")
        );
    }

    #[test]
    fn empty_env_values_are_ignored() {
        let env = env_of(&[
            ("RIME_TYPE_STATS_DATA_DIR", ""),
            ("XDG_DATA_HOME", ""),
            ("RIME_TYPE_STATS_RIME_DIR", ""),
        ]);
        assert_eq!(
            linux_data_dir(&env, Path::new("/home/u")),
            PathBuf::from("/home/u/.local/share/rime-type-stats")
        );
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists_in(&[])),
            PathBuf::from("/home/u/.local/share/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_prefers_env_override() {
        let env = env_of(&[("RIME_TYPE_STATS_RIME_DIR", "/custom/stats.userdb")]);
        let exists = exists_in(&["/home/u/.local/share/fcitx5/rime/stats.userdb"]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/custom/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_picks_first_existing_candidate() {
        let env = env_of(&[]);
        let exists = exists_in(&["/home/u/.config/ibus/rime/stats.userdb"]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/home/u/.config/ibus/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_prefers_fcitx5_when_multiple_exist() {
        let env = env_of(&[]);
        let exists = exists_in(&[
            "/home/u/.local/share/fcitx5/rime/stats.userdb",
            "/home/u/.config/ibus/rime/stats.userdb",
        ]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/home/u/.local/share/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_respects_xdg_data_home() {
        let env = env_of(&[("XDG_DATA_HOME", "/xdg")]);
        let exists = exists_in(&["/xdg/fcitx5/rime/stats.userdb"]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists),
            PathBuf::from("/xdg/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn linux_rime_dir_defaults_to_fcitx5_when_none_exist() {
        let env = env_of(&[]);
        assert_eq!(
            linux_rime_dir(&env, Path::new("/home/u"), &exists_in(&[])),
            PathBuf::from("/home/u/.local/share/fcitx5/rime/stats.userdb")
        );
    }

    #[test]
    fn macos_paths_use_library_dirs() {
        let env = env_of(&[]);
        assert_eq!(
            macos_rime_dir(&env, Path::new("/Users/u")),
            PathBuf::from("/Users/u/Library/Rime/stats.userdb")
        );
        assert_eq!(
            macos_data_dir(&env, Path::new("/Users/u")),
            PathBuf::from("/Users/u/Library/Application Support/rime-type-stats")
        );
    }
}
