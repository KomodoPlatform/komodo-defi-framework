
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
//  LP_utxo.c
//  marketmaker
//


struct LP_inuse_info
{
    bits256 txid,otherpub;
    uint32_t expiration;
    int32_t vout,ind;
} LP_inuse[1024];
int32_t LP_numinuse;

cJSON *LP_inuse_json()
{
    int32_t i; cJSON *item,*array; struct LP_inuse_info *lp;
    array = cJSON_CreateArray();
    for (i=0; i<LP_numinuse; i++)
    {
        lp = &LP_inuse[i];
        if ( lp->expiration != 0 )
        {
            item = cJSON_CreateObject();
            jaddnum(item,"expiration",lp->expiration);
            jaddbits256(item,"txid",lp->txid);
            jaddnum(item,"vout",lp->vout);
            if ( bits256_nonz(lp->otherpub) != 0 )
                jaddbits256(item,"otherpub",lp->otherpub);
            jaddi(array,item);
        }
    }
    return(array);
}

struct LP_inuse_info *_LP_inuse_find(bits256 txid,int32_t vout)
{
    int32_t i;
    if ( bits256_nonz(txid) != 0 )
    {
        for (i=0; i<LP_numinuse; i++)
            if ( vout == LP_inuse[i].vout && bits256_cmp(LP_inuse[i].txid,txid) == 0 )
                return(&LP_inuse[i]);
    }
    return(0);
}

int32_t _LP_inuse_delete(bits256 txid,int32_t vout)
{
    struct LP_inuse_info *lp; int32_t ind; char str[65];
    if ( (lp= _LP_inuse_find(txid,vout)) != 0 )
    {
        ind = lp->ind;
        if ( LP_numinuse > 0 )
            *lp = LP_inuse[--LP_numinuse];
        lp->ind = ind;
        memset(&LP_inuse[LP_numinuse],0,sizeof(struct LP_inuse_info));
        //printf("_LP_inuse_delete mark as free %s/v%d find.%p\n",bits256_str(str,txid),vout,_LP_inuse_find(txid,vout));
        for (ind=0; ind<LP_numinuse; ind++)
            if ( LP_inuse[ind].ind != ind )
                printf("ind.%d of %d: mismatched ind.%d\n",ind,LP_numinuse,LP_inuse[ind].ind);
    } else printf("_LP_inuse_delete couldnt find %s/v%d\n",bits256_str(str,txid),vout);
    return(-1);
}

struct LP_inuse_info *_LP_inuse_add(uint32_t expiration,bits256 otherpub,bits256 txid,int32_t vout)
{
    struct LP_inuse_info *lp; int32_t i,n,oldesti; uint32_t now,oldest;
    if ( LP_numinuse >= sizeof(LP_inuse)/sizeof(*LP_inuse) )
    {
        now = (uint32_t)time(NULL);
        n = 0;
        oldesti = -1;
        oldest = 0;
        for (i=0; i<sizeof(LP_inuse)/sizeof(*LP_inuse); i++)
        {
            lp = &LP_inuse[i];
            if ( now > lp->expiration )
                _LP_inuse_delete(lp->txid,lp->vout), n++;
            else if ( oldest == 0 || lp->expiration < oldest )
            {
                oldest = lp->expiration;
                oldesti = i;
            }
        }
        if ( n == 0 )
        {
            printf("_LP_inuse_add out of slots error, pick oldesti %d\n",oldesti);
            lp = &LP_inuse[oldesti];
            _LP_inuse_delete(lp->txid,lp->vout);
        } else printf("expired %d inuse slots\n",n);
    }
    if ( bits256_nonz(txid) != 0 )
    {
        if ( (lp= _LP_inuse_find(txid,vout)) == 0 )
        {
            lp = &LP_inuse[LP_numinuse];
            memset(lp,0,sizeof(*lp));
            lp->txid = txid;
            lp->vout = vout;
            lp->expiration = expiration;
            lp->otherpub = otherpub;
            lp->ind = LP_numinuse++;
        }
        else
        {
            if ( bits256_nonz(otherpub) != 0 )
                lp->otherpub = otherpub;
            //if ( expiration > lp->expiration || expiration == 0 )
                lp->expiration = expiration;
        }
        //char str[65]; printf("set inuse until %u lag.%d for %s/v%d\n",expiration,(int32_t)(expiration-(uint32_t)time(NULL)),bits256_str(str,txid),vout);
        return(lp);
    } else printf("_LP_inuse_add [%d] overflow\n",LP_numinuse);
    return(0);
}

