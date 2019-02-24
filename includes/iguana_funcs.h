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

#ifndef H_IGUANAFUNCS_H
#define H_IGUANAFUNCS_H
#define SIGHASH_ALL 1

cJSON *SuperNET_helpjson();

// peers
int32_t iguana_verifypeer(struct iguana_info *coin,void *key,void *value,int32_t itemind,int32_t itemsize);
int32_t iguana_peermetrics(struct supernet_info *myinfo,struct iguana_info *coin);
void iguana_peersloop(void *arg);
int32_t iguana_queue_send(struct iguana_peer *addr,int32_t delay,uint8_t *serialized,char *cmd,int32_t len);
uint32_t iguana_rwiAddrind(struct iguana_info *coin,int32_t rwflag,struct iguana_iAddr *iA,uint32_t ind);
void iguana_connections(void *arg);
uint32_t iguana_possible_peer(struct iguana_info *coin,char *ip_port);
//int32_t iguana_set_iAddrheight(struct iguana_info *coin,uint32_t ipbits,int32_t height);
//struct iguana_peer *iguana_choosepeer(struct iguana_info *coin);
void iguana_initpeer(struct iguana_info *coin,struct iguana_peer *addr,uint64_t ipbits);
void iguana_startconnection(void *arg);
void iguana_shutdownpeers(struct iguana_info *coin,int32_t forceflag);
void iguana_acceptloop(void *args);
void iguana_recvloop(void *args);
int32_t iguana_send(struct iguana_info *coin,struct iguana_peer *addr,uint8_t *serialized,int32_t len);
uint32_t iguana_updatemetrics(struct supernet_info *myinfo,struct iguana_info *coin);
void *iguana_peeralloc(struct iguana_info *coin,struct iguana_peer *addr,int32_t datalen);
int64_t iguana_peerfree(struct iguana_info *coin,struct iguana_peer *addr,void *ptr,int32_t datalen);
int64_t iguana_peerallocated(struct iguana_info *coin,struct iguana_peer *addr);

// serdes
int32_t iguana_rwmem(int32_t rwflag,uint8_t *serialized,int32_t len,void *endianedp);
int32_t iguana_rwnum(int32_t rwflag,uint8_t *serialized,int32_t len,void *endianedp);
int32_t iguana_rwvarint32(int32_t rwflag,uint8_t *serialized,uint32_t *int32p);
int32_t iguana_rwbignum(int32_t rwflag,uint8_t *serialized,int32_t len,uint8_t *endianedp);
int32_t iguana_rwblock(struct supernet_info *myinfo,char *symbol,uint8_t zcash,uint8_t auxpow,int32_t (*hashalgo)(uint8_t *blockhashp,uint8_t *serialized,int32_t len),int32_t rwflag,bits256 *hash2p,uint8_t *serialized,struct iguana_msgzblock *zmsg,int32_t maxlen);
int32_t iguana_serialize_block(struct supernet_info *myinfo,struct iguana_chain *chain,bits256 *hash2p,uint8_t serialized[sizeof(struct iguana_msgblock)],struct iguana_block *block);
int32_t iguana_blockconv(uint8_t zcash,uint8_t auxpow,struct iguana_zblock *zdest,struct iguana_msgzblock *zmsg,bits256 hash2,int32_t height);
int32_t iguana_msgparser(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,struct OS_memspace *rawmem,struct OS_memspace *txmem,struct OS_memspace *hashmem,struct iguana_msghdr *H,uint8_t *data,int32_t datalen,int32_t fromcache);

// send message
int32_t iguana_validatehdr(char *symbol,struct iguana_msghdr *H);
int32_t iguana_sethdr(struct iguana_msghdr *H,const uint8_t netmagic[4],char *command,uint8_t *data,int32_t datalen);
int32_t iguana_send_version(struct iguana_info *coin,struct iguana_peer *addr,uint64_t myservices);
int32_t iguana_gentxarray(struct supernet_info *myinfo,struct iguana_info *coin,struct OS_memspace *mem,struct iguana_txblock *txblock,int32_t *lenp,uint8_t *data,int32_t datalen);
int32_t iguana_gethdrs(struct iguana_info *coin,uint8_t *serialized,char *cmd,char *hashstr);
int32_t iguana_getdata(struct iguana_info *coin,uint8_t *serialized,int32_t type,bits256 *hashes,int32_t n);
void iguana_blockunconv(uint8_t zcash,uint8_t auxpow,struct iguana_msgzblock *zmsg,struct iguana_zblock *zsrc,int32_t cleartxn_count);
int32_t iguana_peerblockrequest(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *blockspace,int32_t max,struct iguana_peer *addr,bits256 hash2,int32_t validatesigs);
int32_t iguana_validatesigs(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *serialized,int32_t datalen);

// ramchain
int64_t iguana_verifyaccount(struct iguana_info *coin,struct iguana_account *acct,uint32_t pkind);
int32_t iguana_initramchain(struct iguana_info *coin,int32_t initialheight,int32_t mapflags,int32_t fullverify);
void iguana_syncramchain(struct iguana_info *coin);
//int32_t iguana_validateramchain(struct iguana_info *coin,int64_t *netp,uint64_t *creditsp,uint64_t *debitsp,int32_t height,struct iguana_block *block,int32_t hwmheight,struct iguana_prevdep *lp);
int32_t iguana_calcrmd160(struct iguana_info *coin,char *asmstr,struct vin_info *vp,uint8_t *pk_script,int32_t pk_scriptlen,bits256 debugtxid,int32_t vout,uint32_t sequence);
uint32_t iguana_updatescript(struct iguana_info *coin,uint32_t blocknum,uint32_t txidind,uint32_t spendind,uint32_t unspentind,uint64_t value,uint8_t *script,int32_t scriptlen,uint32_t sequence);
void iguana_gotblockM(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,struct iguana_txblock *origtxdata,struct iguana_msgtx *txarray,struct iguana_msghdr *H,uint8_t *data,int32_t recvlen,int32_t fromcache,uint8_t zcash);
int32_t iguana_parseblock(struct iguana_info *coin,struct iguana_block *block,struct iguana_msgtx *tx,int32_t numtx);
uint32_t iguana_txidind(struct iguana_info *coin,uint32_t *firstvoutp,uint32_t *firstvinp,bits256 txid);
bits256 iguana_txidstr(struct iguana_info *coin,uint32_t *firstvoutp,uint32_t *firstvinp,char *txidstr,uint32_t txidind);
int32_t iguana_updateramchain(struct iguana_info *coin);
//void iguana_emittxarray(struct iguana_info *coin,FILE *fp,struct iguana_bundle *bundle,struct iguana_block *block,struct iguana_msgtx *txarray,int32_t numtx);

// blockchain
int32_t iguana_needhdrs(struct iguana_info *coin);
struct iguana_chain *iguana_chainfind(struct supernet_info *myinfo,char *name,cJSON *argjson,int32_t createflag);
int32_t iguana_chainextend(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_block *newblock);
uint64_t iguana_miningreward(struct iguana_info *coin,uint32_t blocknum);

// tx
int32_t iguana_rwtx(struct supernet_info *myinfo,uint8_t zcash,int32_t rwflag,struct iguana_info *coin,struct OS_memspace *mem,uint8_t *serialized,struct iguana_msgtx *msg,int32_t maxsize,bits256 *txidp,int32_t hastimestamp,int32_t isvpncoin);
void iguana_gottxidsM(struct iguana_info *coin,struct iguana_peer *addr,bits256 *txids,int32_t n);
void iguana_gotquotesM(struct iguana_info *coin,struct iguana_peer *addr,bits256 *quotes,int32_t n);
void iguana_gotunconfirmedM(struct iguana_info *coin,struct iguana_peer *addr,struct iguana_msgtx *tx,uint8_t *data,int32_t datalen);
void iguana_gotblockhashesM(struct iguana_info *coin,struct iguana_peer *addr,bits256 *blockhashes,int32_t n);

// blocks
bits256 iguana_blockhash(struct iguana_info *coin,int32_t height);
#define iguana_blockfind(str,coin,hash2) iguana_blockhashset(str,coin,-1,hash2,0)
struct iguana_block *iguana_blockhashset(char *debugstr,struct iguana_info *coin,int32_t height,bits256 hash2,int32_t createflag);
struct iguana_block *iguana_prevblock(struct iguana_info *coin,struct iguana_block *block,int32_t PoSflag);
uint32_t iguana_targetbits(struct iguana_info *coin,struct iguana_block *hwmchain,struct iguana_block *prev,struct iguana_block *prev2,int32_t PoSflag,int32_t targetspacing,int32_t targettimespan);

uint32_t iguana_syncs(struct iguana_info *coin);
void iguana_gotdata(struct iguana_info *coin,struct iguana_peer *addr,int32_t height);
//int64_t iguana_getbalance(struct iguana_info *coin,uint64_t *creditsp,uint64_t *debitsp,int32_t *nump,uint32_t *unspents,long max,struct iguana_pkhash *P,uint32_t pkind);
int32_t iguana_queueblock(struct iguana_info *coin,int32_t height,bits256 hash2,int32_t priority);
int32_t iguana_updatewaiting(struct iguana_info *coin,int32_t starti,int32_t max);

// recvbits
int32_t iguana_recvinit(struct iguana_info *coin,int32_t initialheight);
int32_t ramcoder_decompress(uint8_t *data,int32_t maxlen,uint8_t *bits,uint32_t numbits,bits256 seed);
int32_t ramcoder_compress(uint8_t *bits,int32_t maxlen,uint8_t *data,int32_t datalen,bits256 seed);
uint64_t hconv_bitlen(uint64_t bitlen);
struct iguana_block *iguana_blockptr(char *debugstr,struct iguana_info *coin,int32_t height);
int32_t iguana_processrecv(struct supernet_info *myinfo,struct iguana_info *coin); // single threaded
//void iguana_recvalloc(struct iguana_info *coin,int32_t numitems);
void iguana_coins(void *arg);
int32_t iguana_savehdrs(struct iguana_info *coin);

