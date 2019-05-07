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
//  lp_native_dex.rs
//  marketmaker
//

use futures::{Future};
use futures::sync::oneshot::Sender;
use gstuff::now_ms;
use hex;
use hyper::StatusCode;
use libc::{self, c_char, c_void};
use rand::{random, Rng, SeedableRng};
use rand::rngs::SmallRng;
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;
use std::fs;
use std::ffi::{CStr, CString};
use std::io::{Cursor, Read, Write};
use std::mem::{transmute, zeroed};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::Path;
use std::ptr::null_mut;
use std::str;
use std::str::from_utf8;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, sleep};
use std::time::Duration;

use coins::utxo::{ChecksumType, compressed_key_pair_from_bytes};
use peers::http_fallback::new_http_fallback;
use portfolio::prices_loop;

use crate::common::{
    bits256, coins_iter, lp, lp_queue_command_for_c, os, slurp_url,
    CORE, CJSON, MM_VERSION};
use crate::common::log::TagParam;
use crate::common::mm_ctx::{MmCtx, MmArc};
use crate::mm2::lp_network::{lp_command_q_loop, seednode_loop, client_p2p_loop};
use crate::mm2::lp_ordermatch::{lp_trade_command, lp_trades_loop};
use crate::mm2::rpc::{self, SINGLE_THREADED_C_LOCK};

/*
#include <stdio.h>
#ifndef MM_VERSION
#define MM_VERSION "UNKNOWN"
#endif

long LP_cjson_allocated,LP_cjson_total,LP_cjson_count;

struct LP_millistats
{
    double lastmilli,millisum,threshold;
    uint32_t count;
    char name[64];
} LP_psockloop_stats,LP_reserved_msgs_stats,utxosQ_loop_stats,command_rpcloop_stats,queue_loop_stats,prices_loop_stats,LP_coinsloop_stats,LP_coinsloopBTC_stats,LP_coinsloopKMD_stats,LP_pubkeysloop_stats,LP_swapsloop_stats,LP_gcloop_stats,LP_tradesloop_stats;
extern int32_t IAMLP;
char LP_methodstr[64];

void LP_millistats_update(struct LP_millistats *mp)
{
    double elapsed,millis;
    if ( mp == 0 )
    {
        if ( IAMLP != 0 )
        {
            mp = &LP_psockloop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        }
        mp = &LP_reserved_msgs_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &utxosQ_loop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &command_rpcloop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &queue_loop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &prices_loop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &LP_coinsloop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &LP_coinsloopBTC_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &LP_coinsloopKMD_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &LP_pubkeysloop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &LP_tradesloop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &LP_swapsloop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
        mp = &LP_gcloop_stats, printf("%32s lag %10.2f millis, threshold %10.2f, ave %10.2f millis, count.%u\n",mp->name,OS_milliseconds() - mp->lastmilli,mp->threshold,mp->millisum/(mp->count > 0 ? mp->count: 1),mp->count);
    }
    else
    {
        if ( mp->lastmilli == 0. )
            mp->lastmilli = OS_milliseconds();
        else
        {
            mp->count++;
            millis = OS_milliseconds();
            elapsed = (millis - mp->lastmilli);
            mp->millisum += elapsed;
            if ( mp->threshold != 0. && elapsed > mp->threshold )
            {
                //if ( IAMLP == 0 )
                    printf("%32s elapsed %10.2f millis > threshold %10.2f, ave %10.2f millis, count.%u %s\n",mp->name,elapsed,mp->threshold,mp->millisum/mp->count,mp->count,LP_methodstr);
            }
            mp->lastmilli = millis;
        }
    }
}

#include "LP_include.h"

#ifndef NOTETOMIC
#include "LP_etomic.h"
#endif

portable_mutex_t LP_peermutex,LP_UTXOmutex,LP_utxomutex,LP_commandmutex,LP_cachemutex,LP_swaplistmutex,LP_forwardmutex,LP_pubkeymutex,LP_networkmutex,LP_psockmutex,LP_coinmutex,LP_messagemutex,LP_electrummutex,LP_butxomutex,LP_reservedmutex,LP_nanorecvsmutex,LP_tradebotsmutex,LP_gcmutex,LP_inusemutex,LP_cJSONmutex,LP_logmutex,LP_statslogmutex,LP_tradesmutex,LP_commandQmutex,LP_blockinit_mutex,LP_pendswap_mutex,LP_listmutex,LP_gtcmutex;
int32_t LP_canbind;
char *Broadcaststr,*Reserved_msgs[2][1000];
int32_t num_Reserved_msgs[2],max_Reserved_msgs[2];
struct LP_peerinfo  *LP_peerinfos,*LP_mypeer;
struct LP_forwardinfo *LP_forwardinfos;
struct iguana_info *LP_coins;
struct LP_pubkey_info *LP_pubkeyinfos;
struct rpcrequest_info *LP_garbage_collector;
struct LP_address_utxo *LP_garbage_collector2;
struct LP_trade *LP_trades,*LP_tradesQ;

//uint32_t LP_deadman_switch;
uint16_t LP_fixed_pairport;//,LP_publicport;
uint32_t LP_lastnonce,LP_swap_endcritical,LP_swap_critical,LP_RTcount,LP_swapscount;
int32_t LP_STOP_RECEIVED,LP_numactive_LP;//,LP_mybussock = -1;
int32_t LP_mypubsock = -1,IPC_ENDPOINT = -1;
int32_t LP_cmdcount,LP_mypullsock = -1;
int32_t LP_numfinished,LP_showwif,IAMLP = 0;
double LP_profitratio = 1.;

struct LP_privkey { bits256 privkey; uint8_t rmd160[20]; };

struct LP_globals
{
    //struct LP_utxoinfo  *LP_utxoinfos[2],*LP_utxoinfos2[2];
    bits256 LP_mypub25519,LP_privkey,LP_mypriv25519,LP_passhash;
    uint64_t LP_skipstatus[10000], LP_required_etomic_balance;
    uint16_t netid;
    uint8_t LP_myrmd160[20],LP_pubsecp[33];
    uint32_t LP_sessionid,counter,mpnet;
    int32_t LP_IAMLP,LP_pendingswaps,USERPASS_COUNTER,LP_numprivkeys,initializing,waiting,LP_numskips;
    char seednode[64],USERPASS[65],USERPASS_WIFSTR[64],LP_myrmd160str[41],gui[65],LP_NXTaddr[64];
    struct LP_privkey LP_privkeys[100];
} G;

uint32_t LP_rand()
{
    uint32_t retval;
    retval = rand();
    retval = (retval << 7) ^ (retval >> 17) ^ rand();
    retval = (retval << 13) ^ (retval >> 13) ^ rand();
    retval = (retval << 17) ^ (retval >> 7) ^ rand();
    return(retval);
}

#include "LP_network.c"

char GLOBAL_DBDIR[] = { "DB" };
char LP_myipaddr[64],USERHOME[512] = { "/root" };
char LP_gui[65] = { "cli" };

// stubs

void tradebot_swap_balancingtrade(struct basilisk_swap *swap,int32_t iambob)
{
    
}

void tradebot_pendingadd(cJSON *tradejson,char *base,double basevolume,char *rel,double relvolume)
{
    // add to trades
}

char *LP_getdatadir()
{
    return(USERHOME);
}

char *blocktrail_listtransactions(char *symbol,char *coinaddr,int32_t num,int32_t skip)
{
    return(0);
}

#include "LP_mmjson.c"
#include "LP_socket.c"
#include "LP_secp.c"
#include "LP_bitcoin.c"
#include "LP_coins.c"
#include "LP_rpc.c"
#include "LP_mpnet.c"
#include "LP_NXT.c"
#include "LP_cache.c"
#include "LP_RTmetrics.c"
#include "LP_utxo.c"
#include "LP_prices.c"
#include "LP_scan.c"
#include "LP_transaction.c"
#include "LP_stats.c"
#include "LP_remember.c"
#include "LP_instantdex.c"
#include "LP_swap.c"
#include "LP_peers.c"
#include "LP_privkey.c"
#include "LP_forwarding.c"
#include "LP_signatures.c"
#include "LP_ordermatch.c"
#include "LP_tradebots.c"
#include "LP_messages.c"
#include "LP_commands.c"
*/

/// Process a previously queued command that wasn't handled by the RPC `dispatcher`.  
/// NB: It might be preferable to port more commands into the RPC `dispatcher`, rather than `lp_command_process`, because:  
/// 1) It allows us to more easily test such commands through the local HTTP endpoint;  
/// 2) It allows the command handler to run asynchronously and use more time wihtout slowing down the queue loop;  
/// 3) By being present in the `dispatcher` table the commands are easier to find and to be accounted for;  
/// 4) No need for `unsafe`, `CJSON` and `*mut c_char` there.
pub unsafe fn lp_command_process(
    ctx: MmArc,
    pub_sock: i32,
    json: Json,
    c_json: CJSON,
    stats_json_only: i32,
) -> *mut libc::c_char {
    let my_ip = fomat!((unwrap!(ctx.rpc_ip_port()).ip()) '\0');
    if !json["result"].is_null() || !json["error"].is_null() {
        null_mut()
    } else {
        if std::env::var("LOG_COMMANDS").is_ok() {
            log!("Got command: " [json]);
        }
        let _lock = SINGLE_THREADED_C_LOCK.lock();
        let mut trade_command = -1;
        if stats_json_only == 0 {
            trade_command = lp_trade_command(ctx.clone(), json, &c_json);
        }
        if trade_command <= 0 {
            lp::stats_JSON(
                ctx.btc_ctx() as *mut c_void,
                my_ip.as_ptr() as *mut c_char,
                pub_sock,
                c_json.0,
                b"127.0.0.1\x00" as *const u8 as *const libc::c_char as *mut libc::c_char,
                stats_json_only as u16,
                0,
                1,
            )
        } else {
            null_mut()
        }
    }
}

