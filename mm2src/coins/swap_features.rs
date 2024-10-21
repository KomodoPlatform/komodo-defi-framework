/// Framework to activate new swap protocol features at certain protocol version

#[derive(PartialEq)]
pub enum SwapFeature {
    // Sending part of dex fee to a dedicated account to exchange it on KMD coins and burn them
    SendToPreBurnAccount,
}

impl SwapFeature {
    // add new features to activate
    const SWAP_FEATURE_ACTIVATION: &[(u16, SwapFeature)] = &[(1, SwapFeature::SendToPreBurnAccount)];

    pub fn is_active(feature: SwapFeature, version: u16) -> bool {
        if let Some(found) = Self::SWAP_FEATURE_ACTIVATION.iter().find(|fv| fv.1 == feature) {
            return version >= found.0;
        }
        false
    }
}
