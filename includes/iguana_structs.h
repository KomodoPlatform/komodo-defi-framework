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

#ifndef H_IGUANASTRUCTS_H
#define H_IGUANASTRUCTS_H

#ifdef WIN32
#define PACKEDSTRUCT
#else
#define PACKEDSTRUCT __attribute__((packed))
#endif

struct iguana_blockreq { struct queueitem DL; bits256 hash2,*blockhashes; struct iguana_bundle *bp; int32_t n,height,bundlei; };

struct iguana_peermsgrequest { struct queueitem DL; struct iguana_peer *addr; bits256 hash2; int32_t type; };

struct iguana_chain
{
	//const int32_t chain_id;
    char name[32],symbol[16],messagemagic[64];
    uint8_t pubtype,p2shtype,wiftype,netmagic[4];
    char *genesis_hash,*genesis_hex; // hex string
    uint16_t portp2p,rpcport;
    uint8_t isPoS,unitval;
    uint64_t rewards[512][2];
    uint8_t genesis_hashdata[32],minconfirms;
    uint16_t bundlesize,hasheaders;
    char gethdrsmsg[16];
    uint64_t txfee,minoutput,dust,halvingduration,initialreward;
    blockhashfunc hashalgo;
    char userhome[512],serverport[128],userpass[1024];
    char use_addmultisig,do_opreturn;
    int32_t estblocktime,protover;
    bits256 genesishash2,PoWtarget,PoStargets[16]; int32_t numPoStargets,PoSheights[16];
    uint8_t zcash,fixit,auxpow,debug,havecltv,alertpubkey[65];
    uint16_t targetspacing,targettimespan; uint32_t nBits,normal_txversion,locktime_txversion;
};

struct iguana_msgaddress {	uint32_t nTime; uint64_t nServices; uint8_t ip[16]; uint16_t port; }PACKEDSTRUCT;

struct iguana_msgversion
{
	uint32_t nVersion;
	uint64_t nServices;
	int64_t nTime;
	struct iguana_msgaddress addrTo,addrFrom;
	uint64_t nonce;
	char strSubVer[80];
	uint32_t nStartingHeight;
    uint8_t relayflag;
}PACKEDSTRUCT;

struct iguana_msgalert // warning, many varints/variable length fields, struct is 1:1
{
    int32_t version;
    int64_t relayuntil,expiration;
    int32_t ID,cancel;
    uint32_t numcancellist;
    int32_t minver,maxver;
    uint32_t setsubvervar; char subver[1024];
    int32_t priority;
    char comment[1024],statusbar[1024],reserved[1024];
    uint8_t siglen,sig[74];
    uint32_t list[64];
};

struct iguana_VPNversion
{
	uint32_t nVersion;
	uint64_t nServices;
	int64_t nTime;
	struct iguana_msgaddress addrTo,addrFrom;
	uint64_t nonce;
	char strSubVer[80];
	uint32_t nStartingHeight;
    uint32_t iVer,v_Network_id; uint16_t wPort; uint8_t bIsGui; uint16_t wCtPort,wPrPort;
} PACKEDSTRUCT;

struct iguana_msgblockhdr
{
    uint32_t version;
    bits256 prev_block,merkle_root;
    uint32_t timestamp,bits,nonce;
} PACKEDSTRUCT;

#define ZKSNARK_PROOF_SIZE 296
#define ZCASH_SOLUTION_ELEMENTS 1344

struct iguana_msgzblockhdr
{
    uint32_t version;
    bits256 prev_block,merkle_root,reserved;
    uint32_t timestamp,bits;
    bits256 bignonce;
    uint8_t var_numelements[3];
    uint8_t solution[ZCASH_SOLUTION_ELEMENTS];
} PACKEDSTRUCT;

/*int32_t nVersion;
uint256 hashPrevBlock;
uint256 hashMerkleRoot;
uint256 hashReserved;
uint32_t nTime;
uint32_t nBits;
uint256 nNonce;
std::vector<unsigned char> nSolution;*/

