pub(super) struct CurrentDirGuard {
    previous: std::path::PathBuf,
}

pub(super) struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl CurrentDirGuard {
    pub(super) fn enter(path: &std::path::Path) -> Self {
        let previous = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(path).expect("set current dir");
        Self { previous }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.previous).expect("restore current dir");
    }
}

impl EnvVarGuard {
    pub(super) fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: runtime-extension tests are serialized with `serial_test`;
        // the guard restores the process environment before the next test runs.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: paired restoration for `EnvVarGuard::set_path`; serialized
        // tests prevent concurrent environment mutation inside this module.
        unsafe {
            match &self.previous {
                Some(previous) => std::env::set_var(self.key, previous),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