/*
char *LP_decrypt(uint8_t decoded[LP_ENCRYPTED_MAXSIZE + crypto_box_ZEROBYTES],uint8_t *ptr,int32_t *recvlenp)
{
    uint8_t *nonce,*cipher; int32_t recvlen,cipherlen; char *jsonstr = 0;
    recvlen = *recvlenp;
    nonce = &ptr[2];
    cipher = &ptr[2 + crypto_box_NONCEBYTES];
    cipherlen = recvlen - (2 + crypto_box_NONCEBYTES);
    if ( cipherlen > 0 && cipherlen <= LP_ENCRYPTED_MAXSIZE + crypto_box_ZEROBYTES )
    {
        if ( (jsonstr= (char *)_SuperNET_decipher(nonce,cipher,decoded,cipherlen,GENESIS_PUBKEY,G.LP_mypriv25519)) != 0 )
        {
            recvlen = (cipherlen - crypto_box_ZEROBYTES);
            if ( strlen(jsonstr)+1 != recvlen )
            {
                printf("unexpected len %d vs recvlen.%d\n",(int32_t)strlen(jsonstr)+1,recvlen);
                jsonstr = 0;
            } //else printf("decrypted (%s)\n",jsonstr);
        }
    } else printf("cipher.%d too big for %d\n",cipherlen,LP_ENCRYPTED_MAXSIZE + crypto_box_ZEROBYTES);
    *recvlenp = recvlen;
    return(jsonstr);
}

char *LP_process_message(void *ctx,char *typestr,char *myipaddr,int32_t pubsock,uint8_t *ptr,int32_t recvlen,int32_t recvsock)
{
    static uint32_t dup,uniq;
    uint8_t jdecoded[LP_ENCRYPTED_MAXSIZE + crypto_box_ZEROBYTES]; int32_t i,len,cipherlen,datalen=0,duplicate=0,encrypted=0; char *method,*method2,*tmp,*cipherstr,*retstr=0,*jsonstr=0; cJSON *argjson; uint32_t crc32;
    //double millis = OS_milliseconds();
    crc32 = calc_crc32(0,&ptr[2],recvlen-2);
    if ( (crc32 & 0xff) == ptr[0] && ((crc32>>8) & 0xff) == ptr[1] )
        encrypted = 1;
    i = LP_crc32find(&duplicate,-1,crc32);
    if ( duplicate != 0 )
        dup++;
    else uniq++;
    portable_mutex_lock(&LP_commandmutex);
    if ( (LP_rand() % 100000) == 0 )
        printf("%s dup.%d (%u / %u) %.1f%% encrypted.%d recv.%u [%02x %02x] vs %02x %02x\n",typestr,duplicate,dup,dup+uniq,(double)100*dup/(dup+uniq),encrypted,crc32,ptr[0],ptr[1],crc32&0xff,(crc32>>8)&0xff);
    if ( duplicate == 0 )
    {
        if ( i >= 0 )
            LP_crc32find(&duplicate,i,crc32);
        if ( encrypted != 0 )
            jsonstr = LP_decrypt(jdecoded,ptr,&recvlen);
        else if ( (datalen= is_hexstr((char *)ptr,0)) > 0 )
        {
            datalen >>= 1;
            jsonstr = malloc(datalen + 1);
            decode_hex((void *)jsonstr,datalen,(char *)ptr);
            jsonstr[datalen] = 0;
        } else jsonstr = (char *)ptr;
        if ( jsonstr != 0 && (argjson= cJSON_Parse(jsonstr)) != 0 )
        {
            uint8_t decoded[LP_ENCRYPTED_MAXSIZE + crypto_box_ZEROBYTES];
            //printf("[%s]\n",jsonstr);
            cipherlen = 0;
            if ( (cipherstr= jstr(argjson,"cipher")) != 0 && (cipherlen= is_hexstr(cipherstr,0)) > 32 && cipherlen <= sizeof(decoded)*2 )
            {
                method2 = jstr(argjson,"method2");
                if ( (method= jstr(argjson,"method")) != 0 && (strcmp(method,"encrypted") == 0 ||(method2 != 0 && strcmp(method2,"encrypted") == 0)) )
                {
                    cipherlen >>= 1;
                    decode_hex(decoded,cipherlen,cipherstr);
                    crc32 = calc_crc32(0,&decoded[2],cipherlen-2);
                    if ( (tmp= LP_decrypt(jdecoded,decoded,&cipherlen)) != 0 )
                    {
                        jsonstr = tmp;
                        free_json(argjson);
                        argjson = cJSON_Parse(jsonstr);
                        recvlen = cipherlen;
                        encrypted = 1;
                        if ( (crc32 & 0xff) == decoded[0] && ((crc32>>8) & 0xff) == decoded[1] )
                        {
                            i = LP_crc32find(&duplicate,-1,crc32);
                            if ( duplicate == 0 && i >= 0 )
                                LP_crc32find(&duplicate,i,crc32);
                        }
                        printf("%02x %02x %08x duplicate.%d decrypted.(%s)\n",decoded[0],decoded[1],crc32,duplicate,jsonstr);
                    }
                    else
                    {
                        //printf("packet not for this node %u\n",crc32);
                    }
                } else printf("error (%s) method is %s\n",jsonstr,method);
            }
            if ( jsonstr != 0 && argjson != 0 )
            {
                len = (int32_t)strlen(jsonstr) + 1;
                if ( (method= jstr(argjson,"method")) != 0 && strcmp(method,"gettradestatus") != 0 && strcmp(method,"psock") != 0 && strcmp(method,"broadcast") == 0 )
                {
                    bits256 zero; cJSON *reqjson; char *cipherstr; int32_t cipherlen; uint8_t cipher[LP_ENCRYPTED_MAXSIZE];
                    if ( (reqjson= LP_dereference(argjson,"broadcast")) != 0 )
                    {
                        Broadcaststr = jprint(reqjson,0);
                        if ( (cipherstr= jstr(reqjson,"cipher")) != 0 )
                        {
                            cipherlen = (int32_t)strlen(cipherstr) >> 1;
                            if ( cipherlen <= sizeof(cipher) )
                            {
                                decode_hex(cipher,cipherlen,cipherstr);
                                LP_queuesend(calc_crc32(0,&cipher[2],cipherlen-2),LP_mypubsock,"","",cipher,cipherlen);
                            } else retstr = clonestr("{\"error\":\"cipher too big\"}");
                        }
                        else
                        {
                            memset(zero.bytes,0,sizeof(zero));
                            if ( 0 && (method= jstr(reqjson,"method")) != 0 && (strcmp(method,"tradestatus") == 0) )
                                    printf("broadcast.(%s)\n",Broadcaststr);
                            LP_reserved_msg(0,"","",zero,jprint(reqjson,0));
                        }
                        retstr = clonestr("{\"result\":\"success\"}");
                        free_json(reqjson);
                    } else retstr = clonestr("{\"error\":\"couldnt dereference sendmessage\"}");
                }
                else
                {
                    LP_queuecommand(0,jsonstr,pubsock,0,0);
                    //if ( (retstr= LP_command_process(ctx,myipaddr,pubsock,argjson,&((uint8_t *)ptr)[len],recvlen - len)) != 0 )
                    //{
                    //}
                }
            }
            if ( argjson != 0 )
                free_json(argjson);
        }
    } //else printf("DUPLICATE.(%s)\n",(char *)ptr);
    portable_mutex_unlock(&LP_commandmutex);
    if ( jsonstr != 0 && (void *)jsonstr != (void *)ptr && encrypted == 0 )
        free(jsonstr);
    return(retstr);
}

int32_t LP_sock_check(char *typestr,void *ctx,char *myipaddr,int32_t pubsock,int32_t sock,char *remoteaddr,int32_t maxdepth)
{
    static char *line;
    int32_t recvlen=1,msglen,nonz = 0; cJSON *recvjson; void *ptr,*msg; char methodstr[64],*decodestr,*retstr,*str; struct nn_pollfd pfd;
    if ( line == 0 )
        line = calloc(1,1024*1024);
    if ( sock >= 0 )
    {
        while ( nonz < maxdepth && recvlen > 0 )
        {
            decodestr = 0;
            nonz++;
            memset(&pfd,0,sizeof(pfd));
            pfd.fd = sock;
            pfd.events = NN_POLLIN;
            if ( nn_poll(&pfd,1,1) != 1 )
                break;
            ptr = 0;
            if ( (recvlen= nn_recv(sock,&ptr,NN_MSG,0)) > 0 )
            {
                //printf("%s nn_recv.%d\n",typestr,recvlen);
                decodestr = 0;
                if ( recvlen > 32768 )
                {
                    printf("unexpectedly large packet\n");
                }
                else
                {
                    msg = ptr;
                    msglen = recvlen;
                    if ( (recvjson= cJSON_Parse((char *)ptr)) == 0 )
                    {
                        if ( (decodestr= MMJSON_decode(ptr,recvlen)) != 0 )
                        {
                            if ( (recvjson= cJSON_Parse(decodestr)) != 0 )
                            {
                                msg = decodestr;
                                msglen = (int32_t)strlen(decodestr) + 1;
                            }
                            //printf("decoded.(%s)\n",decodestr);
                        } else printf("couldnt decode linebuf[%d]\n",recvlen);
                    }
                    methodstr[0] = 0;
                    if ( recvjson != 0 )
                    {
                        safecopy(LP_methodstr,jstr(recvjson,"method"),sizeof(LP_methodstr));
                        free_json(recvjson);
                    }
                    int32_t validreq = 1;
                    /*if ( strlen((char *)ptr)+sizeof(bits256) <= recvlen )
                     {
                     if ( LP_magic_check(ptr,recvlen,remoteaddr) <= 0 )
                     {
                     //printf("magic check error\n");
                     } else validreq = 1;
                     recvlen -= sizeof(bits256);
                     }*/
                    if ( validreq != 0 )
                    {
                        if ( (retstr= LP_process_message(ctx,typestr,myipaddr,pubsock,msg,msglen,sock)) != 0 )
                            free(retstr);
                        
                        if ( Broadcaststr != 0 )
                        {
                            //printf("self broadcast.(%s)\n",Broadcaststr);
                            str = Broadcaststr;
                            Broadcaststr = 0;
                            LP_queuecommand(0,str,pubsock,0,0);
                            /*if ( (argjson= cJSON_Parse(str)) != 0 )
                            {
                                //portable_mutex_lock(&LP_commandmutex);
                                if ( LP_tradecommand(0,ctx,myipaddr,pubsock,argjson,0,0) <= 0 )
                                {
                                    if ( (retstr= stats_JSON(ctx,0,myipaddr,pubsock,argjson,remoteaddr,0)) != 0 )
                                        free(retstr);
                                }
                                //portable_mutex_unlock(&LP_commandmutex);
                                free_json(argjson);
                            }*/
                            free(str);
                        }
                    }
                }
            }
            if ( ptr != 0 )
            {
                nn_freemsg(ptr), ptr = 0;
                //free(buf);
            }
            if ( decodestr != 0 )
                free(decodestr);
        }
    }
    return(nonz);
}

int32_t LP_nanomsg_recvs(void *ctx)
{
    int32_t n=0,nonz = 0; char *origipaddr; struct LP_peerinfo *peer,*tmp;
    if ( (origipaddr= LP_myipaddr) == 0 )
        origipaddr = "127.0.0.1";
    portable_mutex_lock(&LP_nanorecvsmutex);
    HASH_ITER(hh,LP_peerinfos,peer,tmp)
    {
        if ( n++ > 0 && peer->errors >= LP_MAXPEER_ERRORS )
        {
            if ( (LP_rand() % 10000) == 0 )
                peer->errors--;
            else
            {
                //printf("skip %s\n",peer->ipaddr);
                continue;
            }
        }
        nonz += LP_sock_check("SUB",ctx,origipaddr,LP_mypubsock,peer->subsock,peer->ipaddr,1);
    }
    /*HASH_ITER(hh,LP_coins,coin,ctmp) // firstrefht,firstscanht,lastscanht
     {
     if ( coin->inactive != 0 )
     continue;
     if ( coin->bussock >= 0 )
     nonz += LP_sock_check(coin->symbol,ctx,origipaddr,-1,coin->bussock,LP_profitratio - 1.);
     }*/
    if ( LP_mypullsock >= 0 )
    {
        nonz += LP_sock_check("PULL",ctx,origipaddr,-1,LP_mypullsock,"127.0.0.1",1);
    }
    portable_mutex_unlock(&LP_nanorecvsmutex);
    //if ( G.mpnet != 0 )
        LP_mpnet_check(ctx,origipaddr,LP_mypubsock);
    return(nonz);
}

void command_rpcloop(void *ctx)
{
    int32_t nonz = 0;
    strcpy(command_rpcloop_stats.name,"command_rpcloop");
    command_rpcloop_stats.threshold = 2500.;
    while ( LP_STOP_RECEIVED == 0 )
    {
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        LP_millistats_update(&command_rpcloop_stats);
        nonz = LP_nanomsg_recvs(ctx);
        //if ( LP_mybussock >= 0 )
        //    nonz += LP_sock_check("BUS",ctx,origipaddr,-1,LP_mybussock);
        if ( nonz == 0 )
        {
            if ( IAMLP != 0 )
                usleep(10000);
            else usleep(50000);
        }
        else if ( IAMLP == 0 )
            usleep(1000);
    }
}

