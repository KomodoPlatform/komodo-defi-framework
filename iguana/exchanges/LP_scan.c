
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
//  LP_scan.c
//  marketmaker
//


char *banned_txids[] =
{
    "78cb4e21245c26b015b888b14c4f5096e18137d2741a6de9734d62b07014dfca", //233559
    "00697be658e05561febdee1aafe368b821ca33fbb89b7027365e3d77b5dfede5", //234172
    "e909465788b32047c472d73e882d79a92b0d550f90be008f76e1edaee6d742ea", //234187
    "f56c6873748a327d0b92b8108f8ec8505a2843a541b1926022883678fb24f9dc", //234188
    "abf08be07d8f5b3a433ddcca7ef539e79a3571632efd6d0294ec0492442a0204", //234213
    "3b854b996cc982fba8c06e76cf507ae7eed52ab92663f4c0d7d10b3ed879c3b0", //234367
    "fa9e474c2cda3cb4127881a40eb3f682feaba3f3328307d518589024a6032cc4", //234635
    "ca746fa13e0113c4c0969937ea2c66de036d20274efad4ce114f6b699f1bc0f3", //234662
    "43ce88438de4973f21b1388ffe66e68fda592da38c6ef939be10bb1b86387041", //234697
    "0aeb748de82f209cd5ff7d3a06f65543904c4c17387c9d87c65fd44b14ad8f8c", //234899
    "bbd3a3d9b14730991e1066bd7c626ca270acac4127131afe25f877a5a886eb25", //235252
    "fa9943525f2e6c32cbc243294b08187e314d83a2870830180380c3c12a9fd33c", //235253
    "a01671c8775328a41304e31a6693bbd35e9acbab28ab117f729eaba9cb769461", //235265
    "2ef49d2d27946ad7c5d5e4ab5c089696762ff04e855f8ab48e83bdf0cc68726d", //235295
    "c85dcffb16d5a45bd239021ad33443414d60224760f11d535ae2063e5709efee", //235296
    // all vouts banned
    "c4ea1462c207547cd6fb6a4155ca6d042b22170d29801a465db5c09fec55b19d", //246748
    "305dc96d8bc23a69d3db955e03a6a87c1832673470c32fe25473a46cc473c7d1", //247204
};

int32_t komodo_bannedset(int32_t *indallvoutsp,bits256 *array,int32_t max)
{
    int32_t i;
    if ( sizeof(banned_txids)/sizeof(*banned_txids) > max )
    {
        printf("komodo_bannedset: buffer too small %ld vs %d\n",(long)sizeof(banned_txids)/sizeof(*banned_txids),max);
        exit(-1);
    }
    for (i=0; i<sizeof(banned_txids)/sizeof(*banned_txids); i++)
        decode_hex(array[i].bytes,sizeof(array[i]),banned_txids[i]);
    *indallvoutsp = i-2;
    return(i);
}

int sort_balance(void *a,void *b)
{
    int64_t aval,bval;
    /* compare a to b (cast a and b appropriately)
     * return (int) -1 if (a < b)
     * return (int)  0 if (a == b)
     * return (int)  1 if (a > b)
     */
    aval = ((struct LP_address *)a)->balance;
    bval = ((struct LP_address *)b)->balance;
    //printf("%.8f vs %.8f -> %d\n",dstr(aval),dstr(bval),(int32_t)(bval - aval));
    return((aval == bval) ? 0 : ((aval < bval) ? 1 : -1));
}

// a primitive restore can be done by loading the previous snapshot and creating a virtual tx for all the balance at height-1. this wont allow anything but new snapshots, but for many use cases that is all that is needed