// hdrs
struct iguana_bundle *iguana_bundlecreate(struct iguana_info *coin,int32_t *bundleip,int32_t bundleheight,bits256 bundlehash2,bits256 allhash,int32_t issueflag);
struct iguana_block *iguana_updatehdrs(struct iguana_info *coin,int32_t *newhwmp,struct iguana_block *block,bits256 prevhash2,bits256 hash2);
void iguana_parseline(struct supernet_info *myinfo,struct iguana_info *coin,int32_t iter,FILE *fp);
int32_t iguana_gotheadersM(struct iguana_info *coin,struct iguana_peer *addr,struct iguana_zblock *zblocks,int32_t n);
void iguana_emittxdata(struct iguana_info *coin,struct iguana_bundle *bp);
int32_t iguana_pollQsPT(struct iguana_info *coin,struct iguana_peer *addr);
int32_t iguana_avail(struct iguana_info *coin,int32_t height,int32_t n);
int32_t iguana_updatebundles(struct iguana_info *coin);
uint64_t iguana_utxoaddrtablefind(struct iguana_info *coin,int16_t search_hdrsi,uint32_t search_pkind,uint8_t rmd160[20]);
void iguana_bundlestats(struct supernet_info *myinfo,struct iguana_info *coin,char *str,int32_t lag);
void iguana_chaininit(struct supernet_info *myinfo,struct iguana_chain *chain,int32_t hasheaders,cJSON *argjson);
void iguana_coinargs(char *symbol,int64_t *maxrecvcachep,int32_t *minconfirmsp,int32_t *maxpeersp,int32_t *initialheightp,uint64_t *servicesp,int32_t *maxrequestsp,int32_t *maxbundlesp,cJSON *json);
struct iguana_info *iguana_setcoin(char *symbol,void *launched,int32_t maxpeers,int64_t maxrecvcache,uint64_t services,int32_t initialheight,int32_t maphash,int32_t minconfirms,int32_t maxrequests,int32_t maxbundles,cJSON *json,int32_t virtcoin);

// init
struct iguana_info *iguana_coinstart(struct supernet_info *myinfo,struct iguana_info *coin,int32_t initialheight,int32_t mapflags);
void iguana_callcoinstart(struct supernet_info *myinfo,struct iguana_info *coin);
void iguana_initcoin(struct iguana_info *coin,cJSON *argjson);
void iguana_coinloop(void *arg);

// utils
double PoW_from_compact(uint32_t nBits,uint8_t unitval);
void calc_rmd160(char *hexstr,uint8_t buf[20],uint8_t *msg,int32_t len);
void calc_OP_HASH160(char *hexstr,uint8_t hash160[20],char *msg);
double dxblend(double *destp,double val,double decay);
double _xblend(float *destp,double val,double decay);

// json
int32_t iguana_processjsonQ(struct iguana_info *coin); // reentrant, can be called during any idletime
char *iguana_JSON(void *_myinfo,void *_coin,char *jsonstr,uint16_t port);
char *SuperNET_p2p(struct iguana_info *coin,struct iguana_peer *addr,int32_t *delaymillisp,char *ipaddr,uint8_t *data,int32_t datalen,int32_t compressed);

char *mbstr(char *str,double);
int init_hexbytes_noT(char *hexbytes,unsigned char *message,long len);
int32_t decode_hex(unsigned char *bytes,int32_t n,char *hex);
char hexbyte(int32_t c);
char *clonestr(char *str);
long _stripwhite(char *buf,int accept);
int32_t myatoi(char *str,int32_t range);
int32_t safecopy(char *dest,char *src,long len);
void escape_code(char *escaped,char *str);
int32_t is_zeroes(char *str);
int64_t conv_floatstr(char *numstr);
int32_t has_backslash(char *str);

int32_t iguana_numthreads(struct iguana_info *coin,int32_t mask);
void iguana_terminator(void *arg);

int32_t is_hexstr(char *str,int32_t n);
void iguana_emitQ(struct iguana_info *coin,struct iguana_bundle *bp);
void iguana_txdataQ(struct iguana_info *coin,struct iguana_peer *addr,FILE *fp,long fpos,int32_t datalen);
void iguana_helper(void *arg);

struct iguana_helper { struct queueitem DL; void *coin,*addr,*bp,*nextbp,*fp; long fpos; int32_t allocsize,type,hdrsi,bundlei,datalen,timelimit; uint32_t starttime; };
int32_t iguana_helpertask(FILE *fp,struct OS_memspace *mem,struct OS_memspace *memB,struct iguana_helper *ptr);
void iguana_flushQ(struct iguana_info *coin,struct iguana_peer *addr);
//struct iguana_txdatabits iguana_peerfilePT(struct iguana_info *coin,struct iguana_peer *addr,bits256 hash2,struct iguana_txdatabits txdatabits,int32_t recvlen);
struct iguana_txdatabits iguana_calctxidbits(uint32_t addrind,uint32_t filecount,uint32_t fpos,uint32_t datalen);
int32_t iguana_bundlesaveHT(struct supernet_info *myinfo,struct iguana_info *coin,struct OS_memspace *mem,struct OS_memspace *memB,struct iguana_bundle *bp,uint32_t starttime); // helper thread
int32_t iguana_bundlemergeHT(struct supernet_info *myinfo,char *fname,struct iguana_info *coin,struct OS_memspace *mem,struct OS_memspace *memB,struct iguana_bundle *bp,struct iguana_bundle *nextbp,uint32_t starttime); // helper thread

void iguana_peerfilename(struct iguana_info *coin,char *fname,uint32_t addrind,uint32_t filecount);

struct iguana_txblock *iguana_ramchainptrs(struct iguana_txid **Tptrp,struct iguana_unspent20 **Uptrp,struct iguana_spend256 **Sptrp,struct iguana_pkhash **Pptrp,bits256 **externalTptrp,struct OS_memspace *mem,struct iguana_txblock *origtxdata);

int32_t iguana_ramchainsave(struct iguana_info *coin,struct iguana_ramchain *ramchain);
int32_t iguana_ramchainfree(struct iguana_info *coin,struct OS_memspace *mem,struct iguana_ramchain *ramchain);
struct iguana_ramchain *iguana_ramchainmergeHT(struct iguana_info *coin,struct OS_memspace *mem,struct iguana_ramchain *ramchains[],int32_t n,struct iguana_bundle *bp);
void iguana_ramchainmerge(struct iguana_info *coin);

int32_t iguana_blockQ(char *argstr,struct iguana_info *coin,struct iguana_bundle *bp,int32_t bundlei,bits256 hash2,int32_t priority);
void iguana_blockcopy(uint8_t zcash,uint8_t auxpow,struct iguana_info *coin,struct iguana_block *block,struct iguana_block *origblock);
int32_t iguana_rpctest(struct iguana_info *coin);
extern const char *Hardcoded_coins[][3];
void iguana_main(void *arg);
void iguana_exit(struct supernet_info *myinfo,struct iguana_bundle *bp);

int32_t iguana_peerfname(struct iguana_info *coin,int32_t *hdrsip,char *dirname,char *fname,uint32_t ipbits,bits256 hash2,bits256 prevhash2,int32_t numblocks,int32_t dispflag);
struct iguana_txblock *iguana_peertxdata(struct iguana_info *coin,int32_t *bundleip,char *fname,struct OS_memspace *mem,uint32_t ipbits,bits256 hash2);
int32_t iguana_peerfile_exists(struct iguana_info *coin,struct iguana_peer *addr,char *dirname,char *fname,bits256 hash2,bits256 prevhash2,int32_t numblocks);
struct iguana_ramchain *iguana_ramchainset(struct iguana_info *coin,struct iguana_ramchain *ramchain,struct iguana_txblock *txdata);
void *iguana_iAddriterator(struct iguana_info *coin,struct iguana_iAddr *iA,struct iguana_peer *addr);
long iguana_ramchain_data(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,struct iguana_txblock *origtxdata,struct iguana_msgtx *txarray,int32_t txn_count,uint8_t *data,int32_t recvlen,struct iguana_bundle *bp,struct iguana_block *block);
int32_t iguana_bundlehash2add(struct iguana_info *coin,struct iguana_block **blockp,struct iguana_bundle *bp,int32_t bundlei,bits256 hash2);
struct iguana_block *iguana_bundleblockadd(struct iguana_info *coin,struct iguana_bundle **bpp,int32_t *bundleip,struct iguana_block *origblock);
int32_t iguana_chainextend(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_block *newblock);
int32_t iguana_blockvalidate(struct supernet_info *myinfo,struct iguana_info *coin,int32_t *validp,struct iguana_block *block,int32_t dispflag);
char *iguana_bundledisp(struct iguana_info *coin,struct iguana_bundle *prevbp,struct iguana_bundle *bp,struct iguana_bundle *nextbp,int32_t m);
struct iguana_bundle *iguana_bundlefind(struct iguana_info *coin,struct iguana_bundle **bpp,int32_t *bundleip,bits256 hash2);
//int32_t iguana_chainheight(struct iguana_info *coin,struct iguana_block *origblock);
bits256 *iguana_blockhashptr(struct iguana_info *coin,int32_t height);
int32_t iguana_hash2set(struct iguana_info *coin,char *debugstr,struct iguana_bundle *bp,int32_t bundlei,bits256 newhash2);
struct iguana_block *_iguana_chainlink(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_block *newblock);
int32_t iguana_hashfree(struct iguana_kvitem *hashtable,int32_t freeitem);
int32_t iguana_processbundlesQ(struct iguana_info *coin,int32_t *newhwmp); // single threaded
int32_t iguana_ramchainverifyPT(struct iguana_info *coin,struct iguana_ramchain *ramchain);
void *map_file(char *fname,long *filesizep,int32_t enablewrite);
void iguana_rpcloop(void *args);
int32_t iguana_socket(int32_t bindflag,char *hostname,uint16_t port);
void iguana_mergeQ(struct iguana_info *coin,struct iguana_bundle *bp,struct iguana_bundle *nextbp);

