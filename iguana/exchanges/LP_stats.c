
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
//  LP_stats.c
//  marketmaker
//

#define LP_STATSLOG_FNAME "stats.log"

struct LP_swapstats *LP_swapstats,*LP_RTstats;
int32_t LP_statslog_parsequote(char *method,cJSON *lineobj);

char *LP_stats_methods[] = { "unknown", "request", "reserved", "connect", "connected", "tradestatus" };
#define LP_TRADESTATUS_METHODIND 5

static uint32_t LP_requests,LP_reserveds,LP_connects,LP_connecteds,LP_tradestatuses,LP_parse_errors,LP_unknowns,LP_duplicates,LP_aliceids;

void LP_dPoW_request(struct iguana_info *coin)
{
    bits256 zero; cJSON *reqjson;
    reqjson = cJSON_CreateObject();
    jaddstr(reqjson,"method","getdPoW");
    jaddstr(reqjson,"coin",coin->symbol);
    memset(zero.bytes,0,sizeof(zero));
    //printf("request %s\n",jprint(reqjson,0));
    LP_reserved_msg(0,zero,jprint(reqjson,1));
}

void LP_dPoW_broadcast(struct iguana_info *coin)
{
    bits256 zero; cJSON *reqjson;
    if ( time(NULL) > coin->dPoWtime+60 && (coin->isassetchain != 0 || strcmp(coin->symbol,"KMD") == 0) )
    {
        reqjson = cJSON_CreateObject();
        jaddstr(reqjson,"method","dPoW");
        jaddstr(reqjson,"coin",coin->symbol);
        jaddnum(reqjson,"notarized",coin->notarized);
        jaddbits256(reqjson,"notarizedhash",coin->notarizedhash);
        jaddbits256(reqjson,"notarizationtxid",coin->notarizationtxid);
        memset(zero.bytes,0,sizeof(zero));
        //printf("broadcast %s\n",jprint(reqjson,0));
        LP_reserved_msg(0,zero,jprint(reqjson,1));
        coin->dPoWtime = (uint32_t)time(NULL);
    }
}

char *LP_dPoW_recv(cJSON *argjson)
{
    int32_t notarized; bits256 notarizedhash,notarizationtxid; char *symbol; struct iguana_info *coin;
    if ( (symbol= jstr(argjson,"coin")) != 0 && (coin= LP_coinfind(symbol)) != 0 && coin->electrum != 0 )
    {
        notarized = jint(argjson,"notarized");
        notarizedhash = jbits256(argjson,"notarizedhash");
        notarizationtxid = jbits256(argjson,"notarizationtxid");
        //printf("dPoW %s\n",jprint(argjson,0));
    }
    return(clonestr("{\"result\":\"success\"}"));
}

/*int32_t LP_dPoWheight(struct iguana_info *coin) // get dPoW protected height
{
    int32_t notarized,oldnotarized;
    if ( coin->electrum == 0 )
    {
        coin->heighttime = (uint32_t)(time(NULL) - 61);
        oldnotarized = coin->notarized;
        LP_getheight(&notarized,coin);
        if ( notarized != 0 && notarized != oldnotarized )
        {
            printf("dPoWheight.%s %d <- %d\n",coin->symbol,oldnotarized,notarized);
            coin->notarized = notarized;
        }
    }
    else if ( coin->notarized == 0 )
        LP_dPoW_request(coin);
    return(coin->notarized);
}*/

void LP_tradecommand_log(cJSON *argjson)
{
    static FILE *logfp; char *jsonstr;
    if ( logfp == 0 )
    {
#ifndef _WIN32
        if ( (logfp= fopen(LP_STATSLOG_FNAME,"rb+")) != 0 )
            fseek(logfp,0,SEEK_END);
        else
#endif
            logfp = fopen(LP_STATSLOG_FNAME,"wb");
    }
    if ( logfp != 0 )
    {
        if ( (jsonstr= jprint(argjson,0)) != 0 )
        {
            if ( IPC_ENDPOINT >= 0 )
                LP_QUEUE_COMMAND(0,jsonstr,IPC_ENDPOINT,-1,0);
            fprintf(logfp,"%s\n",jsonstr);
            free(jsonstr);
            fflush(logfp);
        }
    }
}

void LP_statslog_parseline(cJSON *lineobj)
{
    char *method; cJSON *obj;
    if ( (method= jstr(lineobj,"method")) != 0 )
    {
        if ( strcmp(method,"request") == 0 )
            LP_requests++;
        else if ( strcmp(method,"reserved") == 0 )
            LP_reserveds++;
        else if ( strcmp(method,"connect") == 0 )
        {
            if ( (obj= jobj(lineobj,"trade")) == 0 )
                obj = lineobj;
            LP_statslog_parsequote(method,obj);
            LP_connects++;
        }
        else if ( strcmp(method,"connected") == 0 )
        {
            LP_statslog_parsequote(method,lineobj);
            LP_connecteds++;
        }
        else if ( strcmp(method,"tradestatus") == 0 )
        {
            LP_statslog_parsequote(method,lineobj);
            LP_tradestatuses++;
        }
        else
        {
            LP_unknowns++;
            printf("parseline unknown method.(%s) (%s)\n",method,jprint(lineobj,0));
        }
    } else printf("parseline no method.(%s)\n",jprint(lineobj,0));
}

