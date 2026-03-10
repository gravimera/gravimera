use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const GRAVIMERA_HOME_ENV: &str = "GRAVIMERA_HOME";
const REALMS_DIR_NAME: &str = "realm";
const DEFAULT_REALM_ID: &str = "default";
const DEFAULT_SCENE_ID: &str = "default";

pub(crate) fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }

    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(profile));
    }

    let drive = std::env::var_os("HOMEDRIVE").filter(|v| !v.is_empty());
    let path = std::env::var_os("HOMEPATH").filter(|v| !v.is_empty());
    match (drive, path) {
        (Some(drive), Some(path)) => {
            let combined = format!("{}{}", drive.to_string_lossy(), path.to_string_lossy());
            (!combined.trim().is_empty()).then_some(PathBuf::from(combined))
        }
        _ => None,
    }
}

pub(crate) fn gravimera_dir() -> PathBuf {
    if let Some(override_dir) = std::env::var_os(GRAVIMERA_HOME_ENV).and_then(|v| path_from_env(&v))
    {
        return override_dir;
    }

    home_dir()
        .map(|home| home.join(".gravimera"))
        .unwrap_or_else(|| PathBuf::from(".gravimera"))
}

pub(crate) fn default_config_path() -> PathBuf {
    gravimera_dir().join("config.toml")
}

pub(crate) fn legacy_scene_dat_path() -> PathBuf {
    gravimera_dir().join("scene.dat")
}

pub(crate) fn default_cache_dir() -> PathBuf {
    gravimera_dir().join("cache")
}

pub(crate) fn default_gen3d_cache_dir() -> PathBuf {
    default_cache_dir().join("gen3d")
}

pub(crate) fn ensure_default_dirs() -> std::io::Result<()> {
    let base = gravimera_dir();
    std::fs::create_dir_all(&base)?;
    std::fs::create_dir_all(realms_dir())?;
    std::fs::create_dir_all(default_cache_dir())?;
    std::fs::create_dir_all(default_gen3d_cache_dir())?;
    Ok(())
}

pub(crate) fn realms_dir() -> PathBuf {
    gravimera_dir().join(REALMS_DIR_NAME)
}

pub(crate) fn default_realm_id() -> &'static str {
    DEFAULT_REALM_ID
}

pub(crate) fn default_scene_id() -> &'static str {
    DEFAULT_SCENE_ID
}

pub(crate) fn realm_dir(realm_id: &str) -> PathBuf {
    realms_dir().join(realm_id)
}

pub(crate) fn realm_prefabs_dir(realm_id: &str) -> PathBuf {
    realm_dir(realm_id).join("prefabs")
}

pub(crate) fn realm_prefab_package_dir(realm_id: &str, root_prefab_id: u128) -> PathBuf {
    let uuid = uuid::Uuid::from_u128(root_prefab_id).to_string();
    realm_prefabs_dir(realm_id).join(uuid)
}

pub(crate) fn scene_dir(realm_id: &str, scene_id: &str) -> PathBuf {
    realm_dir(realm_id).join("scenes").join(scene_id)
}

pub(crate) fn scene_prefabs_dir(realm_id: &str, scene_id: &str) -> PathBuf {
    scene_dir(realm_id, scene_id).join("prefabs")
}

pub(crate) fn scene_prefab_package_dir(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) -> PathBuf {
    let uuid = uuid::Uuid::from_u128(root_prefab_id).to_string();
    scene_prefabs_dir(realm_id, scene_id).join(uuid)
}

pub(crate) fn scene_src_dir(realm_id: &str, scene_id: &str) -> PathBuf {
    scene_dir(realm_id, scene_id).join("src")
}

pub(crate) fn scene_build_dir(realm_id: &str, scene_id: &str) -> PathBuf {
    scene_dir(realm_id, scene_id).join("build")
}

pub(crate) fn scene_dat_path(realm_id: &str, scene_id: &str) -> PathBuf {
    scene_build_dir(realm_id, scene_id).join("scene.dat")
}

pub(crate) fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
}

pub(crate) fn legacy_path_next_to_exe(file_name: &str) -> Option<PathBuf> {
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return None;
    }
    exe_dir().map(|dir| dir.join(file_name))
}

pub(crate) fn looks_like_macos_app_bundle_exe_dir(exe_dir: &Path) -> bool {
    let macos = exe_dir.file_name() == Some(OsStr::new("MacOS"));
    let contents = exe_dir
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|name| name == OsStr::new("Contents"));
    macos && contents
}

pub(crate) fn resolve_assets_dir() -> PathBuf {
    // macOS app bundles store assets under `MyApp.app/Contents/Resources/assets`.
    if let Some(exe_dir) = exe_dir() {
        if looks_like_macos_app_bundle_exe_dir(&exe_dir) {
            if let Some(contents_dir) = exe_dir.parent() {
                let candidate = contents_dir.join("Resources").join("assets");
                if candidate.is_dir() {
                    return candidate;
                }
            }
        }

        let candidate = exe_dir.join("assets");
        if candidate.is_dir() {
            return candidate;
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("assets");
        if candidate.is_dir() {
            return candidate;
        }
    }

    PathBuf::from("assets")
}

fn path_from_env(value: &OsStr) -> Option<PathBuf> {
    let raw = value.to_string_lossy();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(expand_tilde_path(trimmed))
}

pub(crate) fn expand_tilde_path(value: &str) -> PathBuf {
    let value = value.trim();
    if value == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}