#define bits256_nonz(a) (((a).ulongs[0] | (a).ulongs[1] | (a).ulongs[2] | (a).ulongs[3]) != 0)
//int32_t btc_addr2univ(uint8_t *addrtypep,uint8_t rmd160[20],char *coinaddr);

struct iguana_agent
{
    char name[32],hostname[64]; void *methods; uint16_t port; int32_t sock,nummethods;
    bits256 pubkey,privkey;
    char *(*parsefunc)(struct iguana_agent *agent,char *method,void *json,char *remoteaddr);
};

int32_t iguana_txbytes(struct iguana_info *coin,uint8_t *serialized,int32_t maxlen,bits256 *txidp,struct iguana_txid *tx,int32_t height,struct iguana_msgvin *vins,struct iguana_msgvout *vouts);
int32_t iguana_vinset(struct iguana_info *coin,uint8_t *scriptspace,int32_t height,struct iguana_msgvin *vin,struct iguana_txid *tx,int32_t i);
int32_t iguana_voutset(struct iguana_info *coin,uint8_t *scriptspace,char *asmstr,int32_t height,struct iguana_msgvout *vout,struct iguana_txid *tx,int32_t i);
//int32_t btc_convrmd160(char *coinaddr,uint8_t addrtype,uint8_t rmd160[20]);
struct iguana_txid *iguana_bundletx(struct iguana_info *coin,struct iguana_bundle *bp,int32_t bundlei,struct iguana_txid *tx,int32_t txidind);
int32_t iguana_txidreq(struct iguana_info *coin,char **retstrp,bits256 txid);
void iguana_bundleiclear(struct iguana_info *coin,struct iguana_bundle *bp,int32_t bundlei);
int32_t hcalc_bitsize(uint64_t x);
struct iguana_txid *iguana_txidfind(struct iguana_info *coin,int32_t *heightp,struct iguana_txid *tx,bits256 txid,int32_t lasthdrsi);
int32_t iguana_scriptgen(struct iguana_info *coin,int32_t *Mp,int32_t *nump,char *coinaddr,uint8_t *script,char *asmstr,uint8_t rmd160[20],uint8_t type,const struct vin_info *vp,int32_t txi);
int32_t iguana_ramchain_spendtxid(struct iguana_info *coin,uint32_t *unspentindp,bits256 *txidp,struct iguana_txid *T,int32_t numtxids,bits256 *X,int32_t numexternaltxids,struct iguana_spend *s);
void iguana_dedicatedloop(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr);
struct iguana_peer *iguana_peerslot(struct iguana_info *coin,uint64_t ipbits,int32_t forceflag);
void iguana_dedicatedglue(void *arg);
void SuperNET_remotepeer(struct supernet_info *myinfo,struct iguana_info *coin,char *symbol,char *ipaddr,int32_t supernetflag);
void iguana_peer_meminit(struct iguana_info *coin,struct iguana_peer *addr);
void SuperNET_yourip(struct supernet_info *myinfo,char *yourip);
void iguana_peerkill(struct iguana_info *coin);
int32_t blockhash_sha256(uint8_t *blockhashp,uint8_t *serialized,int32_t len);
void iguana_nameset(char name[64],char *symbol,cJSON *json);
cJSON *iguana_getinfo(struct supernet_info *myinfo,struct iguana_info *coin);
void *basilisk_getinfo(struct basilisk_item *Lptr,struct supernet_info *myinfo,struct iguana_info *coin,char *remoteaddr,uint32_t basilisktag,int32_t timeoutmillis,cJSON *valsobj);
void basilisk_unspents_process(struct supernet_info *myinfo,struct iguana_info *coin,char *retstr);
char *busdata_sync(uint32_t *noncep,char *jsonstr,char *broadcastmode,char *destNXTaddr);
void peggy();
int32_t opreturns_init(uint32_t blocknum,uint32_t blocktimestamp,char *path);
struct iguana_info *iguana_coinfind(char *symbol);
struct iguana_info *iguana_coinadd(char *symbol,char *nane,cJSON *json,int32_t virtcoin);
struct iguana_ramchain *iguana_bundleload(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_ramchain *ramchain,struct iguana_bundle *bp,int32_t extraflag);
int32_t iguana_sendblockreq(struct iguana_info *coin,struct iguana_peer *addr,struct iguana_bundle *bp,int32_t bundlei,bits256 hash2,int32_t iamthreadsafe);
int32_t iguana_send_supernet(struct iguana_peer *addr,char *jsonstr,int32_t delay);

struct iguana_waccount *iguana_waccountfind(struct supernet_info *myinfo,char *account);
struct iguana_waddress *iguana_waccountadd(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_waccount **wacctp,char *walletaccount,char *coinaddr);
struct iguana_waddress *iguana_waccountswitch(struct supernet_info *myinfo,struct iguana_info *coin,char *account,char *coinaddr,char *redeemScript);
struct iguana_waddress *iguana_waddresscalc(struct supernet_info *myinfo,uint8_t pubval,uint8_t wiftype,struct iguana_waddress *addr,bits256 privkey);
struct iguana_waddress *iguana_waddressfind(struct supernet_info *myinfo,struct iguana_waccount *wacct,char *coinaddr);
void dex_init(struct supernet_info *myinfo);
char *iguana_coinjson(struct iguana_info *coin,char *method,cJSON *json);
cJSON *iguana_peersjson(struct iguana_info *coin,int32_t addronly);
//int32_t btc_priv2wif(char *wifstr,uint8_t privkey[32],uint8_t addrtype);
//int32_t btc_pub2rmd(uint8_t rmd160[20],uint8_t pubkey[33]);
int32_t iguana_launchcoin(struct supernet_info *myinfo,char *symbol,cJSON *json,int32_t virtcoin);
int32_t iguana_bundleinitmap(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,int32_t height,bits256 hash2,bits256 hash1);
int32_t iguana_jsonQ(struct supernet_info *myinfo,struct iguana_info *coin);
int32_t is_bitcoinrpc(struct supernet_info *myinfo,char *method,char *remoteaddr);
char *iguana_bitcoinRPC(struct supernet_info *myinfo,char *method,cJSON *json,char *remoteaddr,uint16_t port);
cJSON *iguana_pubkeyjson(struct iguana_info *coin,char *pubkeystr);
void iguana_bundleQ(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,int32_t timelimit);
int32_t iguana_bundleiters(struct supernet_info *myinfo,struct iguana_info *coin,struct OS_memspace *mem,struct OS_memspace *memB,struct iguana_bundle *bp,int32_t timelimit,int32_t lag);
void ramcoder_test(void *data,int64_t len);
void iguana_exit(struct supernet_info *myinfo,struct iguana_bundle *bp);
int32_t iguana_pendingaccept(struct iguana_info *coin);
char *iguana_blockingjsonstr(struct supernet_info *myinfo,struct iguana_info *coin,char *jsonstr,uint64_t tag,int32_t maxmillis,char *remoteaddr,uint16_t port);
void iguana_iAkill(struct iguana_info *coin,struct iguana_peer *addr,int32_t markflag);
cJSON *SuperNET_bits2json(uint8_t *serialized,int32_t datalen);
int32_t SuperNET_sendmsg(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,bits256 destpub,bits256 mypriv,bits256 mypub,uint8_t *msg,int32_t len,uint8_t *data,int32_t delaymillis);
int32_t category_peer(struct supernet_info *myinfo,struct iguana_peer *addr,bits256 category,bits256 subhash);
int32_t bitcoin_wif2priv(uint8_t *addrtypep,bits256 *privkeyp,char *wifstr);
int32_t bitcoin_priv2wif(char *wifstr,bits256 privkey,uint8_t addrtype);
int32_t bitcoin_priv2wiflong(char *wifstr,bits256 privkey,uint8_t addrtype);
bits256 iguana_chaingenesis(struct supernet_info *myinfo,char *symbol,uint8_t zcash,uint8_t auxpow,int32_t (*hashalgo)(uint8_t *blockhashp,uint8_t *serialized,int32_t len),bits256 genesishash,char *genesisblock,char *hashalgostr,int32_t version,uint32_t timestamp,uint32_t bits,uint32_t nonce,bits256 merkle_root);
int32_t iguana_send_ConnectTo(struct iguana_info *coin,struct iguana_peer *addr);
cJSON *iguana_txjson(struct iguana_info *coin,struct iguana_txid *tx,int32_t height,struct vin_info *V);
char *iguana_txscan(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *json,uint8_t *data,int32_t recvlen,bits256 txid);
char *iguana_rawtxbytes(struct iguana_info *coin,int32_t height,cJSON *json,struct iguana_msgtx *msgtx,int32_t suppress_pubkeys);
int32_t iguana_send_VPNversion(struct iguana_info *coin,struct iguana_peer *addr,uint64_t myservices);
void exchanges777_init(struct supernet_info *myinfo,cJSON *exchanges,int32_t sleepflag);
int32_t iguana_rwvout(int32_t rwflag,struct OS_memspace *mem,uint8_t *serialized,struct iguana_msgvout *msg);
int32_t iguana_rwvin(int32_t rwflag,struct iguana_info *coin,struct OS_memspace *mem,uint8_t *serialized,struct iguana_msgvin *msg,int32_t vini);
int32_t iguana_rwmsgtx(struct iguana_info *coin,int32_t height,int32_t rwflag,cJSON *json,uint8_t *serialized,int32_t maxsize,struct iguana_msgtx *msg,bits256 *txidp,char *vpnstr,uint8_t *extraspace,int32_t extralen,cJSON *vins,int32_t suppress_pubkeys);
int32_t iguana_ramtxbytes(struct iguana_info *coin,uint8_t *serialized,int32_t maxlen,bits256 *txidp,struct iguana_txid *tx,int32_t height,struct iguana_msgvin *vins,struct iguana_msgvout *vouts,int32_t validatesigs);
cJSON *bitcoin_txtest(struct iguana_info *coin,char *rawtxstr,bits256 txid);
cJSON *iguana_blockjson(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_block *block,int32_t txidsflag);
int32_t iguana_rwblockhdr(int32_t rwflag,uint8_t zcash,uint8_t *serialized,struct iguana_msgzblock *zmsg);
//int32_t iguana_sig(uint8_t *sig,int32_t maxsize,uint8_t *data,int32_t datalen,bits256 privkey);
//int32_t iguana_ver(uint8_t *sig,int32_t siglen,uint8_t *data,int32_t datalen,bits256 pubkey);
//int32_t iguana_ver(uint8_t *sig,int32_t siglen,uint8_t *data,int32_t datalen,uint8_t *pubkey);
void calc_rmd160_sha256(uint8_t rmd160[20],uint8_t *data,int32_t datalen);
int32_t bitcoin_checklocktimeverify(uint8_t *script,int32_t n,uint32_t locktime);
struct bitcoin_spend *iguana_spendset(struct supernet_info *myinfo,struct iguana_info *coin,int64_t amount,int64_t txfee,cJSON *addresses,int32_t minconf);
cJSON *bitcoin_hex2json(struct iguana_info *coin,int32_t height,bits256 *txidp,struct iguana_msgtx *msgtx,char *txbytes,uint8_t *extrapace,int32_t extralen,uint8_t *serialized,cJSON *vins,int32_t suppress_pubkeys);
cJSON *iguana_signtx(struct supernet_info *myinfo,struct iguana_info *coin,bits256 *txidp,char **signedtxp,struct bitcoin_spend *spend,cJSON *txobj,cJSON *vins);
void iguana_addscript(struct iguana_info *coin,cJSON *dest,uint8_t *script,int32_t scriptlen,char *fieldname);
bits256 iguana_genesis(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_chain *chain);

