
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
//  prices.rs
//  marketmaker
//

use common::{dstr, lp, rpc_response, rpc_err_response, HyRes, SATOSHIDEN, SMALLVAL};
use common::wio::slurp_req;
use common::mm_ctx::{MmArc, MmWeak};
use common::log::TagParam;
use coins::{lp_coinfind};
use futures::{self, Future, Async, Poll};
use futures::task::{self};
use gstuff::{now_float};
use http::{Request, StatusCode};
use http::header::CONTENT_TYPE;
use libc::{c_char};
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::fmt;
use std::iter::once;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use super::{default_pricing_provider, register_interest_in_coin_prices, PortfolioContext, InterestingCoins};
use url;

/*
struct LP_orderbookentry
{
    bits256 pubkey;
    double price;
    int64_t avesatoshis,maxsatoshis,depth,dynamictrust;
    uint32_t timestamp;
    int32_t numutxos;
    char coinaddr[64];
};

int32_t LP_numpriceinfos;

struct LP_cacheinfo
{
    UT_hash_handle hh;
    struct LP_quoteinfo Q;
    uint8_t key[sizeof(bits256)+sizeof(uint64_t)*2+sizeof(int32_t)];
    double price;
    uint32_t timestamp;
} *LP_cacheinfos;

void LP_priceinfos_clear()
{
    int32_t i; struct LP_priceinfo *pp;
    for (i=0; i<LP_numpriceinfos; i++)
    {
        pp = &LP_priceinfos[i];
        memset(pp->myprices,0,sizeof(pp->myprices));
        memset(pp->minprices,0,sizeof(pp->minprices));
        memset(pp->fixedprices,0,sizeof(pp->fixedprices));
        memset(pp->buymargins,0,sizeof(pp->buymargins));
        memset(pp->sellmargins,0,sizeof(pp->sellmargins));
        memset(pp->offsets,0,sizeof(pp->offsets));
        memset(pp->factors,0,sizeof(pp->factors));
    }
}

float LP_pubkey_price(int32_t *numutxosp,int64_t *avesatoshisp,int64_t *maxsatoshisp,struct LP_pubkey_info *pubp,uint32_t baseind,uint32_t relind)
{
    struct LP_pubkey_quote *pq,*tmp; int32_t scale; int64_t scale64;
    *numutxosp = 0;
    *avesatoshisp = *maxsatoshisp = 0;
    DL_FOREACH_SAFE(pubp->quotes,pq,tmp)
    {
        if ( baseind == pq->baseind && relind == pq->relind )
        {
            if ( (scale= pq->scale) == 0 )
                pq->scale = scale = 6;
            scale64 = 1;
            while ( scale > 0 )
            {
                scale64 *= 10;
                scale--;
            }
            *numutxosp = pq->numutxos;
            *avesatoshisp = pq->aveutxo * scale64;
            *maxsatoshisp = pq->maxutxo * scale64;
            return(pq->price);
        }
    }
    return(0);
}

void LP_pubkey_update(struct LP_pubkey_info *pubp,uint32_t baseind,uint32_t relind,float price,int64_t balance,char *utxocoin,int32_t numutxos,int64_t minutxo,int64_t maxutxo)
{
    struct LP_pubkey_quote *pq,*tmp; int64_t aveutxo,scale64,ave64,max64; int32_t scale;
    DL_FOREACH_SAFE(pubp->quotes,pq,tmp)
    {
        if ( baseind == pq->baseind && relind == pq->relind )
            break;
        pq = 0;
    }
    if ( pq == 0 )
    {
        pq = calloc(1,sizeof(*pq));
        pq->baseind = baseind;
        pq->relind = relind;
        pq->scale = 6; // millions of SATOSHIS, ie. 0.01
        DL_APPEND(pubp->quotes,pq); // already serialized as only path is via stats_JSON()
        //printf("create pubp quotes %d/%d\n",baseind,relind);
    }
//printf("%d/%d price %.8f balance %.8f %s num.%d min %.8f max %.8f\n",baseind,relind,price,dstr(balance),utxocoin,numutxos,dstr(minutxo),dstr(maxutxo));
    pq->price = price;
    if ( utxocoin != 0 && utxocoin[0] != 0 )
    {
        if ( (scale= pq->scale) == 0 )
            pq->scale = scale = 6;
        scale64 = 1;
        while ( scale > 0 )
        {
            scale64 *= 10;
            scale--;
        }
        if ( numutxos >= 256 )
            pq->numutxos = 255;
        else pq->numutxos = numutxos;
        aveutxo = (balance + (scale64>>1)) / numutxos;
        if ( (ave64= (aveutxo / scale64)) >= (1LL << 32) )
            ave64 = (1LL << 32) - 1;
        max64 = ((maxutxo + (scale64>>1)) / scale64);
        if ( max64 >= (1LL << 32) )
            max64 = (1LL << 32) - 1;
        pq->aveutxo = (uint32_t)ave64;
        pq->maxutxo = (uint32_t)max64;
        if ( 0 )
        {
            printf("price %.8f base.%s rel.%s utxocoin.%s balance %.8f numutxos.%u %u scale64 = %llu, ave %llu, ave32 %u (%llu) max32 %u (%llu)\n",price,LP_priceinfos[baseind].symbol,LP_priceinfos[relind].symbol,utxocoin,dstr(balance),numutxos,pq->numutxos,(long long)scale64,(long long)aveutxo,pq->aveutxo,(long long)pq->aveutxo * scale64,pq->maxutxo,(long long)pq->maxutxo * scale64);
            int64_t avesatoshis,maxsatoshis;
            price = LP_pubkey_price(&numutxos,&avesatoshis,&maxsatoshis,pubp,baseind,relind);
            printf("checkprice %.8f numutxos.%d ave %.8f max %.8f\n",price,numutxos,dstr(avesatoshis),dstr(maxsatoshis));
        }
    }
}

struct LP_priceinfo *LP_priceinfo(int32_t ind)
{
    if ( ind < 0 || ind >= LP_MAXPRICEINFOS )
        return(0);
    else return(&LP_priceinfos[ind]);
}

char *LP_priceinfostr(int32_t ind)
{
    if ( ind < 0 || ind >= LP_MAXPRICEINFOS )
        return("UNKNOWN");
    else return(LP_priceinfos[ind].symbol);
}

int32_t LP_pricevalid(double price)
{
    if ( price > SMALLVAL && isnan(price) == 0 && price < SATOSHIDEN )
        return(1);
    else return(0);
}

struct LP_priceinfo *LP_priceinfofind(char *symbol)
{
    int32_t i; struct LP_priceinfo *pp; uint64_t coinbits;
    if ( symbol == 0 || symbol[0] == 0 )
        return(0);
    if ( LP_numpriceinfos > 0 )
    {
        coinbits = stringbits(symbol);
        pp = LP_priceinfos;
        for (i=0; i<LP_numpriceinfos; i++,pp++)
            if ( pp->coinbits == coinbits )
                return(pp);
    }
    return(0);
}

int32_t LP_priceinfoind(char *symbol)
{
    struct LP_priceinfo *pp;
    if ( (pp= LP_priceinfofind(symbol)) != 0 )
        return(pp->ind);
    else return(-1);
}

struct LP_priceinfo *LP_priceinfoptr(int32_t *indp,char *base,char *rel)
{
    struct LP_priceinfo *basepp,*relpp;
    if ( (basepp= LP_priceinfofind(base)) != 0 && (relpp= LP_priceinfofind(rel)) != 0 )
    {
        *indp = relpp->ind;
        return(basepp);
    }
    else
    {
        *indp = -1;
        return(0);
    }
}

int32_t LP_cachekey(uint8_t *key,char *base,char *rel,bits256 txid,int32_t vout)
{
    uint64_t basebits,relbits; int32_t offset = 0;
    basebits = stringbits(base);
    relbits = stringbits(rel);
    memcpy(&key[offset],&basebits,sizeof(basebits)), offset += sizeof(basebits);
    memcpy(&key[offset],&relbits,sizeof(relbits)), offset += sizeof(relbits);
    memcpy(&key[offset],&txid,sizeof(txid)), offset += sizeof(txid);
    memcpy(&key[offset],&vout,sizeof(vout)), offset += sizeof(vout);
    return(offset);
}

struct LP_cacheinfo *LP_cachefind(char *base,char *rel,bits256 txid,int32_t vout)
{
    struct LP_cacheinfo *ptr=0; uint8_t key[sizeof(bits256)+sizeof(uint64_t)*2+sizeof(vout)];
    if ( base == 0 || rel == 0 )
        return(0);
    if ( LP_cachekey(key,base,rel,txid,vout) == sizeof(key) )
    {
        portable_mutex_lock(&LP_cachemutex);
        HASH_FIND(hh,LP_cacheinfos,key,sizeof(key),ptr);
        portable_mutex_unlock(&LP_cachemutex);
    } else printf("LP_cachefind keysize mismatch?\n");
    if ( 0 && ptr != 0 && ptr->timestamp != 0 && ptr->timestamp < time(NULL)-LP_CACHEDURATION )
    {
        printf("expire price %.8f\n",ptr->price);
        ptr->price = 0.;
        ptr->timestamp = 0;
        memset(&ptr->Q,0,sizeof(ptr->Q));
    }
    return(ptr);
}

struct LP_pubkey_info *LP_pubkey_rmd160find(uint8_t rmd160[20])
{
    struct LP_pubkey_info *pubp=0,*tmp;
    portable_mutex_lock(&LP_pubkeymutex);
    HASH_ITER(hh,LP_pubkeyinfos,pubp,tmp)
    {
        if ( memcmp(rmd160,pubp->rmd160,sizeof(pubp->rmd160)) == 0 )
            break;
        pubp = 0;
    }
    portable_mutex_unlock(&LP_pubkeymutex);
    return(pubp);
}

struct LP_address *_LP_addressfind(struct iguana_info *coin,char *coinaddr)
{
    uint8_t rmd160[20],addrtype; struct LP_address *ap; struct LP_pubkey_info *pubp;
    HASH_FIND(hh,coin->addresses,coinaddr,strlen(coinaddr),ap);
    if ( ap != 0 && bits256_nonz(ap->pubkey) == 0 )
    {
        bitcoin_addr2rmd160(coin->symbol,coin->taddr,&addrtype,rmd160,coinaddr);
        if ( (pubp= LP_pubkey_rmd160find(rmd160)) != 0 )
        {
            ap->pubkey = pubp->pubkey;
            memcpy(ap->pubsecp,pubp->pubsecp,sizeof(ap->pubsecp));
        }
    }
    return(ap);
}

struct LP_address *_LP_addressadd(struct iguana_info *coin,char *coinaddr)
{
    uint8_t rmd160[20],addrtype; struct LP_address *ap; struct LP_pubkey_info *pubp;
    ap = calloc(1,sizeof(*ap));
    safecopy(ap->coinaddr,coinaddr,sizeof(ap->coinaddr));
    bitcoin_addr2rmd160(coin->symbol,coin->taddr,&addrtype,rmd160,coinaddr);
    if ( (pubp= LP_pubkey_rmd160find(rmd160)) != 0 )
    {
        ap->pubkey = pubp->pubkey;
        memcpy(ap->pubsecp,pubp->pubsecp,sizeof(ap->pubsecp));
    }
    //printf("LP_ADDRESS %s ADD.(%s)\n",coin->symbol,coinaddr);
    HASH_ADD_KEYPTR(hh,coin->addresses,ap->coinaddr,strlen(ap->coinaddr),ap);
    return(ap);
}

struct LP_pubkey_info *LP_pubkeyfind(bits256 pubkey)
{
    struct LP_pubkey_info *pubp=0;
    portable_mutex_lock(&LP_pubkeymutex);
    HASH_FIND(hh,LP_pubkeyinfos,&pubkey,sizeof(pubkey),pubp);
    portable_mutex_unlock(&LP_pubkeymutex);
    return(pubp);
}

struct LP_pubkey_info *LP_pubkeyadd(bits256 pubkey)
{
    char str[65]; struct LP_pubkey_info *pubp=0;
    portable_mutex_lock(&LP_pubkeymutex);
    HASH_FIND(hh,LP_pubkeyinfos,&pubkey,sizeof(pubkey),pubp);
    if ( pubp == 0 )
    {
        pubp = calloc(1,sizeof(*pubp));
        pubp->pubkey = pubkey;
        pubp->pairsock = -1;
        if ( bits256_cmp(G.LP_mypub25519,pubkey) == 0 )
        {
            memcpy(pubp->rmd160,G.LP_myrmd160,sizeof(pubp->rmd160));
            memcpy(pubp->pubsecp,G.LP_pubsecp,sizeof(pubp->pubsecp));
        }
        HASH_ADD_KEYPTR(hh,LP_pubkeyinfos,&pubp->pubkey,sizeof(pubp->pubkey),pubp);
        HASH_FIND(hh,LP_pubkeyinfos,&pubkey,sizeof(pubkey),pubp);
        if ( pubp == 0 )
            printf("pubkeyadd find %s error after add\n",bits256_str(str,pubp->pubkey));
    }
    portable_mutex_unlock(&LP_pubkeymutex);
    return(pubp);
}

int32_t LP_pubkey_istrusted(bits256 pubkey)
{
    struct LP_pubkey_info *pubp;
    if ( (pubp= LP_pubkeyadd(pubkey)) != 0 )
        return(pubp->istrusted != 0);
    return(0);
}

char *LP_pubkey_trustset(bits256 pubkey,uint32_t trustval)
{
    struct LP_pubkey_info *pubp;
    if ( (pubp= LP_pubkeyadd(pubkey)) != 0 )
    {
        pubp->istrusted = trustval;
        return(clonestr("{\"result\":\"success\"}"));
    }
    return(clonestr("{\"error\":\"pubkey not found\"}"));
}

char *LP_pubkey_trusted()
{
    struct LP_pubkey_info *pubp,*tmp; cJSON *array = cJSON_CreateArray();
    HASH_ITER(hh,LP_pubkeyinfos,pubp,tmp)
    {
        if ( pubp->istrusted != 0 )
            jaddibits256(array,pubp->pubkey);
    }
    return(jprint(array,1));
}

int64_t LP_unspents_metric(struct iguana_info *coin,char *coinaddr)
{
    cJSON *array,*item; int32_t i,n; int64_t metric=0,total;
    //LP_listunspent_both(coin->symbol,coinaddr,0);
    if ( (array= LP_address_utxos(coin,coinaddr,1)) != 0 )
    {
        total = 0;
        if ( (n= cJSON_GetArraySize(array)) > 0 )
        {
            for (i=0; i<n; i++)
            {
                item = jitem(array,i);
                total += j64bits(item,"value");
            }
        }
        metric = _LP_unspents_metric(total,n);
        free_json(array);
    }
    return(metric);
}

cJSON *LP_pubkeyjson(struct LP_pubkey_info *pubp)
{
    int32_t baseid,relid,numutxos; int64_t avesatoshis,maxsatoshis; char *base,hexstr[67],hexstr2[67],sigstr[256]; double price; cJSON *item,*array,*obj;
    obj = cJSON_CreateObject();
    array = cJSON_CreateArray();
    for (baseid=0; baseid<LP_numpriceinfos; baseid++)
    {
        base = LP_priceinfos[baseid].symbol;
        for (relid=0; relid<LP_numpriceinfos; relid++)
        {
            price = LP_pubkey_price(&numutxos,&avesatoshis,&maxsatoshis,pubp,baseid,relid);//pubp->matrix[baseid][relid];
            if ( LP_pricevalid(price) > 0 )
            {
                item = cJSON_CreateArray();
                jaddistr(item,base);
                jaddistr(item,LP_priceinfos[relid].symbol);
                jaddinum(item,price);
                jaddi(array,item);
            }
        }
    }
    jaddbits256(obj,"pubkey",pubp->pubkey);
    init_hexbytes_noT(hexstr,pubp->rmd160,sizeof(pubp->rmd160));
    jaddstr(obj,"rmd160",hexstr);
    init_hexbytes_noT(hexstr2,pubp->pubsecp,sizeof(pubp->pubsecp));
    jaddstr(obj,"pubsecp",hexstr2);
    init_hexbytes_noT(sigstr,pubp->sig,pubp->siglen);
    jaddstr(obj,"sig",sigstr);
    jaddnum(obj,"timestamp",pubp->timestamp);
    jadd(obj,"asks",array);
    if ( pubp->istrusted != 0 )
        jaddnum(obj,"istrusted",pubp->istrusted);
    return(obj);
}

char *LP_prices()
{
    struct LP_pubkey_info *pubp,*tmp; cJSON *array = cJSON_CreateArray();
    HASH_ITER(hh,LP_pubkeyinfos,pubp,tmp)
    {
        jaddi(array,LP_pubkeyjson(pubp));
    }
    return(jprint(array,1));
}

double LP_pricecache(struct LP_quoteinfo *qp,char *base,char *rel,bits256 txid,int32_t vout)
{
    struct LP_cacheinfo *ptr;
    if ( (ptr= LP_cachefind(base,rel,txid,vout)) != 0 )
    {
        if ( qp != 0 )
            (*qp) = ptr->Q;
        if ( ptr->price == 0. && ptr->Q.satoshis > ptr->Q.txfee )
        {
            ptr->price = (double)ptr->Q.destsatoshis / (ptr->Q.satoshis - ptr->Q.txfee);
            if ( LP_pricevalid(ptr->price) <= 0 )
                ptr->price = 0.;
            printf("LP_pricecache: set %s/%s ptr->price %.8f\n",base,rel,ptr->price);
        }
        //printf(">>>>>>>>>> found %s/%s %.8f\n",base,rel,ptr->price);
        return(ptr->price);
    }
    //char str[65]; printf("cachemiss %s/%s %s/v%d\n",base,rel,bits256_str(str,txid),vout);
    return(0.);
}

void LP_priceinfoupdate(char *base,char *rel,double price)
{
    struct LP_priceinfo *basepp,*relpp;
    if ( LP_pricevalid(price) > 0 )
    {
        if ( (basepp= LP_priceinfofind(base)) != 0 && (relpp= LP_priceinfofind(rel)) != 0 )
        {
            dxblend(&basepp->relvals[relpp->ind],price,0.9);
            dxblend(&relpp->relvals[basepp->ind],1. / price,0.9);
            //basepp->relvals[relpp->ind] = price;
            //relpp->relvals[basepp->ind] = 1. / price;
        }
    }
}

double LP_myprice(int32_t iambob,double *bidp,double *askp,char *base,char *rel)
{
    struct LP_priceinfo *basepp,*relpp; double val;
    *bidp = *askp = 0.;
    if ( (basepp= LP_priceinfofind(base)) != 0 && (relpp= LP_priceinfofind(rel)) != 0 )
    {
        *askp = basepp->myprices[iambob][relpp->ind];
        if ( LP_pricevalid(*askp) > 0 )
        {
            val = relpp->myprices[iambob][basepp->ind];
            if ( LP_pricevalid(val) > 0 )
            {
                *bidp = 1. / val;
                return((*askp + *bidp) * 0.5);
            }
            else
            {
                *bidp = 0.;
                return(*askp);
            }
        }
        else
        {
            val = relpp->myprices[iambob][basepp->ind];
            if ( LP_pricevalid(val) > 0 )
            {
                *bidp = 1. / val;
                *askp = 0.;
                return(*bidp);
            }
        }
    }
    return(0.);
}

int32_t LP_mypriceset(int32_t iambob,int32_t *changedp,char *base,char *rel,double price)
{
    struct LP_priceinfo *basepp=0,*relpp=0; struct LP_pubkey_info *pubp; double minprice,maxprice,margin,buymargin,sellmargin;
    *changedp = 0;
    //if ( strcmp("DEX",base) == 0 || strcmp("DEX",rel) == 0 )
    //    printf("%s/%s setprice %.8f\n",base,rel,price);
    if ( base != 0 && rel != 0 && (basepp= LP_priceinfofind(base)) != 0 && (relpp= LP_priceinfofind(rel)) != 0 )
    {
        
        if ( price == 0. || fabs(basepp->myprices[iambob][relpp->ind] - price)/price > 0.001 )
            *changedp = 1;
        if ( iambob != 0 )
        {
            sellmargin = relpp->sellmargins[basepp->ind];
            buymargin = relpp->buymargins[basepp->ind];
            margin = (sellmargin + buymargin) * 0.5;
            if ( price == 0. )
            {
                relpp->minprices[basepp->ind] = 0.;
                relpp->fixedprices[basepp->ind] = 0.;
                relpp->buymargins[basepp->ind] = 0.;
                relpp->sellmargins[basepp->ind] = 0.;
                relpp->offsets[basepp->ind] = 0.;
                relpp->factors[basepp->ind] = 0.;
                LP_autoref_clear(base,rel);
                margin = 0.;
            }
            else if ( (minprice= basepp->minprices[relpp->ind]) > SMALLVAL && price < minprice )
            {
                //printf("%s/%s price %.8f less than minprice %.8f\n",base,rel,price,minprice);
                price = minprice * (1. - margin);
            }
            else if ( (maxprice= relpp->minprices[basepp->ind]) > SMALLVAL )
            {
                if ( price > (1. / maxprice) )
                {
                    //printf("%s/%s price %.8f less than maxprice %.8f, more than %.8f\n",base,rel,price,maxprice,1./maxprice);
                    price = (1. / maxprice) * (1. + margin);
                }
            }
        }
        /*else if ( basepp->myprices[relpp->ind] > SMALLVAL )
        {
            price = (basepp->myprices[relpp->ind] * 0.9) + (0.1 * price);
        }*/
        basepp->myprices[iambob][relpp->ind] = price;          // ask
        //printf("LP_mypriceset base.%s rel.%s <- price %.8f\n",base,rel,price);
        //relpp->myprices[basepp->ind] = (1. / price);   // bid, but best to do one dir at a time
        if ( iambob != 0 && (pubp= LP_pubkeyadd(G.LP_mypub25519)) != 0 )
        {
            pubp->timestamp = (uint32_t)time(NULL);
            LP_pubkey_update(pubp,basepp->ind,relpp->ind,price,0,0,0,0,0);
            //pubp->matrix[basepp->ind][relpp->ind] = price;
            //pubp->timestamps[basepp->ind][relpp->ind] = pubp->timestamp;
            //pubp->matrix[relpp->ind][basepp->ind] = (1. / price);
        }
        return(0);
    }
    printf("base.%s rel.%s %p %p price %.8f error case\n",base!=0?base:"",rel!=0?rel:"",basepp,relpp,price);
    return(-1);
}

