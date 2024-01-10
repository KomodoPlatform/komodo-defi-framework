use crate::proto::messages_management::Features;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrezorDeviceInfo {
    /// The device model.
    model: Option<String>,
    /// Name given to the device.
    device_name: Option<String>,
    /// Unique device identifier.
    device_id: Option<String>,
}

impl From<Features> for TrezorDeviceInfo {
    fn from(features: Features) -> Self {
        TrezorDeviceInfo {
            model: features.model,
            device_name: features.label,
            device_id: features.device_id,
        }
    }
}
