
/******************************************************************************
 * Copyright © 2014-2018 The SuperNET Developers.                             *
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
//  ordermatch.rs
//  marketmaker
//
use common::{lp, lp_queue_command_for_c, free_c_ptr, c_char_to_string, sat_to_f, SATOSHIS, SMALLVAL, CJSON, dstr, rpc_response, rpc_err_response, HyRes};
use common::for_c::broadcast_p2p_msg_for_c;
use common::mm_ctx::{from_ctx, MmArc, MmWeak};
use coins::{lp_coinfind, MmCoinEnum};
use coins::utxo::{compressed_pub_key_from_priv_raw, ChecksumType};
use futures::future::Future;
use gstuff::now_ms;
use hashbrown::hash_map::{Entry, HashMap};
use libc::{self, c_void, c_char, strcpy, strlen, calloc};
use rand::{thread_rng, RngCore};
use serde_json::{self as json, Value as Json};
use std::collections::{VecDeque};
use std::ffi::{CString, CStr};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;

use crate::mm2::lp_swap::{MakerSwap, run_maker_swap, TakerSwap, run_taker_swap};

/// Temporary kludge, improving readability of the not-yet-fully-ported code. Should be removed eventually.
macro_rules! c2s {($cs: expr) => {unwrap!(CStr::from_ptr($cs.as_ptr()).to_str())}}

#[link="c"]
extern {
    fn printf(_: *const libc::c_char, ...) -> libc::c_int;
}
#[derive(Default, Clone, Copy)]
struct BobCompetition {
    pub alice_id: u64,
    pub best_price: f64,
    pub start_time: u64,
    pub counter: i32,
}

struct OrdermatchContext {
    pub lp_trades: Mutex<HashMap<u64, lp::LP_trade>>,
    lp_trades_queue: Mutex<VecDeque<lp::LP_trade>>,
    bob_competitions: Mutex<[BobCompetition; 512]>,
}

impl OrdermatchContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx (ctx: &MmArc) -> Result<Arc<OrdermatchContext>, String> {
        Ok (try_s! (from_ctx (&ctx.ordermatch_ctx, move || {
            Ok (OrdermatchContext {
                lp_trades: Mutex::new (HashMap::default()),
                lp_trades_queue: Mutex::new (VecDeque::new()),
                bob_competitions: Mutex::new ([BobCompetition::default(); 512]),
            })
        })))
    }

    /// Obtains a reference to this crate context, creating it if necessary.
    #[allow(dead_code)]
    fn from_ctx_weak (ctx_weak: &MmWeak) -> Result<Arc<OrdermatchContext>, String> {
        let ctx = try_s! (MmArc::from_weak (ctx_weak) .ok_or ("Context expired"));
        Self::from_ctx (&ctx)
    }

    fn add_trade_to_queue(&self, trade: lp::LP_trade) {
        let mut queue = unwrap!(self.lp_trades_queue.lock());
        queue.push_back(trade);
    }

    fn pop_trade_from_queue(&self) -> Option<lp::LP_trade> {
        let mut queue = unwrap!(self.lp_trades_queue.lock());
        queue.pop_front()
    }

    fn is_trade_queue_empty(&self) -> bool {
        let queue = unwrap!(self.lp_trades_queue.lock());
        queue.is_empty()
    }
}
/*
struct LP_quoteinfo LP_Alicequery,LP_Alicereserved;
double LP_Alicemaxprice;
bits256 LP_Alicedestpubkey,LP_bobs_reserved;
uint32_t Alice_expiration,Bob_expiration;
struct { uint64_t aliceid; double bestprice; uint32_t starttime,counter; } Bob_competition[512];


void LP_failedmsg(uint32_t requestid,uint32_t quoteid,double val,char *uuidstr)
{
    char *msg; cJSON *retjson;
    if ( IPC_ENDPOINT >= 0 )
    {
        retjson = cJSON_CreateObject();
        jaddstr(retjson,"method","failed");
        jaddstr(retjson,"uuid",uuidstr);
        jaddnum(retjson,"error",val);
        jaddnum(retjson,"requestid",requestid);
        jaddnum(retjson,"quoteid",quoteid);
        msg = jprint(retjson,1);
        LP_queuecommand(0,msg,IPC_ENDPOINT,-1,0);
        free(msg);
    }
}
*/
unsafe fn lp_bob_competition(ctx: &MmArc, counterp: *mut i32, aliceid: u64, price: f64, counter: i32) -> f64 {
    let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(ctx));
    let mut competitions = unwrap!(ordermatch_ctx.bob_competitions.lock());

    let mut firsti: Option<u32> = None;
    let now = now_ms() / 1000;
    *counterp = 0;
    for i in 0..competitions.len() {
        if competitions[i].alice_id == aliceid {
            if counter < 0 || now > competitions[i].start_time + lp::LP_AUTOTRADE_TIMEOUT as u64 {
//printf("aliceid.%llu expired\n",(long long)aliceid);
                competitions[i].best_price = 0.;
                competitions[i].start_time = now;
                competitions[i].counter = 0;
            }
            if price != 0. && competitions[i].best_price == 0. || price < competitions[i].best_price {
                competitions[i].best_price = price;
//printf("Bob competition aliceid.%llu <- bestprice %.8f\n",(long long)aliceid,price);
            }
            competitions[i].counter += counter;
            *counterp = competitions[i].counter;
            return competitions[i].best_price;
        } else if competitions[i].alice_id == 0 {
            firsti = Some(i as u32);
        }
    }
    if firsti == None {
        firsti = Some(lp::LP_rand() % competitions.len() as u32);
    }
    let firsti = firsti.unwrap();
    competitions[firsti as usize].start_time = now_ms() / 1000;
    competitions[firsti as usize].counter = counter;
    competitions[firsti as usize].alice_id = aliceid;
    competitions[firsti as usize].best_price = price;
    *counterp = counter;
