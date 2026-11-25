pub mod dbus;
pub mod exec;
pub mod launch;
pub mod process;
pub mod process_manager;
pub mod resolve;
use std::{
    collections::{HashMap, HashSet},
    env, fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    pub terminal: bool,
    pub working_directory: Option<PathBuf>,
    pub startup_wm_class: Option<String>,
    pub dbus_activatable: bool,
    pub desktop_file: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct ApplicationCatalog {
    pub applications: Vec<Application>,
}

impl ApplicationCatalog {
    pub fn discover() -> io::Result<Self> {
        let mut directories = Vec::new();

        let data_home = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));

        if let Some(path) = data_home {
            directories.push(path.join("applications"));
        }

        let data_dirs =
            env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());

        for directory in env::split_paths(&data_dirs) {
            directories.push(directory.join("applications"));
        }

        Self::discover_from(&directories)
    }

    pub fn discover_from(directories: &[PathBuf]) -> io::Result<Self> {
        let desktops = env::var("XDG_CURRENT_DESKTOP").ok().map(|value| {
            value
                .split(':')
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });

        Self::discover_from_for_desktop(directories, desktops.as_deref())
    }

    pub fn discover_from_for_desktop(
        directories: &[PathBuf],
        current_desktops: Option<&[String]>,
    ) -> io::Result<Self> {
        let mut seen = HashSet::new();
        let mut applications = Vec::new();

        for root in directories {
            if !root.is_dir() {
                continue;
            }

            let mut files = Vec::new();
            collect_desktop_files(root, &mut files);
            files.sort();

            for file in files {
                let relative = match file.strip_prefix(root) {
                    Ok(path) => path,
                    Err(_) => continue,
                };

                let id = desktop_file_id(relative);

                if seen.contains(&id) {
                    continue;
                }

                let contents = match fs::read_to_string(&file) {
                    Ok(contents) => contents,
                    Err(_) => continue,
                };

                let entries = parse_desktop_entry(&contents);
                if entries.is_empty() {
                    continue;
                }

                // Higher-priority entries mask lower-priority entries,
                // including when hidden or excluded from this desktop.
                seen.insert(id.clone());

                if entries.get("Type").map(String::as_str) != Some("Application")
                    || is_true(entries.get("Hidden"))
                    || is_true(entries.get("NoDisplay"))
                    || !is_visible_on_desktop(&entries, current_desktops)
                {
                    continue;
                }

                let name =
                    localized_value(&entries, "Name").or_else(|| entries.get("Name").cloned());

                let Some(name) = name else {
                    continue;
                };

                let exec = entries.get("Exec").cloned();
                let dbus_activatable = is_true(entries.get("DBusActivatable"));

                if exec.is_none() && !dbus_activatable {
                    continue;
                }

                let categories = entries
                    .get("Categories")
                    .map(|value| {
                        value
                            .split(';')
                            .filter(|item| !item.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();

                applications.push(Application {
                    id,
                    name,
                    generic_name: localized_value(&entries, "GenericName")
                        .or_else(|| entries.get("GenericName").cloned()),
                    comment: localized_value(&entries, "Comment")
                        .or_else(|| entries.get("Comment").cloned()),
                    icon: entries.get("Icon").cloned(),
                    categories,
                    exec,
                    try_exec: entries.get("TryExec").cloned(),
                    terminal: is_true(entries.get("Terminal")),
                    working_directory: entries.get("Path").map(PathBuf::from),
                    startup_wm_class: entries.get("StartupWMClass").cloned(),
                    dbus_activatable,
                    desktop_file: file,
                });
            }
        }

        applications.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });

        Ok(Self { applications })
    }

    pub fn find(&self, id: &str) -> Option<&Application> {
        self.applications.iter().find(|app| app.id == id)
    }
}

fn collect_desktop_files(directory: &Path, output: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            collect_desktop_files(&path, output);
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "desktop") {
            output.push(path);
        }
    }
}

fn desktop_file_id(relative: &Path) -> String {
    relative.to_string_lossy().replace('/', "-")
}

fn parse_desktop_entry(contents: &str) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    let mut in_desktop_entry = false;

    for line in contents.lines() {
        let line = line.trim();

        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }

        if !in_desktop_entry || line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        entries.insert(key.trim().to_owned(), unescape(value.trim()));
    }

    entries
}