void LP_coinsloop(void *_coins)
{
    static int32_t didfilescreate;
    struct LP_address *ap=0; struct LP_transaction *tx; cJSON *retjson; struct LP_address_utxo *up,*tmp; struct iguana_info *coin,*ctmp; char str[65],*retstr,*hexstr,*txidstr; struct electrum_info *ep,*backupep=0; bits256 zero; int32_t notarized,oldht,j,nonz; char *coins = _coins;
    if ( strcmp("BTC",coins) == 0 )
    {
        strcpy(LP_coinsloopBTC_stats.name,"BTC coin loop");
        LP_coinsloopBTC_stats.threshold = 200000.;
    }
    else if ( strcmp("KMD",coins) == 0 )
    {
        strcpy(LP_coinsloopKMD_stats.name,"KMD coin loop");
        LP_coinsloopKMD_stats.threshold = 100000.;
    }
    else
    {
        strcpy(LP_coinsloop_stats.name,"other coins loop");
        LP_coinsloop_stats.threshold = 50000.;
    }
    while ( LP_STOP_RECEIVED == 0 )
    {
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        if ( strcmp("BTC",coins) == 0 )
            LP_millistats_update(&LP_coinsloopBTC_stats);
        else if ( strcmp("KMD",coins) == 0 )
            LP_millistats_update(&LP_coinsloopKMD_stats);
        else LP_millistats_update(&LP_coinsloop_stats);
        nonz = 0;
        HASH_ITER(hh,LP_coins,coin,ctmp) // firstrefht,firstscanht,lastscanht
        {
            if ( coins != 0 )
            {
                if ( coins[0] != 0 )
                {
                    if ( strcmp(coins,coin->symbol) != 0 )
                        continue;
                }
                else // avoid hardcode special case LP_coinsloop
                {
                    if ( strcmp("BTC",coin->symbol) == 0 || strcmp("KMD",coin->symbol) == 0 )
                        continue;
                }
            }
            if ( coin->smartaddr[0] == 0 )
            {
                //printf("%s has no smartaddress??\n",coin->symbol);
                continue;
            }
            if ( didfilescreate == 0 && strcmp("KMD",coin->symbol) == 0 )
            {
                LP_instantdex_filescreate(coin->smartaddr);
                didfilescreate = 1;
            }
            memset(&zero,0,sizeof(zero));
            if ( coin->inactive != 0 )
                continue;
            if ( coin->did_addrutxo_reset == 0 )
            {
                int32_t num;
                LP_address_utxo_reset(&num,coin);
                coin->did_addrutxo_reset = 1;
            }
            //free_json(LP_address_balance(coin,coin->smartaddr,1)); expensive invoking gettxout
            if ( coin->do_autofill_merge != 0 )
            {
                if ( (retstr= LP_autofillbob(coin,coin->do_autofill_merge*1.02)) != 0 )
                {
                    if ( (retjson= cJSON_Parse(retstr)) != 0 )
                    {
                        if ( (hexstr= jstr(retjson,"hex")) != 0 )
                        {
                            if ( (txidstr= LP_sendrawtransaction(coin->symbol,hexstr,0)) != 0 )
                            {
                                printf("autofill created %s\n",txidstr);
                                free(txidstr);
                                coin->fillsatoshis = coin->do_autofill_merge;
                                coin->do_autofill_merge = 0;
                                coin->bobfillheight = LP_getheight(&notarized,coin);
                            }
                        }
                        free_json(retjson);
                    }
                    free(retstr);
                }
            }
            if ( coin->longestchain == 1 ) // special init value
                coin->longestchain = LP_getheight(&notarized,coin);
            if ( (ep= coin->electrum) != 0 )
            {
                if ( (backupep= ep->prev) == 0 )
                    backupep = ep;
                if ( (retjson= electrum_address_listunspent(coin->symbol,ep,&retjson,coin->smartaddr,1,zero,zero)) != 0 )
                    free_json(retjson);
                if ( (ap= LP_addressfind(coin,coin->smartaddr)) != 0 )
                {
                    DL_FOREACH_SAFE(ap->utxos,up,tmp)
                    {
                        if ( up->U.height > 0 && up->spendheight < 0 )
                        {
                            if ( up->SPV == 0 )
                            {
                                nonz++;
                                up->SPV = LP_merkleproof(coin,coin->smartaddr,backupep,up->U.txid,up->U.height);
                                if ( up->SPV > 0 )
                                {
                                    if ( (tx= LP_transactionfind(coin,up->U.txid)) != 0 && tx->SPV == 0 )
                                    {
                                        tx->SPV = up->SPV;
                                        //printf("%s %s: SPV.%d\n",coin->symbol,bits256_str(str,up->U.txid),up->SPV);
                                    }
                                }
                            }
                            else if ( up->SPV == -1 )
                            {
                                nonz++;
                                printf("SPV failure for %s %s\n",coin->symbol,bits256_str(str,up->U.txid));
                                oldht = up->U.height;
                                LP_txheight_check(coin,ap->coinaddr,up->U.txid);
                                if ( oldht != up->U.height )
                                    up->SPV = LP_merkleproof(coin,coin->smartaddr,backupep,up->U.txid,up->U.height);
                                if ( up->SPV <= 0 )
                                    up->SPV = -2;
                                else printf("%s %s: corrected SPV.%d\n",coin->symbol,bits256_str(str,up->U.txid),up->SPV);
                            }
                        }
                    }
                }
                while ( ep != 0 )
                {
                    if ( time(NULL) > ep->keepalive+LP_ELECTRUM_KEEPALIVE )
                    {
                        //printf("%s electrum.%p needs a keepalive: lag.%d\n",ep->symbol,ep,(int32_t)(time(NULL) - ep->keepalive));
                        if ( (retjson= electrum_banner(coin->symbol,ep,&retjson)) != 0 )
                            free_json(retjson);
                        ep->keepalive = (uint32_t)time(NULL);
                    }
                    ep = ep->prev;
                }
                continue;
            }
            if ( coin->firstrefht == 0 )
                continue;
            else if ( coin->firstscanht == 0 )
                coin->lastscanht = coin->firstscanht = coin->firstrefht;
            else if ( coin->firstrefht < coin->firstscanht )
            {
                printf("detected %s firstrefht.%d < firstscanht.%d\n",coin->symbol,coin->firstrefht,coin->firstscanht);
                coin->lastscanht = coin->firstscanht = coin->firstrefht;
            }
            if ( coin->lastscanht == coin->longestchain+1 )
            {
                //printf("%s lastscanht.%d is longest.%d + 1\n",coin->symbol,coin->lastscanht,coin->longestchain);
                continue;
            }
            else if ( coin->lastscanht > coin->longestchain+1 )
            {
                printf("detected chain rewind lastscanht.%d vs longestchain.%d, first.%d ref.%d\n",coin->lastscanht,coin->longestchain,coin->firstscanht,coin->firstrefht);
                LP_undospends(coin,coin->longestchain-1);
                //LP_mempoolscan(coin->symbol,zero);
                coin->lastscanht = coin->longestchain - 1;
                if ( coin->firstscanht < coin->lastscanht )
                    coin->lastscanht = coin->firstscanht;
                continue;
            }
            //if ( strcmp(coin->symbol,"BTC") != 0 && strcmp(coin->symbol,"KMD") != 0 ) // SPV as backup
            if ( coin->lastscanht < coin->longestchain )
            {
                nonz++;
                if ( strcmp("BTC",coins) == 0 )//&& coin->lastscanht < coin->longestchain-3 )
                    printf("[%s]: %s ref.%d scan.%d to %d, longest.%d\n",coins,coin->symbol,coin->firstrefht,coin->firstscanht,coin->lastscanht,coin->longestchain);
                for (j=0; j<100; j++)
                {
                    if ( LP_blockinit(coin,coin->lastscanht) < 0 )
                    {
                        printf("please ignore this blockinit.%s %d error\n",coin->symbol,coin->lastscanht);
                        sleep(10);
                        break;
                    }
                    coin->lastscanht++;
                    if ( coin->lastscanht == coin->longestchain+1 || strcmp("BTC",coins) == 0 )
                        break;
                }
                if ( strcmp("BTC",coins) == 0 )
                    printf("done [%s]: %s ref.%d scan.%d to %d, longest.%d\n",coins,coin->symbol,coin->firstrefht,coin->firstscanht,coin->lastscanht,coin->longestchain);
            }
        }
        if ( coins == 0 )
            return;
        //if ( nonz == 0 )
            usleep(100000);
    }
}

int32_t LP_mainloop_iter(void *ctx,char *myipaddr,struct LP_peerinfo *mypeer,int32_t pubsock)
{
    static uint32_t counter;//,didinstantdex;
    struct iguana_info *coin,*ctmp; char *origipaddr; uint32_t now; int32_t notarized,height,nonz = 0;
    if ( (origipaddr= myipaddr) == 0 )
        origipaddr = "127.0.0.1";
    if ( mypeer == 0 )
        myipaddr = "127.0.0.1";
    HASH_ITER(hh,LP_coins,coin,ctmp) // firstrefht,firstscanht,lastscanht
    {
        now = (uint32_t)time(NULL);
#ifdef bruteforce
        if ( IAMLP != 0 && coin->inactive == 0 && coin->electrum == 0 && didinstantdex == 0 && strcmp("KMD",coin->symbol) == 0 )
        {
            LP_instantdex_deposits(coin);
            didinstantdex = now;
        }
#endif
        /*if ( (coin->addr_listunspent_requested != 0 && now > coin->lastpushtime+LP_ORDERBOOK_DURATION*.5) || now > coin->lastpushtime+LP_ORDERBOOK_DURATION*5 )
        {
            //printf("PUSH addr_listunspent_requested %u\n",coin->addr_listunspent_requested);
            coin->lastpushtime = (uint32_t)now;
            LP_smartutxos_push(coin);
            coin->addr_listunspent_requested = 0;
        }*/
        if ( coin->electrum == 0 && coin->inactive == 0 && now > coin->lastgetinfo+LP_GETINFO_INCR )
        {
            nonz++;
            if ( (height= LP_getheight(&notarized,coin)) > coin->longestchain )
            {
                coin->longestchain = height;
                if ( notarized != 0 && notarized > coin->notarized )
                {
                    coin->notarized = notarized;
                    if ( IAMLP != 0 )
                        LP_dPoW_broadcast(coin);
                }
                if ( 0 && coin->firstrefht != 0 )
                    printf(">>>>>>>>>> set %s longestchain %d (ref.%d [%d, %d])\n",coin->symbol,height,coin->firstrefht,coin->firstscanht,coin->lastscanht);
            } //else LP_mempoolscan(coin->symbol,zero);
            coin->lastgetinfo = (uint32_t)now;
        }
    }
    counter++;
    return(nonz);
}

int my_strncasecmp(const char *s1,const char *s2,size_t n)
{
    size_t i = 0;
    while ( i < n )
    {
        char c1 = s1[i];
        char c2 = s2[i];
        if ( c1 >= 'A' && c1 <= 'Z')
            c1 = (c1 - 'A') + 'a';
        if ( c2 >= 'A' && c2 <= 'Z')
            c2 = (c2 - 'A') + 'a';
        if ( c1 < c2 )
            return(-1);
        if ( c1 > c2 )
            return(1);
        if ( c1 == 0 )
            return(0);
        ++i;
    }
    return(0);
}

void bech32_tests()
{
    //char *test = "an83characterlonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1tt5tgs";
    //char *test = "bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a";
    //char *test = "bitcoincash:qr95sy3j9xwd2ap32xkykttr4cvcu7as4y0qverfuy";
    //char *test = "prefix:x64nx6hz";
    char *test = "bitcoincash:pq4p38fll9uuh2mzkesqhmgt66du4u0zzy92jchqqa"; // 35Xbnq3kLoNsjN67knFewiRc9fqewrCzMW
    uint8_t data[82],data2[64],rmd160[21],addrtype; char rebuild[92],hrp[84]; int32_t data_len,data_len2; int32_t i;
    if ( bech32_decode(hrp,data,&data_len,test) == 0 )
    {
        printf("bech32_decode fails: '%s'\n",test);
    }
    else
    {
        bitcoin_addr2rmd160("BCH",0,&addrtype,rmd160,"pq4p38fll9uuh2mzkesqhmgt66du4u0zzy92jchqqa");
        bitcoin_address("BTC",rebuild,0,5,rmd160,20);
        for (i=0; i<20; i++)
            printf("%02x",rmd160[i]);
        printf("addr2rmd160 %d -> %s\n",addrtype,rebuild);
        
        data_len2 = 0;
        if ( bech32_convert_bits(data2,&data_len2,8,data,data_len,5,0) == 0 )
            printf("error converting data5\n");
        for (i=0; i<data_len2; i++)
            printf("%02x",data2[i]);
        printf(" compacted 5's -> %d\n",data_len2);
        bitcoin_addr2rmd160("BTC",0,&addrtype,rmd160+1,"35Xbnq3kLoNsjN67knFewiRc9fqewrCzMW");
        for (i=0; i<data_len; i++)
            printf("%02x",data[i]);
        printf(" datalen.%d <- %s (%s) -> ",(int32_t)data_len,test,"35Xbnq3kLoNsjN67knFewiRc9fqewrCzMW");
        for (i=0; i<20; i++)
            printf("%02x",rmd160[i+1]);
        printf("\n");
    }
    data_len2 = 0;
    rmd160[0] = (1 << 3);
    bech32_convert_bits(data2,&data_len2,5,rmd160,21,8,1);
    for (i=0; i<data_len2; i++)
        printf("%02x",data2[i]);
    printf(" converted bits.%d\n",(int32_t)data_len2);
    if ( bech32_encode(rebuild,hrp,data2,data_len2) == 0 )
    {
        for (i=0; i<data_len; i++)
            printf("%02x",data[i]);
        printf(" bech32_encode fails: '%s' -> hrp.(%s) datalen.%d\n",test,hrp,(int32_t)data_len);
    }
    if ( my_strncasecmp(rebuild,test,92))
    {
        printf("bech32_encode produces incorrect result: '%s' vs (%s)\n",test,rebuild);
    }
    printf("end of bech32 tests\n");
}

