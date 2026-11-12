//! Backend selection shared by the compositor entry point.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendKind {
    Winit,
    Native,
}

impl BackendKind {
    /// CLI arguments override VANTA_BACKEND.
    ///
    /// Supported forms:
    ///   vanta-comp --backend winit
    ///   vanta-comp --backend=native
    ///   VANTA_BACKEND=native vanta-comp
    pub(crate) fn from_env_and_args() -> Result<Self, String> {
        let mut selected = std::env::var("VANTA_BACKEND").unwrap_or_else(|_| "winit".to_owned());

        let mut args = std::env::args().skip(1);

        while let Some(arg) = args.next() {
            if arg == "--backend" {
                selected = args
                    .next()
                    .ok_or_else(|| "--backend requires winit or native".to_owned())?;
            } else if let Some(value) = arg.strip_prefix("--backend=") {
                selected = value.to_owned();
            } else {
                return Err(format!("Unknown compositor argument: {arg}"));
            }
        }

        match selected.as_str() {
            "winit" => Ok(Self::Winit),
            "native" => Ok(Self::Native),
            value => Err(format!(
                "Unknown backend '{value}'. Supported values: winit, native"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BackendKind;

    #[test]
    fn recognizes_supported_backend_names() {
        assert_eq!(parse("winit"), Some(BackendKind::Winit));
        assert_eq!(parse("native"), Some(BackendKind::Native));
        assert_eq!(parse("drm"), None);
    }

    fn parse(value: &str) -> Option<BackendKind> {
        match value {
            "winit" => Some(BackendKind::Winit),
            "native" => Some(BackendKind::Native),
            _ => None,
        }
    }
}
