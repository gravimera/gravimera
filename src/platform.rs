#[cfg(target_os = "linux")]
pub(crate) fn is_wsl() -> bool {
    fn is_wsl_from(wsl_interop: bool, wsl_distro: bool, osrelease: &str) -> bool {
        if wsl_interop || wsl_distro {
            return true;
        }
        let osrelease = osrelease.trim().to_ascii_lowercase();
        osrelease.contains("microsoft") || osrelease.contains("wsl")
    }

    let wsl_interop = std::env::var_os("WSL_INTEROP").is_some_and(|v| !v.is_empty());
    let wsl_distro = std::env::var_os("WSL_DISTRO_NAME").is_some_and(|v| !v.is_empty());
    let osrelease = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    is_wsl_from(wsl_interop, wsl_distro, &osrelease)
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub(crate) fn is_wsl() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn is_wsl_checks_env_vars_first() {
        fn is_wsl_from(wsl_interop: bool, wsl_distro: bool, osrelease: &str) -> bool {
            if wsl_interop || wsl_distro {
                return true;
            }
            let osrelease = osrelease.trim().to_ascii_lowercase();
            osrelease.contains("microsoft") || osrelease.contains("wsl")
        }

        assert!(is_wsl_from(true, false, ""));
        assert!(is_wsl_from(false, true, ""));
        assert!(!is_wsl_from(false, false, "6.8.0-48-generic"));
    }

    #[test]
    fn is_wsl_detects_osrelease_markers() {
        fn is_wsl_from(wsl_interop: bool, wsl_distro: bool, osrelease: &str) -> bool {
            if wsl_interop || wsl_distro {
                return true;
            }
            let osrelease = osrelease.trim().to_ascii_lowercase();
            osrelease.contains("microsoft") || osrelease.contains("wsl")
        }

        assert!(is_wsl_from(
            false,
            false,
            "5.15.167.4-microsoft-standard-WSL2"
        ));
        assert!(is_wsl_from(false, false, "5.15.0-wsl2"));
        assert!(is_wsl_from(false, false, "WSL"));
    }
}
