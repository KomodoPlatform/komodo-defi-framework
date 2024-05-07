use clap::{Args, Subcommand};
use rpc::v1::types::H256 as H256Json;
use std::mem::take;
use std::str::FromStr;

use mm2_rpc::data::legacy::{BanPubkeysRequest, UnbanPubkeysRequest as UnbanPubkeysRequestImpl};

use crate::rpc_data::utility::GetCurrentMtpRequest;
use crate::rpc_data::UnbanPubkeysRequest;

#[derive(Subcommand)]
pub(crate) enum UtilityCommands {
    #[command(
        visible_alias = "ban",
        about = "Bans the given pubkey ignoring its order matching messages and preventing its \
                     orders from displaying in the orderbook. \
                     Use the secp256k1 pubkey without prefix for this method input"
    )]
    BanPubkey(BanPubkeyArgs),
    #[command(
        visible_aliases = ["list", "ban-list", "list-banned"],
        about = "Returns a list of public keys of nodes that are banned from interacting with the node executing the method"
    )]
    ListBannedPubkeys,
    #[command(
        visible_alias = "unban",
        about = "Remove all currently banned pubkeys from ban list, or specific pubkeys"
    )]
    UnbanPubkeys(UnbanPubkeysArgs),
    #[command(
        visible_aliases = ["current-mtp", "mtp"],
        about = "Returns the Median Time Past (MTP) from electrum servers for UTXO coins"
    )]
    GetCurrentMtp(GetCurrentMtpArgs),
}

#[derive(Args)]
pub(crate) struct BanPubkeyArgs {
    #[arg(
        long,
        short,
        value_parser = H256Json::from_str,
        help = "Pubkey to ban"
    )]
    pubkey: H256Json,
    #[arg(long, short, help = "Reason of banning")]
    reason: String,
}

impl From<&mut BanPubkeyArgs> for BanPubkeysRequest {
    fn from(value: &mut BanPubkeyArgs) -> Self {
        BanPubkeysRequest {
            pubkey: take(&mut value.pubkey),
            reason: take(&mut value.reason),
        }
    }
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub(crate) struct UnbanPubkeysArgs {
    #[arg(
        long,
        short,
        group = "unban-pubkeys",
        default_value_t = false,
        help = "Whether to unban all pubkeys"
    )]
    pub(crate) all: bool,
    #[arg(
        long,
        short,
        group = "unban-pubkeys",
        value_parser = H256Json::from_str,
        help="Pubkey to unban"
    )]
    pub(crate) pubkey: Vec<H256Json>,
}

impl From<&mut UnbanPubkeysArgs> for UnbanPubkeysRequest {
    fn from(value: &mut UnbanPubkeysArgs) -> Self {
        UnbanPubkeysRequest {
            unban_by: if value.all {
                UnbanPubkeysRequestImpl::All
            } else {
                UnbanPubkeysRequestImpl::Few(take(&mut value.pubkey))
            },
        }
    }
}

#[derive(Args)]
pub(crate) struct GetCurrentMtpArgs {
    #[arg(help = "A compatible (UTXO) coin's ticker")]
    coin: String,
}

impl From<&mut GetCurrentMtpArgs> for GetCurrentMtpRequest {
    fn from(value: &mut GetCurrentMtpArgs) -> Self {
        GetCurrentMtpRequest {
            coin: take(&mut value.coin),
        }
    }
}