fn unescape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('s') => output.push(' '),
            Some('n') => output.push('\n'),
            Some('t') => output.push('\t'),
            Some('r') => output.push('\r'),
            Some('\\') => output.push('\\'),
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }

    output
}

fn is_true(value: Option<&String>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn is_visible_on_desktop(
    entries: &HashMap<String, String>,
    current_desktops: Option<&[String]>,
) -> bool {
    let Some(desktops) = current_desktops.filter(|items| !items.is_empty()) else {
        return true;
    };

    let matches = |key: &str| {
        entries.get(key).is_some_and(|value| {
            value
                .split(';')
                .filter(|item| !item.is_empty())
                .any(|item| desktops.iter().any(|desktop| desktop == item))
        })
    };

    if entries.contains_key("OnlyShowIn") && !matches("OnlyShowIn") {
        return false;
    }

    !matches("NotShowIn")
}

fn localized_value(entries: &HashMap<String, String>, key: &str) -> Option<String> {
    let locale = env::var("LANG").ok()?;
    let locale = locale.split('.').next().unwrap_or(&locale);
    let locale = locale.split('@').next().unwrap_or(locale);

    let mut candidates = Vec::new();

    if !locale.is_empty() && locale != "C" && locale != "POSIX" {
        candidates.push(format!("{key}[{locale}]"));

        if let Some(language) = locale.split('_').next() {
            candidates.push(format!("{key}[{language}]"));
        }
    }

    candidates
        .into_iter()
        .find_map(|candidate| entries.get(&candidate).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_file_id_uses_hyphenated_relative_path() {
        assert_eq!(
            desktop_file_id(Path::new("Utilities/example.desktop")),
            "Utilities-example.desktop"
        );
    }

    #[test]
    fn desktop_entry_parser_only_reads_desktop_entry_group() {
        let input = "[Desktop Entry]\nName=Test\\sApp\nType=Application\n[Other]\nName=Ignored\n";
        let entries = parse_desktop_entry(input);

        assert_eq!(entries.get("Name").map(String::as_str), Some("Test App"));
        assert_eq!(entries.get("Type").map(String::as_str), Some("Application"));
    }

    #[test]
    fn hidden_boolean_is_case_insensitive() {
        assert!(is_true(Some(&"TRUE".to_owned())));
        assert!(!is_true(Some(&"false".to_owned())));
    }

    #[test]
    fn only_show_in_requires_a_matching_desktop() {
        let entries = HashMap::from([("OnlyShowIn".to_owned(), "GNOME;KDE;".to_owned())]);
        let desktops = vec!["GNOME".to_owned()];

        assert!(is_visible_on_desktop(&entries, Some(&desktops)));
        assert!(!is_visible_on_desktop(&entries, Some(&["XFCE".to_owned()])));
    }

    #[test]
    fn not_show_in_excludes_matching_desktop() {
        let entries = HashMap::from([("NotShowIn".to_owned(), "GNOME;KDE;".to_owned())]);

        assert!(!is_visible_on_desktop(
            &entries,
            Some(&["GNOME".to_owned()])
        ));
        assert!(is_visible_on_desktop(&entries, Some(&["XFCE".to_owned()])));
    }

    #[test]
    fn only_show_in_and_not_show_in_are_both_applied() {
        let entries = HashMap::from([
            ("OnlyShowIn".to_owned(), "GNOME;KDE;".to_owned()),
            ("NotShowIn".to_owned(), "KDE;".to_owned()),
        ]);

        assert!(is_visible_on_desktop(&entries, Some(&["GNOME".to_owned()])));
        assert!(!is_visible_on_desktop(&entries, Some(&["KDE".to_owned()])));
    }

    #[test]
    fn desktop_filters_are_ignored_without_session_identity() {
        let entries = HashMap::from([
            ("OnlyShowIn".to_owned(), "GNOME;".to_owned()),
            ("NotShowIn".to_owned(), "GNOME;".to_owned()),
        ]);

        assert!(is_visible_on_desktop(&entries, None));
        assert!(is_visible_on_desktop(&entries, Some(&[])));
    }
}
