use std::{
    io,
    process::{Child, Command},
};

use crate::launch::LaunchPlan;

#[derive(Debug)]
pub enum ProcessLaunchError {
    UnsupportedPlan(&'static str),
    Spawn(io::Error),
}

impl std::fmt::Display for ProcessLaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlan(reason) => write!(f, "{reason}"),
            Self::Spawn(error) => write!(f, "Application process failed to start: {error}"),
        }
    }
}

impl std::error::Error for ProcessLaunchError {}

impl From<io::Error> for ProcessLaunchError {
    fn from(error: io::Error) -> Self {
        Self::Spawn(error)
    }
}

/// Spawn a normal application and return its child handle.
///
/// Arguments are passed directly to the executable. No shell is involved.
pub fn spawn_process(plan: &LaunchPlan) -> Result<Child, ProcessLaunchError> {
    let LaunchPlan::Process {
        executable,
        arguments,
        working_directory,
        ..
    } = plan
    else {
        return Err(ProcessLaunchError::UnsupportedPlan(match plan {
            LaunchPlan::Terminal { .. } => "Terminal applications must be handed to Vanta Terminal",
            LaunchPlan::DbusActivate { .. } => "D-Bus applications must use D-Bus activation",
            LaunchPlan::DbusOpen { .. } => "D-Bus URI requests must use D-Bus Open",
            LaunchPlan::Process { .. } => unreachable!(),
        }));
    };

    let mut command = Command::new(executable);

    if arguments.len() > 1 {
        command.args(&arguments[1..]);
    }

    if let Some(directory) = working_directory {
        command.current_dir(directory);
    }

    Ok(command.spawn()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[cfg(unix)]
    fn true_plan() -> LaunchPlan {
        LaunchPlan::Process {
            executable: PathBuf::from("/usr/bin/true"),
            arguments: vec!["/usr/bin/true".into()],
            working_directory: None,
            startup_wm_class: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn spawns_process_and_returns_child_handle() {
        let mut child = spawn_process(&true_plan()).unwrap();
        let status = child.wait().unwrap();

        assert!(status.success());
    }

    #[test]
    fn rejects_terminal_plan() {
        let plan = LaunchPlan::Terminal {
            executable: PathBuf::from("/usr/bin/htop"),
            arguments: vec!["htop".into()],
            working_directory: None,
            startup_wm_class: None,
        };

        assert!(matches!(
            spawn_process(&plan),
            Err(ProcessLaunchError::UnsupportedPlan(_))
        ));
    }

    #[test]
    fn rejects_dbus_activation_plan() {
        let plan = LaunchPlan::DbusActivate {
            application_id: "org.example.Test".into(),
        };

        assert!(matches!(
            spawn_process(&plan),
            Err(ProcessLaunchError::UnsupportedPlan(_))
        ));
    }

    #[test]
    fn rejects_dbus_open_plan() {
        let plan = LaunchPlan::DbusOpen {
            application_id: "org.example.Test".into(),
            uris: vec!["https://example.com".into()],
        };

        assert!(matches!(
            spawn_process(&plan),
            Err(ProcessLaunchError::UnsupportedPlan(_))
        ));
    }
}