char *Notaries_elected1[][4] =
{
    {"0dev1_jl777", "03b7621b44118017a16043f19b30cc8a4cfe068ac4e42417bae16ba460c80f3828", "RNJmgYaFF5DbnrNUX6pMYz9rcnDKC2tuAc", "GWsW2A1ud72KoKJZysVLtEAYmgYZZzbMxG" },
    {"0dev2_kolo", "030f34af4b908fb8eb2099accb56b8d157d49f6cfb691baa80fdd34f385efed961" },
    {"0dev3_kolo", "025af9d2b2a05338478159e9ac84543968fd18c45fd9307866b56f33898653b014" },
    {"0dev4_decker", "028eea44a09674dda00d88ffd199a09c9b75ba9782382cc8f1e97c0fd565fe5707" },
    {"a-team_SH", "03b59ad322b17cb94080dc8e6dc10a0a865de6d47c16fb5b1a0b5f77f9507f3cce" },
    {"artik_AR", "029acf1dcd9f5ff9c455f8bb717d4ae0c703e089d16cf8424619c491dff5994c90" },
    {"artik_EU", "03f54b2c24f82632e3cdebe4568ba0acf487a80f8a89779173cdb78f74514847ce" },
    {"artik_NA", "0224e31f93eff0cc30eaf0b2389fbc591085c0e122c4d11862c1729d090106c842" },
    {"artik_SH", "02bdd8840a34486f38305f311c0e2ae73e84046f6e9c3dd3571e32e58339d20937" },
    {"badass_EU", "0209d48554768dd8dada988b98aca23405057ac4b5b46838a9378b95c3e79b9b9e" },
    {"badass_NA", "02afa1a9f948e1634a29dc718d218e9d150c531cfa852843a1643a02184a63c1a7" }, // 10
    {"batman_AR", "033ecb640ec5852f42be24c3bf33ca123fb32ced134bed6aa2ba249cf31b0f2563" },
    {"batman_SH", "02ca5898931181d0b8aafc75ef56fce9c43656c0b6c9f64306e7c8542f6207018c" },
    {"ca333_EU", "03fc87b8c804f12a6bd18efd43b0ba2828e4e38834f6b44c0bfee19f966a12ba99" },
    {"chainmakers_EU", "02f3b08938a7f8d2609d567aebc4989eeded6e2e880c058fdf092c5da82c3bc5ee" },
    {"chainmakers_NA", "0276c6d1c65abc64c8559710b8aff4b9e33787072d3dda4ec9a47b30da0725f57a" },
    {"chainstrike_SH", "0370bcf10575d8fb0291afad7bf3a76929734f888228bc49e35c5c49b336002153" },
    {"cipi_AR", "02c4f89a5b382750836cb787880d30e23502265054e1c327a5bfce67116d757ce8" },
    {"cipi_NA", "02858904a2a1a0b44df4c937b65ee1f5b66186ab87a751858cf270dee1d5031f18" },
    {"crackers_EU", "03bc819982d3c6feb801ec3b720425b017d9b6ee9a40746b84422cbbf929dc73c3" },
    {"crackers_NA", "03205049103113d48c7c7af811b4c8f194dafc43a50d5313e61a22900fc1805b45" }, // 20
    {"dwy_EU", "0259c646288580221fdf0e92dbeecaee214504fdc8bbdf4a3019d6ec18b7540424" },
    {"emmanux_SH", "033f316114d950497fc1d9348f03770cd420f14f662ab2db6172df44c389a2667a" },
    {"etszombi_EU", "0281b1ad28d238a2b217e0af123ce020b79e91b9b10ad65a7917216eda6fe64bf7" },
    {"fullmoon_AR", "03380314c4f42fa854df8c471618751879f9e8f0ff5dbabda2bd77d0f96cb35676" },
    {"fullmoon_NA", "030216211d8e2a48bae9e5d7eb3a42ca2b7aae8770979a791f883869aea2fa6eef" },
    {"fullmoon_SH", "03f34282fa57ecc7aba8afaf66c30099b5601e98dcbfd0d8a58c86c20d8b692c64" },
    {"goldenman_EU", "02d6f13a8f745921cdb811e32237bb98950af1a5952be7b3d429abd9152f8e388d" },
    {"indenodes_AR", "02ec0fa5a40f47fd4a38ea5c89e375ad0b6ddf4807c99733c9c3dc15fb978ee147" },
    {"indenodes_EU", "0221387ff95c44cb52b86552e3ec118a3c311ca65b75bf807c6c07eaeb1be8303c" },
    {"indenodes_NA", "02698c6f1c9e43b66e82dbb163e8df0e5a2f62f3a7a882ca387d82f86e0b3fa988" }, // 30
    {"indenodes_SH", "0334e6e1ec8285c4b85bd6dae67e17d67d1f20e7328efad17ce6fd24ae97cdd65e" },
    {"jackson_AR", "038ff7cfe34cb13b524e0941d5cf710beca2ffb7e05ddf15ced7d4f14fbb0a6f69" },
    {"jeezy_EU", "023cb3e593fb85c5659688528e9a4f1c4c7f19206edc7e517d20f794ba686fd6d6" },
    {"karasugoi_NA", "02a348b03b9c1a8eac1b56f85c402b041c9bce918833f2ea16d13452309052a982" },
    {"komodoninja_EU", "038e567b99806b200b267b27bbca2abf6a3e8576406df5f872e3b38d30843cd5ba" },
    {"komodoninja_SH", "033178586896915e8456ebf407b1915351a617f46984001790f0cce3d6f3ada5c2" },
    {"komodopioneers_SH", "033ace50aedf8df70035b962a805431363a61cc4e69d99d90726a2d48fb195f68c" },
    {"libscott_SH", "03301a8248d41bc5dc926088a8cf31b65e2daf49eed7eb26af4fb03aae19682b95" },
    {"lukechilds_AR", "031aa66313ee024bbee8c17915cf7d105656d0ace5b4a43a3ab5eae1e14ec02696" },
    {"madmax_AR", "03891555b4a4393d655bf76f0ad0fb74e5159a615b6925907678edc2aac5e06a75" }, // 40
    {"meshbits_AR", "02957fd48ae6cb361b8a28cdb1b8ccf5067ff68eb1f90cba7df5f7934ed8eb4b2c" },
    {"meshbits_SH", "025c6e94877515dfd7b05682b9cc2fe4a49e076efe291e54fcec3add78183c1edb" },
    {"metaphilibert_AR", "02adad675fae12b25fdd0f57250b0caf7f795c43f346153a31fe3e72e7db1d6ac6" },
    {"metaphilibert_SH", "0284af1a5ef01503e6316a2ca4abf8423a794e9fc17ac6846f042b6f4adedc3309" },
    {"patchkez_SH", "0296270f394140640f8fa15684fc11255371abb6b9f253416ea2734e34607799c4" },
    {"pbca26_NA", "0276aca53a058556c485bbb60bdc54b600efe402a8b97f0341a7c04803ce204cb5" },
    {"peer2cloud_AR", "034e5563cb885999ae1530bd66fab728e580016629e8377579493b386bf6cebb15" },
    {"peer2cloud_SH", "03396ac453b3f23e20f30d4793c5b8ab6ded6993242df4f09fd91eb9a4f8aede84" },
    {"polycryptoblog_NA", "02708dcda7c45fb54b78469673c2587bfdd126e381654819c4c23df0e00b679622" },
    {"hyper_AR", "020f2f984d522051bd5247b61b080b4374a7ab389d959408313e8062acad3266b4" }, // 50
    {"hyper_EU", "03d00cf9ceace209c59fb013e112a786ad583d7de5ca45b1e0df3b4023bb14bf51" },
    {"hyper_SH", "0383d0b37f59f4ee5e3e98a47e461c861d49d0d90c80e9e16f7e63686a2dc071f3" },
    {"hyper_NA", "03d91c43230336c0d4b769c9c940145a8c53168bf62e34d1bccd7f6cfc7e5592de" },
    {"popcornbag_AR", "02761f106fb34fbfc5ddcc0c0aa831ed98e462a908550b280a1f7bd32c060c6fa3" },
    {"popcornbag_NA", "03c6085c7fdfff70988fda9b197371f1caf8397f1729a844790e421ee07b3a93e8" },
    {"alien_AR", "0348d9b1fc6acf81290405580f525ee49b4749ed4637b51a28b18caa26543b20f0" },
    {"alien_EU", "020aab8308d4df375a846a9e3b1c7e99597b90497efa021d50bcf1bbba23246527" },
    {"thegaltmines_NA", "031bea28bec98b6380958a493a703ddc3353d7b05eb452109a773eefd15a32e421" },
    {"titomane_AR", "029d19215440d8cb9cc6c6b7a4744ae7fb9fb18d986e371b06aeb34b64845f9325" },
    {"titomane_EU", "0360b4805d885ff596f94312eed3e4e17cb56aa8077c6dd78d905f8de89da9499f" }, // 60
    {"titomane_SH", "03573713c5b20c1e682a2e8c0f8437625b3530f278e705af9b6614de29277a435b" },
    {"webworker01_NA", "03bb7d005e052779b1586f071834c5facbb83470094cff5112f0072b64989f97d7" },
    {"xrobesx_NA", "03f0cc6d142d14a40937f12dbd99dbd9021328f45759e26f1877f2a838876709e1" },
};

void gameaddrs()
{
    struct iguana_info *gamecoin,*kmdcoin; int32_t i; uint8_t pubkey33[33]; char gameaddr[64],kmdaddr[64];
    gamecoin = LP_coinfind("GAME");
    kmdcoin = LP_coinfind("KMD");
    if ( gamecoin != 0 && kmdcoin != 0 )
    {
        for (i=0; i<64; i++)
        {
            decode_hex(pubkey33,33,Notaries_elected1[i][1]);
            bitcoin_address(gamecoin->symbol,gameaddr,gamecoin->taddr,gamecoin->pubtype,pubkey33,33);
            bitcoin_address(kmdcoin->symbol,kmdaddr,kmdcoin->taddr,kmdcoin->pubtype,pubkey33,33);
            printf("{\"%s\", \"%s\", \"%s\", \"%s\"},\n",Notaries_elected1[i][0],Notaries_elected1[i][1],kmdaddr,gameaddr);
        }
    }
}

*/

// TODO: Use MM2-nightly seed nodes.
//       MM1 nodes no longer compatible due to the UTXO reforms in particular.
//       We might also diverge in how we handle the p2p communication in the future.

/// Aka `default_LPnodes`. Initial nodes of the peer-to-peer network.
const P2P_SEED_NODES: [&'static str; 5] = [
    "5.9.253.195",
    "173.212.225.176",
    "136.243.45.140",
    "23.254.202.142",
    "45.32.19.196"
];

