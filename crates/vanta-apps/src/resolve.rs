use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use crate::Application;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    EmptyExecutable,
    NotFound(String),
    NotExecutable(PathBuf),
    InvalidTryExec(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyExecutable => write!(f, "Executable name is empty"),
            Self::NotFound(name) => write!(f, "Executable not found: {name}"),
            Self::NotExecutable(path) => {
                write!(f, "File is not executable: {}", path.display())
            }
            Self::InvalidTryExec(name) => {
                write!(f, "Application TryExec is unavailable: {name}")
            }
        }
    }
}

impl std::error::Error for ResolveError {}

pub fn resolve_executable(
    executable: &OsStr,
    search_path: Option<&OsStr>,
) -> Result<PathBuf, ResolveError> {
    if executable.is_empty() {
        return Err(ResolveError::EmptyExecutable);
    }

    let candidate = Path::new(executable);

    if candidate.components().count() > 1 || candidate.is_absolute() {
        return validate_executable(candidate);
    }

    let search_path = search_path
        .map(OsString::from)
        .or_else(|| env::var_os("PATH"))
        .unwrap_or_default();

    for directory in env::split_paths(&search_path) {
        if directory.as_os_str().is_empty() {
            continue;
        }

        let candidate = directory.join(candidate);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }

    Err(ResolveError::NotFound(
        executable.to_string_lossy().into_owned(),
    ))
}

pub fn resolve_application_executable(
    application: &Application,
    arguments: &[String],
) -> Result<PathBuf, ResolveError> {
    if let Some(try_exec) = &application.try_exec {
        if resolve_executable(OsStr::new(try_exec), None).is_err() {
            return Err(ResolveError::InvalidTryExec(try_exec.clone()));
        }
    }

    let executable = arguments.first().ok_or(ResolveError::EmptyExecutable)?;
    resolve_executable(OsStr::new(executable), None)
}

fn validate_executable(path: &Path) -> Result<PathBuf, ResolveError> {
    if is_executable(path) {
        Ok(path.to_path_buf())
    } else if path.exists() {
        Err(ResolveError::NotExecutable(path.to_path_buf()))
    } else {
        Err(ResolveError::NotFound(path.to_string_lossy().into_owned()))
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("vanta-apps-resolve-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn resolves_current_executable_by_absolute_path() {
        let executable = env::current_exe().unwrap();
        let resolved = resolve_executable(executable.as_os_str(), None).unwrap();

        assert_eq!(resolved, executable);
    }

    #[test]
    fn rejects_missing_executable() {
        let error =
            resolve_executable(OsStr::new("/vanta-test/nonexistent-executable"), None).unwrap_err();

        assert!(matches!(error, ResolveError::NotFound(_)));
    }

    #[cfg(unix)]
    #[test]
    fn resolves_executable_using_supplied_path() {
        use std::os::unix::fs::symlink;

        let directory = test_dir();
        let executable = env::current_exe().unwrap();
        let link = directory.join("vanta-test-program");
        symlink(executable, &link).unwrap();

        let resolved = resolve_executable(
            OsStr::new("vanta-test-program"),
            Some(directory.as_os_str()),
        )
        .unwrap();

        assert_eq!(resolved, link);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_dir();
        let file = directory.join("not-executable");
        fs::write(&file, "test").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(matches!(
            resolve_executable(file.as_os_str(), None),
            Err(ResolveError::NotExecutable(_))
        ));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn try_exec_must_resolve_before_launch() {
        let mut application = Application {
            id: "org.example.Test.desktop".into(),
            name: "Test".into(),
            generic_name: None,
            comment: None,
            icon: None,
            categories: Vec::new(),
            exec: Some("true".into()),
            try_exec: Some("vanta-missing-tryexec".into()),
            terminal: false,
            working_directory: None,
            startup_wm_class: None,
            dbus_activatable: false,
            desktop_file: PathBuf::from("/tmp/Test.desktop"),
        };

        assert!(matches!(
            resolve_application_executable(&application, &["true".into()]),
            Err(ResolveError::InvalidTryExec(_))
        ));

        application.try_exec = None;
        assert!(resolve_application_executable(&application, &["true".into()]).is_ok());
    }
}
