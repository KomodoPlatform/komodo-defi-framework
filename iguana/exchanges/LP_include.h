
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
//  LP_include.h
//  marketmaker
//

#ifndef LP_INCLUDE_H
#define LP_INCLUDE_H

#include <stdint.h>

#ifndef LP_TECHSUPPORT
#define LP_TECHSUPPORT 1
#endif

#define LP_DONT_CMDCHANNEL

#ifdef FROMGUI
#define printf dontprintf
//#define fprintf fdontprintf dont do this!

void dontprintf(char *formatstr,...) {}
//void fdontprintf(FILE *fp,char *formatstr,...) {}
#endif

#define LP_MAJOR_VERSION "0"
#define LP_MINOR_VERSION "1"
#define LP_BUILD_NUMBER "27774"
#define LP_BARTERDEX_VERSION 1
#define LP_MAGICBITS 1

#define LP_DONT_IMPORTPRIVKEY

#ifdef FROM_JS
#include <emscripten.h>
#define sleep(x) emscripten_usleep((x) * 1000000)
void emscripten_usleep(int32_t x); // returns immediate, no sense for sleeping
#define usleep(x) emscripten_usleep(x)
// ./autogen.sh
// emconfigure ./configure CFLAGS="-s PTHREAD_POOL_SIZE=8 -s USE_PTHREADS=1 -O2"
// Edit src/core/sock.c and add here #include <limits.h> for INT_MAX support
// emmake make
// cp .libs/libnanomsg.a ~/SuperNET/OSlibs/js
#endif
//#define LP_STRICTPEERS

//#define LP_DISABLE_DISTCOMBINE

#define LP_MAXVINS 64
#define LP_HTTP_TIMEOUT 10 // 1 is too small due to edge cases of time(NULL)
#define ELECTRUM_TIMEOUT 13
#define LP_ELECTRUM_KEEPALIVE 60
#define LP_ELECTRUM_MAXERRORS 777
#define LP_MEMPOOL_TIMEINCR 10
#define LP_SCREENWIDTH 1024

#define LP_MIN_PEERS 8
#define LP_MAX_PEERS 32

#define LP_MAXDESIRED_UTXOS (IAMLP != 0 ? 256 : 64)
#define LP_MINDESIRED_UTXOS (IAMLP != 0 ? 64 : 16)
#define LP_DUSTCOMBINE_THRESHOLD 1000000

// RTmetrics
#define LP_RTMETRICS_TOPGROUP 1.01
//#define LP_MAXPENDING_SWAPS 13
#define LP_CLIENT_STATSPARSE (90 * 1024 * 1024)

#define DPOW_MIN_ASSETCHAIN_SIGS 11
#define LP_ENCRYPTED_MAXSIZE (16384 + 2 + crypto_box_NONCEBYTES + crypto_box_ZEROBYTES)

#define LP_MAXPUBKEY_ERRORS 10
#define PSOCK_KEEPALIVE 3600
#define MAINLOOP_PERSEC 100
#define MAX_PSOCK_PORT 60000
#define MIN_PSOCK_PORT 10000
#define LP_GETINFO_INCR 30

#define LP_MAXPEER_ERRORS 3
#define LP_MINPEER_GOOD 20
#define LP_PEERGOOD_ERRORDECAY 0.9

#define LP_SWAPSTEP_TIMEOUT 30
/// Used to initialize `iguana_info::txfee` for all the currencies except "BTC".
#define LP_MIN_TXFEE 1000
#define LP_MINVOL 100
#define LP_MINCLIENTVOL 1000
#define LP_MINSIZE_TXFEEMULT 10
#define LP_REQUIRED_TXFEE 0.75

#define LP_DEXFEE(destsatoshis) ((destsatoshis) / INSTANTDEX_INSURANCEDIV)
#define LP_DEPOSITSATOSHIS(satoshis) ((satoshis) + (satoshis >> 3))

#define INSTANTDEX_DECKSIZE 1000
#define INSTANTDEX_LOCKTIME (3600*2 + 300*2)
#define INSTANTDEX_INSURANCEDIV 777
#define INSTANTDEX_PUBKEY "03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"
#define INSTANTDEX_ETH_ADDR "0x3f17f1962B36e491b30A40b2405849e597Ba5FB5"
#define INSTANTDEX_RMD160 "ca1e04745e8ca0c60d8c5881531d51bec470743f"
#define JUMBLR_RMD160 "5177f8b427e5f47342a4b8ab5dac770815d4389e"
#define TIERNOLAN_RMD160 "daedddd8dbe7a2439841ced40ba9c3d375f98146"
#define INSTANTDEX_BTC "1KRhTPvoxyJmVALwHFXZdeeWFbcJSbkFPu"
#define INSTANTDEX_KMD "RThtXup6Zo7LZAi8kRWgjAyi1s4u6U9Cpf"
/// Used for "KMD" with `LP_importaddress`.
#define BOTS_BONDADDRESS "RNdqHx26GWy9bk8MtmH1UiXjQcXE4RKK2P"
#define BOTS_BONDPUBKEY33 "03e641d22e1ff5a7d45c8880537e0b0a114d7b9fee2c18a6b4a8a80b6285292990"
#define LP_WEEKMULTBAD (7 * 24 * 2600)
#define LP_WEEKMULT (7 * 24 * 3600)
#define LP_FIRSTWEEKTIME 1510790400 // must be 0 mod LP_WEEKMULT

