
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
//  LP_portfolio.c
//  marketmaker
//

struct LP_autoprice_ref LP_autorefs[1024];

int32_t LP_autoprices,num_LP_autorefs;
char LP_portfolio_base[128],LP_portfolio_rel[128];
double LP_portfolio_relvolume;

void LP_portfolio_reset()
{
    struct iguana_info *coin,*tmp; void* fundjson; int32_t i; struct LP_autoprice_ref *ptr;
    for (i=0; i<num_LP_autorefs; i++)
    {
        ptr = &LP_autorefs[i];
        if ( (fundjson= ptr->fundvalue_req) != 0 )
        {
            ptr->fundvalue_req = 0;
            //it's a boxed `AutopriceReq` now, will free it after the port of `LP_portfolio_reset`// free_json(fundjson);
        }
    }
    memset(LP_autorefs,0,sizeof(LP_autorefs));
    LP_autoprices = 0;
    num_LP_autorefs = 0;
    strcpy(LP_portfolio_base,"");
    strcpy(LP_portfolio_rel,"");
    LP_portfolio_relvolume = 0.;
    HASH_ITER(hh,LP_coins,coin,tmp)
    {
        coin->maxamount = 0;
        coin->perc = 0;
        coin->goal = 0;
        coin->goalperc = 0;
        coin->relvolume = 0;
        coin->force = 0;
        coin->balanceA = 0;
        coin->valuesumA = 0;
        coin->balanceB = 0;
        coin->valuesumB = 0;
    }
}

cJSON *LP_portfolio_entry(struct iguana_info *coin)
{
    cJSON *item = cJSON_CreateObject();
    jaddstr(item,"coin",coin->symbol);
    jaddstr(item,"address",coin->smartaddr);
    jaddnum(item,"amount",dstr(coin->maxamount));
    jaddnum(item,"price",coin->price_kmd);
    jaddnum(item,"kmd_equiv",dstr(coin->kmd_equiv));
    jaddnum(item,"perc",coin->perc);
    jaddnum(item,"goal",coin->goal);
    jaddnum(item,"goalperc",coin->goalperc);
    jaddnum(item,"relvolume",coin->relvolume);
    jaddnum(item,"force",coin->force);
    jaddnum(item,"balanceA",dstr(coin->balanceA));
    jaddnum(item,"valuesumA",dstr(coin->valuesumA));
    if ( coin->valuesumA != 0 )
        jaddnum(item,"aliceutil",100. * (double)coin->balanceA/coin->valuesumA);
    jaddnum(item,"balanceB",dstr(coin->balanceB));
    jaddnum(item,"valuesumB",dstr(coin->valuesumB));
    jaddnum(item,"balance",dstr(coin->maxamount));
    if ( coin->valuesumB != 0 )
        jaddnum(item,"bobutil",100. * (double)coin->balanceB/coin->valuesumB);
    return(item);
}

/*int32_t LP_autofill(char *base,char *rel,double maxprice,double totalrelvolume)
{
    struct LP_priceinfo *basepp,*relpp;
    if ( (basepp= LP_priceinfofind(base)) != 0 && (relpp= LP_priceinfofind(rel)) != 0 )
    {
        basepp->maxprices[relpp->ind] = maxprice;
        basepp->relvols[relpp->ind] = totalrelvolume;
        LP_autofills++;
        return(0);
    }
    return(-1);
}*/