//printf("Bob competition aliceid.%llu %.8f\n",(long long)aliceid,price);
    price
}
/*
uint64_t LP_txfeecalc(struct iguana_info *coin,uint64_t txfee,int32_t txlen)
{
    if ( coin != 0 )
    {
        if ( strcmp(coin->symbol,"BTC") == 0 )
        {
            if ( txlen == 0 )
                txlen = LP_AVETXSIZE;
            coin->rate = LP_getestimatedrate(coin);
            if ( (txfee= SATOSHIDEN * coin->rate * txlen) <= 20000 )
            {
                //coin->rate = -1.;
                coin->rate = _LP_getestimatedrate(coin);
                if ( (txfee= SATOSHIDEN * coin->rate * txlen) <= 20000 )
                    txfee = 20000;
            }
        } else txfee = coin->txfee;
        if ( txfee < LP_MIN_TXFEE )
            txfee = LP_MIN_TXFEE;
    }
    return(txfee);
}

int32_t LP_quote_checkmempool(struct LP_quoteinfo *qp,struct LP_utxoinfo *autxo,struct LP_utxoinfo *butxo)
{
    int32_t selector,spendvini; bits256 spendtxid;
    if ( butxo != 0 && (selector= LP_mempool_vinscan(&spendtxid,&spendvini,qp->srccoin,qp->coinaddr,qp->txid,qp->vout,qp->txid2,qp->vout2)) >= 0 )
    {
        char str[65]; printf("LP_tradecommand selector.%d in mempool %s vini.%d",selector,bits256_str(str,spendtxid),spendvini);
        return(-1);
    }
    if ( autxo != 0 && (selector= LP_mempool_vinscan(&spendtxid,&spendvini,qp->destcoin,qp->destaddr,qp->desttxid,qp->destvout,qp->feetxid,qp->feevout)) >= 0 )
    {
        char str[65]; printf("LP_tradecommand dest selector.%d in mempool %s vini.%d",selector,bits256_str(str,spendtxid),spendvini);
        return(-1);
    }
    return(0);
}

double LP_quote_validate(struct LP_utxoinfo *autxo,struct LP_utxoinfo *butxo,struct LP_quoteinfo *qp,int32_t iambob)
{
    double qprice=0.; char str[65],srccoin[65],destcoin[65],bobtomic[64],alicetomic[64]; cJSON *txout; uint64_t txfee,desttxfee,srcvalue=0,srcvalue2=0,destvalue=0,destvalue2=0;
    LP_etomicsymbol(srccoin,bobtomic,qp->srccoin);
    LP_etomicsymbol(destcoin,alicetomic,qp->destcoin);
  //printf(">>>>>>> quote satoshis.(%.8f %.8f) %s %.8f -> %s %.8f\n",dstr(qp->satoshis),dstr(qp->destsatoshis),qp->srccoin,dstr(qp->satoshis),qp->destcoin,dstr(qp->destsatoshis));
    if ( butxo != 0 )
    {
        if ( LP_iseligible(&srcvalue,&srcvalue2,1,srccoin,qp->txid,qp->vout,qp->satoshis,qp->txid2,qp->vout2) == 0 )
        {
            //printf("bob not eligible %s (%.8f %.8f)\n",jprint(LP_quotejson(qp),1),dstr(srcvalue),dstr(srcvalue2));
            return(-2);
        }
        if ( (txout= LP_gettxout(srccoin,qp->coinaddr,qp->txid,qp->vout)) != 0 )
            free_json(txout);
        else
        {
            printf("%s %s payment %s/v%d is spent\n",srccoin,qp->coinaddr,bits256_str(str,qp->txid),qp->vout);
            return(-21);
        }
        if ( (txout= LP_gettxout(srccoin,qp->coinaddr,qp->txid2,qp->vout2)) != 0 )
            free_json(txout);
        else
        {
            printf("%s %s deposit %s/v%d is spent\n",srccoin,qp->coinaddr,bits256_str(str,qp->txid2),qp->vout2);
            return(-22);
        }
        if ( bits256_cmp(butxo->deposit.txid,qp->txid2) != 0 || butxo->deposit.vout != qp->vout2 )
        {
            char str[65],str2[65]; printf("%s != %s v%d != %d\n",bits256_str(str,butxo->deposit.txid),bits256_str(str2,qp->txid2),butxo->deposit.vout,qp->vout2);
            return(-6);
        }
        if ( strcmp(butxo->coinaddr,qp->coinaddr) != 0 )
        {
            printf("(%s) != (%s)\n",butxo->coinaddr,qp->coinaddr);
            return(-7);
        }
    }
    if ( autxo != 0 )
    {
        if ( LP_iseligible(&destvalue,&destvalue2,0,destcoin,qp->desttxid,qp->destvout,qp->destsatoshis,qp->feetxid,qp->feevout) == 0 )
        {
            //alice not eligible 0.36893923 -> dest 0.55020000 1.49130251 (0.61732249 0.00104324) 14b8b74808d2d34a70e5eddd1cad47d855858f8b23cac802576d4d37b5f8af8f/v1 abec6e76169bcb738235ca67fab02cc55390f39e422aa71f1badf8747c290cc4/v1
            char str[65],str2[65]; printf("alice not eligible %.8f -> dest %.8f %.8f (%.8f %.8f) %s/v%d %s/v%d\n",dstr(qp->satoshis),dstr(qp->destsatoshis),(double)qp->destsatoshis/qp->satoshis,dstr(destvalue),dstr(destvalue2),bits256_str(str,qp->desttxid),qp->destvout,bits256_str(str2,qp->feetxid),qp->feevout);
            return(-3);
        }
        if ( (txout= LP_gettxout(destcoin,qp->destaddr,qp->desttxid,qp->destvout)) != 0 )
            free_json(txout);
        else
        {
            printf("%s %s Apayment %s/v%d is spent\n",destcoin,qp->destaddr,bits256_str(str,qp->desttxid),qp->destvout);
            return(-23);
        }
        if ( (txout= LP_gettxout(destcoin,qp->destaddr,qp->feetxid,qp->feevout)) != 0 )
            free_json(txout);
        else
        {
            printf("%s %s dexfee %s/v%d is spent\n",destcoin,qp->destaddr,bits256_str(str,qp->feetxid),qp->feevout);
            return(-24);
        }
    }
    //printf("checked autxo and butxo\n");
    if ( LP_quote_checkmempool(qp,autxo,butxo) < 0 )
        return(-4);
    if ( iambob == 0 && autxo != 0 )
    {
        if ( bits256_cmp(autxo->fee.txid,qp->feetxid) != 0 || autxo->fee.vout != qp->feevout )
            return(-9);
        if ( strcmp(autxo->coinaddr,qp->destaddr) != 0 )
            return(-10);
    }
    if ( strcmp(destcoin, "ETOMIC") != 0 && autxo != 0 && destvalue < qp->desttxfee+qp->destsatoshis )
    {
        printf("destvalue %.8f  destsatoshis %.8f is too small txfee %.8f?\n",dstr(destvalue),dstr(qp->destsatoshis),dstr(qp->desttxfee));
        return(-11);
    }
    if ( strcmp(srccoin, "ETOMIC") != 0 && butxo != 0 && srcvalue < qp->txfee+qp->satoshis )
    {
        printf("srcvalue %.8f [%.8f] satoshis %.8f is too small txfee %.8f?\n",dstr(srcvalue),dstr(srcvalue) - dstr(qp->txfee+qp->satoshis),dstr(qp->satoshis),dstr(qp->txfee));
        return(-33);
    }
    LP_txfees(&txfee,&desttxfee,qp->srccoin,qp->destcoin);
    if ( txfee < qp->txfee )
        txfee = qp->txfee;
    if ( desttxfee < qp->desttxfee )
        desttxfee = qp->desttxfee;
    if ( qp->satoshis != 0 )
        qprice = ((double)qp->destsatoshis / (qp->satoshis-qp->txfee));
    //printf("qprice %.8f <- %.8f/%.8f txfees.(%.8f %.8f) vs (%.8f %.8f)\n",qprice,dstr(qp->destsatoshis),dstr(qp->satoshis),dstr(qp->txfee),dstr(qp->desttxfee),dstr(txfee),dstr(desttxfee));
    if ( qp->txfee < LP_REQUIRED_TXFEE*txfee || qp->desttxfee < LP_REQUIRED_TXFEE*desttxfee )
    {
        printf("error -14: txfee %.8f < %.8f or desttxfee %.8f < %.8f\n",dstr(qp->txfee),dstr(LP_REQUIRED_TXFEE*txfee),dstr(qp->desttxfee),dstr(LP_REQUIRED_TXFEE*desttxfee));
        return(-14);
    }
    if ( butxo != 0 && strcmp(srccoin, "ETOMIC") != 0)
    {
        if ( qp->satoshis < (srcvalue / LP_MINVOL) || srcvalue < qp->txfee*LP_MINSIZE_TXFEEMULT )
        {
            printf("utxo payment %.8f is less than %f covered by Q %.8f or <10x txfee %.8f [%d %d]\n",dstr(srcvalue),1./LP_MINVOL,dstr(qp->satoshis),dstr(qp->txfee),qp->satoshis < (srcvalue / LP_MINVOL),srcvalue < qp->txfee*LP_MINSIZE_TXFEEMULT);
            return(-12);
        }
    }
    if ( autxo != 0 )
    {
        if ( qp->destsatoshis < (destvalue / LP_MINCLIENTVOL) || destvalue < qp->desttxfee*LP_MINSIZE_TXFEEMULT )
        {
            printf("destsatoshis %.8f is less than %f of value %.8f or < 10x txfee %.8f\n",dstr(qp->destsatoshis),1./LP_MINCLIENTVOL,dstr(destvalue),dstr(qp->desttxfee));
            return(-13);
        }
    }
    return(qprice);
}

int32_t LP_arrayfind(cJSON *array,bits256 txid,int32_t vout)
{
    int32_t i,n = cJSON_GetArraySize(array); cJSON *item;
    for (i=0; i<n; i++)
    {
        item = jitem(array,i);
        if ( vout == jint(item,"vout") && bits256_cmp(txid,jbits256(item,"txid")) == 0 )
            return(i);
    }
    return(-1);
}

int32_t LP_nanobind(void *ctx,char *pairstr)
{
    int32_t i,r,pairsock = -1; uint16_t mypullport; char bindaddr[128];
    if ( LP_canbind != 0 )
    {
        if ( (pairsock= nn_socket(AF_SP,NN_PAIR)) < 0 )
            printf("error creating utxo->pair\n");
        else
        {
            for (i=0; i<10000; i++)
            {
                r = (10000 + (LP_rand() % 50000)) & 0xffff;
                if ( LP_fixed_pairport != 0 )
                    r = LP_fixed_pairport;
                nanomsg_transportname(0,pairstr,LP_myipaddr,r);
                nanomsg_transportname(1,bindaddr,LP_myipaddr,r);
                if ( nn_bind(pairsock,bindaddr) >= 0 )
                {
                    //timeout = 1;
                    //nn_setsockopt(pairsock,NN_SOL_SOCKET,NN_SNDTIMEO,&timeout,sizeof(timeout));
                    //nn_setsockopt(pairsock,NN_SOL_SOCKET,NN_RCVTIMEO,&timeout,sizeof(timeout));
                    //printf("nanobind %s to %d\n",pairstr,pairsock);
                    return(pairsock);
                } // else printf("error binding to %s for %s\n",bindaddr,pairstr);
                if ( LP_fixed_pairport != 0 )
                    break;
            }
            printf("%d ports all used\n",i);
            nn_close(pairsock);
            pairsock = -1;
        }
    } else pairsock = LP_initpublicaddr(ctx,&mypullport,pairstr,"127.0.0.1",0,1);
    return(pairsock);
}

void LP_abutxo_set(struct LP_utxoinfo *autxo,struct LP_utxoinfo *butxo,struct LP_quoteinfo *qp)
{
    if ( butxo != 0 )
    {
        memset(butxo,0,sizeof(*butxo));
        butxo->pubkey = qp->srchash;
        safecopy(butxo->coin,qp->srccoin,sizeof(butxo->coin));
        safecopy(butxo->coinaddr,qp->coinaddr,sizeof(butxo->coinaddr));
        butxo->payment.txid = qp->txid;
        butxo->payment.vout = qp->vout;
        //butxo->payment.value = qp->value;
        butxo->iambob = 1;
        butxo->deposit.txid = qp->txid2;
        butxo->deposit.vout = qp->vout2;
        //butxo->deposit.value = up2->U.value;
        butxo->swap_satoshis = qp->satoshis;
    }
    if ( autxo != 0 )
    {
        memset(autxo,0,sizeof(*autxo));
        autxo->pubkey = qp->desthash;
        safecopy(autxo->coin,qp->destcoin,sizeof(autxo->coin));
        safecopy(autxo->coinaddr,qp->destaddr,sizeof(autxo->coinaddr));
        autxo->payment.txid = qp->desttxid;
        autxo->payment.vout = qp->destvout;
        //autxo->payment.value = qp->value;
        autxo->iambob = 0;
        autxo->fee.txid = qp->feetxid;
        autxo->fee.vout = qp->feevout;
        //autxo->deposit.value = up2->U.value;
        autxo->swap_satoshis = qp->destsatoshis;
    }
}
*/
fn lp_base_satoshis(
    relvolume: f64,
    price: f64,
    desttxfee: u64,
) -> u64 {
    //printf("basesatoshis %.8f (rel %.8f / price %.8f)\n",dstr(SATOSHIDEN * ((relvolume) / price) + 2*txfee),relvolume,price);
    if relvolume > desttxfee as f64 / 100000000.0 && price > 1e-15f64 {
        (100000000.0 * (relvolume / price)) as u64
    } else {
        0
    }
}

