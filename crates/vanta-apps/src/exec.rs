use std::path::Path;

use crate::Application;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    EmptyCommand,
    UnterminatedQuote,
    TrailingEscape,
    InvalidFieldCode(char),
    MultiArgumentFieldCode(char),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyCommand => write!(f, "Desktop Exec command is empty"),
            Self::UnterminatedQuote => write!(f, "Unterminated double quote in Exec"),
            Self::TrailingEscape => write!(f, "Trailing escape in Exec"),
            Self::InvalidFieldCode(code) => {
                write!(f, "Unsupported desktop-entry field code: %{code}")
            }
            Self::MultiArgumentFieldCode(code) => {
                write!(f, "Field code %{code} must be a separate argument")
            }
        }
    }
}

impl std::error::Error for ExecError {}

#[derive(Debug, Default)]
pub struct ExecContext<'a> {
    pub files: &'a [String],
    pub uris: &'a [String],
}

pub fn parse_exec(
    application: &Application,
    context: &ExecContext<'_>,
) -> Result<Vec<String>, ExecError> {
    let command = application.exec.as_deref().ok_or(ExecError::EmptyCommand)?;
    let tokens = tokenize(command)?;

    if tokens.is_empty() {
        return Err(ExecError::EmptyCommand);
    }

    let mut output = Vec::new();

    for token in tokens {
        if token == "%F" {
            output.extend(context.files.iter().cloned());
            continue;
        }
        if token == "%U" {
            output.extend(context.uris.iter().cloned());
            continue;
        }
        if token == "%i" {
            if let Some(icon) = &application.icon {
                output.push("--icon".to_owned());
                output.push(icon.clone());
            }
            continue;
        }

        output.push(expand_single_argument(&token, application, context)?);
    }

    if output.is_empty() || output[0].is_empty() {
        return Err(ExecError::EmptyCommand);
    }

    Ok(output)
}

fn tokenize(input: &str) -> Result<Vec<String>, ExecError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut started = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            started = true;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
                started = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                started = true;
            }
            ch if ch.is_whitespace() && !in_quotes => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(ch);
                started = true;
            }
        }
    }

    if escaped {
        return Err(ExecError::TrailingEscape);
    }
    if in_quotes {
        return Err(ExecError::UnterminatedQuote);
    }
    if started {
        args.push(current);
    }

    Ok(args)
}

fn expand_single_argument(
    token: &str,
    application: &Application,
    context: &ExecContext<'_>,
) -> Result<String, ExecError> {
    let mut output = String::new();
    let mut chars = token.chars();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            output.push(ch);
            continue;
        }

        let Some(code) = chars.next() else {
            return Err(ExecError::InvalidFieldCode('%'));
        };

        match code {
            '%' => output.push('%'),
            'c' => output.push_str(&application.name),
            'k' => output.push_str(&application.desktop_file.to_string_lossy()),
            'f' => {
                if let Some(file) = context.files.first() {
                    output.push_str(file);
                }
            }
            'u' => {
                if let Some(uri) = context.uris.first() {
                    output.push_str(uri);
                }
            }
            'F' | 'U' | 'i' => {
                return Err(ExecError::MultiArgumentFieldCode(code));
            }
            // Deprecated field codes are ignored by the desktop-entry specification.
            'd' | 'D' | 'n' | 'N' | 'v' | 'm' => {}
            other => return Err(ExecError::InvalidFieldCode(other)),
        }
    }

    Ok(output)
}

pub fn executable_path(arguments: &[String]) -> Option<&Path> {
    arguments.first().map(|executable| Path::new(executable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn application(exec: &str) -> Application {
        Application {
            id: "org.example.Test.desktop".into(),
            name: "Test App".into(),
            generic_name: None,
            comment: None,
            icon: Some("test-icon".into()),
            categories: Vec::new(),
            exec: Some(exec.into()),
            try_exec: None,
            terminal: false,
            working_directory: None,
            startup_wm_class: None,
            dbus_activatable: false,
            desktop_file: PathBuf::from("/usr/share/applications/org.example.Test.desktop"),
        }
    }

    #[test]
    fn parses_quoted_arguments() {
        let app = application(r#"test-app --title "A Test Window""#);
        let args = parse_exec(&app, &ExecContext::default()).unwrap();

        assert_eq!(args, ["test-app", "--title", "A Test Window"]);
    }

    #[test]
    fn parses_escaped_spaces() {
        let app = application(r"test-app /tmp/a\ file.txt");
        let args = parse_exec(&app, &ExecContext::default()).unwrap();

        assert_eq!(args, ["test-app", "/tmp/a file.txt"]);
    }

    #[test]
    fn expands_application_metadata() {
        let app = application("test-app --name %c --desktop %k --percent %%");
        let args = parse_exec(&app, &ExecContext::default()).unwrap();

        assert_eq!(args[2], "Test App");
        assert_eq!(args[4], "/usr/share/applications/org.example.Test.desktop");
        assert_eq!(args[6], "%");
    }

    #[test]
    fn expands_file_uri_and_icon_codes() {
        let app = application("test-app %f %u %F %U %i");
        let context = ExecContext {
            files: &["/tmp/a.txt".into(), "/tmp/b.txt".into()],
            uris: &["file:///tmp/a.txt".into(), "https://example.com".into()],
        };

        let args = parse_exec(&app, &context).unwrap();

        assert_eq!(
            args,
            [
                "test-app",
                "/tmp/a.txt",
                "file:///tmp/a.txt",
                "/tmp/a.txt",
                "/tmp/b.txt",
                "file:///tmp/a.txt",
                "https://example.com",
                "--icon",
                "test-icon",
            ]
        );
    }

    #[test]
    fn rejects_unterminated_quotes_and_escapes() {
        assert_eq!(
            tokenize(r#"test-app "unfinished"#),
            Err(ExecError::UnterminatedQuote)
        );
        assert_eq!(
            tokenize("test-app trailing\\"),
            Err(ExecError::TrailingEscape)
        );
    }

    #[test]
    fn rejects_unknown_and_misplaced_multi_argument_codes() {
        let unknown = application("test-app %z");
        assert_eq!(
            parse_exec(&unknown, &ExecContext::default()),
            Err(ExecError::InvalidFieldCode('z'))
        );

        let misplaced = application("test-app prefix%F");
        assert_eq!(
            parse_exec(&misplaced, &ExecContext::default()),
            Err(ExecError::MultiArgumentFieldCode('F'))
        );
    }

    #[test]
    fn ignores_deprecated_field_codes() {
        let app = application("test-app %d %D %n %N %v %m");
        let args = parse_exec(&app, &ExecContext::default()).unwrap();

        assert_eq!(args, ["test-app", "", "", "", "", "", ""]);
    }
}
