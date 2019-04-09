
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
//  LP_tradebots.c
//  marketmaker
//

#define TRADEBOTS_GAPTIME 120
#define LP_TRADEBOTS_MAXTRADES 10

struct LP_tradebot_trade
{
    double maxprice,totalrelvolume,basevol,relvol;
    uint64_t aliceid;
    int32_t dispdir;
    uint32_t started,finished,requestid,quoteid,tradeid,expired;
    char base[65],rel[65],event[32];
};

struct LP_tradebot
{
    struct LP_tradebot *next,*prev;
    char name[128],base[65],rel[65];
    int32_t numtrades,numpending,completed,dispdir;
    double maxprice,totalrelvolume,totalbasevolume,basesum,relsum,pendbasesum,pendrelsum;
    uint32_t lasttime,dead,pause,userpause,started,id;
    struct LP_tradebot_trade *trades[LP_TRADEBOTS_MAXTRADES];
} *LP_tradebots;

void LP_tradebot_pauseall()
{
    struct LP_tradebot *bot,*tmp;
    DL_FOREACH_SAFE(LP_tradebots,bot,tmp)
    {
        bot->userpause = bot->pause = (uint32_t)time(NULL);
    }
}

void LP_tradebot_updatestats(struct LP_tradebot *bot,struct LP_tradebot_trade *tp)
{
    char *swapstr,*status; int32_t flag; cJSON *swapjson;
    if ( (swapstr= basilisk_swapentry(0,tp->requestid,tp->quoteid,1)) != 0 )
    {
        flag = 0;
        if ( (swapjson= cJSON_Parse(swapstr)) != 0 )
        {
            tp->basevol = dstr(j64bits(swapjson,"satoshis"));
            tp->relvol = dstr(j64bits(swapjson,"destsatoshis"));
            tp->aliceid = (uint64_t)juint(swapjson,"aliceid");
            if ( (status= jstr(swapjson,"status")) != 0 )
            {
                if ( strcmp(status,"finished") == 0 )
                {
                    if ( tp->finished == 0 )
                        tp->finished = (uint32_t)time(NULL);
                }
            }
            free_json(swapjson);
        }
        free(swapstr);
    }
}

void LP_tradebot_calcstats(struct LP_tradebot *bot)
{
    int32_t i; struct LP_tradebot_trade *tp;
    bot->basesum = bot->relsum = bot->pendbasesum = bot->pendrelsum = 0.;
    bot->numpending = bot->completed = 0;
    for (i=0; i<bot->numtrades; i++)
    {
        if ( (tp= bot->trades[i]) == 0 )
            continue;
        if ( tp->finished == 0 && time(NULL) > tp->started+LP_atomic_locktime(bot->base,bot->rel)*2 )
        {
            tp->expired = tp->finished = (uint32_t)time(NULL);
            printf("tradeid.%u expired\n",tp->tradeid);
        }
        if ( tp->finished != 0 )
        {
            if ( tp->expired == 0 )
            {
                bot->basesum += tp->basevol;
                bot->relsum += tp->relvol;
                bot->completed++;
            }
        }
        else
        {
            if ( tp->requestid != 0 && tp->quoteid != 0 )
            {
                bot->pendbasesum += tp->basevol;
                bot->pendrelsum += tp->relvol;
                bot->numpending++;
            }
        }
        //LP_tradebot_updatestats(bot,bot->trades[i]);
    }
    //printf("completed.%d (%.8f / %.8f)  pending.%d (%.8f / %.8f)\n",bot->completed,bot->basesum,bot->relsum,bot->numpending,bot->pendbasesum,bot->pendrelsum);
}

double LP_pricevol_invert(double *basevolumep,double maxprice,double relvolume)
{
    double price;
    *basevolumep = 0.;
    if ( maxprice > SMALLVAL && maxprice < SATOSHIDEN )
    {
        price = (1. / maxprice);
        *basevolumep = (relvolume * price);
        return(price);
    }
    return(0.);
}