cJSON *bitcoin_txcreate(char *symbol,int32_t isPoS,int64_t locktime,uint32_t txversion,uint32_t timestamp);
cJSON *bitcoin_txoutput(cJSON *txobj,uint8_t *paymentscript,int32_t len,uint64_t satoshis);
cJSON *bitcoin_txinput(struct iguana_info *coin,cJSON *txobj,bits256 txid,int32_t vout,uint32_t sequenceid,uint8_t *spendscript,int32_t spendlen,uint8_t *redeemscript,int32_t p2shlen,uint8_t *pubkeys[],int32_t numpubkeys,uint8_t *sigscript,int32_t siglen);

int32_t bitcoin_changescript(struct iguana_info *coin,uint8_t *changescript,int32_t n,uint64_t *changep,char *changeaddr,uint64_t inputsatoshis,uint64_t satoshis,uint64_t txfee);
//cJSON *bitcoin_addinput(struct iguana_info *coin,cJSON *txobj,bits256 txid,int32_t vout,uint32_t sequenceid,uint8_t *spendscript,int32_t spendlen,uint8_t *redeemscript,int32_t p2shlen,uint8_t *pubkeys[],int32_t numpubkeys);
int32_t bitcoin_verifytx(struct iguana_info *coin,bits256 *signedtxidp,char **signedtx,char *rawtxstr,struct vin_info *V,int32_t numinputs);
int32_t bitcoin_verify(void *ctx,uint8_t *sig,int32_t siglen,bits256 txhash2,uint8_t *pubkey,int32_t plen);
cJSON *SuperNET_decryptedjson(char *destfname,char *passphrase,int32_t passsize,bits256 wallethash,char *fname2fa,int32_t fnamesize,bits256 wallet2priv);
int32_t bitcoin_txaddspend(struct iguana_info *coin,cJSON *txobj,char *destaddress,uint64_t satoshis);
char *_setVsigner(struct iguana_info *coin,struct vin_info *V,int32_t ind,char *pubstr,char *wifstr);