int32_t LP_statslog_parse()
{
    static long lastpos;
    FILE *fp; long fpos; char line[8192]; cJSON *lineobj; int32_t c,n = 0;
    if ( (fp= fopen(LP_STATSLOG_FNAME,"rb")) != 0 )
    {
        if ( lastpos > 0 )
        {
            fseek(fp,0,SEEK_END);
            if ( ftell(fp) >= lastpos )
                fseek(fp,lastpos,SEEK_SET);
            else
            {
                fclose(fp);
                return(0);
            }
        }
        else if ( 1 )
        {
            if ( IAMLP == 0 )
            {
                fseek(fp,0,SEEK_END);
                if ( (fpos= ftell(fp)) > LP_CLIENT_STATSPARSE )
                {
                    fseek(fp,fpos-LP_CLIENT_STATSPARSE,SEEK_SET);
                    while ( (c= fgetc(fp)) >= 0 && c != '\n' )
                        ;
                    printf("start scanning %s from %ld, found boundary %ld\n",LP_STATSLOG_FNAME,fpos-LP_CLIENT_STATSPARSE,ftell(fp));
                } else rewind(fp);
            }
        }
        while ( fgets(line,sizeof(line),fp) > 0 )
        {
            lastpos = ftell(fp);
            if ( (lineobj= cJSON_Parse(line)) != 0 )
            {
                n++;
                LP_statslog_parseline(lineobj);
                //printf("%s\n",jprint(lineobj,0));
                free_json(lineobj);
            }
        }
        fclose(fp);
    }
    return(n);
}

struct LP_swapstats *LP_swapstats_find(uint64_t aliceid)
{
    struct LP_swapstats *sp;
    portable_mutex_lock(&LP_statslogmutex);
    HASH_FIND(hh,LP_RTstats,&aliceid,sizeof(aliceid),sp);
    if ( sp == 0 )
        HASH_FIND(hh,LP_swapstats,&aliceid,sizeof(aliceid),sp);
    portable_mutex_unlock(&LP_statslogmutex);
    return(sp);
}

struct LP_swapstats *LP_swapstats_add(uint64_t aliceid,int32_t RTflag)
{
    struct LP_swapstats *sp;
    if ( (sp= LP_swapstats_find(aliceid)) == 0 )
    {
        sp = calloc(1,sizeof(*sp));
        sp->aliceid = aliceid;
        portable_mutex_lock(&LP_statslogmutex);
        if ( RTflag != 0 )
            HASH_ADD(hh,LP_RTstats,aliceid,sizeof(aliceid),sp);
        else HASH_ADD(hh,LP_swapstats,aliceid,sizeof(aliceid),sp);
        portable_mutex_unlock(&LP_statslogmutex);
    }
    return(LP_swapstats_find(aliceid));
}

uint64_t LP_aliceid_calc(bits256 desttxid,int32_t destvout,bits256 feetxid,int32_t feevout)
{
    return((((uint64_t)desttxid.uints[0] << 48) | ((uint64_t)destvout << 32) | ((uint64_t)feetxid.uints[0] << 16) | (uint32_t)feevout));
}

void LP_swapstats_line(int32_t *numtrades,uint64_t *basevols,uint64_t *relvols,char *line,struct LP_swapstats *sp)
{
    char tstr[64]; int32_t baseind,relind;
    if ( (baseind= LP_priceinfoind(sp->Q.srccoin)) >= 0 )
        basevols[baseind] += sp->Q.satoshis, numtrades[baseind]++;
    if ( (relind= LP_priceinfoind(sp->Q.destcoin)) >= 0 )
        relvols[relind] += sp->Q.destsatoshis, numtrades[relind]++;
    sprintf(line,"%s (%s).(%s) %-4d %9s %22llu: (%.8f %5s) -> (%.8f %5s) %.8f finished.%u expired.%u",utc_str(tstr,sp->Q.timestamp),sp->alicegui,sp->bobgui,sp->ind,LP_stats_methods[sp->methodind],(long long)sp->aliceid,dstr(sp->Q.satoshis),sp->Q.srccoin,dstr(sp->Q.destsatoshis),sp->Q.destcoin,sp->qprice,sp->finished,sp->expired);
}