cJSON *LP_snapshot(struct iguana_info *coin,int32_t height)
{
   static bits256 bannedarray[64]; static int32_t numbanned,indallvouts,maxsnapht; static char lastcoin[65];
    struct LP_transaction *tx,*tmp; struct LP_address *ap,*atmp; int32_t isKMD,i,j,n,skipflag=0,startht,endht,ht; uint64_t banned_balance=0,balance=0,noaddr_balance=0; cJSON *retjson,*array,*item;
    if ( bannedarray[0].txid == 0 )
        numbanned = komodo_bannedset(&indallvouts,bannedarray,(int32_t)(sizeof(bannedarray)/sizeof(*bannedarray)));
    startht = 1;
    endht = height-1;
    if ( strcmp(coin->symbol,lastcoin) == 0 )
    {
        if ( maxsnapht > height )
            skipflag = 1;
        else startht = maxsnapht+1;
    }
    else
    {
        maxsnapht = 0;
        strcpy(lastcoin,coin->symbol);
    }
    retjson = cJSON_CreateObject();
    portable_mutex_lock(coin->_txmutex);
    portable_mutex_lock(coin->_addrmutex);
    HASH_ITER(hh,coin->addresses,ap,atmp)
    {
        ap->balance = 0;
    }
    isKMD = (strcmp(coin->symbol,"KMD") == 0) ? 1 : 0;
    HASH_ITER(hh,coin->transactions,tx,tmp)
    {
        if ( tx->height < height )
        {
            if ( isKMD != 0 )
            {
                for (j=0; j<numbanned; j++)
                    if ( bits256_cmp(bannedarray[j],tx->txid) == 0 )
                        break;
                if ( j < numbanned )
                {
                    for (i=0; i<tx->numvouts; i++)
                        banned_balance += tx->outpoints[i].value;
                    //char str[256]; printf("skip banned %s bannedtotal: %.8f\n",bits256_str(str,tx->txid),dstr(banned_balance));
                    continue;
                }
            }
            for (i=0; i<tx->numvouts; i++)
            {
                if ( (ht=tx->outpoints[i].spendheight) > 0 && ht < height )
                    continue;
                if ( tx->outpoints[i].coinaddr[0] != 0 && (ap= _LP_address(coin,tx->outpoints[i].coinaddr)) != 0 )
                {
                    balance += tx->outpoints[i].value;
                    ap->balance += tx->outpoints[i].value;
                    //printf("(%s/%s) %.8f %.8f\n",tx->outpoints[i].coinaddr,ap->coinaddr,dstr(tx->outpoints[i].value),dstr(ap->balance));
                } else noaddr_balance += tx->outpoints[i].value;
            }
        }
    }
    HASH_SORT(coin->addresses,sort_balance);
    portable_mutex_unlock(coin->_addrmutex);
    portable_mutex_unlock(coin->_txmutex);
    printf("%s balance %.8f at height.%d\n",coin->symbol,dstr(balance),height);
    array = cJSON_CreateArray();
    n = 0;
    HASH_ITER(hh,coin->addresses,ap,atmp)
    {
        if ( ap->balance != 0 )
        {
            item = cJSON_CreateObject();
            jaddnum(item,ap->coinaddr,dstr(ap->balance));
            jaddi(array,item);
            n++;
        }
    }
    jadd(retjson,"balances",array);
    jaddstr(retjson,"coin",coin->symbol);
    jaddnum(retjson,"height",height);
    jaddnum(retjson,"numaddresses",n);
    jaddnum(retjson,"total",dstr(balance));
    jaddnum(retjson,"noaddr_total",dstr(noaddr_balance));
    return(retjson);
}

