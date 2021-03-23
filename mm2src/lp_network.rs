/******************************************************************************
 * Copyright © 2014-2019 The SuperNET Developers.                             *
 *                                                                            *
 * See the AUTHORS, DEVELOPER-AGREEMENT and LICENSE files at                  *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * SuperNET software, including this file may be copied, modified, propagated *
 * or distributed except according to the terms contained in the LICENSE file *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  lp_network.rs
//  marketmaker
//
use common::executor::spawn;
use common::log;
use common::mm_ctx::MmArc;
use common::mm_metrics::{ClockOps, MetricsOps};
use futures::{channel::oneshot, lock::Mutex as AsyncMutex, StreamExt};
use mm2_libp2p::atomicdex_behaviour::{AdexBehaviourCmd, AdexBehaviourEvent, AdexCmdTx, AdexEventRx, AdexResponse,
                                      AdexResponseChannel};
use mm2_libp2p::{decode_message, encode_message, GossipsubMessage, MessageId, PeerId, TOPIC_SEPARATOR};
#[cfg(test)] use mocktopus::macros::*;
use serde::de;
use std::sync::Arc;

use crate::mm2::{lp_ordermatch, lp_swap};

#[derive(Eq, Debug, Deserialize, PartialEq, Serialize)]
pub enum P2PRequest {
    Ordermatch(lp_ordermatch::OrdermatchRequest),
}

pub struct P2PContext {
    /// Using Mutex helps to prevent cloning which can actually result to channel being unbounded in case of using 1 tx clone per 1 message.
    pub cmd_tx: AsyncMutex<AdexCmdTx>,
}

#[cfg_attr(test, mockable)]
impl P2PContext {
    pub fn new(cmd_tx: AdexCmdTx) -> Self {
        P2PContext {
            cmd_tx: AsyncMutex::new(cmd_tx),
        }
    }

    pub fn store_to_mm_arc(self, ctx: &MmArc) { *ctx.p2p_ctx.lock().unwrap() = Some(Arc::new(self)) }

    pub fn fetch_from_mm_arc(ctx: &MmArc) -> Arc<Self> {
        ctx.p2p_ctx
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
            .downcast()
            .unwrap()
    }
}

pub async fn p2p_event_process_loop(ctx: MmArc, mut rx: AdexEventRx, i_am_relay: bool) {
    while !ctx.is_stopping() {
        match rx.next().await {
            Some(AdexBehaviourEvent::Message(peer_id, message_id, message)) => {
                spawn(process_p2p_message(
                    ctx.clone(),
                    peer_id,
                    message_id,
                    message,
                    i_am_relay,
                ));
            },
            Some(AdexBehaviourEvent::PeerRequest {
                peer_id,
                request,
                response_channel,
            }) => {
                if let Err(e) = process_p2p_request(ctx.clone(), peer_id, request, response_channel).await {
                    log::error!("Error on process P2P request: {:?}", e);
                }
            },
            None => break,
            _ => (),
        }
    }
}

async fn process_p2p_message(
    ctx: MmArc,
    peer_id: PeerId,
    message_id: MessageId,
    message: GossipsubMessage,
    i_am_relay: bool,
) {
    let mut to_propagate = false;
    let mut orderbook_pairs = vec![];

    for topic in message.topics {
        let mut split = topic.as_str().split(TOPIC_SEPARATOR);
        match split.next() {
            Some(lp_ordermatch::ORDERBOOK_PREFIX) => {
                if let Some(pair) = split.next() {
                    orderbook_pairs.push(pair.to_string());
                }
            },
            Some(lp_swap::SWAP_PREFIX) => {
                lp_swap::process_msg(ctx.clone(), split.next().unwrap_or_default(), &message.data);
                to_propagate = true;
            },
            None | Some(_) => (),
        }
    }

    if !orderbook_pairs.is_empty() {
        let process_fut = lp_ordermatch::process_msg(
            ctx.clone(),
            orderbook_pairs,
            peer_id.to_string(),
            &message.data,
            i_am_relay,
        );
        if process_fut.await {
            to_propagate = true;
        }
    }

    if to_propagate && i_am_relay {
        propagate_message(&ctx, message_id, peer_id);
    }
}

async fn process_p2p_request(
    ctx: MmArc,
    _peer_id: PeerId,
    request: Vec<u8>,
    response_channel: AdexResponseChannel,
) -> Result<(), String> {
    let request = try_s!(decode_message::<P2PRequest>(&request));
    let result = match request {
        P2PRequest::Ordermatch(req) => lp_ordermatch::process_peer_request(ctx.clone(), req).await,
    };

    let res = match result {
        Ok(Some(response)) => AdexResponse::Ok { response },
        Ok(None) => AdexResponse::None,
        Err(e) => AdexResponse::Err { error: e },
    };

    let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd = AdexBehaviourCmd::SendResponse { res, response_channel };
    try_s!(p2p_ctx.cmd_tx.lock().await.try_send(cmd));
    Ok(())
}

pub fn broadcast_p2p_msg(ctx: &MmArc, topics: Vec<String>, msg: Vec<u8>) {
    let ctx = ctx.clone();
    spawn(async move {
        let cmd = AdexBehaviourCmd::PublishMsg { topics, msg };
        let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
        if let Err(e) = p2p_ctx.cmd_tx.lock().await.try_send(cmd) {
            log::error!("broadcast_p2p_msg cmd_tx.send error {:?}", e);
        };
    });
}

/// Subscribe to the given `topic`.
///
/// # Safety
///
/// The function locks the [`MmCtx::p2p_ctx`] mutext.
pub async fn subscribe_to_topic(ctx: &MmArc, topic: String) {
    let p2p_ctx = P2PContext::fetch_from_mm_arc(ctx);
    let cmd = AdexBehaviourCmd::Subscribe { topic };
    if let Err(e) = p2p_ctx.cmd_tx.lock().await.try_send(cmd) {
        log::error!("subscribe_to_topic cmd_tx.send error {:?}", e);
    };
}

pub async fn request_any_relay<T: de::DeserializeOwned>(
    ctx: MmArc,
    req: P2PRequest,
) -> Result<Option<(T, PeerId)>, String> {
    let encoded = try_s!(encode_message(&req));

    let (response_tx, response_rx) = oneshot::channel();
    let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd = AdexBehaviourCmd::RequestAnyRelay {
        req: encoded,
        response_tx,
    };
    try_s!(p2p_ctx.cmd_tx.lock().await.try_send(cmd));
    match try_s!(response_rx.await) {
        Some((from_peer, response)) => {
            let response = try_s!(decode_message::<T>(&response));
            Ok(Some((response, from_peer)))
        },
        None => Ok(None),
    }
}

pub enum PeerDecodedResponse<T> {
    Ok(T),
    None,
    Err(String),
}

#[allow(dead_code)]
pub async fn request_relays<T: de::DeserializeOwned>(
    ctx: MmArc,
    req: P2PRequest,
) -> Result<Vec<(PeerId, PeerDecodedResponse<T>)>, String> {
    let encoded = try_s!(encode_message(&req));

    let (response_tx, response_rx) = oneshot::channel();
    let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd = AdexBehaviourCmd::RequestRelays {
        req: encoded,
        response_tx,
    };
    try_s!(p2p_ctx.cmd_tx.lock().await.try_send(cmd));
    let responses = try_s!(response_rx.await);
    Ok(parse_peers_responses(responses))
}

pub async fn request_peers<T: de::DeserializeOwned>(
    ctx: MmArc,
    req: P2PRequest,
    peers: Vec<String>,
) -> Result<Vec<(PeerId, PeerDecodedResponse<T>)>, String> {
    let encoded = try_s!(encode_message(&req));

    let (response_tx, response_rx) = oneshot::channel();
    let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd = AdexBehaviourCmd::RequestPeers {
        req: encoded,
        peers,
        response_tx,
    };
    try_s!(p2p_ctx.cmd_tx.lock().await.try_send(cmd));
    let responses = try_s!(response_rx.await);
    Ok(parse_peers_responses(responses))
}

pub async fn request_one_peer<T: de::DeserializeOwned>(
    ctx: MmArc,
    req: P2PRequest,
    peer: String,
) -> Result<Option<T>, String> {
    let clock = ctx.metrics.clock().expect("Metrics clock is not available");
    let start = clock.now();
    let mut responses = try_s!(request_peers::<T>(ctx.clone(), req, vec![peer.clone()]).await);
    let end = clock.now();
    mm_timing!(ctx.metrics, "peer.outgoing_request.timing", start, end, "peer" => peer);
    if responses.len() != 1 {
        return ERR!("Expected 1 response, found {}", responses.len());
    }
    let (_, response) = responses.remove(0);
    match response {
        PeerDecodedResponse::Ok(response) => Ok(Some(response)),
        PeerDecodedResponse::None => Ok(None),
        PeerDecodedResponse::Err(e) => ERR!("{}", e),
    }
}

fn parse_peers_responses<T: de::DeserializeOwned>(
    responses: Vec<(PeerId, AdexResponse)>,
) -> Vec<(PeerId, PeerDecodedResponse<T>)> {
    responses
        .into_iter()
        .map(|(peer_id, res)| {
            let res = match res {
                AdexResponse::Ok { response } => match decode_message::<T>(&response) {
                    Ok(res) => PeerDecodedResponse::Ok(res),
                    Err(e) => PeerDecodedResponse::Err(ERRL!("{}", e)),
                },
                AdexResponse::None => PeerDecodedResponse::None,
                AdexResponse::Err { error } => PeerDecodedResponse::Err(error),
            };
            (peer_id, res)
        })
        .collect()
}

pub fn propagate_message(ctx: &MmArc, message_id: MessageId, propagation_source: PeerId) {
    let ctx = ctx.clone();
    spawn(async move {
        let p2p_ctx = P2PContext::fetch_from_mm_arc(&ctx);
        let cmd = AdexBehaviourCmd::PropagateMessage {
            message_id,
            propagation_source,
        };
        if let Err(e) = p2p_ctx.cmd_tx.lock().await.try_send(cmd) {
            log::error!("propagate_message cmd_tx.send error {:?}", e);
        };
    });
}