double LP_price(int32_t iambob,char *base,char *rel)
{
    struct LP_priceinfo *basepp; int32_t relind; double price = 0.;
    if ( (basepp= LP_priceinfoptr(&relind,base,rel)) != 0 )
    {
        if ( (price= basepp->myprices[iambob][relind]) == 0. )
        {
            price = basepp->relvals[relind];
        }
    }
    return(price);
}

double LP_getmyprice(int32_t iambob,char *base,char *rel)
{
    struct LP_priceinfo *basepp; int32_t relind; double price = 0.;
    if ( (basepp= LP_priceinfoptr(&relind,base,rel)) != 0 )
    {
        if ( (price= basepp->myprices[iambob][relind]) == 0. )
        {
        }
    }
    return(price);
}

cJSON *LP_priceinfomatrix(int32_t iambob,int32_t usemyprices)
{
    int32_t i,j,n,m; double total,sum,val; struct LP_priceinfo *pp; uint32_t now; struct LP_cacheinfo *ptr,*tmp; cJSON *vectorjson = cJSON_CreateObject();
    now = (uint32_t)time(NULL);
    HASH_ITER(hh,LP_cacheinfos,ptr,tmp)
    {
        if ( ptr->timestamp < now-3600*2 || ptr->price == 0. )
            continue;
        LP_priceinfoupdate(ptr->Q.srccoin,ptr->Q.destcoin,ptr->price);
    }
    pp = LP_priceinfos;
    total = m = 0;
    for (i=0; i<LP_numpriceinfos; i++,pp++)
    {
        pp->diagval = sum = n = 0;
        for (j=0; j<LP_numpriceinfos; j++)
        {
            if ( usemyprices == 0 || (val= pp->myprices[iambob][j]) == 0. )
                val = pp->relvals[j];
            if ( val > SMALLVAL )
            {
                sum += val;
                n++;
            }
        }
        if ( n > 0 )
        {
            pp->diagval = sum / n;
            total += pp->diagval, m++;
        }
    }
    if ( m > 0 )
    {
        pp = LP_priceinfos;
        for (i=0; i<LP_numpriceinfos; i++,pp++)
        {
            if ( pp->diagval > SMALLVAL )
            {
                pp->diagval /= total;
                jaddnum(vectorjson,pp->symbol,pp->diagval);
            }
        }
    }
    return(vectorjson);
}