char *bitcoin_json2hex(struct supernet_info *myinfo,struct iguana_info *coin,bits256 *txidp,cJSON *txjson,struct vin_info *V);
int32_t bitcoin_addr2rmd160(uint8_t *addrtypep,uint8_t rmd160[20],char *coinaddr);
char *issue_startForging(struct supernet_info *myinfo,char *secret);
struct bitcoin_unspent *iguana_unspentsget(struct supernet_info *myinfo,struct iguana_info *coin,char **retstrp,double *balancep,int32_t *numunspentsp,double minconfirms,char *address);
void iguana_chainparms(struct supernet_info *myinfo,struct iguana_chain *chain,cJSON *argjson);
int32_t iguana_RTpkhasharray(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *array,int32_t minconf,int32_t maxconf,uint64_t *totalp,struct iguana_pkhash *P,int32_t max,uint8_t rmd160[20],char *coinaddr,uint8_t *pubkey33,int32_t lastheight,struct iguana_outpoint *unspents,int32_t *numunspentsp,int32_t maxunspents,char *remoteaddr,int32_t includespent);
long iguana_spentsfile(struct iguana_info *coin,int32_t n);
uint8_t *iguana_rmdarray(struct supernet_info *myinfo,struct iguana_info *coin,int32_t *numrmdsp,cJSON *array,int32_t firsti);
uint64_t iguana_RTunspents(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *array,int32_t minconf,int32_t maxconf,uint8_t *rmdarray,int32_t numrmds,int32_t lastheight,struct iguana_outpoint *unspents,int32_t *numunspentsp,char *remoteaddr,int32_t includespent);
uint8_t *iguana_walletrmds(struct supernet_info *myinfo,struct iguana_info *coin,int32_t *numrmdsp);
void iguana_hhutxo_purge(struct iguana_info *coin);
char *iguana_bundleaddrs(struct iguana_info *coin,int32_t hdrsi);
uint32_t iguana_sparseaddpk(uint8_t *bits,int32_t width,uint32_t tablesize,uint8_t rmd160[20],struct iguana_pkhash *P,uint32_t pkind,struct iguana_ramchain *ramchain);
int32_t iguana_vinscriptparse(struct iguana_info *coin,struct vin_info *vp,uint32_t *sigsizep,uint32_t *pubkeysizep,uint32_t *p2shsizep,uint32_t *suffixp,uint8_t *vinscript,int32_t scriptlen);
void iguana_parsebuf(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,struct iguana_msghdr *H,uint8_t *buf,int32_t len,int32_t fromcache);
int32_t _iguana_calcrmd160(struct iguana_info *coin,struct vin_info *vp);
int32_t iguana_spendvectors(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,struct iguana_ramchain *ramchain,int32_t starti,int32_t numblocks,int32_t convertflag,int32_t iterate);
int32_t iguana_balancegen(struct iguana_info *coin,int32_t incremental,struct iguana_bundle *bp,int32_t startheight,int32_t endheight,int32_t startemit);
struct iguana_utxoaddr *iguana_utxoaddrfind(int32_t createflag,struct iguana_info *coin,int16_t hdrsi,uint32_t pkind,uint8_t rmd160[20],struct iguana_utxoaddr **prevp,int32_t p2shflag);
int32_t iguana_bundlevalidate(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,int32_t forceflag);
void iguana_validateQ(struct iguana_info *coin,struct iguana_bundle *bp);
struct iguana_bloominds iguana_calcbloom(bits256 hash2);
int32_t iguana_bloomfind(struct iguana_info *coin,struct iguana_bloom16 *bloom,int32_t incr,struct iguana_bloominds bit);
struct iguana_bloominds iguana_bloomset(struct iguana_info *coin,struct iguana_bloom16 *bloom,int32_t incr,struct iguana_bloominds bit);
int32_t iguana_Xspendmap(struct iguana_info *coin,struct iguana_ramchain *ramchain,struct iguana_bundle *bp);
//void iguana_balancesQ(struct iguana_info *coin,struct iguana_bundle *bp);
int32_t iguana_balanceflush(struct supernet_info *myinfo,struct iguana_info *coin,int32_t refhdrsi);
int32_t iguana_bundleissue(struct iguana_info *coin,struct iguana_bundle *bp,int32_t starti,int32_t max);
int32_t iguana_balancecalc(struct iguana_info *coin,struct iguana_bundle *bp,int32_t startheight,int32_t endheight);
int32_t iguana_sendblockreqPT(struct iguana_info *coin,struct iguana_peer *addr,struct iguana_bundle *bp,int32_t bundlei,bits256 hash2,int32_t iamthreadsafe);
int32_t iguana_blockreq(struct iguana_info *coin,int32_t height,int32_t priority);
int64_t iguana_bundlecalcs(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,int32_t lag);
int32_t iguana_ramchain_prefetch(struct iguana_info *coin,struct iguana_ramchain *ramchain,int32_t txonly);
int32_t iguana_realtime_update(struct supernet_info *myinfo,struct iguana_info *coin);
int32_t iguana_volatilesmap(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_ramchain *ramchain);
void iguana_volatilespurge(struct iguana_info *coin,struct iguana_ramchain *ramchain);
int32_t iguana_volatilesinit(struct supernet_info *myinfo,struct iguana_info *coin);
void iguana_initfinal(struct supernet_info *myinfo,struct iguana_info *coin,bits256 lastbundle);
int64_t iguana_ramchainopen(struct supernet_info *myinfo,char *fname,struct iguana_info *coin,struct iguana_ramchain *ramchain,struct OS_memspace *mem,struct OS_memspace *hashmem,int32_t bundleheight,bits256 hash2);
int32_t iguana_ramchain_free(struct iguana_info *coin,struct iguana_ramchain *ramchain,int32_t deleteflag);
void iguana_blocksetcounters(struct iguana_info *coin,struct iguana_block *block,struct iguana_ramchain * ramchain);
int32_t iguana_ramchain_iterate(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_ramchain *dest,struct iguana_ramchain *ramchain,struct iguana_bundle *bp,int16_t bundlei);
void *iguana_bundlefile(struct iguana_info *coin,char *fname,long *filesizep,struct iguana_bundle *bp,int32_t bundlei,int32_t renameflag);
int32_t iguana_mapchaininit(char *fname,struct iguana_info *coin,struct iguana_ramchain *mapchain,struct iguana_bundle *bp,int32_t bundlei,struct iguana_block *block,void *ptr,long filesize);
void iguana_RTdataset_free(struct iguana_info *coin);
void iguana_autoextend(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp);
//void iguana_RTramchainfree(struct iguana_info *coin,struct iguana_bundle *bp);
void iguana_coinpurge(struct iguana_info *coin);
void tradebot_liquidity_command(struct supernet_info *myinfo,char *targetcoin,bits256 hash,cJSON *vals);
int32_t iguana_setmaxbundles(struct iguana_info *coin);
void iguana_bundlepurgefiles(struct iguana_info *coin,struct iguana_bundle *bp);
uint32_t iguana_sparseaddtx(uint8_t *bits,int32_t width,uint32_t tablesize,bits256 txid,struct iguana_txid *T,uint32_t txidind,struct iguana_ramchain *ramchain);
void iguana_launchpeer(struct iguana_info *coin,char *ipaddr,int32_t forceflag);
//void iguana_spendvectorsQ(struct iguana_info *coin,struct iguana_bundle *bp);
int8_t iguana_blockstatus(struct iguana_info *coin,struct iguana_block *block);
int32_t iguana_peerslotinit(struct iguana_info *coin,struct iguana_peer *addr,int32_t slotid,uint64_t ipbits);
void iguana_blockunmark(struct iguana_info *coin,struct iguana_block *block,struct iguana_bundle *bp,int32_t i,int32_t deletefile);
int32_t iguana_reqblocks(struct supernet_info *myinfo,struct iguana_info *coin);
void iguana_walletlock(struct supernet_info *myinfo,struct iguana_info *coin);
int32_t _SuperNET_encryptjson(struct supernet_info *myinfo,char *destfname,char *passphrase,int32_t passsize,char *fname2fa,int32_t fnamesize,cJSON *argjson);
int32_t bitcoin_pubkeylen(const uint8_t *pubkey);
struct iguana_block *iguana_bundleblock(struct iguana_info *coin,bits256 *hash2p,struct iguana_bundle *bp,int32_t i);
void *iguana_ramchainfile(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_ramchain *dest,struct iguana_ramchain *R,struct iguana_bundle *bp,int32_t bundlei,struct iguana_block *block);
int32_t iguana_bundlehashadd(struct iguana_info *coin,struct iguana_bundle *bp,int32_t bundlei,struct iguana_block *block);
int32_t iguana_convert(struct iguana_info *coin,int32_t helperid,struct iguana_bundle *bp,int32_t RTflag,int32_t starti);
int32_t iguana_bundleissuemissing(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,int32_t priority,double mult);
FILE *myfopen(char *fname,char *mode);
int32_t myfclose(FILE *fp);
char *iguana_scriptaddress(struct iguana_info *coin,char *coinaddr,uint8_t *script,int32_t scriptlen);
int32_t iguana_walkchain(struct iguana_info *coin,int32_t skipflag);
struct iguana_block *iguana_fastlink(struct iguana_info *coin,int32_t hwmheight);
int32_t iguana_balancenormal(struct iguana_info *coin,struct iguana_bundle *bp,int32_t startheight,int32_t endheight);
int32_t iguana_spendvectorsaves(struct iguana_info *coin);
int32_t iguana_convertfinished(struct iguana_info *coin);
int32_t iguana_emitfinished(struct supernet_info *myinfo,struct iguana_info *coin,int32_t queueincomplete);
int32_t iguana_utxofinished(struct iguana_info *coin);
int32_t iguana_balancefinished(struct iguana_info *coin);
int32_t iguana_alloctxbits(struct iguana_info *coin,struct iguana_ramchain *ramchain);
void iguana_allocvolatile(struct iguana_info *coin,struct iguana_ramchain *ramchain);
int32_t iguana_rwaddr(int32_t rwflag,uint8_t *serialized,struct iguana_msgaddress *addr,int32_t protover);
struct iguana_bundle *iguana_bundleset(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_block **blockp,int32_t *bundleip,struct iguana_block *origblock);
struct iguana_waddress *iguana_waddresscreate(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_waccount *wacct,char *coinaddr,char *redeemScript);

int32_t iguana_peerhdrrequest(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *serialized,int32_t maxsize,struct iguana_peer *addr,bits256 hash2);
int32_t iguana_peeraddrrequest(struct iguana_info *coin,struct iguana_peer *addr,uint8_t *space,int32_t max);
int32_t iguana_peerdatarequest(struct iguana_info *coin,struct iguana_peer *addr,uint8_t *space,int32_t max);
int32_t iguana_peergetrequest(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,uint8_t *data,int32_t recvlen,int32_t getblock);
int32_t iguana_bundlefname(struct iguana_info *coin,struct iguana_bundle *bp,char *fname);
int32_t iguana_bundleremove(struct iguana_info *coin,int32_t hdrsi,int32_t tmpfiles);
int32_t iguana_voutsfname(struct iguana_info *coin,int32_t roflag,char *fname,int32_t slotid);
int32_t iguana_vinsfname(struct iguana_info *coin,int32_t roflag,char *fname,int32_t slotid);
bits256 iguana_merkle(char *symbol,bits256 *tree,int32_t txn_count);
int32_t iguana_bundleready(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,int32_t requiredflag);
int32_t iguana_blast(struct iguana_info *coin,struct iguana_peer *addr);
int32_t iguana_validated(struct iguana_info *coin);
void iguana_volatilesalloc(struct iguana_info *coin,struct iguana_ramchain *ramchain,int32_t copyflag);
int32_t iguana_send_ping(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr);
int32_t iguana_process_msgrequestQ(struct supernet_info *myinfo,struct iguana_info *coin);
uint32_t iguana_fastfindinit(struct iguana_info *coin);
int32_t iguana_outptset(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_outpoint *outpt,bits256 txid,int32_t vout,int64_t value,char *spendscriptstr);
int32_t iguana_unspentindfind(struct supernet_info *myinfo,struct iguana_info *coin,uint64_t *spentamountp,char *coinaddr,uint8_t *spendscript,int32_t *scriptlenp,uint64_t *valuep,int32_t *heightp,bits256 txid,int32_t vout,int32_t lasthdrsi,int32_t mempool);
int32_t iguana_RTunspentindfind(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_outpoint *outpt,char *coinaddr,uint8_t *spendscript,int32_t *scriptlenp,uint64_t *valuep,int32_t *heightp,bits256 txid,int32_t vout,int32_t lasthdrsi,int32_t mempool);
int32_t iguana_addressvalidate(struct iguana_info *coin,uint8_t *addrtypep,char *address);
int32_t bitcoin_sign(void *ctx,char *symbol,uint8_t *sig,bits256 txhash2,bits256 privkey,int32_t recoverflag);
void revcalc_rmd160_sha256(uint8_t rmd160[20],bits256 revhash);
struct iguana_utxo iguana_utxofind(struct iguana_info *coin,struct iguana_outpoint spentpt,int32_t *RTspendflagp,int32_t lockflag);
int32_t iguana_vinifind(struct supernet_info *myinfo,struct iguana_info *coin,bits256 *spentfrom,bits256 txid,int32_t vout);
int32_t iguana_scriptsigextract(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *script,int32_t maxsize,bits256 txid,int32_t vini);
void iguana_unspents_markinit(struct supernet_info *myinfo,struct iguana_info *coin);