/*struct iguana_msgblockhdr_zcash
{
    bits256 bignonce;
    uint8_t numelements;
    uint32_t solution[ZCASH_SOLUTION_ELEMENTS];
    //bits256 reserved; // only here if auxpow is set
}PACKEDSTRUCT;*/

struct iguana_msgmerkle
{
    uint32_t branch_length;
    bits256 branch_hash[4096];
    uint32_t branch_side_mask;
}; //PACKEDSTRUCT;

struct iguana_msgblock
{
    struct iguana_msgblockhdr H; // double hashed for blockhash
    uint32_t txn_count;
} PACKEDSTRUCT;

struct iguana_msgzblock
{
    struct iguana_msgzblockhdr zH; // double hashed for blockhash
    uint32_t txn_count;
} PACKEDSTRUCT;

struct iguana_msgvin { bits256 prev_hash; uint8_t *vinscript,*userdata,*spendscript,*redeemscript; uint32_t prev_vout,sequence; uint16_t scriptlen,p2shlen,userdatalen,spendlen; }; //PACKEDSTRUCT;

struct iguana_msgvout { uint64_t value; uint32_t pk_scriptlen; uint8_t *pk_script; }; //PACKEDSTRUCT;

struct iguana_msgtx
{
    uint32_t version,tx_in,tx_out,lock_time;
    struct iguana_msgvin *vins;
    struct iguana_msgvout *vouts;
    bits256 txid;
    int32_t allocsize,timestamp,numinputs,numoutputs;
    int64_t inputsum,outputsum,txfee;
    uint8_t *serialized;
};// PACKEDSTRUCT;

struct iguana_msgjoinsplit
{
    uint64_t vpub_old,vpub_new;
    bits256 anchor,nullifiers[2],commitments[2],ephemeralkey;
    bits256 randomseed,vmacs[2];
    uint8_t zkproof[ZKSNARK_PROOF_SIZE];
    uint8_t ciphertexts[2][601];
}PACKEDSTRUCT;

struct iguana_packet { struct queueitem DL; struct iguana_peer *addr; struct tai embargo; int32_t datalen,getdatablock; uint8_t serialized[]; };

struct msgcounts { uint32_t version,verack,getaddr,addr,inv,getdata,notfound,getblocks,getheaders,headers,tx,block,mempool,ping,pong,reject,filterload,filteradd,filterclear,merkleblock,alert; };

//struct iguana_fileitem { bits256 hash2; struct iguana_txdatabits txdatabits; };

struct iguana_kvitem { UT_hash_handle hh; uint8_t keyvalue[]; };

struct iguana_iAddr
{
    UT_hash_handle hh; uint64_t ipbits;
    uint32_t lastkilled,lastconnect;
    int32_t status,height,numkilled,numconnects;
};

struct iguana_cacheptr { struct queueitem DL; int32_t allocsize,recvlen; uint8_t *data; };

// iguana blocks
struct iguana_blockRO
{
    bits256 hash2,prev_block,merkle_root;
    uint32_t timestamp,nonce,bits,version;
    uint32_t firsttxidind,firstvin,firstvout,firstpkind,firstexternalind,recvlen:24,tbd:8;
    uint16_t txn_count,numvouts,numvins,allocsize;
}PACKEDSTRUCT;

struct iguana_zcashRO { bits256 bignonce; uint32_t numelements; uint8_t solution[ZCASH_SOLUTION_ELEMENTS]; } PACKEDSTRUCT;

struct iguana_zblockRO
{
    struct iguana_blockRO RO;
    struct iguana_zcashRO zRO;
} PACKEDSTRUCT;

#define iguana_blockfields      double PoW; \
int32_t height,fpos; uint32_t fpipbits,issued,lag:17,sigsvalid:1,protected:1,peerid:12,processed:1; \
uint16_t hdrsi:15,mainchain:1; int16_t bundlei:12,valid:1,queued:1,txvalid:1,newtx:1; \
UT_hash_handle hh; struct iguana_bundlereq *req; \
struct iguana_blockRO RO

