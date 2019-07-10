
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
//  LP_nativeDEX.c
//  marketmaker
//
// portfolio to set prices from historical
// portfolio value based on ask?
// else claim path
//
// WONTFIX:
// dPoW security -> 4: KMD notarized, 5: BTC notarized, after next notary elections
// bigendian architectures need to use little endian for sighash calcs
// improve critical section detection when parallel trades
// use electrum in case of addr change in swap
// locktime claiming on sporadic assetchains
// there is an issue about waiting for notarization for a swap that never starts (expiration ok)

#include <stdio.h>
#ifndef MM_VERSION
#define MM_VERSION "UNKNOWN"
#endif

#ifndef NATIVE_WINDOWS
# include <pthread.h>
#endif

#ifdef __APPLE__
# include "TargetConditionals.h"
#endif

typedef struct queue
{
	struct queueitem *list;
	pthread_mutex_t mutex;
    char name[64],initflag;
} queue_t;

void queue_enqueue(char *name,queue_t *queue,struct queueitem *origitem);

struct electrum_info
{
    queue_t sendQ,pendingQ;
    pthread_mutex_t mutex,txmutex;
    struct electrum_info *prev;
    int32_t bufsize,sock,*heightp,numerrors;
    struct iguana_info *coin;
    uint32_t stratumid,lasttime,keepalive,pending,*heighttimep;
    char ipaddr[64],symbol[66];
    uint16_t port;
    uint8_t buf[];
};

struct rpcrequest_info
{
    struct rpcrequest_info *next,*prev;
    pthread_t T;
    int32_t sock;
    uint32_t ipbits;
    uint16_t port,pad;
};

long LP_cjson_allocated,LP_cjson_total,LP_cjson_count;

struct LP_millistats
{
    double lastmilli,millisum,threshold;
    uint32_t count;
    char name[64];
} LP_psockloop_stats,LP_reserved_msgs_stats,utxosQ_loop_stats,command_rpcloop_stats,queue_loop_stats,prices_loop_stats,LP_coinsloop_stats,LP_coinsloopBTC_stats,LP_coinsloopKMD_stats,LP_pubkeysloop_stats,LP_swapsloop_stats,LP_gcloop_stats,LP_tradesloop_stats;
extern int32_t IAMLP;
char LP_methodstr[64];

int32_t bits256_nonz(bits256 a) {
    return (((a).ulongs[0] | (a).ulongs[1] | (a).ulongs[2] | (a).ulongs[3]) != 0);
}

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

portable_mutex_t LP_peermutex,LP_UTXOmutex,LP_utxomutex,LP_commandmutex,LP_cachemutex,LP_swaplistmutex,LP_forwardmutex,LP_pubkeymutex,LP_networkmutex,LP_psockmutex,LP_coinmutex,LP_messagemutex,LP_portfoliomutex,LP_electrummutex,LP_butxomutex,LP_reservedmutex,LP_nanorecvsmutex,LP_tradebotsmutex,LP_gcmutex,LP_inusemutex,LP_cJSONmutex,LP_logmutex,LP_statslogmutex,LP_tradesmutex,LP_commandQmutex,LP_blockinit_mutex,LP_pendswap_mutex,LP_listmutex,LP_gtcmutex;
int32_t LP_canbind;
char *Broadcaststr,*Reserved_msgs[2][1000];
int32_t num_Reserved_msgs[2],max_Reserved_msgs[2];
struct LP_peerinfo  *LP_peerinfos,*LP_mypeer;
struct LP_forwardinfo *LP_forwardinfos;
struct iguana_info *LP_coins;
struct LP_pubkey_info *LP_pubkeyinfos;
struct LP_trade *LP_trades,*LP_tradesQ;

//uint32_t LP_deadman_switch;
uint16_t LP_fixed_pairport;//,LP_publicport;
uint32_t LP_lastnonce,LP_swap_endcritical,LP_swap_critical,LP_RTcount,LP_swapscount;
int32_t LP_STOP_RECEIVED,LP_numactive_LP;//,LP_mybussock = -1;
int32_t LP_mypubsock = -1,IPC_ENDPOINT = -1;
int32_t LP_cmdcount,LP_mypullsock = -1;
int32_t LP_numfinished,LP_showwif,IAMLP = 0;
double LP_profitratio = 1.;

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

char *activecoins[] = { "BTC", "KMD" };
char GLOBAL_DBDIR[512] = { "DB" };
char LP_myipaddr[64],USERHOME[512] = { "/root" };
uint8_t LP_myipaddr_from_command_line = 0;
char LP_gui[65] = { "cli" };

// cf. `P2P_SEED_NODES`
char *default_LPnodes[] = { "5.9.253.195", "173.212.225.176", "136.243.45.140", "23.254.202.142", "45.32.19.196"
    //"24.54.206.138", "107.72.162.127", "72.50.16.86", "51.15.202.191", "173.228.198.88",
    //"51.15.203.171", "51.15.86.136", "51.15.94.249", "51.15.80.18", "51.15.91.40", "51.15.54.2", "51.15.86.31", "51.15.82.29", "51.15.89.155", "173.212.225.176", "136.243.45.140"
};