char *LP_snapshot_balance(struct iguana_info *coin,int32_t height,cJSON *argjson)
{
    cJSON *snapjson,*retjson,*balances,*array,*addrs,*child,*item,*item2; char *coinaddr,*refaddr; int32_t i,n,j,m; uint64_t total=0,value,balance = 0;
    retjson = cJSON_CreateObject();
    array = cJSON_CreateArray();
    if ( (snapjson= LP_snapshot(coin,height)) != 0 )
    {
        total = jdouble(snapjson,"total") * SATOSHIDEN;
        if ( (addrs= jarray(&m,argjson,"addresses")) != 0 )
        {
            if ( (balances= jarray(&n,snapjson,"balances")) != 0 )
            {
                for (i=0; i<n; i++)
                {
                    item = jitem(balances,i);
                    if ( (child= item->child) != 0 )
                    {
                        value = (uint64_t)(child->valuedouble * SATOSHIDEN);
                        if ( (refaddr= get_cJSON_fieldname(child)) != 0 )
                        {
                            //printf("check %s %.8f against %d\n",refaddr,dstr(value),m);
                            for (j=0; j<m; j++)
                            {
                                if ( (coinaddr= jstri(addrs,j)) != 0 )
                                {
                                    if ( strcmp(coinaddr,refaddr) == 0 )
                                    {
                                        item2 = cJSON_CreateObject();
                                        jaddnum(item2,coinaddr,dstr(value));
                                        jaddi(array,item2);
                                        balance += value;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        free_json(snapjson);
    }
    jadd(retjson,"balances",array);
    jaddstr(retjson,"coin",coin->symbol);
    jaddnum(retjson,"height",height);
    jaddnum(retjson,"balance",dstr(balance));
    jaddnum(retjson,"total",dstr(total));
    return(jprint(retjson,1));
}

char *LP_dividends(struct iguana_info *coin,int32_t height,cJSON *argjson)
{
    cJSON *array,*retjson,*item,*child,*exclude=0; int32_t i,j,emitted=0,dusted=0,n,execflag=0,flag,iter,numexcluded=0; char buf[1024],*field,*prefix="",*suffix=""; uint64_t dustsum=0,excluded=0,total=0,dividend=0,value,val,emit=0,dust=0; double ratio = 1.;
    if ( (retjson= LP_snapshot(coin,height)) != 0 )
    {
        //printf("SNAPSHOT.(%s)\n",retstr);
        if ( (array= jarray(&n,retjson,"balances")) != 0 )
        {
            if ( (n= cJSON_GetArraySize(array)) != 0 )
            {
                if ( argjson != 0 )
                {
                    exclude = jarray(&numexcluded,argjson,"exclude");
                    dust = (uint64_t)(jdouble(argjson,"dust") * SATOSHIDEN);
                    dividend = (uint64_t)(jdouble(argjson,"dividend") * SATOSHIDEN);
                    if ( jstr(argjson,"prefix") != 0 )
                        prefix = jstr(argjson,"prefix");
                    if ( jstr(argjson,"suffix") != 0 )
                        suffix = jstr(argjson,"suffix");
                    execflag = jint(argjson,"system");
                }
                for (iter=0; iter<2; iter++)
                {
                    for (i=0; i<n; i++)
                    {
                        flag = 0;
                        item = jitem(array,i);
                        if ( (child= item->child) != 0 )
                        {
                            value = (uint64_t)(child->valuedouble * SATOSHIDEN);
                            if ( (field= get_cJSON_fieldname(child)) != 0 )
                            {
                                for (j=0; j<numexcluded; j++)
                                    if ( strcmp(field,jstri(exclude,j)) == 0 )
                                    {
                                        flag = 1;
                                        break;
                                    }
                            }
                            //printf("(%s %s %.8f) ",jprint(item,0),field,dstr(value));
                            if ( iter == 0 )
                            {
                                if ( flag != 0 )
                                    excluded += value;
                                else total += value;
                            }
                            else
                            {
                                if ( flag == 0 )
                                {
                                    val = ratio * value;
                                    if ( val >= dust )
                                    {
                                        sprintf(buf,"%s %s %.8f %s",prefix,field,dstr(val),suffix);
                                        if ( execflag != 0 )
                                        {
                                            #ifdef TARGET_OS_IPHONE
                                                printf("error system.(%s).system not supported on iOS\n",buf);
                                            #else
                                                if ( system(buf) != 0 )
                                                    printf("error system.(%s)\n",buf);
                                            #endif
                                        }
                                        else printf("%s\n",buf);
                                        emit += val;
                                        emitted++;
                                    } else dustsum += val, dusted++;
                                }
                            }
                        }
                    }
                    if ( iter == 0 )
                    {
                        if ( total > 0 )
                        {
                            if ( dividend == 0 )
                                dividend = total;
                            ratio = (double)dividend / total;
                        } else break;
                    }
                }
            }
        }
        free_json(retjson);
        retjson = cJSON_CreateObject();
        jaddstr(retjson,"coin",coin->symbol);
        jaddnum(retjson,"height",height);
        jaddnum(retjson,"total",dstr(total));
        jaddnum(retjson,"emitted",emitted);
        jaddnum(retjson,"excluded",dstr(excluded));
        if ( dust != 0 )
        {
            jaddnum(retjson,"dust",dstr(dust));
            jaddnum(retjson,"dusted",dusted);
        }
        if ( dustsum != 0 )
            jaddnum(retjson,"dustsum",dstr(dustsum));
        jaddnum(retjson,"dividend",dstr(dividend));
        jaddnum(retjson,"dividends",dstr(emit));
        jaddnum(retjson,"ratio",ratio);
        if ( execflag != 0 )
            jaddnum(retjson,"system",execflag);
        /*if ( prefix[0] != 0 )
            jaddstr(retjson,"prefix",prefix);
        if ( suffix[0] != 0 )
            jaddstr(retjson,"suffix",suffix);*/
        return(jprint(retjson,1));
    }
    return(clonestr("{\"error\":\"symbol not found\"}"));
}