bits256 iguana_str2priv(struct supernet_info *myinfo,struct iguana_info *coin,char *str);
int32_t iguana_RTspentflag(struct supernet_info *myinfo,struct iguana_info *coin,uint64_t *RTspendp,int32_t *spentheightp,struct iguana_ramchain *ramchain,struct iguana_outpoint spentpt,int32_t height,int32_t minconf,int32_t maxconf,uint64_t amount);
int32_t iguana_voutscript(struct iguana_info *coin,struct iguana_bundle *bp,uint8_t *scriptspace,char *asmstr,struct iguana_unspent *u,struct iguana_pkhash *p,int32_t txi);
struct iguana_RTaddr *iguana_RTaddrfind(struct iguana_info *coin,uint8_t *rmd160,char *coinaddr);
cJSON *iguana_RTunspentjson(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_outpoint outpt,bits256 txid,int32_t vout,uint64_t value,struct iguana_unspent *up,uint8_t rmd160[20],char *coinaddr,uint8_t *pubkey33,int32_t spentheight,char *remoteaddr);
int32_t bitcoin_standardspend(uint8_t *script,int32_t n,uint8_t rmd160[20]);
struct iguana_waddress *iguana_waddresssearch(struct supernet_info *myinfo,struct iguana_waccount **wacctp,char *coinaddr);
cJSON *iguana_RTlistunspent(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *argarray,int32_t minconf,int32_t maxconf,char *remoteaddr,int32_t includespends);
uint64_t iguana_interest(struct supernet_info *myinfo,struct iguana_info *coin,bits256 txid,int32_t vout,uint64_t value);
void calc_shares(struct supernet_info *myinfo,uint8_t *shares,uint8_t *secret,int32_t size,int32_t width,int32_t M,int32_t N,uint8_t *sharenrs,uint8_t *space,int32_t spacesize);
//struct basilisk_spend *basilisk_addspend(struct supernet_info *myinfo,char *symbol,bits256 txid,uint16_t vout,int32_t addflag);
//void iguana_wallet_Cclear(struct supernet_info *myinfo,char *symbol);
int64_t _RTgettxout(struct iguana_info *coin,struct iguana_RTtxid **ptrp,int32_t *heightp,int32_t *scriptlenp,uint8_t *script,uint8_t *rmd160,char *coinaddr,bits256 txid,int32_t vout,int32_t mempool);
int32_t _iguana_RTunspentfind(struct supernet_info *myinfo,struct iguana_info *coin,bits256 *txidp,int32_t *voutp,uint8_t *spendscript,struct iguana_outpoint outpt,int64_t value);
int32_t basilisk_unspentfind(struct supernet_info *myinfo,struct iguana_info *coin,bits256 *txidp,int32_t *voutp,uint8_t *spendscript,struct iguana_outpoint outpt,int64_t value);
int64_t iguana_addressreceived(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *json,char *remoteaddr,cJSON *txids,cJSON *vouts,cJSON *unspents,cJSON *spends,char *coinaddr,int32_t minconf,int32_t firstheight);
cJSON *iguana_walletjson(struct supernet_info *myinfo);
int32_t iguana_payloadupdate(struct supernet_info *myinfo,struct iguana_info *coin,char *retstr,struct iguana_waddress *waddr,char *account);
int32_t bitcoin_MofNspendscript(uint8_t p2sh_rmd160[20],uint8_t *script,int32_t n,const struct vin_info *vp);
cJSON *basilisk_balance_valsobj(struct supernet_info *myinfo,struct iguana_info *coin);
void basilisk_unspents_update(struct supernet_info *myinfo,struct iguana_info *coin);
struct iguana_txid *iguana_blocktx(struct iguana_info *coin,struct iguana_txid *tx,struct iguana_block *block,int32_t i);
cJSON *iguana_p2shjson(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *retjson,struct iguana_waddress *waddr);
char *setaccount(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_waddress **waddrp,char *account,char *coinaddr,char *redeemScript);
char *iguana_APIrequest(struct iguana_info *coin,bits256 blockhash,bits256 txid,int32_t seconds);
int32_t bitcoin_verifyvins(struct iguana_info *coin,int32_t height,bits256 *signedtxidp,char **signedtx,struct iguana_msgtx *msgtx,uint8_t *serialized,int32_t maxsize,struct vin_info *V,uint32_t sighash,int32_t signtx,int32_t suppress_pubkeys);
char *iguana_validaterawtx(struct supernet_info *myinfo,struct iguana_info *coin,int32_t height,struct iguana_msgtx *msgtx,uint8_t *extraspace,int32_t extralen,char *rawtx,int32_t mempool,int32_t suppress_pubkeys);
int64_t iguana_fastfindcreate(struct iguana_info *coin);
int32_t bitcoin_validaddress(struct iguana_info *coin,char *coinaddr);
int32_t iguana_volatileupdate(struct iguana_info *coin,int32_t incremental,struct iguana_ramchain *spentchain,int16_t spent_hdrsi,uint32_t spent_unspentind,uint32_t spent_pkind,uint64_t spent_value,uint32_t spendind,uint32_t fromheight);
void iguana_utxoaddrs_purge(struct iguana_info *coin);
int32_t iguana_utxoupdate(struct iguana_info *coin,int16_t spent_hdrsi,uint32_t spent_unspentind,uint32_t spent_pkind,uint64_t spent_value,uint32_t spendind,uint32_t fromheight,uint8_t *rmd160);
int32_t iguana_RTunspentslists(struct supernet_info *myinfo,struct iguana_info *coin,uint64_t *totalp,struct iguana_outpoint *unspents,int32_t max,uint64_t required,int32_t minconf,cJSON *addresses,char *remoteaddr);
double basilisk_request_listprocess(struct supernet_info *myinfo,struct basilisk_request *issueR,struct basilisk_request *list,int32_t n);
void iguana_unspents_mark(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *vins);
int32_t iguana_immediate(struct iguana_info *coin,int32_t immedmillis);
int32_t iguana_fastfindreset(struct iguana_info *coin);
int64_t iguana_unspentset(struct supernet_info *myinfo,struct iguana_info *coin);
int32_t iguana_txidfastfind(struct iguana_info *coin,int32_t *heightp,bits256 txid,int32_t lasthdrsi);
uint8_t iguana_addrtype(struct iguana_info *coin,uint8_t script_type);
struct iguana_waddress *iguana_waddressadd(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_waccount *wacct,struct iguana_waddress *addwaddr,char *redeemScript);
cJSON *iguana_createvins(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *txobj,cJSON *vins);
bits256 bitcoin_pubkey33(void *ctx,uint8_t *data,bits256 privkey);
bits256 bitcoin_randkey(void *ctx);
int32_t bitcoin_recoververify(void *ctx,char *symbol,uint8_t *sig64,bits256 messagehash2,uint8_t *pubkey,size_t plen);
int32_t bitcoin_assembler(struct iguana_info *coin,cJSON *logarray,uint8_t script[IGUANA_MAXSCRIPTSIZE],cJSON *scriptobj,int32_t interpret,int64_t nLockTime,struct vin_info *V);
cJSON *iguana_spendasm(struct iguana_info *coin,uint8_t *spendscript,int32_t spendlen);
uint64_t iguana_unspentavail(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_outpoint outpt,int32_t minconf,int32_t maxconf);
int32_t iguana_RTutxofunc(struct iguana_info *coin,int32_t *fromheightp,int32_t *lockedflagp,struct iguana_outpoint spentpt,int32_t *RTspendflagp,int32_t lockflag,int32_t spentheight);
int32_t iguana_signrawtransaction(struct supernet_info *myinfo,struct iguana_info *coin,int32_t height,struct iguana_msgtx *msgtx,char **signedtxp,bits256 *signedtxidp,struct vin_info *V,int32_t numinputs,char *rawtx,cJSON *vins,cJSON *privkeys);
cJSON *iguana_privkeysjson(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *vins);
char *iguana_RTinputaddress(struct supernet_info *myinfo,struct iguana_info *coin,char *coinaddr,struct iguana_outpoint *spentptp,cJSON *vinobj);
struct iguana_waddress *iguana_getaccountaddress(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *json,char *remoteaddr,char *coinaddr,char *account);
int32_t iguana_RTuvaltxid(struct supernet_info *myinfo,bits256 *txidp,struct iguana_info *coin,struct iguana_outpoint outpt);
struct instantdex_accept *instantdex_quotefind(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,bits256 encodedhash);
int32_t instantdex_quoterequest(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *serialized,int32_t maxlen,struct iguana_peer *addr,bits256 encodedhash);
int32_t instantdex_peerhas_clear(struct iguana_info *coin,struct iguana_peer *addr);
int32_t instantdex_quotep2p(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,uint8_t *serialized,int32_t recvlen);
void instantdex_update(struct supernet_info *myinfo);
cJSON *iguana_getaddressesbyaccount(struct supernet_info *myinfo,struct iguana_info *coin,char *account);
int32_t iguana_interpreter(struct iguana_info *coin,cJSON *logarray,int64_t nLockTime,struct vin_info *V,int32_t numvins);
int32_t iguana_parsevinobj(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *serialized,int32_t maxsize,struct iguana_msgvin *vin,cJSON *vinobj,struct vin_info *V);
//int64_t iguana_availunspents(struct supernet_info *myinfo,uint64_t **unspentsp,int32_t *nump,struct iguana_info *coin,int32_t minconf,char *account,void *ptr,int32_t maxsize);
char *iguana_signunspents(struct supernet_info *myinfo,struct iguana_info *coin,bits256 *signedtxidp,int32_t *completedp,cJSON *txobj,uint64_t satoshis,char *changeaddr,uint64_t txfee,struct iguana_outpoint *unspents,int32_t num);
bits256 iguana_sendrawtransaction(struct supernet_info *myinfo,struct iguana_info *coin,char *signedtx);
int32_t iguana_inv2packet(uint8_t *serialized,int32_t maxsize,int32_t type,bits256 *hashes,int32_t n);
int32_t instantdex_inv2data(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_peer *addr,struct exchange_info *exchange);
uint32_t basilisk_crcrecv(struct supernet_info *myinfo,int32_t width,uint8_t *verifybuf,int32_t maxlen,int32_t *datalenp,bits256 srchash,bits256 desthash,uint32_t channel,uint32_t msgbits);
struct iguana_bundlereq *instantdex_recvquotes(struct iguana_info *coin,struct iguana_bundlereq *req,bits256 *encodedhash,int32_t n);
uint32_t basilisk_crcsend(struct supernet_info *myinfo,int32_t width,uint8_t *verifybuf,int32_t maxlen,bits256 srchash,bits256 desthash,uint32_t channel,uint32_t msgbits,uint8_t *data,int32_t datalen,uint32_t crcs[2]);
struct exchange_info *exchange_create(char *exchangestr,cJSON *argjson);
int32_t iguana_inv2poll(struct supernet_info *myinfo,struct iguana_info *coin);
struct iguana_bundlereq *iguana_bundlereq(struct iguana_info *coin,struct iguana_peer *addr,int32_t type,uint8_t *data,int32_t datalen);
void instantdex_FSMinit();
//void dpow_handler(struct supernet_info *myinfo,struct basilisk_message *msg);
int32_t datachain_opreturnscript(struct iguana_info *coin,uint8_t *script,char *datastr,int32_t datalen);
int32_t basilisk_messagekeyread(uint8_t *key,uint32_t *channelp,uint32_t *msgidp,bits256 *srchashp,bits256 *desthashp);
cJSON *basilisk_channelget(struct supernet_info *myinfo,bits256 srchash,bits256 desthash,uint32_t channel,uint32_t msgid,int32_t width);
int32_t basilisk_channelsend(struct supernet_info *myinfo,bits256 srchash,bits256 desthash,uint32_t channel,uint32_t msgid,uint8_t *data,int32_t datalen,uint32_t duration);
cJSON *dpow_listunspent(struct supernet_info *myinfo,struct iguana_info *coin,char *coinaddr);
void iguana_dPoWupdate(struct supernet_info *myinfo,struct dpow_info *dp);
char *iguana_utxoduplicates(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *pubkey33,uint64_t satoshis,int32_t duplicates,int32_t *completedp,bits256 *signedtxidp,int32_t sendflag,cJSON *addresses);
int32_t bitcoin_p2shspend(uint8_t *script,int32_t n,uint8_t rmd160[20]);
void iguana_RTunspentslock(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *vins);
char *iguana_calcrawtx(struct supernet_info *myinfo,struct iguana_info *coin,cJSON **vinsp,cJSON *txobj,int64_t satoshis,char *changeaddr,int64_t txfee,cJSON *addresses,int32_t minconf,uint8_t *opreturn,int32_t oplen,int64_t burnamount,char *remoteaddr,struct vin_info *V,int32_t maxmode);
int64_t datachain_update(struct supernet_info *myinfo,int32_t ordered,struct iguana_info *coin,uint32_t timestamp,struct iguana_bundle *bp,uint8_t rmd160[20],int64_t crypto777_payment,uint8_t type,int32_t height,uint64_t hdrsi_unspentind,int64_t burned,uint32_t fileid,uint64_t scriptpos,int32_t scriptlen,bits256 txid,int32_t vout);
void datachain_update_spend(struct supernet_info *myinfo,int32_t ordered,struct iguana_info *coin,uint32_t timestamp,struct iguana_bundle *bp,int32_t height,bits256 txid,int32_t vout,uint8_t rmd160[20],int64_t value);
cJSON *bitcoin_data2json(struct iguana_info *coin,int32_t height,bits256 *txidp,struct iguana_msgtx *msgtx,uint8_t *extraspace,int32_t extralen,uint8_t *serialized,int32_t len,cJSON *vins,int32_t suppress_pubkeys);
int32_t iguana_unspentind2txid(struct supernet_info *myinfo,struct iguana_info *coin,int32_t *spentheightp,bits256 *txidp,int32_t *voutp,struct iguana_outpoint outpt);
int32_t iguana_RTunspent_check(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_outpoint outpt);
char *basilisk_bitcoinrawtx(struct supernet_info *myinfo,struct iguana_info *coin,char *remoteaddr,uint32_t basilisktag,int32_t timeoutmillis,cJSON *valsobj,struct vin_info *V);
int32_t iguana_markedunspents_find(struct iguana_info *coin,int32_t *firstslotp,bits256 txid,int32_t vout);
void jumblr_opidsupdate(struct supernet_info *myinfo,struct iguana_info *coin);