int32_t LP_reservation_check(bits256 txid,int32_t vout,bits256 pubkey)
{
    struct LP_inuse_info *lp; int32_t retval = -1;
    if ( bits256_nonz(pubkey) != 0 )
    {
        char str[65],str2[65];
        portable_mutex_lock(&LP_inusemutex);
        if ( (lp= _LP_inuse_find(txid,vout)) != 0 )
        {
            if ( bits256_cmp(lp->otherpub,pubkey) == 0 )
                retval = 0;
            else printf("otherpub.%s != %s\n",bits256_str(str,lp->otherpub),bits256_str(str2,pubkey));
        } else printf("couldnt find %s/v%d\n",bits256_str(str,txid),vout);
        portable_mutex_unlock(&LP_inusemutex);
    } else printf("LP_reservation_check null pubkey\n");
    return(retval);
}

uint32_t LP_allocated(bits256 txid,int32_t vout)
{
    struct LP_inuse_info *lp; uint32_t now,duration = 0;
    now = (uint32_t)time(NULL);
    portable_mutex_lock(&LP_inusemutex);
    if ( (lp= _LP_inuse_find(txid,vout)) != 0 )
    {
        if ( lp->expiration != 0 && now < lp->expiration )
            duration = (lp->expiration - now);
    }
    portable_mutex_unlock(&LP_inusemutex);
    return(duration);
}

void LP_unavailableset(bits256 txid,int32_t vout,uint32_t expiration,bits256 otherpub)
{
    portable_mutex_lock(&LP_inusemutex);
    _LP_inuse_add(expiration,otherpub,txid,vout);
    portable_mutex_unlock(&LP_inusemutex);
}

void LP_availableset(bits256 txid,int32_t vout)
{
    portable_mutex_lock(&LP_inusemutex);
    _LP_inuse_delete(txid,vout);
    portable_mutex_unlock(&LP_inusemutex);
}

int32_t LP_maxvalue(uint64_t *values,int32_t n)
{
    int32_t i,maxi = -1; uint64_t maxval = 0;
    for (i=0; i<n; i++)
        if ( values[i] > maxval )
        {
            maxi = i;
            maxval = values[i];
        }
    return(maxi);
}

int32_t LP_nearestvalue(int32_t iambob,uint64_t *values,int32_t n,uint64_t targetval)
{
    int32_t i,mini = -1; int64_t dist; uint64_t mindist = (1 << 31);
    for (i=0; i<n; i++)
    {
        dist = (values[i] - targetval);
        if ( iambob != 0 && dist < 0 && -dist < values[i]/10 )
            dist = -dist;
        //printf("(%.8f %.8f %.8f).%d ",dstr(values[i]),dstr(dist),dstr(mindist),mini);
        if ( dist >= 0 && dist < mindist )
        {
            mini = i;
            mindist = dist;
        }
    }
    return(mini);
}

uint64_t LP_value_extract(cJSON *obj,int32_t addinterest,bits256 utxotxid)
{
    double val = 0.; uint64_t interest,value = 0; int32_t electrumflag;
    electrumflag = (jobj(obj,"tx_hash") != 0);
    if ( electrumflag == 0 )
    {
        if ( (val= jdouble(obj,"amount")) < SMALLVAL )
            val = jdouble(obj,"value");
        value = (val + 0.0000000049) * SATOSHIDEN;
    } else value = j64bits(obj,"value");
    return(value);
}

int32_t LP_destaddr(char *destaddr,cJSON *item)
{
    int32_t m,retval = -1; cJSON *addresses,*skey; char *addr;
    if ( (skey= jobj(item,"scriptPubKey")) != 0 && (addresses= jarray(&m,skey,"addresses")) != 0 )
    {
        item = jitem(addresses,0);
        if ( (addr= jstr(item,0)) != 0 )
        {
            safecopy(destaddr,addr,64);
            retval = 0;
        }
        //printf("item.(%s) -> dest.(%s)\n",jprint(item,0),destaddr);
    }
    return(retval);
}

int32_t LP_txdestaddr(char *destaddr,bits256 txid,int32_t vout,cJSON *txobj)
{
    int32_t n,retval = -1; cJSON *vouts;
    if ( (vouts= jarray(&n,txobj,"vout")) != 0 && vout < n )
        retval = LP_destaddr(destaddr,jitem(vouts,vout));
    return(retval);
}

struct LP_address *_LP_address(struct iguana_info *coin,char *coinaddr)
{
    struct LP_address *ap = 0;
    if ( (ap= _LP_addressfind(coin,coinaddr)) == 0 )
    {
        ap = _LP_addressadd(coin,coinaddr);
        //printf("LP_address %s %s\n",coin->symbol,coinaddr);
    }
    return(ap);
}

