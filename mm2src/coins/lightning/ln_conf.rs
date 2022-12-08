use crate::utxo::BlockchainNetwork;
use lightning::util::config::{ChannelConfig, ChannelHandshakeConfig, ChannelHandshakeLimits, UserConfig};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlatformCoinConfirmationTargets {
    pub background: u32,
    pub normal: u32,
    pub high_priority: u32,
}

#[derive(Clone, Debug)]
pub struct LightningProtocolConf {
    pub platform_coin_ticker: String,
    pub network: BlockchainNetwork,
    pub avg_block_time: u64,
    pub confirmation_targets: PlatformCoinConfirmationTargets,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChannelOptions {
    /// Amount (in millionths of a satoshi) charged per satoshi for payments forwarded outbound
    /// over the channel.
    pub proportional_fee_in_millionths_sats: Option<u32>,
    /// Amount (in milli-satoshi) charged for payments forwarded outbound over the channel, in
    /// excess of proportional_fee_in_millionths_sats.
    pub base_fee_msat: Option<u32>,
    pub cltv_expiry_delta: Option<u16>,
    /// Limit our total exposure to in-flight HTLCs which are burned to fees as they are too
    /// small to claim on-chain.
    pub max_dust_htlc_exposure_msat: Option<u64>,
    /// The additional fee we're willing to pay to avoid waiting for the counterparty's
    /// locktime to reclaim funds.
    pub force_close_avoidance_max_fee_sats: Option<u64>,
}

impl ChannelOptions {
    pub fn update_according_to(&mut self, options: ChannelOptions) {
        if let Some(fee) = options.proportional_fee_in_millionths_sats {
            self.proportional_fee_in_millionths_sats = Some(fee);
        }

        if let Some(fee) = options.base_fee_msat {
            self.base_fee_msat = Some(fee);
        }

        if let Some(expiry) = options.cltv_expiry_delta {
            self.cltv_expiry_delta = Some(expiry);
        }

        if let Some(dust) = options.max_dust_htlc_exposure_msat {
            self.max_dust_htlc_exposure_msat = Some(dust);
        }

        if let Some(fee) = options.force_close_avoidance_max_fee_sats {
            self.force_close_avoidance_max_fee_sats = Some(fee);
        }
    }
}

impl From<ChannelOptions> for ChannelConfig {
    fn from(options: ChannelOptions) -> Self {
        let mut channel_config = ChannelConfig::default();

        if let Some(fee) = options.proportional_fee_in_millionths_sats {
            channel_config.forwarding_fee_proportional_millionths = fee;
        }

        if let Some(fee) = options.base_fee_msat {
            channel_config.forwarding_fee_base_msat = fee;
        }

        if let Some(expiry) = options.cltv_expiry_delta {
            channel_config.cltv_expiry_delta = expiry;
        }

        if let Some(dust) = options.max_dust_htlc_exposure_msat {
            channel_config.max_dust_htlc_exposure_msat = dust;
        }

        if let Some(fee) = options.force_close_avoidance_max_fee_sats {
            channel_config.force_close_avoidance_max_fee_satoshis = fee;
        }

        channel_config
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct OurChannelsConfigs {
    /// Confirmations we will wait for before considering an inbound channel locked in.
    pub inbound_channels_confirmations: Option<u32>,
    /// The number of blocks we require our counterparty to wait to claim their money on chain
    /// if they broadcast a revoked transaction. We have to be online at least once during this time to
    /// punish our counterparty for broadcasting a revoked transaction.
    /// We have to account also for the time to broadcast and confirm our transaction,
    /// possibly with time in between to RBF (Replace-By-Fee) the spending transaction.
    pub counterparty_locktime: Option<u16>,
    /// The smallest value HTLC we will accept to process. The channel gets closed any time
    /// our counterparty misbehaves by sending us an HTLC with a value smaller than this.
    pub our_htlc_minimum_msat: Option<u64>,
    /// If set, we attempt to negotiate the `scid_privacy` (referred to as `scid_alias` in the
    /// BOLTs) option for outbound private channels. This provides better privacy by not including
    /// our real on-chain channel UTXO in each invoice and requiring that our counterparty only
    /// relay HTLCs to us using the channel's SCID alias.
    pub negotiate_scid_privacy: Option<bool>,
    /// Sets the percentage of the channel value we will cap the total value of outstanding inbound
    /// HTLCs to.
    pub max_inbound_in_flight_htlc_percent: Option<u8>,
    /// Set to announce the channel publicly and notify all nodes that they can route via this
    /// channel.
    pub announced_channel: Option<bool>,
    /// When set, we commit to an upfront shutdown_pubkey at channel open.
    pub commit_upfront_shutdown_pubkey: Option<bool>,
}

impl OurChannelsConfigs {
    pub fn update_according_to(&mut self, config: OurChannelsConfigs) {
        if let Some(confs) = config.inbound_channels_confirmations {
            self.inbound_channels_confirmations = Some(confs);
        }

        if let Some(delay) = config.counterparty_locktime {
            self.counterparty_locktime = Some(delay);
        }

        if let Some(min) = config.our_htlc_minimum_msat {
            self.our_htlc_minimum_msat = Some(min);
        }

        if let Some(scid_privacy) = config.negotiate_scid_privacy {
            self.negotiate_scid_privacy = Some(scid_privacy);
        }

        if let Some(max_inbound_htlc) = config.max_inbound_in_flight_htlc_percent {
            self.max_inbound_in_flight_htlc_percent = Some(max_inbound_htlc);
        }

        if let Some(announce) = config.announced_channel {
            self.announced_channel = Some(announce);
        }

        if let Some(commit) = config.commit_upfront_shutdown_pubkey {
            self.commit_upfront_shutdown_pubkey = Some(commit);
        }
    }
}

impl From<OurChannelsConfigs> for ChannelHandshakeConfig {
    fn from(config: OurChannelsConfigs) -> Self {
        let mut channel_handshake_config = ChannelHandshakeConfig::default();

        if let Some(confs) = config.inbound_channels_confirmations {
            channel_handshake_config.minimum_depth = confs;
        }

        if let Some(delay) = config.counterparty_locktime {
            channel_handshake_config.our_to_self_delay = delay;
        }

        if let Some(min) = config.our_htlc_minimum_msat {
            channel_handshake_config.our_htlc_minimum_msat = min;
        }

        if let Some(scid_privacy) = config.negotiate_scid_privacy {
            channel_handshake_config.negotiate_scid_privacy = scid_privacy;
        }

        if let Some(max_inbound_htlc) = config.max_inbound_in_flight_htlc_percent {
            channel_handshake_config.max_inbound_htlc_value_in_flight_percent_of_channel = max_inbound_htlc;
        }

        if let Some(announce) = config.announced_channel {
            channel_handshake_config.announced_channel = announce;
        }

        if let Some(commit) = config.commit_upfront_shutdown_pubkey {
            channel_handshake_config.commit_upfront_shutdown_pubkey = commit;
        }

        channel_handshake_config
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct CounterpartyLimits {
    /// Minimum allowed satoshis when an inbound channel is funded.
    pub min_funding_sats: Option<u64>,
    /// Maximum allowed satoshis when an inbound channel is funded.
    pub max_funding_sats: Option<u64>,
    /// The remote node sets a limit on the minimum size of HTLCs we can send to them. This allows
    /// us to limit the maximum minimum-size they can require.
    pub max_htlc_minimum_msat: Option<u64>,
    /// The remote node sets a limit on the maximum value of pending HTLCs to them at any given
    /// time to limit their funds exposure to HTLCs. This allows us to set a minimum such value.
    pub min_max_htlc_value_in_flight_msat: Option<u64>,
    /// The remote node will require us to keep a certain amount in direct payment to ourselves at all
    /// time, ensuring that we are able to be punished if we broadcast an old state. This allows us
    /// to limit the amount which we will have to keep to ourselves (and cannot use for HTLCs).
    pub max_channel_reserve_sats: Option<u64>,
    /// The remote node sets a limit on the maximum number of pending HTLCs to them at any given
    /// time. This allows us to set a minimum such value.
    pub min_max_accepted_htlcs: Option<u16>,
    /// This config allows us to set a limit on the maximum confirmations to wait before the outbound channel is usable.
    pub outbound_channels_confirmations: Option<u32>,
    /// Set to force an incoming channel to match our announced channel preference in ChannelOptions announced_channel.
    pub force_announced_channel_preference: Option<bool>,
    /// Set to the amount of time we're willing to wait to claim money back to us.
    pub our_locktime_limit: Option<u16>,
    /// When set an outbound channel can be used straight away without waiting for any on-chain confirmations.
    /// https://docs.rs/lightning/latest/lightning/util/config/struct.ChannelHandshakeLimits.html#structfield.trust_own_funding_0conf
    pub allow_outbound_0conf: Option<bool>,
}

impl From<CounterpartyLimits> for ChannelHandshakeLimits {
    fn from(limits: CounterpartyLimits) -> Self {
        let mut channel_handshake_limits = ChannelHandshakeLimits::default();

        if let Some(sats) = limits.min_funding_sats {
            channel_handshake_limits.min_funding_satoshis = sats;
        }

        if let Some(sats) = limits.max_funding_sats {
            channel_handshake_limits.max_funding_satoshis = sats;
        }

        if let Some(msat) = limits.max_htlc_minimum_msat {
            channel_handshake_limits.max_htlc_minimum_msat = msat;
        }

        if let Some(msat) = limits.min_max_htlc_value_in_flight_msat {
            channel_handshake_limits.min_max_htlc_value_in_flight_msat = msat;
        }

        if let Some(sats) = limits.max_channel_reserve_sats {
            channel_handshake_limits.max_channel_reserve_satoshis = sats;
        }

        if let Some(min) = limits.min_max_accepted_htlcs {
            channel_handshake_limits.min_max_accepted_htlcs = min;
        }

        if let Some(confs) = limits.outbound_channels_confirmations {
            channel_handshake_limits.max_minimum_depth = confs;
        }

        if let Some(is_0conf) = limits.allow_outbound_0conf {
            channel_handshake_limits.trust_own_funding_0conf = is_0conf;
        }

        if let Some(pref) = limits.force_announced_channel_preference {
            channel_handshake_limits.force_announced_channel_preference = pref;
        }

        if let Some(blocks) = limits.our_locktime_limit {
            channel_handshake_limits.their_to_self_delay = blocks;
        }

        channel_handshake_limits
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LightningCoinConf {
    #[serde(rename = "coin")]
    pub ticker: String,
    pub decimals: u8,
    pub accept_inbound_channels: Option<bool>,
    pub accept_forwards_to_priv_channels: Option<bool>,
    pub channel_options: Option<ChannelOptions>,
    pub our_channels_configs: Option<OurChannelsConfigs>,
    pub counterparty_channel_config_limits: Option<CounterpartyLimits>,
    pub sign_message_prefix: Option<String>,
}

impl From<LightningCoinConf> for UserConfig {
    fn from(conf: LightningCoinConf) -> Self {
        let mut user_config = UserConfig::default();
        if let Some(config) = conf.our_channels_configs {
            user_config.channel_handshake_config = config.into();
        }
        if let Some(limits) = conf.counterparty_channel_config_limits {
            user_config.channel_handshake_limits = limits.into();
        }
        if let Some(options) = conf.channel_options {
            user_config.channel_config = options.into();
        }
        if let Some(accept_forwards) = conf.accept_forwards_to_priv_channels {
            user_config.accept_forwards_to_priv_channels = accept_forwards;
        }
        if let Some(accept_inbound) = conf.accept_inbound_channels {
            user_config.accept_inbound_channels = accept_inbound;
        }
        // This allows OpenChannelRequest event to be fired
        user_config.manually_accept_inbound_channels = true;

        user_config
    }
}