unsafe fn lp_connect_start_bob(ctx: &MmArc, base: *mut c_char, rel: *mut c_char, qp: *mut lp::LP_quoteinfo) -> i32 {
    let dex_selector = 0;
    let mut retval: i32 = -1;
    let mut pair_str: [c_char; 512] = [0; 512];
    (*qp).quotetime = (now_ms() / 1000) as u32;

    if lp::G.LP_mypub25519 == (*qp).srchash {
        lp::LP_requestinit(&mut (*qp).R, (*qp).srchash, (*qp).desthash, base, (*qp).satoshis, rel, (*qp).destsatoshis, (*qp).timestamp, (*qp).quotetime, dex_selector, (*qp).fill as i32, (*qp).gtc as i32);
        let loop_thread = thread::Builder::new().name("maker_loop".into()).spawn({
            let taker_str = unwrap!(CStr::from_ptr(rel).to_str());
            let taker_coin = unwrap!(unwrap! (lp_coinfind (ctx, taker_str)));
            let maker_str = unwrap!(CStr::from_ptr(base).to_str());
            let maker_coin = unwrap!(unwrap! (lp_coinfind (ctx, maker_str)));
            let ctx = ctx.clone();
            let alice = (*qp).desthash;
            let maker_amount = (*qp).R.srcamount as u64;
            let taker_amount = (*qp).R.destamount as u64;
            let my_persistent_pub = unwrap!(compressed_pub_key_from_priv_raw(&lp::G.LP_privkey.bytes, ChecksumType::DSHA256));
            let uuid = CStr::from_ptr((*qp).uuidstr.as_ptr()).to_string_lossy().into_owned();
            move || {
                log!("Entering the maker_swap_loop " (maker_coin.ticker()) "/" (taker_coin.ticker()));
                let maker_swap = MakerSwap::new(
                    ctx,
                    alice,
                    maker_coin,
                    taker_coin,
                    maker_amount,
                    taker_amount,
                    my_persistent_pub,
                    uuid,
                );
                run_maker_swap(maker_swap);
            }
        });
        match loop_thread {
            Ok(_h) => {
                let req_json = lp::LP_quotejson(qp);
                lp::LP_swapsfp_update((*qp).R.requestid, (*qp).R.quoteid);
                lp::jaddstr(req_json, b"method\x00".as_ptr() as *mut c_char, b"connected\x00".as_ptr() as *mut c_char);
                lp::jaddstr(req_json, b"pair\x00".as_ptr() as *mut c_char, pair_str.as_mut_ptr());
                broadcast_p2p_msg_for_c((*qp).desthash, lp::jprint(req_json, 0), unwrap!(ctx.ffi_handle()));
                thread::sleep(Duration::from_secs(1));
                printf(b"send CONNECT for %u-%u\n\x00".as_ptr() as *const c_char, (*qp).R.requestid, (*qp).R.quoteid);
                // broadcast_p2p_msg(zero, lp::jprint(req_json, 0));
                if lp::IPC_ENDPOINT >= 0 {
                    lp_queue_command_for_c(null_mut(), lp::jprint(req_json, 0), lp::IPC_ENDPOINT, -1, 0);
                }
                if (*qp).mpnet != 0 && (*qp).gtc == 0 {
                    let msg = lp::jprint(req_json, 0);
                    lp::LP_mpnet_send(0, msg, 1, (*qp).destaddr.as_mut_ptr());
                    free_c_ptr(msg as *mut c_void);
                }
                lp::free_json(req_json);
                retval = 0;
            },
            Err(e) => {
                log!({ "Got error launching bob swap loop: {}", e });
                lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, -3002.0, (*qp).uuidstr.as_mut_ptr());
            }
        }
    } else {
        lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, -3004.0, (*qp).uuidstr.as_mut_ptr());
        log!("lp::G.LP_mypub25519 " (lp::G.LP_mypub25519) " != (*qp).srchash " ((*qp).srchash));
    }
    retval
}
/*
void LP_gtc_iteration(void *ctx,char *myipaddr,int32_t mypubsock)
{
    struct LP_gtcorder *gtc,*tmp; struct LP_quoteinfo *qp; uint64_t destvalue,destvalue2; uint32_t oldest = 0;
    if ( Alice_expiration != 0 )
        return;
    DL_FOREACH_SAFE(GTCorders,gtc,tmp)
    {
        qp = &gtc->Q;
        if ( gtc->cancelled == 0 && (oldest == 0 || gtc->pending < oldest) )
            oldest = gtc->pending;
    }
    DL_FOREACH_SAFE(GTCorders,gtc,tmp)
    {
        qp = &gtc->Q;
        if ( gtc->cancelled == 0 && LP_iseligible(&destvalue,&destvalue2,0,qp->destcoin,qp->desttxid,qp->destvout,qp->destsatoshis,qp->feetxid,qp->feevout) == 0 )
        {
            gtc->cancelled = (uint32_t)time(NULL);
            LP_failedmsg(qp->R.requestid,qp->R.quoteid,-9997,qp->uuidstr);
        }
        if ( gtc->cancelled != 0 )
        {
            portable_mutex_lock(&LP_gtcmutex);
            DL_DELETE(GTCorders,gtc);
            free(gtc);
            portable_mutex_unlock(&LP_gtcmutex);
        }
        else
        {
            if ( gtc->pending <= oldest+60 && time(NULL) > gtc->pending+LP_AUTOTRADE_TIMEOUT*10 )
            {
                gtc->pending = qp->timestamp = (uint32_t)time(NULL);
                LP_query(ctx,myipaddr,mypubsock,"request",qp);
                LP_Alicequery = *qp, LP_Alicemaxprice = gtc->Q.maxprice, Alice_expiration = qp->timestamp + 2*LP_AUTOTRADE_TIMEOUT, LP_Alicedestpubkey = qp->srchash;
                char str[65]; printf("LP_gtc fill.%d gtc.%d %s/%s %.8f vol %.8f dest.(%s) maxprice %.8f etomicdest.(%s) uuid.%s fill.%d gtc.%d\n",qp->fill,qp->gtc,qp->srccoin,qp->destcoin,dstr(qp->satoshis),dstr(qp->destsatoshis),bits256_str(str,LP_Alicedestpubkey),gtc->Q.maxprice,qp->etomicdest,qp->uuidstr,qp->fill,qp->gtc);
                break;
            }
        }
    }
}
*/

lazy_static! {static ref GTC_LOCK: Mutex<()> = Mutex::new(());}

unsafe fn lp_gtc_addorder(qp: *mut lp::LP_quoteinfo) -> () {
    let _lock = unwrap!(GTC_LOCK.lock());
    let gtc = calloc(
        1usize,
        ::std::mem::size_of::<lp::LP_gtcorder>() as usize,
    ) as *mut lp::LP_gtcorder;
    (*gtc).Q = *qp;
    (*gtc).pending = (now_ms() / 1000) as u32;
    if !lp::GTCorders.is_null() {
        (*gtc).prev = (*lp::GTCorders).prev;
        (*(*lp::GTCorders).prev).next = gtc;
        (*lp::GTCorders).prev = gtc;
        (*gtc).next = null_mut();
    } else {
        lp::GTCorders = gtc;
        (*lp::GTCorders).prev = lp::GTCorders;
        (*lp::GTCorders).next = null_mut();
    }
}

fn lp_trade(
    qp: *mut lp::LP_quoteinfo,
    price: f64,
    timeout: i32,
    trade_id: u32,
    dest_pub_key: lp::bits256,
    uuid: *mut c_char,
    ctx: &MmArc,
) -> Result<String, String> {
    unsafe {
        (*qp).aliceid = lp::LP_rand() as u64;
        (*qp).tradeid = if trade_id > 0 {
            trade_id
        } else {
            lp::LP_rand()
        };
        (*qp).srchash = dest_pub_key;
        strcpy((*qp).uuidstr.as_ptr() as *mut c_char, uuid);
        (*qp).maxprice = price;
        (*qp).timestamp = (now_ms()  / 1000) as u32;
        if (*qp).gtc != 0 {
            let uuid_len = strlen((*qp).uuidstr.as_ptr());
            strcpy(
                (*qp).uuidstr[uuid_len - 6..].as_ptr() as *mut c_char,
                b"cccccc\x00".as_ptr() as *const c_char
            );
            lp_gtc_addorder(qp);
        }
        // TODO: discuss if LP_query should run in case of gtc order as LP_gtciteration will run it anyway
        lp::LP_query(b"request\x00".as_ptr() as *mut c_char, qp, unwrap!(ctx.ffi_handle()));
        lp::LP_Alicequery = *qp;
        lp::LP_Alicemaxprice = (*qp).maxprice;
        log!({"lp_trade] Alice max price: {}", lp::LP_Alicemaxprice});
        lp::Alice_expiration = (*qp).timestamp + timeout as u32;
        lp::LP_Alicedestpubkey = (*qp).srchash;
        if (*qp).gtc == 0 {
            let msg = lp::jprint(lp::LP_quotejson(qp), 1);
            lp::LP_mpnet_send(1, msg, 1, null_mut());
            free_c_ptr(msg as *mut c_void);
        }
        Ok(try_s!(c_char_to_string(lp::LP_recent_swaps(0, uuid))))
    }
}
/*
int32_t LP_quotecmp(int32_t strictflag,struct LP_quoteinfo *qp,struct LP_quoteinfo *qp2)
{
    if ( lp::bits256_nonz(LP_Alicedestpubkey) != 0 )
    {
        if (bits256_cmp(LP_Alicedestpubkey,qp->srchash) != 0 )
        {
            printf("reject quote from non-matching pubkey\n");
            return(-1);
        } else printf("dont reject quote from destpubkey\n");
    }
    if ( bits256_cmp(qp->desthash,qp2->desthash) == 0 && strcmp(qp->srccoin,qp2->srccoin) == 0 && strcmp(qp->destcoin,qp2->destcoin) == 0 && bits256_cmp(qp->desttxid,qp2->desttxid) == 0 && qp->destvout == qp2->destvout && bits256_cmp(qp->feetxid,qp2->feetxid) == 0 && qp->feevout == qp2->feevout && qp->destsatoshis == qp2->destsatoshis && qp->txfee >= qp2->txfee && qp->desttxfee == qp2->desttxfee )
    {
        if ( strictflag == 0 || (qp->aliceid == qp2->aliceid && qp->R.requestid == qp2->R.requestid && qp->R.quoteid == qp2->R.quoteid && qp->vout == qp2->vout && qp->vout2 == qp2->vout2 && qp->satoshis == qp2->satoshis && bits256_cmp(qp->txid,qp2->txid) == 0 && bits256_cmp(qp->txid2,qp2->txid2) == 0 && bits256_cmp(qp->srchash,qp2->srchash) == 0) )
            return(0);
        else printf("strict compare failure\n");
    }
    return(-1);
}

void LP_alicequery_clear()
{
    memset(&LP_Alicequery,0,sizeof(LP_Alicequery));
    memset(&LP_Alicedestpubkey,0,sizeof(LP_Alicedestpubkey));
    LP_Alicemaxprice = 0.;
    Alice_expiration = 0;
}

int32_t LP_alice_eligible(uint32_t quotetime)
{
    if ( Alice_expiration != 0 && quotetime > Alice_expiration )
    {
        if ( LP_Alicequery.uuidstr[0] != 0 )
            LP_failedmsg(LP_Alicequery.R.requestid,LP_Alicequery.R.quoteid,-9999,LP_Alicequery.uuidstr);
        printf("time expired for Alice_request\n");
        LP_alicequery_clear();
    }
    return(Alice_expiration == 0 || time(NULL) < Alice_expiration);
}

char *LP_cancel_order(char *uuidstr)
{
    int32_t num = 0; cJSON *retjson;
    if ( uuidstr != 0 )
    {
        if ( uuidstr[0] == 'G' )
        {
            struct LP_gtcorder *gtc,*tmp;
            DL_FOREACH_SAFE(GTCorders,gtc,tmp)
            {
                if ( strcmp(gtc->Q.uuidstr,uuidstr) == 0 )
                {
                    retjson = cJSON_CreateObject();
                    jaddstr(retjson,"result","success");
                    jaddstr(retjson,"cancelled",uuidstr);
                    jaddnum(retjson,"pending",gtc->pending);
                    if ( gtc->cancelled == 0 )
                    {
                        gtc->cancelled = (uint32_t)time(NULL);
                        jaddstr(retjson,"status","uuid canceled");
                        LP_failedmsg(gtc->Q.R.requestid,gtc->Q.R.quoteid,-9997,gtc->Q.uuidstr);
                    }
                    else
                    {
                        jaddstr(retjson,"status","uuid already canceled");
                        LP_failedmsg(gtc->Q.R.requestid,gtc->Q.R.quoteid,-9996,gtc->Q.uuidstr);
                    }
                }
            }
            return(clonestr("{\"error\":\"gtc uuid not found\"}"));
        }
        else
        {
            num = LP_trades_canceluuid(uuidstr);
            retjson = cJSON_CreateObject();
            jaddstr(retjson,"result","success");
            jaddnum(retjson,"numentries",num);
            if ( strcmp(LP_Alicequery.uuidstr,uuidstr) == 0 )
            {
                LP_failedmsg(LP_Alicequery.R.requestid,LP_Alicequery.R.quoteid,-9998,LP_Alicequery.uuidstr);
                LP_alicequery_clear();
                jaddstr(retjson,"status","uuid canceled");
            } else jaddstr(retjson,"status","will stop trade negotiation, but if swap started it wont cancel");
        }
        return(jprint(retjson,1));
    }
    return(clonestr("{\"error\":\"uuid not cancellable\"}"));
}
*/
unsafe fn lp_connected_alice(ctx_ffi_handle: u32, qp: *mut lp::LP_quoteinfo, pairstr: *mut c_char) { // alice
    if (*qp).desthash != lp::G.LP_mypub25519 {
        lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"error1\x00".as_ptr() as *mut c_char, 0, 0);
        lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, -4000.0, (*qp).uuidstr.as_mut_ptr());
        return;
    }
    let ctx = unwrap!(MmArc::from_ffi_handle(ctx_ffi_handle));
    printf("CONNECTED mpnet.%d fill.%d gtc.%d numpending.%d tradeid.%u requestid.%u quoteid.%u pairstr.%s\n\x00".as_ptr() as *const c_char,
           (*qp).mpnet, (*qp).fill, (*qp).gtc, lp::G.LP_pendingswaps, (*qp).tradeid, (*qp).R.requestid, (*qp).R.quoteid, pairstr);
    let dex_selector = 0;
    lp::LP_requestinit(&mut (*qp).R, (*qp).srchash, (*qp).desthash, (*qp).srccoin.as_mut_ptr(), (*qp).satoshis, (*qp).destcoin.as_mut_ptr(), (*qp).destsatoshis, (*qp).timestamp, (*qp).quotetime, dex_selector, (*qp).fill as i32, (*qp).gtc as i32);