struct LP_priceinfo *LP_priceinfoadd(char *symbol)
{
    struct LP_priceinfo *pp; cJSON *retjson;
    if ( symbol == 0 )
        return(0);
    if ( (pp= LP_priceinfofind(symbol)) != 0 )
    {
        printf("%s already there\n",symbol);
        return(pp);
    }
    if ( LP_numpriceinfos >= sizeof(LP_priceinfos)/sizeof(*LP_priceinfos) )
    {
        printf("cant add any more priceinfos than %d\n",LP_numpriceinfos);
        return(0);
    }
    pp = &LP_priceinfos[LP_numpriceinfos];
    memset(pp,0,sizeof(*pp));
    safecopy(pp->symbol,symbol,sizeof(pp->symbol));
    pp->coinbits = stringbits(symbol);
    pp->ind = LP_numpriceinfos++;
    //LP_numpriceinfos++;
    if ( (retjson= LP_priceinfomatrix(1,0)) != 0 )
        free_json(retjson);
    return(pp);
}

static int _cmp_orderbook(const void *a,const void *b)
{
    int32_t retval = 0;
#define ptr_a (*(struct LP_orderbookentry **)a)->price
#define ptr_b (*(struct LP_orderbookentry **)b)->price
    if ( ptr_b > ptr_a )
        retval = -1;
    else if ( ptr_b < ptr_a )
        retval = 1;
    else
    {
#undef ptr_a
#undef ptr_b
#define ptr_a ((struct LP_orderbookentry *)a)->maxsatoshis
#define ptr_b ((struct LP_orderbookentry *)b)->maxsatoshis
        if ( ptr_b > ptr_a )
            return(-1);
        else if ( ptr_b < ptr_a )
            return(1);
    }
    // printf("%.8f vs %.8f -> %d\n",ptr_a,ptr_b,retval);
    return(retval);
#undef ptr_a
#undef ptr_b
}