//#define BASILISK_DISABLEWAITTX
//#define BASILISK_DISABLESENDTX
#define LP_RPCPORT 7783

#define LP_PROPAGATION_SLACK 100 // txid ordering is not enforced, so getting extra recent txid
#define LP_AVETXSIZE 256
#define LP_CACHEDURATION 60
#define BASILISK_DEFAULT_NUMCONFIRMS 1
#define BASILISK_DEFAULT_MAXCONFIRMS 6
#define DEX_SLEEP 3
#define BASILISK_KEYSIZE ((int32_t)(2*sizeof(bits256)+sizeof(uint32_t)*2))

#define LP_IS_ZCASHPROTOCOL 1
#define LP_IS_BITCOINCASH 2
#define LP_IS_BITCOINGOLD 79

#define SIGHASH_FORKID 0x40
#define ZKSNARK_PROOF_SIZE 296
#define ZCASH_SOLUTION_ELEMENTS 1344

#define LP_REQUEST 0
#define LP_RESERVED 1
#define LP_CONNECT 2
#define LP_CONNECTED 3

#define LP_DONTCHANGE_ERRMSG0 "couldnt find coin locally installed"
#define LP_DONTCHANGE_ERRMSG1 "coin is disabled"

extern char GLOBAL_DBDIR[512];

/// This flag translates as "I Am Liquidity Provider (that is, Bob, Maker)",
/// With MM2 all users can be a Maker, to this flag must ALWAYS be 1.
extern int32_t IAMLP;

struct iguana_msgvin
{
    bits256 prev_hash;
    uint8_t *vinscript,*userdata,*spendscript,*redeemscript;
    uint32_t prev_vout,sequence; uint16_t scriptlen,p2shlen,userdatalen,spendlen;
};

struct iguana_msgvout { uint64_t value; uint32_t pk_scriptlen; uint8_t *pk_script; };

struct iguana_msgtx
{
    uint32_t version,tx_in,tx_out,lock_time;
    struct iguana_msgvin *vins;
    struct iguana_msgvout *vouts;
    bits256 txid;
    int32_t allocsize,timestamp,numinputs,numoutputs;
    int64_t inputsum,outputsum,txfee;
    uint8_t *serialized;
};

struct iguana_msgjoinsplit
{
    uint64_t vpub_old,vpub_new;
    bits256 anchor,nullifiers[2],commitments[2],ephemeralkey;
    bits256 randomseed,vmacs[2];
    uint8_t zkproof[ZKSNARK_PROOF_SIZE];
    uint8_t ciphertexts[2][601];
};

struct vin_signer { bits256 privkey; char coinaddr[64]; uint8_t siglen,sig[80],rmd160[20],pubkey[66]; };

struct vin_info
{
    struct iguana_msgvin vin; uint64_t amount; cJSON *extras; bits256 sigtxid;
    int32_t M,N,validmask,spendlen,type,p2shlen,numpubkeys,numsigs,height,userdatalen,suppress_pubkeys,ignore_cltverr;
    uint32_t sequence,unspentind,hashtype; struct vin_signer signers[16]; char coinaddr[65];
    uint8_t rmd160[20],spendscript[10000],p2shscript[10000],userdata[10000];
};

/*struct basilisk_swapmessage
{
    bits256 srchash,desthash;
    uint32_t crc32,msgbits,quoteid,datalen;
    uint8_t *data;
};*/

struct basilisk_rawtxinfo
{
    char destaddr[64],ethTxid[75];
    bits256 txid,signedtxid,actualtxid;
    int64_t amount,change,inputsum,eth_amount;
    int32_t redeemlen,datalen,completed,vintype,vouttype,numconfirms,spendlen,secretstart,suppress_pubkeys;
    uint32_t locktime,crcs[2];
    uint8_t addrtype,pubkey33[33],rmd160[20];
};

struct basilisk_request
{
    uint32_t requestid,timestamp,quoteid,quotetime; // 0 to 15
    int64_t srcamount,unused; // 16 to 31
    bits256 srchash; // 32 to 63
    bits256 desthash;
    char src[68],dest[68];
    uint64_t destamount;
    uint32_t optionhours,DEXselector;
};

struct basilisk_rawtx
{
    char name[32],symbol[65];
    struct iguana_msgtx msgtx;
    struct basilisk_rawtxinfo I;
    char vinstr[8192],p2shaddr[64];
    cJSON *vins;
    bits256 utxotxid; int32_t utxovout;
    uint8_t txbytes[16384],spendscript[512],redeemscript[1024],extraspace[4096],pubkey33[33];
};

#define BASILISK_MYFEE 0
#define BASILISK_OTHERFEE 1
#define BASILISK_BOBDEPOSIT 2
#define BASILISK_ALICEPAYMENT 3
#define BASILISK_BOBPAYMENT 4
#define BASILISK_ALICESPEND 5
#define BASILISK_BOBSPEND 6
#define BASILISK_BOBREFUND 7
#define BASILISK_BOBRECLAIM 8
#define BASILISK_ALICERECLAIM 9
#define BASILISK_ALICECLAIM 10
//0, 0, 1, 1, 1, 0, 1, 1, 1, 0, 0
static char *txnames[] = { "myfee", "otherfee", "bobdeposit", "alicepayment", "bobpayment", "alicespend", "bobspend", "bobrefund", "bobreclaim", "alicereclaim", "aliceclaim" };