//printf("calculated requestid.%u quoteid.%u\n",qp->R.requestid,qp->R.quoteid);
    let mut changed = 0;
    if lp::LP_Alicequery.srccoin[0] != 0 && lp::LP_Alicequery.destcoin[0] != 0 {
        lp::LP_mypriceset(0, &mut changed, lp::LP_Alicequery.destcoin.as_mut_ptr(), lp::LP_Alicequery.srccoin.as_mut_ptr(), 0.);
        lp::LP_mypriceset(0, &mut changed, lp::LP_Alicequery.srccoin.as_mut_ptr(), lp::LP_Alicequery.destcoin.as_mut_ptr(), 0.);
    }
    lp::LP_alicequery_clear();
    lp::LP_Alicereserved = lp::LP_quoteinfo::default();
    lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"connected\x00".as_ptr() as *mut c_char, (*qp).R.requestid, (*qp).R.quoteid);
    /*
    let qprice = lp::LP_quote_validate(&mut autxo, &mut butxo, qp, 0);
    if qprice <= SMALLVAL {
        lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"error4\x00".as_ptr() as *mut c_char, 0, 0);
        lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, qprice, (*qp).uuidstr.as_mut_ptr());
        printf(b"quote %s/%s validate error %.0f\n\x00".as_ptr() as *const c_char, (*qp).srccoin, (*qp).destcoin, qprice);
        return;
    }
    */
    let mut bid = 0.;
    let mut ask = 0.;
    if lp::LP_myprice(0, &mut bid, &mut ask, (*qp).srccoin.as_mut_ptr(), (*qp).destcoin.as_mut_ptr()) <= SMALLVAL || bid <= SMALLVAL {
        printf(b"this node has no price for %s/%s (%.8f %.8f)\n\x00".as_ptr() as *const c_char, (*qp).destcoin.as_ptr(), (*qp).srccoin.as_ptr(), bid, ask);
        lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"error5\x00".as_ptr() as *mut c_char, 0, 0);
        lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, -4002.0, (*qp).uuidstr.as_mut_ptr());
        return;
    }
    printf(b"%s/%s bid %.8f ask %.8f\n\x00".as_ptr() as *const c_char, (*qp).srccoin.as_ptr(), (*qp).destcoin.as_ptr(), bid, ask);
    lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"started\x00".as_ptr() as *mut c_char, (*qp).R.requestid, (*qp).R.quoteid);
    printf(b"alice pairstr.(%s)\n\x00".as_ptr() as *const c_char, pairstr);
    let alice_loop_thread = thread::Builder::new().name("taker_loop".into()).spawn({
        let ctx = ctx.clone();
        let maker = (*qp).srchash;
        let taker_str = c2s!((*qp).R.dest);
        let taker_coin = unwrap!(unwrap! (lp_coinfind (&ctx, taker_str)));
        let maker_str = c2s!((*qp).R.src);
        let maker_coin = unwrap!(unwrap! (lp_coinfind (&ctx, maker_str)));
        let maker_amount = (*qp).R.srcamount as u64;
        let taker_amount = (*qp).R.destamount as u64;
        let my_persistent_pub = unwrap!(compressed_pub_key_from_priv_raw(&lp::G.LP_privkey.bytes, ChecksumType::DSHA256));
        let uuid = CStr::from_ptr((*qp).uuidstr.as_ptr()).to_string_lossy().into_owned();
        move || {
            log!("Entering the taker_swap_loop " (maker_coin.ticker()) "/" (taker_coin.ticker()));
            let taker_swap = TakerSwap::new(
                ctx,
                maker,
                maker_coin,
                taker_coin,
                maker_amount,
                taker_amount,
                my_persistent_pub,
                uuid,
            );
            run_taker_swap(taker_swap);
        }
    });
    match alice_loop_thread {
        Ok(_h) => {
            let retjson = CJSON(lp::LP_quotejson(qp));
            lp::jaddstr(retjson.0, b"result\x00".as_ptr() as *mut c_char, b"success\x00".as_ptr() as *mut c_char);
            lp::LP_swapsfp_update((*qp).R.requestid, (*qp).R.quoteid);
            if lp::IPC_ENDPOINT >= 0 {
                let msg = lp::jprint(retjson.0, 0);
                lp_queue_command_for_c(null_mut(), msg, lp::IPC_ENDPOINT, -1, 0);
                free_c_ptr(msg as *mut c_void);
            }
        },
        Err(e) => {
            log!({ "Got error trying to start taker loop {}", e });
            lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"error9\x00".as_ptr() as *mut c_char, (*qp).R.requestid, (*qp).R.quoteid);
            lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, -4006.0, (*qp).uuidstr.as_mut_ptr());
        }
    }
}
/*
int32_t LP_aliceonly(char *symbol)
{
    return(0);
}

int32_t LP_validSPV(char *symbol,char *coinaddr,bits256 txid,int32_t vout)
{
    struct electrum_info *ep,*backupep; cJSON *txobj; struct LP_address_utxo *up; struct iguana_info *coin; int32_t height; struct LP_transaction *tx;
    coin = LP_coinfind(symbol);
    if ( coin != 0 && (ep= coin->electrum) != 0 )
    {
        if ( (up= LP_address_utxofind(coin,coinaddr,txid,vout)) == 0 )
        {
            if ( (txobj= electrum_transaction(&height,symbol,ep,&txobj,txid,coinaddr)) != 0 )
                free_json(txobj);
            if ( (tx= LP_transactionfind(coin,txid)) != 0 )
            {
                if ( vout < tx->numvouts && tx->height > 0 )
                {
                    printf("added missing utxo for SPV checking\n");
                    LP_address_utxoadd(0,(uint32_t)time(NULL),"LP_validSPV",coin,coinaddr,txid,vout,tx->outpoints[vout].value,tx->height,-1);
                }
            }
        }
        if ( (up= LP_address_utxofind(coin,coinaddr,txid,vout)) != 0 )
        {
            if ( up->SPV > 0 )
                return(0);
            if ( up->SPV < 0 )
                return(-1);
            if ( (backupep= ep->prev) == 0 )
                backupep = ep;
            up->SPV = LP_merkleproof(coin,coinaddr,backupep,up->U.txid,up->U.height);
            if ( up->SPV < 0 )
                return(-1);
        }
    }
    return(0);
}

double LP_trades_alicevalidate(void *ctx,struct LP_quoteinfo *qp)
{
    double qprice; struct LP_utxoinfo A,B,*autxo,*butxo; char str[65];
    autxo = &A;
    butxo = &B;
    memset(autxo,0,sizeof(*autxo));
    memset(butxo,0,sizeof(*butxo));
    LP_abutxo_set(autxo,butxo,qp);
    if ( (qprice= LP_quote_validate(autxo,butxo,qp,0)) <= SMALLVAL )
    {
        printf("reserved quote validate error %.0f\n",qprice);
        return((int32_t)qprice);
    }
    if ( LP_validSPV(qp->srccoin,qp->coinaddr,qp->txid,qp->vout) < 0 )
    {
        sleep(1);
        if ( LP_validSPV(qp->srccoin,qp->coinaddr,qp->txid,qp->vout) < 0 )
        {
            printf("LP_trades_alicevalidate %s src %s failed SPV check\n",qp->srccoin,bits256_str(str,qp->txid));
            return(-44);
        }
    }
    else if ( LP_validSPV(qp->srccoin,qp->coinaddr,qp->txid2,qp->vout2) < 0 )
    {
        sleep(1);
        if ( LP_validSPV(qp->srccoin,qp->coinaddr,qp->txid2,qp->vout2) < 0 )
        {
            printf("LP_trades_alicevalidate %s src2 %s failed SPV check\n",qp->srccoin,bits256_str(str,qp->txid2));
            return(-55);
        }
    }
    return(qprice);
}
*/
unsafe fn lp_reserved(qp: *mut lp::LP_quoteinfo, ctx: &MmArc) {
    let maxprice = lp::LP_Alicemaxprice;
    let price = maxprice;
    //let price = lp::LP_pricecache(qp, (*qp).srccoin.as_mut_ptr(), (*qp).destcoin.as_mut_ptr(), (*qp).txid, (*qp).vout);
    if lp::LP_pricevalid(price) > 0 && maxprice > SMALLVAL && price <= maxprice {
        (*qp).tradeid = lp::LP_Alicequery.tradeid;
        lp::LP_Alicereserved = *qp;
        lp::LP_alicequery_clear();
        //printf("send CONNECT\n");
        lp::LP_query(
            b"connect\x00" as *const u8 as *mut libc::c_char,
            qp,
            unwrap!(ctx.ffi_handle())
        );
    } else {
        log!({"LP_reserved {} price {} vs maxprice {}", (*qp).aliceid, price, maxprice});
    }
}
/*
double LP_trades_bobprice(double *bidp,double *askp,struct LP_quoteinfo *qp)
{
    double price; struct iguana_info *coin; char str[65];
    price = LP_myprice(1,bidp,askp,qp->srccoin,qp->destcoin);
    if ( (coin= LP_coinfind(qp->srccoin)) == 0 || price <= SMALLVAL || *askp <= SMALLVAL )
    {
        //printf("this node has no price for %s/%s\n",qp->srccoin,qp->destcoin);
        return(0.);
    }
    price = *askp;
    //printf("MYPRICE %s/%s %.8f vs qprice %.8f\n",qp->srccoin,qp->destcoin,price,(double)qp->destsatoshis/qp->satoshis);
    if ( LP_validSPV(qp->destcoin,qp->destaddr,qp->desttxid,qp->destvout) < 0 )
    {
        printf("LP_trades_bobprice %s dest %s failed SPV check\n",qp->destcoin,bits256_str(str,qp->desttxid));
        return(0.);
    }
    else if (LP_validSPV(qp->destcoin,qp->destaddr,qp->feetxid,qp->feevout) < 0 )
    {
        printf("LP_trades_bobprice %s dexfee %s failed SPV check\n",qp->destcoin,bits256_str(str,qp->feetxid));
        return(0.);
    }
    return(*askp);
}

double LP_trades_pricevalidate(struct LP_quoteinfo *qp,struct iguana_info *coin,double price)
{
    double qprice; struct LP_utxoinfo A,B,*autxo,*butxo;
    autxo = &A;
    butxo = &B;
    memset(autxo,0,sizeof(*autxo));
    memset(butxo,0,sizeof(*butxo));
    LP_abutxo_set(autxo,butxo,qp);
    if ( coin->etomic[0] == 0 && strcmp(qp->coinaddr,coin->smartaddr) != 0 )
    {
        printf("bob is patching qp->coinaddr %s mismatch != %s\n",qp->coinaddr,coin->smartaddr);
        strcpy(qp->coinaddr,coin->smartaddr);
    }
    if ( butxo == 0 || lp::bits256_nonz(butxo->payment.txid) == 0 || lp::bits256_nonz(butxo->deposit.txid) == 0 || butxo->payment.vout < 0 || butxo->deposit.vout < 0 )
    {
        char str[65],str2[65]; printf("couldnt find bob utxos for autxo %s/v%d %s/v%d %.8f -> %.8f\n",bits256_str(str,qp->txid),qp->vout,bits256_str(str2,qp->txid2),qp->vout2,dstr(qp->satoshis),dstr(qp->destsatoshis));
        return(-66);
    }
    if ( (qprice= LP_quote_validate(autxo,butxo,qp,1)) <= SMALLVAL )
    {
        printf("quote %s/%s validate error %.0f\n",qp->srccoin,qp->destcoin,qprice);
        return(-3);
    }
    if ( qprice < (price - 0.00000001) * 0.998)
    {
        printf(" quote price %.8f (%llu/%llu %.8f) too low vs %.8f for %s/%s price %.8f %.8f\n",qprice,(long long)qp->destsatoshis,(long long)(qp->satoshis-qp->txfee),(double)qp->destsatoshis/(qp->satoshis-qp->txfee),price,qp->srccoin,qp->destcoin,price,(price - 0.00000001) * 0.998);
        return(-77);
    }
    return(qprice);
}
*/
unsafe fn lp_trades_gotrequest(ctx: &MmArc, qp: *mut lp::LP_quoteinfo, newqp: *mut lp::LP_quoteinfo) -> *mut lp::LP_quoteinfo {
    let p;
    let qprice;
    *newqp = *qp;
    let qp = newqp;
    let mut str: [c_char; 65] = [0; 65];
    // AG: The Alice p2p ID seems to be in the `qp.desthash`.
    printf(b"bob %s received REQUEST.(%s) mpnet.%d fill.%d gtc.%d\n\x00".as_ptr() as *const c_char, lp::bits256_str(str.as_mut_ptr(), lp::G.LP_mypub25519), (*qp).uuidstr[32..].as_ptr(), (*qp).mpnet, (*qp).fill, (*qp).gtc);
    let coin = match unwrap!(lp_coinfind(ctx, c2s!((*qp).srccoin))) {Some(c) => c, None => return null_mut()};

    let mut bid = 0.;
    let mut ask = 0.;
    let my_price = lp::LP_trades_bobprice(&mut bid, &mut ask, qp);
    if my_price == 0. {
        log!({"myprice {} bid {} ask {}", my_price, bid, ask});
        return null_mut();
    }
    log!({"dest sat {} sat {} tx_fee {}", (*qp).destsatoshis, (*qp).satoshis, (*qp).txfee});
    unwrap!(safecopy!((*qp).coinaddr, "{}", coin.my_address()));
    if (*qp).srchash.nonz() == false || (*qp).srchash == lp::G.LP_mypub25519 {
        qprice = (*qp).destsatoshis as f64 / (*qp).satoshis as f64;
        strcpy((*qp).gui.as_mut_ptr(), lp::G.gui.as_ptr());
        (*qp).srchash = lp::G.LP_mypub25519;
    } else {
        return null_mut();
    }

    if qprice < my_price {
        printf(b"%s/%s ignore as qprice %.8f vs myprice %.8f\n\x00".as_ptr() as *const c_char, (*qp).srccoin.as_ptr(), (*qp).destcoin.as_ptr(), qprice, my_price);
        return null_mut();
    }
//LP_RTmetrics_update(qp->srccoin,qp->destcoin);
    if lp::LP_RTmetrics_blacklisted((*qp).desthash) >= 0 {
        printf(b"request from blacklisted %s, ignore\n\x00".as_ptr() as *const c_char, lp::bits256_str(str.as_mut_ptr(), (*qp).desthash));
        return null_mut();
    }
    if qprice >= my_price {
        unwrap!(safecopy!((*qp).gui, "{}", c2s!(lp::G.gui)));
        unwrap!(safecopy!((*qp).coinaddr, "{}", coin.my_address()));
        (*qp).srchash = lp::G.LP_mypub25519;
        (*qp).satoshis = lp_base_satoshis(dstr((*qp).destsatoshis as i64, 8), my_price, (*qp).desttxfee);
        (*qp).quotetime = (now_ms() / 1000) as u32;
        /*
        if (*qp).fill != 0 {
            break;
        }
        (*qp).destsatoshis = ((*qp).destsatoshis * 2) / 3;
        */
    } else {
        return null_mut();
    }

    if (*qp).satoshis <= (*qp).txfee {
        return null_mut();
    }
    p = (*qp).destsatoshis as f64 / (*qp).satoshis as f64;
    if lp::LP_trades_pricevalidate(qp, coin.iguana_info(), p) < 0. {
        if (*qp).fill != 0 {
            return null_mut();
        }
    }

    printf(b"%s/%s qprice %.8f myprice %.8f [%.8f]\n\x00".as_ptr() as *const c_char, (*qp).srccoin.as_ptr(), (*qp).destcoin.as_ptr(), qprice, my_price, p);
    let reqjson = lp::LP_quotejson(qp);
    if (*qp).quotetime == 0 {
        (*qp).quotetime = (now_ms() / 1000) as u32;
    }
    lp::jaddnum(reqjson, b"quotetime\x00".as_ptr() as *mut c_char, (*qp).quotetime as f64);
    lp::jaddnum(reqjson, b"pending\x00".as_ptr() as *mut c_char, ((*qp).timestamp + lp::LP_RESERVETIME) as f64);
    lp::jaddstr(reqjson, b"method\x00".as_ptr() as *mut c_char, b"reserved\x00".as_ptr() as *mut c_char);
    broadcast_p2p_msg_for_c((*qp).desthash, lp::jprint(reqjson, 0), unwrap!(ctx.ffi_handle()));
    // let zero = lp::bits256::default();
    // broadcast_p2p_msg(zero, lp::jprint(reqjson, 0));
    if (*qp).mpnet != 0 && (*qp).gtc == 0 {
        let msg = lp::jprint(reqjson, 0);
        lp::LP_mpnet_send(0, msg, 1, (*qp).destaddr.as_mut_ptr());
        free_c_ptr(msg as *mut c_void);
    }
    lp::free_json(reqjson);
    log!({"Send RESERVED id.{}",(*qp).aliceid});
    qp
}