static int _revcmp_orderbook(const void *a,const void *b)
{
    int32_t retval = 0;
#define ptr_a (*(struct LP_orderbookentry **)a)->price
#define ptr_b (*(struct LP_orderbookentry **)b)->price
    if ( ptr_b > ptr_a )
        retval = 1;
    else if ( ptr_b < ptr_a )
        retval = -1;
    else
    {
#undef ptr_a
#undef ptr_b
#define ptr_a ((struct LP_orderbookentry *)a)->maxsatoshis
#define ptr_b ((struct LP_orderbookentry *)b)->maxsatoshis
        if ( ptr_b > ptr_a )
            return(-1);
        else if ( ptr_b < ptr_a )
            return(1);
    }
    // printf("%.8f vs %.8f -> %d\n",ptr_a,ptr_b,retval);
    return(retval);
#undef ptr_a
#undef ptr_b
}

cJSON *LP_orderbookjson(char *symbol,struct LP_orderbookentry *op)
{
    cJSON *item = cJSON_CreateObject();
    if ( LP_pricevalid(op->price) > 0 )
    {
        jaddstr(item,"coin",symbol);
        jaddstr(item,"address",op->coinaddr);
        jaddnum(item,"price",op->price);
        jaddnum(item,"numutxos",op->numutxos);
        jaddnum(item,"avevolume",dstr(op->avesatoshis));
        jaddnum(item,"maxvolume",dstr(op->maxsatoshis));
        jaddnum(item,"depth",dstr(op->depth));
        jaddbits256(item,"pubkey",op->pubkey);
        jaddnum(item,"age",time(NULL)-op->timestamp);
        jaddnum(item,"zcredits",dstr(op->dynamictrust));
    }
    return(item);
}

struct LP_orderbookentry *LP_orderbookentry(char *address,char *base,char *rel,double price,int32_t numutxos,int64_t avesatoshis,int64_t maxsatoshis,bits256 pubkey,uint32_t timestamp,int64_t balance,int64_t dynamictrust)
{
    struct LP_orderbookentry *op;
    if ( (op= calloc(1,sizeof(*op))) != 0 )
    {
        safecopy(op->coinaddr,address,sizeof(op->coinaddr));
        op->price = price;
        op->numutxos = numutxos;
        op->avesatoshis = avesatoshis;
        op->maxsatoshis = maxsatoshis;
        op->pubkey = pubkey;
        op->timestamp = timestamp;
        op->depth = balance;
        op->dynamictrust = dynamictrust;
    }
    return(op);
}

void LP_pubkeys_query()
{
    uint8_t zeroes[20]; bits256 zero; cJSON *reqjson; struct LP_pubkey_info *pubp=0,*tmp;
    memset(zero.bytes,0,sizeof(zero));
    memset(zeroes,0,sizeof(zeroes));
    HASH_ITER(hh,LP_pubkeyinfos,pubp,tmp)
    {
        if ( memcmp(zeroes,pubp->rmd160,sizeof(pubp->rmd160)) == 0 && time(NULL) > pubp->lasttime+60 )
        {
            pubp->lasttime = (uint32_t)time(NULL);
            reqjson = cJSON_CreateObject();
            jaddstr(reqjson,"method","wantnotify");
            jaddbits256(reqjson,"pub",pubp->pubkey);
            //printf("LP_pubkeys_query %s\n",jprint(reqjson,0));
            LP_reserved_msg(0,"","",zero,jprint(reqjson,1));
        }
    }
}