/// Default seed nodes for netid 9999 that is used for MM2 testing
const P2P_SEED_NODES_9999: [&'static str; 3] = [
    "195.201.116.176",
    "46.4.87.18",
    "46.4.78.11",
];

/// Setup the peer-to-peer network.
#[allow(unused_variables)]  // delme
pub unsafe fn lp_initpeers (ctx: &MmArc, pubsock: i32, mut mypeer: *mut lp::LP_peerinfo, myipaddr: &IpAddr, myport: u16,
                            netid: u16, seednodes: Option<Vec<String>>) -> Result<(), String> {
    // Pick our ports.
    let (mut pullport, mut pubport, mut busport) = (0, 0, 0);
    lp::LP_ports (&mut pullport, &mut pubport, &mut busport, netid);
    // Add ourselves into the list of known peers.
    try_s! (peers::initialize (ctx, netid, lp::G.LP_mypub25519, pubport + 1, lp::G.LP_sessionid));
    let myipaddr_c = try_s! (CString::new (fomat! ((myipaddr))));
    mypeer = lp::LP_addpeer (mypeer, pubsock, myipaddr_c.as_ptr() as *mut c_char, myport, pullport, pubport, 1, lp::G.LP_sessionid, netid);
    lp::LP_mypeer = mypeer;
    if mypeer == null_mut() {return ERR! ("Error adding {} into the p2p ring", myipaddr)}

    type IP<'a> = Cow<'a, str>;

    /// True if the node is a liquid provider (e.g. Bob, Maker).  
    /// NB: We want the peers to be equal, freely functioning as either a Bob or an Alice, and I wonder how the p2p LP flags are affected by that.
    type IsLp = bool;

    let seeds: Vec<(IP, IsLp)> = if let Some (seednodes) = seednodes {
        for seednode in seednodes.iter() {
            // A custom `seednode` is often used in automatic or manual tests
            // in order to directly give the Taker the address of the Maker.
            // We don't want to unnecessarily spam the friendlist of a public seed node,
            // but for a custom `seednode` we invoke the `investigate_peer`,
            // facilitating direct `peers` communication between the two.
            try_s!(peers::investigate_peer (ctx, &seednode, pubport + 1));
        }
        seednodes.into_iter().map(|ip| (Cow::Owned (ip), true)).collect()
    } else if netid > 0 && netid < 9 {
        vec! [(format! ("5.9.253.{}", 195 + netid) .into(), true)]
    } else if netid == 0 { // Default production netid is 0.
        P2P_SEED_NODES.iter().map (|ip| (Cow::Borrowed (&ip[..]), false)) .collect()
    } else if netid == 9999 { // MM2 testing netid is 9999.
        P2P_SEED_NODES_9999.iter().map (|ip| (Cow::Borrowed (&ip[..]), false)) .collect()
    } else if netid == 9000 { // A public seed node helps NAT traversal on netid 9000.
        vec![(Cow::Borrowed ("195.201.42.102"), false)]
    } else { // If we're using a non-default netid then we should skip adding the hardcoded seed nodes.
        Vec::new()
    };

    let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);
    if !i_am_seed {
        if seeds.len() == 0 {
            return ERR!("At least 1 IP must be provided");
        }
        let seed_ips = seeds.iter().map(|(ip, _)| fomat!((ip) ":" (pubport))).collect();
        try_s!(thread::Builder::new().name ("client_p2p_loop".into()) .spawn ({
            let ctx = ctx.clone();
            move || client_p2p_loop(ctx, seed_ips)
        }));
    }

    for (seed_ip, is_lp) in seeds {
        let ip = try_s! (CString::new (&seed_ip[..]));
        lp::LP_addpeer (mypeer, pubsock, ip.as_ptr() as *mut c_char, myport, pullport, pubport, if is_lp {1} else {0}, lp::G.LP_sessionid, netid);
    }
    Ok(())
}

/*
void LP_pubkeysloop(void *ctx)
{
    static uint32_t lasttime;
    strcpy(LP_pubkeysloop_stats.name,"LP_pubkeysloop");
    LP_pubkeysloop_stats.threshold = 15000.;
    sleep(10);
    while ( LP_STOP_RECEIVED == 0 )
    {
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        {
            LP_millistats_update(&LP_pubkeysloop_stats);
            if ( time(NULL) > lasttime+100 )
            {
                //printf("LP_pubkeysloop %u\n",(uint32_t)time(NULL));
                LP_notify_pubkeys(ctx,LP_mypubsock);
                lasttime = (uint32_t)time(NULL);
            }
        }
        sleep(3);
    }
}

struct LP_pendswap
{
    struct LP_pendswap *next,*prev;
    uint32_t expiration,requestid,quoteid,finished;
};

struct LP_pendswap *LP_pendingswaps;

void LP_pendswap_add(uint32_t expiration,uint32_t requestid,uint32_t quoteid)
{
    struct LP_pendswap *sp;
    printf("LP_pendswap_add expiration.%u %u-%u\n",expiration,requestid,quoteid);
    portable_mutex_lock(&LP_pendswap_mutex);
    sp = calloc(1,sizeof(*sp));
    sp->expiration = expiration;
    sp->requestid = requestid;
    sp->quoteid = quoteid;
    DL_APPEND(LP_pendingswaps,sp);
    portable_mutex_unlock(&LP_pendswap_mutex);
}

void LP_swapsloop(void *ctx)
{
    char *retstr; cJSON *retjson; uint32_t requestid,quoteid; int32_t i,nonz; struct LP_pendswap *sp,*tmp;
    strcpy(LP_swapsloop_stats.name,"LP_swapsloop");
    LP_swapsloop_stats.threshold = 605000.;
    if ( (retstr= basilisk_swapentry(0,0,0,1)) != 0 )
    {
        if ( (retjson= cJSON_Parse(retstr)) != 0 )
        {
            if ( (requestid= juint(retjson,"requestid")) != 0 && (quoteid= juint(retjson,"quoteid")) != 0 && jobj(retjson,"error") == 0 )
                LP_pendswap_add(0,requestid,quoteid);
        }
        free(retstr);
    }
    while ( LP_STOP_RECEIVED == 0 )
    {
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        LP_millistats_update(&LP_swapsloop_stats);
        nonz = 0;
        DL_FOREACH_SAFE(LP_pendingswaps,sp,tmp)
        {
            if ( sp->finished == 0 )
            {
                nonz++;
                if ( (sp->finished= LP_swapwait(0,sp->requestid,sp->quoteid,-1,0)) != 0 )
                {
                }
                sleep(3);
            }
        }
        if ( nonz == 0 )
        {
            for (i=0; i<10; i++)
            {
                //fprintf(stderr,"check on alice expiration\n");
                LP_alice_eligible((uint32_t)time(NULL));
                sleep(6);
            }
        } else sleep(10);
        LP_gtc_iteration(ctx,LP_myipaddr,LP_mypubsock);
    }
}

void gc_loop(void *ctx)
{
    uint32_t now; struct LP_address_utxo *up,*utmp; struct rpcrequest_info *req,*rtmp; int32_t flag = 0;
    strcpy(LP_gcloop_stats.name,"gc_loop");
    LP_gcloop_stats.threshold = 11000.;
    while ( LP_STOP_RECEIVED == 0 )
    {
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        flag = 0;
        LP_millistats_update(&LP_gcloop_stats);
        portable_mutex_lock(&LP_gcmutex);
        DL_FOREACH_SAFE(LP_garbage_collector,req,rtmp)
        {
            DL_DELETE(LP_garbage_collector,req);
            //printf("garbage collect ipbits.%x\n",req->ipbits);
            free(req);
            flag++;
        }
        now = (uint32_t)time(NULL);
        DL_FOREACH_SAFE(LP_garbage_collector2,up,utmp)
        {
            if ( now > (uint32_t)up->spendheight+120 )
            {
                DL_DELETE(LP_garbage_collector2,up);
                //char str[65]; printf("garbage collect %s/v%d lag.%d\n",bits256_str(str,up->U.txid),up->U.vout,now-up->spendheight);
                free(up);
            }
            flag++;
        }
        portable_mutex_unlock(&LP_gcmutex);
        if ( 0 && flag != 0 )
            printf("gc_loop.%d\n",flag);
        sleep(10);
    }
}

void queue_loop(void *ctx)
{
    struct LP_queue *ptr,*tmp; cJSON *json; uint8_t linebuf[32768]; int32_t k,sentbytes,nonz,flag,duplicate,n=0;
    strcpy(queue_loop_stats.name,"queue_loop");
    queue_loop_stats.threshold = 1000.;
    while ( LP_STOP_RECEIVED == 0 )
    {
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        LP_millistats_update(&queue_loop_stats);
        n = nonz = flag = 0;
        DL_FOREACH_SAFE(LP_Q,ptr,tmp)
        {
            n++;
            flag = 0;
            if ( ptr->sock >= 0 )
            {
                //printf("sock.%d len.%d notready.%d\n",ptr->sock,ptr->msglen,ptr->notready);
                if ( ptr->notready == 0 || (LP_rand() % ptr->notready) == 0 )
                {
                    if ( LP_sockcheck(ptr->sock) > 0 )
                    {
                        //bits256 magic;
                        //magic = LP_calc_magic(ptr->msg,(int32_t)(ptr->msglen - sizeof(bits256)));
                        //memcpy(&ptr->msg[ptr->msglen - sizeof(bits256)],&magic,sizeof(magic));
                        if ( 0 )
                        {
                            static FILE *fp;
                            if ( fp == 0 )
                                fp = fopen("packet.log","wb");
                            if ( fp != 0 )
                            {
                                fprintf(fp,"%s\n",(char *)ptr->msg);
                                fflush(fp);
                            }
                        }
                        if ( (json= cJSON_Parse((char *)ptr->msg)) != 0 )
                        {
                            if ( ptr->msglen < sizeof(linebuf) )
                            {
                                if ( (k= MMJSON_encode(linebuf,(char *)ptr->msg)) > 0 )
                                {
                                    if ( (sentbytes= nn_send(ptr->sock,linebuf,k,0)) != k )
                                        printf("%d LP_send mmjson sent %d instead of %d\n",n,sentbytes,k);
                                    else
                                    {
                                        flag++;
                                        ptr->sock = -1;
                                    }
                                }
                                //printf("k.%d flag.%d SEND.(%s) sock.%d\n",k,flag,(char *)ptr->msg,ptr->sock);
                            }
                            free_json(json);
                        }
                        if ( flag == 0 )
                        {
                           // printf("non-encoded len.%d SEND.(%s) sock.%d\n",ptr->msglen,(char *)ptr->msg,ptr->sock);
                            if ( (sentbytes= nn_send(ptr->sock,ptr->msg,ptr->msglen,0)) != ptr->msglen )
                                printf("%d LP_send sent %d instead of %d\n",n,sentbytes,ptr->msglen);
                            else
                            {
                                flag++;
                                ptr->sock = -1;
                            }
                        }
                        if ( ptr->peerind > 0 )
                            ptr->starttime = (uint32_t)time(NULL);
                    }
                    else
                    {
                        if ( ptr->notready++ > 100 )
                        {
                            flag = 1;
                            //printf("queue_loop sock.%d len.%d notready.%d, skip\n",ptr->sock,ptr->msglen,ptr->notready);
                            ptr->sock = -1;
                        }
                    }
                }
            }
            else if ( 0 && time(NULL) > ptr->starttime+13 )
            {
                LP_crc32find(&duplicate,-1,ptr->crc32);
                if ( duplicate > 0 )
                {
                    LP_Qfound++;
                    if ( (LP_Qfound % 100) == 0 )
                        printf("found.%u Q.%d err.%d match.%d\n",ptr->crc32,LP_Qenqueued,LP_Qerrors,LP_Qfound);
                    flag++;
                }
                else if ( 0 ) // too much beyond duplicate filter when network is busy
                {
                    printf("couldnt find.%u peerind.%d Q.%d err.%d match.%d\n",ptr->crc32,ptr->peerind,LP_Qenqueued,LP_Qerrors,LP_Qfound);
                    ptr->peerind++;
                    if ( (ptr->sock= LP_peerindsock(&ptr->peerind)) < 0 )
                    {
                        printf("%d no more peers to try at peerind.%d %p Q_LP.%p\n",n,ptr->peerind,ptr,LP_Q);
                        flag++;
                        LP_Qerrors++;
                    }
                }
            }
            if ( flag != 0 )
            {
                nonz++;
                portable_mutex_lock(&LP_networkmutex);
                DL_DELETE(LP_Q,ptr);
                portable_mutex_unlock(&LP_networkmutex);
                free(ptr);
                ptr = 0;
                break;
            }
        }
        if ( nonz == 0 )
        {
            if ( IAMLP == 0 )
                usleep(50000);
            else usleep(10000);
        }
    }
}

void LP_reserved_msgs(void *ignore)
{
    bits256 zero; int32_t flag,nonz; struct nn_pollfd pfd;
    memset(zero.bytes,0,sizeof(zero));
    strcpy(LP_reserved_msgs_stats.name,"LP_reserved_msgs");
    LP_reserved_msgs_stats.threshold = 1000.;
    while ( LP_STOP_RECEIVED == 0 )
    {
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        nonz = 0;
        LP_millistats_update(&LP_reserved_msgs_stats);
        if ( num_Reserved_msgs[1] > 0 )
        {
            nonz++;
            portable_mutex_lock(&LP_reservedmutex);
            if ( num_Reserved_msgs[1] > 0 )
            {
                num_Reserved_msgs[1]--;
                //printf("PRIORITY BROADCAST.(%s)\n",Reserved_msgs[1][num_Reserved_msgs[1]]);
                LP_broadcast_message(LP_mypubsock,"","",zero,Reserved_msgs[1][num_Reserved_msgs[1]]);
                Reserved_msgs[1][num_Reserved_msgs[1]] = 0;
            }
            portable_mutex_unlock(&LP_reservedmutex);
        }
        else if ( num_Reserved_msgs[0] > 0 )
        {
            nonz++;
            flag = 0;
            if ( flag == 0 && LP_mypubsock >= 0 )
            {
                memset(&pfd,0,sizeof(pfd));
                pfd.fd = LP_mypubsock;
                pfd.events = NN_POLLOUT;
                if ( nn_poll(&pfd,1,1) == 1 )
                    flag = 1;
            } else flag = 1;
            if ( flag == 1 )
            {
                portable_mutex_lock(&LP_reservedmutex);
                num_Reserved_msgs[0]--;
                //printf("BROADCAST.(%s)\n",Reserved_msgs[0][num_Reserved_msgs[0]]);
                LP_broadcast_message(LP_mypubsock,"","",zero,Reserved_msgs[0][num_Reserved_msgs[0]]);
                Reserved_msgs[0][num_Reserved_msgs[0]] = 0;
                portable_mutex_unlock(&LP_reservedmutex);
            }
        }
        if ( ignore == 0 )
            break;
        if ( nonz == 0 )
            usleep(5000);
    }
}

int32_t LP_reserved_msg(int32_t priority,char *base,char *rel,bits256 pubkey,char *msg)
{
    struct LP_pubkey_info *pubp; uint32_t timestamp; char *method; cJSON *argjson; int32_t skip,sentbytes,n = 0;
    skip = 0;
    if ( (argjson= cJSON_Parse(msg)) != 0 )
    {
        if ( (method= jstr(argjson,"method")) != 0 )
        {
            if ( strcmp(method,"gettradestatus") == 0 || strcmp(method,"wantnotify") == 0 || strcmp(method,"getdPoW") == 0 )
                skip = 1;
        }
        if ( (timestamp= juint(argjson,"timestamp")) != 0 && time(NULL) > timestamp+60 )
            skip = 1;
        free_json(argjson);
    }
    if ( skip != 0 )
        return(-1);
    //if ( strcmp(G.USERPASS,"1d8b27b21efabcd96571cd56f91a40fb9aa4cc623d273c63bf9223dc6f8cd81f") == 0 )
    //    return(-1);
    if ( priority > 0 && bits256_nonz(pubkey) != 0 )
    {
        if ( (pubp= LP_pubkeyfind(pubkey)) != 0 )
        {
            if ( pubp->pairsock >= 0 )
            {
                if ( (sentbytes= nn_send(pubp->pairsock,msg,(int32_t)strlen(msg)+1,0)) < 0 )
                {
                    //pubp->pairsock = -1;
                    //LP_peer_pairsock(pubkey);
                    //printf("mark cmdchannel %d closed sentbytes.%d\n",pubp->pairsock,sentbytes);
                }
                else
                {
                    printf("sent %d bytes to cmdchannel.%d\n",sentbytes,pubp->pairsock);
                    return(sentbytes);
                }
            }
        }
    }
    portable_mutex_lock(&LP_reservedmutex);
    if ( num_Reserved_msgs[priority] < sizeof(Reserved_msgs[priority])/sizeof(*Reserved_msgs[priority]) )
    {
        Reserved_msgs[priority][num_Reserved_msgs[priority]++] = msg;
        n = num_Reserved_msgs[priority];
    } //else LP_broadcast_message(LP_mypubsock,base,rel,pubkey,msg);
    if ( num_Reserved_msgs[priority] > max_Reserved_msgs[priority] )
    {
        max_Reserved_msgs[priority] = num_Reserved_msgs[priority];
        //if ( (max_Reserved_msgs[priority] % 100) == 0 )
            printf("New priority.%d max_Reserved_msgs.%d\n",priority,max_Reserved_msgs[priority]);
    }
    portable_mutex_unlock(&LP_reservedmutex);
    return(n);
}

*/
/// True during the threads initialization in `lp_init`.  
/// Mirrors the C `bitcoind_RPC_inittime`.
const BITCOIND_RPC_INITIALIZING: AtomicBool = AtomicBool::new (false);