void LP_autopriceset(int32_t ind,void *ctx,int32_t dir,struct LP_priceinfo *basepp,struct LP_priceinfo *relpp,double price,char *refbase,char *refrel)
{
    static uint32_t lasttime;
    double margin,minprice,newprice,oppomargin,fixedprice,factor,offset; double bid,ask; int32_t changed;
    margin = basepp->sellmargins[relpp->ind];
    oppomargin = basepp->buymargins[relpp->ind];
    if ( (fixedprice= basepp->fixedprices[relpp->ind]) > SMALLVAL )
    {
        LP_mypriceset(relpp->symbol,basepp->symbol,fixedprice,0.);
        //printf("autoprice FIXED %s/%s <- %.8f\n",basepp->symbol,relpp->symbol,fixedprice);
        LP_pricepings(relpp->symbol,basepp->symbol,fixedprice);
        return;
    }
    if ( margin != 0. || oppomargin != 0. )
    {
        offset = basepp->offsets[relpp->ind];
        factor = basepp->factors[relpp->ind];
        if ( fabs(price) < SMALLVAL && refbase != 0 && refrel != 0 )
        {
            price = LP_myprice(&bid,&ask,refbase,refrel);
            //printf("%s/%s USE ref %s/%s %.8f factor %.8f offset %.8f margin %.8f/%.8f\n",basepp->symbol,relpp->symbol,refbase,refrel,price,factor,offset,oppomargin,margin);
        }
        if ( LP_pricevalid(price) > 0 )
        {
            if ( factor > SMALLVAL )
            {
                //double tmp = (price * factor) + offset;
                //printf("price %.8f -> %.8f factor %.8f offset %.8f margin %.8f [%.8f %.8f] [%.8f %.8f]\n",price,tmp,factor,offset,margin,(tmp * (1. + margin)),1./(tmp * (1. - margin)),(tmp * (1. - margin)),1./(tmp * (1. + margin)));
                price = (price * factor) + offset;
            }
            if ( margin == 0. )
                margin = oppomargin;
            //printf("min %.8f %s/%s %.8f dir.%d margin %.8f (%.8f %.8f)\n",basepp->minprices[relpp->ind],relpp->symbol,basepp->symbol,price,dir,margin,1. / (price * (1. - margin)),(price * (1. + margin)));
            if ( dir > 0 )
                newprice = (1. / price) * (1. + margin);
            else newprice = (price * (1. + margin));
            if ( (minprice= basepp->minprices[relpp->ind]) == 0. || price >= minprice )
            {
                if ( ind >= 0 )
                {
                    if ( LP_autorefs[ind].lastask < SMALLVAL )
                        LP_autorefs[ind].lastask = newprice;
                    else LP_autorefs[ind].lastask = (LP_autorefs[ind].lastask * 0.99) + (0.01 *newprice);
                    newprice = LP_autorefs[ind].lastask;
                    //printf("autopriceset %s/%s <- %.8f %.8f (%.8f %.8f)\n",basepp->symbol,relpp->symbol,price,newprice,LP_autorefs[ind].lastbid,LP_autorefs[ind].lastask);
                }
                LP_mypriceset(relpp->symbol,basepp->symbol,newprice,0.);
                if ( changed != 0 || time(NULL) > lasttime+LP_ORDERBOOK_DURATION*.777)
                {
                    lasttime = (uint32_t)time(NULL);
                    LP_pricepings(relpp->symbol,basepp->symbol,newprice);
                }
            }
        }
    }
}

