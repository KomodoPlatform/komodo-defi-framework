use crate::proto::messages_management::Features;

#[derive(Clone, Debug, Serialize)]
pub enum TrezorModel {
    One,
    T,
    Other(String),
}

impl From<String> for TrezorModel {
    fn from(model: String) -> Self {
        match model.as_str() {
            "1" => TrezorModel::One,
            "T" => TrezorModel::T,
            _ => TrezorModel::Other(model),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TrezorDeviceInfo {
    /// The device model.
    model: Option<TrezorModel>,
    /// Name given to the device.
    device_name: Option<String>,
    /// Unique device identifier.
    device_id: Option<String>,
}

impl From<Features> for TrezorDeviceInfo {
    fn from(features: Features) -> Self {
        TrezorDeviceInfo {
            model: features.model.map(TrezorModel::from),
            device_name: features.label,
            device_id: features.device_id,
        }
    }
}