int32_t LP_orderbook_utxoentries(uint32_t now,int32_t polarity,char *base,char *rel,struct LP_orderbookentry *(**arrayp),int32_t num,int32_t cachednum,int32_t duration)
{
    char coinaddr[64]; uint8_t zeroes[20]; struct LP_pubkey_info *pubp=0,*tmp; struct LP_priceinfo *basepp; struct LP_orderbookentry *op; struct LP_address *ap; struct iguana_info *basecoin; uint32_t oldest; double price; int32_t baseid,relid,n; int64_t maxsatoshis,balance,avesatoshis;
    if ( (basepp= LP_priceinfoptr(&relid,base,rel)) != 0 )
        baseid = basepp->ind;
    else return(num);
    if ( (basecoin= LP_coinfind(base)) == 0 )
        return(-1);
    now = (uint32_t)time(NULL);
    oldest = now - duration;
    memset(zeroes,0,sizeof(zeroes));
    HASH_ITER(hh,LP_pubkeyinfos,pubp,tmp)
    {
        if ( memcmp(zeroes,pubp->rmd160,sizeof(pubp->rmd160)) == 0 )
        {
            //printf("skip pubp since no rmd160\n");
            continue;
        }
        if ( pubp->timestamp < oldest )
            continue;
        bitcoin_address(base,coinaddr,basecoin->taddr,basecoin->pubtype,pubp->pubsecp,33);
        avesatoshis = maxsatoshis = n = 0;
        ap = 0;
        if ( (price= LP_pubkey_price(&n,&avesatoshis,&maxsatoshis,pubp,baseid,relid)) > SMALLVAL ) //pubp->matrix[baseid][relid]) > SMALLVAL )//&& pubp->timestamps[baseid][relid] >= oldest )
        {
            balance = avesatoshis * n;
            //if ( (ap= LP_addressfind(basecoin,coinaddr)) != 0 )
            {
                //n = LP_address_minmax(&balance,&minsatoshis,&maxsatoshis,ap);
                if ( polarity > 0 )
                {
                    balance *= price;
                    avesatoshis *= price;
                    maxsatoshis *= price;
                }
                //printf("%s/%s %s n.%d ap->n.%d %.8f\n",base,rel,coinaddr,n,ap->n,dstr(ap->total));
            }
            if ( (op= LP_orderbookentry(coinaddr,base,rel,polarity > 0 ? price : 1./price,n,avesatoshis,maxsatoshis,pubp->pubkey,pubp->timestamp,balance,pubp->dynamictrust)) != 0 )
            {
                *arrayp = realloc(*arrayp,sizeof(*(*arrayp)) * (num+1));
                (*arrayp)[num++] = op;
            }
        }
        //printf("pubp.(%s) %.8f %p\n",coinaddr,price,ap);
    }
    return(num);
}

char *LP_orderbook(char *base,char *rel,int32_t duration)
{
    uint32_t now,i; int64_t depth,askdepth=0,biddepth=0; struct LP_priceinfo *basepp=0,*relpp=0; struct LP_orderbookentry **bids = 0,**asks = 0; cJSON *retjson,*array; struct iguana_info *basecoin,*relcoin; int32_t n,numbids=0,numasks=0,cachenumbids,cachenumasks,baseid,relid,suppress_prefetch=0;
    basecoin = LP_coinfind(base);
    relcoin = LP_coinfind(rel);
    if ( basecoin == 0 || relcoin == 0 )
        return(clonestr("{\"error\":\"base or rel not added\"}"));
    if ( (basepp= LP_priceinfofind(base)) == 0 || (relpp= LP_priceinfofind(rel)) == 0 )
        return(clonestr("{\"error\":\"base or rel not added\"}"));
    if ( duration <= 0 )
    {
        if ( duration < 0 )
            suppress_prefetch = 1;
        duration = LP_ORDERBOOK_DURATION;
    }
    //LP_pubkeys_query();
    baseid = basepp->ind;
    relid = relpp->ind;
    now = (uint32_t)time(NULL);
    basecoin->obooktime = now;
    relcoin->obooktime = now;
    cachenumbids = numbids, cachenumasks = numasks;
    //printf("start cache.(%d %d) numbids.%d numasks.%d\n",cachenumbids,cachenumasks,numbids,numasks);
    numasks = LP_orderbook_utxoentries(now,1,base,rel,&asks,numasks,cachenumasks,duration);
    numbids = LP_orderbook_utxoentries(now,-1,rel,base,&bids,numbids,cachenumbids,duration);
    retjson = cJSON_CreateObject();
    array = cJSON_CreateArray();
    if ( numbids > 1 )
    {
        qsort(bids,numbids,sizeof(*bids),_revcmp_orderbook);
        depth = 0;
        for (i=0; i<numbids; i++)
        {
            depth += bids[i]->depth;
            bids[i]->depth = depth;
        }
    }
    if ( numasks > 1 )
    {
        qsort(asks,numasks,sizeof(*asks),_cmp_orderbook);
        depth = 0;
        for (i=0; i<numasks; i++)
        {
            depth += asks[i]->depth;
            asks[i]->depth = depth;
        }
    }
    for (i=n=0; i<numbids; i++)
    {
        biddepth = bids[i]->depth;
        jaddi(array,LP_orderbookjson(rel,bids[i]));
        if ( suppress_prefetch == 0 && n < 3 && bids[i]->numutxos == 0 )
        {
            //printf("bid ping %s %s\n",rel,bids[i]->coinaddr);
            LP_address(relcoin,bids[i]->coinaddr);
            /*if ( 0 && relcoin->electrum == 0 )
            {
                LP_listunspent_issue(rel,bids[i]->coinaddr,0);
            //else if ( (tmpjson= LP_listunspent(rel,bids[i]->coinaddr)) != 0 )
            //    free_json(tmpjson);
                LP_listunspent_query(rel,bids[i]->coinaddr);
            }*/
            n++;
        }
        if ( i == 0 )
        {
            LP_priceinfoupdate(rel,base,1. / bids[i]->price);
            //printf("update %s/%s %.8f [%.8f]\n",rel,base,1./bids[i]->price,bids[i]->price);
        }
        free(bids[i]);
        bids[i] = 0;
    }
    if ( n > 0 && relcoin->lastmonitor > 3600 )
        relcoin->lastmonitor -= 3600;
    jadd(retjson,"bids",array);
    jaddnum(retjson,"numbids",numbids);
    jaddnum(retjson,"biddepth",dstr(biddepth));
    array = cJSON_CreateArray();
    for (i=n=0; i<numasks; i++)
    {
        askdepth = asks[i]->depth;
        jaddi(array,LP_orderbookjson(base,asks[i]));
        if ( suppress_prefetch == 0 && n < 3 && asks[i]->numutxos == 0 )
        {
            //printf("ask ping %s %s\n",base,asks[i]->coinaddr);
            LP_address(basecoin,asks[i]->coinaddr);
            /*if ( 0 && basecoin->electrum == 0 )
            {
                LP_listunspent_issue(base,asks[i]->coinaddr,0);
            //else if ( (tmpjson= LP_listunspent(base,asks[i]->coinaddr)) != 0 )
            //    free_json(tmpjson);
                LP_listunspent_query(base,asks[i]->coinaddr);
            }*/
            n++;
        }
        if ( i == 0 )
        {
            LP_priceinfoupdate(base,rel,asks[i]->price);
            //printf("update %s/%s %.8f [%.8f]\n",base,rel,asks[i]->price,1./asks[i]->price);
        }
        free(asks[i]);
        asks[i] = 0;
    }
    if ( n > 0 && basecoin->lastmonitor > 3600 )
        basecoin->lastmonitor -= 3600;
    jadd(retjson,"asks",array);
    jaddnum(retjson,"numasks",numasks);
    jaddnum(retjson,"askdepth",dstr(askdepth));
    jaddstr(retjson,"base",base);
    jaddstr(retjson,"rel",rel);
    jaddnum(retjson,"timestamp",now);
    jaddnum(retjson,"netid",G.netid);
    if ( bids != 0 )
        free(bids);
    if ( asks != 0 )
        free(asks);
    return(jprint(retjson,1));
}

