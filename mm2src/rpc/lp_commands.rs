
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
//  rpc_commands.rs
//  marketmaker
//
use common::{bitcoin_address, bits256, coins_iter, lp, rpc_response, rpc_err_response, HyRes, CORE, MM_VERSION};
use common::mm_ctx::MmArc;
use coins::lp_coinfind;
use futures::Future;
use futures_timer::Delay;
use gstuff::now_ms;
use hex;
use libc::{c_void, free};
use serde_json::{self as json, Value as Json};
use std::ffi::{CStr};
use std::mem::zeroed;
use std::ptr::{null_mut, write_volatile};
use std::time::Duration;

use crate::mm2::lp_native_dex::lp_passphrase_init;

/*
char *LP_numutxos()
{
    cJSON *retjson = cJSON_CreateObject();
    if ( LP_mypeer != 0 )
    {
        jaddstr(retjson,"ipaddr",LP_mypeer->ipaddr);
        jaddnum(retjson,"port",LP_mypeer->port);
        //jaddnum(retjson,"numutxos",LP_mypeer->numutxos);
        jaddnum(retjson,"numpeers",LP_mypeer->numpeers);
        jaddnum(retjson,"session",G.LP_sessionid);
    } else jaddstr(retjson,"error","client node");
    return(jprint(retjson,1));
}

char *stats_JSON(void *ctx,int32_t fastflag,char *myipaddr,int32_t pubsock,cJSON *argjson,char *remoteaddr,uint16_t port) // from rpc port
{
    char *method,*userpass,*base,*rel,*coin,*passphrase,*retstr = 0; int32_t authenticated=0,changed,flag = 0; cJSON *retjson,*reqjson = 0; struct iguana_info *ptr;
    method = jstr(argjson,"method");
    if ( method != 0 && (strcmp(method,"addr_unspents") == 0 || strcmp(method,"uitem") == 0 || strcmp(method,"postutxos") == 0) )
        return(0);
//printf("stats_JSON.(%s)\n",jprint(argjson,0));
    /*if ( (ipaddr= jstr(argjson,"ipaddr")) != 0 && (argport= juint(argjson,"port")) != 0 && (method == 0 || strcmp(method,"electrum") != 0) )
    {
        if ( strcmp(ipaddr,"127.0.0.1") != 0 && argport >= 1000 )
        {
            flag = 1;
            if ( (pushport= juint(argjson,"push")) == 0 )
                pushport = argport + 1;
            if ( (subport= juint(argjson,"sub")) == 0 )
                subport = argport + 2;
            if ( (peer= LP_peerfind((uint32_t)calc_ipbits(ipaddr),argport)) != 0 )
            {
                if ( 0 && (otherpeers= jint(argjson,"numpeers")) > peer->numpeers )
                    peer->numpeers = otherpeers;
                if ( peer->sessionid == 0 )
                    peer->sessionid = juint(argjson,"session");
                //printf("peer.(%s) found (%d %d) (%d %d) (%s)\n",peer->ipaddr,peer->numpeers,peer->numutxos,otherpeers,othernumutxos,jprint(argjson,0));
            } else LP_addpeer(LP_mypeer,LP_mypubsock,ipaddr,argport,pushport,subport,jint(argjson,"numpeers"),jint(argjson,"numutxos"),juint(argjson,"session"));
        }
    }*/
    if ( method == 0 )
    {
        if ( is_cJSON_Array(argjson) != 0 )
            printf("RAWARRAY command? %s\n",jprint(argjson,0));
        if ( flag == 0 || jobj(argjson,"result") != 0 )
            printf("stats_JSON no method: (%s)\n",jprint(argjson,0));
        return(0);
    }
    if ( strcmp(method,"hello") == 0 )
    {
        //int32_t i; cJSON *array = cJSON_CreateArray();
        retjson = cJSON_CreateObject();
        jaddstr(retjson,"result","success");
        jaddstr(retjson,"status","got hello");
        //for (i=0; i<10000; i++)
        //    jaddinum(array,i);
        //jadd(retjson,"array",array);
        return(jprint(retjson,1));
        //printf("got hello from %s:%u\n",ipaddr!=0?ipaddr:"",argport);
        //return(clonestr("{\"result\":\"success\",\"status\":\"got hello\"}"));
    }
    /*else if ( strcmp(method,"sendmessage") == 0 && jobj(argjson,"userpass") == 0 )
     {
         static char *laststr;
         char *newstr; bits256 pubkey = jbits256(argjson,"pubkey");
         if ( bits256_nonz(pubkey) == 0 || bits256_cmp(pubkey,G.LP_mypub25519) == 0 )
         {
             newstr = jprint(argjson,0);
             if ( laststr == 0 || strcmp(laststr,newstr) != 0 )
             {
                 printf("got message.(%s) from %s:%u\n",newstr,ipaddr!=0?ipaddr:"",argport);
                 if ( laststr != 0 )
                     free(laststr);
                 laststr = newstr;
                 LP_gotmessage(argjson);
                 retstr = clonestr(laststr);
             }
         } else retstr = clonestr("{\"error\":\"duplicate message\"}");
     }*/
    //else if ( strcmp(method,"nn_tests") == 0 )
    //    return(clonestr("{\"result\":\"success\"}"));
