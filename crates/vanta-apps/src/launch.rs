use std::path::PathBuf;

use crate::{
    dbus::{self, DbusActivationError},
    exec::{parse_exec, ExecContext, ExecError},
    resolve::{resolve_application_executable, ResolveError},
    Application,
};

#[derive(Debug, Clone, Default)]
pub struct LaunchInput {
    pub files: Vec<String>,
    pub uris: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    DbusActivate {
        application_id: String,
    },
    DbusOpen {
        application_id: String,
        uris: Vec<String>,
    },
    Process {
        executable: PathBuf,
        arguments: Vec<String>,
        working_directory: Option<PathBuf>,
        startup_wm_class: Option<String>,
    },
    Terminal {
        executable: PathBuf,
        arguments: Vec<String>,
        working_directory: Option<PathBuf>,
        startup_wm_class: Option<String>,
    },
}

#[derive(Debug)]
pub enum LaunchError {
    Exec(ExecError),
    Resolve(ResolveError),
    Dbus(DbusActivationError),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exec(error) => write!(f, "Desktop Exec parsing failed: {error}"),
            Self::Resolve(error) => write!(f, "Executable resolution failed: {error}"),
            Self::Dbus(error) => write!(f, "D-Bus activation failed: {error}"),
        }
    }
}

impl std::error::Error for LaunchError {}

impl From<ExecError> for LaunchError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<ResolveError> for LaunchError {
    fn from(error: ResolveError) -> Self {
        Self::Resolve(error)
    }
}

impl From<DbusActivationError> for LaunchError {
    fn from(error: DbusActivationError) -> Self {
        Self::Dbus(error)
    }
}

pub fn prepare_launch(
    application: &Application,
    input: &LaunchInput,
) -> Result<LaunchPlan, LaunchError> {
    if application.dbus_activatable {
        let application_id = dbus::destination_for_desktop_id(&application.id)?;

        if !input.uris.is_empty() {
            return Ok(LaunchPlan::DbusOpen {
                application_id,
                uris: input.uris.clone(),
            });
        }

        return Ok(LaunchPlan::DbusActivate { application_id });
    }

    let context = ExecContext {
        files: &input.files,
        uris: &input.uris,
    };

    let mut arguments = parse_exec(application, &context)?;
    let executable = resolve_application_executable(application, &arguments)?;

    // Use the resolved executable path for process creation. Never invoke a shell.
    arguments[0] = executable.to_string_lossy().into_owned();

    let working_directory = application
        .working_directory
        .as_ref()
        .filter(|path| path.is_dir())
        .cloned();

    let common = (
        executable,
        arguments,
        working_directory,
        application.startup_wm_class.clone(),
    );

    if application.terminal {
        let (executable, arguments, working_directory, startup_wm_class) = common;
        Ok(LaunchPlan::Terminal {
            executable,
            arguments,
            working_directory,
            startup_wm_class,
        })
    } else {
        let (executable, arguments, working_directory, startup_wm_class) = common;
        Ok(LaunchPlan::Process {
            executable,
            arguments,
            working_directory,
            startup_wm_class,
        })
    }
}

pub async fn activate_dbus(
    application: &Application,
    plan: &LaunchPlan,
) -> Result<(), LaunchError> {
    match plan {
        LaunchPlan::DbusActivate { .. } => dbus::activate(application).await?,
        LaunchPlan::DbusOpen { uris, .. } => dbus::open(application, uris.clone()).await?,
        LaunchPlan::Process { .. } | LaunchPlan::Terminal { .. } => {
            return Err(LaunchError::Dbus(DbusActivationError::NotActivatable));
        }
    }

    Ok(())
}

/// Run D-Bus activation from a dedicated worker thread.
/// Never call this from the Wayland event-loop thread.
pub fn activate_dbus_blocking(
    application: &Application,
    plan: &LaunchPlan,
) -> Result<(), LaunchError> {
    zbus::block_on(activate_dbus(application, plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn application(exec: &str) -> Application {
        Application {
            id: "org.example.Test.desktop".into(),
            name: "Test App".into(),
            generic_name: None,
            comment: None,
            icon: None,
            categories: Vec::new(),
            exec: Some(exec.into()),
            try_exec: None,
            terminal: false,
            working_directory: None,
            startup_wm_class: Some("TestApp".into()),
            dbus_activatable: false,
            desktop_file: PathBuf::from("/tmp/org.example.Test.desktop"),
        }
    }

    #[test]
    fn plans_normal_process_launch() {
        let app = application("true");
        let plan = prepare_launch(&app, &LaunchInput::default()).unwrap();

        assert!(matches!(
            plan,
            LaunchPlan::Process {
                startup_wm_class: Some(ref class),
                ..
            } if class == "TestApp"
        ));
    }

    #[test]
    fn routes_terminal_applications_to_vanta_terminal_plan() {
        let mut app = application("htop");
        app.terminal = true;

        let plan = prepare_launch(&app, &LaunchInput::default()).unwrap();

        assert!(matches!(plan, LaunchPlan::Terminal { .. }));
    }

    #[test]
    fn dbus_activation_takes_precedence_over_exec() {
        let mut app = application("this-executable-does-not-exist");
        app.dbus_activatable = true;

        let plan = prepare_launch(&app, &LaunchInput::default()).unwrap();

        assert_eq!(
            plan,
            LaunchPlan::DbusActivate {
                application_id: "org.example.Test".into()
            }
        );
    }

    #[test]
    fn dbus_open_is_selected_when_uris_are_supplied() {
        let mut app = application("unused");
        app.dbus_activatable = true;

        let input = LaunchInput {
            files: Vec::new(),
            uris: vec!["https://example.com".into()],
        };

        let plan = prepare_launch(&app, &input).unwrap();

        assert_eq!(
            plan,
            LaunchPlan::DbusOpen {
                application_id: "org.example.Test".into(),
                uris: vec!["https://example.com".into()]
            }
        );
    }

    #[test]
    fn terminal_and_working_directory_metadata_are_preserved() {
        let mut app = application("true");
        app.terminal = true;
        app.working_directory = Some(std::env::temp_dir());

        let plan = prepare_launch(&app, &LaunchInput::default()).unwrap();

        assert!(matches!(
            plan,
            LaunchPlan::Terminal {
                working_directory: Some(_),
                ..
            }
        ));
    }
}
