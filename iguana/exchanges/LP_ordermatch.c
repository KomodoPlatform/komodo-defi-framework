
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
//  LP_ordermatch.c
//  marketmaker
//

struct LP_quoteinfo LP_Alicequery,LP_Alicereserved;
double LP_Alicemaxprice;
bits256 LP_Alicedestpubkey,LP_bobs_reserved;
uint32_t Alice_expiration,Bob_expiration;

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
        LP_QUEUE_COMMAND(0,msg,IPC_ENDPOINT,-1,0);
        free(msg);
    }
}

uint64_t LP_txfeecalc(struct iguana_info *coin,uint64_t txfee,int32_t txlen)
{
    if ( coin != 0 )
    {
        txfee = coin->txfee;
        if ( txfee < LP_MIN_TXFEE )
            txfee = LP_MIN_TXFEE;
    }
    return(txfee);
}

double LP_quote_validate(struct LP_utxoinfo *autxo,struct LP_utxoinfo *butxo,struct LP_quoteinfo *qp,int32_t iambob)
{
    double qprice=0.; char str[65],srccoin[65],destcoin[65],bobtomic[64],alicetomic[64]; cJSON *txout; uint64_t txfee,desttxfee,srcvalue=0,srcvalue2=0,destvalue=0,destvalue2=0;
  //printf(">>>>>>> quote satoshis.(%.8f %.8f) %s %.8f -> %s %.8f\n",dstr(qp->satoshis),dstr(qp->destsatoshis),qp->srccoin,dstr(qp->satoshis),qp->destcoin,dstr(qp->destsatoshis));
    if ( butxo != 0 )
    {
        if ( strcmp(butxo->coinaddr,qp->coinaddr) != 0 )
        {
            printf("(%s) != (%s)\n",butxo->coinaddr,qp->coinaddr);
            return(-7);
        }
    }
    //printf("checked autxo and butxo\n");
    if ( iambob == 0 && autxo != 0 )
    {
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

void LP_abutxo_set(struct LP_utxoinfo *autxo,struct LP_utxoinfo *butxo,struct LP_quoteinfo *qp)
{
    if ( butxo != 0 )
    {
        memset(butxo,0,sizeof(*butxo));
        butxo->pubkey = qp->srchash;
        safecopy(butxo->coin,qp->srccoin,sizeof(butxo->coin));
        safecopy(butxo->coinaddr,qp->coinaddr,sizeof(butxo->coinaddr));
        //butxo->payment.value = qp->value;
        butxo->iambob = 1;
        //butxo->deposit.value = up2->U.value;
        butxo->swap_satoshis = qp->satoshis;
    }
    if ( autxo != 0 )
    {
        memset(autxo,0,sizeof(*autxo));
        autxo->pubkey = qp->desthash;
        safecopy(autxo->coin,qp->destcoin,sizeof(autxo->coin));
        safecopy(autxo->coinaddr,qp->destaddr,sizeof(autxo->coinaddr));
        //autxo->payment.value = qp->value;
        autxo->iambob = 0;
        //autxo->deposit.value = up2->U.value;
        autxo->swap_satoshis = qp->destsatoshis;
    }
}

uint64_t LP_basesatoshis(double relvolume,double price,uint64_t txfee,uint64_t desttxfee)
{
    //printf("basesatoshis %.8f (rel %.8f / price %.8f)\n",dstr(SATOSHIDEN * ((relvolume) / price) + 2*txfee),relvolume,price);
    if ( relvolume > dstr(desttxfee) && price > SMALLVAL )
        return(SATOSHIDEN * (relvolume / price) + 2*txfee);
    else return(0);
}

struct LP_utxoinfo *LP_address_myutxopair(struct LP_utxoinfo *butxo,int32_t iambob,struct LP_address_utxo **utxos,int32_t max,struct iguana_info *coin,char *coinaddr,uint64_t txfee,double relvolume,double price,uint64_t desttxfee)
{
    struct LP_address *ap; uint64_t fee,targetval,targetval2; int32_t m,mini; struct LP_address_utxo *up,*up2; double ratio;
    if ( coin->etomic[0] != 0 )
    {
        if ( (coin= LP_coinfind("ETOMIC")) != 0 )
            coinaddr = coin->smartaddr;
    }
    if ( coin == 0 )
        return(0);
    memset(butxo,0,sizeof(*butxo));
    if ( iambob != 0 )
    {
        if (strcmp(coin->symbol, "ETOMIC") == 0) {
            targetval = 100000000 + 3*txfee;
        } else {
            targetval = LP_basesatoshis(relvolume,price,txfee,desttxfee) + 3*txfee;
        }
        targetval2 = (targetval / 8) * 9 + 3*txfee;
        fee = txfee;
        ratio = LP_MINVOL;
    }
    else
    {
        if (strcmp(coin->symbol, "ETOMIC") == 0) {
            targetval = 100000000 + 3*desttxfee;
        } else {
            targetval = relvolume*SATOSHIDEN + 3*desttxfee;
        }
        targetval2 = (targetval / 777) + 3*desttxfee;
        fee = desttxfee;
        ratio = LP_MINCLIENTVOL;
    }
    printf("address_myutxopair couldnt find %s %s targets %.8f %.8f\n",coin->symbol,coinaddr,dstr(targetval),dstr(targetval2));
    return(0);
}

void LP_gtc_iteration(uint32_t ctx_h)
{
    struct LP_gtcorder *gtc,*tmp; struct LP_quoteinfo *qp; uint64_t destvalue,destvalue2; uint32_t oldest = 0;
    if ( Alice_expiration != 0 )
        return;
    DL_FOREACH_SAFE(GTCorders,gtc,tmp)
    {
        if ( gtc->cancelled == 0 && (oldest == 0 || gtc->pending < oldest) )
            oldest = gtc->pending;
    }
    DL_FOREACH_SAFE(GTCorders,gtc,tmp)
    {
        qp = &gtc->Q;
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
                LP_query("request",qp, ctx_h);
                LP_Alicequery = *qp, LP_Alicemaxprice = gtc->Q.maxprice, Alice_expiration = qp->timestamp + 2*LP_AUTOTRADE_TIMEOUT, LP_Alicedestpubkey = qp->srchash;
                char str[65]; printf("LP_gtc fill.%d gtc.%d %s/%s %.8f vol %.8f dest.(%s) maxprice %.8f etomicdest.(%s) uuid.%s fill.%d gtc.%d\n",qp->fill,qp->gtc,qp->srccoin,qp->destcoin,dstr(qp->satoshis),dstr(qp->destsatoshis),bits256_str(str,LP_Alicedestpubkey),gtc->Q.maxprice,qp->etomicdest,qp->uuidstr,qp->fill,qp->gtc);
                break;
            }
        }
    }
}