double LP_pricesparse(void *ctx,int32_t trexflag,char *retstr,struct LP_priceinfo *btcpp)
{
    //{"success":true,"message":"","result":[{"MarketName":"BTC-KMD","High":0.00040840,"Low":0.00034900,"Volume":328042.46061669,"Last":0.00037236,"BaseVolume":123.36439511,"TimeStamp":"2017-07-15T13:50:21.87","Bid":0.00035721,"Ask":0.00037069,"OpenBuyOrders":343,"OpenSellOrders":1690,"PrevDay":0.00040875,"Created":"2017-02-11T23:04:01.853"},
    //{"TradePairId":4762,"Label":"WAVES/BTC","AskPrice":0.00099989,"BidPrice":0.00097350,"Low":0.00095000,"High":0.00108838,"Volume":6501.24403100,"LastPrice":0.00098028,"BuyVolume":1058994.86554882,"SellVolume":2067.87377158,"Change":-7.46,"Open":0.00105926,"Close":0.00098028,"BaseVolume":6.52057452,"BuyBaseVolume":2.33098660,"SellBaseVolume":1167.77655709},
    int32_t i,j,n,iter; double price,kmdbtc,bid,ask,nxtkmd=0.; struct LP_priceinfo *coinpp,*refpp; char symbol[65],*name,*refcoin; cJSON *retjson,*array,*item;
    if ( (retjson= cJSON_Parse(retstr)) != 0 )
    {
        //printf("got.(%s)\n",retstr);
        kmdbtc = 0.;
        refcoin = "BTC";
        refpp = btcpp;
        if ( (array= jarray(&n,retjson,trexflag != 0 ? "result" : "Data")) != 0 )
        {
            for (iter=0; iter<2; iter++)
            {
                for (i=0; i<n; i++)
                {
                    item = jitem(array,i);
                    if ( (name= jstr(item,trexflag != 0 ? "MarketName" : "Label")) != 0 )
                    {
                        symbol[0] = 0;
                        if ( trexflag != 0 )
                        {
                            if ( strncmp("BTC-",name,4) == 0 )
                            {
                                name += 4;
                                strcpy(symbol,name);
                            }
                        }
                        else
                        {
                            for (j=0; j<sizeof(symbol)-1; j++)
                                if ( (symbol[j]= name[j]) == '/' )
                                    break;
                            symbol[j] = 0;
                            if ( strcmp(name+j+1,"BTC") != 0 )
                                continue;
                        }
                        if ( symbol[0] != 0 )
                        {
                            //printf("%s\n",jprint(item,0));
                            bid = jdouble(item,trexflag != 0 ? "Bid" : "BidPrice");
                            ask = jdouble(item,trexflag != 0 ? "Ask" : "AskPrice");
                            if ( iter == 1 && kmdbtc > SMALLVAL && strcmp(symbol,"NXT") == 0 )
                                nxtkmd = 0.5 * (bid + ask) / kmdbtc;
                            if ( (coinpp= LP_priceinfofind(symbol)) != 0 )
                            {
                                coinpp->high[trexflag] = jdouble(item,"High");
                                coinpp->low[trexflag] = jdouble(item,"Low");
                                //coinpp->volume = jdouble(item,"Volume");
                                //coinpp->btcvolume = jdouble(item,"BaseVolume");
                                coinpp->last[trexflag] = jdouble(item,trexflag != 0 ? "Last" : "LastPrice");
                                coinpp->bid[trexflag] = bid;
                                coinpp->ask[trexflag] = ask;
                                //coinpp->prevday = jdouble(item,"PrevDay");
                                //printf("iter.%d trexflag.%d %s high %.8f, low %.8f, last %.8f hbla.(%.8f %.8f)\n",iter,trexflag,symbol,coinpp->high[trexflag],coinpp->low[trexflag],coinpp->last[trexflag],coinpp->bid[trexflag],coinpp->ask[trexflag]);
                                if ( coinpp->bid[trexflag] > SMALLVAL && coinpp->ask[trexflag] > SMALLVAL )
                                {
                                    price = 0.5 * (coinpp->bid[trexflag] + coinpp->ask[trexflag]);
                                    if ( iter == 0 )
                                    {
                                        if ( strcmp(symbol,"KMD") == 0 )
                                            kmdbtc = price;
                                    }
                                    else
                                    {
                                        if ( strcmp(symbol,"KMD") == 0 )
                                            continue;
                                        //printf("(%s/%s) iter.%d trexflag.%d %s %.8f %.8f\n",refpp->symbol,coinpp->symbol,iter,trexflag,symbol,price,price/kmdbtc);
                                        price /= kmdbtc;
                                    }
                                    if ( trexflag == 0 && coinpp->bid[1] > SMALLVAL && coinpp->ask[1] > SMALLVAL )
                                    {
                                        //printf("have trex: iter.%d trexflag.%d %s %.8f %.8f\n",iter,trexflag,symbol,coinpp->bid[1],coinpp->ask[1]);
                                        continue;
                                    }
                                    LP_autopriceset(-1,ctx,1,coinpp,refpp,price,0,0);
                                    LP_autopriceset(-1,ctx,-1,refpp,coinpp,price,0,0);
                                }
                            }
                        }
                    }
                }
                refcoin = "KMD";
                if ( kmdbtc == 0. || (refpp= LP_priceinfofind("KMD")) == 0 )
                    break;
            }
        }
        free_json(retjson);
    }
    return(nxtkmd);
}

