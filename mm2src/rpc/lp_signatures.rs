
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
//  LP_signatures.c
//  marketmaker
//

use common::{jbits256, lp, rpc_response, HyRes, CJSON};
use common::mm_ctx::MmArc;
use libc::c_char;
use rpc::v1::types::{H160 as H160Json, H264 as H264Json};
use serde_json::{self as json, Value as Json};
use std::ffi::{CString};
use gstuff::now_ms;

/*
struct basilisk_request *LP_requestinit(struct basilisk_request *rp,bits256 srchash,bits256 desthash,char *src,uint64_t srcsatoshis,char *dest,uint64_t destsatoshis,uint32_t timestamp,uint32_t quotetime,int32_t DEXselector,int32_t fillflag,int32_t gtcflag)
{
    struct basilisk_request R;
    memset(rp,0,sizeof(*rp));
    rp->srchash = srchash;
    rp->srcamount = srcsatoshis;
    rp->timestamp = timestamp;
    rp->DEXselector = DEXselector;
    safecopy(rp->src,src,sizeof(rp->src));
    safecopy(rp->dest,dest,sizeof(rp->dest));
    R = *rp;
    rp->requestid = basilisk_requestid(rp);
    rp->quotetime = quotetime;
    rp->desthash = desthash;
    rp->destamount = destsatoshis;
    rp->quoteid = basilisk_quoteid(rp);
    //printf("r.%u %u, q.%u %u: %s %.8f -> %s %.8f\n",rp->timestamp,rp->requestid,rp->quotetime,rp->quoteid,rp->src,dstr(rp->srcamount),rp->dest,dstr(rp->destamount));
    return(rp);
}

cJSON *LP_quotejson(struct LP_quoteinfo *qp)
{
    double price; char etomic[64],activesymbol[65]; cJSON *retjson = cJSON_CreateObject();
    if ( jobj(retjson,"gui") == 0 )
        jaddstr(retjson,"gui",qp->gui[0] != 0 ? qp->gui : LP_gui);
    jaddstr(retjson,"uuid",qp->uuidstr);
    if ( qp->maxprice != 0 )
        jaddnum(retjson,"maxprice",qp->maxprice);
    if ( qp->mpnet != 0 )
        jaddnum(retjson,"mpnet",qp->mpnet);
    if ( qp->gtc != 0 )
        jaddnum(retjson,"gtc",qp->gtc);
    if ( qp->fill != 0 )
        jaddnum(retjson,"fill",qp->fill);
    jaddnum(retjson,"aliceid",qp->aliceid);
    jaddnum(retjson,"tradeid",qp->tradeid);
    jaddstr(retjson,"base",qp->srccoin);
    if ( LP_etomicsymbol(activesymbol,etomic,qp->srccoin) != 0 )
        jaddstr(retjson,"bobtomic",etomic);
    jaddstr(retjson,"etomicsrc",qp->etomicsrc);
    jaddstr(retjson,"rel",qp->destcoin);
    if ( LP_etomicsymbol(activesymbol,etomic,qp->destcoin) != 0 )
        jaddstr(retjson,"alicetomic",etomic);
    jaddstr(retjson,"etomicdest",qp->etomicdest);
    if ( qp->coinaddr[0] != 0 )
        jaddstr(retjson,"address",qp->coinaddr);
    if ( qp->timestamp != 0 )
        jaddnum(retjson,"timestamp",qp->timestamp);
    if ( bits256_nonz(qp->srchash) != 0 )
        jaddbits256(retjson,"srchash",qp->srchash);
    if ( qp->txfee != 0 )
        jadd64bits(retjson,"txfee",qp->txfee);
    if ( qp->quotetime != 0 )
        jaddnum(retjson,"quotetime",qp->quotetime);
    if ( qp->satoshis != 0 )
        jadd64bits(retjson,"satoshis",qp->satoshis);
    if ( bits256_nonz(qp->desthash) != 0 )
        jaddbits256(retjson,"desthash",qp->desthash);
    if ( qp->desttxfee != 0 )
        jadd64bits(retjson,"desttxfee",qp->desttxfee);
    if ( qp->destsatoshis != 0 )
    {
        jadd64bits(retjson,"destsatoshis",qp->destsatoshis);
        if ( qp->satoshis != 0 )
        {
            price = (double)qp->destsatoshis / (qp->satoshis - qp->txfee);
            jaddnum(retjson,"price",price);
        }
    }
    if ( qp->R.requestid != 0 )
        jaddnum(retjson,"requestid",qp->R.requestid);
    if ( qp->R.quoteid != 0 )
        jaddnum(retjson,"quoteid",qp->R.quoteid);
    return(retjson);
}

int32_t LP_quoteparse(struct LP_quoteinfo *qp,cJSON *argjson)
{
    uint32_t rid,qid; char etomic[64],activesymbol[65],*etomicstr;
    memset(qp,0,sizeof(*qp));
    qp->maxprice = jdouble(argjson,"maxprice");
    qp->mpnet = juint(argjson,"mpnet");
    qp->gtc = juint(argjson,"gtc");
    qp->fill = juint(argjson,"fill");
    safecopy(qp->gui,LP_gui,sizeof(qp->gui));
    safecopy(qp->srccoin,jstr(argjson,"base"),sizeof(qp->srccoin));
    safecopy(qp->uuidstr,jstr(argjson,"uuid"),sizeof(qp->uuidstr));
#ifndef NOTETOMIC
    if ( LP_etomicsymbol(activesymbol,etomic,qp->srccoin) != 0 )
    {
        if ( (etomicstr= jstr(argjson,"bobtomic")) == 0 || compare_addresses(etomicstr,etomic) == 0 )
        {
            printf("etomic src mismatch (%s) vs (%s)\n",etomicstr!=0?etomicstr:"",etomic);
            return(-1);
        }
    }
#endif
    safecopy(qp->coinaddr,jstr(argjson,"address"),sizeof(qp->coinaddr));
    safecopy(qp->etomicsrc,jstr(argjson,"etomicsrc"),sizeof(qp->etomicsrc));
    safecopy(qp->destcoin,jstr(argjson,"rel"),sizeof(qp->destcoin));
#ifndef NOTETOMIC
    if ( LP_etomicsymbol(activesymbol,etomic,qp->destcoin) != 0 )
    {
        if ( (etomicstr= jstr(argjson,"alicetomic")) == 0 || compare_addresses(etomicstr,etomic) == 0 )
        {
            printf("etomic dest mismatch (%s) vs (%s)\n",etomicstr!=0?etomicstr:"",etomic);
            return(-1);
        }
    }
#endif
    safecopy(qp->destaddr,jstr(argjson,"destaddr"),sizeof(qp->destaddr));
    safecopy(qp->etomicdest,jstr(argjson,"etomicdest"),sizeof(qp->etomicdest));
    qp->aliceid = (uint64_t)juint(argjson,"aliceid");
    qp->tradeid = juint(argjson,"tradeid");
    qp->timestamp = juint(argjson,"timestamp");
    qp->quotetime = juint(argjson,"quotetime");
    qp->srchash = jbits256(argjson,"srchash");
    qp->desthash = jbits256(argjson,"desthash");
    qp->txfee = j64bits(argjson,"txfee");
    if ( (qp->satoshis= j64bits(argjson,"satoshis")) > qp->txfee && fabs(qp->maxprice) < SMALLVAL )
    {
        qp->maxprice = (double)qp->destsatoshis / (qp->satoshis - qp->txfee);
    }
    qp->destsatoshis = j64bits(argjson,"destsatoshis");
    qp->desttxfee = j64bits(argjson,"desttxfee");
    qp->R.requestid = juint(argjson,"requestid");
    qp->R.quoteid = juint(argjson,"quoteid");
    if ( qp->R.requestid == 0 )
    {
        rid = basilisk_requestid(&qp->R);
        if ( qp->R.requestid != 0 && qp->R.requestid != rid )
            printf("requestid.%u -> %u\n",qp->R.requestid,rid);
        qp->R.requestid = rid;
    }
    if ( qp->R.quoteid == 0 )
    {
        qid = basilisk_quoteid(&qp->R);
        if ( qp->R.quoteid != 0 && qp->R.quoteid != qid )
            printf("quoteid.%u -> %u\n",qp->R.quoteid,qid);
        qp->R.quoteid = qid;
    }
    return(0);
}

void LP_txfees(uint64_t *txfeep,uint64_t *desttxfeep,char *base,char *rel)
{
    *txfeep = LP_txfeecalc(LP_coinfind(base),0,0);
    *desttxfeep = LP_txfeecalc(LP_coinfind(rel),0,0);
    //printf("LP_txfees(%.8f %.8f)\n",dstr(*txfeep),dstr(*desttxfeep));
}

int32_t LP_quoteinfoinit(struct LP_quoteinfo *qp,struct LP_utxoinfo *utxo,char *destcoin,double price,uint64_t satoshis,uint64_t destsatoshis)
{
    memset(qp,0,sizeof(*qp));
    if ( qp->timestamp == 0 )
        qp->timestamp = (uint32_t)time(NULL);
    safecopy(qp->destcoin,destcoin,sizeof(qp->destcoin));
    LP_txfees(&qp->txfee,&qp->desttxfee,utxo->coin,qp->destcoin);
    qp->satoshis = satoshis;//(destsatoshis / price) + 0.49;
    qp->destsatoshis = destsatoshis;
    /*if ( qp->txfee >= qp->satoshis || qp->txfee >= utxo->deposit.value || utxo->deposit.value < LP_DEPOSITSATOSHIS(qp->satoshis) ) //utxo->iambob == 0 ||
    {
        printf("quoteinit error.(%d %d %d %d) %.8f vs %.8f\n",utxo->iambob == 0,qp->txfee >= qp->satoshis,qp->txfee >= utxo->deposit.value,utxo->deposit.value < LP_DEPOSITSATOSHIS(qp->satoshis),dstr(utxo->deposit.value),dstr(LP_DEPOSITSATOSHIS(qp->satoshis)));
        return(-1);
    }*/
    if ( qp->desttxfee >= qp->destsatoshis )
    {
        printf("quoteinit desttxfee %.8f < %.8f destsatoshis\n",dstr(qp->desttxfee),dstr(qp->destsatoshis));
        return(-2);
    }
    safecopy(qp->srccoin,utxo->coin,sizeof(qp->srccoin));
    safecopy(qp->coinaddr,utxo->coinaddr,sizeof(qp->coinaddr));
    qp->srchash = utxo->pubkey;
    return(0);
}