struct iguana_block
{
    iguana_blockfields;
    struct iguana_zcashRO zRO[];
} ;

struct iguana_zblock // mu
{
    iguana_blockfields; // this is to minimize code needed to support both types
    struct iguana_zcashRO zRO; // if zRO is changed, the RO part must also be updated
} ;

#define IGUANA_LHASH_BLOCKS 0
#define IGUANA_LHASH_TXIDS 1 //
#define IGUANA_LHASH_UNSPENTS 2 //
#define IGUANA_LHASH_SPENDS 3 //
#define IGUANA_LHASH_PKHASHES 4 //
#define IGUANA_LHASH_ACCOUNTS 5 //
#define IGUANA_LHASH_EXTERNALS 6 //
#define IGUANA_LHASH_KSPACE 7 //
#define IGUANA_LHASH_TXBITS 8 //
#define IGUANA_LHASH_PKBITS 9 //
#define IGUANA_NUMLHASHES (IGUANA_LHASH_PKBITS + 1)

struct iguana_counts
{
    uint32_t firsttxidind,firstunspentind,firstspendind,firstpkind;
    uint64_t credits,debits;
    struct iguana_block block;
} PACKEDSTRUCT;

struct iguana_blocks
{
    char coin[16];
	struct iguanakv *db;
    struct iguana_block *hash;
    int32_t maxblocks,initblocks,hashblocks,pending,issuedblocks,recvblocks,emitblocks,parsedblocks,dirty;
	struct iguana_zblock hwmchain,prev,prev2;
};

struct iguana_ledger
{
    struct iguana_counts snapshot;
    //struct iguana_account accounts[];
} PACKEDSTRUCT;

// ramchain temp file structures
struct iguana_unspent20 { uint64_t value; uint32_t scriptpos,txidind:28,type:4; uint16_t scriptlen,fileid; uint8_t rmd160[20]; }PACKEDSTRUCT;
struct iguana_spend256 { bits256 prevhash2; uint64_t scriptpos:48,vinscriptlen:16; uint32_t sequenceid; int16_t prevout; uint16_t spendind,fileid; }PACKEDSTRUCT;

// permanent readonly structs
struct iguana_txid { bits256 txid; uint64_t txidind:29,firstvout:28,firstvin:28,bundlei:11,locktime:32,version:32,timestamp:32,extraoffset:32; uint16_t numvouts,numvins; }PACKEDSTRUCT;

struct iguana_unspent { uint64_t value; uint32_t txidind,pkind,prevunspentind,scriptpos,scriptlen:13,fileid:14,type:5; uint16_t hdrsi; int16_t vout; } PACKEDSTRUCT;

struct iguana_spend { uint64_t scriptpos:48,scriptlen:16; uint32_t spendtxidind,sequenceid; int16_t prevout; uint16_t fileid:14,external:1,tbd:1; }PACKEDSTRUCT; // numsigs:4,numpubkeys:4,p2sh:1,sighash:4

struct iguana_pkhash { uint8_t rmd160[20]; uint32_t pkind; }PACKEDSTRUCT; //firstunspentind,pubkeyoffset

// dynamic
struct iguana_account { int64_t total; uint32_t lastunspentind; }PACKEDSTRUCT;
struct iguana_utxo { uint32_t fromheight:31,lockedflag:1,prevunspentind:31,spentflag:1,spendind; }PACKEDSTRUCT;

#ifdef DEPRECATED_HHUTXO
struct iguana_hhaccount { UT_hash_handle hh; uint64_t pval; struct iguana_account a; }PACKEDSTRUCT;
#endif
struct iguana_hhutxo { UT_hash_handle hh; uint64_t uval; struct iguana_utxo u; };
struct iguana_utxoaddr { UT_hash_handle hh; uint64_t histbalance; uint32_t pkind:30,p2sh:1,searchedhist:1; uint16_t hdrsi; uint8_t rmd160[20]; };