double LP_autoprice_newprice(int32_t bidask,double price,double newprice)
{
    double gap; int32_t r;
    if ( price > SMALLVAL && ((bidask == 0 && newprice < price) || (bidask != 0 && newprice > price)) )
    {
        gap = fabs(newprice - price) * 2;
        r = (rand() % 100);
        if ( bidask == 0 )
            price -= (gap * r) / 100.;
        else price += (gap * r) / 100.;
    }
    else if ( price > SMALLVAL )
        price = (price * 0.95) + (0.05 * newprice);
    else price = newprice;
    return(price);
}

double LP_tickered_price(int32_t bidask,char *base,char *rel,double price,cJSON *tickerjson)
{
    int32_t i,n; cJSON *item; double basevol,relvol,itemprice;
    //printf("%s %s/%s %.8f -> ",bidask == 0 ? "bid" : "ask",base,rel,price);
    if ( (n= cJSON_GetArraySize(tickerjson)) > 0 )
    {
        for (i=n-1; i>=0; i--)
        {
            // {"timestamp":1513235320,"KMD":860.45202538,"SUPERNET":20.00010000,"price":0.02324371}
            item = jitem(tickerjson,i);
            if ( (basevol= jdouble(item,base)) > SMALLVAL && (relvol= jdouble(item,rel)) > SMALLVAL )
            {
                itemprice = (relvol / basevol);
                //printf("%.8f ",itemprice);
                price = LP_autoprice_newprice(bidask,price,itemprice);
            }
        }
    }
    //printf("-> %.8f\n",price);
    return(price);
}

int32_t LP_autoref_clear(char *base,char *rel)
{
    int32_t i;
    for (i=0; i<num_LP_autorefs; i++)
    {
        if ( (strcmp(rel,LP_autorefs[i].rel) == 0 && strcmp(base,LP_autorefs[i].base) == 0) ||
            (strcmp(base,LP_autorefs[i].rel) == 0 && strcmp(rel,LP_autorefs[i].base) == 0) )
        {
            memset(&LP_autorefs[i],0,sizeof(LP_autorefs[i]));
            return(i);
        }
    }
    return(-1);
}

