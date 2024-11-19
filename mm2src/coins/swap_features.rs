/// Framework to activate new swap protocol features at certain protocol version

#[derive(PartialEq)]
pub enum LegacySwapFeature {
    // Sending part of dex fee to a dedicated account to exchange it on KMD coins and burn them
    SendToPreBurnAccount,
}

impl LegacySwapFeature {
    /// Features activated/deactivated by protocol version. Tuple elements are:
    /// element.0 is version when the feature is activated,
    /// element.1 is version when the feature is discontinued,
    /// element.2 is a feature itself, registered in the LegacySwapFeature enum
    const SWAP_FEATURE_ACTIVATION: &[(u16, Option<u16>, LegacySwapFeature)] = &[
        (1, None, LegacySwapFeature::SendToPreBurnAccount),
        // add more features to activate...
    ];

    /// Returns true if feature is active for the protocol version param
    pub fn is_active(feature: LegacySwapFeature, version: u16) -> bool {
        if let Some(found) = Self::SWAP_FEATURE_ACTIVATION.iter().find(|fv| fv.2 == feature) {
            return version >= found.0
                && found
                    .1
                    .map(|ver_discontinued| version < ver_discontinued)
                    .unwrap_or(true);
        }
        false
    }
}