struct LP_swap_remember
{
    bits256 privAm,privBn,paymentspent,Apaymentspent,depositspent,myprivs[2],txids[sizeof(txnames)/sizeof(*txnames)];
    uint64_t Atxfee,Btxfee,srcamount,destamount,aliceid,alicerealsat,bobrealsat;
    int64_t values[sizeof(txnames)/sizeof(*txnames)];
    uint32_t finishtime,tradeid,requestid,quoteid,plocktime,dlocktime,expiration,state,otherstate;
    int32_t iambob,finishedflag,origfinishedflag,Predeemlen,Dredeemlen,sentflags[sizeof(txnames)/sizeof(*txnames)];
    uint8_t secretAm[20],secretAm256[32],secretBn[20],secretBn256[32],Predeemscript[1024],Dredeemscript[1024],pubkey33[33],other33[33];
    uint8_t pubA0[33],pubB0[33],pubB1[33];
    char uuidstr[65],Agui[65],Bgui[65],gui[65],src[65],dest[65],bobtomic[128],alicetomic[128],etomicsrc[65],etomicdest[65],destaddr[64],Adestaddr[64],Sdestaddr[64],alicepaymentaddr[64],bobpaymentaddr[64],bobdepositaddr[64],alicecoin[65],bobcoin[65],*txbytes[sizeof(txnames)/sizeof(*txnames)];
    char eth_tx_ids[sizeof(txnames)/sizeof(*txnames)][75];
    int64_t eth_values[sizeof(txnames)/sizeof(*txnames)];
};

struct LP_outpoint
{
    bits256 spendtxid;
    uint64_t value,interest;
    int32_t spendvini,spendheight;
    char coinaddr[64];
};

struct LP_transaction
{
    UT_hash_handle hh;
    bits256 txid;
    long fpos;
    int32_t height,numvouts,numvins,len,SPV;
    uint8_t *serialized;
    struct LP_outpoint outpoints[];
};

struct iguana_info
{
    UT_hash_handle hh;
    void *_txmutex, *_addrmutex, *_addressutxo_mutex;
    struct LP_transaction *transactions;
    struct LP_address *addresses;
    uint64_t txfee,do_autofill_merge;
    int32_t numutxos,notarized,longestchain,firstrefht,firstscanht,lastscanht,height,txversion; uint16_t busport,did_addrutxo_reset;
    uint32_t dPoWtime,lastautosplit,lastresetutxo,loadedcache,electrumlist,lastunspent,importedprivkey,lastpushtime,lastutxosync,addr_listunspent_requested,lastutxos,updaterate,counter,inactive,lastmempool,lastgetinfo,ratetime,heighttime,lastmonitor,obooktime;
    uint8_t pubtype,p2shtype,isPoS,wiftype,wiftaddr,taddr,noimportprivkey_flag,userconfirms,isassetchain,maxconfirms;
    char symbol[128],smartaddr[64],userpass[1024];
    /// The "$host:$port" address of the coin wallet.  
    /// Fetched from the wallet config when we can find it.
    char serverport[128];
    char instantdex_address[64],estimatefeestr[32],getinfostr[32],etomic[64],validateaddress[64];
    // portfolio
    double price_kmd,force,perc,goal,goalperc,relvolume,rate;
    void *electrum; void *ctx;
    uint64_t maxamount,kmd_equiv,balanceA,balanceB,valuesumA,valuesumB,fillsatoshis;
    uint8_t pubkey33[33],zcash,decimals,overwintered;
    int32_t privkeydepth,bobfillheight;
    void *curl_handle; void* _curl_mutex;
    bits256 cachedtxid,notarizationtxid; uint8_t *cachedtxiddata; int32_t cachedtxidlen;
    bits256 cachedmerkle,notarizedhash; int32_t cachedmerkleheight;
};
extern struct iguana_info *LP_coins;

struct _LP_utxoinfo { bits256 txid; uint64_t value; int32_t vout,height; };

struct LP_utxostats { uint32_t sessionid,lasttime,errors,swappending,spentflag,lastspentcheck,bestflag; };

struct LP_utxoinfo
{
    UT_hash_handle hh,hh2;
    bits256 pubkey;
    struct _LP_utxoinfo payment,deposit,fee;
    struct LP_utxostats T;
    int64_t swap_satoshis;
    //struct LP_utxoswap S;
    int32_t iambob,iamlp;
    uint8_t key[sizeof(bits256) + sizeof(int32_t)];
    uint8_t key2[sizeof(bits256) + sizeof(int32_t)];
    char coin[65],coinaddr[64],gui[16];//spendscript[256];
};

struct LP_address_utxo
{
    struct LP_address_utxo *next,*prev;
    struct _LP_utxoinfo U;
    int32_t SPV,spendheight;
    //uint32_t timestamp;
};

struct LP_address
{
    UT_hash_handle hh;
    struct LP_address_utxo *utxos;
    bits256 pubkey;
    int64_t balance,total,instantdex_credits;
    uint32_t timestamp,n,unspenttime,instantdextime;
    int32_t unspentheight;
    char coinaddr[64];
    uint8_t pubsecp[33],didinstantdex;
};

