use clap::Args;
use mm2_number::MmNumber;
use mm2_rpc::data::legacy::UpdateMakerOrderRequest;
use std::mem::take;
use uuid::Uuid;

use crate::cli_cmd_args::parse_mm_number;

#[derive(Args, Clone)]
pub(crate) struct UpdateMakerOrderArgs {
    #[arg(long, short, help = "Uuid of the order the user desires to update")]
    uuid: Uuid,
    #[arg(
        long,
        short,
        value_parser = parse_mm_number,
        help = "Price in rel the user is willing to receive per one unit of the base coin"
    )]
    price: Option<MmNumber>,
    #[command(flatten)]
    volume: UpdateMakerVolumeArg,
    #[arg(
        long,
        short,
        value_parser = parse_mm_number,
        help = "Minimum amount of base coin available for the order; it must be less or equal than the new volume; the following values must be greater than or equal to the min_trading_vol of the corresponding coin"
    )]
    min_volume: Option<MmNumber>,
    #[arg(
        long,
        visible_alias = "bc",
        help = "Number of required blockchain confirmations for base coin atomic swap transaction"
    )]
    base_confs: Option<u64>,
    #[arg(
        long,
        visible_alias = "bn",
        help = "Whether dPoW notarization is required for base coin atomic swap transaction"
    )]
    base_nota: Option<bool>,
    #[arg(
        long,
        visible_alias = "rc",
        help = "Number of required blockchain confirmations for rel coin atomic swap transaction"
    )]
    rel_confs: Option<u64>,
    #[arg(
        long,
        visible_alias = "rn",
        help = "Whether dPoW notarization is required for rel coin atomic swap transaction"
    )]
    rel_nota: Option<bool>,
}

#[derive(Args, Clone)]
#[group(required = false, multiple = false)]
struct UpdateMakerVolumeArg {
    #[arg(
        long,
        short = 'M',
        help = "Whether to use the entire coin balance for the order, taking 0.001 coins into reserve to account for fees",
        default_value_t = false
    )]
    max_volume: bool,
    #[arg(
        long,
        short = 'd',
        value_parser = parse_mm_number, help = "Volume added to or subtracted from the max_base_vol of the order to be updated, resulting in the new volume which is the maximum amount of base coin available for the order, ignored if max is true"
    )]
    volume_delta: Option<MmNumber>,
}

impl From<&mut UpdateMakerOrderArgs> for UpdateMakerOrderRequest {
    fn from(value: &mut UpdateMakerOrderArgs) -> Self {
        UpdateMakerOrderRequest {
            uuid: take(&mut value.uuid),
            new_price: value.price.take(),
            max: value.volume.max_volume.then_some(true),
            volume_delta: value.volume.volume_delta.take(),
            min_volume: value.min_volume.take(),
            base_confs: value.base_confs.take(),
            base_nota: value.base_nota.take(),
            rel_confs: value.rel_confs.take(),
            rel_nota: value.rel_nota.take(),
        }
    }
}