struct LP_address *LP_addressfind(struct iguana_info *coin,char *coinaddr)
{
    struct LP_address *ap = 0;
    if ( coin != 0 )
    {
        portable_mutex_lock(coin->_addrmutex);
        ap = _LP_addressfind(coin,coinaddr);
        portable_mutex_unlock(coin->_addrmutex);
    }
    return(ap);
}

struct LP_address *LP_address(struct iguana_info *coin,char *coinaddr)
{
    struct LP_address *ap = 0;
    if ( coin != 0 )
    {
        portable_mutex_lock(coin->_addrmutex);
        ap = _LP_address(coin,coinaddr);
        portable_mutex_unlock(coin->_addrmutex);
    }
    return(ap);
}

struct LP_address_utxo *LP_address_utxofind(struct iguana_info *coin,char *coinaddr,bits256 txid,int32_t vout)
{
    struct LP_address *ap; struct LP_address_utxo *up,*tmp;
    //printf("LP_address_utxofind %s add addr.%s\n",coin->symbol,coinaddr);
    if ( (ap= _LP_address(coin,coinaddr)) != 0 )
    {
        DL_FOREACH_SAFE(ap->utxos,up,tmp)
        {
            if ( vout == up->U.vout && bits256_cmp(up->U.txid,txid) == 0 )
                return(up);
        }
    }
    return(0);
}

void LP_mark_spent(char *symbol,bits256 txid,int32_t vout)
{
    struct iguana_info *coin; struct LP_transaction *tx; struct LP_address_utxo *up;
    if ( (coin= LP_coinfind(symbol)) != 0 )
    {
        if ( (tx= LP_transactionfind(coin,txid)) != 0 )
        {
            if ( vout < tx->numvouts )
            {
                tx->outpoints[vout].spendheight = 1;
                if ( (up= LP_address_utxofind(coin,tx->outpoints[vout].coinaddr,txid,vout)) != 0 )
                    up->spendheight = 1;
            }
        }
    }
}

cJSON *LP_address_balance(struct iguana_info *coin,char *coinaddr,int32_t electrumret)
{
    return(0);
}

cJSON *LP_balances(char *coinaddr)
{
    struct iguana_info *coin,*tmp; char address[64]; uint8_t taddr,addrtype,rmd160[20]; uint64_t balance,KMDvalue,sum = 0; cJSON *array,*item,*retjson;
    if ( coinaddr != 0 && coinaddr[0] == 't' && (coinaddr[1] == '1' || coinaddr[1] == '3') )
        taddr = 1;
    else taddr = 0;
    array = cJSON_CreateArray();
    HASH_ITER(hh,LP_coins,coin,tmp)
    {
        if ( coin->electrum != 0 || (coinaddr != 0 && coinaddr[0] != 0 && strcmp(coinaddr,coin->smartaddr) != 0) )
        {
            if ( coinaddr == 0 || coinaddr[0] == 0 )
                strcpy(address,coin->smartaddr);
            else
            {
                bitcoin_addr2rmd160("KMD",taddr,&addrtype,rmd160,coinaddr);
                bitcoin_address(coin->symbol,address,coin->taddr,coin->pubtype,rmd160,20);
                //printf("%s taddr.%d addrtype.%u %s -> %s [%c %c].%d\n",coin->symbol,taddr,addrtype,coinaddr,address,coinaddr[0],coinaddr[1],coinaddr[0] == 't' && (coinaddr[1] == '1' || coinaddr[1] == '3'));
            }
            if ( (retjson= LP_address_balance(coin,address,1)) != 0 )
            {
                if ( (balance= jdouble(retjson,"balance")*SATOSHIDEN) > 0 )
                {
                    item = cJSON_CreateObject();
                    jaddstr(item,"coin",coin->symbol);
                    jaddnum(item,"balance",dstr(balance));
                    if ( (KMDvalue= LP_KMDvalue(coin,balance)) != 0 )
                    {
                        jaddnum(item,"KMDvalue",dstr(KMDvalue));
                        sum += KMDvalue;
                    }
                    jaddi(array,item);
                }
                free_json(retjson);
            }
        }
    }
    if ( sum != 0 )
    {
        item = cJSON_CreateObject();
        jaddstr(item,"coin","total");
        jaddnum(item,"balance",dstr(sum));
        jaddi(array,item);
    }
    return(array);
}

struct LP_transaction *LP_transactionfind(struct iguana_info *coin,bits256 txid)
{
    struct LP_transaction *tx;
    portable_mutex_lock(coin->_txmutex);
    HASH_FIND(hh,coin->transactions,txid.bytes,sizeof(txid),tx);
    portable_mutex_unlock(coin->_txmutex);
    return(tx);
}