double LP_fomoprice(char *base,char *rel,double *relvolumep)
{
    char *retstr; cJSON *retjson,*asks,*item; int32_t i,numasks; double maxvol=0.,relvolume,biggest,price,fomoprice = 0.;
    relvolume = *relvolumep;
    if ( (retstr= LP_orderbook(base,rel,0)) != 0 )
    {
        if ( (retjson= cJSON_Parse(retstr)) != 0 )
        {
            if ( (asks= jarray(&numasks,retjson,"asks")) != 0 && numasks > 0 )
            {
                for (i=0; i<numasks; i++)
                {
                    item = jitem(asks,i);
                    biggest = jdouble(item,"maxvolume");
                    price = jdouble(item,"price");
                    if ( biggest > maxvol )
                    {
                        maxvol = biggest;
                        fomoprice = price;
                    }
                    printf("fomoprice (%.8f) i.%d %.8f vol %.8f [max %.8f @ %.8f]\n",relvolume,i,price,biggest,maxvol,fomoprice);
                }
            }
            free_json(retjson);
        }
        free(retstr);
    }
    if ( maxvol > 0. && fomoprice > 0. )
    {
        if ( maxvol < relvolume )
            relvolume = maxvol * 0.98;
        fomoprice /= 0.95;
    } else fomoprice = 0.;
    *relvolumep = relvolume;
    return(fomoprice);
}

int64_t LP_KMDvalue(struct iguana_info *coin,int64_t balance)
{
    double price = 0.; int64_t KMDvalue=0;
    if ( balance != 0 )
    {
        if ( strcmp(coin->symbol,"KMD") == 0 )
            KMDvalue = balance;
        else
        {
            if ( (price= LP_price(1,coin->symbol,"KMD")) > SMALLVAL )
                KMDvalue = price * balance;
        }
    }
    return(KMDvalue);
}

void LP_priceupdate(char *base,char *rel,double price,double avebid,double aveask,double highbid,double lowask,double PAXPRICES[32])
{
    LP_priceinfoupdate(base,rel,price);
}

void LP_pricefname(char *fname,char *base,char *rel)
{
    sprintf(fname,"%s/PRICES/%s_%s",GLOBAL_DBDIR,base,rel);
    OS_compatible_path(fname);
}
*/

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy)]
pub enum PriceUnit {Bitcoin, UsDollar}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum PricingProvider {CoinGecko, CoinMarketCap (String)}

impl fmt::Display for PricingProvider {
    fn fmt (&self, ft: &mut fmt::Formatter) -> fmt::Result {
        let label = match *self {
            PricingProvider::CoinGecko => "CoinGecko",
            PricingProvider::CoinMarketCap (_) => "CoinMarketCap"
        };
        ft.write_str (label)
    }
}

/// Things like "komodo", "bitcoin-cash" or "litecoin" are kept there.  
/// The value is the one we're getting from the RPC API in "refbase".  
/// According to the examples in https://docs.komodoplatform.com/barterDEX/barterDEX-API.html the "refbase"
/// might be a lowercased coin name or it's ticker symbol (dash/DASH, litecoin/LTC, komodo/KMD).
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct CoinId (pub String);
impl CoinId {
    pub fn for_provider<'a, 'b> (&'a self, coins: &[Json], provider: &'b PricingProvider) -> Result<Cow<'static, str>, String> {
        match provider {
            PricingProvider::CoinGecko => Ok (self.0.clone().into()),
            PricingProvider::CoinMarketCap (_) => {
                let mut it = coins.iter();
                loop {
                    // Example of the command-line configuration we might be getting:
                    // https://github.com/atomiclabs/hyperdex/blob/1d4ed3234b482e769124725c7e979eef5cd72d24/app/marketmaker/supported-currencies.js#L12
                    // Note how it currently lacks the "name" field.
                    // We need this "name" field added into the "coins" configuration in order to be able to convert between the coin names and their ticker symbols.
                    let coin_conf = match it.next() {
                        Some (v) => v,
                        // Found no match in the `coins`.
                        None => break ERR! ("CoinId] Unknown coin: {} (Check the 'name' and 'coin' fields in the 'coins' configuration)", self.0)
                    };
                    let name = match coin_conf["name"].as_str() {Some (n) => n, None => continue};
                    let ticker_symbol = match coin_conf["coin"].as_str() {Some (n) => n, None => continue};
                    if name == self.0 || name.to_lowercase() == self.0  {break Ok (Cow::Owned (ticker_symbol.into()))}
                }
            }
        }
    }
    pub fn from_gecko (_coins: &[Json], label: &str) -> Result<CoinId, String> {
        Ok (CoinId (label.into()))
    }
    /// CoinMarketCap gives us both the ticker symbol and the CoinGecko-compatible "slug" in its reply.  
    /// The code in `lp_autoprice_iter` presently uses the coin names ("komodo", "bitcoin-cash", "litecoin") so we prefer to get these.  
    /// Given a bit of ambiguity coming with the unregulated ticker symbols and names we're trying to match with the `coins` first.
    pub fn from_cmc (coins: &[Json], ticker_symbol: &str, slug: &str) -> Result<CoinId, String> {
        for coin_conf in coins {
            let name = coin_conf["name"].as_str();
            if name == Some (slug) {
                // Exact match over the coin name.
                return Ok (CoinId (slug.into()))
            }
            if coin_conf["coin"].as_str() == Some (ticker_symbol) && name.is_some() {
                // Converting the CMC ticker symbol into our coin name.
                return Ok (CoinId (unwrap! (name) .into()))
            }
        }
        return Ok (CoinId (slug.into()))
    }
}

/// Prices we've fetched from an external pricing provider (CoinMarketCap, CoinGecko).
/// Note that there is a delay between updating `Coins` and getting new `ExternalPrices`.
#[derive(Clone, Debug)]
pub struct ExternalPrices {pub prices: HashMap<CoinId, f64>, pub at: f64}

/// Coins discovered so far. Shared with the external resource future, in order not to create new futures for every new coin.
#[derive(Debug)]
pub struct Coins {
    /// A map from the coin id to the last time we've see it used. The latter allows us to eventually clean the map.
    pub ids: Mutex<HashMap<CoinId, f64>>
}

mod cmc_reply {
    use std::collections::HashMap;

    pub type PriceUnit = String;
    pub type TickerSymbol = String;

    #[derive(Deserialize, Debug)]
    pub struct Status {
        /// Seems to match the HTTP status code.
        pub error_code: i32,
        pub error_message: Option<String>,
        pub elapsed: i32
    }

    #[derive(Deserialize, Debug)]
    pub struct Quote {
        pub price: f64
    }

    #[derive(Deserialize, Debug)]
    pub struct Currency {
        /// The lowercased name, like the one we're using with CoinGecko.
        pub slug: String,
        pub quote: HashMap<PriceUnit, Quote>
    }

    /// https://coinmarketcap.com/api/documentation/v1/#operation/getV1CryptocurrencyQuotesHistorical
    #[derive(Deserialize, Debug)]
    pub struct MarketQuotes {
        pub status: Status,
        #[serde(default)]  // NB: "default" helps if we're getting an error reply without the "data" field.
        pub data: HashMap<TickerSymbol, Currency>
    }
}

mod gecko_reply {
    #[derive(Deserialize, Debug)]
    pub struct CoinGecko<'a> {
        pub id: &'a str,
        pub symbol: &'a str,
        pub current_price: f64
    }
}