// stateless function
int32_t LP_quotedestinfo(struct LP_quoteinfo *qp,bits256 desthash,char *destaddr)
{
    qp->desthash = desthash;
    safecopy(qp->destaddr,destaddr,sizeof(qp->destaddr));
    return(0);
}

char *LP_quotereceived(struct LP_quoteinfo *qp)
{
    struct LP_cacheinfo *ptr; double price;
    //LP_quoteparse(&Q,argjson);
    price = (double)qp->destsatoshis / (qp->satoshis - qp->txfee);
    //if ( (ptr= LP_cacheadd(qp->srccoin,qp->destcoin,qp->txid,qp->vout,price,qp)) != 0 )
    //{
      //  ptr->Q = *qp;
        printf(">>>>>>>>>> received quote %s/%s %.8f\n",qp->srccoin,qp->destcoin,price);
        return(clonestr("{\"result\":\"updated\"}"));
    //} else return(clonestr("{\"error\":\"nullptr\"}"));
}

int32_t LP_bitcoinsig_add(cJSON *item,bits256 priv,uint8_t *pubsecp,bits256 sighash)
{
    static void *ctx; int32_t i,j,siglen; uint8_t pub33[33],sig[65]; char sigstr[256];
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    for (j=0; j<100; j++)
    {
        if ( (siglen= bitcoin_sign(ctx,"sigadd",sig,sighash,priv,1)) > 0 && siglen == 65 )
        {
            memset(pub33,0,33);
            if ( bitcoin_recoververify(ctx,"test",sig,sighash,pub33,0) == 0 && memcmp(pub33,pubsecp,33) == 0 )
            {
                init_hexbytes_noT(sigstr,sig,siglen);
                jaddstr(item,"sig",sigstr);
                return(siglen);
            }
            if ( 0 )
            {
                for (i=0; i<33; i++)
                    printf("%02x",pubsecp[i]);
                printf(" pubsecp -> ");
                for (i=0; i<33; i++)
                    printf("%02x",pub33[i]);
                printf(" mismatched recovered pubkey.%d of %d\n",j,100);
            }
        }
    }
    return(-1);
}