cJSON *LP_tradebot_tradejson(struct LP_tradebot_trade *tp,int32_t dispflag)
{
    double price,basevol; cJSON *item = cJSON_CreateObject();
    if ( tp == 0 )
        return(cJSON_Parse("{}"));
    if ( tp->event[0] != 0 )
        jaddstr(item,"status",tp->event);
    if ( tp->requestid != 0 && tp->quoteid != 0 )
    {
        jaddnum(item,"requestid",tp->requestid);
        jaddnum(item,"quoteid",tp->quoteid);
    } else jaddnum(item,"tradeid",tp->tradeid);
    if ( tp->aliceid != 0 )
        jaddnum(item,"aliceid",tp->aliceid);
    if ( tp->basevol > SMALLVAL && tp->relvol > SMALLVAL )
    {
        if ( dispflag > 0 )
        {
            jaddnum(item,"price",tp->relvol/tp->basevol);
            jaddnum(item,"volume",tp->relvol);
        }
        else
        {
            price = LP_pricevol_invert(&basevol,tp->relvol / tp->basevol,tp->relvol);
            jaddnum(item,"price",price);
            jaddnum(item,"volume",basevol);
        }
    }
    return(item);
}

cJSON *LP_tradebot_json(struct LP_tradebot *bot)
{
    int32_t i; double aveprice,basevolume,vol; cJSON *json,*array;
    LP_tradebot_calcstats(bot);
    json = cJSON_CreateObject();
    jaddstr(json,"result","success");
    jaddstr(json,"name",bot->name);
    jaddnum(json,"botid",bot->id);
    jaddnum(json,"started",bot->started);
    if ( bot->pause != 0 || bot->userpause != 0 )
        jaddnum(json,"paused",bot->userpause != 0 ? bot->userpause : bot->pause);
    if ( bot->dead != 0 )
        jaddnum(json,"stopped",bot->dead);
    if ( bot->dispdir > 0 )
    {
        jaddstr(json,"action","buy");
        jaddstr(json,"base",bot->base);
        jaddstr(json,"rel",bot->rel);
        jaddnum(json,"maxprice",bot->maxprice);
        jaddnum(json,"totalrelvolume",bot->totalrelvolume);
        jaddnum(json,"totalbasevolume",bot->totalbasevolume);
        if ( (vol= bot->relsum) > SMALLVAL && bot->basesum > SMALLVAL )
        {
            jaddnum(json,"aveprice",vol/bot->basesum);
            jaddnum(json,"volume",vol);
        }
    }
    else
    {
        jaddstr(json,"action","sell");
        jaddstr(json,"base",bot->rel);
        jaddstr(json,"rel",bot->base);
        aveprice = LP_pricevol_invert(&basevolume,bot->maxprice,bot->totalrelvolume);
        jaddnum(json,"minprice",aveprice);
        jaddnum(json,"totalbasevolume",bot->totalrelvolume);
        jaddnum(json,"totalrelvolume",basevolume);
        if ( (vol= bot->relsum) > SMALLVAL && bot->basesum > SMALLVAL )
        {
            aveprice = LP_pricevol_invert(&basevolume,vol/bot->basesum,vol);
            jaddnum(json,"aveprice",aveprice);
            jaddnum(json,"volume",basevolume);
        }
    }
    array = cJSON_CreateArray();
    for (i=0; i<bot->numtrades; i++)
        jaddi(array,LP_tradebot_tradejson(bot->trades[i],bot->dispdir));
    jadd(json,"trades",array);
    if ( bot->basesum > SMALLVAL && bot->relsum > SMALLVAL && bot->completed > 0 )
    {
        jadd(json,"complete",bot->completed!=0?jtrue():jfalse());
        jaddnum(json,"percentage",100. * (bot->relsum / bot->totalrelvolume));
        if ( bot->dispdir > 0 )
        {
            jaddnum(json,"aveprice",bot->relsum / bot->basesum);
            jaddnum(json,"volume",bot->relsum);
        }
        else
        {
            jaddnum(json,"aveprice",bot->basesum / bot->relsum);
            jaddnum(json,"volume",bot->basesum);
        }
    }
    if ( bot->pendbasesum > SMALLVAL && bot->pendrelsum > SMALLVAL && bot->numpending > 0 )
    {
        jaddnum(json,"pending",bot->numpending);
        if ( bot->dispdir > 0 )
        {
            jaddnum(json,"pendingprice",bot->pendrelsum / bot->pendbasesum);
            jaddnum(json,"pendingvolume",bot->pendrelsum);
        }
        else
        {
            jaddnum(json,"pendingprice",bot->pendbasesum / bot->pendrelsum);
            jaddnum(json,"pendingvolume",bot->pendbasesum);
        }
    }
    return(json);
}

