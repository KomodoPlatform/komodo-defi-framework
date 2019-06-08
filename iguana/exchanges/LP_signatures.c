
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
    jaddstr(retjson,"rel",qp->destcoin);
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
    safecopy(qp->coinaddr,jstr(argjson,"address"),sizeof(qp->coinaddr));
    safecopy(qp->destcoin,jstr(argjson,"rel"),sizeof(qp->destcoin));
    safecopy(qp->destaddr,jstr(argjson,"destaddr"),sizeof(qp->destaddr));
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
    return(0);
}

// stateless function
int32_t LP_quotedestinfo(struct LP_quoteinfo *qp,bits256 desthash,char *destaddr)
{
    qp->desthash = desthash;
    safecopy(qp->destaddr,destaddr,sizeof(qp->destaddr));
    return(0);
}

char *LP_bitcoinsig_str(bits256 priv,uint8_t *pubsecp,bits256 sighash)
{
    static void *ctx; int32_t i,j,siglen; uint8_t pub33[33],sig[65]; char *sigstr;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    for (j=0; j<100; j++) {
        if ((siglen = bitcoin_sign(ctx, "sigadd", sig, sighash, priv, 1)) > 0 && siglen == 65) {
            memset(pub33, 0, 33);
            if (bitcoin_recoververify(ctx, "test", sig, sighash, pub33, 0) == 0 && memcmp(pub33, pubsecp, 33) == 0) {
                sigstr = malloc((size_t) siglen * 2 + 1);
                init_hexbytes_noT(sigstr, sig, siglen);
                return (sigstr);
            }
        }
    }
    return(0);
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

char *LP_price_sig(uint32_t timestamp,bits256 priv,uint8_t *pubsecp,bits256 pubkey,char *base,char *rel,uint64_t price64)
{
    bits256 sighash;
    sighash = LP_price_sighash(timestamp,pubsecp,pubkey,base,rel,price64);
    return(LP_bitcoinsig_str(priv,pubsecp,sighash));
}

char *LP_pricepings(char *base,char *rel,double price)
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
        char *sig = LP_price_sig(timestamp,G.LP_privkey,G.LP_pubsecp,G.LP_mypub25519,base,rel,price64);
        if (sig != NULL) {
            jaddstr(reqjson, "sig", sig);
            free(sig);
        }
        LP_reserved_msg(0,zero,jprint(reqjson,1));
        return(clonestr("{\"result\":\"success\"}"));
    } else return(clonestr("{\"error\":\"electrum node cant post bob asks\"}"));
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
    char *sig = LP_bitcoinsig_str(priv,pubsecp,sighash);
    if (sig != NULL) {
        jaddstr(item,"sig",sig);
        free(sig);
    }
    return(1);
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
                }
            }
        }
    }
    return(retval);
}

void LP_notify_pubkeys(uint32_t ctx_h)
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
    broadcast_p2p_msg_for_c(zero,jprint(reqjson,1), ctx_h);
}

void LP_query(char *method,struct LP_quoteinfo *qp, uint32_t ctx_h)
{
    cJSON *reqjson; bits256 zero; char *msg; struct iguana_info *coin; int32_t flag = 0;
    reqjson = LP_quotejson(qp);
    jaddbits256(reqjson,"pubkey",qp->srchash);
    jaddstr(reqjson,"method",method);
    if ( jobj(reqjson,"timestamp") == 0 )
        jaddnum(reqjson,"timestamp",time(NULL));
    msg = jprint(reqjson,1);
        //printf("QUERY.(%s)\n",msg);
    if ( IPC_ENDPOINT >= 0 )
        LP_QUEUE_COMMAND(0,msg,IPC_ENDPOINT,-1,0);
    memset(&zero,0,sizeof(zero));
    // broadcast_p2p_msg(zero,clonestr(msg));
    broadcast_p2p_msg_for_c(qp->srchash,clonestr(msg), ctx_h);
    if ( strcmp(method,"connect") == 0 && qp->mpnet != 0 && qp->gtc == 0 )
        LP_mpnet_send(0,msg,1,qp->coinaddr);
    free(msg);
}