struct LP_peerinfo
{
    UT_hash_handle hh;
    bits256 pubkey;
    uint64_t ip_port;
    uint32_t recvtime,numrecv,ipbits,errortime,errors,numpeers,needping,lasttime,connected,lastutxos,lastpeers,diduquery,good,sessionid;
    int32_t pushsock,subsock,isLP,pairsock;
    uint16_t port,netid;
    char ipaddr[64];
};
extern struct LP_peerinfo *LP_mypeer;

struct LP_quoteinfo
{
    struct basilisk_request R;
    bits256 srchash,desthash,privkey;
    double maxprice;
    int64_t othercredits;
    uint64_t satoshis,txfee,destsatoshis,desttxfee,aliceid;
    uint32_t timestamp,quotetime,tradeid,gtc,fill,mpnet;
    int32_t pair;
    char srccoin[65],coinaddr[64],destcoin[65],destaddr[64],gui[64],etomicsrc[65],etomicdest[65],uuidstr[65];
};

struct LP_pubkey_quote
{
    struct LP_pubkey_quote *next,*prev;
    float price;
    uint8_t baseind,relind,scale;
    double_t balance;
};

struct LP_swapstats
{
    UT_hash_handle hh;
    struct LP_quoteinfo Q;
    bits256 bobdeposit,alicepayment,bobpayment,paymentspent,Apaymentspent,depositspent;
    int32_t bobdeposit_ht,alicepayment_ht,bobpayment_ht,paymentspent_ht,Apaymentspent_ht,depositspent_ht;
    double qprice;
    uint64_t aliceid;
    int32_t bobneeds_dPoW,aliceneeds_dPoW;
    uint32_t ind,methodind,finished,expired,lasttime,dPoWfinished;
    char alicegui[65],bobgui[65];
};

struct LP_pubswap { struct LP_pubswap *next,*prev; struct LP_swapstats *swap; };

#define LP_MAXPRICEINFOS 255
struct LP_pubkey_info
{
    UT_hash_handle hh;
    bits256 pubkey;
    struct LP_pubkey_quote *quotes;
    struct LP_pubswap *bobswaps,*aliceswaps;
    int64_t dynamictrust,unconfcredits;
    uint32_t timestamp,numerrors,lasttime,slowresponse;
    int32_t istrusted,pairsock;
    uint8_t rmd160[20],sig[65],pubsecp[33],siglen;
};

struct LP_trade
{
    uint64_t aliceid,lastprocessed,negotiationdone,connectsent,firstprocessed,newtime;
    int64_t besttrust,bestunconfcredits;
    double bestprice;
    uint32_t bestresponse,firsttime,lasttime,cancelled,funcid;
    char pairstr[64],iambob;
    struct LP_quoteinfo Qs[4],Q;
};

uint32_t LP_sighash(char *symbol,int32_t zcash);

/// Cryptographically checks the `pubp.pubkey`
/// using the `item` fields "rmd160", "pubsecp", "sig" and "timestamp".
/// Initializes `pubp.rmd160`, `pubp.pubsecp`, `pubp.sig`, `pubp.siglen`, `pubp.timestamp`,
/// or increments `pubp.numerrors` on error.
int32_t LP_pubkey_sigcheck(struct LP_pubkey_info *pubp,cJSON *item);