struct LP_tradebot *_LP_tradebotfind(uint32_t botid)
{
    struct LP_tradebot *tmp,*bot,*retbot = 0;
    DL_FOREACH_SAFE(LP_tradebots,bot,tmp)
    {
        if ( botid == bot->id )
        {
            retbot = bot;
            break;
        }
    }
    return(retbot);
}

struct LP_tradebot *LP_tradebotfind(uint32_t botid)
{
    struct LP_tradebot *retbot = 0;
    portable_mutex_lock(&LP_tradebotsmutex);
    retbot = _LP_tradebotfind(botid);
    portable_mutex_unlock(&LP_tradebotsmutex);
    return(retbot);
}

void LP_tradebotadd(struct LP_tradebot *bot)
{
    portable_mutex_lock(&LP_tradebotsmutex);
    while ( _LP_tradebotfind(bot->id) != 0 )
    {
        printf("BOT collision at %u, ok if rare\n",bot->id);
        bot->id++;
    }
    DL_APPEND(LP_tradebots,bot);
    portable_mutex_unlock(&LP_tradebotsmutex);
}

struct LP_tradebot_trade *LP_tradebot_pending(struct LP_tradebot *bot,cJSON *pending,uint32_t tradeid)
{
    struct LP_tradebot_trade *tp;
    tp = calloc(1,sizeof(*tp));
    tp->tradeid = tradeid;
    tp->maxprice = bot->maxprice;
    tp->totalrelvolume = bot->totalrelvolume;
    tp->started = (uint32_t)time(NULL);
    tp->dispdir = bot->dispdir;
    strcpy(tp->base,bot->base);
    strcpy(tp->rel,bot->rel);
    tp->aliceid = (uint64_t)juint(pending,"aliceid");
    tp->basevol = jdouble(pending,"basevalue");
    tp->relvol = jdouble(pending,"relvalue");
    printf("tradebot pending basevol %.8f relvol %.8f\n",tp->basevol,tp->relvol);
    return(tp);
}

double LP_orderbook_maxrel(char *base,char *rel,double maxprice)
{
    char *retstr; int32_t i,numasks; cJSON *retjson,*asks,*item; double maxvol,maxrel = 0.;
    if ( (retstr= LP_orderbook(base,rel,0)) != 0 )
    {
        //printf("maxprice %.8f %s/%s\n",maxprice,base,rel);
        if ( (retjson= cJSON_Parse(retstr)) != 0 )
        {
            if ( (asks= jarray(&numasks,retjson,"asks")) != 0 )
            {
                for (i=0; i<numasks; i++)
                {
                    item = jitem(asks,i);
                    if ( jdouble(item,"price") > maxprice )
                        break;
                    maxvol = jdouble(item,"maxvolume");
                    //printf("(%s) -> %.8f\n",jprint(item,0),maxvol);
                    if ( maxvol > maxrel )
                        maxrel = maxvol;
                }
            }
            free_json(retjson);
        }
        free(retstr);
    }
    return(maxrel);
}

void LP_tradebot_timeslice(struct LP_tradebot *bot, uint32_t ctx_h)
{
    double remaining,maxrel; struct LP_tradebot_trade *tp; int32_t i,maxiters = 10; uint32_t tradeid; bits256 destpubkey; char *retstr,*liststr; cJSON *retjson,*retjson2,*pending;
    memset(destpubkey.bytes,0,sizeof(destpubkey));
    LP_tradebot_calcstats(bot);
    if ( bot->dead == 0 && bot->pause == 0 && bot->userpause == 0 && bot->numtrades < sizeof(bot->trades)/sizeof(*bot->trades) )
    {
        if ( (liststr= LP_recent_swaps(0,0)) != 0 )
        {
            if ( (retjson= cJSON_Parse(liststr)) != 0 )
            {
                if ( jobj(retjson,"pending") == 0 )
                {
                    remaining = bot->totalrelvolume - (bot->relsum + bot->pendrelsum);
                    maxrel = LP_orderbook_maxrel(bot->base,bot->rel,bot->maxprice);
                    printf("try autobuy %s/%s remaining %.8f maxprice %.8f maxrel %.8f\n",bot->base,bot->rel,remaining,bot->maxprice,maxrel);
                    if ( maxrel < remaining )
                        remaining = maxrel;
                    tradeid = LP_rand();
                    for (i=1; i<=maxiters; i++)
                    {
                        if ( remaining < 0.001 )
                            break;
                        if ( (retstr= LP_autobuy(0,bot->base,bot->rel,bot->maxprice,remaining/i,0,0,G.gui,0,destpubkey,tradeid,0,0,ctx_h)) != 0 )
                        {
                            if ( (retjson2= cJSON_Parse(retstr)) != 0 )
                            {
                                if ( (pending= jobj(retjson2,"pending")) != 0 && juint(pending,"tradeid") == tradeid )
                                {
                                    bot->trades[bot->numtrades++] = tp = LP_tradebot_pending(bot,pending,tradeid);
                                    printf("issued bot trade.%u %s\n",tradeid,retstr);
                                    free_json(retjson2);
                                    free(retstr);
                                    break;
                                } else printf("iter.%d/%d %.8f didnt get any trade pending %s %s\n\n",i,maxiters,remaining/i,bot->name,retstr);
                                free_json(retjson2);
                            } else printf("iter.%d/%d %.8f %s\n",i,maxiters,remaining/i,retstr);
                            free(retstr);
                        }
                    }
                    LP_tradebot_calcstats(bot);
                }
                free_json(retjson);
            }
            free(liststr);
        }
    }
    else if ( bot->pause == 0 )
        bot->pause = (uint32_t)time(NULL);
}

