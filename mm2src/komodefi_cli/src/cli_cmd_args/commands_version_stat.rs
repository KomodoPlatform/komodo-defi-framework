use clap::{Args, Subcommand};
use std::mem::take;

use crate::rpc_data::version_stat::{VStatStartCollectionRequest, VStatUpdateCollectionRequest,
                                    VersionStatAddNodeRequest, VersionStatRemoveNodeRequest};

#[derive(Subcommand)]
pub(crate) enum VersionStatCommands {
    #[command(
        short_flag = 'a',
        visible_aliases = ["add", "add-node-to-version-stat"],
        about = "Adds a Node's name, IP address and PeerID to a local database to track which version of MM2 it is running. \
                 Note: To allow collection of version stats, added nodes must open port 38890"
    )]
    AddNode(VersionStatAddNodeArgs),
    #[command(
        short_flag = 'r',
        visible_aliases = ["remove", "remove-node-from-version-stat"],
        about = "Removes a Node (by name) from the local database which tracks which version of MM2 it is running"
    )]
    RemoveNode(VersionStatRemoveNodeArgs),
    #[command(
        short_flag = 's',
        visible_aliases = ["start", "start-version-stat-collection"],
        about = "Initiates storing version statistics for nodes previously registered via the add-node command"
    )]
    StartCollect(VStatStartCollectionArgs),
    #[command(
        short_flag = 'S',
        visible_aliases = ["stop", "stop-version-stat-collection"],
        about = "Stops the collection of version stats at the end of the current loop interval"
    )]
    StopCollect,
    #[command(
        short_flag = 'u',
        visible_aliases = ["update", "update-version-stat-collection"],
        about = "Updates the polling interval for version stats collection. Note: the new interval \
                 will take effect after the current interval loop has completed."
    )]
    UpdateCollect(VStatUpdateCollectionArgs),
}

#[derive(Args)]
pub(crate) struct VersionStatAddNodeArgs {
    #[arg(
        long,
        short,
        help = "The name assigned to the node, arbitrary identifying string, such as \"seed_alpha\" or \"dragonhound_DEV\""
    )]
    name: String,
    #[arg(long, short, help = "The Node's IP address or domain names")]
    address: String,
    #[arg(
        long,
        short,
        help = "The Peer ID can be found in the MM2 log file after a connection has been initiated"
    )]
    peer_id: String,
}

impl From<&mut VersionStatAddNodeArgs> for VersionStatAddNodeRequest {
    fn from(value: &mut VersionStatAddNodeArgs) -> Self {
        VersionStatAddNodeRequest {
            name: take(&mut value.name),
            address: take(&mut value.address),
            peer_id: take(&mut value.peer_id),
        }
    }
}

#[derive(Args)]
pub(crate) struct VersionStatRemoveNodeArgs {
    #[arg(
        help = "The name assigned to the node, arbitrary identifying string, such as \"seed_alpha\" or \"dragonhound_DEV\""
    )]
    name: String,
}

impl From<&mut VersionStatRemoveNodeArgs> for VersionStatRemoveNodeRequest {
    fn from(value: &mut VersionStatRemoveNodeArgs) -> Self {
        VersionStatRemoveNodeRequest {
            name: take(&mut value.name),
        }
    }
}

#[derive(Args)]
pub(crate) struct VStatStartCollectionArgs {
    #[arg(help = "Polling rate (in seconds) to check node versions")]
    interval: f64,
}

type VStatUpdateCollectionArgs = VStatStartCollectionArgs;

impl From<&mut VStatStartCollectionArgs> for VStatStartCollectionRequest {
    fn from(value: &mut VStatStartCollectionArgs) -> Self {
        VStatStartCollectionRequest {
            interval: value.interval,
        }
    }
}

impl From<&mut VStatUpdateCollectionArgs> for VStatUpdateCollectionRequest {
    fn from(value: &mut VStatUpdateCollectionArgs) -> Self {
        VStatUpdateCollectionRequest {
            interval: value.interval,
        }
    }
}
