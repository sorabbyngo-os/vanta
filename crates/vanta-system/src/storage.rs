//! Read-only, filesystem-agnostic storage discovery through UDisks2.
//!
//! UDisks2 and kernel drivers own storage operations. Vanta only discovers
//! storage topology and metadata in this module.

use serde::de::DeserializeOwned;
use zbus::{Connection, OwnedObjectPath, Proxy};

use std::collections::HashMap;

const UDISKS_BUS: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

const DRIVE_IFACE: &str = "org.freedesktop.UDisks2.Drive";
const BLOCK_IFACE: &str = "org.freedesktop.UDisks2.Block";
const PARTITION_IFACE: &str = "org.freedesktop.UDisks2.Partition";
const PARTITION_TABLE_IFACE: &str = "org.freedesktop.UDisks2.PartitionTable";
const FILESYSTEM_IFACE: &str = "org.freedesktop.UDisks2.Filesystem";
const ENCRYPTED_IFACE: &str = "org.freedesktop.UDisks2.Encrypted";

#[derive(Debug, Clone)]
pub struct StorageSnapshot {
    pub drives: Vec<StorageDrive>,
    pub block_devices: Vec<BlockDevice>,
}

#[derive(Debug, Clone)]
pub struct StorageDrive {
    pub object_path: String,
    pub id: Option<String>,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub revision: Option<String>,
    pub serial: Option<String>,
    pub size_bytes: u64,
    pub connection_bus: Option<String>,
    pub media: Option<String>,
    pub media_available: Option<bool>,
    pub media_removable: Option<bool>,
    pub removable: Option<bool>,
    pub ejectable: Option<bool>,
    pub optical: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct BlockDevice {
    pub object_path: String,
    pub device: String,
    pub preferred_device: Option<String>,
    pub symlinks: Vec<String>,
    pub size_bytes: u64,
    pub read_only: bool,

    pub drive_path: Option<String>,
    pub crypto_backing_device: Option<String>,

    pub id: Option<String>,
    pub filesystem_type: Option<String>,
    pub filesystem_label: Option<String>,
    pub filesystem_uuid: Option<String>,
    pub filesystem_usage: Option<String>,
    pub filesystem_version: Option<String>,

    pub hint_name: Option<String>,
    pub hint_icon_name: Option<String>,
    pub hint_system: Option<bool>,
    pub hint_ignore: Option<bool>,

    pub partition: Option<PartitionInfo>,
    pub partition_table: Option<PartitionTableInfo>,
    pub filesystem: Option<FilesystemInfo>,
    pub encryption: Option<EncryptionInfo>,
}

#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub number: u32,
    pub name: Option<String>,
    pub type_guid: Option<String>,
    pub uuid: Option<String>,
    pub flags: u64,
    pub offset_bytes: u64,
    pub size_bytes: u64,
    pub table_path: Option<String>,
    pub is_container: Option<bool>,
    pub is_contained: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PartitionTableInfo {
    pub table_type: Option<String>,
    pub partitions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FilesystemInfo {
    /// Raw filesystem name supplied by UDisks2. This intentionally remains
    /// a String so new filesystem types do not require Vanta updates.
    pub filesystem_type: Option<String>,
    pub label: Option<String>,
    pub uuid: Option<String>,
    pub mount_points: Vec<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct EncryptionInfo {
    pub cleartext_device: Option<String>,
    pub hint_encryption_type: Option<String>,
    pub metadata_size_bytes: Option<u64>,
}

async fn property<T>(proxy: &Proxy<'_>, name: &str) -> Result<T, String>
where
    T: DeserializeOwned + zbus::zvariant::Type,
{
    proxy
        .get_property(name)
        .await
        .map_err(|error| format!("{name}: {error}"))
}

async fn optional_property<T>(proxy: &Proxy<'_>, name: &str) -> Option<T>
where
    T: DeserializeOwned + zbus::zvariant::Type,
{
    property(proxy, name).await.ok()
}

fn bytes_to_string(bytes: Vec<u8>) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

fn byte_paths_to_strings(paths: Vec<Vec<u8>>) -> Vec<String> {
    paths.into_iter().filter_map(bytes_to_string).collect()
}

fn non_root_path(path: OwnedObjectPath) -> Option<String> {
    let value = path.to_string();
    (value != "/").then_some(value)
}

async fn read_drive(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Result<StorageDrive, String> {
    let proxy = Proxy::new(connection, UDISKS_BUS, path.as_str(), DRIVE_IFACE)
        .await
        .map_err(|error| format!("Drive {}: {error}", path.as_str()))?;

    Ok(StorageDrive {
        object_path: path.to_string(),
        id: optional_property(&proxy, "Id").await,
        model: optional_property(&proxy, "Model").await,
        vendor: optional_property(&proxy, "Vendor").await,
        revision: optional_property(&proxy, "Revision").await,
        serial: optional_property(&proxy, "Serial").await,
        size_bytes: optional_property(&proxy, "Size").await.unwrap_or(0),
        connection_bus: optional_property(&proxy, "ConnectionBus").await,
        media: optional_property(&proxy, "Media").await,
        media_available: optional_property(&proxy, "MediaAvailable").await,
        media_removable: optional_property(&proxy, "MediaRemovable").await,
        removable: optional_property(&proxy, "Removable").await,
        ejectable: optional_property(&proxy, "Ejectable").await,
        optical: optional_property(&proxy, "Optical").await,
    })
}

async fn read_partition(connection: &Connection, path: &OwnedObjectPath) -> Option<PartitionInfo> {
    let proxy = Proxy::new(connection, UDISKS_BUS, path.as_str(), PARTITION_IFACE)
        .await
        .ok()?;

    Some(PartitionInfo {
        number: optional_property(&proxy, "Number").await?,
        name: optional_property(&proxy, "Name").await,
        type_guid: optional_property(&proxy, "Type").await,
        uuid: optional_property(&proxy, "UUID").await,
        flags: optional_property(&proxy, "Flags").await.unwrap_or(0),
        offset_bytes: optional_property(&proxy, "Offset").await.unwrap_or(0),
        size_bytes: optional_property(&proxy, "Size").await.unwrap_or(0),
        table_path: optional_property::<OwnedObjectPath>(&proxy, "Table")
            .await
            .and_then(non_root_path),
        is_container: optional_property(&proxy, "IsContainer").await,
        is_contained: optional_property(&proxy, "IsContained").await,
    })
}

async fn read_partition_table(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Option<PartitionTableInfo> {
    let proxy = Proxy::new(connection, UDISKS_BUS, path.as_str(), PARTITION_TABLE_IFACE)
        .await
        .ok()?;

    let partitions: Vec<OwnedObjectPath> = optional_property(&proxy, "Partitions")
        .await
        .unwrap_or_default();

    Some(PartitionTableInfo {
        table_type: optional_property(&proxy, "Type").await,
        partitions: partitions
            .into_iter()
            .map(|path| path.to_string())
            .collect(),
    })
}

async fn read_filesystem(
    connection: &Connection,
    path: &OwnedObjectPath,
    filesystem_type: Option<String>,
    label: Option<String>,
    uuid: Option<String>,
) -> Option<FilesystemInfo> {
    let proxy = Proxy::new(connection, UDISKS_BUS, path.as_str(), FILESYSTEM_IFACE)
        .await
        .ok()?;

    let mount_points: Vec<Vec<u8>> = optional_property(&proxy, "MountPoints")
        .await
        .unwrap_or_default();

    Some(FilesystemInfo {
        filesystem_type,
        label,
        uuid,
        mount_points: byte_paths_to_strings(mount_points),
        size_bytes: optional_property(&proxy, "Size").await,
    })
}

async fn read_encryption(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Option<EncryptionInfo> {
    let proxy = Proxy::new(connection, UDISKS_BUS, path.as_str(), ENCRYPTED_IFACE)
        .await
        .ok()?;

    Some(EncryptionInfo {
        cleartext_device: optional_property::<OwnedObjectPath>(&proxy, "CleartextDevice")
            .await
            .and_then(non_root_path),
        hint_encryption_type: optional_property(&proxy, "HintEncryptionType").await,
        metadata_size_bytes: optional_property(&proxy, "MetadataSize").await,
    })
}

async fn read_block(
    connection: &Connection,
    path: &OwnedObjectPath,
    interfaces: &HashMap<String, HashMap<String, zbus::OwnedVariant>>,
) -> Result<BlockDevice, String> {
    let proxy = Proxy::new(connection, UDISKS_BUS, path.as_str(), BLOCK_IFACE)
        .await
        .map_err(|error| format!("Block {}: {error}", path.as_str()))?;

    let device_bytes: Vec<u8> = property(&proxy, "Device").await?;
    let device = bytes_to_string(device_bytes)
        .ok_or_else(|| format!("Block {} has an invalid Device path", path.as_str()))?;

    let preferred_device = optional_property::<Vec<u8>>(&proxy, "PreferredDevice")
        .await
        .and_then(bytes_to_string);

    let symlink_bytes: Vec<Vec<u8>> = optional_property(&proxy, "Symlinks")
        .await
        .unwrap_or_default();

    let drive_path = optional_property::<OwnedObjectPath>(&proxy, "Drive")
        .await
        .and_then(non_root_path);

    let crypto_backing_device = optional_property::<OwnedObjectPath>(&proxy, "CryptoBackingDevice")
        .await
        .and_then(non_root_path);

    let id_type: Option<String> = optional_property(&proxy, "IdType").await;
    let id_label: Option<String> = optional_property(&proxy, "IdLabel").await;
    let id_uuid: Option<String> = optional_property(&proxy, "IdUUID").await;
    let id_usage: Option<String> = optional_property(&proxy, "IdUsage").await;
    let id_version: Option<String> = optional_property(&proxy, "IdVersion").await;

    let has_partition = interfaces.contains_key(PARTITION_IFACE);
    let has_partition_table = interfaces.contains_key(PARTITION_TABLE_IFACE);
    let has_filesystem = interfaces.contains_key(FILESYSTEM_IFACE);
    let has_encrypted = interfaces.contains_key(ENCRYPTED_IFACE);

    let partition = if has_partition {
        read_partition(connection, path).await
    } else {
        None
    };

    let partition_table = if has_partition_table {
        read_partition_table(connection, path).await
    } else {
        None
    };

    let filesystem = if has_filesystem {
        read_filesystem(
            connection,
            path,
            id_type.clone(),
            id_label.clone(),
            id_uuid.clone(),
        )
        .await
    } else {
        None
    };

    let encryption = if has_encrypted {
        read_encryption(connection, path).await
    } else {
        None
    };

    Ok(BlockDevice {
        object_path: path.to_string(),
        device,
        preferred_device,
        symlinks: byte_paths_to_strings(symlink_bytes),
        size_bytes: property(&proxy, "Size").await?,
        read_only: optional_property(&proxy, "ReadOnly").await.unwrap_or(false),
        drive_path,
        crypto_backing_device,
        id: optional_property(&proxy, "Id").await,
        filesystem_type: id_type,
        filesystem_label: id_label,
        filesystem_uuid: id_uuid,
        filesystem_usage: id_usage,
        filesystem_version: id_version,
        hint_name: optional_property(&proxy, "HintName").await,
        hint_icon_name: optional_property(&proxy, "HintIconName").await,
        hint_system: optional_property(&proxy, "HintSystem").await,
        hint_ignore: optional_property(&proxy, "HintIgnore").await,
        partition,
        partition_table,
        filesystem,
        encryption,
    })
}

pub(crate) async fn read_storage_snapshot(
    connection: &Connection,
) -> Result<StorageSnapshot, String> {
    let manager = Proxy::new(connection, UDISKS_BUS, ROOT, OBJECT_MANAGER)
        .await
        .map_err(|error| format!("UDisks2 ObjectManager: {error}"))?;

    let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, zbus::OwnedVariant>>> =
        manager
            .call("GetManagedObjects", &())
            .await
            .map_err(|error| format!("UDisks2 GetManagedObjects: {error}"))?;

    let mut drives = Vec::new();
    let mut block_devices = Vec::new();

    for (path, interfaces) in &objects {
        if interfaces.contains_key(DRIVE_IFACE) {
            match read_drive(connection, path).await {
                Ok(drive) => drives.push(drive),
                Err(error) => eprintln!("Vanta: skipping unreadable storage drive: {error}"),
            }
        }

        if interfaces.contains_key(BLOCK_IFACE) {
            match read_block(connection, path, interfaces).await {
                Ok(block) => block_devices.push(block),
                Err(error) => eprintln!("Vanta: skipping unreadable block device: {error}"),
            }
        }
    }

    drives.sort_by(|a, b| a.object_path.cmp(&b.object_path));
    block_devices.sort_by(|a, b| a.device.cmp(&b.device));

    Ok(StorageSnapshot {
        drives,
        block_devices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_path_decoding_handles_null_termination() {
        assert_eq!(
            bytes_to_string(b"/dev/sda\0".to_vec()).as_deref(),
            Some("/dev/sda")
        );
    }

    #[test]
    fn byte_path_decoding_handles_missing_null() {
        assert_eq!(
            bytes_to_string(b"/mnt/data".to_vec()).as_deref(),
            Some("/mnt/data")
        );
    }

    #[test]
    fn empty_byte_path_is_none() {
        assert_eq!(bytes_to_string(vec![0]), None);
    }

    #[test]
    fn root_object_path_is_not_a_real_relationship() {
        let path = OwnedObjectPath::try_from("/").unwrap();
        assert_eq!(non_root_path(path), None);
    }

    #[test]
    fn valid_object_path_relationship_is_preserved() {
        let path =
            OwnedObjectPath::try_from("/org/freedesktop/UDisks2/block_devices/dm_2d0").unwrap();
        assert_eq!(
            non_root_path(path).as_deref(),
            Some("/org/freedesktop/UDisks2/block_devices/dm_2d0")
        );
    }
}