bits256 LP_price_sighash(uint32_t timestamp,uint8_t *pubsecp,bits256 pubkey,char *base,char *rel,uint64_t price64)
{
    uint8_t buf[sizeof(pubkey) + 33 + sizeof(uint64_t)*3 + sizeof(timestamp)]; uint64_t basebits,relbits; bits256 sighash;
    basebits = stringbits(base);
    relbits = stringbits(rel);
    memcpy(buf,pubkey.bytes,sizeof(pubkey));
    memcpy(&buf[sizeof(pubkey)],pubsecp,33);
    memcpy(&buf[sizeof(pubkey)+33],&price64,sizeof(price64));
    memcpy(&buf[sizeof(pubkey)+33+sizeof(price64)],&basebits,sizeof(basebits));
    memcpy(&buf[sizeof(pubkey)+33+sizeof(price64)+sizeof(basebits)],&relbits,sizeof(relbits));
    memcpy(&buf[sizeof(pubkey)+33+sizeof(price64)+sizeof(basebits)+sizeof(relbits)],&timestamp,sizeof(timestamp));
    vcalc_sha256(0,sighash.bytes,buf,sizeof(buf));
    return(sighash);
}

bits256 LP_pubkey_sighash(uint32_t timestamp,bits256 pubkey,uint8_t *rmd160,uint8_t *pubsecp)
{
    uint8_t buf[sizeof(pubkey) + 20 + 33 + sizeof(timestamp)]; bits256 sighash;
    memcpy(buf,pubkey.bytes,sizeof(pubkey));
    memcpy(&buf[sizeof(pubkey)],rmd160,20);
    memcpy(&buf[sizeof(pubkey)+20],pubsecp,33);
    memcpy(&buf[sizeof(pubkey)+20+33],&timestamp,sizeof(timestamp));
    vcalc_sha256(0,sighash.bytes,buf,sizeof(buf));
    return(sighash);
}