void LP_gtc_addorder(struct LP_quoteinfo *qp)
{
    struct LP_gtcorder *gtc;
    gtc = calloc(1,sizeof(*gtc));
    gtc->Q = *qp;
    gtc->pending = (uint32_t)time(NULL);
    portable_mutex_lock(&LP_gtcmutex);
    DL_APPEND(GTCorders,gtc);
    portable_mutex_unlock(&LP_gtcmutex);
}

char *LP_trade(struct LP_quoteinfo *qp,double maxprice,int32_t timeout,uint32_t tradeid,bits256 destpubkey,char *uuidstr, uint32_t ctx_h)
{
    qp->aliceid = (uint64_t)LP_rand();
    if ( (qp->tradeid= tradeid) == 0 )
        qp->tradeid = LP_rand();
    qp->srchash = destpubkey;
    strncpy(qp->uuidstr,uuidstr,sizeof(qp->uuidstr)-1);
    qp->maxprice = maxprice;
    qp->timestamp = (uint32_t)time(NULL);
    if ( qp->gtc != 0 )
    {
        strcpy(&qp->uuidstr[strlen(qp->uuidstr)-6],"cccccc");
        LP_gtc_addorder(qp);
    }
    {
        LP_query("request",qp, ctx_h);
        LP_Alicequery = *qp, LP_Alicemaxprice = qp->maxprice, Alice_expiration = qp->timestamp + timeout, LP_Alicedestpubkey = qp->srchash;
    }
    if ( qp->gtc == 0 )
    {
        cJSON *reqjson = LP_quotejson(qp);
        char *msg = jprint(reqjson,1);
        LP_mpnet_send(1,msg,1,0);
        free(msg);
    }
    char str[65]; printf("LP_trade mpnet.%d fill.%d gtc.%d %s/%s %.8f vol %.8f dest.(%s) maxprice %.8f etomicdest.(%s) uuid.%s fill.%d gtc.%d\n",qp->mpnet,qp->fill,qp->gtc,qp->srccoin,qp->destcoin,dstr(qp->satoshis),dstr(qp->destsatoshis),bits256_str(str,LP_Alicedestpubkey),qp->maxprice,qp->etomicdest,qp->uuidstr,qp->fill,qp->gtc);
    return(LP_recent_swaps(0,uuidstr));
}