// stubs

char *LP_getdatadir()
{
    return(USERHOME);
}

char *blocktrail_listtransactions(char *symbol,char *coinaddr,int32_t num,int32_t skip)
{
    return(0);
}

int32_t LP_merkleproof(struct iguana_info *coin,char *coinaddr,struct electrum_info *ep,bits256 txid,int32_t height);

#include "LP_mmjson.c"
#include "LP_secp.c"
#include "LP_bitcoin.c"
#include "LP_coins.c"
#include "LP_rpc.c"
#include "LP_mpnet.c"
#include "LP_RTmetrics.c"
#include "LP_utxo.c"
#include "LP_prices.c"
#include "LP_scan.c"
#include "LP_transaction.c"
#include "LP_stats.c"
#include "LP_remember.c"
#include "LP_swap.c"
#include "LP_peers.c"
#include "LP_privkey.c"
#include "LP_forwarding.c"
#include "LP_signatures.c"
#include "LP_ordermatch.c"
#include "LP_tradebots.c"
#include "LP_portfolio.c"

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
                            LP_reserved_msg(0,zero,jprint(reqjson,0));
                        }
                        retstr = clonestr("{\"result\":\"success\"}");
                        free_json(reqjson);
                    } else retstr = clonestr("{\"error\":\"couldnt dereference sendmessage\"}");
                }
                else
                {
                    LP_QUEUE_COMMAND(0,jsonstr,pubsock,0,0);
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

void LP_pubkeysloop(void *ctx_h)
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
            if ( time(NULL) > lasttime+20 )
            {
                //printf("LP_pubkeysloop %u\n",(uint32_t)time(NULL));
                LP_notify_pubkeys(*(uint32_t *)ctx_h);
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

int32_t LP_reserved_msg(int32_t priority,bits256 pubkey,char *msg)
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

extern int32_t bitcoind_RPC_inittime;

void LP_mutex_init() {
    portable_mutex_init(&LP_peermutex);
    portable_mutex_init(&LP_utxomutex);
    portable_mutex_init(&LP_UTXOmutex);
    portable_mutex_init(&LP_commandmutex);
    portable_mutex_init(&LP_swaplistmutex);
    portable_mutex_init(&LP_cachemutex);
    portable_mutex_init(&LP_networkmutex);
    portable_mutex_init(&LP_gcmutex);
    portable_mutex_init(&LP_forwardmutex);
    portable_mutex_init(&LP_inusemutex);
    portable_mutex_init(&LP_psockmutex);
    portable_mutex_init(&LP_coinmutex);
    portable_mutex_init(&LP_pubkeymutex);
    portable_mutex_init(&LP_electrummutex);
    portable_mutex_init(&LP_messagemutex);
    portable_mutex_init(&LP_portfoliomutex);
    portable_mutex_init(&LP_butxomutex);
    portable_mutex_init(&LP_reservedmutex);
    portable_mutex_init(&LP_nanorecvsmutex);
    portable_mutex_init(&LP_tradebotsmutex);
    portable_mutex_init(&LP_cJSONmutex);
    portable_mutex_init(&LP_logmutex);
    portable_mutex_init(&LP_statslogmutex);
    portable_mutex_init(&LP_tradesmutex);
    portable_mutex_init(&LP_commandQmutex);
    portable_mutex_init(&LP_blockinit_mutex);
    portable_mutex_init(&LP_pendswap_mutex);
    portable_mutex_init(&LP_listmutex);
    portable_mutex_init(&LP_gtcmutex);
}

void* r_btc_ctx(uint32_t mm_ctx_id);

void LPinit(uint32_t mm_ctx_id)
{
    void* ctx;
#ifndef FROM_JS
    if ((ctx = r_btc_ctx(mm_ctx_id)) == 0) return;
    if ( OS_thread_create(malloc(sizeof(pthread_t)),NULL,(void *)LP_pubkeysloop,(void *)&mm_ctx_id) != 0 )
    {
        printf("error launching LP_pubkeysloop for ctx.%p\n",ctx);
        exit(-1);
    }
    int32_t didremote=0;
    LP_statslog_parse();

    if (SPAWN_RPC != 0) SPAWN_RPC(mm_ctx_id);
    else
    {
        printf("SPAWN_RPC is not initialized!\n");
        exit(-1);
    }

    //LP_mpnet_init(); seems better to have the GUI send in persistent orders, exit mm is a cancel all
    while ( LP_STOP_RECEIVED == 0 )
    {
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
        if (r_btc_ctx(mm_ctx_id) == 0) return;
        if ( IAMLP != 0 && didremote == 0 && LP_cmdcount > 0 )
        {
            didremote = 1;
            printf("start remote port\n");
        }
        sleep(1);
    }
#endif
    printf("marketmaker exiting in 5 seconds\n");
    sleep(5);
}

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