/// Load coin prices from CoinGecko or, if `cmc_key` is given, from CoinMarketCap.
/// 
/// NB: We're using the MM command-line configuration ("coins") to convert between the coin names and the ticker symbols,
/// meaning that the price loader futures are not reusable across the MM instances (the `MmWeak` argument hints at it).
pub fn lp_btcprice (ctx_weak: MmWeak, provider: &PricingProvider, unit: PriceUnit, coins: &Arc<Coins>) -> Box<dyn Future<Item=ExternalPrices, Error=String> + Send> {
    let coin_labels: Vec<String> = {
        let ctx = try_fus! (MmArc::from_weak (&ctx_weak) .ok_or ("Context expired"));
        let coins_conf = try_fus! (ctx.conf["coins"].as_array().ok_or ("No 'coins' array in configuration"));

        let coin_ids = try_fus! (coins.ids.lock());
        try_fus! (coin_ids.keys().map (|c| c.for_provider (coins_conf, provider) .map (|s| s.into_owned())) .collect())
    };

    let cmc_price_unit: cmc_reply::PriceUnit = match unit {PriceUnit::Bitcoin => "BTC", PriceUnit::UsDollar => "USD"} .into();
    let gecko_price_unit = match unit {PriceUnit::Bitcoin => "btc", PriceUnit::UsDollar => "usd"};

    let (request, curl_example) = match provider {
        PricingProvider::CoinMarketCap (ref cmc_key) => {
            let url = fomat! (
                "https://pro-api.coinmarketcap.com/v1/cryptocurrency/quotes/latest?symbol="
                for coin in coin_labels {(coin)} separated {','}
                "&convert=" (cmc_price_unit)
            );
            ( try_fus! (Request::builder().uri (&url) .header ("X-CMC_PRO_API_KEY", &cmc_key[..]) .body (Vec::new())),
              format! ("curl --header \"X-CMC_PRO_API_KEY: {}\" \"{}\"", cmc_key, url) )
        },
        PricingProvider::CoinGecko => {
            let mut params = url::form_urlencoded::Serializer::new (String::new());
            params.append_pair ("ids", &fomat! (for coin in coin_labels {(coin)} separated {','}));
            params.append_pair ("vs_currency", gecko_price_unit);
            let url = fomat! ("https://api.coingecko.com/api/v3/coins/markets?" (params.finish()));
            ( try_fus! (Request::builder().uri (&url) .body (Vec::new())),
              format! ("curl \"{}\"", url) )
        }
    };
    log! ({"lp_btcprice] Fetching prices, akin to\n$ {}", curl_example});

    let f = slurp_req (request);

    let provider = provider.clone();
    let f = f.then (move |r| -> Result<ExternalPrices, String> {
        let (status_code, headers, body) = try_s! (r);
        if status_code != StatusCode::OK {
            // See if we have an error message to show.
            match provider {
                PricingProvider::CoinMarketCap (_) => {
                    if let Ok (reply) = json::from_slice::<cmc_reply::MarketQuotes> (&body) {
                        if let Some (message) = reply.status.error_message {
                            return ERR! ("CMC error: {:?}, {}", status_code, message)
                }   }   }
                _ => ()
            };
            return ERR! ("status_code {:?}", status_code)
        }
        let ct = match headers.get (CONTENT_TYPE) {Some (ct) => ct, None => return ERR! ("No Content-Type")};
        let ct = try_s! (ct.to_str());
        if !ct.starts_with ("application/json") {return ERR! ("Content-Type not JSON: {}", ct)}

        let ctx = try_s! (MmArc::from_weak (&ctx_weak) .ok_or ("Context expired"));
        let coins_conf = try_s! (ctx.conf["coins"].as_array().ok_or ("No 'coins' array in configuration"));

        let mut prices: HashMap<CoinId, f64> = HashMap::new();
        match provider {
            PricingProvider::CoinMarketCap (_) => {
                let market_quotes: cmc_reply::MarketQuotes = match json::from_slice (&body) {
                    Ok (q) => q,
                    Err (err) => {
                        log! ("lp_btcprice] Error parsing the CoinMarketCap reply: " (err) "\n" (String::from_utf8_lossy (&body)));
                        return ERR! ("Error parsing the CoinMarketCap reply: {}", err)
                }   };
                for (ticker_symbol, currency) in market_quotes.data {
                    let coin_id = try_s! (CoinId::from_cmc (coins_conf, &ticker_symbol, &currency.slug));
                    if let Some (quote) = currency.quote.get (&cmc_price_unit) {
                        prices.insert (coin_id, quote.price);
                    } else {
                        log! ("lp_btcprice] CMC quote for " (ticker_symbol) " lacks the " (cmc_price_unit) " price unit\n" (String::from_utf8_lossy (&body)));
                        return ERR! ("CMC quote for {} lacks the {} price unit", ticker_symbol, cmc_price_unit)
            }   }   },
            PricingProvider::CoinGecko => {
                let reply: Vec<gecko_reply::CoinGecko> = match json::from_slice (&body) {
                    Ok (r) => r,
                    Err (err) => {
                        log! ("lp_btcprice] Can't parse the CoinGecko response: " (err) "\n" (String::from_utf8_lossy (&body)));
                        return ERR! ("Can't parse the CoinGecko response: {}", err)
                }   };
                for cg in reply {
                    let coin_id = try_s! (CoinId::from_gecko (coins_conf, cg.id));
                    prices.insert (coin_id, cg.current_price);
        }   }   }
        Ok (ExternalPrices {prices, at: now_float()})
    });

    Box::new (f)
}

#[derive(Clone, Deserialize, Debug)]
struct FundvalueHoldingReq {
    /// The name of the coin ("litecoin") or its ticker symbol ("KMD").
    coin: String,
    balance: f64
}

/// JSON structure passed to the "fundvalue" RPC call.  
/// cf. https://docs.komodoplatform.com/barterDEX/barterDEX-API.html#fundvalue
#[derive(Clone, Deserialize, Debug)]
struct FundvalueReq {
    /// The address (usually WIF) with some coins in it.
    address: Option<String>,
    #[serde(default)]
    holdings: Vec<FundvalueHoldingReq>,
    divisor: Option<f64>
}

#[allow(non_snake_case)]
#[derive(Clone, Deserialize, Debug, Default, Serialize)]
pub struct FundvalueHoldingRes {
    coin: String,
    balance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    KMD: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    BTC: Option<f64>
}

#[allow(non_snake_case)]
#[derive(Clone, Deserialize, Debug, Default, Serialize)]
pub struct FundvalueRes {
    #[serde(skip_serializing_if = "Option::is_none")]
    KMD_BTC: Option<f64>,
    KMDholdings: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    btc2kmd: Option<f64>,
    btcsum: f64,
    fundvalue: f64,
    holdings: Vec<FundvalueHoldingRes>,
    /// True if there are holdings with a zero balance.  
    /// (Triggers a special code path in `lp_autoprice_iter`).
    pub missing: u32,
    /// "success"
    result: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    divisor: Option<f64>,
    /// Used in `lp_autoprice_iter`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub NAV_KMD: Option<f64>,
    /// Used in `lp_autoprice_iter`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub NAV_BTC: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assetNAV_KMD: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assetNAV_BTC: Option<f64>
}