// GLOBAL one zero to non-zero write (unless reorg)
struct iguana_spendvector { uint64_t value; uint32_t pkind,unspentind; int32_t fromheight; uint16_t hdrsi:15,tmpflag:1; }PACKEDSTRUCT; // unspentind
//struct iguana_pkextra { uint32_t firstspendind; } PACKEDSTRUCT; // pkind

struct iguana_txblock
{
    uint32_t numtxids,numunspents,numspends,extralen,recvlen;
    // following set during second pass (still in peer context)
    uint32_t numpkinds,numexternaltxids,datalen,pkoffset;
    uint8_t space[256]; // order: extra[], T, U, S, P, external txids
    struct iguana_zblock zblock;
};

#if defined(_M_X64)
/*
* calculate the address in a portable manner
* in all platform sizeof(char) / sizeof(uchar) == 1
* @author - fadedreamz@gmail.com
*/
#define RAMCHAIN_PTR(rdata,offset) ((void *)((unsigned char *)rdata + rdata->offset))
#else
#define RAMCHAIN_PTR(rdata,offset) ((void *)(long)((long)(rdata) + (long)(rdata)->offset))
#endif

struct iguana_ramchaindata
{
    bits256 sha256;
    bits256 lhashes[IGUANA_NUMLHASHES],firsthash2,prevhash2;
    int64_t allocsize,Boffset,Toffset,Uoffset,Soffset,Poffset,Aoffset,Xoffset,TXoffset,PKoffset,Koffset;
    int32_t numblocks,height,firsti,hdrsi,txsparsebits,pksparsebits;
    uint32_t numtxids,numunspents,numspends,numpkinds,numexternaltxids,numtxsparse,numpksparse,scriptspace,stackspace;
    uint8_t rdata[];
};

struct iguana_ramchain_hdr
{
    uint32_t txidind,unspentind,spendind,scriptoffset,stacksize; uint16_t hdrsi,bundlei:15,ROflag:1;
    struct iguana_ramchaindata *data;
};

struct iguana_ramchain
{
    struct iguana_ramchain_hdr H; bits256 lasthash2; uint64_t datasize,allocatedA2,allocatedU2;
    uint32_t numblocks:31,expanded:1,pkind,externalind,height,numXspends;
    long sparseadds,sparsesearches,sparseadditers,sparsesearchiters,sparsehits,sparsemax;
    struct iguana_kvitem *txids,*pkhashes;
    struct OS_memspace *hashmem; long filesize,sigsfilesize,debitsfilesize,lastspendsfilesize;
    void *fileptr,*sigsfileptr,*Xspendptr,*debitsfileptr,*lastspendsfileptr;
    char from_ro,from_roX,from_roA,from_roU;
    struct iguana_account *A,*A2,*creditsA; struct iguana_spendvector *Xspendinds;
    struct iguana_utxo *Uextras; uint8_t *txbits; struct iguana_txid *cacheT;
    //int16_t permutation[IGUANA_MAXBUNDLES];
    //struct iguana_Uextra *U2,*roU2; struct iguana_pkextra *P2,*roP2;
};

struct iguana_bloom16 { uint8_t hash2bits[65536 / 8]; };
struct iguana_bloominds { uint16_t inds[8]; };