void LP_aliceid(uint32_t tradeid,uint64_t aliceid,char *event,uint32_t requestid,uint32_t quoteid)
{
    struct LP_tradebot *bot,*tmp; int32_t i,matched = 0; struct LP_tradebot_trade *tp;
    if ( tradeid == 0 )
        return;
    DL_FOREACH_SAFE(LP_tradebots,bot,tmp)
    {
        for (i=0; i<bot->numtrades; i++)
        {
            if ( (tp= bot->trades[i]) != 0 )
            {
                if ( tp->finished == 0 && tp->tradeid == tradeid )
                {
                    tp->aliceid = aliceid;
                    printf("bot event tradeid.%u aliceid.%llu (%s) r.%u q.%u\n",tradeid,(long long)aliceid,event,requestid,quoteid);
                    if ( requestid != 0 && quoteid != 0 )
                    {
                        tp->requestid = requestid;
                        tp->quoteid = quoteid;
                    }
                    strcpy(tp->event,event);
                    matched = 1;
                    break;
                } else printf("tradeid.%u finished.%u\n",tp->tradeid,tp->finished);
            }
        }
        if ( matched != 0 )
            break;
    }
    if ( 0 && matched == 0 )
        printf("NO MATCH: bot event tradeid.%u aliceid.%llu (%s) r.%u q.%u\n",tradeid,(long long)aliceid,event,requestid,quoteid);
}

void LP_tradebot_finished(uint32_t tradeid,uint32_t requestid,uint32_t quoteid)
{
    struct LP_tradebot *bot,*tmp; int32_t i; struct LP_tradebot_trade *tp;
    DL_FOREACH_SAFE(LP_tradebots,bot,tmp)
    {
        for (i=0; i<bot->numtrades; i++)
        {
            if ( (tp= bot->trades[i]) != 0 && tp->finished == 0 && tp->tradeid == tradeid )
            {
                tp->requestid = requestid;
                tp->quoteid = quoteid;
                printf("bot.%u detected completion tradeid.%u aliceid.%llu r.%u q.%u, numpending.%d completed.%d\n",bot->id,tp->tradeid,(long long)tp->aliceid,tp->requestid,tp->quoteid,bot->numpending,bot->completed);
                tp->finished = (uint32_t)time(NULL);
                strcpy(tp->event,"finished");
                break;
            }
        }
    }
}

char *LP_tradebot_list(void *ctx,int32_t pubsock,cJSON *argjson)
{
    struct LP_tradebot *bot,*tmp; cJSON *array = cJSON_CreateArray();
    DL_FOREACH_SAFE(LP_tradebots,bot,tmp)
    {
        jaddinum(array,bot->id);
    }
    return(jprint(array,1));
}

char *LP_tradebot_statuslist(void *ctx,int32_t pubsock,cJSON *argjson)
{
    struct LP_tradebot *bot,*tmp; cJSON *array = cJSON_CreateArray();
    DL_FOREACH_SAFE(LP_tradebots,bot,tmp)
    {
        jaddi(array,LP_tradebot_json(bot));
    }
    return(jprint(array,1));
}