bits256 LP_utxos_sighash(uint32_t timestamp,uint8_t *pubsecp,bits256 pubkey,bits256 utxoshash)
{
    uint8_t buf[sizeof(pubkey)+sizeof(utxoshash)+33+sizeof(timestamp)]; bits256 sighash;
    memcpy(buf,pubkey.bytes,sizeof(pubkey));
    memcpy(&buf[sizeof(pubkey)],pubsecp,33);
    memcpy(&buf[sizeof(pubkey)+33],&timestamp,sizeof(timestamp));
    memcpy(&buf[sizeof(pubkey)+33+sizeof(timestamp)],utxoshash.bytes,sizeof(utxoshash));
    vcalc_sha256(0,sighash.bytes,buf,sizeof(buf));
    return(sighash);
}

int32_t LP_utxos_sigcheck(uint32_t timestamp,char *sigstr,char *pubsecpstr,bits256 pubkey,bits256 utxoshash)
{
    static void *ctx; int32_t retval=-1; uint8_t pub33[33],pubsecp[33],sig[65]; bits256 sighash; char str[65]; struct LP_pubkey_info *pubp;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    pubp = LP_pubkeyfind(pubkey);
    if ( (pubp == 0 || pubp->numerrors < LP_MAXPUBKEY_ERRORS) && sigstr != 0 && pubsecpstr != 0 && strlen(sigstr) == 65*2 && strlen(pubsecpstr) == 33 *2 )
    {
        decode_hex(sig,65,sigstr);
        decode_hex(pubsecp,33,pubsecpstr);
        sighash = LP_utxos_sighash(timestamp,pubsecp,pubkey,utxoshash);
        retval = bitcoin_recoververify(ctx,"utxos",sig,sighash,pub33,0);
        if ( memcmp(pub33,pubsecp,33) != 0 || retval != 0 )
        {
            static uint32_t counter;
            if ( counter++ <= LP_MAXPUBKEY_ERRORS )
            {
                if ( pubp != 0 )
                    pubp->numerrors++;
                if ( pubp != 0 && pubp->numerrors > LP_MAXPUBKEY_ERRORS/2 )
                    printf("LP_utxos_sigcheck failure.%d, probably from %s with older version\n",pubp!=0?pubp->numerrors:-1,bits256_str(str,pubkey));
            }
            retval = -1;
        } else retval = 0;
    }
    return(retval);
}

bits256 LP_utxoshash_calc(cJSON *array)
{
    int32_t i,j,n; bits256 utxoshash,txid; cJSON *item;
    memset(utxoshash.bytes,0,sizeof(utxoshash));
    if ( (n= cJSON_GetArraySize(array)) > 0 )
    {
        for (i=0; i<n; i++)
        {
            item = jitem(array,i);
            txid = jbits256(item,"tx_hash");
            for (j=0; j<4; j++)
                utxoshash.ulongs[j] ^= txid.ulongs[j];
            utxoshash.uints[0] ^= jint(item,"tx_pos");
        }
    }
    return(utxoshash);
}

int32_t LP_utxos_sigadd(cJSON *item,uint32_t timestamp,bits256 priv,uint8_t *pubsecp,bits256 pubkey,bits256 utxoshash)
{
    bits256 sighash;
    sighash = LP_utxos_sighash(timestamp,pubsecp,pubkey,utxoshash);
    return(LP_bitcoinsig_add(item,priv,pubsecp,sighash));
}