char *iguana_signrawtx(struct supernet_info *myinfo,struct iguana_info *coin,int32_t height,bits256 *signedtxidp,int32_t *completedp,cJSON *vins,char *rawtx,cJSON *privkeys,struct vin_info *V);
bits256 scrypt_blockhash(const void *input);
bits256 iguana_calcblockhash(char *symbol,int32_t (*hashalgo)(uint8_t *blockhashp,uint8_t *serialized,int32_t len),uint8_t *serialized,int32_t len);
struct bitcoin_eventitem *instantdex_event(char *cmdstr,cJSON *argjson,cJSON *newjson,uint8_t *serdata,int32_t serdatalen);
void instantdex_eventfree(struct bitcoin_eventitem *ptr);
struct iguana_monitorinfo *iguana_txidmonitor(struct iguana_info *coin,bits256 txid);
struct iguana_monitorinfo *iguana_txidreport(struct iguana_info *coin,bits256 txid,struct iguana_peer *addr);
double iguana_txidstatus(struct supernet_info *myinfo,struct iguana_info *coin,bits256 txid);
void basilisk_functions(struct iguana_info *coin,int32_t protocol);
char *bitcoind_passthru(char *coinstr,char *serverport,char *userpass,char *method,char *params);
char *bitcoin_calcrawtx(struct supernet_info *myinfo,struct iguana_info *coin,cJSON **vinsp,int64_t satoshis,char *paymentscriptstr,char *changeaddr,int64_t txfee,cJSON *addresses,int32_t minconf,uint32_t locktime);
char *bitcoin_blockhashstr(char *coinstr,char *serverport,char *userpass,int32_t height);
bits256 basilisk_blockhash(struct iguana_info *coin,bits256 prevhash2);
void calc_scrypthash(uint32_t *hash,void *data);
int32_t iguana_rwvarstr(int32_t rwflag,uint8_t *serialized,int32_t maxlen,char *endianedp);
bits256 bitcoin_sharedsecret(void *ctx,bits256 privkey,uint8_t *pubkey,int32_t plen);
int32_t iguana_blockhdrsize(char *symbol,uint8_t zcash,uint8_t auxpow);//,uint8_t *serialized,int32_t maxlen);
int32_t iguana_blockROsize(uint8_t zcash);
void *iguana_blockzcopyRO(uint8_t zcash,struct iguana_blockRO *dest,int32_t desti,struct iguana_blockRO *src,int32_t srci);
void iguana_blockzcopy(uint8_t zcash,struct iguana_block *dest,struct iguana_block *src);
int32_t iguana_blocksizecheck(char *debugstr,uint8_t zcash,struct iguana_block *block);
void basilisk_miner(struct supernet_info *myinfo,struct iguana_info *btcd,struct iguana_info *virt,int32_t maxmillis,char *mineraddr);
int32_t bitcoin_secret160verify(uint8_t *script,int32_t n,uint8_t secret160[20]);
int32_t bitcoin_secret256spend(uint8_t *script,int32_t n,bits256 secret);
int32_t bitcoin_pubkeyspend(uint8_t *script,int32_t n,uint8_t pubkey[66]);
int32_t basilisk_blocksubmit(struct supernet_info *myinfo,struct iguana_info *btcd,struct iguana_info *virt,struct iguana_peer *addr,char *blockstr,bits256 hash2,int32_t height);
struct supernet_info *SuperNET_MYINFO(char *passphrase);
bits256 calc_categoryhashes(bits256 *subhashp,char *category,char *subcategory);
struct gecko_chain *category_find(bits256 categoryhash,bits256 subhash);
void *category_subscribe(struct supernet_info *myinfo,bits256 category,bits256 keyhash);
char *bitcoin_address(char *coinaddr,uint8_t addrtype,uint8_t *pubkey_or_rmd160,int32_t len);
char *SuperNET_JSON(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *json,char *remoteaddr,uint16_t port);
int64_t iguana_txdetails(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *item,bits256 txid,int32_t vout,int32_t height);
int32_t iguana_txidheight(struct supernet_info *myinfo,struct iguana_info *coin,bits256 txid);
int64_t iguana_txidamount(struct supernet_info *myinfo,struct iguana_info *coin,char *coinaddr,bits256 txid,int32_t vout);
char *iguana_txidcategory(struct supernet_info *myinfo,struct iguana_info *coin,char *account,char *coinaddr,bits256 txid,int32_t vout);
struct supernet_info *SuperNET_accountfind(cJSON *json);
cJSON *SuperNET_rosettajson(struct supernet_info *myinfo,bits256 privkey,int32_t showprivs);
double instantdex_aveprice(struct supernet_info *myinfo,struct exchange_quote *sortbuf,int32_t max,double *totalvolp,char *base,char *rel,double basevolume,cJSON *argjson);
char *SuperNET_keysinit(struct supernet_info *myinfo,char *argjsonstr);
char *SuperNET_parser(struct supernet_info *myinfo,char *agentstr,char *method,cJSON *json,char *remoteaddr);
char *SuperNET_htmlstr(char *fname,char *htmlstr,int32_t maxsize,char *agentstr);
void SuperNET_setkeys(struct supernet_info *myinfo,void *pass,int32_t passlen,int32_t dosha256);
int32_t iguana_headerget(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t *serialized,int32_t maxsize,struct iguana_block *block);
int32_t iguana_bundlefinalize(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp,struct OS_memspace *mem,struct OS_memspace *memB);
bits256 iguana_parsetxobj(struct supernet_info *myinfo,struct iguana_info *coin,int32_t *txstartp,uint8_t *serialized,int32_t maxsize,struct iguana_msgtx *msg,cJSON *txobj,struct vin_info *V);
int32_t iguana_ROallocsize(struct iguana_info *virt);
uint64_t iguana_utxoaddr_gen(struct supernet_info *myinfo,struct iguana_info *coin,int32_t maxheight);
long iguana_bundlesload(struct supernet_info *myinfo,struct iguana_info *coin);
void basilisk_wait(struct supernet_info *myinfo,struct iguana_info *coin);
int32_t bitcoin_pubkey_combine(void *ctx,uint8_t *combined_pub,uint8_t *skipkey,bits256 *evenkeys,int32_t n,bits256 *oddkeys,int32_t m);
bits256 bitcoin_pub256(void *ctx,bits256 *privkeyp,uint8_t odd_even);
bits256 bitcoin_schnorr_noncepair(void *ctx,uint8_t *pubnonce,bits256 txhash2,bits256 privkey);
int32_t bitcoin_schnorr_combine(void *ctx,uint8_t *sig64,uint8_t *allpub,uint8_t **sigs,int32_t n,bits256 txhash2);
int32_t bitcoin_schnorr_verify(void *ctx,uint8_t *sig64,bits256 txhash2,uint8_t *pubkey,int32_t plen);
int32_t iguana_parsevoutobj(struct iguana_info *coin,uint8_t *serialized,int32_t maxsize,struct iguana_msgvout *vout,cJSON *voutobj);
struct gecko_memtx *gecko_unspentfind(struct gecko_memtx ***ptrpp,struct iguana_info *virt,bits256 txid);
int64_t *gecko_valueptr(struct gecko_memtx *memtx,int32_t vout);
struct iguana_peer *iguana_peerfindipaddr(struct iguana_info *coin,char *ipaddr,int32_t needalive);
struct iguana_peer *iguana_peerfindipbits(struct iguana_info *coin,uint32_t ipbits,int32_t needalive);
//int32_t basilisk_relays_send(struct supernet_info *myinfo,struct iguana_peer *addr);
int32_t basilisk_hashes_send(struct supernet_info *myinfo,struct iguana_info *virt,struct iguana_peer *addr,char *CMD,bits256 *txids,int32_t num);
int32_t iguana_opreturn(struct supernet_info *myinfo,int32_t ordered,struct iguana_info *coin,uint32_t timestamp,struct iguana_bundle *bp,int64_t crypto777_payment,int32_t height,uint64_t hdrsi_unspentind,int64_t payment,uint32_t fileid,uint64_t scriptpos,uint32_t scriptlen);
/*
* because the address passed in a non-portable way we defined uint64_t as parameter to 
* allow the pass of 64bit memory address in windows 64
* @author - fadedreamz@gmail.com
*/
#if defined(_M_X64)
int32_t iguana_scriptdata(struct iguana_info *coin, uint8_t *scriptspace, uint64_t fileptr[2], char *fname, uint64_t scriptpos, int32_t scriptlen);
#else
int32_t iguana_scriptdata(struct iguana_info *coin,uint8_t *scriptspace,long fileptr[2],char *fname,uint64_t scriptpos,int32_t scriptlen);
#endif
void basilisk_ensurerelay(struct supernet_info *myinfo,struct iguana_info *notaries,uint32_t ipbits);
void dpow_nanomsginit(struct supernet_info *myinfo,char *ipaddr);
int32_t iguana_datachain_scan(struct supernet_info *myinfo,struct iguana_info *coin,uint8_t rmd160[20]);
int32_t basilisk_requests_poll(struct supernet_info *myinfo);
void dpow_psockloop(void *_ptr);
int32_t smartaddress_add(struct supernet_info *myinfo,bits256 privkey,char *typestr,char *symbol,double maxbid,double minask);
int32_t smartaddress(struct supernet_info *myinfo,char *typestr,double *bidaskp,bits256 *privkeyp,char *symbol,char *coinaddr);
int32_t smartaddress_pubkey(struct supernet_info *myinfo,char *typestr,double *bidaskp,bits256 *privkeyp,char *symbol,bits256 pubkey);
int32_t smartaddress_pubkey33(struct supernet_info *myinfo,char *typestr,double *bidaskp,bits256 *privkeyp,char *symbol,uint8_t *pubkey33);