struct iguana_bundle
{
    struct queueitem DL; struct iguana_info *coin; struct iguana_bundle *nextbp;
    struct iguana_bloom16 bloom; int64_t totaldurations,duplicatedurations; int32_t durationscount,duplicatescount;
    uint32_t issuetime,hdrtime,emitfinish,mergefinish,purgetime,queued,startutxo,balancefinish,validated,lastspeculative,dirty,nexttime,currenttime,lastprefetch,lastRT,missingstime,unsticktime,converted,utxofinish;
    int32_t numhashes,numrecv,numsaved,numcached,generrs,currentflag,origmissings,numissued,Xvalid;
    int32_t minrequests,n,hdrsi,bundleheight,numtxids,numspends,numunspents,numspec,isRT;
    double avetime,threshold,metric; uint64_t datasize,estsize;
    struct iguana_block *blocks[IGUANA_MAXBUNDLESIZE];
    uint8_t *speculativecache[IGUANA_MAXBUNDLESIZE],haveblock[IGUANA_MAXBUNDLESIZE/3+1];
    uint32_t issued[IGUANA_MAXBUNDLESIZE],firsttxidinds[IGUANA_MAXBUNDLESIZE];
    bits256 prevbundlehash2,hashes[IGUANA_MAXBUNDLESIZE+1],nextbundlehash2,allhash,*speculative,validatehash;
    struct iguana_ramchain ramchain; uint8_t red,green,blue;
    struct iguana_spendvector *tmpspends; int32_t numtmpspends;
    uint64_t *weights,supply; int32_t numweights;
};

struct iguana_bundlereq
{
    struct queueitem DL; struct iguana_info *coin; int32_t type;
    struct iguana_peer *addr; struct iguana_zblock *blocks; bits256 *hashes,txid;
    //struct iguana_txdatabits txdatabits;
    struct iguana_msghdr H;
    int32_t allocsize,datalen,n,recvlen,numtx; uint32_t ipbits;
    struct iguana_zblock zblock;
    uint8_t copyflag,serializeddata[];
};

struct iguana_bitmap { int32_t width,height,amplitude; char name[52]; uint8_t data[IGUANA_WIDTH*IGUANA_HEIGHT*3]; };

struct basilisk_spend { bits256 txid,spentfrom; uint64_t relaymask,value; uint32_t timestamp; int32_t vini,vout,height,unspentheight,ismine; char destaddr[64],symbol[16]; };

struct basilisk_unspent { bits256 txid; uint64_t value,relaymask; uint32_t unspentind,timestamp; int32_t RTheight,height,spentheight; int16_t status,hdrsi,vout,spendlen; char symbol[16]; uint8_t script[256]; };

struct iguana_waddress { UT_hash_handle hh; uint64_t balance; uint16_t scriptlen; uint8_t rmd160[20],pubkey[33],wiftype,addrtype; bits256 privkey; char symbol[16],coinaddr[36],wifstr[54]; uint8_t redeemScript[]; };
struct iguana_waccount { UT_hash_handle hh; struct iguana_waddress *waddr,*current; char account[]; };
struct iguana_wallet { UT_hash_handle hh; struct iguana_waccount *wacct; };

struct scriptinfo { UT_hash_handle hh; uint32_t fpos; uint16_t scriptlen; uint8_t script[]; };
struct hhbits256 { UT_hash_handle hh; bits256 txid; int32_t height; uint16_t firstvout; };

struct iguana_monitorinfo { bits256 txid; int32_t numreported; uint8_t peerbits[IGUANA_MAXPEERS >> 3]; };

struct iguana_RTunspent
{
    uint8_t rmd160[20];
    int64_t value;
    int32_t vout,height,fromheight;
    struct iguana_RTtxid *parent;
    struct iguana_RTspend *spend;
    struct iguana_RTunspent *prevunspent;
    int16_t scriptlen;
    uint8_t locked,validflag;
    uint8_t script[];
};

struct iguana_RTspend
{
    bits256 prev_hash;
    struct iguana_RTunspent *bundle_unspent;
    int16_t prev_vout,scriptlen;
    uint8_t vinscript[];
};

struct iguana_RTaddr
{
    UT_hash_handle hh;
    char coinaddr[64];
    int64_t histbalance,debits,credits;
    int32_t numunspents;
    struct iguana_RTunspent *lastunspent;
};