// See if the CRC32 we have in Rust matches the C version.
#[test]
fn test_crc32() {
    use crc::crc32;
    use libc::c_void;
    assert_eq! (crc32::checksum_ieee (b"123456789"), 0xcbf43926);
    assert_eq! (unsafe {lp::calc_crc32 (0, b"123456789".as_ptr() as *mut c_void, 9)}, 0xcbf43926);
}

/// Invokes `OS_ensure_directory`,
/// then prints an error and returns `false` if the directory is not writeable.
fn ensure_dir_is_writable(dir_path: &Path) -> bool {
    let c_dir_path = unwrap! (dir_path.to_str());
    let c_dir_path = unwrap! (CString::new (c_dir_path));
    unsafe {os::OS_ensure_directory (c_dir_path.as_ptr() as *mut c_char)};

    /*
    char fname[512],str[65],str2[65]; bits256 r,check; FILE *fp;
    */
    let r: [u8; 32] = random();
    let mut check: Vec<u8> = Vec::with_capacity (r.len());
    let fname = dir_path.join ("checkval");
    let mut fp = match fs::File::create (&fname) {
        Ok (fp) => fp,
        Err (_) => {
            log! ({"FATAL ERROR cant create {:?}", fname});
            return false
        }
    };
    if fp.write_all (&r) .is_err() {
        log! ({"FATAL ERROR writing {:?}", fname});
        return false
    }
    drop (fp);
    let mut fp = match fs::File::open (&fname) {
        Ok (fp) => fp,
        Err (_) => {
            log! ({"FATAL ERROR cant open {:?}", fname});
            return false
        }
    };
    if fp.read_to_end (&mut check).is_err() || check.len() != r.len() {
        log! ({"FATAL ERROR reading {:?}", fname});
        return false
    }
    if check != r {
        log! ({"FATAL ERROR error comparing {:?} {:?} vs {:?}", fname, r, check});
        return false
    }
    true
}

fn ensure_file_is_writable(file_path: &Path) -> Result<(), String> {
    if let Err(_) = fs::File::open(file_path) {
        // try to create file if opening fails
        if let Err(e) = fs::OpenOptions::new().write(true).create_new(true).open(file_path) {
            return ERR!("{} when trying to create the file {}", e, file_path.display())
        }
    } else {
        // try to open file in write append mode
        if let Err(e) = fs::OpenOptions::new().write(true).append(true).open(file_path) {
            return ERR!("{} when trying to open the file {} in write mode", e, file_path.display())
        }
    }
    Ok(())
}