int32_t LP_quotecmp(int32_t strictflag,struct LP_quoteinfo *qp,struct LP_quoteinfo *qp2)
{
    if ( bits256_nonz(LP_Alicedestpubkey) != 0 )
    {
        if (bits256_cmp(LP_Alicedestpubkey,qp->srchash) != 0 )
        {
            printf("reject quote from non-matching pubkey\n");
            return(-1);
        } else printf("dont reject quote from destpubkey\n");
    }
    if ( bits256_cmp(qp->desthash,qp2->desthash) == 0 && strcmp(qp->srccoin,qp2->srccoin) == 0 && strcmp(qp->destcoin,qp2->destcoin) == 0 && qp->destsatoshis == qp2->destsatoshis && qp->txfee >= qp2->txfee && qp->desttxfee == qp2->desttxfee )
    {
        if ( strictflag == 0 || (qp->aliceid == qp2->aliceid && qp->R.requestid == qp2->R.requestid && qp->R.quoteid == qp2->R.quoteid && qp->satoshis == qp2->satoshis && bits256_cmp(qp->srchash,qp2->srchash) == 0) )
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

int32_t LP_aliceonly(char *symbol)
{
    return(0);
}

double LP_trades_alicevalidate(struct LP_quoteinfo *qp)
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
    return(qprice);
}

double LP_trades_bobprice(double *bidp,double *askp,struct LP_quoteinfo *qp)
{
    double price; struct iguana_info *coin; char str[65];
    price = LP_myprice(1,bidp,askp,qp->srccoin,qp->destcoin);
    if ( (coin= LP_coinfind(qp->srccoin)) == 0 || price <= SMALLVAL || *askp <= SMALLVAL )
    {
        //printf("this node has no price for %s/%s\n",qp->srccoin,qp->destcoin);
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
    if ( butxo == 0 || bits256_nonz(butxo->payment.txid) == 0 || bits256_nonz(butxo->deposit.txid) == 0 || butxo->payment.vout < 0 || butxo->deposit.vout < 0 )
    {
        //char str[65],str2[65]; printf("couldnt find bob utxos for autxo %s/v%d %s/v%d %.8f -> %.8f\n",bits256_str(str,qp->txid),qp->vout,bits256_str(str2,qp->txid2),qp->vout2,dstr(qp->satoshis),dstr(qp->destsatoshis));
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

int32_t LP_trades_canceluuid(char *uuidstr)
{
    /*
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
     */
}

void gen_quote_uuid(char *result, char *base, char* rel) {
    uint8_t uuidhash[256]; bits256 hash; uint64_t millis; int32_t len = 0;
    memcpy(uuidhash,&G.LP_mypub25519,sizeof(bits256)), len += sizeof(bits256);
    millis = OS_milliseconds();
    memcpy(&uuidhash[len],&millis,sizeof(millis)), len += sizeof(millis);
    memcpy(&uuidhash[len],base,(int32_t)strlen(base)), len += (int32_t)strlen(base);
    memcpy(&uuidhash[len],rel,(int32_t)strlen(rel)), len += (int32_t)strlen(rel);
    vcalc_sha256(0,hash.bytes,uuidhash,len);
    bits256_str(result,hash);
}

char *LP_autobuy(int32_t fomoflag,char *base,char *rel,double maxprice,double relvolume,int32_t timeout,int32_t duration,char *gui,uint32_t nonce,bits256 destpubkey,uint32_t tradeid,int32_t fillflag,int32_t gtcflag, uint32_t ctx_h)
{
    uint64_t desttxfee,txfee; uint32_t lastnonce; int64_t bestsatoshis=0,destsatoshis; struct iguana_info *basecoin,*relcoin; struct LP_utxoinfo *autxo,B,A; struct LP_quoteinfo Q; bits256 pubkeys[100]; struct LP_address_utxo *utxos[4096]; int32_t num=0,maxiters=100,i,max=(int32_t)(sizeof(utxos)/sizeof(*utxos));
    basecoin = LP_coinfind(base);
    relcoin = LP_coinfind(rel);
    if ( gui == 0 )
        gui = "nogui";
    if ( basecoin == 0 || basecoin->inactive != 0 || relcoin == 0 || relcoin->inactive != 0 )
        return(clonestr("{\"error\":\"base or rel not found or inactive\"}"));
    if ( LP_aliceonly(base) > 0 )
        return(clonestr("{\"error\":\"GAME can only be alice coin\"}"));
    printf("LP_autobuy %s/%s price %.8f vol %.8f nonce %u\n",base,rel,maxprice,relvolume,nonce);
    if ( (lastnonce= LP_lastnonce) != 0 && nonce <= lastnonce )
    {
        printf("nonce.%u not bigger than lastnonce.%u\n",nonce,lastnonce);
        return(clonestr("{\"error\":\"invalid nonce\"}"));
    }
    LP_lastnonce = nonce;
    if ( duration <= 0 )
        duration = LP_ORDERBOOK_DURATION;
    if ( timeout <= 0 )
        timeout = LP_AUTOTRADE_TIMEOUT;
    if ( basecoin->electrum != 0 && relcoin->electrum != 0 )
    {
        if ( timeout < 2*LP_AUTOTRADE_TIMEOUT )
            timeout = 2*LP_AUTOTRADE_TIMEOUT;
    }
    else if ( basecoin->electrum != 0 || relcoin->electrum != 0 )
    {
        if ( timeout < 1.5*LP_AUTOTRADE_TIMEOUT )
            timeout = 1.5*LP_AUTOTRADE_TIMEOUT;
    }
    if ( time(NULL) < Alice_expiration )
    {
        cJSON *retjson = cJSON_CreateObject();
        jaddstr(retjson,"error","only one pending request at a time");
        jaddnum(retjson,"wait",Alice_expiration-time(NULL));
        return(jprint(retjson,1));
    } else LP_alicequery_clear();
    LP_txfees(&txfee,&desttxfee,base,rel);
    if ( txfee != 0 && txfee < 10000 )
        txfee = 10000;
    if ( desttxfee != 0 && desttxfee < 10000 )
        desttxfee = 10000;
    if ( fomoflag != 0 )
    {
        uint64_t median,minutxo,maxutxo;
        maxprice = 0.; // fomo -> price is 1. and needs to be set
        if ( maxutxo > 0 )
        {
            relvolume = MIN(relvolume,dstr(maxutxo) - dstr(desttxfee)*3);
            printf("maxutxo %.8f relvolume %.8f desttxfee %.8f\n",dstr(maxutxo),relvolume,dstr(desttxfee));
            maxprice = LP_fomoprice(base,rel,&relvolume);
            printf("fomoprice %.8f relvolume %.8f\n",maxprice,relvolume);
            if ( maxprice == 0. )
                return(clonestr("{\"error\":\"no orderbook entry found to handle request\"}"));
        } else printf("no utxo available\n");
    }
    if ( maxprice <= 0. || relvolume <= 0. || LP_priceinfofind(base) == 0 || LP_priceinfofind(rel) == 0 )
        return(clonestr("{\"error\":\"invalid parameter\"}"));
    if ( strcmp("BTC",rel) == 0 )
        maxprice *= 1.01;
    else maxprice *= 1.001;
    memset(pubkeys,0,sizeof(pubkeys));
    destsatoshis = SATOSHIDEN * relvolume + 2*desttxfee;
    autxo = 0;
    for (i=0; i<maxiters; i++)
    {
        memset(&A,0,sizeof(A));
        if ( (autxo= LP_address_myutxopair(&A,0,utxos,max,relcoin,relcoin->smartaddr,txfee,dstr(destsatoshis),maxprice,desttxfee)) != 0 )
            break;
        if ( fillflag != 0 )
        {
            return(clonestr("{\"error\":\"cant find a deposit that is big enough in size. make another deposit that is just a bit larger than what you want to trade\"}"));
        }
        destsatoshis *= 0.98;
        if ( destsatoshis < desttxfee*LP_MINSIZE_TXFEEMULT )
            break;
    }
    if ( destsatoshis < desttxfee*LP_MINSIZE_TXFEEMULT || i == maxiters )
    {
        return(clonestr("{\"error\":\"cant find a deposit that is close enough in size. make another deposit that is just a bit larger than what you want to trade\"}"));
    }
    printf("bestfit.[%d] selected alice (%.8f %.8f) for %.8f sats %.8f\n",i,dstr(autxo->payment.value),dstr(autxo->fee.value),dstr(destsatoshis),dstr(autxo->swap_satoshis));
    if ( destsatoshis - desttxfee < autxo->swap_satoshis )
    {
        destsatoshis -= desttxfee;
        autxo->swap_satoshis = destsatoshis;
        //printf("first path dest %.8f from %.8f\n",dstr(destsatoshis),dstr(autxo->swap_satoshis));
    }
    else if ( autxo->swap_satoshis - desttxfee < destsatoshis && relcoin->etomic[0] == 0)
    {
        autxo->swap_satoshis -= desttxfee;
        destsatoshis = autxo->swap_satoshis;
        printf("second path dest %.8f from %.8f\n",dstr(destsatoshis),dstr(autxo->swap_satoshis));
    }
    if ( relcoin->etomic[0] == 0 && (destsatoshis < (autxo->payment.value / LP_MINCLIENTVOL) || autxo->payment.value < desttxfee*LP_MINSIZE_TXFEEMULT ))
    {
        printf("destsatoshis %.8f vs utxo %.8f this would have triggered an quote error -13\n",dstr(destsatoshis),dstr(autxo->payment.value));
        return(clonestr("{\"error\":\"cant find a deposit that is close enough in size. make another deposit that is a bit larger than what you want to trade\"}"));
    }
    bestsatoshis = 1.001 * LP_basesatoshis(dstr(destsatoshis),maxprice,txfee,desttxfee);
    memset(&B,0,sizeof(B));
    strcpy(B.coin,base);
    if ( LP_quoteinfoinit(&Q,&B,rel,maxprice,bestsatoshis,destsatoshis) < 0 )
        return(clonestr("{\"error\":\"cant set ordermatch quote\"}"));
    if ( LP_quotedestinfo(&Q,G.LP_mypub25519,autxo->coinaddr) < 0 )
        return(clonestr("{\"error\":\"cant set ordermatch quote info\"}"));
    if ( relcoin->etomic[0] != 0 || basecoin->etomic[0] != 0 )
    {
        struct iguana_info *coin;
        if ( relcoin->etomic[0] != 0 )
            strcpy(Q.etomicdest,relcoin->smartaddr);
        else if (basecoin->etomic[0] != 0 )
        {
            strcpy(Q.etomicdest,basecoin->smartaddr);
            //printf("Q.etomicdest (%s)\n",Q.etomicdest);
        }
        if ( relcoin->etomic[0] != 0 )
        {
            if ((coin= LP_coinfind("ETOMIC")) != 0 )
                strcpy(Q.destaddr,coin->smartaddr);
            else return(clonestr("{\"error\":\"cant find ETOMIC\"}"));
        }
    }
    int32_t changed;
    Q.mpnet = G.mpnet;
    Q.fill = fillflag;
    Q.gtc = gtcflag;
    LP_mypriceset(0,&changed,rel,base,1. / maxprice);
    LP_mypriceset(0,&changed,base,rel,0.);
    char uuidstr[65];
    gen_quote_uuid(uuidstr, base, rel);
    return(LP_trade(&Q,maxprice,timeout,tradeid,destpubkey,uuidstr,ctx_h));
}
