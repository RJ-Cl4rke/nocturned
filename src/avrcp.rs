use crate::error::{NocturnedError, Result};
use bluer::Address;
use dbus::blocking::stdintf::org_freedesktop_dbus::ObjectManager;
use dbus::blocking::Connection;
use std::time::Duration;

/// Basic transport controls exposed by BlueZ's AVRCP controller role
/// (`org.bluez.MediaControl1`) once a phone is connected as an AVRCP target.
#[derive(Debug, Clone, Copy)]
pub enum AvrcpAction {
    Play,
    Pause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
}

impl AvrcpAction {
    fn dbus_method(self) -> &'static str {
        match self {
            AvrcpAction::Play => "Play",
            AvrcpAction::Pause => "Pause",
            AvrcpAction::Next => "Next",
            AvrcpAction::Previous => "Previous",
            AvrcpAction::VolumeUp => "VolumeUp",
            AvrcpAction::VolumeDown => "VolumeDown",
        }
    }
}

/// Sends an AVRCP transport command to `address` via BlueZ's
/// `org.bluez.MediaControl1` interface on that device's D-Bus object path.
pub async fn send(address: Address, action: AvrcpAction) -> Result<()> {
    let addr_str = address.to_string();
    let method = action.dbus_method();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = Connection::new_system().map_err(|e| {
            NocturnedError::Config(format!("Failed to connect to D-Bus: {}", e))
        })?;
        let proxy = conn.with_proxy("org.bluez", "/", Duration::from_secs(2));
        let objects = proxy
            .get_managed_objects()
            .map_err(|e| NocturnedError::General(anyhow::anyhow!(e)))?;

        let device_path = objects
            .into_iter()
            .find_map(|(path, ifaces)| {
                let props = ifaces.get("org.bluez.Device1")?;
                let addr = props.get("Address")?.0.as_str()?;
                (addr == addr_str).then_some(path)
            })
            .ok_or_else(|| {
                NocturnedError::General(anyhow::anyhow!("Device not found in BlueZ"))
            })?;

        let dev_proxy = conn.with_proxy("org.bluez", device_path, Duration::from_secs(4));
        let result: std::result::Result<(), dbus::Error> =
            dev_proxy.method_call("org.bluez.MediaControl1", method, ());

        result.map_err(|e| NocturnedError::General(anyhow::anyhow!(e.to_string())))
    })
    .await
    .map_err(|e| NocturnedError::General(anyhow::anyhow!(e.to_string())))?
}