fn fix_directories(ctx: &MmCtx) -> Result<(), String> {
    unsafe {os::OS_ensure_directory (lp::GLOBAL_DBDIR.as_ptr() as *mut c_char)};
    let dbdir = ctx.dbdir();
    if !ensure_dir_is_writable(&dbdir.join ("SWAPS")) {return ERR!("SWAPS db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("SWAPS").join ("MY")) {return ERR!("SWAPS/MY db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("SWAPS").join ("STATS")) {return ERR!("SWAPS/STATS db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("SWAPS").join ("STATS").join ("MAKER")) {return ERR!("SWAPS/STATS/MAKER db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("SWAPS").join ("STATS").join ("TAKER")) {return ERR!("SWAPS/STATS/TAKER db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("TRANSACTIONS")) {return ERR!("TRANSACTIONS db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("GTC")) {return ERR!("GTC db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("PRICES")) {return ERR!("PRICES db dir is not writeable")}
    if !ensure_dir_is_writable(&dbdir.join ("UNSPENTS")) {return ERR!("UNSPENTS db dir is not writeable")}
    try_s!(ensure_file_is_writable(&dbdir.join ("GTC").join ("orders")));
    Ok(())
}

/// Resets the context (most of which resides currently in `lp::G` but eventually would move into `MmCtx`).
/// Restarts the peer connections.
/// Reloads the coin keys.
/// 
/// Besides the `passphrase` it also allows changing the `seednode` and `gui` at runtime.  
/// AG: While there might be value in changing `seednode` at runtime, I'm not sure if changing `gui` is actually necessary.
/// 
/// AG: If possible, I think we should avoid calling this function on a working MM, using it for initialization only,
///     in order to avoid the possibility of invalid state.
#[allow(unused_unsafe)]
pub unsafe fn lp_passphrase_init (ctx: &MmArc, passphrase: Option<&str>, gui: Option<&str>, seednodes: Option<Vec<String>>) -> Result<(), String> {
    let passphrase = match passphrase {
        None | Some ("") => return ERR! ("jeezy says we cant use the nullstring as passphrase and I agree"),
        Some (s) => s.to_string()
    };
    if lp::G.LP_pendingswaps != 0 {return ERR! ("There are pending swaps")}
    // Prepare and check some of the `lp_initpeers` parameters.
    let netid = lp::G.netid;
    let myipaddr: IpAddr = try_s! (try_s! (CStr::from_ptr (lp::LP_myipaddr.as_ptr()) .to_str()) .parse());

    let gui: Cow<str> = match gui {
        Some (g) => g.into(),
        None => match try_s! (CStr::from_ptr (lp::G.gui.as_ptr()) .to_str()) {
            "" => "cli".into(),  // Default.
            have => have.to_string().into()  // Reuse the existing `gui`.
        }
    };
    let userpass_counter = lp::G.USERPASS_COUNTER;
    try_s! (coins_iter (&mut |coin| {
        (*coin).importedprivkey = 0;
        Ok(())
    }));

    {
        let mut status = ctx.log.status_handle();
        while lp::G.waiting == 0 {
            status.status (&[&"lp_passphrase_init"], "Waiting for `G.waiting`...");
            sleep (Duration::from_millis (100))
        }
        status.append (" Done.");
    }

    lp::G = zeroed();
    lp::G.initializing = 1;
    lp::G.netid = netid;
    lp::vcalc_sha256 (null_mut(), lp::G.LP_passhash.bytes.as_mut_ptr(), passphrase.as_ptr() as *mut u8, passphrase.len() as i32);
    let passphrase_c = try_s! (CString::new (&passphrase[..]));
    let mut pubkey33: [u8; 100] = zeroed();
    lp::bitcoin_pubkey33 (ctx.btc_ctx() as *mut c_void, pubkey33.as_mut_ptr(), lp::G.LP_privkey);
    lp::calc_rmd160_sha256 (lp::G.LP_myrmd160.as_mut_ptr(), pubkey33.as_mut_ptr(), 33);
    try_s! (safecopy! (lp::G.LP_myrmd160str, "{}", hex::encode (lp::G.LP_myrmd160)));
    lp::G.LP_sessionid = (now_ms() / 1000) as u32;
    try_s! (safecopy! (lp::G.gui, "{}", gui));
    let c_passphrase = try_s! (CString::new (&passphrase[..]));
    unsafe {
        let mut pubkey: bits256 = zeroed();
        let pk = lp::LP_privkeycalc (
            &mut pubkey,                           // bits256 *pubkeyp
            c_passphrase.as_ptr() as *mut c_char,  // char *passphrase
        );
        if !pk.nonz() {return ERR! ("!LP_privkeycalc")}
        if !lp::G.LP_privkey.nonz() {return ERR! ("Error initializing the global private key (G.LP_privkey)")}
        let key_pair = try_s!(compressed_key_pair_from_bytes(&lp::G.LP_privkey.bytes, 0, ChecksumType::DSHA256));
        lp::G.LP_pubsecp.clone_from_slice(&**key_pair.public());
    }
    try_s! (lp_initpeers (ctx, lp::LP_mypubsock, lp::LP_mypeer, &myipaddr, lp::RPC_port, netid, seednodes));

    lp::LP_tradebot_pauseall();
    lp::LP_portfolio_reset();
    lp::LP_priceinfos_clear();

    // Copy some of the fields from the old G.
    lp::G.USERPASS_COUNTER = userpass_counter;

    lp::G.initializing = 0;
    Ok(())
}

/// Tries to serve on the given IP to check if it's available.  
/// We need this check because our external IP, particularly under NAT,
/// might be outside of the set of IPs we can open and run a server on.
/// 
/// Returns an error if the address did not work
/// (like when the `ip` does not belong to a connected interface).
/// 
/// The primary concern of this function is to test the IP,
/// but this opportunity is also used to start the HTTP fallback server,
/// in order to improve the reliability of the said server (in the Lean "stop the line" manner).
/// 
/// If the IP has passed the communication check then a shutdown Sender is returned.
/// Dropping or using that Sender will stop the HTTP fallback server.
/// 
/// Also the port of the HTTP fallback server is returned.
fn test_ip (ctx: &MmArc, ip: IpAddr) -> Result<(Sender<()>, u16), String> {
    // NB: The `bind` just always works on certain operating systems.
    // To actually check the address we should try communicating on it.
    // Reusing the HTTP fallback server for that.

    let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or (false);
    let netid = ctx.netid();

    let (server, port) = if i_am_seed {
        // NB: We might need special permissions to bind on 80.
        // HTTP fallback should work on that port though
        // in order to function with those providers which only allow the usual traffic.
        let port = 80;
        (try_s! (new_http_fallback (ctx.weak(), SocketAddr::new (ip, port))), port)
    } else {
        // Try a few pseudo-random ports.
        // `netid` is used as the seed in order for the port selection to be determenistic,
        // similar to how the port selection and probing worked before (since MM1)
        // and in order to reduce the likehood of *unexpected* port conflicts.
        let mut attempts_left = 9;
        let mut rng = SmallRng::seed_from_u64 (netid as u64);
        loop {
            if attempts_left < 1 {return ERR! ("Out of attempts")}
            attempts_left -= 1;
            // TODO: Avoid `mypubport`.
            let port = rng.gen_range (1111, 65535);
            log! ("test_ip] Trying to listen on " (ip) ':' (port));
            match new_http_fallback (ctx.weak(), SocketAddr::new (ip, port)) {
                Ok (s) => break (s, port),
                Err (err) => {
                    if attempts_left == 0 {return ERR! ("{}", err)}
                    continue
                }
            }
        }
    };

    // Finish the server `Future` when `shutdown_rx` fires.
    let (shutdown_tx, shutdown_rx) = futures::sync::oneshot::channel::<()>();
    let server = server.select2 (shutdown_rx) .then (|_| Ok(()));
    CORE.spawn (move |_| server);

    let url = fomat! ("http://" if ip.is_unspecified() {"127.0.0.1"} else {(ip)} ":" (port) "/test_ip");
    log! ("test_ip] Checking " (url));
    let rc = slurp_url (&url) .wait();
    let (status, _h, body) = try_s! (rc);
    if status != StatusCode::OK {return ERR! ("Status not OK")}
    if body != b"k" {return ERR! ("body not k")}
    Ok ((shutdown_tx, port))
}

pub fn lp_init (mypullport: u16, mypubport: u16, conf: Json, c_conf: CJSON, ctx_cb: &Fn (u32))
-> Result<(), String> {
    unsafe {lp::G.initializing = 1}  // Tells some of the spawned threads to wait till the `lp_passphrase_init` is done.
    BITCOIND_RPC_INITIALIZING.store (true, Ordering::Relaxed);
    if lp::LP_MAXPRICEINFOS > 255 {
        return ERR! ("LP_MAXPRICEINFOS {} wont fit in a u8, need to increase the width of the baseind and relind for struct LP_pubkey_quote", lp::LP_MAXPRICEINFOS)
    }
    unsafe {lp::LP_showwif = if conf["wif"] == 1 {1} else {0}};
    log! ({"showwif.{} version: {}", unsafe {lp::LP_showwif}, MM_VERSION});
    if conf["gui"] == 1 {
        // Replace "cli\0" with "gui\0".
        let lp_gui: &mut [c_char] = unsafe {&mut lp::LP_gui[..]};
        let lp_gui: &mut [u8] = unsafe {transmute (lp_gui)};
        let mut cur = Cursor::new (lp_gui);
        unwrap! (write! (&mut cur, "gui\0"))
    }

    unsafe {
        lp::LP_fixed_pairport = if conf["canbind"].is_null() {
            0
        } else {
            let canbind = unwrap! (conf["canbind"].as_i64());
            if canbind <= 1000 {return ERR! ("canbind <= 1000")}
            if canbind > 65535 {return ERR! ("canbind > u16")}
            canbind as u16
        }
    }

    if !conf["userhome"].is_null() {
        let userhome = unwrap! (conf["userhome"].as_str()) .trim();
        if !userhome.is_empty() {
            let global: &mut [c_char] = unsafe {&mut lp::USERHOME[..]};
            let global: &mut [u8] = unsafe {transmute (global)};
            let mut cur = Cursor::new (global);
            try_s! (write! (&mut cur, "{}", userhome));
            if cfg! (target_os = "macos") {
                try_s! (write! (&mut cur, "/Library/Application Support"))
            }
            try_s! (write! (&mut cur, "\0"))
        }
    }
    if !conf["dbdir"].is_null() {
        let dbdir = unwrap! (conf["dbdir"].as_str()) .trim();
        if !dbdir.is_empty() {
            let global: &mut [c_char] = unsafe {&mut lp::GLOBAL_DBDIR[..]};
            let global: &mut [u8] = unsafe {transmute (global)};
            let mut cur = Cursor::new (global);
            try_s! (write! (&mut cur, "{}", dbdir));
            try_s! (write! (&mut cur, "\0"))
        }
    }
    unsafe {lp::LP_mutex_init()};

    let ctx = MmCtx::new (conf);
    ctx_cb (try_s! (ctx.ffi_handle()));

    try_s! (fix_directories (&ctx));

    fn simple_ip_extractor (ip: &str) -> Result<IpAddr, String> {
        let ip = ip.trim();
        Ok (match ip.parse() {Ok (ip) => ip, Err (err) => return ERR! ("Error parsing IP address '{}': {}", ip, err)})
    }

    let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);
    let netid = ctx.netid();

    let mut http_fallback_shutdown = None;
    let mut http_fallback_port = None;

    let myipaddr: IpAddr = if Path::new ("myipaddr") .exists() {
        match fs::File::open ("myipaddr") {
            Ok (mut f) => {
                let mut buf = String::new();
                if let Err (err) = f.read_to_string (&mut buf) {
                    return ERR! ("Can't read from 'myipaddr': {}", err)
                }
                try_s! (simple_ip_extractor (&buf))
            },
            Err (err) => return ERR! ("Can't read from 'myipaddr': {}", err)
        }
    } else if !ctx.conf["myipaddr"].is_null() {
        let s = try_s! (ctx.conf["myipaddr"].as_str().ok_or ("'myipaddr' is not a string"));
        let ip = try_s! (simple_ip_extractor (s));
        unsafe {lp::LP_myipaddr_from_command_line = 1};
        ip
    } else {
        // Detect the real IP address.
        // 
        // We're detecting the outer IP address, visible to the internet.
        // Later we'll try to *bind* on this IP address,
        // and this will break under NAT or forwarding because the internal IP address will be different.
        // Which might be a good thing, allowing us to detect the likehoodness of NAT early.

        let ip_providers: [(&'static str, fn (&str) -> Result<IpAddr, String>); 2] = [
            ("http://checkip.amazonaws.com/", simple_ip_extractor),
            ("http://api.ipify.org", simple_ip_extractor)
        ];

        let mut ip_providers_it = ip_providers.iter();
        loop {
            let (url, extactor) = match ip_providers_it.next() {Some (t) => t, None => return ERR! ("Can't fetch the real IP")};
            log! ({"lp_init] Trying to fetch the real IP from '{}' ...", url});
            let (status, _headers, ip) = match slurp_url (url) .wait() {
                Ok (t) => t,
                Err (err) => {
                    log! ({"lp_init] Failed to fetch IP from '{}': {}", url, err});
                    continue
                }
            };
            if !status.is_success() {
                log! ({"lp_init] Failed to fetch IP from '{}': status {:?}", url, status});
                continue
            }
            let ip = match from_utf8 (&ip) {
                Ok (ip) => ip,
                Err (err) => {
                    log! ({"lp_init] Failed to fetch IP from '{}', not UTF-8: {}", url, err});
                    continue
                }
            };
            let ip = match extactor (ip) {
                Ok (ip) => ip,
                Err (err) => {
                    log! ({"lp_init] Failed to parse IP '{}' fetched from '{}': {}", ip, url, err});
                    continue
                }
            };

            // Try to bind on this IP.
            // If we're not behind a NAT then the bind will likely suceed.
            // If the bind fails then emit a user-visible warning and fall back to 0.0.0.0.
            let tags: &[&TagParam] = &[&"myipaddr"];
            match test_ip (&ctx, ip) {
                Ok ((hf_shutdown, hf_port)) => {
                    ctx.log.log ("🙂", tags, &fomat! (
                        "We've detected an external IP " (ip) " and we can bind on it (port " (mypubport) ")"
                        ", so probably a dedicated IP."));
                    if i_am_seed {
                        http_fallback_shutdown = Some (hf_shutdown);
                        http_fallback_port = Some (hf_port);
                    }
                    break ip
                },
                Err (err) => log! ("IP " (ip) " doesn't check: " (err))
            }
            let all_interfaces = Ipv4Addr::new (0, 0, 0, 0) .into();
            if test_ip (&ctx, all_interfaces) .is_ok() {
                ctx.log.log ("😅", tags, &fomat! (
                    "We couldn't bind on the external IP " (ip) ", so NAT is likely to be present. We'll be okay though."));
                break all_interfaces
            }
            let locahost = Ipv4Addr::new (127, 0, 0, 1) .into();
            if test_ip (&ctx, locahost) .is_ok() {
                ctx.log.log ("🤫", tags, &fomat! (
                    "We couldn't bind on " (ip) " or 0.0.0.0!"
                    " Looks like we can bind on 127.0.0.1 as a workaround, but that's not how we're supposed to work."));
                break locahost
            }
            ctx.log.log ("🤒", tags, &fomat! (
                "Couldn't bind on " (ip) ", 0.0.0.0 or 127.0.0.1."));
            break all_interfaces  // Seems like a better default than 127.0.0.1, might still work for other ports.
        }
    };

    unsafe {lp::IAMLP = 1}  // Anyone can be a Maker.
    unsafe {lp::LP_canbind = 1}
    unsafe {lp::G.netid = netid}
    unsafe {lp::LP_mypubsock = -1}

    let seednode_thread = if i_am_seed {
        log! ("i_am_seed at " (myipaddr) ":" (mypubport));
        let listener: TcpListener = try_s!(TcpListener::bind(&fomat!((myipaddr) ":" (mypubport))));
        try_s!(listener.set_nonblocking(true));
        Some(try_s!(thread::Builder::new().name ("seednode_loop".into()) .spawn ({
            let ctx = ctx.clone();
            move || seednode_loop(ctx, listener)
        })))
    } else {
        None
    };
    try_s! (coins::lp_initcoins (&ctx));
    unsafe {lp::RPC_port = try_s! (ctx.rpc_ip_port()) .port()}
    unsafe {lp::G.waiting = 1}
    try_s! (unsafe {safecopy! (lp::LP_myipaddr, "{}", myipaddr)});

    let seednodes: Option<Vec<String>> = try_s!(json::from_value(ctx.conf["seednodes"].clone()));
    unsafe {try_s! (lp_passphrase_init (&ctx,
        ctx.conf["passphrase"].as_str(), ctx.conf["gui"].as_str(), seednodes))};
/*
#ifndef FROM_JS
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_psockloop,(void *)myipaddr) != 0 )
    {
        printf("error launching LP_psockloop for (%s)\n",myipaddr);
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_reserved_msgs,(void *)myipaddr) != 0 )
    {
        printf("error launching LP_reserved_msgs for (%s)\n",myipaddr);
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)stats_rpcloop,(void *)&myport) != 0 )
    {
        printf("error launching stats rpcloop for port.%u\n",myport);
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)command_rpcloop,ctx) != 0 )
    {
        printf("error launching command_rpcloop for ctx.%p\n",ctx);
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)queue_loop,ctx) != 0 )
    {
        printf("error launching queue_loop for ctx.%p\n",ctx);
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)gc_loop,ctx) != 0 )
    {
        printf("error launching gc_loop for port.%p\n",ctx);
        exit(-1);
    }
    */
    ctx.initialized.store (true, Ordering::Relaxed);
    let prices = try_s! (thread::Builder::new().name ("prices".into()) .spawn ({
        let ctx = ctx.clone();
        move || prices_loop (ctx)
    }));
    /*
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_coinsloop,(void *)"") != 0 )
    {
        printf("error launching LP_coinsloop for (%s)\n","");
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_coinsloop,(void *)"BTC") != 0 )
    {
        printf("error launching LP_coinsloop for (%s)\n","BTC");
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_coinsloop,(void *)"KMD") != 0 )
    {
        printf("error launching LP_coinsloop for (%s)\n","KMD");
        exit(-1);
    }
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_pubkeysloop,ctx) != 0 )
    {
        printf("error launching LP_pubkeysloop for ctx.%p\n",ctx);
        exit(-1);
    }
*/
    let trades = try_s! (thread::Builder::new().name ("trades".into()) .spawn ({
        let ctx = ctx.clone();
        move || unsafe { lp_trades_loop (ctx) }
    }));

    let command_queue = try_s! (thread::Builder::new().name ("command_queue".into()) .spawn ({
        let ctx = ctx.clone();
        move || unsafe { lp_command_q_loop (ctx) }
    }));
/*
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_swapsloop,ctx) != 0 )
    {
        printf("error launching LP_swapsloop for ctx.%p\n",ctx);
        exit(-1);
    }
    int32_t nonz,didremote=0;
    LP_statslog_parse();
    bitcoind_RPC_inittime = 0;
    //LP_mpnet_init(); seems better to have the GUI send in persistent orders, exit mm is a cancel all
    while ( LP_STOP_RECEIVED == 0 )
    {
        nonz = 0;
        G.waiting = 1;
        while ( G.initializing != 0 ) //&& strcmp(G.USERPASS,"1d8b27b21efabcd96571cd56f91a40fb9aa4cc623d273c63bf9223dc6f8cd81f") == 0 )
        {
            //fprintf(stderr,".");
            sleep(3);
        }
        if ( G.initializing != 0 )
        {
            sleep(1);
            continue;
        }
        if ( LP_mainloop_iter(ctx,myipaddr,mypeer,LP_mypubsock) != 0 )
            nonz++;
        if ( IAMLP != 0 && didremote == 0 && LP_cmdcount > 0 )
        {
            didremote = 1;
            uint16_t myport2 = RPC_port-1;
            printf("start remote port\n");
            if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)stats_rpcloop,(void *)&myport2) != 0 )
            {
                printf("error launching stats rpcloop for port.%u\n",myport);
                exit(-1);
            }
        }
        if ( nonz == 0 )
            usleep(1000);
        else if ( IAMLP == 0 )
            usleep(1000);
    }
#endif
    printf("marketmaker exiting in 5 seconds\n");
    sleep(5);
    exit(0);
*/
    let passphrase = try_s! (CString::new (unwrap! (ctx.conf["passphrase"].as_str())));
    let ctx_id = try_s! (ctx.ffi_handle());

    // `LPinit` currently fails to stop in a timely manner, so we're dropping the `lp_init` context early
    // in order to be able to use and test the `Drop` implementations withing the context.
    // In the future, when `LPinit` stops in a timely manner, we might relinquish the early `drop`.
    drop (ctx);

    // When building etomicrs tests (cargo test --package etomicrs) we have no access
    // to C functions defined here in the mm2 binary crate. Such functions should be shared dynamically
    // in order not to interfere with the linking of the etomicrs test binary.
    unsafe {lp::SPAWN_RPC = Some (rpc::spawn_rpc)};
    unsafe {lp::LP_QUEUE_COMMAND = Some (lp_queue_command_for_c)};

    let myipaddr = unsafe {lp::LP_myipaddr.as_ptr() as *mut c_char};
    unsafe {lp::LPinit (myipaddr, mypullport, mypubport, passphrase.as_ptr() as *mut c_char, c_conf.0, ctx_id)};
    unwrap! (prices.join());
    unwrap! (trades.join());
    unwrap! (command_queue.join());
    if let Some(seednode) = seednode_thread {
        unwrap! (seednode.join());
    }
    Ok(())
}
/*

#ifdef FROM_JS
extern void *Nanomsg_threadarg;
void *nn_thread_main_routine(void *arg);

void emscripten_usleep(int32_t x)
{
}

char *bitcoind_RPC(char **retstrp,char *debugstr,char *url,char *userpass,char *command,char *params,int32_t timeout)
{
    static uint32_t counter; char fname[512],*retstr; long fsize;
    if ( strncmp("http://",url,strlen("http://")) != 0 )
        return(clonestr("{\"error\":\"only http allowed\"}"));
    sprintf(fname,"bitcoind_RPC/request.%d",counter % 10);
    counter++;
    //printf("issue.(%s)\n",url);
    emscripten_wget(url,fname);
    retstr = OS_filestr(&fsize,fname);
    //printf("bitcoind_RPC(%s) -> fname.(%s) %s\n",url,fname,retstr);
    return(retstr);
}

char *barterDEX(char *argstr)
{
    static void *ctx;
    cJSON *argjson; char *retstr;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    printf("barterDEX.(%s)\n",argstr);
    if ( (argjson= cJSON_Parse(argstr)) != 0 )
    {
        LP_queuecommand(&retstr,argstr,LP_mypubsock);
        //retstr = LP_command_process(ctx,LP_myipaddr,LP_mypubsock,argjson,(uint8_t *)argstr,(int32_t)strlen(argstr));
        while ( retstr == 0 )
            usleep(50000);
        free_json(argjson);
    } else retstr = clonestr("{\"error\":\"couldnt parse request\"}");
    return(retstr);
}

void LP_fromjs_iter()
{
    static void *ctx; char *retstr;
    if ( G.initializing != 0 )
    {
        printf("LP_fromjs_iter during G.initializing, skip\n");
        return;
    }
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    //if ( Nanomsg_threadarg != 0 )
    //    nn_thread_main_routine(Nanomsg_threadarg);
    //LP_pubkeys_query();
    //LP_utxosQ_process();
    //LP_nanomsg_recvs(ctx);
    LP_mainloop_iter(ctx,LP_myipaddr,0,LP_mypubsock);
    //queue_loop(0);
    if ( 0 ) // 10 seconds
    {
        LP_coinsloop(0);
        if ( 0 ) // 100 seconds
        {
            LP_notify_pubkeys(ctx,LP_mypubsock);
            LP_privkey_updates(ctx,LP_mypubsock,0);
            if ( (retstr= basilisk_swapentry(0,0,0,0)) != 0 )
                free(retstr);
        }
    }
}

#endif

#undef calloc
#undef free
#undef realloc
#undef clonestr

struct LP_memory_list
{
    struct LP_memory_list *next,*prev;
    uint32_t timestamp,len;
    void *ptr;
} *LP_memory_list;
int32_t zeroval() { return(0); }

void *LP_alloc(uint64_t len)
{
//return(calloc(1,len));
    LP_cjson_allocated += len;
    LP_cjson_total += len;
    LP_cjson_count++;
    struct LP_memory_list *mp;
    mp = calloc(1,sizeof(*mp) + len);
    mp->ptr = calloc(1,len);
    //printf(">>>>>>>>>>> LP_alloc mp.%p ptr.%p len.%llu %llu\n",mp,mp->ptr,(long long)len,(long long)LP_cjson_allocated);
    mp->timestamp = (uint32_t)time(NULL);
    mp->len = (uint32_t)len;
    portable_mutex_lock(&LP_cJSONmutex);
    DL_APPEND(LP_memory_list,mp);
    portable_mutex_unlock(&LP_cJSONmutex);
    return(mp->ptr);
}

void LP_free(void *ptr)
{
    static uint32_t lasttime,unknown; static int64_t lasttotal;
//free(ptr); return;
    uint32_t now; char str[65]; int32_t n,lagging; uint64_t total = 0; struct LP_memory_list *mp,*tmp;
    if ( (now= (uint32_t)time(NULL)) > lasttime+1 )
    {
        n = lagging = 0;
        DL_FOREACH_SAFE(LP_memory_list,mp,tmp)
        {
            total += mp->len;
            n++;
            if ( 0 && now > mp->timestamp+120 )
            {
                lagging++;
                if ( now > mp->timestamp+240 )
                {
                    portable_mutex_lock(&LP_cJSONmutex);
                    DL_DELETE(LP_memory_list,mp);
                    portable_mutex_unlock(&LP_cJSONmutex);
                    free(mp->ptr);
                    free(mp);
                }
            }
        }
        printf("[%lld] total %d allocated total %llu/%llu [%llu %llu] %.1f ave %s unknown.%u lagging.%d\n",(long long)(total-lasttotal),n,(long long)total,(long long)LP_cjson_allocated,(long long)LP_cjson_total,(long long)LP_cjson_count,(double)LP_cjson_total/LP_cjson_count,mbstr(str,total),unknown,lagging);
        lasttime = (uint32_t)time(NULL);
        lasttotal = total;
    }
    DL_FOREACH_SAFE(LP_memory_list,mp,tmp)
    {
        if ( mp->ptr == ptr )
            break;
        mp = 0;
    }
    if ( mp != 0 )
    {
        LP_cjson_allocated -= mp->len;
        portable_mutex_lock(&LP_cJSONmutex);
        DL_DELETE(LP_memory_list,mp);
        portable_mutex_unlock(&LP_cJSONmutex);
        //printf(">>>>>>>>>>> LP_free ptr.%p mp.%p len.%u %llu\n",ptr,mp,mp->len,(long long)LP_cjson_allocated);
        free(mp->ptr);
        free(mp);
    } else unknown++; // free from source file with #define redirect for alloc that wasnt
}

/*char *LP_clonestr(char *str)
{
    char *retstr = LP_alloc(strlen(str)+1);
    strcpy(retstr,str);
    return(retstr);
}

void *LP_realloc(void *ptr,uint64_t len)
{
    return(realloc(ptr,len));
}*/
*/