void LP_postutxos(char *symbol,char *coinaddr)
{
    bits256 zero; uint32_t timestamp; bits256 utxoshash; char pubsecpstr[67]; struct iguana_info *coin; cJSON *array,*reqjson = cJSON_CreateObject();
return;
    if ( (coin= LP_coinfind(symbol)) != 0 && (array= LP_address_utxos(coin,coinaddr,1)) != 0 )
    {
        //printf("LP_postutxos pubsock.%d %s %s\n",pubsock,symbol,coin->smartaddr);
        if ( cJSON_GetArraySize(array) == 0 )
            free_json(array);
        else
        {
            memset(zero.bytes,0,sizeof(zero));
            jaddstr(reqjson,"method","postutxos");
            jaddstr(reqjson,"coin",symbol);
            jaddstr(reqjson,"coinaddr",coinaddr);
            jadd(reqjson,"utxos",array);
            timestamp = (uint32_t)time(NULL);
            jaddnum(reqjson,"timetamp",timestamp);
            init_hexbytes_noT(pubsecpstr,G.LP_pubsecp,33);
            jaddstr(reqjson,"pubsecp",pubsecpstr);
            jaddbits256(reqjson,"pubkey",G.LP_mypub25519);
            utxoshash = LP_utxoshash_calc(array);
            //char str[65]; printf("utxoshash add %s\n",bits256_str(str,utxoshash));
            LP_utxos_sigadd(reqjson,timestamp,G.LP_privkey,G.LP_pubsecp,G.LP_mypub25519,utxoshash);
            //printf("post (%s) -> %d\n",msg,LP_mypubsock);
            LP_reserved_msg(0,symbol,symbol,zero,jprint(reqjson,1));
        }
    }
}

int32_t LP_price_sigcheck(uint32_t timestamp,char *sigstr,char *pubsecpstr,bits256 pubkey,char *base,char *rel,uint64_t price64)
{
    static void *ctx; int32_t retval=-1; uint8_t pub33[33],pubsecp[33],sig[65]; bits256 sighash; struct LP_pubkey_info *pubp;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    pubp = LP_pubkeyfind(pubkey);
    if ( (pubp == 0 || pubp->numerrors < LP_MAXPUBKEY_ERRORS) && sigstr != 0 && pubsecpstr != 0 && strlen(sigstr) == 65*2 && strlen(pubsecpstr) == 33 *2 )
    {
        decode_hex(sig,65,sigstr);
        decode_hex(pubsecp,33,pubsecpstr);
        sighash = LP_price_sighash(timestamp,pubsecp,pubkey,base,rel,price64);
        retval = bitcoin_recoververify(ctx,"price",sig,sighash,pub33,0);
        if ( memcmp(pub33,pubsecp,33) != 0 || retval != 0 )
        {
            if ( pubp != 0 )
                pubp->numerrors++;
            printf("LP_price_sigcheck failure\n");
            retval = -1;
        }
        else
        {
            retval = 0;
            //printf("valid price sig %s/%s %.8f\n",base,rel,dstr(price64));
        }
    }
    return(retval);
}

int32_t LP_price_sigadd(cJSON *item,uint32_t timestamp,bits256 priv,uint8_t *pubsecp,bits256 pubkey,char *base,char *rel,uint64_t price64)
{
    bits256 sighash;
    sighash = LP_price_sighash(timestamp,pubsecp,pubkey,base,rel,price64);
    return(LP_bitcoinsig_add(item,priv,pubsecp,sighash));
}

char *LP_pricepings(void *ctx,char *myipaddr,int32_t pubsock,char *base,char *rel,double price)
{
    struct iguana_info *basecoin,*relcoin,*kmd; struct LP_address *ap; char pubsecpstr[67]; uint32_t numutxos,timestamp; uint64_t price64,median,minsize,maxsize; bits256 zero; cJSON *reqjson;
    reqjson = cJSON_CreateObject();
    if ( (basecoin= LP_coinfind(base)) != 0 && (relcoin= LP_coinfind(rel)) != 0 )//&& basecoin->electrum == 0 )//&& relcoin->electrum == 0 )
    {
        memset(zero.bytes,0,sizeof(zero));
        jaddbits256(reqjson,"pubkey",G.LP_mypub25519);
        jaddstr(reqjson,"base",base);
        jaddstr(reqjson,"rel",rel);
        price64 = price * SATOSHIDEN + 0.0000000049;
        jaddnum(reqjson,"price",price);
        jadd64bits(reqjson,"price64",price64);
        jaddstr(reqjson,"method","postprice");
        timestamp = (uint32_t)time(NULL);
        jaddnum(reqjson,"timestamp",timestamp);
        init_hexbytes_noT(pubsecpstr,G.LP_pubsecp,33);
        jaddstr(reqjson,"pubsecp",pubsecpstr);
        if ( (kmd= LP_coinfind("KMD")) != 0 && (ap= LP_address(kmd,kmd->smartaddr)) != 0 && ap->instantdex_credits != 0 )
            jaddnum(reqjson,"credits",dstr(ap->instantdex_credits));
#ifndef NOTETOMIC
        if (basecoin->etomic[0] != 0) {
            int error = 0;
            uint64_t etomic_coin_balance = LP_etomic_get_balance(basecoin, basecoin->smartaddr, &error);
            jaddstr(reqjson,"utxocoin","ETH_OR_ERC20");
            jaddnum(reqjson,"bal",dstr(etomic_coin_balance));
            jaddnum(reqjson,"min",dstr(etomic_coin_balance));
            jaddnum(reqjson,"max",dstr(etomic_coin_balance));
            jaddnum(reqjson,"n",1);
        } else
#endif
        if ( (numutxos= LP_address_minmax(1,&median,&minsize,&maxsize,basecoin,basecoin->smartaddr)) != 0 )
        {
            //printf("send %s numutxos.%d median %.8f min %.8f max %.8f\n",base,numutxos,dstr(median),dstr(minsize),dstr(maxsize));
            jaddstr(reqjson,"utxocoin",base);
            jaddnum(reqjson,"n",numutxos);
            jaddnum(reqjson,"bal",dstr(median) * numutxos);
            jaddnum(reqjson,"min",dstr(minsize));
            jaddnum(reqjson,"max",dstr(maxsize));
        }
        LP_price_sigadd(reqjson,timestamp,G.LP_privkey,G.LP_pubsecp,G.LP_mypub25519,base,rel,price64);
        LP_reserved_msg(0,base,rel,zero,jprint(reqjson,1));
        return(clonestr("{\"result\":\"success\"}"));
    } else return(clonestr("{\"error\":\"electrum node cant post bob asks\"}"));
}