struct iguana_RTtxid
{
    UT_hash_handle hh; struct iguana_info *coin; struct iguana_block *block;
    bits256 txid;
    int32_t height,txi,txn_count,numvouts,numvins,txlen;
    uint32_t locktime,version,timestamp;
    uint8_t *rawtxbytes;
    struct iguana_RTunspent **unspents;
    struct iguana_RTspend *spends[];
};

struct hashstr_item { UT_hash_handle hh; char address[40]; };

struct jumblr_pending { bits256 splittxid,txid; int32_t vout,ind; };

struct DEXcoin_info
{
    bits256 deposit_privkey,jumblr_privkey;
    struct iguana_info *coin;
    double btcprice,BTC2KMD,kmdprice,USD_average,DEXpending,maxbid,minask,avail,jumblravail;
    uint32_t lasttime,counter; int32_t numpending;
    char CMCname[32],symbol[16],depositaddr[64],KMDdepositaddr[64],KMDjumblraddr[64],jumblraddr[64];
    struct jumblr_pending *pending;
};

struct vin_signer { bits256 privkey; char coinaddr[64]; uint8_t siglen,sig[80],rmd160[20],pubkey[66]; };

struct vin_info
{
    struct iguana_msgvin vin; uint64_t amount; cJSON *extras; bits256 sigtxid;
    int32_t M,N,validmask,spendlen,type,p2shlen,numpubkeys,numsigs,height,hashtype,userdatalen,suppress_pubkeys,ignore_cltverr;
    uint32_t sequence,unspentind; struct vin_signer signers[16]; char coinaddr[65];
    uint8_t rmd160[20],spendscript[IGUANA_MAXSCRIPTSIZE],p2shscript[IGUANA_MAXSCRIPTSIZE],userdata[IGUANA_MAXSCRIPTSIZE];
};

struct bitcoin_unspent
{
    bits256 txid,privkeys[16]; uint64_t value; int32_t vout,spendlen,p2shlen,numpubkeys; uint32_t sequence;
    uint8_t addrtype,rmd160[20],pubkeys[16][65],spendscript[IGUANA_MAXSCRIPTSIZE],p2shscript[IGUANA_MAXSCRIPTSIZE];
};

struct bitcoin_spend
{
    char changeaddr[64]; uint8_t change160[20];
    int32_t numinputs;
    int64_t txfee,input_satoshis,satoshis,change;
    struct bitcoin_unspent inputs[];
};

struct iguana_outpoint { void *ptr; bits256 txid; int64_t value; uint32_t unspentind; int16_t hdrsi,vout,spendlen:15,isptr:1; uint8_t spendscript[512]; };

struct exchange_quote { uint64_t satoshis,orderid,offerNXT,exchangebits; double price,volume; uint32_t timestamp,val; };

struct _gfshare_ctx
{
    uint32_t sharecount,threshold,size,buffersize,allocsize;
    uint8_t sharenrs[255],buffer[];
};

struct basilisk_p2pitem
{
    struct queueitem DL;
    struct iguana_info *coin; struct iguana_peer *addr;
    uint32_t ipbits,datalen; char type[4];
    uint8_t data[];
};

struct basilisk_request
{
    uint32_t requestid,timestamp,quoteid,quotetime; // 0 to 15
    uint64_t srcamount,minamount; // 16 to 31
    bits256 srchash; // 32 to 63
    bits256 desthash;
    char src[8],dest[8];
    //char volatile_start,message[43];
    uint64_t destamount;
    int32_t optionhours,profitmargin;//,DEXselector,extraspace;
} PACKEDSTRUCT;

struct basilisk_relaystatus
{
    uint8_t pingdelay;
};

struct basilisk_relay
{
    bits256 pubkey; int32_t relayid,oldrelayid; uint32_t ipbits,lastping; uint8_t pubkey33[33];
    struct basilisk_request *requests; int32_t maxrequests,numrequests;
    struct basilisk_relaystatus direct,reported[BASILISK_MAXRELAYS];
};

#endif