*/
pub fn help() -> HyRes {
    rpc_response(200, "
        buy(base, rel, price, relvolume, timeout=10, duration=3600)
        electrum(coin, urls)
        enable(coin, urls, swap_contract_address)
        myprice(base, rel)
        my_balance(coin)
        my_swap_status(params/uuid)
        orderbook(base, rel, duration=3600)
        sell(base, rel, price, basevolume, timeout=10, duration=3600)
        send_raw_transaction(coin, tx_hex)
        setprice(base, rel, price, broadcast=1)
        stop()
        version
        withdraw(coin, amount, to)
    ")
}

pub fn version() -> HyRes { rpc_response(200, MM_VERSION) }
/*
    if ( (base= jstr(argjson,"base")) == 0 )
        base = "";
    if ((rel= jstr(argjson,"rel")) == 0 )
        rel = "";
    if ( (coin= jstr(argjson,"coin")) == 0 )
        coin = "";
    if ( G.USERPASS[0] != 0 && strcmp(remoteaddr,"127.0.0.1") == 0 && port != 0 && strcmp(method,"psock") != 0 ) // protected localhost
    {
        if ( G.USERPASS_COUNTER == 0 )
        {
            G.USERPASS_COUNTER = 1;
            LP_cmdcount++;
        }
*/

/// JSON structure passed to the "passphrase" RPC call.  
/// cf. https://docs.komodoplatform.com/barterDEX/barterDEX-API.html#passphrase
#[derive(Clone, Deserialize, Debug)]
struct PassphraseReq {
    passphrase: String,
    /// Optional because we're checking the `passphrase` hash first.
    userpass: Option<String>,
    /// Defaults to "cli" (in `lp_passphrase_init`).
    gui: Option<String>,
    seednodes: Option<Vec<String>>
}

pub fn passphrase (ctx: MmArc, req: Json) -> HyRes {
    let matching_userpass = super::auth (&req, &ctx) .is_ok();
    let req: PassphraseReq = try_h! (json::from_value (req));

    let mut passhash: bits256 = unsafe {zeroed()};
    unsafe {lp::vcalc_sha256 (null_mut(), passhash.bytes.as_mut_ptr(), req.passphrase.as_ptr() as *mut u8, req.passphrase.len() as i32)};
    let matching_passphrase = unsafe {passhash == lp::G.LP_passhash};
    if !matching_passphrase {
        log! ({"passphrase] passhash {} != G {}", passhash, unsafe {lp::G.LP_passhash}});
        if !matching_userpass {return rpc_err_response (500, "authentication error")}
    }

    unsafe {lp::G.USERPASS_COUNTER = 1}


    unsafe {try_h! (lp_passphrase_init (Some (&req.passphrase), req.gui.as_ref().map (|s| &s[..])))};

    let mut coins = Vec::new();
    try_h! (unsafe {coins_iter (&mut |coin| {
        let coin_json = lp::LP_coinjson (coin, lp::LP_showwif);
        let cjs = lp::jprint (coin_json, 1);
        let cjs_copy = Vec::from (CStr::from_ptr (cjs) .to_bytes());
        free (cjs as *mut c_void);
        lp::free_json (coin_json);
        let rcjs: Json = try_s! (json::from_slice (&cjs_copy));
        coins.push (rcjs);
        Ok(())
    })});

    let retjson = json! ({
        "result": "success",
        "userpass": try_h! (unsafe {CStr::from_ptr (lp::G.USERPASS.as_ptr())} .to_str()),
        "mypubkey": fomat! ((unsafe {lp::G.LP_mypub25519})),
        "pubsecp": hex::encode (unsafe {&lp::G.LP_pubsecp[..]}),
        "KMD": try_h! (bitcoin_address ("KMD", 60, unsafe {lp::G.LP_myrmd160})),
        "BTC": try_h! (bitcoin_address ("BTC", 0, unsafe {lp::G.LP_myrmd160})),
        "NXT": try_h! (unsafe {CStr::from_ptr (lp::G.LP_NXTaddr.as_ptr())} .to_str()),
        "coins": coins
    });

    rpc_response (200, try_h! (json::to_string (&retjson)))
}
/*
        else if ( strcmp(method,"instantdex_deposit") == 0 )
        {
            if ( (ptr= LP_coinsearch("KMD")) != 0 )
            {
                if ( jint(argjson,"weeks") <= 0 ) {
                    return(clonestr("{\"error\":\"instantdex_deposit weeks param must be greater than zero\"}"));
                }
                if ( jdouble(argjson,"amount") < 10. ) {
                    return(clonestr("{\"error\":\"instantdex_deposit amount param must be equal or greater than 10\"}"));
                }

                return(LP_instantdex_deposit(ptr,juint(argjson,"weeks"),jdouble(argjson,"amount"),jobj(argjson,"broadcast") != 0 ? jint(argjson,"broadcast") : 1));
            }
            return(clonestr("{\"error\":\"cant find KMD\"}"));
        }
*/
pub fn mpnet(json: &Json) -> HyRes {
    if !json["onoff"].is_u64() {
        return rpc_err_response(400, "onoff must be unsigned int");
    }

    let onoff = json["onoff"].as_u64().unwrap();
    if onoff > 1 {
        return rpc_err_response(400, "onoff must be 0 or 1");
    }

    unsafe { lp::G.mpnet = onoff as u32 };
    log!({"MPNET onoff.{}", onoff});
    rpc_response (200, r#"{"result": "success"}"#)
}
/*
        else if ( strcmp(method,"getendpoint") == 0 )
        {
            int32_t err,mode; uint16_t wsport = 5555; char endpoint[64],bindpoint[64];
            if ( juint(argjson,"port") != 0 )
                wsport = juint(argjson,"port");
            retjson = cJSON_CreateObject();
            if ( IPC_ENDPOINT >= 0 )
            {
                jaddstr(retjson,"error","IPC endpoint already exists");
                jaddnum(retjson,"socket",IPC_ENDPOINT);
            }
            else
            {
                if ( (IPC_ENDPOINT= nn_socket(AF_SP,NN_PAIR)) >= 0 )
                {
                    sprintf(bindpoint,"ws://*:%u",wsport);
                    sprintf(endpoint,"ws://127.0.0.1:%u",wsport);
                    if ( (err= nn_bind(IPC_ENDPOINT,bindpoint)) >= 0 )
                    {
                        jaddstr(retjson,"result","success");
                        jaddstr(retjson,"endpoint",endpoint);
                        jaddnum(retjson,"socket",IPC_ENDPOINT);
                        mode = NN_WS_MSG_TYPE_TEXT;
                        err = nn_setsockopt(IPC_ENDPOINT,NN_SOL_SOCKET,NN_WS_MSG_TYPE,&mode,sizeof(mode));
                        jaddnum(retjson,"sockopt",err);
                    }
                    else
                    {
                        jaddstr(retjson,"error",(char *)nn_strerror(nn_errno()));
                        jaddstr(retjson,"bind",bindpoint);
                        jaddnum(retjson,"err",err);
                        jaddnum(retjson,"socket",IPC_ENDPOINT);
                        nn_close(IPC_ENDPOINT);
                        IPC_ENDPOINT = -1;
                    }
                } else jaddstr(retjson,"error","couldnt get NN_PAIR socket");
            }
            return(jprint(retjson,1));
        }
        else if ( strcmp(method,"verus") == 0 )
        {
            return(verusblocks());
        }
        else if ( strcmp(method,"instantdex_claim") == 0 )
        {
            if ( (ptr= LP_coinsearch("KMD")) != 0 )
            {
                return(LP_instantdex_claim(ptr));
            }
            return(clonestr("{\"error\":\"cant find KMD\"}"));
        }
        else if ( strcmp(method,"jpg") == 0 )
        {
            return(LP_jpg(jstr(argjson,"srcfile"),jstr(argjson,"destfile"),jint(argjson,"power2"),jstr(argjson,"password"),jstr(argjson,"data"),jint(argjson,"required"),juint(argjson,"ind")));
        }
        /*else if ( strcmp(method,"sendmessage") == 0 )
        {
            if ( jobj(argjson,"method2") == 0 )
            {
                LP_broadcast_message(LP_mypubsock,base!=0?base:coin,rel,jbits256(argjson,"pubkey"),jprint(argjson,0));
            }
            return(clonestr("{\"result\":\"success\"}"));
        }
        else if ( strcmp(method,"getmessages") == 0 )
        {
            if ( (retjson= LP_getmessages(jint(argjson,"firsti"),jint(argjson,"num"))) != 0 )
                return(jprint(retjson,1));
            else return(clonestr("{\"error\":\"null messages\"}"));
        }
        else if ( strcmp(method,"deletemessages") == 0 )
        {
            LP_deletemessages(jint(argjson,"firsti"),jint(argjson,"num"));
            return(clonestr("{\"result\":\"success\"}"));
        }*/
        else if ( strcmp(method,"cancel") == 0 )
        {
            return(LP_cancel_order(jstr(argjson,"uuid")));
        }
        else if ( strcmp(method,"recentswaps") == 0 )
        {
            return(LP_recent_swaps(jint(argjson,"limit"),0));
        }
*/*/
pub fn stop (ctx: MmArc) -> HyRes {
    // Should delay the shutdown a bit in order not to trip the "stop" RPC call in unit tests.
    // Stopping immediately leads to the "stop" RPC call failing with the "errno 10054" sometimes.
    let pause_f = Delay::new (Duration::from_millis (50));
    let stop_f = pause_f.then (move |r| -> Result<(), ()> {
        if let Err (err) = r {log! ("stop] Warning, there was a Delay error: " (err))}
        unsafe {write_volatile (&mut lp::LP_STOP_RECEIVED, 1)}
        ctx.stop();
        Ok(())
    });
    CORE.spawn (move |_| stop_f);
    rpc_response (200, r#"{"result": "success"}"#)
}
/*
        else if ( strcmp(method,"millis") == 0 )
        {
            LP_millistats_update(0);
            return(clonestr("{\"result\":\"success\"}"));
        }
        else if ( strcmp(method,"sleep") == 0 )
        {
            if ( jint(argjson,"seconds") == 0 )
                sleep(180);
            else sleep(jint(argjson,"seconds"));
            return(clonestr("{\"result\":\"success\",\"status\":\"feeling good after sleeping\"}"));
        }
        else if ( strcmp(method,"getprices") == 0 )
            return(LP_prices());
        else if ( strcmp(method,"getpeers") == 0 )
            return(LP_peers());
        else if ( strcmp(method,"getcoins") == 0 )
            return(jprint(LP_coinsjson(0),1));  // Was also superficially ported in RPC `passphrase`.
        else if ( strcmp(method,"notarizations") == 0 )
        {
            if ( (ptr= LP_coinsearch(coin)) != 0 )
            {
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"result","success");
                jaddstr(retjson,"coin",coin);
                jaddnum(retjson,"lastnotarization",ptr->notarized);
                jaddnum(retjson,"bestheight",ptr->height);
                return(jprint(retjson,1));
            } else return(clonestr("{\"error\":\"cant find coin\"}"));
        }
        else if ( strcmp(method,"portfolio") == 0 )
        {
            return(LP_portfolio());
        }
        else if ( strcmp(method,"calcaddress") == 0 )
        {
            bits256 privkey,pub; uint8_t pubtype,wiftaddr,p2shtype,taddr,wiftype,pubkey33[33]; char *passphrase,coinaddr[64],wifstr[64],pubsecp[67];
            if ( (passphrase= jstr(argjson,"passphrase")) != 0 )
            {
                conv_NXTpassword(privkey.bytes,pub.bytes,(uint8_t *)passphrase,(int32_t)strlen(passphrase));
                privkey.bytes[0] &= 248, privkey.bytes[31] &= 127, privkey.bytes[31] |= 64;
                if ( (coin= jstr(argjson,"coin")) == 0 || (ptr= LP_coinfind(coin)) == 0 )
                {
                    coin = "KMD";
                    taddr = 0;
                    pubtype = 60;
                    p2shtype = 85;
                    wiftype = 188;
                    wiftaddr = 0;
                }
                else
                {
                    coin = ptr->symbol;
                    taddr = ptr->taddr;
                    pubtype = ptr->pubtype;
                    p2shtype = ptr->p2shtype;
                    wiftype = ptr->wiftype;
                    wiftaddr = ptr->wiftaddr;
                }
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"passphrase",passphrase);
                bitcoin_priv2pub(ctx,coin,pubkey33,coinaddr,privkey,taddr,pubtype);
                init_hexbytes_noT(pubsecp,pubkey33,33);
                jaddstr(retjson,"pubsecp",pubsecp);
                jaddstr(retjson,"coinaddr",coinaddr);
                bitcoin_priv2pub(ctx,coin,pubkey33,coinaddr,privkey,taddr,p2shtype);
                jaddstr(retjson,"p2shaddr",coinaddr);
                jaddbits256(retjson,"privkey",privkey);
                bitcoin_priv2wif(coin,wiftaddr,wifstr,privkey,wiftype);
                jaddstr(retjson,"wif",wifstr);
                return(jprint(retjson,1));
            } else return(clonestr("{\"error\":\"need to have passphrase\"}"));
        }
        else if ( strcmp(method,"statsdisp") == 0 )
        {
            return(jprint(LP_statslog_disp(juint(argjson,"starttime"),juint(argjson,"endtime"),jstr(argjson,"gui"),jbits256(argjson,"pubkey"),jstr(argjson,"base"),jstr(argjson,"rel")),1));
        }
        else if ( strcmp(method,"ticker") == 0 )
        {
            return(LP_ticker(jstr(argjson,"base"),jstr(argjson,"rel")));
        }
        else if ( strcmp(method,"gen64addrs") == 0 )
        {
            uint8_t taddr,pubtype;
            pubtype = (jobj(argjson,"pubtype") == 0) ? 60 : juint(argjson,"pubtype");
            taddr = (jobj(argjson,"taddr") == 0) ? 0 : juint(argjson,"taddr");
            return(LP_gen64addrs(ctx,jstr(argjson,"passphrase"),taddr,pubtype));
        }
        else if ( strcmp(method,"secretaddresses") == 0 )
        {
            uint8_t taddr,pubtype;
            pubtype = (jobj(argjson,"pubtype") == 0) ? 60 : juint(argjson,"pubtype");
            taddr = (jobj(argjson,"taddr") == 0) ? 0 : juint(argjson,"taddr");
            return(LP_secretaddresses(ctx,jstr(argjson,"prefix"),jstr(argjson,"passphrase"),juint(argjson,"num"),taddr,pubtype));
        }
        else if ( strcmp(method,"kickstart") == 0 )
        {
            uint32_t requestid,quoteid;
            if ( (requestid= juint(argjson,"requestid")) != 0 && (quoteid= juint(argjson,"quoteid")) != 0 )
                return(LP_kickstart(requestid,quoteid));
            else return(clonestr("{\"error\":\"kickstart needs requestid and quoteid\"}"));
        }
        else if ( strcmp(method,"dynamictrust") == 0 )
        {
            struct LP_address *ap; char *coinaddr;
            if ( (ptr= LP_coinsearch("KMD")) != 0 && (coinaddr= jstr(argjson,"address")) != 0 )
            {
                //LP_zeroconf_deposits(ptr);
                if ( (ap= LP_addressfind(ptr,coinaddr)) != 0 )
                {
                    retjson = cJSON_CreateObject();
                    jaddstr(retjson,"result","success");
                    jaddstr(retjson,"address",coinaddr);
                    jaddnum(retjson,"zcredits",dstr(ap->instantdex_credits));
                    return(jprint(retjson,1));
                }
            }
            return(clonestr("{\"error\":\"cant find address\"}"));
        }
        else if ( strcmp(method,"inuse") == 0 )
            return(jprint(LP_inuse_json(),1));
        else if ( (retstr= LP_istradebots_command(ctx,pubsock,method,argjson)) != 0 )
            return(retstr);
        if ( base[0] != 0 && rel[0] != 0 )
        {
            double price,bid,ask;

            else if ( strcmp(method,"pricearray") == 0 )
            {
                uint32_t firsttime;
                if ( base[0] != 0 && rel[0] != 0 )
                {
                    if ( (firsttime= juint(argjson,"starttime")) < time(NULL)-30*24*3600 )
                        firsttime = (uint32_t)(time(NULL)-30*24*3600);
                    return(jprint(LP_pricearray(base,rel,firsttime,juint(argjson,"endtime"),jint(argjson,"timescale")),1));
                } else return(clonestr("{\"error\":\"pricearray needs base and rel\"}"));
            }
            else if ( strcmp(method,"tradesarray") == 0 )
            {
                return(jprint(LP_tradesarray(base,rel,juint(argjson,"starttime"),juint(argjson,"endtime"),jint(argjson,"timescale")),1));
            }
            else if ( strcmp(method,"getprice") == 0 || strcmp(method,"getmyprice") == 0 )
            {
                double price,bid,ask;
                if ( strcmp(method,"getprice") == 0 )
                {
                    ask = LP_price(1,base,rel);
                    if ( (bid= LP_price(1,rel,base)) > SMALLVAL )
                        bid = 1./bid;
                }
                else
                {
                    ask = LP_getmyprice(1,base,rel);
                    if ( (bid= LP_getmyprice(1,rel,base)) > SMALLVAL )
                        bid = 1./bid;
                }
                price = _pairaved(bid,ask);
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"result","success");
                jaddstr(retjson,"base",base);
                jaddstr(retjson,"rel",rel);
                jaddnum(retjson,"timestamp",time(NULL));
                jaddnum(retjson,"bid",bid);
                jaddnum(retjson,"ask",ask);
                jaddnum(retjson,"price",price);
                return(jprint(retjson,1));
            }
            else if ( strcmp(method,"orderbook") == 0 )
                return(LP_orderbook(base,rel,jint(argjson,"duration")));
            if ( IAMLP == 0 && LP_isdisabled(base,rel) != 0 )
                return(clonestr("{\"error\":\"at least one of coins disabled\"}"));
            price = jdouble(argjson,"price");
            if ( strcmp(method,"setprice") == 0 )
            {
                if ( LP_mypriceset(1,&changed,base,rel,price) < 0 )
                    return(clonestr("{\"error\":\"couldnt set price\"}"));
                //else if ( LP_mypriceset(1,&changed,rel,base,1./price) < 0 )
                //    return(clonestr("{\"error\":\"couldnt set price\"}"));
                else if ( price == 0. || jobj(argjson,"broadcast") == 0 || jint(argjson,"broadcast") != 0 )
                    return(LP_pricepings(ctx,myipaddr,LP_mypubsock,base,rel,price * LP_profitratio));
                else return(clonestr("{\"result\":\"success\"}"));
            }
            else if ( strcmp(method,"myprice") == 0 )
            {
                if ( LP_myprice(1,&bid,&ask,base,rel) > SMALLVAL )
                {
                    retjson = cJSON_CreateObject();
                    jaddstr(retjson,"base",base);
                    jaddstr(retjson,"rel",rel);
                    jaddnum(retjson,"bid",bid);
                    jaddnum(retjson,"ask",ask);
                    return(jprint(retjson,1));
                } else return(clonestr("{\"error\":\"no price set\"}"));
            }
*/
/*
        else if ( coin[0] != 0 )
        {
            if ( strcmp(method,"disable") == 0 )
            {
                //*
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    ptr->inactive = (uint32_t)time(NULL);
                    cJSON *array = cJSON_CreateArray();
                    jaddi(array,LP_coinjson(ptr,0));
                    return(jprint(array,1));
                } else return(clonestr("{\"error\":\"couldnt find coin\"}"));
            }
            else if ( strcmp(method,"listunspent") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    char *coinaddr; bits256 zero;
                    memset(zero.bytes,0,sizeof(zero));
                    if ( (coinaddr= jstr(argjson,"address")) != 0 )
                    {
                        if ( coinaddr[0] != 0 )
                        {
                            LP_address(ptr,coinaddr);
                            if ( strcmp(coinaddr,ptr->smartaddr) == 0 && bits256_nonz(G.LP_privkey) != 0 )
                            {
                                LP_listunspent_issue(coin,coinaddr,2,zero,zero);
                                //LP_privkey_init(-1,ptr,G.LP_privkey,G.LP_mypub25519);
                            }
                            return(jprint(LP_listunspent(coin,coinaddr,zero,zero),1));
                        }
                    }
                    return(clonestr("{\"error\":\"no address specified\"}"));
                } else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"balance") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(jprint(LP_address_balance(ptr,jstr(argjson,"address"),1),1));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"getfee") == 0 )
            {
                uint64_t txfee;
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    txfee = LP_txfeecalc(ptr,0,0);
                    retjson = cJSON_CreateObject();
                    jaddstr(retjson,"result","success");
                    jaddstr(retjson,"coin",coin);
                    jaddnum(retjson,"txfee",dstr(txfee));
                    return(jprint(retjson,1));
                } else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"sendrawtransaction") == 0 )
            {
                return(LP_sendrawtransaction(coin,jstr(argjson,"signedtx"),jint(argjson,"needjson")));
            }
            else if ( strcmp(method,"convaddress") == 0 )
            {
                return(LP_convaddress(coin,jstr(argjson,"address"),jstr(argjson,"destcoin")));
            }
            else if ( strcmp(method,"timelock") == 0 )
            {
                return(LP_timelock(coin,juint(argjson,"duration"),jstr(argjson,"destaddr"),jdouble(argjson,"amount")*SATOSHIDEN));
            }
            else if ( strcmp(method,"opreturndecrypt") == 0 )
            {
                return(LP_opreturndecrypt(ctx,coin,jbits256(argjson,"txid"),jstr(argjson,"passphrase")));
            }
            else if ( strcmp(method,"unlockedspend") == 0 )
            {
                return(LP_unlockedspend(ctx,coin,jbits256(argjson,"txid")));
            }
            // cJSON *LP_listtransactions(char *symbol,char *coinaddr,int32_t count,int32_t skip)
            else if ( strcmp(method,"listtransactions") == 0 )
            {
                if ( (ptr= LP_coinfind(coin)) != 0 )
                    return(jprint(LP_listtransactions(coin,jstr(argjson,"address"),juint(argjson,"count"),juint(argjson,"skip")),1));
            }
            else if ( strcmp(method,"getrawtransaction") == 0 )
            {
                return(jprint(LP_gettx("stats_JSON",coin,jbits256(argjson,"txid"),0),1));
            }
            else if ( strcmp(method,"txblast") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(LP_txblast(ptr,argjson));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"movecoinbases") == 0 )
            {
                return(LP_movecoinbases(coin));
            }
            else if ( strcmp(method,"withdraw") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    if ( jobj(argjson,"outputs") == 0 && jstr(argjson,"opreturn") == 0 )
                        return(clonestr("{\"error\":\"withdraw needs to have outputs\"}"));
                    else if ( ptr->etomic[0] != 0 )
                        return(clonestr("{\"error\":\"use eth_withdraw for ETH/ERC20\"}"));
                    else return(LP_withdraw(ptr,argjson));
                }
                return(clonestr("{\"error\":\"cant find coind\"}"));
            }
#ifndef NOTETOMIC
            else if ( strcmp(method,"eth_withdraw") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 ) {
                    return LP_eth_withdraw(ptr, argjson);
                }
            }
#endif
            else if ( strcmp(method,"setconfirms") == 0 )
            {
                int32_t n;
                n = jint(argjson,"numconfirms");
                if ( n < 0 )
                    return(clonestr("{\"error\":\"illegal numconfirms\"}"));
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    ptr->userconfirms = n;
                    if ( (n= jint(argjson,"maxconfirms")) > 0 )
                        ptr->maxconfirms = n;
                    if ( ptr->maxconfirms > 0 && ptr->userconfirms > ptr->maxconfirms )
                        ptr->userconfirms = ptr->maxconfirms;
                    return(clonestr("{\"result\":\"success\"}"));
                } else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"snapshot") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(jprint(LP_snapshot(ptr,juint(argjson,"height")),1));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"dividends") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(LP_dividends(ptr,juint(argjson,"height"),argjson));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"snapshot_balance") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(LP_snapshot_balance(ptr,juint(argjson,"height"),argjson));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            if ( LP_isdisabled(coin,0) != 0 )
            {
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"error",LP_DONTCHANGE_ERRMSG1);
                return(jprint(retjson,1));
            }
            */*/

pub fn inventory (ctx: MmArc, req: Json) -> HyRes {
    let ticker = match req["coin"].as_str() {Some (s) => s, None => return rpc_err_response (500, "No 'coin' argument in request")};
    let coin = match lp_coinfind (&ctx, ticker) {
        Ok (Some (t)) => t,
        Ok (None) => return rpc_err_response (500, &fomat! ("No such coin: " (ticker))),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind(" (ticker) "): " (err)))
    };
    let ii = coin.iguana_info();

    unsafe {lp::LP_address (ii, (*ii).smartaddr.as_mut_ptr())};
    if unsafe {lp::G.LP_privkey.nonz()} {
        unsafe {lp::LP_privkey_init (-1, ii, lp::G.LP_privkey, lp::G.LP_mypub25519)};
    } else {
        log! ("inventory] no LP_privkey");
    }
    let retjson = json! ({
        "result": "success",
        "coin": ticker,
        "timestamp": now_ms() / 1000,
        "alice": []  // LP_inventory(coin)
        // "bob": LP_inventory(coin,1)
    });
    //LP_smartutxos_push(ptr);
    rpc_response (200, try_h! (json::to_string (&retjson)))
}

            /*/*/*
            else if ( strcmp(method,"goal") == 0 )
                return(LP_portfolio_goal(coin,jdouble(argjson,"val")));
            else if ( strcmp(method,"getcoin") == 0 )
                return(LP_getcoin(coin));
        }
        else if ( strcmp(method,"goal") == 0 )
            return(LP_portfolio_goal("*",100.));
        else if ( strcmp(method,"lastnonce") == 0 )
        {
            cJSON *retjson = cJSON_CreateObject();
            jaddstr(retjson,"result","success");
            jaddnum(retjson,"lastnonce",LP_lastnonce);
            return(jprint(retjson,1));
        }
        else if ( strcmp(method,"myprices") == 0 )
            return(LP_myprices(1));
        else if ( strcmp(method,"trust") == 0 )
            return(LP_pubkey_trustset(jbits256(argjson,"pubkey"),jint(argjson,"trust")));
        else if ( strcmp(method,"trusted") == 0 )
            return(LP_pubkey_trusted());
    } // end of protected localhost commands
    if ( IAMLP == 0 )
    {
        if ( (reqjson= LP_dereference(argjson,"broadcast")) != 0 )
        {
            if ( jobj(reqjson,"method2") != 0 )
            {
                jdelete(reqjson,"method");
                method = jstr(reqjson,"method2");
                jaddstr(reqjson,"method",method);
            }
            argjson = reqjson;
        }
        if ( strcmp(method,"getdPoW") == 0 )
            retstr = clonestr("{\"result\":\"success\"}");
    }
    else
    {
        if ( strcmp(method,"tradesarray") == 0 )
        {
            return(jprint(LP_tradesarray(base,rel,juint(argjson,"starttime"),juint(argjson,"endtime"),jint(argjson,"timescale")),1));
        }
        else if ( strcmp(method,"getdPoW") == 0 )
        {
            if ( (ptr= LP_coinfind(jstr(argjson,"coin"))) != 0 )
                LP_dPoW_broadcast(ptr);
            retstr = clonestr("{\"result\":\"success\"}");
        }
    }
    // received response
    if ( strcmp(method,"swapstatus") == 0 )
        return(LP_swapstatus_recv(argjson));
    else if ( strcmp(method,"gettradestatus") == 0 )
    {
        retstr = clonestr("{\"error\":\"deprecated\"}");
        //return(LP_gettradestatus(j64bits(argjson,"aliceid"),juint(argjson,"requestid"),juint(argjson,"quoteid")));
    }
    else if ( strcmp(method,"postprice") == 0 )
        return(LP_postprice_recv(argjson));
    else if ( strcmp(method,"uitem") == 0 )
        return(LP_uitem_recv(argjson));
    else if ( strcmp(method,"dPoW") == 0 )
        return(LP_dPoW_recv(argjson));
    else if ( strcmp(method,"getpeers") == 0 )
        return(LP_peers());
    else if ( strcmp(method,"balances") == 0 )
        return(jprint(LP_balances(jstr(argjson,"address")),1));
    else if ( strcmp(method,"getprice") == 0 || strcmp(method,"getmyprice") == 0 )
    {
        double price,bid,ask;
        if ( strcmp(method,"getprice") == 0 )
        {
            ask = LP_price(1,base,rel);
            if ( (bid= LP_price(1,rel,base)) > SMALLVAL )
                bid = 1./bid;
        }
        else
        {
            ask = LP_getmyprice(1,base,rel);
            if ( (bid= LP_getmyprice(1,rel,base)) > SMALLVAL )
                bid = 1./bid;
        }
        price = _pairaved(bid,ask);
        retjson = cJSON_CreateObject();
        jaddstr(retjson,"result","success");
        jaddstr(retjson,"base",base);
        jaddstr(retjson,"rel",rel);
        jaddnum(retjson,"timestamp",time(NULL));
        jaddnum(retjson,"bid",bid);
        jaddnum(retjson,"ask",ask);
        jaddnum(retjson,"price",price);
        return(jprint(retjson,1));
    }
    /*else if ( strcmp(method,"getpeers") == 0 )
    {
        char *tmpstr;
        if ( (tmpstr= jstr(argjson,"LPnode")) != 0 )
            LP_addpeer(LP_mypeer,LP_mypubsock,tmpstr,RPC_port,RPC_port+10,RPC_port+20,1,G.LP_sessionid);
        if ( IAMLP != 0 )
        {
            printf("send peers list %s\n",LP_peers());
            bits256 zero; memset(zero.bytes,0,sizeof(zero));
            LP_reserved_msg(0,"","",zero,LP_peers());
        }
        retstr = clonestr("{\"result\":\"success\"}");
    }*/
    // end received response
    
    else if ( strcmp(method,"tradestatus") == 0 )
    {
        LP_tradecommand_log(argjson);
        //printf("%-4d tradestatus | aliceid.%llu RT.%d %d\n",(uint32_t)time(NULL) % 3600,(long long)j64bits(argjson,"aliceid"),LP_RTcount,LP_swapscount);
        retstr = clonestr("{\"result\":\"success\"}");
    }
    else if ( strcmp(method,"wantnotify") == 0 )
    {
        bits256 pub; static uint32_t lastnotify;
        pub = jbits256(argjson,"pub");
        //char str[65]; printf("got wantnotify.(%s) vs %s\n",jprint(argjson,0),bits256_str(str,G.LP_mypub25519));
        if ( bits256_cmp(pub,G.LP_mypub25519) == 0 && time(NULL) > lastnotify+60 )
        {
            lastnotify = (uint32_t)time(NULL);
            //printf("wantnotify for me!\n");
            LP_notify_pubkeys(ctx,LP_mypubsock);
        }
        retstr = clonestr("{\"result\":\"success\"}");
    }
    else if ( strcmp(method,"addr_unspents") == 0 )
    {
        //printf("GOT ADDR_UNSPENTS %s %s\n",jstr(argjson,"coin"),jstr(argjson,"address"));
        if ( (ptr= LP_coinsearch(coin)) != 0 )
        {
            char *coinaddr;
            if ( (coinaddr= jstr(argjson,"address")) != 0 )
            {
                if ( coinaddr[0] != 0 )
                {
                    LP_address(ptr,coinaddr);
                    if ( strcmp(coinaddr,ptr->smartaddr) == 0 && bits256_nonz(G.LP_privkey) != 0 )
                    {
                        //printf("ADDR_UNSPENTS %s %s is my address being asked for!\n",ptr->symbol,coinaddr);
                        if ( ptr->lastpushtime > 0 && ptr->addr_listunspent_requested > (uint32_t)time(NULL)-10 )
                            ptr->lastpushtime -= LP_ORDERBOOK_DURATION*0.1;
                        ptr->addr_listunspent_requested = (uint32_t)time(NULL);
                    }
                }
            }
        }
        retstr = clonestr("{\"result\":\"success\"}");
    }
    else if ( strcmp(method,"encrypted") == 0 )
        retstr = clonestr("{\"result\":\"success\"}");
    else // psock requests/response
    {
        if ( IAMLP != 0 )
        {
            if ( strcmp(method,"psock") == 0 )
            {
                int32_t psock;
                if ( myipaddr == 0 || myipaddr[0] == 0 || strcmp(myipaddr,"127.0.0.1") == 0 )
                {
                    if ( LP_mypeer != 0 )
                        myipaddr = LP_mypeer->ipaddr;
                    else printf("LP_psock dont have actual ipaddr?\n");
                }
                if ( jint(argjson,"ispaired") != 0 && jobj(argjson,"netid") != 0 && juint(argjson,"netid") == G.netid )
                {
                    retstr = LP_psock(&psock,myipaddr,1,jint(argjson,"cmdchannel"),jbits256(argjson,"pubkey"));
                    //printf("LP_commands.(%s)\n",retstr);
                    return(retstr);
                }
                else return(clonestr("{\"error\":\"you are running an obsolete version, update\"}"));
            }
        }
        else
        {
            if ( strcmp(method,"psock") == 0 )
            {
                //printf("nonLP got (%s)\n",jprint(argjson,0));
                retstr = clonestr("{\"result\":\"success\"}");
            }
        }
    }
    if ( retstr == 0 )
        printf("ERROR.(%s)\n",jprint(argjson,0));
    if ( reqjson != 0 )
        free_json(reqjson);
    if ( retstr != 0 )
    {
        free(retstr);
        return(0);
    }
    return(0);
}
*/
*/*/