char *LP_postprice_recv(cJSON *argjson)
{
    bits256 pubkey; double price; uint8_t pubkey33[33]; char *base,*rel,*argstr,coinaddr[64];
    //printf("PRICE POSTED.(%s)\n",jprint(argjson,0));
    if ( (base= jstr(argjson,"base")) != 0 && (rel= jstr(argjson,"rel")) != 0 && (price= jdouble(argjson,"price")) > SMALLVAL )
    {
        pubkey = jbits256(argjson,"pubkey");
        if ( bits256_nonz(pubkey) != 0 )
        {
            if ( LP_price_sigcheck(juint(argjson,"timestamp"),jstr(argjson,"sig"),jstr(argjson,"pubsecp"),pubkey,base,rel,j64bits(argjson,"price64")) == 0 )
            {
                if ( IPC_ENDPOINT >= 0 )
                {
                    if ( (argstr= jprint(argjson,0)) != 0 )
                    {
                        LP_QUEUE_COMMAND(0,argstr,IPC_ENDPOINT,-1,0);
                        free(argstr);
                    }
                }
                //printf("call pricefeed update\n");
                LP_pricefeedupdate(pubkey,base,rel,price,jstr(argjson,"utxocoin"),jint(argjson,"n"),jdouble(argjson,"bal")*SATOSHIDEN,jdouble(argjson,"min")*SATOSHIDEN,jdouble(argjson,"max")*SATOSHIDEN,jdouble(argjson,"credits")*SATOSHIDEN);
                return(clonestr("{\"result\":\"success\"}"));
            }
            else
            {
                if ( jstr(argjson,"pubsecp") != 0 )
                {
                    static char lasterror[64];
                    decode_hex(pubkey33,33,jstr(argjson,"pubsecp"));
                    bitcoin_address("KMD",coinaddr,0,60,pubkey33,33);
                    if ( strcmp(coinaddr,lasterror) != 0 )
                    {
                        printf("sig failure.(%s) %s\n",jprint(argjson,0),coinaddr);
                        strcpy(lasterror,coinaddr);
                    }
                }
                return(clonestr("{\"error\":\"sig failure\"}"));
            }
        }
    }
    return(clonestr("{\"error\":\"missing fields in posted price\"}"));
}

int32_t _LP_pubkey_sigcheck(uint8_t *sig,int32_t siglen,uint32_t timestamp,bits256 pub,uint8_t *rmd160,uint8_t *pubsecp)
{
    static void *ctx; uint8_t pub33[33]; bits256 sighash;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    sighash = LP_pubkey_sighash(timestamp,pub,rmd160,pubsecp);
    return(bitcoin_recoververify(ctx,"pubkey",sig,sighash,pub33,0));
}

int32_t LP_pubkey_sigadd(cJSON *item,uint32_t timestamp,bits256 priv,bits256 pub,uint8_t *rmd160,uint8_t *pubsecp)
{
    bits256 sighash;
    sighash = LP_pubkey_sighash(timestamp,pub,rmd160,pubsecp);
    return(LP_bitcoinsig_add(item,priv,pubsecp,sighash));
}

