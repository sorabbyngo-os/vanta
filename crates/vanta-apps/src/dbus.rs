use std::collections::HashMap;

use zbus::{Connection, OwnedVariant, Proxy};

use crate::Application;

const APPLICATION_PATH: &str = "/org/freedesktop/Application";
const APPLICATION_INTERFACE: &str = "org.freedesktop.Application";

#[derive(Debug)]
pub enum DbusActivationError {
    NotActivatable,
    InvalidApplicationId(String),
    Connection(String),
    Proxy(String),
    Activation(String),
}

impl std::fmt::Display for DbusActivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotActivatable => write!(f, "Application does not support D-Bus activation"),
            Self::InvalidApplicationId(id) => {
                write!(f, "Invalid D-Bus application ID: {id}")
            }
            Self::Connection(error) => write!(f, "Session bus connection failed: {error}"),
            Self::Proxy(error) => write!(f, "D-Bus application proxy failed: {error}"),
            Self::Activation(error) => write!(f, "D-Bus activation failed: {error}"),
        }
    }
}

impl std::error::Error for DbusActivationError {}

pub fn destination_for_desktop_id(id: &str) -> Result<String, DbusActivationError> {
    let Some(destination) = id.strip_suffix(".desktop") else {
        return Err(DbusActivationError::InvalidApplicationId(id.to_owned()));
    };

    let valid = destination.contains('.')
        && destination.split('.').all(|component| {
            !component.is_empty()
                && !component.starts_with(|c: char| c.is_ascii_digit())
                && component
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        });

    if !valid {
        return Err(DbusActivationError::InvalidApplicationId(id.to_owned()));
    }

    Ok(destination.to_owned())
}

async fn application_proxy<'a>(
    connection: &'a Connection,
    destination: &'a str,
) -> Result<Proxy<'a>, DbusActivationError> {
    Proxy::new(
        connection,
        destination,
        APPLICATION_PATH,
        APPLICATION_INTERFACE,
    )
    .await
    .map_err(|error| DbusActivationError::Proxy(error.to_string()))
}

pub async fn activate(application: &Application) -> Result<(), DbusActivationError> {
    if !application.dbus_activatable {
        return Err(DbusActivationError::NotActivatable);
    }

    let destination = destination_for_desktop_id(&application.id)?;
    let connection = Connection::session()
        .await
        .map_err(|error| DbusActivationError::Connection(error.to_string()))?;

    let proxy = application_proxy(&connection, &destination).await?;
    let platform_data = HashMap::<String, OwnedVariant>::new();

    proxy
        .call::<_, _, ()>("Activate", &(platform_data,))
        .await
        .map_err(|error| DbusActivationError::Activation(error.to_string()))
}

pub async fn open(application: &Application, uris: Vec<String>) -> Result<(), DbusActivationError> {
    if !application.dbus_activatable {
        return Err(DbusActivationError::NotActivatable);
    }

    let destination = destination_for_desktop_id(&application.id)?;
    let connection = Connection::session()
        .await
        .map_err(|error| DbusActivationError::Connection(error.to_string()))?;

    let proxy = application_proxy(&connection, &destination).await?;
    let platform_data = HashMap::<String, OwnedVariant>::new();

    proxy
        .call::<_, _, ()>("Open", &(uris, platform_data))
        .await
        .map_err(|error| DbusActivationError::Activation(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_loupe_bus_name() {
        assert_eq!(
            destination_for_desktop_id("org.gnome.Loupe.desktop").unwrap(),
            "org.gnome.Loupe"
        );
    }

    #[test]
    fn rejects_non_reverse_dns_desktop_ids() {
        assert!(destination_for_desktop_id("Utilities-example.desktop").is_err());
        assert!(destination_for_desktop_id("example.desktop").is_err());
        assert!(destination_for_desktop_id("org..example.desktop").is_err());
        assert!(destination_for_desktop_id("org.1example.App.desktop").is_err());
    }

    #[test]
    fn requires_desktop_suffix() {
        assert!(destination_for_desktop_id("org.gnome.Loupe").is_err());
    }
}