unsafe fn lp_trades_gotreserved(qp: *mut lp::LP_quoteinfo, newqp: *mut lp::LP_quoteinfo) -> *mut lp::LP_quoteinfo {
    log!({"alice {:x?} received RESERVED.({}) {} mpnet.{} fill.{} gtc.{}",
             lp::G.LP_mypub25519.bytes, c2s!((*qp).uuidstr[32..]),
             (*qp).destsatoshis / ((*qp).satoshis + 1), (*qp).mpnet, (*qp).fill, (*qp).gtc});
    *newqp = *qp;
    let qp = newqp;
    // let qprice = lp::LP_trades_alicevalidate(qp);
    let qprice = 1.0;
    if qprice > 0. {
//printf("got qprice %.8f\n",qprice);
            lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"reserved\x00".as_ptr() as *mut c_char, 0, 0);
            let retstr = lp::LP_quotereceived(qp);
            free_c_ptr(retstr as *mut c_void);
            return qp;
    } else {
        lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, qprice, (*qp).uuidstr.as_mut_ptr());
    }
    null_mut()
}

unsafe fn lp_trades_got_connect(ctx: &MmArc, qp: *mut lp::LP_quoteinfo, new_qp: *mut lp::LP_quoteinfo) -> *mut lp::LP_quoteinfo {
    *new_qp = *qp;
    let qp = new_qp;
    let coin = unwrap!(lp_coinfind(ctx, c2s!((*qp).srccoin)));
    if coin.is_none() {return null_mut()}
    let mut bid = 0.;
    let mut ask = 0.;
    let my_price = lp::LP_trades_bobprice(&mut bid, &mut ask, qp);
    if my_price == 0. {
        log!("Bob my price is zero!");
        return null_mut();
    }
    //let q_price = lp::LP_trades_pricevalidate(qp, coin, my_price);
    //if q_price < 0. {
    //    log!("Bob q_price is less than zero!");
    //    return null_mut();
    //}
    //if lp::LP_reservation_check((*qp).txid, (*qp).vout, (*qp).desthash) == 0 && lp::LP_reservation_check((*qp).txid2, (*qp).vout2, (*qp).desthash) == 0 {
    // AG: The Alice p2p ID seems to be in the `qp.desthash`.
    log!({"bob {} received CONNECT.({})", lp::G.LP_mypub25519, c2s!((*qp).uuidstr[32..])});
    lp_connect_start_bob(&ctx, (*qp).srccoin.as_mut_ptr(), (*qp).destcoin.as_mut_ptr(), qp);
    return qp;
    //} else {
    //    lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, -1.0, (*qp).uuidstr.as_mut_ptr());
    //    log!({"connect message from non-reserved ({})", (*qp).aliceid});
    //}
}