int32_t LP_pubkey_sigcheck(struct LP_pubkey_info *pubp,cJSON *item)
{
    int32_t i,len,siglen,retval=-1; uint8_t rmd160[20],checkrmd160[20],pubsecp[33],sig[65],zeroes[20]; char *sigstr,*hexstr,*pubsecpstr;
    if ( (hexstr= jstr(item,"rmd160")) != 0 && strlen(hexstr) == 2*sizeof(rmd160) )
    {
        decode_hex(rmd160,sizeof(rmd160),hexstr);
        memset(zeroes,0,sizeof(zeroes));
        if ( memcmp(zeroes,rmd160,sizeof(rmd160)) != 0 )
        {
            //if ( memcmp(rmd160,pubp->rmd160,20) != 0 )
            {
                if ( (pubsecpstr= jstr(item,"pubsecp")) != 0 && is_hexstr(pubsecpstr,0) == 66 )
                {
                    decode_hex(pubsecp,sizeof(pubsecp),pubsecpstr);
                    calc_rmd160_sha256(checkrmd160,pubsecp,33);
                    if ( memcmp(checkrmd160,rmd160,20) == 0 )
                    {
                        if ( (sigstr= jstr(item,"sig")) != 0 && (len= is_hexstr(sigstr,0)) == 65*2 )
                        {
                            siglen = len >> 1;
                            decode_hex(sig,siglen,sigstr);
                            if ( _LP_pubkey_sigcheck(sig,siglen,juint(item,"timestamp"),pubp->pubkey,rmd160,pubsecp) == 0 )
                            {
                                if ( memcmp(rmd160,pubp->rmd160,20) != 0 )
                                {
                                    //for (i=0; i<20; i++)
                                    //    printf("%02x",pubp->rmd160[i]);
                                    memcpy(pubp->rmd160,rmd160,sizeof(pubp->rmd160));
                                    memcpy(pubp->pubsecp,pubsecp,sizeof(pubp->pubsecp));
                                    memcpy(pubp->sig,sig,sizeof(pubp->sig));
                                    pubp->siglen = siglen;
                                    //char str[65]; printf(" -> rmd160.(%s) for %s (%s) sig.%s\n",hexstr,bits256_str(str,pubp->pubkey),pubsecpstr,sigstr);
                                }
                                pubp->timestamp = juint(item,"timestamp");
                                retval = 0;
                            } else pubp->numerrors++;
                        }
                    }
                    else if ( 0 )
                    {
                        for (i=0; i<20; i++)
                            printf("%02x",rmd160[i]);
                        printf(" rmd160 vs ");
                        for (i=0; i<20; i++)
                            printf("%02x",checkrmd160[i]);
                        printf(" for %s\n",pubsecpstr);
                   }
                }
            }
        }
    }
    return(retval);
}

void LP_notify_pubkeys(void *ctx,int32_t pubsock)
{
    bits256 zero; uint32_t timestamp; char LPipaddr[64],secpstr[67]; cJSON *reqjson = cJSON_CreateObject();
    memset(zero.bytes,0,sizeof(zero));
    jaddstr(reqjson,"method","notify");
    jaddstr(reqjson,"rmd160",G.LP_myrmd160str);
    jaddbits256(reqjson,"pub",G.LP_mypub25519);
    init_hexbytes_noT(secpstr,G.LP_pubsecp,33);
    jaddstr(reqjson,"pubsecp",secpstr);
    timestamp = (uint32_t)time(NULL);
    jaddnum(reqjson,"timestamp",timestamp);
    LP_pubkey_sigadd(reqjson,timestamp,G.LP_privkey,G.LP_mypub25519,G.LP_myrmd160,G.LP_pubsecp);
    if ( IAMLP != 0 )
    {
        if ( LP_rarestpeer(LPipaddr) != 0 )
        {
            jaddstr(reqjson,"isLP",LPipaddr);
            if ( strcmp(LPipaddr,LP_myipaddr) == 0 )
                jaddnum(reqjson,"ismine",1);
            //printf("notify send isLP.%s ismine.%d\n",LPipaddr,strcmp(LPipaddr,LP_myipaddr) == 0);
        } else printf("no LPipaddr\n");
    }
    jaddnum(reqjson,"session",G.LP_sessionid);
    LP_reserved_msg(1,"","",zero,jprint(reqjson,1));
}

*/

