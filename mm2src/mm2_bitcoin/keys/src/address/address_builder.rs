use crypto::ChecksumType;
use {Address, AddressFormat, AddressHashEnum, AddressPrefix, AddressScriptType, NetworkAddressPrefixes};

/// Params for AddressBuilder to select output script type
#[derive(PartialEq)]
pub enum AddressBuilderOption {
    /// build for pay to pubkey hash output (witness or legacy)
    BuildAsPubkeyHash,
    /// build for pay to script hash output (witness or legacy)
    BuildAsScriptHash,
}

/// Builds Address struct depending on addr_format, validates params to build Address
pub struct AddressBuilder {
    /// Coin base58 address prefixes from coin config
    prefixes: NetworkAddressPrefixes,
    /// Segwit addr human readable part
    hrp: Option<String>,
    /// Public key hash
    hash: AddressHashEnum,
    /// Checksum type
    checksum_type: ChecksumType,
    /// Address Format
    addr_format: AddressFormat,
    /// Indicate whether tx output for this address is pubkey hash or script hash
    build_option: Option<AddressBuilderOption>,
}

impl AddressBuilder {
    pub fn new(
        addr_format: AddressFormat,
        hash: AddressHashEnum,
        checksum_type: ChecksumType,
        prefixes: NetworkAddressPrefixes,
        hrp: Option<String>,
    ) -> Self {
        Self {
            addr_format,
            hash,
            checksum_type,
            prefixes,
            hrp,
            build_option: None,
        }
    }

    /// Sets build option for Address tx output script type
    pub fn with_build_option(mut self, build_option: AddressBuilderOption) -> Self {
        self.build_option = Some(build_option);
        self
    }

    /// Sets Address tx output script type as p2pkh or p2wpkh
    pub fn as_pkh(mut self) -> Self {
        self.build_option = Some(AddressBuilderOption::BuildAsPubkeyHash);
        self
    }

    /// Sets Address tx output script type as p2sh or p2wsh
    pub fn as_sh(mut self) -> Self {
        self.build_option = Some(AddressBuilderOption::BuildAsScriptHash);
        self
    }

    pub fn build(&self) -> Result<Address, String> {
        let build_option = self.build_option.as_ref().ok_or("no address builder option set")?;
        match &self.addr_format {
            AddressFormat::Standard => Ok(Address {
                prefix: self.get_address_prefix(build_option)?,
                hrp: None,
                hash: self.hash.clone(),
                checksum_type: self.checksum_type,
                addr_format: self.addr_format.clone(),
                script_type: self.get_legacy_script_type(build_option),
            }),
            AddressFormat::Segwit => {
                self.check_segwit_hrp()?;
                self.check_segwit_hash(build_option)?;
                Ok(Address {
                    prefix: AddressPrefix::default(),
                    hrp: self.hrp.clone(),
                    hash: self.hash.clone(),
                    checksum_type: self.checksum_type,
                    addr_format: self.addr_format.clone(),
                    script_type: self.get_segwit_script_type(build_option),
                })
            },
            AddressFormat::CashAddress { .. } => Ok(Address {
                prefix: self.get_address_prefix(build_option)?,
                hrp: None,
                hash: self.hash.clone(),
                checksum_type: self.checksum_type,
                addr_format: self.addr_format.clone(),
                script_type: self.get_legacy_script_type(build_option),
            }),
        }
    }

    fn get_address_prefix(&self, build_option: &AddressBuilderOption) -> Result<AddressPrefix, String> {
        let prefix = match build_option {
            AddressBuilderOption::BuildAsPubkeyHash => &self.prefixes.p2pkh,
            AddressBuilderOption::BuildAsScriptHash => &self.prefixes.p2sh,
        };
        if prefix.is_empty() {
            return Err("no prefix for address set".to_owned());
        }
        Ok(prefix.clone())
    }

    fn get_legacy_script_type(&self, build_option: &AddressBuilderOption) -> AddressScriptType {
        match build_option {
            AddressBuilderOption::BuildAsPubkeyHash => AddressScriptType::P2PKH,
            AddressBuilderOption::BuildAsScriptHash => AddressScriptType::P2SH,
        }
    }

    fn get_segwit_script_type(&self, build_option: &AddressBuilderOption) -> AddressScriptType {
        match build_option {
            AddressBuilderOption::BuildAsPubkeyHash => AddressScriptType::P2WPKH,
            AddressBuilderOption::BuildAsScriptHash => AddressScriptType::P2WSH,
        }
    }

    fn check_segwit_hrp(&self) -> Result<(), String> {
        if self.hrp.is_none() {
            return Err("no hrp for address".to_owned());
        }
        Ok(())
    }

    fn check_segwit_hash(&self, build_option: &AddressBuilderOption) -> Result<(), String> {
        let is_hash_valid = match build_option {
            AddressBuilderOption::BuildAsPubkeyHash => self.hash.is_address_hash(),
            AddressBuilderOption::BuildAsScriptHash => self.hash.is_witness_script_hash(),
        };
        if !is_hash_valid {
            return Err("invalid hash for segwit address".to_owned());
        }
        Ok(())
    }
}