unsafe fn lp_trades_gotconnected(
    ctx: u32,
    mut qp: *mut lp::LP_quoteinfo,
    newqp: *mut lp::LP_quoteinfo,
    pairstr: *mut c_char
) -> *mut lp::LP_quoteinfo {
    log!({"alice {} received CONNECTED.({}) mpnet.{} fill.{} gtc.{}",
            lp::G.LP_mypub25519, (*qp).aliceid, (*qp).mpnet, (*qp).fill, (*qp).gtc});
    *newqp = *qp;
    qp = newqp;
    // let val = lp::LP_trades_alicevalidate(qp);
    let val = 1.0;
    let mut changed = 0;
    if val > 0. {
//printf("CONNECTED ALICE uuid.%s\n",qp->uuidstr);
            lp::LP_aliceid((*qp).tradeid, (*qp).aliceid, b"connected\x00" as *const u8 as *mut c_char, 0, 0);
            lp_connected_alice(ctx, qp, pairstr);
            lp::LP_mypriceset(0, &mut changed, (*qp).destcoin.as_mut_ptr(), (*qp).srccoin.as_mut_ptr(), 0.);
            lp::LP_alicequery_clear();
            return qp;
    } else {
        lp::LP_failedmsg((*qp).R.requestid, (*qp).R.quoteid, val, (*qp).uuidstr.as_mut_ptr());
    }
//printf("LP_trades_alicevalidate error\n");
    null_mut()
}

unsafe fn lp_trades_bestpricecheck(tp: *mut lp::LP_trade) -> i32 {
    let mut flag = 0;
    let mut q = (*tp).Q;
    let pubp = lp::LP_pubkeyadd(q.srchash);
//printf("check bestprice %.8f vs new price %.8f\n",tp->bestprice,(double)Q.destsatoshis/Q.satoshis);
    if q.satoshis != 0 && !pubp.is_null() { //(qprice= LP_trades_alicevalidate(ctx,&Q)) > 0. )
        let qprice = q.destsatoshis as f64 / (q.satoshis - q.txfee) as f64;
        lp::LP_aliceid(q.tradeid, (*tp).aliceid, b"reserved\x00" as *const u8 as *mut c_char, 0, 0);
        let retstr = lp::LP_quotereceived(&mut q);
        free_c_ptr(retstr as *mut c_void);
//LP_trades_gotreserved(ctx,&Q,&tp->Qs[LP_RESERVED]);
        if (*tp).bestprice == 0. {
            flag = 1;
        } else if qprice < (*tp).bestprice && (*pubp).slowresponse <= ((*tp).bestresponse as f64 * 1.05) as u32 {
            flag = 1;
        } else if qprice <= (*tp).bestprice && (*pubp).unconfcredits > (*tp).bestunconfcredits && (*pubp).slowresponse <= (*tp).bestresponse {
            flag = 1;
        }
        if flag != 0 {
            (*tp).Qs[lp::LP_CONNECT as usize] = (*tp).Q;
            (*tp).bestprice = qprice;
            (*tp).bestunconfcredits = (*pubp).unconfcredits;
            (*tp).bestresponse = (*pubp).slowresponse;
            log!({"aliceid.{} got new bestprice {} (unconf {}) slowresponse.{}",
                   (*tp).aliceid, (*tp).bestprice, (*tp).bestunconfcredits as f64 / SATOSHIS as f64, (*tp).bestresponse});
            return 1
        } //else printf("qprice %.8f dynamictrust %.8f not good enough\n",qprice,dstr(dynamictrust));
    } else {
        log!("alice didnt validate");
    }
    0
}
/*
int32_t LP_trades_canceluuid(char *uuidstr)
{
    int32_t num = 0; struct LP_trade *qtp,*tp,*tmp;
    HASH_ITER(hh,LP_trades,tp,tmp)
    {
        if ( strcmp(tp->Q.uuidstr,uuidstr) == 0 )
        {
            tp->cancelled = (uint32_t)time(NULL);
            num++;
        }
    }
    DL_FOREACH_SAFE(LP_tradesQ,qtp,tmp)
    {
        if ( strcmp(qtp->Q.uuidstr,uuidstr) == 0 )
        {
            qtp->cancelled = (uint32_t)time(NULL);
            num++;
        }
    }
    if ( num > 0 )
        fprintf(stderr,"uuid.%s %d cancelled\n",uuidstr,num);
    return(num);
}
*/
pub unsafe fn lp_trades_loop(ctx: MmArc) {
    let mut timeout: u32;
    let mut now: u64;
    let mut q: lp::LP_quoteinfo;
    let mut funcid: u32;
    let mut flag: i32;
    let mut pubp: *mut lp::LP_pubkey_info;
    thread::sleep(Duration::from_secs(5));

    loop {
        if ctx.is_stopping() { break }

        let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(&ctx));
        let mut trades_map = unwrap!(ordermatch_ctx.lp_trades.lock());
        for (_, trade) in trades_map.iter_mut() {
            if trade.negotiationdone != 0 || trade.cancelled != 0 {
                continue;
            }
            timeout = lp::LP_AUTOTRADE_TIMEOUT;
            let coin = unwrap!(lp_coinfind(&ctx, c2s!(trade.Q.srccoin)));
            if let Some(coin) = coin {if coin.iguana_info().electrum != null_mut() {
                timeout += (lp::LP_AUTOTRADE_TIMEOUT as f64 * 0.5) as u32;
            }}
            let coin = unwrap!(lp_coinfind(&ctx, c2s!(trade.Q.destcoin)));
            if let Some(coin) = coin {if coin.iguana_info().electrum != null_mut() {
                timeout += (lp::LP_AUTOTRADE_TIMEOUT as f64 * 0.5) as u32;
            }}
            now = now_ms() / 1000;
            if now > trade.lastprocessed && trade.iambob == 0 && trade.bestprice > 0. {
                if trade.connectsent == 0 {
                    lp::LP_Alicemaxprice = trade.bestprice;
                    lp_reserved(&mut trade.Qs[lp::LP_CONNECT as usize], &ctx); // send LP_CONNECT
                    (*trade).connectsent = now;
                    log!({"send LP_connect aliceid.{} {}", trade.aliceid, trade.bestprice});
                } else if now < trade.firstprocessed + timeout as u64 && ((trade.firstprocessed  + timeout as u64 - now) % 20) == 19 {
                    //LP_Alicemaxprice = tp->bestprice;
                    //LP_reserved(ctx,LP_myipaddr,LP_mypubsock,&tp->Qs[LP_CONNECT]); // send LP_CONNECT
                    //printf("mark slow LP_connect aliceid.%llu %.8f\n",(long long)tp->aliceid,tp->bestprice);
                    pubp = lp::LP_pubkeyfind(trade.Qs[lp::LP_CONNECT as usize].srchash);
                    if !pubp.is_null() {
                        (*pubp).slowresponse += 1;
                    }
                }
            }
        }
        now = now_ms() / 1000;
        for (key, trade) in (*trades_map).clone().iter_mut() {
            timeout = lp::LP_AUTOTRADE_TIMEOUT;
            let coin = unwrap!(lp_coinfind(&ctx, c2s!(trade.Q.srccoin)));
            if let Some(coin) = coin {if coin.iguana_info().electrum != null_mut() {
                timeout += (lp::LP_AUTOTRADE_TIMEOUT as f64 * 0.5) as u32;
            }}
            let coin = unwrap!(lp_coinfind(&ctx, c2s!(trade.Q.destcoin)));
            if let Some(coin) = coin {if coin.iguana_info().electrum != null_mut() {
                timeout += (lp::LP_AUTOTRADE_TIMEOUT as f64 * 0.5) as u32;
            }}
            if now > trade.firstprocessed + timeout as u64 * 10 || trade.cancelled != 0 {
                //printf("purge swap aliceid.%llu\n",(long long)tp->aliceid);
                trades_map.remove(key);
            }
        }

        while !ordermatch_ctx.is_trade_queue_empty() {
            now = now_ms() / 1000;
            let mut qtp = unwrap!(ordermatch_ctx.pop_trade_from_queue());
            q = qtp.Q;
            funcid = qtp.funcid;
//printf("dequeue %p funcid.%d aliceid.%llu iambob.%d\n",qtp,funcid,(long long)qtp->aliceid,qtp->iambob);
            match trades_map.entry(qtp.aliceid) {
                Entry::Occupied(t) => {
                    if t.get().cancelled != 0 {
                        t.remove();
                        continue;
                    }
                },
                Entry::Vacant(t) => {
                    if now > (q.timestamp + lp::LP_AUTOTRADE_TIMEOUT * 2) as u64 || qtp.cancelled != 0 {
                        // eat expired?
                    } else {
                        if qtp.iambob != 0 && funcid == lp::LP_REQUEST { // bob maybe sends LP_RESERVED
                            let qp = lp_trades_gotrequest(
                                &ctx,
                                &mut q,
                                &mut qtp.Qs[lp::LP_REQUEST as usize],
                            );
                            if qp != null_mut() {
                                qtp.Qs[lp::LP_RESERVED as usize] = q;
                            }
                        } else if qtp.iambob == 0 && funcid == lp::LP_RESERVED { // alice maybe sends LP_CONNECT
                            lp_trades_bestpricecheck(&mut qtp);
                        } else if qtp.iambob == 0 && funcid == lp::LP_CONNECTED {
                            qtp.negotiationdone = now;
                            //printf("alice sets negotiationdone.%u\n",now);
                            lp_trades_gotconnected(
                                unwrap!(ctx.ffi_handle()),
                                &mut qtp.Q,
                                &mut qtp.Qs[lp::LP_CONNECTED as usize],
                                qtp.pairstr.as_mut_ptr()
                            );
                        }
                        qtp.firstprocessed =  now_ms() / 1000;
                        qtp.lastprocessed =  now_ms() / 1000;
                        if funcid == lp::LP_CONNECT && qtp.negotiationdone == 0 { // bob all done
                            qtp.negotiationdone = now;
                            //printf("bob sets negotiationdone.%u\n",now);
                            lp_trades_got_connect(
                                &ctx,
                                &mut qtp.Q,
                                &mut qtp.Qs[lp::LP_CONNECT as usize],
                            );
                        }
                        t.insert(qtp);
                    }
                    continue;
                }
            };

            let trade = unwrap!(trades_map.get_mut(&qtp.aliceid));
            (*trade).Q = qtp.Q;
            if qtp.iambob == trade.iambob && qtp.pairstr[0] != 0 {
                strcpy(trade.pairstr.as_mut_ptr(), qtp.pairstr.as_ptr());
            }
//printf("finished dequeue %p funcid.%d aliceid.%llu iambob.%d/%d done.%u\n",qtp,funcid,(long long)qtp->aliceid,qtp->iambob,tp->iambob,tp->negotiationdone);
            flag = 0;
            if trade.iambob == 0 {
                if funcid == lp::LP_RESERVED {
                    if trade.connectsent == 0 {
                        flag = lp_trades_bestpricecheck(trade);
                    }
                } else if funcid == lp::LP_CONNECTED && trade.negotiationdone == 0 { // alice all done  tp->connectsent != 0 &&
                    flag = 1;
                    (*trade).negotiationdone = now;
                    lp_trades_gotconnected(
                        unwrap!(ctx.ffi_handle()),
                        &mut trade.Q,
                        &mut trade.Qs[lp::LP_CONNECTED as usize],
                        trade.pairstr.as_mut_ptr()
                    );
                }
            } else {
                if funcid == lp::LP_REQUEST { // bob maybe sends LP_RESERVED
                    let qp = lp_trades_gotrequest(
                        &ctx,
                        &mut q,
                        &mut trade.Qs[lp::LP_REQUEST as usize],
                    );
                    if !qp.is_null() {
                        (*trade).Qs[lp::LP_RESERVED as usize] = q;
                        flag = 1;
                    }
                } else if funcid == lp::LP_CONNECT && trade.negotiationdone == 0 { // bob all done
                    flag = 1;
                    (*trade).negotiationdone = now;
                    log!({"bob sets negotiationdone.{}", now_ms() / 1000});
                    lp_trades_got_connect(
                        &ctx,
                        &mut trade.Q,
                        &mut trade.Qs[lp::LP_CONNECT as usize],
                    );
                }
            }
            if flag != 0 {
                (*trade).lastprocessed = now_ms() / 1000;
            }
        }
        // drop trades map lock before sleeping to unlock the mutex
        drop(trades_map);
        thread::sleep(Duration::from_secs(1));
    }
}