int32_t LP_pubkey_sigadd(cJSON *item,uint32_t timestamp,bits256 priv,bits256 pub,uint8_t *rmd160,uint8_t *pubsecp);
int32_t LP_quoteparse(struct LP_quoteinfo *qp,cJSON *argjson);
/// Find the given address in `coin->addresses`.
struct LP_address *LP_address(struct iguana_info *coin,char *coinaddr);
uint32_t basilisk_requestid(struct basilisk_request *rp);
uint32_t basilisk_quoteid(struct basilisk_request *rp);
struct basilisk_swap *LP_swapinit(int32_t iambob,int32_t optionduration,bits256 privkey,struct basilisk_request *rp,struct LP_quoteinfo *qp,int32_t dynamictrust);
struct LP_peerinfo *LP_peerfind(uint32_t ipbits,uint16_t port);
uint64_t LP_value_extract(cJSON *obj,int32_t addinterest,bits256 txid);
int64_t LP_kmdvalue(char *symbol,int64_t satoshis);
void LP_availableset(bits256 txid,int32_t vout);
int64_t LP_listunspent_parseitem(struct iguana_info *coin,bits256 *txidp,int32_t *voutp,int32_t *heightp,cJSON *item);
void LP_unspents_cache(char *symbol,char *addr,char *arraystr,int32_t updatedflag);
uint16_t LP_psock_get(char *connectaddr,char *publicaddr,int32_t ispaired,int32_t cmdchannel,char *ipaddr);
void LP_failedmsg(uint32_t requestid,uint32_t quoteid,double val,char *uuidstr);
int32_t LP_nanomsg_recvs(void *ctx);
void LP_aliceid(uint32_t tradeid,uint64_t aliceid,char *event,uint32_t requestid,uint32_t quoteid);
cJSON *LP_cache_transaction(struct iguana_info *coin,bits256 txid,uint8_t *serialized,int32_t len);
uint64_t LP_balance(uint64_t *valuep,int32_t iambob,char *symbol,char *coinaddr);
cJSON *LP_transaction_fromdata(struct iguana_info *coin,bits256 txid,uint8_t *serialized,int32_t len);
uint64_t LP_RTsmartbalance(struct iguana_info *coin);
int32_t LP_getheight(int32_t *notarizedp,struct iguana_info *coin);
int32_t LP_reserved_msg(int32_t priority,bits256 pubkey,char *msg);
void LP_coinadd_(struct iguana_info *cdata, int32_t iguana_info_size);
/// Deprecated, use the ported `coins::lp_coinfind` instead.
struct iguana_info *LP_coinfind(char *symbol);
/// Returns the "BTC" and "KMD" ports defined in `portstrs`. 0 for other currencies.
uint16_t LP_rpcport(char *symbol);
int32_t LP_crc32find(int32_t *duplicatep,int32_t ind,uint32_t crc32);
char *LP_pricepings(char *base,char *rel,double price);
cJSON *LP_myzdebits();
int32_t LP_opreturn_decrypt(uint16_t *ind16p,uint8_t *decoded,uint8_t *encoded,int32_t encodedlen,char *passphrase);
int32_t LP_opreturn_encrypt(uint8_t *dest,int32_t maxsize,uint8_t *data,int32_t datalen,char *passphrase,uint16_t ind16);
void LP_pendswap_add(uint32_t expiration,uint32_t requestid,uint32_t quoteid);
char *bitcoin_address(char *symbol,char *coinaddr,uint8_t taddr,uint8_t addrtype,uint8_t *pubkey_or_rmd160,int32_t len);
void LP_tradebot_finished(uint32_t tradeid,uint32_t requestid,uint32_t quoteid);
uint64_t LP_txfeecalc(struct iguana_info *coin,uint64_t txfee,int32_t txlen);
struct LP_address *_LP_address(struct iguana_info *coin,char *coinaddr);
struct LP_address *_LP_addressfind(struct iguana_info *coin,char *coinaddr);
struct LP_address *_LP_addressadd(struct iguana_info *coin,char *coinaddr);
int32_t iguana_signrawtransaction(void *ctx,char *symbol,uint8_t wiftaddr,uint8_t taddr,uint8_t pubtype,uint8_t p2shtype,uint8_t isPoS,int32_t height,struct iguana_msgtx *msgtx,char **signedtxp,bits256 *signedtxidp,struct vin_info *V,int32_t numinputs,char *rawtx,cJSON *vins,cJSON *privkeysjson,int32_t zcash);
//void LP_butxo_swapfields_set(struct LP_utxoinfo *butxo);
struct LP_address_utxo *LP_address_utxofind(struct iguana_info *coin,char *coinaddr,bits256 txid,int32_t vout);
int64_t LP_myzcredits();
void HashKeccak(uint8_t *hash,void *data,size_t len);
void test_validate(struct iguana_info *coin,char *signedtx);
void LP_instantdex_depositadd(char *coinaddr,bits256 txid);
int64_t LP_instantdex_creditcalc(struct iguana_info *coin,int32_t dispflag,bits256 txid,char *refaddr,char *origcoinaddr);
void LP_ports(uint16_t *pullportp,uint16_t *pubportp,uint16_t *busportp,uint16_t netid);
int32_t LP_destaddr(char *destaddr,cJSON *item);
cJSON *LP_statslog_disp(uint32_t starttime,uint32_t endtime,char *refgui,bits256 refpubkey,char *refbase,char *refrel);
uint32_t LP_claimtime(struct iguana_info *coin,uint32_t expiration);
uint32_t LP_heighttime(char *symbol,int32_t height);
uint64_t LP_unspents_load(char *symbol,char *addr);
int32_t LP_validSPV(char *symbol,char *coinaddr,bits256 txid,int32_t vout);
struct LP_transaction *LP_transactionfind(struct iguana_info *coin,bits256 txid);
int32_t LP_txheight(struct iguana_info *coin,bits256 txid);
int32_t LP_numpeers();
char *basilisk_swapentry(int32_t fastflag,uint32_t requestid,uint32_t quoteid,int32_t forceflag);
int64_t LP_KMDvalue(struct iguana_info *coin,int64_t balance);
cJSON *LP_address_utxos(struct iguana_info *coin,char *coinaddr,int32_t electrumret);
uint16_t LP_randpeer(char *destip);
void LP_tradebot_pauseall();
void LP_portfolio_reset();
int32_t LP_autoref_clear(char *base,char *rel);
int32_t bitcoin_addr2rmd160(char *symbol,uint8_t taddr,uint8_t *addrtypep,uint8_t rmd160[20],char *coinaddr);

/// Points at the `LP_pubkey_info` structure corresponding to the given hash,
/// allocating that structure on the `LP_pubkeyinfos` as necessary.
struct LP_pubkey_info *LP_pubkeyadd(bits256 pubkey);