/// List the holdings, calculating the current price in BTC and KMD, summing things up.
/// 
/// NB: Besides the RPC it's also invoked from the `lp_autoprice_iter` to calculate the number of the `missing` holdings.
/// 
/// * `immediate` - Don't wait for the external pricing resources, returning a "no price source" error for any prices that aren't already available.
pub fn lp_fundvalue (ctx: MmArc, req: Json, immediate: bool) -> HyRes {
    match lp_coinfind (&ctx, "KMD") {
        Ok (Some (_)) => (),
        Ok (None) => return rpc_err_response (500, &fomat! ("KMD and BTC must be enabled to use fundvalue")),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind( KMD ): " (err)))
    };

    match lp_coinfind (&ctx, "BTC") {
        Ok (Some (_)) => (),
        Ok (None) => return rpc_err_response (500, &fomat! ("KMD and BTC must be enabled to use fundvalue")),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind( BTC ): " (err)))
    };

    let req: FundvalueReq = try_h! (json::from_value (req));

    // Combine the explicitly specified `holdings` and the coins that `LP_balances` finds in the `address`.

    let mut holdings: Vec<FundvalueHoldingReq> = Vec::new();

    if let Some (ref address) = req.address {
        let address = try_h! (CString::new (&address[..]));
        let balances = unsafe {lp::LP_balances (address.as_ptr() as *mut c_char)};
        let n = unsafe {lp::cJSON_GetArraySize (balances)};
        for i in 0..n {
            let it = unsafe {lp::jitem (balances, i)};
            let coin = unsafe {lp::jstr (it, b"coin\0".as_ptr() as *mut c_char)};
            let balance = unsafe {lp::jdouble (it, b"balance\0".as_ptr() as *mut c_char)};
            if coin != null_mut() {
                let coin = try_h! (unsafe {CStr::from_ptr (coin)} .to_str()) .into();
                holdings.push (FundvalueHoldingReq {coin, balance})
            }
        }
        unsafe {lp::free_json (balances)};
    }

    for en in &req.holdings {holdings.push (en.clone())}

    // Find all the coins that needs their price to be fetched from an external resource.

    let mut ext_price_coins = HashSet::default();
    let mut holdings_res: Vec<FundvalueHoldingRes> = Vec::new();
    let mut second_pass_holdings: Vec<FundvalueHoldingReq> = Vec::new();

    let mut fundvalue = 0;
    let mut kmd_holdings = 0.;

    for en in holdings {
        if en.balance <= SMALLVAL {continue}
        let coin = try_h! (lp_coinfind (&ctx, &en.coin));
        if let Some (coin) = coin {
            let kmd_value = unsafe {lp::LP_KMDvalue (coin.iguana_info(), (SATOSHIDEN as f64 * en.balance) as i64)};
            if kmd_value > 0 {
                log! ({"lp_fundvalue] LP_KMDvalue of '{}' is {}.", en.coin, kmd_value});
                holdings_res.push (FundvalueHoldingRes {
                    coin: en.coin.clone(),
                    balance: en.balance,
                    KMD: Some (dstr (kmd_value, 8)),
                    ..Default::default()
                });
                fundvalue += kmd_value;
                if en.coin == "KMD" {kmd_holdings += dstr (kmd_value, 8)}
                continue
            }
        }
        ext_price_coins.insert (CoinId (en.coin.clone()));
        second_pass_holdings.push (en)
    }

    ext_price_coins.insert (CoinId ("komodo".into()));

    // Wait for the prices to arrive from the net.

    let portfolio_ctx = try_h! (PortfolioContext::from_ctx (&ctx));

    struct WaitFut {
        provider: PricingProvider,
        ctx: MmArc,
        portfolio_ctx: Arc<PortfolioContext>,
        ext_price_coins: HashSet<CoinId>,
        first_register: Option<f64>,
        immediate: bool
    }
    let f = WaitFut {
        provider: try_h! (default_pricing_provider (&ctx)),
        ctx,
        portfolio_ctx,
        ext_price_coins,
        first_register: None,
        immediate
    };
    impl Future for WaitFut {
        type Item = ExternalPrices;
        type Error = String;
        fn poll (&mut self) -> Poll<ExternalPrices, String> {

            // See if we've got the prices.

            let status_tags: &[&dyn TagParam] = &[&"portfolio", &"fundvalue", &"ext-prices"];
            let ctx = self.ctx.clone();
            let mut status = self.ctx.log.claim_status (status_tags);

            {
                let price_resources = try_s! (self.portfolio_ctx.price_resources.lock());
                if let Some ((_coins, resource)) = price_resources.get (&(self.provider.clone(), PriceUnit::Bitcoin)) {
                    let prices: Option<ExternalPrices> = try_s! (resource.with_result (|result| {
                        let mut new_status_handle;
                        /// Returns the status, creating it if necessary.
                        macro_rules! status_cr {() => {if let Some (status) = status.as_mut() {status} else {
                            new_status_handle = ctx.log.status (status_tags, &fomat! (
                                "Waiting for prices (" for c in &self.ext_price_coins {(c.0)} separated {','} ") ..."
                            ));
                            &mut new_status_handle
                        }}}

                        match result {
                            Some (Ok (prices)) => {
                                // A price can be
                                // a) fetched;
                                // b) not fetched YET (at <= first_register);
                                // c) definitely not fetched (at > first_register).
                                // Stopping when only (a) and (c) remain, or when `immediate` is set.

                                let first_register = if let Some (t) = self.first_register {t} else {0.};

                                let present = self.ext_price_coins.iter().filter (|id| prices.prices.contains_key (id)) .count();

                                if present == self.ext_price_coins.len() {
                                    status.as_mut().map (|s| s.append (" Done."));
                                    return Ok (Some (prices.clone()))
                                } else if prices.at > first_register || self.immediate {
                                    status_cr!().append (&format! (" {} out of {} obtained.", present, self.ext_price_coins.len()));
                                    return Ok (Some (prices.clone()))
                                } else {
                                    status_cr!().detach().append (".")
                                }
                            },
                            Some (Err (err)) => {
                                status_cr!().detach().append (&format! (" Error: {}", err))
                            },
                            None => {
                                status_cr!().detach().append (".")
                            }
                        }
                        Ok (None)
                    }));
                    if let Some (prices) = prices {return Ok (Async::Ready (prices))}
                }
            }

            // Ask for the prices and wait for them.

            let f_task = task::current();
            let ext_price_coins: InterestingCoins = once (((self.provider.clone(), PriceUnit::Bitcoin), (self.ext_price_coins.clone(), vec! [f_task]))) .collect();

            try_s! (register_interest_in_coin_prices (&self.ctx, &self.portfolio_ctx, ext_price_coins));
            self.first_register = Some (now_float());

            Ok (Async::NotReady)
        }
    }

    // Perform the calculations.

    let f = f.then (move |ext_prices| {
        let ext_prices = try_h! (ext_prices);
        let mut missing = 0;
        let mut btcsum = 0.;

        for holding in second_pass_holdings {
            if holding.balance <= SMALLVAL {missing += 1; continue}

            if let Some (btcprice) = ext_prices.prices.get (&CoinId (holding.coin.clone())) {
                btcsum += btcprice * holding.balance;
                holdings_res.push (FundvalueHoldingRes {
                    coin: holding.coin,
                    balance: holding.balance,
                    BTC: Some (btcprice * holding.balance),
                    ..Default::default()
                })
            } else {
                holdings_res.push (FundvalueHoldingRes {
                    coin: holding.coin,
                    balance: holding.balance,
                    error: Some ("no price source".into()),
                    ..Default::default()
                })
            }
        }

        let mut kmd_btc = None;
        let mut btc2kmd = None;
        let mut nav_kmd = None;
        let mut nav_btc = None;
        let mut asset_nav_kmd = None;
        let mut asset_nav_btc = None;

        let btcprice = ext_prices.prices.get (&CoinId ("komodo".into()));
        if let Some (&btcprice) = btcprice {
            if btcsum != 0. {
                let mut num_kmd = btcsum / btcprice;
                fundvalue += (num_kmd * SATOSHIDEN as f64) as i64;
                kmd_btc = Some (btcprice);
                num_kmd += kmd_holdings as f64;
                btc2kmd = Some (num_kmd);

                if let Some (divisor) = req.divisor {
                    nav_kmd = Some (num_kmd / divisor);
                    nav_btc = Some ((btcsum + (kmd_holdings as f64 * btcprice)) / divisor);
                    //jaddnum(retjson,"NAV_USD",(usdprice * numKMD)/divisor);
                }
            } else if let Some (divisor) = req.divisor {
                let num_kmd = dstr (fundvalue, 8);
                asset_nav_kmd = Some (num_kmd / divisor);
                asset_nav_btc = Some ((btcprice * num_kmd) / divisor);
                //jaddnum(retjson,"assetNAV_USD",(usdprice * numKMD)/divisor);
            }
        }

        let res = FundvalueRes {
            KMD_BTC: kmd_btc,
            KMDholdings: kmd_holdings,
            btc2kmd,
            btcsum,
            fundvalue: dstr (fundvalue, 8),
            holdings: holdings_res,
            missing,
            result: "success".into(),

            divisor: req.divisor,
            NAV_KMD: nav_kmd,
            NAV_BTC: nav_btc,
            assetNAV_KMD: asset_nav_kmd,
            assetNAV_BTC: asset_nav_btc
        };
        rpc_response (200, try_h! (json::to_string (&res)))
    });

    Box::new (f)
}