unsafe fn lp_trade_command_q(ctx: &MmArc, qp: *mut lp::LP_quoteinfo, pairstr: *mut c_char, funcid: u32) {
    let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(ctx));

    let mut trade = lp::LP_trade::default();
    if funcid >= 4 {
        return;
    }
    trade.iambob = if funcid == lp::LP_REQUEST || funcid == lp::LP_CONNECT {
        1
    } else {
        0
    };
    trade.aliceid = (*qp).aliceid;
    trade.funcid = funcid;
    trade.newtime = now_ms() / 1000;
    trade.Q = *qp;
    if pairstr != null_mut() {
        strcpy(trade.pairstr.as_mut_ptr(), pairstr);
    }
    ordermatch_ctx.add_trade_to_queue(trade);
}

pub unsafe fn lp_trade_command(
    ctx: MmArc,
    json: Json,
    c_json: &CJSON,
) -> i32 {
    let q_trades: i32 = 1;
    let mut str: [libc::c_char; 65] = [0; 65];
    let mut i: i32;
    let dex_selector: i32 = 0;
    let aliceid: u64;
    let qprice: f64;
    let price: f64;
    let mut bid: f64 = 0.;
    let mut ask: f64 = 0.;
    let rq: u64;
    let mut q = lp::LP_quoteinfo::default();
    let mut q2 = lp::LP_quoteinfo::default();
    let mut counter: i32 = 0;
    let mut retval: i32 = -1i32;
    let method = json["method"].as_str();
    match method {
        Some("reserved") | Some("connected") | Some("request") | Some("connect") => {
            if lp::LP_quoteparse(&mut q, c_json.0) < 0 {
                printf(
                    b"ERROR parsing.(%s)\n\x00" as *const u8 as *const libc::c_char,
                    lp::jprint(c_json.0, 0),
                );
                return 1i32;
            } else if q.satoshis < q.txfee {
                return 1i32;
            } else {
                lp::LP_requestinit(
                    &mut q.R,
                    q.srchash,
                    q.desthash,
                    q.srccoin.as_mut_ptr(),
                    q.satoshis.wrapping_sub(q.txfee),
                    q.destcoin.as_mut_ptr(),
                    q.destsatoshis.wrapping_sub(q.desttxfee),
                    q.timestamp,
                    q.quotetime,
                    dex_selector,
                    q.fill as i32,
                    q.gtc as i32,
                );
                rq = (q.R.requestid as u64) << 32i32 | q.R.quoteid as u64;
                // eat expired packets, some old timestamps floating about?
                if q.uuidstr[0usize] == 0 || q.timestamp > 0 && now_ms() / 1000 > q.timestamp.wrapping_add(30 * 20) as u64 {
                    printf(
                        b"uuid.%s aliceid.%llu is expired by %d\n\x00" as *const u8
                            as *const libc::c_char,
                        q.uuidstr.as_mut_ptr().offset(32isize),
                        q.aliceid as libc::c_longlong,
                        (now_ms() / 1000).wrapping_sub(q.timestamp.wrapping_add(60) as u64));
                    return 1i32;
                } else {
                    lp::LP_tradecommand_log(c_json.0);
                    //jdouble(argjson,"price");
                    qprice = q.destsatoshis as f64 / q.satoshis.wrapping_sub(q.txfee) as f64;
                    //printf("%s\n",jprint(argjson,0));
                    retval = 1i32;
                    aliceid = lp::j64bits(
                        c_json.0,
                        b"aliceid\x00" as *const u8 as *const libc::c_char as *mut libc::c_char,
                    );
                    if method == Some("reserved") {
                        lp_bob_competition(&ctx, &mut counter, aliceid, qprice, 1i32);
                        if (lp::LP_Alicedestpubkey.ulongs[0usize]
                            | lp::LP_Alicedestpubkey.ulongs[1usize]
                            | lp::LP_Alicedestpubkey.ulongs[2usize]
                            | lp::LP_Alicedestpubkey.ulongs[3usize]
                            != 0 as u64) as libc::c_int != 0 {
                            if lp::LP_Alicedestpubkey != q.srchash {
                                printf(
                                    b"got reserved response from different node %s\n\x00"
                                        as *const u8
                                        as *const libc::c_char,
                                    lp::bits256_str(str.as_mut_ptr(), q.srchash),
                                );
                                return retval;
                            } else {
                                printf(
                                    b"got reserved response from destpubkey %s\n\x00" as *const u8
                                        as *const libc::c_char,
                                    lp::bits256_str(str.as_mut_ptr(), q.srchash),
                                );
                            }
                        }
                        // alice
                        if lp::G.LP_mypub25519 == q.desthash && lp::G.LP_mypub25519 != q.srchash {
                            if q_trades == 0 {
                                if q.quotetime as u64 > now_ms() / 1000
                                    && lp::LP_alice_eligible(q.quotetime) > 0
                                    {
                                        lp_trades_gotreserved(&mut q, &mut q2);
                                        if lp::LP_quotecmp(0, &mut q, &mut lp::LP_Alicequery) == 0 {
                                            lp_reserved(&mut q, &ctx);
                                        }
                                    }
                            } else {
                                lp_trade_command_q(
                                    &ctx,
                                    &mut q,
                                    lp::jstr(
                                        c_json.0,
                                        b"pair\x00" as *const u8 as *const libc::c_char
                                            as *mut libc::c_char,
                                    ),
                                    1,
                                );
                            }
                        }
                    } else if method == Some("connected") {
                        lp_bob_competition(&ctx, &mut counter, aliceid, qprice, 1000);
                        // alice
                        if lp::G.LP_mypub25519 == q.desthash && lp::G.LP_mypub25519 != q.srchash
                            {
                                static mut RQS: [u64; 1024] = [0; 1024];
                                i = 0;
                                while (i as u64)
                                    < (::std::mem::size_of::<[u64; 1024]>() as u64)
                                    .wrapping_div(::std::mem::size_of::<u64>() as u64)
                                    {
                                        if rq == RQS[i as usize] {
                                            return retval;
                                        } else {
                                            i += 1
                                        }
                                    }
                                i = 0;
                                while (i as u64)
                                    < (::std::mem::size_of::<[u64; 1024]>() as u64)
                                    .wrapping_div(::std::mem::size_of::<u64>() as u64)
                                    {
                                        if RQS[i as usize] == 0 as u64 {
                                            break;
                                        }
                                        i += 1
                                    }
                                if i as u64
                                    == (::std::mem::size_of::<[u64; 1024]>() as u64)
                                    .wrapping_div(::std::mem::size_of::<u64>() as u64)
                                    {
                                        i = thread_rng().next_u64().wrapping_rem(
                                            (::std::mem::size_of::<[u64; 1024]>() as u64)
                                                .wrapping_div(
                                                    ::std::mem::size_of::<u64>() as u64
                                                ),
                                        ) as i32
                                    }
                                RQS[i as usize] = rq;
                                // AG: Bob's p2p ID (`LP_mypub25519`) is in `json["srchash"]`.
                                log!("CONNECTED.(" (json) ")");
                                if q_trades == 0 {
                                    lp_trades_gotconnected(
                                        unwrap!(ctx.ffi_handle()),
                                        &mut q,
                                        &mut q2,
                                        lp::jstr(
                                            c_json.0,
                                            b"pair\x00" as *const u8 as *const libc::c_char
                                                as *mut libc::c_char,
                                        ),
                                    );
                                } else {
                                    lp_trade_command_q(
                                        &ctx,
                                        &mut q,
                                        lp::jstr(
                                            c_json.0,
                                            b"pair\x00" as *const u8 as *const libc::c_char
                                                as *mut libc::c_char,
                                        ),
                                        3,
                                    );
                                }
                            }
                    }
                    // bob
                    if method == Some("request") || method == Some("connect") {
                        price = lp::LP_myprice(
                            1i32,
                            &mut bid,
                            &mut ask,
                            q.srccoin.as_mut_ptr(),
                            q.destcoin.as_mut_ptr(),
                        )
                    } else {
                        price = lp::LP_myprice(
                            0,
                            &mut bid,
                            &mut ask,
                            q.srccoin.as_mut_ptr(),
                            q.destcoin.as_mut_ptr(),
                        )
                    }
                    let coin = unwrap!(lp_coinfind(&ctx, c2s!(q.srccoin)));
                    if coin.is_none() {
                        //printf("%s is not active\n",Q.srccoin);
                        return retval;
                    } else if price <= 1e-15f64 || ask <= 1e-15f64 {
                        //printf("this node has no price for %s/%s\n",Q.srccoin,Q.destcoin);
                        return retval;
                    } else {
                        // bob
                        if method == Some("request") {
                            //if ( LP_Alicemaxprice != 0. )
                            //    return(retval);
                            lp_bob_competition(&ctx, &mut counter, aliceid, qprice, -1i32);
                            //printf("bestprice %.8f\n",bestprice);
                            //|| (bits256_cmp(Q.srchash,lp::G.LP_mypub25519) == 0 && bits256_cmp(lp::G.LP_mypub25519,Q.desthash) != 0) )
                            if q_trades == 0 {
                                lp_trades_gotrequest(
                                    &ctx,
                                    &mut q,
                                    &mut q2,
                                );
                            } else {
                                lp_trade_command_q(
                                    &ctx,
                                    &mut q,
                                    lp::jstr(
                                        c_json.0,
                                        b"pair\x00" as *const u8 as *const libc::c_char
                                            as *mut libc::c_char,
                                    ),
                                    0,
                                );
                            }
                        } else if method == Some("connect") {
                            lp_bob_competition(&ctx, &mut counter, aliceid, qprice, 1000);
                            // bob
                            if lp::G.LP_mypub25519 == q.srchash && lp::G.LP_mypub25519 != q.desthash {
                                static mut RQS_0: [u64; 1024] = [0; 1024];
                                i = 0;
                                while (i as u64)
                                    < (::std::mem::size_of::<[u64; 1024]>() as u64)
                                    .wrapping_div(::std::mem::size_of::<u64>() as u64)
                                    {
                                        if rq == RQS_0[i as usize] {
                                            return retval;
                                        } else {
                                            i += 1
                                        }
                                    }
                                i = 0;
                                while (i as u64)
                                    < (::std::mem::size_of::<[u64; 1024]>() as u64)
                                    .wrapping_div(::std::mem::size_of::<u64>() as u64)
                                    {
                                        if RQS_0[i as usize] == 0 as u64 {
                                            break;
                                        }
                                        i += 1
                                    }
                                if i as u64
                                    == (::std::mem::size_of::<[u64; 1024]>() as u64)
                                    .wrapping_div(::std::mem::size_of::<u64>() as u64)
                                    {
                                        i = thread_rng().next_u64().wrapping_rem(
                                            (::std::mem::size_of::<[u64; 1024]>() as u64)
                                                .wrapping_div(
                                                    ::std::mem::size_of::<u64>() as u64
                                                ),
                                        ) as i32
                                    }
                                RQS_0[i as usize] = rq;
                                printf(
                                    b"CONNECT.(%s)\n\x00" as *const u8 as *const libc::c_char,
                                    lp::jprint(c_json.0, 0),
                                );
                                if q_trades == 0 {
                                    lp_trades_got_connect(
                                        &ctx,
                                        &mut q,
                                        &mut q2,
                                    );
                                } else {
                                    lp_trade_command_q(
                                        &ctx,
                                        &mut q,
                                        lp::jstr(
                                            c_json.0,
                                            b"pair\x00" as *const u8 as *const libc::c_char
                                                as *mut libc::c_char,
                                        ),
                                        2,
                                    );
                                }
                            }
                        }
                        return retval;
                    }
                }
            }
        },
        _ => return retval
    };
}