bits256 LP_swapstats_txid(cJSON *argjson,char *name,bits256 oldtxid)
{
    bits256 txid,deadtxid;
    decode_hex(deadtxid.bytes,32,"deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    txid = jbits256(argjson,name);
    if ( bits256_nonz(txid) != 0 )
    {
        if ( bits256_cmp(deadtxid,txid) == 0 )
        {
            if ( bits256_nonz(oldtxid) == 0 )
                return(deadtxid);
            else return(oldtxid);
        } else return(txid);
    } else return(oldtxid);
}

int32_t LP_swapstats_update(struct LP_swapstats *sp,struct LP_quoteinfo *qp,cJSON *lineobj)
{
    char *statusstr,*base,*rel,gui[64]; uint32_t requestid,quoteid; uint64_t satoshis,destsatoshis;
    sp->lasttime = (uint32_t)time(NULL);
    safecopy(gui,sp->Q.gui,sizeof(gui));
    if ( strcmp(LP_stats_methods[sp->methodind],"tradestatus") == 0 )
    {
        base = jstr(lineobj,"bob");
        rel = jstr(lineobj,"alice");
        requestid = juint(lineobj,"requestid");
        quoteid = juint(lineobj,"quoteid");
        satoshis = jdouble(lineobj,"srcamount") * SATOSHIDEN;
        destsatoshis = jdouble(lineobj,"destamount") * SATOSHIDEN;
        if ( base != 0 && strcmp(base,sp->Q.srccoin) == 0 && rel != 0 && strcmp(rel,sp->Q.destcoin) == 0 && requestid == sp->Q.R.requestid && quoteid == sp->Q.R.quoteid && llabs((int64_t)(satoshis+2*sp->Q.txfee) - (int64_t)sp->Q.satoshis) <= sp->Q.txfee && llabs((int64_t)(destsatoshis+2*sp->Q.desttxfee) - (int64_t)sp->Q.destsatoshis) <= sp->Q.desttxfee )
        {
            sp->bobdeposit = LP_swapstats_txid(lineobj,"bobdeposit",sp->bobdeposit);
            sp->alicepayment = LP_swapstats_txid(lineobj,"alicepayment",sp->alicepayment);
            sp->bobpayment = LP_swapstats_txid(lineobj,"bobpayment",sp->bobpayment);
            sp->paymentspent = LP_swapstats_txid(lineobj,"paymentspent",sp->paymentspent);
            sp->Apaymentspent = LP_swapstats_txid(lineobj,"Apaymentspent",sp->Apaymentspent);
            sp->depositspent = LP_swapstats_txid(lineobj,"depositspent",sp->depositspent);
            if ( (statusstr= jstr(lineobj,"status")) != 0 && strcmp(statusstr,"finished") == 0 )
            {
                if ( (sp->finished= juint(lineobj,"timestamp")) == 0 )
                    sp->finished = (uint32_t)time(NULL);
            }
            if ( sp->finished == 0 && time(NULL) > sp->Q.timestamp+LP_atomic_locktime(base,rel)*2 )
                sp->expired = (uint32_t)time(NULL);
            return(0);
        }
        else
        {
            if ( 0 && requestid == sp->Q.R.requestid && quoteid == sp->Q.R.quoteid )
                printf("mismatched tradestatus aliceid.%22llu b%s/%s r%s/%s r%u/%u q%u/%u %.8f/%.8f -> %.8f/%.8f\n",(long long)sp->aliceid,base,sp->Q.srccoin,rel,sp->Q.destcoin,requestid,sp->Q.R.requestid,quoteid,sp->Q.R.quoteid,dstr(satoshis+2*sp->Q.txfee),dstr(sp->Q.satoshis),dstr(destsatoshis+2*sp->Q.desttxfee),dstr(sp->Q.destsatoshis));
            return(-1);
        }
        
    } else sp->Q = *qp;
    if ( sp->Q.gui[0] == 0 || strcmp(sp->Q.gui,"nogui") == 0 )
        strcpy(sp->Q.gui,gui);
    return(0);
}

int32_t LP_swap_finished(struct LP_swapstats *sp,int32_t dPoWflag)
{
    struct iguana_info *bob,*alice;
    if ( sp->dPoWfinished != 0 || sp->expired != 0 )
        return(1);
    else if ( dPoWflag == 0 && sp->finished != 0 )
        return(1);
    if ( (bob= LP_coinfind(sp->Q.srccoin)) == 0 )
    {
        //printf("no bobcoin.%s\n",sp->Q.srccoin);
        return(0);
    }
    if ( (alice= LP_coinfind(sp->Q.destcoin)) == 0 )
    {
        //printf("no alicecoin.%s\n",sp->Q.destcoin);
        return(0);
    }
    if ( dPoWflag != 0 )
    {
        if ( (sp->bobneeds_dPoW == 0 || (sp->bobneeds_dPoW > 1 && bob->notarized >= sp->bobneeds_dPoW)) && (sp->aliceneeds_dPoW == 0 || (sp->aliceneeds_dPoW > 1 && alice->notarized >= sp->aliceneeds_dPoW)) )
        {
            sp->dPoWfinished = (uint32_t)time(NULL);
            return(1);
        }
    }
    return(0);
}

struct LP_swapstats *LP_swapstats_create(uint64_t aliceid,int32_t RTflag,struct LP_quoteinfo *qp,double qprice,int32_t methodind)
{
    struct LP_pubswap *ptr; struct iguana_info *alice,*bob; struct LP_pubkey_info *pubp; char *base,*rel; struct LP_swapstats *sp = 0;
    base = qp->srccoin, rel = qp->destcoin;
    if ( (sp= LP_swapstats_add(aliceid,RTflag)) != 0 )
    {
        sp->Q = *qp;
        sp->qprice = qprice;
        sp->methodind = methodind;
        sp->ind = LP_aliceids++;
        sp->lasttime = (uint32_t)time(NULL);
        if ( sp->lasttime > sp->Q.timestamp+LP_atomic_locktime(base,rel)*2 )
            sp->expired = sp->lasttime;
        else
        {
            if ( (alice= LP_coinfind(rel)) != 0 && (alice->isassetchain != 0 || strcmp("KMD",alice->symbol) == 0) )
                sp->aliceneeds_dPoW = 1;
            if ( (bob= LP_coinfind(rel)) != 0 && (bob->isassetchain != 0 || strcmp(bob->symbol,"KMD") == 0) )
                sp->bobneeds_dPoW = 1;
        }
        strcpy(sp->bobgui,"nogui");
        strcpy(sp->alicegui,"nogui");
        if ( LP_swap_finished(sp,1) == 0 ) //sp->finished == 0 && sp->expired == 0 )
        {
            if ( (pubp= LP_pubkeyadd(qp->srchash)) != 0 )
            {
                ptr = calloc(1,sizeof(*ptr));
                ptr->swap = sp;
                DL_APPEND(pubp->bobswaps,ptr);
            }
            if ( (pubp= LP_pubkeyadd(qp->desthash)) != 0 )
            {
                ptr = calloc(1,sizeof(*ptr));
                ptr->swap = sp;
                DL_APPEND(pubp->aliceswaps,ptr);
            }
        }
    } else printf("unexpected LP_swapstats_add failure\n");
    return(sp);
}

int32_t LP_statslog_parsequote(char *method,cJSON *lineobj)
{
    static uint32_t unexpected;
    struct LP_swapstats *sp,*tmp; double qprice; uint32_t requestid,quoteid,timestamp; int32_t i,RTflag,flag,numtrades[LP_MAXPRICEINFOS],methodind,destvout,feevout,duplicate=0; char *statusstr,*gui,*base,*rel; uint64_t aliceid,txfee,satoshis,destsatoshis; bits256 desttxid,feetxid; struct LP_quoteinfo Q; uint64_t basevols[LP_MAXPRICEINFOS],relvols[LP_MAXPRICEINFOS];
    memset(numtrades,0,sizeof(numtrades));
    memset(basevols,0,sizeof(basevols));
    memset(relvols,0,sizeof(relvols));
    memset(&Q,0,sizeof(Q));
    for (i=methodind=0; i<sizeof(LP_stats_methods)/sizeof(*LP_stats_methods); i++)
        if ( strcmp(LP_stats_methods[i],method) == 0 )
        {
            methodind = i;
            break;
        }
    if ( strcmp(method,"tradestatus") == 0 )
    {
        flag = 0;
        aliceid = (uint64_t)juint(lineobj,"aliceid");
        requestid = juint(lineobj,"requestid");
        quoteid = juint(lineobj,"quoteid");
        if ( (sp= LP_swapstats_find(aliceid)) != 0 )
        {
            sp->methodind = methodind;
            sp->Q.R.requestid = requestid;
            sp->Q.R.quoteid = quoteid;
            if ( LP_swapstats_update(sp,&Q,lineobj) == 0 )
                flag = 1;
            //else printf("LP_swapstats_update error\n");
        }
        if ( flag == 0 )
        {
            HASH_ITER(hh,LP_swapstats,sp,tmp)
            {
                static uint32_t counter;
                if ( sp->Q.R.requestid == requestid && sp->Q.R.quoteid == quoteid )
                {
                    sp->methodind = methodind;
                    if ( LP_swapstats_update(sp,&Q,lineobj) == 0 )
                    {
                        flag = 1;
                        break;
                    }
                    if ( counter++ < 1 )
                        printf("error after delayed match\n");
                }
            }
        }
        if ( flag == 0 )
        {
            static uint32_t counter;
            if ( counter++ < 3 )
                printf("unexpected.%d tradestatus aliceid.%llu requestid.%u quoteid.%u\n",unexpected++,(long long)aliceid,requestid,quoteid);//,jprint(lineobj,0));
        }
        return(0);
    }
    if ( LP_quoteparse(&Q,lineobj) < 0 )
    {
        printf("quoteparse_error.(%s)\n",jprint(lineobj,0));
        LP_parse_errors++;
        return(-1);
    }
    else
    {
        gui = jstr(lineobj,"gui");
        if ( gui == 0 || gui[0] == 0 )
            gui = "nogui";
        base = jstr(lineobj,"base");
        rel = jstr(lineobj,"rel");
        satoshis = j64bits(lineobj,"satoshis");
        if ( base == 0 || rel == 0 || satoshis == 0 )
        {
            //printf("quoteparse_error.(%s)\n",jprint(lineobj,0));
            LP_parse_errors++;
            return(-1);
        }
        txfee = j64bits(lineobj,"txfee");
        timestamp = juint(lineobj,"timestamp");
        destsatoshis = j64bits(lineobj,"destsatoshis");
        desttxid = jbits256(lineobj,"desttxid");
        destvout = jint(lineobj,"destvout");
        feetxid = jbits256(lineobj,"feetxid");
        feevout = jint(lineobj,"feevout");
        if ( (statusstr= jstr(lineobj,"status")) != 0 && strcmp(statusstr,"finished") == 0 )
            RTflag = 0;
        else RTflag = 1;
        qprice = ((double)destsatoshis / (satoshis - txfee));
        //printf("%s/v%d %s/v%d\n",bits256_str(str,desttxid),destvout,bits256_str(str2,feetxid),feevout);
        aliceid =  LP_aliceid_calc(desttxid,destvout,feetxid,feevout);
        if ( (sp= LP_swapstats_find(aliceid)) != 0 )
        {
            if ( methodind > sp->methodind )
            {
                sp->methodind = methodind;
                LP_swapstats_update(sp,&Q,lineobj);
            }
            duplicate = 1;
            LP_duplicates++;
        }
        else
        {
            sp = LP_swapstats_create(aliceid,RTflag,&Q,qprice,methodind);
            //printf("create aliceid.%llu\n",(long long)aliceid);
        }
        if ( sp != 0 )
        {
            if ( strcmp(gui,"nogui") != 0 )
            {
                if ( jint(lineobj,"iambob") != 0 )
                    strcpy(sp->bobgui,gui);
                else strcpy(sp->alicegui,gui);
            }
        }
    }
    return(duplicate == 0);
}

cJSON *LP_swapstats_json(struct LP_swapstats *sp)
{
    cJSON *item = cJSON_CreateObject();
    jaddnum(item,"timestamp",sp->Q.timestamp);
    jaddnum(item,"aliceid",sp->aliceid);
    jaddbits256(item,"src",sp->Q.srchash);
    jaddstr(item,"base",sp->Q.srccoin);
    jaddnum(item,"basevol",dstr(sp->Q.satoshis));
    jaddbits256(item,"dest",sp->Q.desthash);
    jaddstr(item,"rel",sp->Q.destcoin);
    jaddnum(item,"relvol",dstr(sp->Q.destsatoshis));
    jaddnum(item,"price",sp->qprice);
    jaddnum(item,"requestid",sp->Q.R.requestid);
    jaddnum(item,"quoteid",sp->Q.R.quoteid);
    jaddnum(item,"finished",sp->finished);
    jaddnum(item,"expired",sp->expired);
    if ( bits256_nonz(sp->bobdeposit) != 0 )
        jaddbits256(item,"bobdeposit",sp->bobdeposit);
    if ( bits256_nonz(sp->alicepayment) != 0 )
        jaddbits256(item,"alicepayment",sp->alicepayment);
    if ( bits256_nonz(sp->bobpayment) != 0 )
        jaddbits256(item,"bobpayment",sp->bobpayment);
    if ( bits256_nonz(sp->paymentspent) != 0 )
        jaddbits256(item,"paymentspent",sp->paymentspent);
    if ( bits256_nonz(sp->Apaymentspent) != 0 )
        jaddbits256(item,"Apaymentspent",sp->Apaymentspent);
    if ( bits256_nonz(sp->depositspent) != 0 )
        jaddbits256(item,"depositspent",sp->depositspent);
    if ( sp->finished == 0 && sp->expired == 0 )
        jaddnum(item,"expires",sp->Q.timestamp + LP_atomic_locktime(sp->Q.srccoin,sp->Q.destcoin)*2 - time(NULL));
    jaddnum(item,"ind",sp->methodind);
    //jaddstr(item,"line",line);
    return(item);
}

char *LP_swapstatus_recv(cJSON *argjson)
{
    struct LP_swapstats *sp; char *statusstr; uint64_t aliceid; double qprice; struct LP_quoteinfo Q; int32_t methodind,RTflag; bits256 txid; //char str[65];
    if ( (aliceid= (uint64_t)juint(argjson,"aliceid")) == 0 )
        return(clonestr("{\"error\":\"LP_swapstatus_recv null aliceid\"}"));
    if ( (sp= LP_swapstats_find(aliceid)) == 0 )
    {
        LP_quoteparse(&Q,argjson);
        if ( Q.satoshis > Q.txfee )
            return(clonestr("{\"error\":\"LP_swapstatus_recv null satoshis\"}"));
        qprice = (double)Q.destsatoshis / (Q.satoshis - Q.txfee);
        if ( (statusstr= jstr(argjson,"status")) != 0 && strcmp(statusstr,"finished") == 0 )
            RTflag = 0;
        else RTflag = 1;
        sp = LP_swapstats_create(aliceid,RTflag,&Q,qprice,LP_TRADESTATUS_METHODIND);
        //printf("create swapstatus from recv\n");
    }
    if ( sp != 0 )
    {
        if ( 0 && IAMLP == 0 )
            printf("swapstatus.(%s)\n",jprint(argjson,0));
        sp->lasttime = (uint32_t)time(NULL);
        if ( (methodind= jint(argjson,"ind")) > sp->methodind && methodind < sizeof(LP_stats_methods)/sizeof(*LP_stats_methods) )
        {
            if ( 0 && sp->finished == 0 && sp->expired == 0 )
                printf("SWAPSTATUS updated %llu %s %u %u\n",(long long)sp->aliceid,LP_stats_methods[sp->methodind],juint(argjson,"finished"),juint(argjson,"expired"));
            sp->methodind = methodind;
            sp->finished = juint(argjson,"finished");
            sp->expired = juint(argjson,"expired");
            txid = jbits256(argjson,"bobdeposit");
            if ( bits256_nonz(txid) != 0 && bits256_nonz(sp->bobdeposit) == 0 )
            {
                sp->bobdeposit = txid;
                //printf("set aliceid.%llu bobdeposit %s\n",(long long)sp->aliceid,bits256_str(str,txid));
            }
            txid = jbits256(argjson,"alicepayment");
            if ( bits256_nonz(txid) != 0 && bits256_nonz(sp->alicepayment) == 0 )
            {
                sp->alicepayment = txid;
                //printf("set aliceid.%llu alicepayment %s\n",(long long)sp->aliceid,bits256_str(str,txid));
            }
            txid = jbits256(argjson,"bobpayment");
            if ( bits256_nonz(txid) != 0 && bits256_nonz(sp->bobpayment) == 0 )
            {
                sp->bobpayment = txid;
                //printf("set aliceid.%llu bobpayment %s\n",(long long)sp->aliceid,bits256_str(str,txid));
            }
            txid = jbits256(argjson,"paymentspent");
            if ( bits256_nonz(txid) != 0 && bits256_nonz(sp->paymentspent) == 0 )
            {
                sp->paymentspent = txid;
                //printf("set aliceid.%llu paymentspent %s\n",(long long)sp->aliceid,bits256_str(str,txid));
            }
            txid = jbits256(argjson,"Apaymentspent");
            if ( bits256_nonz(txid) != 0 && bits256_nonz(sp->Apaymentspent) == 0 )
            {
                sp->Apaymentspent = txid;
                //printf("set aliceid.%llu Apaymentspent %s\n",(long long)sp->aliceid,bits256_str(str,txid));
            }
            txid = jbits256(argjson,"depositspent");
            if ( bits256_nonz(txid) != 0 && bits256_nonz(sp->depositspent) == 0 )
            {
                sp->depositspent = txid;
                //printf("set aliceid.%llu depositspent %s\n",(long long)sp->aliceid,bits256_str(str,txid));
            }
        }
    }
    return(clonestr("{\"result\":\"success\"}"));
}

char *LP_gettradestatus(uint64_t aliceid,uint32_t requestid,uint32_t quoteid)
{
    struct LP_swapstats *sp; struct iguana_info *bob,*alice; char *swapstr,*statusstr; cJSON *reqjson,*swapjson; bits256 zero;
    //printf("gettradestatus.(%llu)\n",(long long)aliceid);
    if ( IAMLP != 0 )
    {
        if ( (sp= LP_swapstats_find(aliceid)) != 0 && sp->Q.satoshis != 0 && sp->Q.destsatoshis != 0 && bits256_nonz(sp->bobdeposit) != 0 )
        {
            if ( time(NULL) > sp->lasttime+60 )
            {
                if ( (reqjson= LP_swapstats_json(sp)) != 0 )
                {
                    jaddstr(reqjson,"method","swapstatus");
                    memset(zero.bytes,0,sizeof(zero));
                    LP_reserved_msg(0,zero,jprint(reqjson,1));
                }
                if ( (bob= LP_coinfind(sp->Q.srccoin)) != 0 )
                    LP_dPoW_broadcast(bob);
                if ( (alice= LP_coinfind(sp->Q.destcoin)) != 0 )
                    LP_dPoW_broadcast(alice);
            }
            return(clonestr("{\"result\":\"success\"}"));
        }
    }
    if ( (swapstr= basilisk_swapentry(1,requestid,quoteid,0)) != 0 )
    {
        if ( (swapjson= cJSON_Parse(swapstr)) != 0 )
        {
            if ( (statusstr= jstr(swapjson,"status")) != 0 && strcmp(statusstr,"finished") == 0 )
            {
                jaddstr(swapjson,"method","swapstatus");
                memset(zero.bytes,0,sizeof(zero));
                printf("send local swapstatus\n");
                LP_reserved_msg(0,zero,jprint(swapjson,0));
            }
            free_json(swapjson);
        }
        free(swapstr);
    }
    return(clonestr("{\"result\":\"success\"}"));
}

int32_t LP_stats_dispiter(cJSON *array,struct LP_swapstats *sp,uint32_t starttime,uint32_t endtime,char *refbase,char *refrel,char *refgui,bits256 refpubkey)
{
    int32_t dispflag,retval = 0;
    if ( sp->finished == 0 && sp->expired == 0 && time(NULL) > sp->Q.timestamp+LP_atomic_locktime(sp->Q.srccoin,sp->Q.destcoin)*2 )
        sp->expired = (uint32_t)time(NULL);
    if ( LP_swap_finished(sp,1) > 0 )
        retval = 1;
    dispflag = 0;
    if ( starttime == 0 && endtime == 0 )
        dispflag = 1;
    else if ( starttime > time(NULL) && endtime == starttime && sp->finished == 0 && sp->expired == 0 )
        dispflag = 1;
    else if ( sp->Q.timestamp >= starttime && sp->Q.timestamp <= endtime )
        dispflag = 1;
    if ( refbase != 0 && refbase[0] != 0 && strcmp(refbase,sp->Q.srccoin) != 0 && strcmp(refbase,sp->Q.destcoin) != 0 )
        dispflag = 0;
    if ( refrel != 0 && refrel[0] != 0 && strcmp(refrel,sp->Q.srccoin) != 0 && strcmp(refrel,sp->Q.destcoin) != 0 )
        dispflag = 0;
    if ( dispflag != 0 )
    {
        dispflag = 0;
        if ( refgui == 0 || refgui[0] == 0 || strcmp(refgui,sp->bobgui) == 0 || strcmp(refgui,sp->alicegui) == 0 )
        {
            if ( bits256_nonz(refpubkey) == 0 || bits256_cmp(refpubkey,sp->Q.srchash) == 0 || bits256_cmp(refpubkey,sp->Q.desthash) == 0 )
                dispflag = 1;
        }
    }
    if ( dispflag != 0 )
        jaddi(array,LP_swapstats_json(sp));
    return(retval);
}

cJSON *LP_statslog_disp(uint32_t starttime,uint32_t endtime,char *refgui,bits256 refpubkey,char *refbase,char *refrel)
{
    static int32_t rval;
    cJSON *retjson,*array,*item; struct LP_pubkey_info *pubp,*ptmp; uint32_t now; struct LP_swapstats *sp,*tmp; int32_t i,n,numtrades[LP_MAXPRICEINFOS]; uint64_t basevols[LP_MAXPRICEINFOS],relvols[LP_MAXPRICEINFOS];
    if ( rval == 0 )
        rval = (LP_rand() % 300) + 60;
    if ( starttime > endtime )
        starttime = endtime;
    n = LP_statslog_parse();
    memset(basevols,0,sizeof(basevols));
    memset(relvols,0,sizeof(relvols));
    memset(numtrades,0,sizeof(numtrades));
    retjson = cJSON_CreateObject();
    jaddstr(retjson,"result","success");
    jaddnum(retjson,"newlines",n);
    array = cJSON_CreateArray();
    LP_RTcount = LP_swapscount = 0;
    now = (uint32_t)time(NULL);
    HASH_ITER(hh,LP_RTstats,sp,tmp)
    {
        if ( LP_stats_dispiter(array,sp,starttime,endtime,refbase,refrel,refgui,refpubkey) > 0 )
        {
            portable_mutex_lock(&LP_statslogmutex);
            HASH_DELETE(hh,LP_RTstats,sp);
            HASH_ADD(hh,LP_swapstats,aliceid,sizeof(sp->aliceid),sp);
            portable_mutex_unlock(&LP_statslogmutex);
        }
        else
        {
            LP_RTcount++;
            /*if ( now > sp->lasttime+rval )
            {
                reqjson = cJSON_CreateObject();
                jaddstr(reqjson,"method","gettradestatus");
                jadd64bits(reqjson,"aliceid",sp->aliceid);
                memset(zero.bytes,0,sizeof(zero));
                LP_reserved_msg(0,"","",zero,jprint(reqjson,1));
            }*/
        }
    }
    HASH_ITER(hh,LP_swapstats,sp,tmp)
    {
        LP_stats_dispiter(array,sp,starttime,endtime,refbase,refrel,refgui,refpubkey);
        LP_swapscount++;
    }
    //printf("RT.%d completed.%d\n",LP_RTcount,LP_swapscount);
    jadd(retjson,"swaps",array);
    jaddnum(retjson,"RTcount",LP_RTcount);
    jaddnum(retjson,"swapscount",LP_swapscount);
    array = cJSON_CreateArray();
    for (i=0; i<LP_MAXPRICEINFOS; i++)
    {
        if ( basevols[i] != 0 || relvols[i] != 0 )
        {
            item = cJSON_CreateObject();
            jaddstr(item,"coin",LP_priceinfostr(i));
            jaddnum(item,"srcvol",dstr(basevols[i]));
            jaddnum(item,"destvol",dstr(relvols[i]));
            jaddnum(item,"numtrades",numtrades[i]);
            jaddnum(item,"total",dstr(basevols[i] + relvols[i]));
            jaddi(array,item);
        }
    }
    jadd(retjson,"volumes",array);
    jaddnum(retjson,"request",LP_requests);
    jaddnum(retjson,"reserved",LP_reserveds);
    jaddnum(retjson,"connect",LP_connects);
    jaddnum(retjson,"connected",LP_connecteds);
    jaddnum(retjson,"duplicates",LP_duplicates);
    jaddnum(retjson,"parse_errors",LP_parse_errors);
    jaddnum(retjson,"uniques",LP_aliceids);
    jaddnum(retjson,"tradestatus",LP_tradestatuses);
    jaddnum(retjson,"unknown",LP_unknowns);
    return(retjson);
}

char *LP_ticker(char *refbase,char *refrel)
{
    cJSON *logjson,*retjson,*item,*retitem,*swapsjson; double basevol,relvol; char *base,*rel; int32_t i,n; bits256 zero; uint32_t now = (uint32_t)time(NULL);
    memset(zero.bytes,0,sizeof(zero));
    if ( (logjson= LP_statslog_disp(now - 3600*24,now,"",zero,refbase,refrel)) != 0 )
    {
        retjson = cJSON_CreateArray();
        if ( (swapsjson= jarray(&n,logjson,"swaps")) != 0 )
        {
            for (i=n-1; i>=0; i--)
            {
                item = jitem(swapsjson,i);
                retitem = cJSON_CreateObject();
                if ( (base= jstr(item,"base")) != 0 && (rel= jstr(item,"rel")) != 0 && (basevol= jdouble(item,"basevol")) > SMALLVAL )
                {
                    relvol = jdouble(item,"relvol");
                    jaddnum(retitem,"timestamp",juint(item,"timestamp"));
                    jaddnum(retitem,base,basevol);
                    jaddnum(retitem,rel,relvol);
                    jaddnum(retitem,"price",relvol/basevol);
                }
                jaddi(retjson,retitem);
            }
        }
        free_json(logjson);
        return(jprint(retjson,1));
    } else return(clonestr("{\"error\":\"couldnt get logjson\"}"));
}

struct LP_ohlc
{
    uint32_t timestamp,firsttime,lasttime,numtrades;
    double high,low,open,close,relsum,basesum;
};

cJSON *LP_ohlc_json(struct LP_ohlc *bar,struct LP_ohlc *prevbar)
{
    cJSON *item; struct LP_ohlc tmp;
    memset(&tmp,0,sizeof(tmp));
    if ( bar->numtrades == 0 )
    {
        memset(&tmp,0,sizeof(tmp));
        tmp.timestamp = bar->timestamp;
        tmp.open = tmp.high = tmp.low = tmp.close = prevbar->close;
        tmp.numtrades = 0;
        tmp.relsum = tmp.basesum = 0.;
    } else tmp = *bar;
    bar = &tmp;
    item = cJSON_CreateArray();
    jaddinum(item,bar->timestamp);
    jaddinum(item,bar->high);
    jaddinum(item,bar->low);
    jaddinum(item,bar->open);
    jaddinum(item,bar->close);
    jaddinum(item,bar->relsum);
    jaddinum(item,bar->basesum);
    if ( bar->basesum != 0 )
        jaddinum(item,bar->relsum / bar->basesum);
    else jaddinum(item,0);
    jaddinum(item,bar->numtrades);
    return(item);
}

void LP_ohlc_update(struct LP_ohlc *bar,uint32_t timestamp,double basevol,double relvol)
{
    double price;
    if ( basevol > SMALLVAL && relvol > SMALLVAL )
    {
        price = relvol / basevol;
        if ( bar->firsttime == 0 || timestamp < bar->firsttime )
        {
            bar->firsttime = timestamp;
            bar->open = price;
        }
        if ( bar->lasttime == 0 || timestamp > bar->lasttime )
        {
            bar->lasttime = timestamp;
            bar->close = price;
        }
        if ( bar->low == 0. || price < bar->low )
            bar->low = price;
        if ( bar->high == 0. || price > bar->high )
            bar->high = price;
        bar->basesum += basevol;
        bar->relsum += relvol;
        bar->numtrades++;
        //printf("%d %.8f/%.8f -> %.8f\n",bar->numtrades,basevol,relvol,price);
    }
}

cJSON *LP_tradesarray(char *refbase,char *refrel,uint32_t starttime,uint32_t endtime,int32_t timescale)
{
    struct LP_ohlc *bars,nonz; cJSON *array,*item,*statsjson,*swaps; uint32_t timestamp; bits256 zero; char *base,*rel; int32_t i,n,numbars,bari;
    if ( timescale < 60 )
        return(cJSON_Parse("{\"error\":\"one minute is shortest timescale\"}"));
    memset(zero.bytes,0,sizeof(zero));
    if ( endtime == 0 )
        endtime = (((uint32_t)time(NULL) / timescale) * timescale);
    if ( starttime == 0 || starttime >= endtime )
        starttime = (endtime - LP_SCREENWIDTH*timescale);
    numbars = ((endtime - starttime) / timescale) + 1;
    bars = calloc(numbars,sizeof(*bars));
    for (bari=0; bari<numbars; bari++)
        bars[bari].timestamp = starttime + bari*timescale;
    if ( (statsjson= LP_statslog_disp(starttime,endtime,"",zero,refbase,refrel)) != 0 )
    {
        if ( (swaps= jarray(&n,statsjson,"swaps")) != 0 )
        {
            for (i=0; i<n; i++)
            {
                item = jitem(swaps,i);
                if ( (timestamp= juint(item,"timestamp")) != 0 && timestamp >= starttime && timestamp <= endtime )
                {
                    bari = (timestamp - starttime) / timescale;
                    base = jstr(item,"base");
                    rel = jstr(item,"rel");
                    if ( strcmp(base,refbase) == 0 && strcmp(rel,refrel) == 0 )
                    LP_ohlc_update(&bars[bari],timestamp,jdouble(item,"basevol"),jdouble(item,"relvol"));
                    else if ( strcmp(rel,refbase) == 0 && strcmp(base,refrel) == 0 )
                        LP_ohlc_update(&bars[bari],timestamp,jdouble(item,"relvol"),jdouble(item,"basevol"));
                } else printf("skip.(%s)\n",jprint(item,0));
            }
        }
        free_json(statsjson);
    }
    array = cJSON_CreateArray();
    memset(&nonz,0,sizeof(nonz));
    for (bari=0; bari<numbars; bari++)
    {
        if ( (item= LP_ohlc_json(&bars[bari],&nonz)) != 0 )
        {
            jaddi(array,item);
            if ( bars[bari].numtrades > 0 )
                nonz = bars[bari];
        }
    }
    free(bars);
    return(array);
}