void iguana_RTreset(struct iguana_info *coin);
void iguana_RTpurge(struct iguana_info *coin,int32_t lastheight);
void iguana_RTnewblock(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_block *block);
cJSON *iguana_listunspents(struct supernet_info *myinfo,struct iguana_info *coin,cJSON *array,int32_t minconf,int32_t maxconf,char *remoteaddr);
void *iguana_RTrawdata(struct iguana_info *coin,bits256 hash2,uint8_t *data,int32_t *recvlenp,int32_t *numtxp,int32_t checkonly);
int32_t iguana_bundlehash2_check(struct iguana_info *coin,bits256 hash2);
void iguana_RTramchainalloc(char *fname,struct iguana_info *coin,struct iguana_bundle *bp);
void iguana_update_balances(struct supernet_info *myinfo,struct iguana_info *coin);
void iguana_RTspendvectors(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_bundle *bp);
int64_t iguana_RTbalance(struct iguana_info *coin,char *coinaddr);
double instantdex_avehbla(struct supernet_info *myinfo,double retvals[4],char *base,char *rel,double basevolume);
int32_t bitcoin_secret160verify(uint8_t *script,int32_t n,uint8_t secret160[20]);
int64_t iguana_lockval(int32_t finalized,int64_t locktime);
uint64_t *iguana_PoS_weights(struct supernet_info *myinfo,struct iguana_info *coin,struct iguana_pkhash **Ptrp,uint64_t *supplyp,int32_t *numacctsp,int32_t *nonzp,int32_t *errsp,int32_t lastheight);
int32_t iguana_staker_sort(struct iguana_info *coin,bits256 *hash2p,uint8_t *refrmd160,struct iguana_pkhash *refP,uint64_t *weights,int32_t numweights,bits256 *sortbuf);
bits256 mpz_div64(bits256 hash,uint64_t divval);
void iguana_walletinitcheck(struct supernet_info *myinfo,struct iguana_info *coin);
void jumblr_iteration(struct supernet_info *myinfo,struct iguana_info *coin,int32_t selector,int32_t modval);
void smartaddress_update(struct supernet_info *myinfo,int32_t selector);
bits256 jumblr_privkey(struct supernet_info *myinfo,char *BTCaddr,uint8_t pubtype,char *KMDaddr,char *prefix);
char *jumblr_importprivkey(struct supernet_info *myinfo,struct iguana_info *coin,char *wifstr);
int64_t iguana_esttxfee(struct supernet_info *myinfo,struct iguana_info *coin,char *rawtx,char *signedtx,int32_t numvins);
int64_t jumblr_balance(struct supernet_info *myinfo,struct iguana_info *coin,char *addr);

// ------------------------------------------------------[ Preparation ]----
// Initialise a gfshare context for producing shares
gfshare_ctx *gfshare_ctx_initenc(uint8_t * /* sharenrs */,uint32_t /* sharecount */, uint8_t /* threshold */,uint32_t /* size */,void *,int32_t);
// Initialise a gfshare context for recombining shares
gfshare_ctx *gfshare_ctx_initdec(uint8_t * /* sharenrs */,uint32_t /* sharecount */,uint32_t /* size */,void *,int32_t);
// Free a share context's memory
void gfshare_ctx_free(gfshare_ctx * /* ctx */);
// --------------------------------------------------------[ Splitting ]----
// Provide a secret to the encoder. (this re-scrambles the coefficients)
void gfshare_ctx_enc_setsecret(gfshare_ctx * /* ctx */,uint8_t * /* secret */);
// Extract a share from the context. 'share' must be preallocated and at least 'size' bytes long. 'sharenr' is the index into the 'sharenrs' array of the share you want.
void gfshare_ctx_encgetshare(uint8_t *logs,uint8_t *exps,gfshare_ctx * /* ctx */,uint8_t /* sharenr */,uint8_t * /* share */);
// ----------------------------------------------------[ Recombination ]----
// Inform a recombination context of a change in share indexes
void gfshare_ctx_dec_newshares(gfshare_ctx * /* ctx */, uint8_t * /* sharenrs */);
// Provide a share context with one of the shares. The 'sharenr' is the index into the 'sharenrs' array
void gfshare_ctx_dec_giveshare(gfshare_ctx * /* ctx */,uint8_t /* sharenr */,uint8_t * /* share */);
// Extract the secret by interpolation of the shares. secretbuf must be allocated and at least 'size' bytes long
void gfshare_ctx_decextract(uint8_t *logs,uint8_t *exps,gfshare_ctx * /* ctx */,uint8_t * /* secretbuf */);
void libgfshare_init(struct supernet_info *myinfo,uint8_t _logs[256],uint8_t _exps[510]);
int32_t init_sharenrs(uint8_t sharenrs[255],uint8_t *orig,int32_t m,int32_t n);
void iguana_schnorr(struct supernet_info *myinfo);
void iguana_fixsecp(struct supernet_info *myinfo);
int32_t bitcoin_timelockspend(uint8_t *script,int32_t n,uint8_t rmd160[20],uint32_t timestamp);

char *iguana_calcutxorawtx(struct supernet_info *myinfo,struct iguana_info *coin,cJSON **vinsp,cJSON *txobj,int64_t *satoshis,int32_t numoutputs,char *changeaddr,int64_t txfee,cJSON *utxos,char *remoteaddr,struct vin_info *V,int32_t maxmode);
uint64_t _iguana_interest(uint32_t now,int32_t height,uint32_t txlocktime,uint64_t value);

#include "../includes/iguana_api.h"


#endif