fn zero_f() -> f64 { 0. }

#[derive(Deserialize, Debug)]
pub struct AutoBuyInput {
    base: String,
    rel: String,
    price: f64,
    #[serde(rename="relvolume")]
    #[serde(default = "zero_f")]
    rel_volume: f64,
    #[serde(rename="basevolume")]
    #[serde(default = "zero_f")]
    base_volume: f64,
    #[serde(default = "zero_f")]
    fomo: f64,
    #[serde(default = "zero_f")]
    dump: f64,
    timeout: Option<u32>,
    /// Not used. Deprecated.
    duration: Option<u32>,
    // TODO: remove this field on API refactoring, method should be separated from params
    method: String,
    fill: Option<u32>,
    gtc: Option<u32>,
    gui: Option<String>,
    #[serde(rename="destpubkey")]
    dest_pub_key: Option<String>
}

pub fn buy(ctx: MmArc, json: Json) -> HyRes {
    let input : AutoBuyInput = try_h!(json::from_value(json.clone()));
    if input.base == input.rel {
        return rpc_err_response(500, "Base and rel must be different coins");
    }
    let rel_coin = try_h!(lp_coinfind(&ctx, &input.rel));
    let rel_coin = match rel_coin {Some(c) => c, None => return rpc_err_response(500, "Rel coin is not found or inactive")};
    let base_coin = try_h!(lp_coinfind(&ctx, &input.base));
    let base_coin: MmCoinEnum = match base_coin {Some(c) => c, None => return rpc_err_response(500, "Base coin is not found or inactive")};
    Box::new(rel_coin.check_i_have_enough_to_trade(input.rel_volume, false).and_then(move |_|
        base_coin.can_i_spend_other_payment().and_then(move |_|
            rpc_response(200, try_h!(lp_auto_buy(&ctx, input)))
        )
    ))
}

pub fn sell(ctx: MmArc, json: Json) -> HyRes {
    let input : AutoBuyInput = try_h!(json::from_value(json.clone()));
    if input.base == input.rel {
        return rpc_err_response(500, "Base and rel must be different coins");
    }
    let base_coin = try_h!(lp_coinfind(&ctx, &input.base));
    let base_coin = match base_coin {Some(c) => c, None => return rpc_err_response(500, "Base coin is not found or inactive")};
    let rel_coin = try_h!(lp_coinfind(&ctx, &input.rel));
    let rel_coin = match rel_coin {Some(c) => c, None => return rpc_err_response(500, "Rel coin is not found or inactive")};
    Box::new(rel_coin.check_i_have_enough_to_trade(input.base_volume, false).and_then(move |_|
        base_coin.can_i_spend_other_payment().and_then(move |_|
            rpc_response(200, try_h!(lp_auto_buy(&ctx, input)))
        )
    ))
}

pub fn lp_auto_buy(ctx: &MmArc, input: AutoBuyInput) -> Result<String, String> {
    if input.price < SMALLVAL {
        return ERR!("Price is too low, minimum is {}", SMALLVAL);
    }

    let (base, volume, price) = match Some(input.method.as_ref()) {
        Some("buy") => {
            let volume = if input.fomo > 0. {
                input.fomo
            } else {
                input.rel_volume
            };
            if volume <= 0. {
                return ERR!("Volume must be greater than 0");
            }
            (try_s!(lp_coinfind(&ctx, &input.base)), volume, input.price)
        },
        Some("sell") => {
            let volume = if input.dump > 0. {
                input.dump
            } else {
                input.base_volume
            };
            if volume <= 0. {
                return ERR!("Volume must be greater than 0");
            }
            (try_s!(lp_coinfind(&ctx, &input.rel)), volume, 1. / input.price)
        },
        _ => return ERR!("Auto buy must be called only from buy/sell RPC methods")
    };
    let base = match base {Some(c) => c, None => return ERR!("Base coin is not found or inactive")};
    let base_ii = base.iguana_info();

    let timeout = input.timeout.unwrap_or(unsafe { lp::LP_AUTOTRADE_TIMEOUT });

    unsafe {
        if now_ms() / 1000 < lp::Alice_expiration as u64 {
            return ERR!("Only 1 pending request at a time, wait {}",
                            lp::Alice_expiration as u64 - now_ms() / 1000);
        } else {
            lp::LP_alicequery_clear();
        }

        let mut tx_fee : u64 = 0;
        let mut dest_tx_fee : u64 = 0;
        let base_str = try_s!(CString::new(input.base.clone()));
        let rel_str = try_s!(CString::new(input.rel.clone()));

        lp::LP_txfees(
            &mut tx_fee as *mut u64,
            &mut dest_tx_fee as *mut u64,
            base_str.as_ptr() as *mut c_char,
            rel_str.as_ptr() as *mut c_char,
        );
        if dest_tx_fee != 0 && dest_tx_fee < 10000 {
            dest_tx_fee = 10000;
        }

        if price <= 0. {
            return ERR!("Resulting price is <= 0");
        }
        if volume <= 0. {
            return ERR!("Resulting volume is <= 0");
        }
        if lp::LP_priceinfofind(base_str.as_ptr() as *mut c_char) == null_mut() {
            return ERR!("No price info found for base coin {}", input.base);
        }
        if lp::LP_priceinfofind(rel_str.as_ptr() as *mut c_char) == null_mut() {
            return ERR!("No price info found for rel coin {}", input.rel);
        }

        let dest_satoshis = (SATOSHIS as f64 * volume) as u64;
        let mut b = lp::LP_utxoinfo::default();
        let fill_flag = input.fill.unwrap_or(0);

        let best_satoshis = lp_base_satoshis(sat_to_f(dest_satoshis), price, dest_tx_fee);
        strcpy(b.coin.as_ptr() as *mut c_char, base_str.as_ptr());
        let mut q = lp::LP_quoteinfo::default();
        if lp::LP_quoteinfoinit(
            &mut q as *mut lp::LP_quoteinfo,
            &mut b as *mut lp::LP_utxoinfo,
            rel_str.as_ptr() as *mut c_char,
            price,
            best_satoshis,
            dest_satoshis,
        ) < 0 {
            return ERR!("cant set ordermatch quote");
        }
        if lp::LP_quotedestinfo(
            &mut q as *mut lp::LP_quoteinfo,
            lp::G.LP_mypub25519,
            (*base_ii).smartaddr.as_mut_ptr(),
        ) < 0 {
            return ERR!("cant set ordermatch quote info");
        }
        let mut changed : i32 = 0;
        q.mpnet = lp::G.mpnet;
        q.fill = fill_flag;
        q.gtc = input.gtc.unwrap_or(0);
        lp::LP_mypriceset(0,
                          &mut changed as *mut i32,
                          rel_str.as_ptr() as *mut c_char,
                          base_str.as_ptr() as *mut c_char,
                          1. / price,
        );
        lp::LP_mypriceset(0,
                          &mut changed as *mut i32,
                          base_str.as_ptr() as *mut c_char,
                          rel_str.as_ptr() as *mut c_char,
                          0.,
        );
        let uuid_str: [c_char; 100] = [0; 100];
        lp::gen_quote_uuid(uuid_str.as_ptr() as *mut c_char, base_str.as_ptr() as *mut c_char, rel_str.as_ptr() as *mut c_char);
        let dest_pub_key = lp::bits256::default();
        if input.dest_pub_key.is_some() {
            let pub_key_str = try_s!(CString::new(input.dest_pub_key.unwrap()));
            lp::decode_hex(dest_pub_key.bytes.as_ptr() as *mut u8, 32, pub_key_str.as_ptr() as *mut c_char);
        }
        Ok(try_s!(lp_trade(
            &mut q as *mut lp::LP_quoteinfo,
            price,
            timeout as i32,
            0,
            dest_pub_key,
            uuid_str.as_ptr() as *mut c_char,
            ctx,
        )))
    }
}