uint32_t LP_atomic_locktime(char *base,char *rel);
struct LP_pubkey_info *LP_pubkeyfind(bits256 pubkey);
char *LP_unspents_filestr(char *symbol,char *addr);
//int32_t LP_butxo_findeither(bits256 txid,int32_t vout);
cJSON *LP_listunspent(char *symbol,char *coinaddr,bits256 reftxid,bits256 reftxid2);
double LP_getestimatedrate(struct iguana_info *coin);
struct LP_utxoinfo *_LP_utxofind(int32_t iambob,bits256 txid,int32_t vout);
struct LP_utxoinfo *_LP_utxo2find(int32_t iambob,bits256 txid,int32_t vout);
int64_t LP_dynamictrust(int64_t credits,bits256 pubkey,int64_t kmdvalue);
struct LP_address *LP_addressfind(struct iguana_info *coin,char *coinaddr);
int32_t LP_trades_canceluuid(char *uuidstr);
int _decreasing_uint64(const void *a,const void *b);
int32_t LP_alice_eligible(uint32_t quotetime);
int32_t LP_is_slowcoin(char *symbol);
void LP_alicequery_clear();

int32_t bitcoin_priv2wif(char *symbol,uint8_t wiftaddr,char *wifstr,bits256 privkey,uint8_t addrtype);
int bech32_convert_bits(uint8_t *out,int32_t *outlen,int outbits,const uint8_t *in,int32_t inlen,int inbits,int pad);
int bech32_decode(char *hrp,uint8_t *data,int32_t *data_len,const char *input);
int bech32_encode(char *output,const char *hrp,const uint8_t *data,int32_t data_len);
void HashGroestl(void * buf, const void * pbegin, int len);
bits256 LP_privkey(char *symbol,char *coinaddr,uint8_t taddr);

struct LP_priceinfo
{
    char symbol[68];
    uint64_t coinbits;
    int32_t ind,pad;
    double diagval,high[2],low[2],last[2],bid[2],ask[2];
    double relvals[LP_MAXPRICEINFOS];
    double myprices[2][LP_MAXPRICEINFOS];
    double minprices[LP_MAXPRICEINFOS]; // autoprice
    double fixedprices[LP_MAXPRICEINFOS]; // fixedprices
    double buymargins[LP_MAXPRICEINFOS];
    double sellmargins[LP_MAXPRICEINFOS];
    double offsets[LP_MAXPRICEINFOS];
    double factors[LP_MAXPRICEINFOS];
} LP_priceinfos[LP_MAXPRICEINFOS];

struct LP_priceinfo *LP_priceinfoadd(char *symbol);

// Gradual port temporaries.

cJSON *LP_NXT_redeems();
void LPinit(char* myipaddr,uint16_t mypullport,uint16_t mypubport,char *passphrase,cJSON *argjson,uint32_t mm_ctx_id);
void unbuffered_output_support(const char* log_path);
void LP_dPoW_request(struct iguana_info *coin);
struct iguana_info *LP_conflicts_find(struct iguana_info *refcoin);
// The `item` here is an entry from the "coins" command-line configuration.
/// Helps `lp_coininit` to initialize the `userpass`.
uint16_t LP_userpass(char *userpass,char *symbol,char *assetname,char *confroot,char *name,char *confpath,uint16_t origport);
/// Helps `lp_coininit` to initialize things that we can't readily initialize from Rust.
void LP_coin_curl_init(struct iguana_info* coin);
void LP_mutex_init();
struct LP_priceinfo *LP_priceinfofind(char *symbol);
/// `num_LP_autorefs` is incremeted in `lp_autoprice`, invoked by RPC method "autoprice".
extern int32_t num_LP_autorefs;
char *LP_portfolio();
int32_t LP_portfolio_trade(void *ctx,uint32_t *requestidp,uint32_t *quoteidp,struct iguana_info *buy,struct iguana_info *sell,double relvolume,int32_t setbaserel,char *gui);
struct LP_portfoliotrade { double metric; char buycoin[65],sellcoin[65]; };
int32_t LP_portfolio_order(struct LP_portfoliotrade *trades,int32_t max,cJSON *array);
double LP_pricesparse(void *ctx,int32_t trexflag,char *retstr,struct LP_priceinfo *btcpp);
char *LP_ticker(char *refbase,char *refrel);
int32_t LP_mypriceset(int32_t iambob,int32_t *changedp,char *base,char *rel,double price);
void LP_autopriceset(int32_t ind,void *ctx,int32_t dir,struct LP_priceinfo *basepp,struct LP_priceinfo *relpp,double price,char *refbase,char *refrel);
cJSON *LP_balances(char *coinaddr);
char *unstringify(char *str);
int32_t LP_privkey_init(int32_t mypubsock,struct iguana_info *coin,bits256 myprivkey,bits256 mypub);
void vcalc_sha256(char hashstr[(256 >> 3) * 2 + 1],uint8_t hash[256 >> 3],uint8_t *src,int32_t len);
cJSON *LP_coinjson(struct iguana_info *coin,int32_t showwif);
bits256 LP_privkeycalc(bits256 *pubkeyp,char *passphrase);
void LP_privkey_updates(void *ctx,int32_t pubsock,char *passphrase);
bits256 bitcoin_pubkey33(void *ctx,uint8_t *data,bits256 privkey);
void LP_priceinfos_clear();
struct LP_peerinfo *LP_addpeer(struct LP_peerinfo *mypeer,int32_t mypubsock,char *ipaddr,uint16_t port,uint16_t pushport,uint16_t subport,int32_t isLP,uint32_t sessionid,uint16_t netid);

/// Finds an `ipaddr` peer entry, increments the `numrecv` counter, sets the `recvtime` timestamp, updates the `pubkey` and `pairsock`.
void LP_peer_recv(char *ipaddr,int32_t ismine,struct LP_pubkey_info *pubp);

