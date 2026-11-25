use std::{env, fs, io, path::PathBuf};

#[derive(Debug, Default, Clone)]
pub struct DockPreferences {
    pub pinned: Vec<String>,
}

impl DockPreferences {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };

        let Ok(contents) = fs::read_to_string(path) else {
            return Self::default();
        };

        let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
            return Self::default();
        };

        let pinned = value
            .get("pinned")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        Self { pinned }
    }

    pub fn toggle(&mut self, id: &str) -> bool {
        if let Some(index) = self.pinned.iter().position(|item| item == id) {
            self.pinned.remove(index);
            false
        } else {
            self.pinned.push(id.to_owned());
            true
        }
    }

    pub fn move_pin(&mut self, id: &str, delta: isize) -> bool {
        let Some(index) = self.pinned.iter().position(|item| item == id) else {
            return false;
        };
        let target = (index as isize + delta).clamp(0, self.pinned.len() as isize - 1) as usize;
        if target == index {
            return false;
        }
        let item = self.pinned.remove(index);
        self.pinned.insert(target, item);
        true
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| io::Error::other("config directory unavailable"))?;

        let parent = path.parent().expect("config path parent");
        fs::create_dir_all(parent)?;

        let data = serde_json::json!({ "pinned": self.pinned });
        let bytes = serde_json::to_vec_pretty(&data)
            .map_err(|error| io::Error::other(error.to_string()))?;

        let temporary = parent.join("dock.json.tmp");
        fs::write(&temporary, bytes)?;
        fs::rename(temporary, path)?;
        Ok(())
    }
}

fn config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;

    Some(base.join("vanta").join("dock.json"))
}