pub fn lp_notify_recv (_ctx: MmArc, req: Json) -> HyRes {
    let pubk = try_h! (jbits256 (&req["pub"]));
    if pubk.nonz() {
        let c_json = {
            let mut req = try_h! (json::to_vec (&req));
            req.push (0);
            try_h! (CJSON::from_zero_terminated (req.as_ptr() as *const c_char))
        };

        let pubp = unsafe {lp::LP_pubkeyadd (pubk.into())};
        unsafe {lp::LP_pubkey_sigcheck (pubp, c_json.0)};
        let pub_secp: H264Json = try_h! (json::from_value(req["pubsecp"].clone()));
        let rmd160: H160Json = try_h! (json::from_value(req["rmd160"].clone()));
        unsafe { (*pubp).pubsecp = pub_secp.0 };
        unsafe { (*pubp).rmd160 = rmd160.0 };
        unsafe { (*pubp).timestamp = (now_ms() / 1000) as u32 };

        if let Some (peer_ip) = req["isLP"].as_str() {
            let peer_ip_c = try_h! (CString::new (peer_ip));
            let ismine = req["ismine"].as_i64().unwrap_or (0) as i32;
            unsafe {lp::LP_peer_recv (peer_ip_c.as_ptr() as *mut c_char, ismine, pubp)};
            // TODO: Figure out what kind of peers we're dealing with here (MM1, MM2, seeds, observers?)
            //       and whether we want them added into the friendlist (to further talk with them through the `peers`).
            //try_h! (peers::investigate_peer (&ctx, peer_ip, unsafe {lp::RPC_port + 20}));
            unsafe {lp::LP_addpeer (
                lp::LP_mypeer,
                lp::LP_mypubsock,
                peer_ip_c.as_ptr() as *mut c_char,
                lp::RPC_port,
                lp::RPC_port + 10,
                lp::RPC_port + 20,
                1,
                req["session"].as_u64().unwrap_or (0) as u32,
                lp::G.netid
            )};
        }
    }
    rpc_response (200, r#"{"result": "success", "notify": "received"}"#)
}

/*
void LP_smartutxos_push(struct iguana_info *coin)
{
    uint64_t value; bits256 zero,txid; int32_t i,vout,height,n; cJSON *array,*item,*req;
return;
    if ( coin->smartaddr[0] == 0 )
        return;
    //LP_notify_pubkeys(coin->ctx,LP_mypubsock);
    if ( (array= LP_address_utxos(coin,coin->smartaddr,1)) != 0 )
    {
        memset(zero.bytes,0,sizeof(zero));
        if ( (n= cJSON_GetArraySize(array)) > 0 )
        {
            //printf("PUSH %s %s\n",coin->symbol,coin->smartaddr);
            for (i=0; i<n; i++)
            {
                item = jitem(array,i);
                txid = jbits256(item,"tx_hash");
                vout = jint(item,"tx_pos");
                value = j64bits(item,"value");
                height = jint(item,"height");
                req = cJSON_CreateObject();
                jaddstr(req,"method","uitem");
                jaddstr(req,"coin",coin->symbol);
                jaddstr(req,"coinaddr",coin->smartaddr);
                jaddbits256(req,"txid",txid);
                jaddnum(req,"vout",vout);
                jaddnum(req,"ht",height);
                jadd64bits(req,"value",value);
                //printf("ADDR_UNSPENTS[] <- %s\n",jprint(req,0));
                LP_reserved_msg(0,"","",zero,jprint(req,1));
            }
        }
        free_json(array);
    }
}

void LP_listunspent_query(char *symbol,char *coinaddr)
{
    bits256 zero; cJSON *reqjson = cJSON_CreateObject();
    memset(zero.bytes,0,sizeof(zero));
    jaddstr(reqjson,"method","addr_unspents");
    jaddstr(reqjson,"coin",symbol);
    jaddstr(reqjson,"address",coinaddr);
    LP_reserved_msg(0,"","",zero,jprint(reqjson,1));
}

void LP_query(char *method,struct LP_quoteinfo *qp)
{
    cJSON *reqjson; bits256 zero; char *msg; struct iguana_info *coin; int32_t flag = 0;
    reqjson = LP_quotejson(qp);
    if ( bits256_nonz(qp->desthash) != 0 )
        flag = 1;
    jaddbits256(reqjson,"pubkey",qp->srchash);
    jaddstr(reqjson,"method",method);
    if ( jobj(reqjson,"timestamp") == 0 )
        jaddnum(reqjson,"timestamp",time(NULL));
    if ( strcmp(method,"connect") == 0 )
    {
        if ( (coin= LP_coinfind("KMD")) != 0 )
            jadd(reqjson,"proof",LP_instantdex_txids(0,coin->smartaddr));
    }
    msg = jprint(reqjson,1);
    {
        //printf("QUERY.(%s)\n",msg);
        if ( IPC_ENDPOINT >= 0 )
            LP_QUEUE_COMMAND(0,msg,IPC_ENDPOINT,-1,0);
        memset(&zero,0,sizeof(zero));
        LP_reserved_msg(1,qp->srccoin,qp->destcoin,zero,clonestr(msg));
        //if ( bits256_nonz(qp->srchash) != 0 )
        {
            sleep(1);
            LP_reserved_msg(1,qp->srccoin,qp->destcoin,qp->srchash,clonestr(msg));
        }
    }
    if ( strcmp(method,"connect") == 0 && qp->mpnet != 0 && qp->gtc == 0 )
        LP_mpnet_send(0,msg,1,qp->coinaddr);
    free(msg);
}
*/