struct LP_autoprice_ref
{
    // AG: Most of these fields are already present in `fundvalue`, duplicating them
    // might actually complicate things by introducing more unnecessary layers between the "autoprice" configuration and the price loop.
    // Consider refactoring away the unnecessary fields.
    char refbase[65],refrel[65],base[65],rel[65],fundbid[16],fundask[16],usdpeg;
    double buymargin,sellmargin,factor,offset,lastbid,lastask;
    // `Box::into_raw` of `AutopriceReq`.
    void* fundvalue_req;
    uint32_t count;
};
extern struct LP_autoprice_ref LP_autorefs[1024];

/**
 * Contains IP bits parsed from the "docker" parameter.  
 * Deprecated (setting IP address should not require Docker,
 * there is now a generic "myipaddr" (`LP_myipaddr`) parameter for that,
 * plus we don't want to be locked into IPv4).
 */
extern uint32_t DOCKERFLAG;
extern int32_t LP_STOP_RECEIVED;
extern double LP_profitratio;
extern int32_t bitcoind_RPC_inittime;
extern int32_t LP_showwif;
extern char LP_gui[65];

/// `0` in the "client" mode, `1` otherwise.
/// My hypothesis is that we might be using the "client" mode to cull out the potentially short-lived instances of MM
/// from the peer-to-peer netowkr in order to make the latter more stable.
extern int32_t LP_canbind;

/// Initialized from the "canbind" configuration knob.
extern uint16_t LP_fixed_pairport;
/// This is either the IP address configured by the user
/// or the automatically discovered outer IP address (which makes it pretty much useless for binding).
/// Recommendation is to avoid using this value,
/// because the address to bind to and the outer IP address are different things.
/// Bind on the `conf["myipaddr"]` instead, and if it is not specified then on a generic address (0.0.0.0 for IPv4).
extern char LP_myipaddr[64];
extern int32_t LP_mypubsock;
/// Defaults to -1.
extern int32_t LP_mypullsock;
extern uint16_t RPC_port;
/**
 * Boolean. `1` if the command-line "myipaddr" field was used to set the `LP_myipaddr`.
 * 
 * Tells "stats.c" to bind on `LP_myipaddr` and not on "0.0.0.0".
 * (We might replicate that binding logic in other places in the future).
 */
extern uint8_t LP_myipaddr_from_command_line;
extern char USERHOME[512];
void (*SPAWN_RPC)(uint32_t);
void (*LP_QUEUE_COMMAND)(char**,char*,int32_t,int32_t,uint32_t);

extern int32_t IPC_ENDPOINT;
char *stats_JSON(void *ctx,char *myipaddr,int32_t mypubsock,cJSON *argjson,char *remoteaddr,uint16_t port,int32_t authenticated, uint8_t loopback_only);
char *LP_instantdex_deposit(struct iguana_info *coin,int32_t weeks,double amount,int32_t broadcast);

struct LP_privkey { bits256 privkey; uint8_t rmd160[20]; };

struct LP_globals
{
    //struct LP_utxoinfo  *LP_utxoinfos[2],*LP_utxoinfos2[2];
    /// Our public peer-to-peer key.
    bits256 LP_mypub25519;
    bits256 LP_privkey,LP_mypriv25519,LP_passhash;
    uint64_t LP_skipstatus[10000];
    uint16_t netid;
    uint8_t LP_myrmd160[20],LP_pubsecp[33];
    uint32_t LP_sessionid,counter,mpnet;
    /// True (`1`) if we've been notified by other peers about ourselves (cf. `fn lp_notify_recv`).
    int32_t LP_IAMLP;
    int32_t LP_pendingswaps;
    /// We set it to `1` in RPC "passphrase".
    int32_t USERPASS_COUNTER;
    int32_t LP_numprivkeys,initializing,waiting,LP_numskips;
    char seednode[64],USERPASS[65],USERPASS_WIFSTR[64],LP_myrmd160str[41],gui[65],LP_NXTaddr[64];
    struct LP_privkey LP_privkeys[100];
} G;

extern uint32_t LP_ORDERBOOK_DURATION;
// Corresponds to the "time expired for Alice_request" timeout failures during the SWAP. Seconds.
extern uint32_t LP_AUTOTRADE_TIMEOUT;
extern uint32_t LP_RESERVETIME;
extern uint32_t Alice_expiration;