int32_t LP_portfolio_trade(void *ctx,uint32_t *requestidp,uint32_t *quoteidp,struct iguana_info *buy,struct iguana_info *sell,double relvolume,int32_t setbaserel,char *gui)
{
    char *retstr2; uint64_t txfee,desttxfee; double bid,ask,maxprice; bits256 zero; uint32_t requestid,quoteid,iter,i; cJSON *retjson2; struct LP_utxoinfo A; struct LP_address_utxo *utxos[1000]; int32_t max=(int32_t)(sizeof(utxos)/sizeof(*utxos));
    LP_txfees(&txfee,&desttxfee,buy->symbol,sell->symbol);
    requestid = quoteid = 0;
    LP_myprice(&bid,&ask,buy->symbol,sell->symbol);
    maxprice = ask;
    if ( setbaserel != 0 )
    {
        strcpy(LP_portfolio_base,"");
        strcpy(LP_portfolio_rel,"");
        LP_portfolio_relvolume = 0.;
    }
    printf("pending.%d base buy.%s, rel sell.%s relvolume %f maxprice %.8f (%.8f %.8f)\n",G.LP_pendingswaps,buy->symbol,sell->symbol,sell->relvolume,maxprice,bid,ask);
    if ( LP_pricevalid(maxprice) > 0 )
    {
        relvolume = sell->relvolume;
        for (iter=0; iter<2; iter++)
        {
            if ( relvolume < dstr(LP_MIN_TXFEE) )
                break;
            if ( LP_address_myutxopair(&A,0,utxos,max,sell,sell->symbol,txfee,relvolume,maxprice,desttxfee) == 0 )
            //if ( LP_utxo_bestfit(sell->symbol,SATOSHIDEN * relvolume) != 0 )
            {
                memset(zero.bytes,0,sizeof(zero));
                if ( (retstr2= LP_autobuy(0,buy->symbol,sell->symbol,maxprice,relvolume,60,24*3600,gui,LP_lastnonce+1,zero,1,0,0,0)) != 0 )
                {
                    if ( (retjson2= cJSON_Parse(retstr2)) != 0 )
                    {
                        if ( (requestid= juint(retjson2,"requestid")) != 0 && (quoteid= juint(retjson2,"quoteid")) != 0 )
                        {
                            
                        }
                        free_json(retjson2);
                    }
                    printf("%s relvolume %.8f LP_autotrade.(%s)\n",sell->symbol,relvolume,retstr2);
                    free(retstr2);
                }
                if ( requestid != 0 && quoteid != 0 )
                    break;
            } else printf("cant find alice %.8f %s\n",relvolume,sell->symbol);
            if ( iter == 0 )
            {
                for (i=0; i<100; i++)
                {
                    relvolume *= .99;
                    if ( LP_address_myutxopair(&A,0,utxos,max,sell,sell->symbol,txfee,relvolume,maxprice,desttxfee) == 0 )
                    //if ( LP_utxo_bestfit(sell->symbol,SATOSHIDEN * relvolume) != 0 )
                    {
                        printf("i.%d relvolume %.8f from %.8f\n",i,relvolume,sell->relvolume);
                        break;
                    }
                }
            }
        }
    }
    else if ( setbaserel != 0 )
    {
        strcpy(LP_portfolio_base,buy->symbol);
        strcpy(LP_portfolio_rel,sell->symbol);
        LP_portfolio_relvolume = sell->relvolume;
    }
    *requestidp = requestid;
    *quoteidp = quoteid;
    if ( requestid != 0 && quoteid != 0 )
        return(0);
    else return(-1);
}

int32_t LP_portfolio_order(struct LP_portfoliotrade *trades,int32_t max,cJSON *array)
{
    int32_t i,j,m,n = 0; cJSON *item; struct LP_portfoliotrade coins[256];
    memset(coins,0,sizeof(coins));
    if ( (m= cJSON_GetArraySize(array)) > 0 )
    {
        for (i=j=0; i<m && i<sizeof(coins)/sizeof(*coins); i++)
        {
            item = jitem(array,i);
            safecopy(coins[j].buycoin,jstr(item,"coin"),sizeof(coins[j].buycoin));
            coins[j].metric = jdouble(item,"force");
            if ( fabs(coins[j].metric) > SMALLVAL && coins[j].buycoin[0] != 0 )
                j++;
        }
        if ( (m= j) > 1 )
        {
            for (i=n=0; i<m-1; i++)
                for (j=i+1; j<m; j++)
                    if ( coins[i].metric*coins[j].metric < 0. )
                    {
                        if ( coins[i].metric > 0. )
                        {
                            trades[n].metric = (coins[i].metric - coins[j].metric);
                            strcpy(trades[n].buycoin,coins[i].buycoin);
                            strcpy(trades[n].sellcoin,coins[j].buycoin);
                            printf("buy %s %f, sell %s %f -> %f\n",trades[n].buycoin,coins[i].metric,trades[n].sellcoin,coins[j].metric,trades[n].metric);
                        }
                        else
                        {
                            trades[n].metric = (coins[j].metric - coins[i].metric);
                            strcpy(trades[n].buycoin,coins[j].buycoin);
                            strcpy(trades[n].sellcoin,coins[i].buycoin);
                            printf("buy %s %f, sell %s %f -> %f\n",trades[n].buycoin,coins[j].metric,trades[n].sellcoin,coins[i].metric,trades[n].metric);
                        }
                        n++;
                        if ( n >= max )
                            break;
                    }
            revsortds((void *)trades,n,sizeof(*trades));
            for (i=0; i<n; i++)
                printf("%d: buy %s, sell %s -> %f\n",i,trades[i].buycoin,trades[i].sellcoin,trades[i].metric);
        }
    }
    return(n);
}

