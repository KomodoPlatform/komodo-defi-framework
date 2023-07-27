//! Contains rpc data layer structures that are not ready to become a part of the mm2_rpc::data module
//!
//! *Note: it's expected that the following data types will be moved to mm2_rpc::data when mm2 is refactored to be able to handle them*
//!

pub(crate) mod activation;
pub(crate) mod network;
pub(crate) mod swaps;
pub(crate) mod trade_preimage;
pub(crate) mod utility;
pub(crate) mod wallet;

pub(crate) use activation::{bch, eth, tendermint, CoinsToKickStartRequest, CoinsToKickstartResponse,
                            DisableCoinFailed, DisableCoinRequest, DisableCoinResponse, DisableCoinSuccess,
                            GetEnabledRequest, SetRequiredConfResponse, SetRequiredNotaResponse};
pub(crate) use network::{GetGossipMeshRequest, GetGossipMeshResponse, GetGossipPeerTopicsRequest,
                         GetGossipPeerTopicsResponse, GetGossipTopicPeersRequest, GetGossipTopicPeersResponse,
                         GetMyPeerIdRequest, GetMyPeerIdResponse, GetPeersInfoRequest, GetPeersInfoResponse,
                         GetRelayMeshRequest, GetRelayMeshResponse};
pub(crate) use swaps::{ActiveSwapsRequest, ActiveSwapsResponse, MakerNegotiationData, MakerSavedEvent, MakerSavedSwap,
                       MakerSwapData, MakerSwapEvent, MyRecentSwapResponse, MyRecentSwapsRequest, MySwapStatusRequest,
                       MySwapStatusResponse, Params, PaymentInstructions, RecoverFundsOfSwapRequest,
                       RecoverFundsOfSwapResponse, SavedSwap, SavedTradeFee, SwapError, TakerNegotiationData,
                       TakerPaymentSpentData, TakerSavedEvent, TakerSavedSwap, TakerSwapData, TakerSwapEvent,
                       TransactionIdentifier};

pub(crate) use trade_preimage::{MakerPreimage, MaxTakerVolRequest, MaxTakerVolResponse, MinTradingVolRequest,
                                TakerPreimage, TotalTradeFeeResponse, TradeFeeResponse, TradePreimageMethod,
                                TradePreimageRequest, TradePreimageResponse};
pub(crate) use utility::{BanReason, ListBannedPubkeysRequest, ListBannedPubkeysResponse, UnbanPubkeysRequest,
                         UnbanPubkeysResponse};
pub(crate) use wallet::{Bip44Chain, HDAccountAddressId, KmdRewardsDetails, SendRawTransactionRequest,
                        SendRawTransactionResponse, WithdrawFee, WithdrawFrom, WithdrawRequest, WithdrawResponse};