void LP_txfees(uint64_t *txfeep,uint64_t *desttxfeep,char *base,char *rel);
double LP_fomoprice(char *base,char *rel,double *relvolumep);
struct LP_utxoinfo *LP_address_myutxopair(struct LP_utxoinfo *butxo,int32_t iambob,struct LP_address_utxo **utxos,int32_t max,struct iguana_info *coin,char *coinaddr,uint64_t txfee,double relvolume,double price,uint64_t desttxfee);
uint64_t LP_basesatoshis(double relvolume,double price,uint64_t txfee,uint64_t desttxfee);
int32_t LP_quoteinfoinit(struct LP_quoteinfo *qp,struct LP_utxoinfo *utxo,char *destcoin,double price,uint64_t satoshis,uint64_t destsatoshis);
int32_t LP_quotedestinfo(struct LP_quoteinfo *qp,bits256 desthash,char *destaddr);
int32_t LP_mypriceset(int32_t iambob,int32_t *changedp,char *base,char *rel,double price);
char *LP_trade(struct LP_quoteinfo *qp,double maxprice,int32_t timeout,uint32_t tradeid,bits256 destpubkey,char *uuidstr, uint32_t ctx_h);
void gen_quote_uuid(char *result, char *base, char* rel);
int32_t decode_hex(unsigned char *bytes,int32_t n,char *hex);
uint64_t LP_aliceid_calc(bits256 desttxid,int32_t destvout,bits256 feetxid,int32_t feevout);
uint32_t LP_rand();
void LP_gtc_addorder(struct LP_quoteinfo *qp);
void LP_query(char *method,struct LP_quoteinfo *qp, uint32_t ctx_h);
extern struct LP_quoteinfo LP_Alicequery;
extern double LP_Alicemaxprice;
extern bits256 LP_Alicedestpubkey;

cJSON *LP_quotejson(struct LP_quoteinfo *qp);
void LP_mpnet_send(int32_t localcopy,char *msg,int32_t sendflag,char *otheraddr);
/// Converts the DB/SWAPS/list into a JSON.
char *LP_recent_swaps(int32_t limit,char *uuidstr);
struct LP_address *LP_address(struct iguana_info *coin,char *coinaddr);
int32_t LP_address_utxo_ptrs(struct iguana_info *coin,int32_t iambob,struct LP_address_utxo **utxos,int32_t max,struct LP_address *ap,char *coinaddr);
int32_t LP_nearest_utxovalue(struct iguana_info *coin,char *coinaddr,struct LP_address_utxo **utxos,int32_t n,uint64_t targetval);
void LP_butxo_set(struct LP_utxoinfo *butxo,int32_t iambob,struct iguana_info *coin,struct LP_address_utxo *up,struct LP_address_utxo *up2,int64_t satoshis);
struct LP_gtcorder
{
    struct LP_gtcorder *next,*prev;
    struct LP_quoteinfo Q;
    uint32_t cancelled,pending;
} *GTCorders;
struct basilisk_request *LP_requestinit(struct basilisk_request *rp,bits256 srchash,bits256 desthash,char *src,uint64_t srcsatoshis,char *dest,uint64_t destsatoshis,uint32_t timestamp,uint32_t quotetime,int32_t DEXselector,int32_t fillflag,int32_t gtcflag);
void LP_tradecommand_log(cJSON *argjson);
extern uint32_t LP_RTcount,LP_swapscount;
int32_t bits256_cmp(bits256 a,bits256 b);
char *bits256_str(char hexstr[65],bits256 x);
int32_t LP_quotecmp(int32_t strictflag,struct LP_quoteinfo *qp,struct LP_quoteinfo *qp2);
int64_t LP_instantdex_proofcheck(char *symbol,char *coinaddr,cJSON *proof,int32_t num);
double LP_myprice(int32_t iambob,double *bidp,double *askp,char *base,char *rel);
double LP_pricecache(struct LP_quoteinfo *qp,char *base,char *rel,bits256 txid,int32_t vout);
int32_t LP_pricevalid(double price);

extern struct LP_quoteinfo LP_Alicereserved;
char *LP_quotereceived(struct LP_quoteinfo *qp);
double LP_trades_alicevalidate(struct LP_quoteinfo *qp);
void LP_abutxo_set(struct LP_utxoinfo *autxo,struct LP_utxoinfo *butxo,struct LP_quoteinfo *qp);
double LP_quote_validate(struct LP_utxoinfo *autxo,struct LP_utxoinfo *butxo,struct LP_quoteinfo *qp,int32_t iambob);
void LP_otheraddress(char *destcoin,char *otheraddr,char *srccoin,char *coinaddr);
void LP_swapsfp_update(uint32_t requestid,uint32_t quoteid);
void LP_unavailableset(bits256 txid,int32_t vout,uint32_t expiration,bits256 otherpub);
double LP_trades_pricevalidate(struct LP_quoteinfo *qp,struct iguana_info *coin,double price);
uint32_t LP_allocated(bits256 txid,int32_t vout);
double LP_trades_bobprice(double *bidp,double *askp,struct LP_quoteinfo *qp);
int32_t LP_RTmetrics_blacklisted(bits256 pubkey);
int32_t LP_reservation_check(bits256 txid,int32_t vout,bits256 pubkey);
int32_t LP_nanobind(void *ctx,char *pairstr);
extern uint32_t LP_swap_critical;
extern uint32_t LP_swap_endcritical;
char *LP_postprice_recv(cJSON *argjson);
char *LP_price_sig(uint32_t timestamp,bits256 priv,uint8_t *pubsecp,bits256 pubkey,char *base,char *rel,uint64_t price64);
#endif

// ---

// Things we presently share with the C code from Rust.
// cf. "for_c.rs"

void log_stacktrace (char const* desc);
struct iguana_info* LP_coinadd (struct iguana_info* ii);
struct iguana_info* LP_coinsearch (char const* ticker);
void LP_get_coin_pointers (struct iguana_info** coins_buf, int32_t coins_size);
uint8_t is_loopback_ip (char *ip);
void broadcast_p2p_msg_for_c(bits256 pubkey,char *msg, uint32_t ctx_h);
