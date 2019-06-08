
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
//  LP_commands.c
//  marketmaker
//

char *LP_numutxos()
{
    cJSON *retjson = cJSON_CreateObject();
    if ( LP_mypeer != 0 )
    {
        jaddstr(retjson,"ipaddr",LP_mypeer->ipaddr);
        jaddnum(retjson,"port",LP_mypeer->port);
        //jaddnum(retjson,"numutxos",LP_mypeer->numutxos);
        jaddnum(retjson,"numpeers",LP_mypeer->numpeers);
        jaddnum(retjson,"session",G.LP_sessionid);
    } else jaddstr(retjson,"error","client node");
    return(jprint(retjson,1));
}

char *stats_JSON(void *ctx,char *myipaddr,int32_t pubsock,cJSON *argjson,char *remoteaddr,uint16_t port,int32_t authenticated, uint8_t loopback_only) // from rpc port
{
    char *method,*userpass,*base,*rel,*coin,*retstr = 0; int32_t flag = 0; cJSON *retjson,*reqjson = 0; struct iguana_info *ptr;
    method = jstr(argjson,"method");
    if ( method != 0 && (strcmp(method,"addr_unspents") == 0 || strcmp(method,"uitem") == 0 || strcmp(method,"postutxos") == 0) )
        return(0);
//printf("stats_JSON.(%s)\n",jprint(argjson,0));
    /*if ( (ipaddr= jstr(argjson,"ipaddr")) != 0 && (argport= juint(argjson,"port")) != 0 && (method == 0 || strcmp(method,"electrum") != 0) )
    {
        if ( strcmp(ipaddr,"127.0.0.1") != 0 && argport >= 1000 )
        {
            flag = 1;
            if ( (pushport= juint(argjson,"push")) == 0 )
                pushport = argport + 1;
            if ( (subport= juint(argjson,"sub")) == 0 )
                subport = argport + 2;
            if ( (peer= LP_peerfind((uint32_t)calc_ipbits(ipaddr),argport)) != 0 )
            {
                if ( 0 && (otherpeers= jint(argjson,"numpeers")) > peer->numpeers )
                    peer->numpeers = otherpeers;
                if ( peer->sessionid == 0 )
                    peer->sessionid = juint(argjson,"session");
                //printf("peer.(%s) found (%d %d) (%d %d) (%s)\n",peer->ipaddr,peer->numpeers,peer->numutxos,otherpeers,othernumutxos,jprint(argjson,0));
            } else LP_addpeer(LP_mypeer,LP_mypubsock,ipaddr,argport,pushport,subport,jint(argjson,"numpeers"),jint(argjson,"numutxos"),juint(argjson,"session"));
        }
    }*/
    if ( method == 0 )
    {
        if ( is_cJSON_Array(argjson) != 0 )
            printf("RAWARRAY command? %s\n",jprint(argjson,0));
        if ( flag == 0 || jobj(argjson,"result") != 0 )
            printf("stats_JSON] no method: (%s)\n",jprint(argjson,0));
        return(0);
    }
    if ( strcmp(method,"hello") == 0 )
    {
        //int32_t i; cJSON *array = cJSON_CreateArray();
        retjson = cJSON_CreateObject();
        jaddstr(retjson,"result","success");
        jaddstr(retjson,"status","got hello");
        //for (i=0; i<10000; i++)
        //    jaddinum(array,i);
        //jadd(retjson,"array",array);
        return(jprint(retjson,1));
        //printf("got hello from %s:%u\n",ipaddr!=0?ipaddr:"",argport);
        //return(clonestr("{\"result\":\"success\",\"status\":\"got hello\"}"));
    }
    /*else if ( strcmp(method,"sendmessage") == 0 && jobj(argjson,"userpass") == 0 )
     {
         static char *laststr;
         char *newstr; bits256 pubkey = jbits256(argjson,"pubkey");
         if ( bits256_nonz(pubkey) == 0 || bits256_cmp(pubkey,G.LP_mypub25519) == 0 )
         {
             newstr = jprint(argjson,0);
             if ( laststr == 0 || strcmp(laststr,newstr) != 0 )
             {
                 printf("got message.(%s) from %s:%u\n",newstr,ipaddr!=0?ipaddr:"",argport);
                 if ( laststr != 0 )
                     free(laststr);
                 laststr = newstr;
                 LP_gotmessage(argjson);
                 retstr = clonestr(laststr);
             }
         } else retstr = clonestr("{\"error\":\"duplicate message\"}");
     }*/
    //else if ( strcmp(method,"nn_tests") == 0 )
    //    return(clonestr("{\"result\":\"success\"}"));
    if ( (base= jstr(argjson,"base")) == 0 )
        base = "";
    if ((rel= jstr(argjson,"rel")) == 0 )
        rel = "";
    if ( (coin= jstr(argjson,"coin")) == 0 )
        coin = "";

    if ( G.USERPASS[0] != 0 && (loopback_only == 0 || is_loopback_ip(remoteaddr) == 1) && port != 0 && strcmp(method,"psock") != 0 ) // protected localhost
    {
        if ( G.USERPASS_COUNTER == 0 )
        {
            G.USERPASS_COUNTER = 1;
            LP_cmdcount++;
        }
        char passhashstr[65]; bits256_str(passhashstr,G.LP_passhash);
        if ( authenticated == 0 && ((userpass= jstr(argjson,"userpass")) == 0 || (strcmp(userpass,G.USERPASS) != 0 && strcmp(userpass,passhashstr) != 0)) )
            return(clonestr("{\"error\":\"authentication error you need to make sure userpass is set\"}"));
        if ( jobj(argjson,"userpass") != 0 )
            jdelete(argjson,"userpass");
        LP_cmdcount++;
        if ( strcmp(method,"jpg") == 0 )
        {
            return(LP_jpg(jstr(argjson,"srcfile"),jstr(argjson,"destfile"),jint(argjson,"power2"),jstr(argjson,"password"),jstr(argjson,"data"),jint(argjson,"required"),juint(argjson,"ind")));
        }
        else if ( strcmp(method,"cancel") == 0 )
        {
            return(LP_cancel_order(jstr(argjson,"uuid")));
        }
        else if ( strcmp(method,"recentswaps") == 0 )
        {
            return(LP_recent_swaps(jint(argjson,"limit"),0));
        }
        else if ( strcmp(method,"millis") == 0 )
        {
            LP_millistats_update(0);
            return(clonestr("{\"result\":\"success\"}"));
        }
        else if ( strcmp(method,"sleep") == 0 )
        {
            if ( jint(argjson,"seconds") == 0 )
                sleep(180);
            else sleep(jint(argjson,"seconds"));
            return(clonestr("{\"result\":\"success\",\"status\":\"feeling good after sleeping\"}"));
        }
        else if ( strcmp(method,"getprices") == 0 )
            return(LP_prices());
        else if ( strcmp(method,"getpeers") == 0 )
            return(LP_peers());
        else if ( strcmp(method,"getcoins") == 0 )
            return(jprint(LP_coinsjson(0),1));  // Was also superficially ported in RPC `passphrase`.
        else if ( strcmp(method,"notarizations") == 0 )
        {
            if ( (ptr= LP_coinsearch(coin)) != 0 )
            {
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"result","success");
                jaddstr(retjson,"coin",coin);
                jaddnum(retjson,"lastnotarization",ptr->notarized);
                jaddnum(retjson,"bestheight",ptr->height);
                return(jprint(retjson,1));
            } else return(clonestr("{\"error\":\"cant find coin\"}"));
        }
        else if ( strcmp(method,"calcaddress") == 0 )
        {
            bits256 privkey,pub; uint8_t pubtype,wiftaddr,p2shtype,taddr,wiftype,pubkey33[33]; char *passphrase,coinaddr[64],wifstr[64],pubsecp[67];
            if ( (passphrase= jstr(argjson,"passphrase")) != 0 )
            {
                conv_NXTpassword(privkey.bytes,pub.bytes,(uint8_t *)passphrase,(int32_t)strlen(passphrase));
                privkey.bytes[0] &= 248, privkey.bytes[31] &= 127, privkey.bytes[31] |= 64;
                if ( (coin= jstr(argjson,"coin")) == 0 || (ptr= LP_coinfind(coin)) == 0 )
                {
                    coin = "KMD";
                    taddr = 0;
                    pubtype = 60;
                    p2shtype = 85;
                    wiftype = 188;
                    wiftaddr = 0;
                }
                else
                {
                    coin = ptr->symbol;
                    taddr = ptr->taddr;
                    pubtype = ptr->pubtype;
                    p2shtype = ptr->p2shtype;
                    wiftype = ptr->wiftype;
                    wiftaddr = ptr->wiftaddr;
                }
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"passphrase",passphrase);
                bitcoin_priv2pub(ctx,coin,pubkey33,coinaddr,privkey,taddr,pubtype);
                init_hexbytes_noT(pubsecp,pubkey33,33);
                jaddstr(retjson,"pubsecp",pubsecp);
                jaddstr(retjson,"coinaddr",coinaddr);
                bitcoin_priv2pub(ctx,coin,pubkey33,coinaddr,privkey,taddr,p2shtype);
                jaddstr(retjson,"p2shaddr",coinaddr);
                jaddbits256(retjson,"privkey",privkey);
                bitcoin_priv2wif(coin,wiftaddr,wifstr,privkey,wiftype);
                jaddstr(retjson,"wif",wifstr);
                return(jprint(retjson,1));
            } else return(clonestr("{\"error\":\"need to have passphrase\"}"));
        }
        else if ( strcmp(method,"statsdisp") == 0 )
        {
            return(jprint(LP_statslog_disp(juint(argjson,"starttime"),juint(argjson,"endtime"),jstr(argjson,"gui"),jbits256(argjson,"pubkey"),jstr(argjson,"base"),jstr(argjson,"rel")),1));
        }
        else if ( strcmp(method,"ticker") == 0 )
        {
            return(LP_ticker(jstr(argjson,"base"),jstr(argjson,"rel")));
        }
        else if ( strcmp(method,"gen64addrs") == 0 )
        {
            uint8_t taddr,pubtype;
            pubtype = (jobj(argjson,"pubtype") == 0) ? 60 : juint(argjson,"pubtype");
            taddr = (jobj(argjson,"taddr") == 0) ? 0 : juint(argjson,"taddr");
            return(LP_gen64addrs(ctx,jstr(argjson,"passphrase"),taddr,pubtype));
        }
        else if ( strcmp(method,"secretaddresses") == 0 )
        {
            uint8_t taddr,pubtype;
            pubtype = (jobj(argjson,"pubtype") == 0) ? 60 : juint(argjson,"pubtype");
            taddr = (jobj(argjson,"taddr") == 0) ? 0 : juint(argjson,"taddr");
            return(LP_secretaddresses(ctx,jstr(argjson,"prefix"),jstr(argjson,"passphrase"),juint(argjson,"num"),taddr,pubtype));
        }
        else if ( strcmp(method,"kickstart") == 0 )
        {
            uint32_t requestid,quoteid;
            if ( (requestid= juint(argjson,"requestid")) != 0 && (quoteid= juint(argjson,"quoteid")) != 0 )
                return(LP_kickstart(requestid,quoteid));
            else return(clonestr("{\"error\":\"kickstart needs requestid and quoteid\"}"));
        }
        else if ( strcmp(method,"dynamictrust") == 0 )
        {
            struct LP_address *ap; char *coinaddr;
            if ( (ptr= LP_coinsearch("KMD")) != 0 && (coinaddr= jstr(argjson,"address")) != 0 )
            {
                //LP_zeroconf_deposits(ptr);
                if ( (ap= LP_addressfind(ptr,coinaddr)) != 0 )
                {
                    retjson = cJSON_CreateObject();
                    jaddstr(retjson,"result","success");
                    jaddstr(retjson,"address",coinaddr);
                    jaddnum(retjson,"zcredits",dstr(ap->instantdex_credits));
                    return(jprint(retjson,1));
                }
            }
            return(clonestr("{\"error\":\"cant find address\"}"));
        }
        else if ( strcmp(method,"inuse") == 0 )
            return(jprint(LP_inuse_json(),1));
        else if ( (retstr= LP_istradebots_command(ctx,pubsock,method,argjson)) != 0 )
            return(retstr);
        if ( base[0] != 0 && rel[0] != 0 )
        {
            double price,bid,ask;
            if ( strcmp(method,"pricearray") == 0 )
            {
                uint32_t firsttime;
                if ( base[0] != 0 && rel[0] != 0 )
                {
                    if ( (firsttime= juint(argjson,"starttime")) < time(NULL)-30*24*3600 )
                        firsttime = (uint32_t)(time(NULL)-30*24*3600);
                    return(jprint(LP_pricearray(base,rel,firsttime,juint(argjson,"endtime"),jint(argjson,"timescale")),1));
                } else return(clonestr("{\"error\":\"pricearray needs base and rel\"}"));
            }
            else if ( strcmp(method,"tradesarray") == 0 )
            {
                return(jprint(LP_tradesarray(base,rel,juint(argjson,"starttime"),juint(argjson,"endtime"),jint(argjson,"timescale")),1));
            }
            else if ( strcmp(method,"getprice") == 0 || strcmp(method,"getmyprice") == 0 )
            {
                double price,bid,ask;
                if ( strcmp(method,"getprice") == 0 )
                {
                    ask = LP_price(1,base,rel);
                    if ( (bid= LP_price(1,rel,base)) > SMALLVAL )
                        bid = 1./bid;
                }
                else
                {
                    ask = LP_getmyprice(1,base,rel);
                    if ( (bid= LP_getmyprice(1,rel,base)) > SMALLVAL )
                        bid = 1./bid;
                }
                price = _pairaved(bid,ask);
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"result","success");
                jaddstr(retjson,"base",base);
                jaddstr(retjson,"rel",rel);
                jaddnum(retjson,"timestamp",time(NULL));
                jaddnum(retjson,"bid",bid);
                jaddnum(retjson,"ask",ask);
                jaddnum(retjson,"price",price);
                return(jprint(retjson,1));
            }
            else if ( strcmp(method,"orderbook") == 0 )
                return(LP_orderbook(base,rel,jint(argjson,"duration")));
            if ( IAMLP == 0 && LP_isdisabled(base,rel) != 0 )
                return(clonestr("{\"error\":\"at least one of coins disabled\"}"));
            price = jdouble(argjson,"price");
            if ( strcmp(method,"myprice") == 0 )
            {
                if ( LP_myprice(&bid,&ask,base,rel) > SMALLVAL )
                {
                    retjson = cJSON_CreateObject();
                    jaddstr(retjson,"base",base);
                    jaddstr(retjson,"rel",rel);
                    jaddnum(retjson,"bid",bid);
                    jaddnum(retjson,"ask",ask);
                    return(jprint(retjson,1));
                } else return(clonestr("{\"error\":\"no price set\"}"));
            }
        }
        else if ( coin[0] != 0 )
        {
            if ( strcmp(method,"disable") == 0 )
            {
                //*
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    ptr->inactive = (uint32_t)time(NULL);
                    cJSON *array = cJSON_CreateArray();
                    jaddi(array,LP_coinjson(ptr,0));
                    return(jprint(array,1));
                } else return(clonestr("{\"error\":\"couldnt find coin\"}"));
            }
            else if ( strcmp(method,"balance") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(jprint(LP_address_balance(ptr,jstr(argjson,"address"),1),1));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"getfee") == 0 )
            {
                uint64_t txfee;
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    txfee = LP_txfeecalc(ptr,0,0);
                    retjson = cJSON_CreateObject();
                    jaddstr(retjson,"result","success");
                    jaddstr(retjson,"coin",coin);
                    jaddnum(retjson,"txfee",dstr(txfee));
                    return(jprint(retjson,1));
                } else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"convaddress") == 0 )
            {
                return(LP_convaddress(coin,jstr(argjson,"address"),jstr(argjson,"destcoin")));
            }
            // cJSON *LP_listtransactions(char *symbol,char *coinaddr,int32_t count,int32_t skip)
            else if ( strcmp(method,"setconfirms") == 0 )
            {
                int32_t n;
                n = jint(argjson,"numconfirms");
                if ( n < 0 )
                    return(clonestr("{\"error\":\"illegal numconfirms\"}"));
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                {
                    ptr->userconfirms = n;
                    if ( (n= jint(argjson,"maxconfirms")) > 0 )
                        ptr->maxconfirms = n;
                    if ( ptr->maxconfirms > 0 && ptr->userconfirms > ptr->maxconfirms )
                        ptr->userconfirms = ptr->maxconfirms;
                    return(clonestr("{\"result\":\"success\"}"));
                } else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"snapshot") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(jprint(LP_snapshot(ptr,juint(argjson,"height")),1));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"dividends") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(LP_dividends(ptr,juint(argjson,"height"),argjson));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            else if ( strcmp(method,"snapshot_balance") == 0 )
            {
                if ( (ptr= LP_coinsearch(coin)) != 0 )
                    return(LP_snapshot_balance(ptr,juint(argjson,"height"),argjson));
                else return(clonestr("{\"error\":\"cant find coind\"}"));
            }
            if ( LP_isdisabled(coin,0) != 0 )
            {
                retjson = cJSON_CreateObject();
                jaddstr(retjson,"error",LP_DONTCHANGE_ERRMSG1);
                return(jprint(retjson,1));
            }
            else if ( strcmp(method,"getcoin") == 0 )
                return(LP_getcoin(coin));
        }
        else if ( strcmp(method,"lastnonce") == 0 )
        {
            cJSON *retjson = cJSON_CreateObject();
            jaddstr(retjson,"result","success");
            jaddnum(retjson,"lastnonce",LP_lastnonce);
            return(jprint(retjson,1));
        }
        else if ( strcmp(method,"myprices") == 0 )
            return(LP_myprices(1));
        else if ( strcmp(method,"trust") == 0 )
            return(LP_pubkey_trustset(jbits256(argjson,"pubkey"),jint(argjson,"trust")));
        else if ( strcmp(method,"trusted") == 0 )
            return(LP_pubkey_trusted());
    } // end of protected localhost commands
    if ( IAMLP == 0 )
    {
        if ( (reqjson= LP_dereference(argjson,"broadcast")) != 0 )
        {
            if ( jobj(reqjson,"method2") != 0 )
            {
                jdelete(reqjson,"method");
                method = jstr(reqjson,"method2");
                jaddstr(reqjson,"method",method);
            }
            argjson = reqjson;
        }
        if ( strcmp(method,"getdPoW") == 0 )
            retstr = clonestr("{\"result\":\"success\"}");
    }
    else
    {
        if ( strcmp(method,"tradesarray") == 0 )
        {
            return(jprint(LP_tradesarray(base,rel,juint(argjson,"starttime"),juint(argjson,"endtime"),jint(argjson,"timescale")),1));
        }
        else if ( strcmp(method,"getdPoW") == 0 )
        {
            if ( (ptr= LP_coinfind(jstr(argjson,"coin"))) != 0 )
                LP_dPoW_broadcast(ptr);
            retstr = clonestr("{\"result\":\"success\"}");
        }
    }
    // received response
    if ( strcmp(method,"swapstatus") == 0 )
        return(LP_swapstatus_recv(argjson));
    else if ( strcmp(method,"gettradestatus") == 0 )
    {
        retstr = clonestr("{\"error\":\"deprecated\"}");
        //return(LP_gettradestatus(j64bits(argjson,"aliceid"),juint(argjson,"requestid"),juint(argjson,"quoteid")));
    }
    else if ( strcmp(method,"dPoW") == 0 )
        return(LP_dPoW_recv(argjson));
    else if ( strcmp(method,"getpeers") == 0 )
        return(LP_peers());
    else if ( strcmp(method,"balances") == 0 )
        return(jprint(LP_balances(jstr(argjson,"address")),1));
    else if ( strcmp(method,"getprice") == 0 || strcmp(method,"getmyprice") == 0 )
    {
        double price,bid,ask;
        if ( strcmp(method,"getprice") == 0 )
        {
            ask = LP_price(1,base,rel);
            if ( (bid= LP_price(1,rel,base)) > SMALLVAL )
                bid = 1./bid;
        }
        else
        {
            ask = LP_getmyprice(1,base,rel);
            if ( (bid= LP_getmyprice(1,rel,base)) > SMALLVAL )
                bid = 1./bid;
        }
        price = _pairaved(bid,ask);
        retjson = cJSON_CreateObject();
        jaddstr(retjson,"result","success");
        jaddstr(retjson,"base",base);
        jaddstr(retjson,"rel",rel);
        jaddnum(retjson,"timestamp",time(NULL));
        jaddnum(retjson,"bid",bid);
        jaddnum(retjson,"ask",ask);
        jaddnum(retjson,"price",price);
        return(jprint(retjson,1));
    }
    /*else if ( strcmp(method,"getpeers") == 0 )
    {
        char *tmpstr;
        if ( (tmpstr= jstr(argjson,"LPnode")) != 0 )
            LP_addpeer(LP_mypeer,LP_mypubsock,tmpstr,RPC_port,RPC_port+10,RPC_port+20,1,G.LP_sessionid);
        if ( IAMLP != 0 )
        {
            printf("send peers list %s\n",LP_peers());
            bits256 zero; memset(zero.bytes,0,sizeof(zero));
            LP_reserved_msg(0,"","",zero,LP_peers());
        }
        retstr = clonestr("{\"result\":\"success\"}");
    }*/
    // end received response
    
    else if ( strcmp(method,"tradestatus") == 0 )
    {
        LP_tradecommand_log(argjson);
        //printf("%-4d tradestatus | aliceid.%llu RT.%d %d\n",(uint32_t)time(NULL) % 3600,(long long)j64bits(argjson,"aliceid"),LP_RTcount,LP_swapscount);
        retstr = clonestr("{\"result\":\"success\"}");
    }
    else if ( strcmp(method,"addr_unspents") == 0 )
    {
        //printf("GOT ADDR_UNSPENTS %s %s\n",jstr(argjson,"coin"),jstr(argjson,"address"));
        if ( (ptr= LP_coinsearch(coin)) != 0 )
        {
            char *coinaddr;
            if ( (coinaddr= jstr(argjson,"address")) != 0 )
            {
                if ( coinaddr[0] != 0 )
                {
                    LP_address(ptr,coinaddr);
                    if ( strcmp(coinaddr,ptr->smartaddr) == 0 && bits256_nonz(G.LP_privkey) != 0 )
                    {
                        //printf("ADDR_UNSPENTS %s %s is my address being asked for!\n",ptr->symbol,coinaddr);
                        if ( ptr->lastpushtime > 0 && ptr->addr_listunspent_requested > (uint32_t)time(NULL)-10 )
                            ptr->lastpushtime -= LP_ORDERBOOK_DURATION*0.1;
                        ptr->addr_listunspent_requested = (uint32_t)time(NULL);
                    }
                }
            }
        }
        retstr = clonestr("{\"result\":\"success\"}");
    }
    else if ( strcmp(method,"encrypted") == 0 )
        retstr = clonestr("{\"result\":\"success\"}");
    else // psock requests/response
    {
        if ( IAMLP != 0 )
        {
            if ( strcmp(method,"psock") == 0 )
            {
                int32_t psock;
                if ( myipaddr == 0 || myipaddr[0] == 0 || strcmp(myipaddr,"127.0.0.1") == 0 )
                {
                    if ( LP_mypeer != 0 )
                        myipaddr = LP_mypeer->ipaddr;
                    else printf("LP_psock dont have actual ipaddr?\n");
                }
                if ( jint(argjson,"ispaired") != 0 && jobj(argjson,"netid") != 0 && juint(argjson,"netid") == G.netid )
                {
                    retstr = LP_psock(&psock,myipaddr,1,jint(argjson,"cmdchannel"),jbits256(argjson,"pubkey"));
                    //printf("LP_commands.(%s)\n",retstr);
                    return(retstr);
                }
                else return(clonestr("{\"error\":\"you are running an obsolete version, update\"}"));
            }
        }
        else
        {
            if ( strcmp(method,"psock") == 0 )
            {
                //printf("nonLP got (%s)\n",jprint(argjson,0));
                retstr = clonestr("{\"result\":\"success\"}");
            }
        }
    }
    if ( retstr == 0 )
        printf("ERROR.(%s)\n",jprint(argjson,0));
    if ( reqjson != 0 )
        free_json(reqjson);
    if ( retstr != 0 )
    {
        free(retstr);
        return(0);
    }
    return(0);
}