char *LP_tradebot_settings(void *ctx,int32_t pubsock,cJSON *argjson,uint32_t botid)
{
    struct LP_tradebot *bot; double newprice,newvolume;
    if ( (bot= LP_tradebotfind(botid)) != 0 )
    {
        if ( bot->dead != 0 )
            return(clonestr("{\"error\":\"botid aleady stopped\"}"));
        newprice = jdouble(argjson,"newprice");
        newvolume = jdouble(argjson,"newvolume");
        if ( (newprice > SMALLVAL && newprice < SATOSHIDEN) || (newvolume > 0.0001 && newvolume < SATOSHIDEN) )
        {
            if ( bot->dispdir < 0 )
            {
                if ( newprice > SMALLVAL )
                    bot->maxprice = 1. / newprice;
                if ( newvolume > SMALLVAL )
                    bot->totalrelvolume = (bot->maxprice * newvolume);
            }
            else
            {
                if ( newprice > SMALLVAL )
                    bot->maxprice = newprice;
                if ( newvolume > SMALLVAL )
                    bot->totalrelvolume = newvolume;
            }
        }
        return(jprint(LP_tradebot_json(bot),1));
    }
    return(clonestr("{\"error\":\"couldnt find botid\"}"));
}

char *LP_tradebot_status(void *ctx,int32_t pubsock,cJSON *argjson,uint32_t botid)
{
    struct LP_tradebot *bot;
    if ( (bot= LP_tradebotfind(botid)) != 0 )
        return(jprint(LP_tradebot_json(bot),1));
    return(clonestr("{\"error\":\"couldnt find botid\"}"));
}

char *LP_tradebot_stop(void *ctx,int32_t pubsock,cJSON *argjson,uint32_t botid)
{
    struct LP_tradebot *bot;
    if ( (bot= LP_tradebotfind(botid)) != 0 )
    {
        bot->dead = (uint32_t)time(NULL);
        return(clonestr("{\"result\":\"success\"}"));
    }
    return(clonestr("{\"error\":\"couldnt find botid\"}"));
}

char *LP_tradebot_pause(void *ctx,int32_t pubsock,cJSON *argjson,uint32_t botid)
{
    struct LP_tradebot *bot;
    if ( (bot= LP_tradebotfind(botid)) != 0 )
    {
        if ( bot->dead != 0 )
            return(clonestr("{\"error\":\"botid aleady stopped\"}"));
        bot->userpause = (uint32_t)time(NULL);
        return(clonestr("{\"result\":\"success\"}"));
    }
    return(clonestr("{\"error\":\"couldnt find botid\"}"));
}

char *LP_tradebot_resume(void *ctx,int32_t pubsock,cJSON *argjson,uint32_t botid)
{
    struct LP_tradebot *bot;
    if ( (bot= LP_tradebotfind(botid)) != 0 )
    {
        if ( bot->dead != 0 )
            return(clonestr("{\"error\":\"botid aleady stopped\"}"));
        if ( bot->userpause == 0 )
            return(clonestr("{\"result\":\"success\",\"status\":\"botid not paused\"}"));
        bot->userpause = 0;
        return(clonestr("{\"result\":\"success\"}"));
    }
    return(clonestr("{\"error\":\"couldnt find botid\"}"));
}

char *LP_istradebots_command(void *ctx,int32_t pubsock,char *method,cJSON *argjson)
{
    uint32_t botid;
    //printf("LP_istradebots_command check %s\n",method);
    if ( strncmp("bot_",method,strlen("bot_")) != 0 )
        return(0);
    if ( strcmp(method,"bot_list") == 0 )
        return(LP_tradebot_list(ctx,pubsock,argjson));
    else if ( strcmp(method,"bot_statuslist") == 0 )
        return(LP_tradebot_statuslist(ctx,pubsock,argjson));
    if ( (botid= juint(argjson,"botid")) == 0 )
        return(clonestr("{\"error\":\"no botid specified\"}"));
    else
    {
        if ( strcmp(method,"bot_status") == 0 )
            return(LP_tradebot_status(ctx,pubsock,argjson,botid));
        else if ( strcmp(method,"bot_settings") == 0 )
            return(LP_tradebot_settings(ctx,pubsock,argjson,botid));
        else if ( strcmp(method,"bot_stop") == 0 )
            return(LP_tradebot_stop(ctx,pubsock,argjson,botid));
        else if ( strcmp(method,"bot_pause") == 0 )
            return(LP_tradebot_pause(ctx,pubsock,argjson,botid));
        else if ( strcmp(method,"bot_resume") == 0 )
            return(LP_tradebot_resume(ctx,pubsock,argjson,botid));
    }
    return(0);
}

