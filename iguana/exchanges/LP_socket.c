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

/**
 * - we need to include WinSock2.h header to correctly use windows structure
 * as the application is still using 32bit structure from mingw so, we need to
 * add the include based on checking
 * @author - fadedreamz@gmail.com
 * @remarks - #if (defined(_M_X64) || defined(__amd64__)) && defined(WIN32)
 *     is equivalent to #if defined(_M_X64) as _M_X64 is defined for MSVC only
 */
#if defined(_M_X64)
#define WIN32_LEAN_AND_MEAN
#include <WinSock2.h>
#endif
#ifdef _WIN32
#include <WinSock2.h>
#endif

#ifdef __ANDROID__
# include <netinet/in.h>
# include <sys/socket.h>
#endif

int32_t set_blocking_mode(int32_t sock,int32_t is_blocking) // from https://stackoverflow.com/questions/2149798/how-to-reset-a-socket-back-to-blocking-mode-after-i-set-it-to-nonblocking-mode?utm_medium=organic&utm_source=google_rich_qa&utm_campaign=google_rich_qa
{
    int32_t ret;
#ifdef _WIN32
    /// @note windows sockets are created in blocking mode by default
    // currently on windows, there is no easy way to obtain the socket's current blocking mode since WSAIsBlocking was deprecated
    u_long non_blocking = is_blocking ? 0 : 1;
    ret = (NO_ERROR == ioctlsocket(sock,FIONBIO,&non_blocking));
#else
    const int flags = fcntl(sock, F_GETFL, 0);
    if ((flags & O_NONBLOCK) && !is_blocking) { fprintf(stderr,"set_blocking_mode(): socket was already in non-blocking mode\n"); return ret; }
    if (!(flags & O_NONBLOCK) && is_blocking) { fprintf(stderr,"set_blocking_mode(): socket was already in blocking mode\n"); return ret; }
    ret = (0 == fcntl(sock, F_SETFL, is_blocking ? (flags ^ O_NONBLOCK) : (flags | O_NONBLOCK)));
#endif
    if ( ret == 0 )
        return(-1);
    else return(0);
}

int32_t komodo_connect(int32_t sock,struct sockaddr *saddr,socklen_t addrlen)
{
    struct timeval tv; fd_set wfd,efd; int32_t res,so_error; socklen_t len;
#ifdef _WIN32
    set_blocking_mode(sock, 0);
#else
    fcntl(sock, F_SETFL, O_NONBLOCK);
#endif // _WIN32
    res = connect(sock,saddr,addrlen);

    if ( res == -1 )
    {
#ifdef _WIN32
		// https://msdn.microsoft.com/en-us/library/windows/desktop/ms737625%28v=vs.85%29.aspx - read about WSAEWOULDBLOCK return
		errno = WSAGetLastError();
		printf("[Decker] errno.%d --> ", errno);
		if ( errno != EINPROGRESS && errno != WSAEWOULDBLOCK ) // connect failed, do something...
#else
		if ( errno != EINPROGRESS ) // connect failed, do something...
#endif
        {
			printf("close socket ...\n");
			closesocket(sock);
            return(-1);
        }
		//printf("continue with select ...\n");
		FD_ZERO(&wfd);
        FD_SET(sock,&wfd);
        FD_ZERO(&efd);
        FD_SET(sock,&efd);
        tv.tv_sec = 10;
        tv.tv_usec = 0;
        res = select(sock+1,NULL,&wfd,&efd,&tv);
        if ( res == -1 ) // select failed, do something...
        {
            closesocket(sock);
            return(-1);
        }
        if ( res == 0 ) // connect timed out...
        {
            closesocket(sock);
            return(-1);
        }
        if ( FD_ISSET(sock,&efd) )
        {
            // connect failed, do something...
            getsockopt(sock,SOL_SOCKET,SO_ERROR,&so_error,&len);
            closesocket(sock);
            return(-1);
        }
    }
    set_blocking_mode(sock,1);
    return(0);
}

int32_t LP_socket(int32_t bindflag,char *hostname,uint16_t port)
{
    int32_t opt,sock,result; char ipaddr[64],checkipaddr[64]; struct timeval timeout;
    struct sockaddr_in saddr; socklen_t addrlen,slen;
    addrlen = sizeof(saddr);
    struct hostent *hostent;
    
    /**
     * gethostbyname() is deprecated and cause crash on x64 windows
     * the solution is to implement similar functionality by using getaddrinfo()
     * it is standard posix function and is correctly supported in win32/win64/linux
     * @author - fadedreamz@gmail.com
     */
#if defined(_M_X64)
    struct addrinfo *addrresult = NULL;
    struct addrinfo *returnptr = NULL;
    struct addrinfo hints;
    struct sockaddr_in * sockaddr_ipv4;
    int retVal;
    int found = 0;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
#endif
    
    if ( parse_ipaddr(ipaddr,hostname) != 0 )
        port = parse_ipaddr(ipaddr,hostname);
    
#if defined(_M_X64)
    retVal = getaddrinfo(ipaddr, NULL, &hints, &addrresult);
    for (returnptr = addrresult; returnptr != NULL && found == 0; returnptr = returnptr->ai_next) {
        switch (returnptr->ai_family) {
            case AF_INET:
                sockaddr_ipv4 = (struct sockaddr_in *) returnptr->ai_addr;
                // we want to break from the loop after founding the first ipv4 address
                found = 1;
                break;
        }
    }
    
    // if we iterate through the loop and didn't find anything,
    // that means we failed in the dns lookup
    if (found == 0) {
        printf("getaddrinfo(%s) returned error\n", hostname);
        freeaddrinfo(addrresult);
        return(-1);
    }
#else
    hostent = gethostbyname(ipaddr);
    if ( hostent == NULL )
    {
        printf("gethostbyname(%s) returned error: %d port.%d ipaddr.(%s)\n",hostname,errno,port,ipaddr);
        return(-1);
    }
#endif
    saddr.sin_family = AF_INET;
    saddr.sin_port = htons(port);
    //#ifdef _WIN32
    //   saddr.sin_addr.s_addr = (uint32_t)calc_ipbits("127.0.0.1");
    //#else
    
#if defined(_M_X64)
    saddr.sin_addr.s_addr = sockaddr_ipv4->sin_addr.s_addr;
    // graceful cleanup
    sockaddr_ipv4 = NULL;
    freeaddrinfo(addrresult);
#else
    memcpy(&saddr.sin_addr.s_addr,hostent->h_addr_list[0],hostent->h_length);
#endif
    expand_ipbits(checkipaddr,saddr.sin_addr.s_addr);
    if ( strcmp(ipaddr,checkipaddr) != 0 )
        printf("bindflag.%d iguana_socket mismatch (%s) -> (%s)\n",bindflag,checkipaddr,ipaddr);
    //#endif
    if ( (sock= socket(AF_INET,SOCK_STREAM,0)) < 0 )
    {
        if ( errno != ETIMEDOUT )
            printf("socket() failed: %s errno.%d", strerror(errno),errno);
        return(-1);
    }
    opt = 1;
    slen = sizeof(opt);
    //printf("set keepalive.%d\n",setsockopt(sock,SOL_SOCKET,SO_KEEPALIVE,(void *)&opt,slen));
#ifndef _WIN32
    if ( 1 )//&& bindflag != 0 )
    {
        opt = 0;
        getsockopt(sock,SOL_SOCKET,SO_KEEPALIVE,(void *)&opt,&slen);
        opt = 1;
        //printf("keepalive.%d\n",opt);
    }
    setsockopt(sock,SOL_SOCKET,SO_REUSEADDR,(void *)&opt,sizeof(opt));
#ifdef __APPLE__
    setsockopt(sock,SOL_SOCKET,SO_NOSIGPIPE,&opt,sizeof(opt));
#endif
#endif
    if ( bindflag == 0 )
    {
//#ifdef _WIN32
        if ( 1 ) // connect using async to allow timeout, then switch to sync
        {
            uint32_t starttime = (uint32_t)time(NULL);
            //printf("call connect sock.%d\n",sock);
            result = komodo_connect(sock,(struct sockaddr *)&saddr,addrlen);
            //printf("called connect result.%d lag.%d\n",result,(int32_t)(time(NULL) - starttime));
            if ( result < 0 )
                return(-1);
            timeout.tv_sec = 10;
            timeout.tv_usec = 0;
            setsockopt(sock,SOL_SOCKET,SO_RCVTIMEO,(void *)&timeout,sizeof(timeout));
        }
//#else
        else
        {
            result = connect(sock,(struct sockaddr *)&saddr,addrlen);
            if ( result != 0 )
            {
                if ( errno != ECONNRESET && errno != ENOTCONN && errno != ECONNREFUSED && errno != ETIMEDOUT && errno != EHOSTUNREACH )
                {
                    //printf("%s(%s) port.%d failed: %s sock.%d. errno.%d\n",bindflag!=0?"bind":"connect",hostname,port,strerror(errno),sock,errno);
                }
                if ( sock >= 0 )
                    closesocket(sock);
                return(-1);
            }
        }
//#endif
    }
    else
    {
        while ( (result= bind(sock,(struct sockaddr*)&saddr,addrlen)) != 0 )
        {
            if ( errno == EADDRINUSE )
            {
                sleep(1);
                printf("ERROR BINDING PORT.%d. this is normal tcp timeout, unless another process is using port\n",port);
                fflush(stdout);
                sleep(3);
                printf("%s(%s) port.%d try again: %s sock.%d. errno.%d\n",bindflag!=0?"bind":"connect",hostname,port,strerror(errno),sock,errno);
                if ( bindflag == 1 )
                {
                    closesocket(sock);
                    return(-1);
                }
                sleep(13);
                //continue;
            }
            if ( errno != ECONNRESET && errno != ENOTCONN && errno != ECONNREFUSED && errno != ETIMEDOUT && errno != EHOSTUNREACH )
            {
                printf("%s(%s) port.%d failed: %s sock.%d. errno.%d\n",bindflag!=0?"bind":"connect",hostname,port,strerror(errno),sock,errno);
                closesocket(sock);
                return(-1);
            }
        }
        if ( listen(sock,64) != 0 )
        {
            printf("listen(%s) port.%d failed: %s sock.%d. errno.%d\n",hostname,port,strerror(errno),sock,errno);
            if ( sock >= 0 )
                closesocket(sock);
            return(-1);
        }
    }
#ifdef __APPLE__
    //timeout.tv_sec = 0;
    //timeout.tv_usec = 30000;
    //setsockopt(sock,SOL_SOCKET,SO_RCVTIMEO,(void *)&timeout,sizeof(timeout));
    timeout.tv_sec = 0;
    timeout.tv_usec = 10000;
    setsockopt(sock,SOL_SOCKET,SO_SNDTIMEO,(void *)&timeout,sizeof(timeout));
#endif
    return(sock);
}

struct electrum_info *Electrums[8192];
int32_t Num_electrums;

struct electrum_info *electrum_server(char *symbol,struct electrum_info *ep)
{
    struct electrum_info *rbuf[128],*recent_ep; uint32_t recent,mostrecent = 0; int32_t i,n = 0;
    portable_mutex_lock(&LP_electrummutex);
    if ( ep == 0 )
    {
        //printf("find random electrum.%s from %d\n",symbol,Num_electrums);
        memset(rbuf,0,sizeof(rbuf));
        recent_ep = 0;
        recent = (uint32_t)time(NULL) - 300;
        for (i=0; i<Num_electrums; i++)
        {
            ep = Electrums[i];
            if ( strcmp(symbol,ep->symbol) == 0 && ep->sock >= 0 )
            {
                if ( ep->lasttime > recent )
                {
                    rbuf[n++] = ep;
                    if ( n == sizeof(rbuf)/sizeof(*rbuf) )
                        break;
                }
                else if ( ep->lasttime > mostrecent )
                {
                    mostrecent = ep->lasttime;
                    recent_ep = ep;
                }
            }
        }
        ep = recent_ep;
        if ( n > 0 )
        {
            i = (LP_rand() % n);
            ep = rbuf[i];
        }
    }
    else if ( Num_electrums < sizeof(Electrums)/sizeof(*Electrums) )
        Electrums[Num_electrums++] = ep;
    else printf("Electrum server pointer buf overflow %d\n",Num_electrums);
    portable_mutex_unlock(&LP_electrummutex);
    return(ep);
}

int32_t electrum_process_array(struct iguana_info *coin,struct electrum_info *ep,char *coinaddr,cJSON *array,int32_t electrumflag,bits256 reftxid,bits256 reftxid2)
{
    int32_t i,v,n,ht,flag = 0; char str[65]; uint64_t value; bits256 txid; cJSON *item,*retjson,*txobj; struct LP_transaction *tx;
    if ( array != 0 && coin != 0 && (n= cJSON_GetArraySize(array)) > 0 )
    {
        //printf("PROCESS %s/%s %s num.%d\n",coin->symbol,ep!=0?ep->symbol:"nanolistunspent",coinaddr,n);
        for (i=0; i<n; i++)
        {
            item = jitem(array,i);
            if ( electrumflag == 0 )
            {
                txid = jbits256(item,"txid");
                v = jint(item,"vout");
                value = LP_value_extract(item,0,txid);
                ht = LP_txheight(coin,txid);
                if ( (retjson= LP_gettxout(coin->symbol,coinaddr,txid,v)) != 0 )
                    free_json(retjson);
                else
                {
                    //printf("external unspent has no gettxout\n");
                    flag += LP_address_utxoadd(0,(uint32_t)time(NULL),"electrum process",coin,coinaddr,txid,v,value,0,1);
                }
            }
            else
            {
                txid = jbits256(item,"tx_hash");
                v = jint(item,"tx_pos");
                value = j64bits(item,"value");
                ht = jint(item,"height");
            }
            if ( bits256_nonz(txid) == 0 )
                continue;
            if ( (tx= LP_transactionfind(coin,txid)) == 0 )
            {
                if ( (bits256_nonz(reftxid) == 0 || bits256_cmp(reftxid,txid) == 0) && (bits256_nonz(reftxid2) == 0 || bits256_cmp(reftxid2,txid) == 0) )
                {
                    txobj = LP_transactioninit(coin,txid,0,0);
                    LP_transactioninit(coin,txid,1,txobj);
                    free_json(txobj);
                    tx = LP_transactionfind(coin,txid);
                }
            }
            if ( tx != 0 )
            {
                if (tx->height <= 0 )
                {
                    tx->height = ht;
                    if ( ep != 0 && coin != 0 && tx->SPV == 0 )
                    {
                        if ( 0 && strcmp(coinaddr,coin->smartaddr) == 0 )
                            tx->SPV = LP_merkleproof(coin,coin->smartaddr,ep,txid,tx->height);
                        //printf("%s %s >>>>>>>>>> set %s <- height %d\n",coin->symbol,coinaddr,bits256_str(str,txid),tx->height);
                    }
                }
                if ( v >= 0 && v < tx->numvouts )
                {
                    if ( tx->outpoints[v].value == 0 && value != tx->outpoints[v].value )
                    {
                        printf("%s %s >>>>>>>>>> set %s/v%d <- %.8f vs %.8f\n",coin->symbol,coinaddr,bits256_str(str,txid),v,dstr(value),dstr(tx->outpoints[v].value));
                        tx->outpoints[v].value = value;
                    }
                }
                if ( tx->height > 0 )
                {
                    //printf("from electrum_process_array\n");
                    flag += LP_address_utxoadd(0,(uint32_t)time(NULL),"electrum process2",coin,coinaddr,txid,v,value,tx->height,-1);
                }
                //printf("v.%d numvouts.%d %.8f (%s)\n",v,tx->numvouts,dstr(tx->outpoints[jint(item,"tx_pos")].value),jprint(item,0));
            } //else printf("cant find tx\n");
        }
    }
    return(flag);
}

struct stritem *electrum_sitem(struct electrum_info *ep,char *stratumreq,int32_t timeout,cJSON **retjsonp)
{
    struct stritem *sitem = (struct stritem *)queueitem(stratumreq);
    sitem->expiration = timeout;
    sitem->DL.type = ep->stratumid++;
    sitem->retptrp = (void **)retjsonp;
    queue_enqueue("sendQ",&ep->sendQ,&sitem->DL);
    return(sitem);
}

void electrum_initial_requests(struct electrum_info *ep)
{
    cJSON *retjson; char stratumreq[1024];
    retjson = 0;
    sprintf(stratumreq,"{ \"jsonrpc\":\"2.0\", \"id\": %u, \"method\":\"%s\", \"params\": %s }\n",ep->stratumid,"blockchain.headers.subscribe","[]");
    electrum_sitem(ep,stratumreq,3,&retjson);
    
    retjson = 0;
    sprintf(stratumreq,"{ \"jsonrpc\":\"2.0\", \"id\": %u, \"method\":\"%s\", \"params\": %s }\n",ep->stratumid,"server.version","[\"barterDEX\", [\"1.1\", \"1.1\"]]");
    electrum_sitem(ep,stratumreq,3,&retjson);
    
    retjson = 0;
    sprintf(stratumreq,"{ \"jsonrpc\":\"2.0\", \"id\": %u, \"method\":\"%s\", \"params\": %s }\n",ep->stratumid,"blockchain.estimatefee","[2]");
    electrum_sitem(ep,stratumreq,3,&retjson);
}

int32_t electrum_kickstart(struct electrum_info *ep)
{
    closesocket(ep->sock);//, ep->sock = -1;
    if ( (ep->sock= LP_socket(0,ep->ipaddr,ep->port)) < 0 )
    {
        printf("error RE-connecting to %s:%u\n",ep->ipaddr,ep->port);
        return(-1);
    }
    else
    {
        ep->stratumid = 0;
        electrum_initial_requests(ep);
        printf("RECONNECT ep.%p %s numerrors.%d too big -> new %s:%u sock.%d\n",ep,ep->symbol,ep->numerrors,ep->ipaddr,ep->port,ep->sock);
        ep->numerrors = 0;
    }
    return(0);
}

int32_t zeroval();

cJSON *electrum_submit(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *method,char *params,int32_t timeout)
{
    // queue id and string and callback
    char stratumreq[16384]; uint32_t expiration; struct stritem *sitem;
    if ( ep == 0 )
        ep = electrum_server(symbol,0);
    while ( ep != 0 )
    {
        if ( strcmp(ep->symbol,symbol) != 0 )
        {
            printf("electrum_submit ep.%p %s %s:%u called for [%s]???\n",ep,ep->symbol,ep->ipaddr,ep->port,symbol);
        }
        if ( ep != 0 && ep->sock >= 0 && retjsonp != 0 )
        {
            *retjsonp = 0;
            sprintf(stratumreq,"{ \"jsonrpc\":\"2.0\", \"id\": %u, \"method\":\"%s\", \"params\": %s }\n",ep->stratumid,method,params);
//printf("timeout.%d exp.%d %s %s",timeout,(int32_t)(expiration-time(NULL)),symbol,stratumreq);
            memset(ep->buf,0,ep->bufsize);
            sitem = electrum_sitem(ep,stratumreq,timeout,retjsonp);
            portable_mutex_lock(&ep->mutex); // this helps performance!
            expiration = (uint32_t)time(NULL) + timeout + 1;
            while ( *retjsonp == 0 && time(NULL) <= expiration )
                usleep(15000);
            portable_mutex_unlock(&ep->mutex);
            if ( *retjsonp == 0 || jobj(*retjsonp,"error") != 0 )
            {
                if ( ++ep->numerrors >= LP_ELECTRUM_MAXERRORS )
                {
                    // electrum_kickstart(ep); seems to hurt more than help
                }
            } else if ( ep->numerrors > 0 )
                ep->numerrors--;
            if ( ep->prev == 0 )
            {
                if ( *retjsonp == 0 )
                {
                    //printf("unexpected %s timeout with null retjson: %s %s\n",ep->symbol,method,params);
                    *retjsonp = cJSON_Parse("{\"error\":\"timeout\"}");
                }
                return(*retjsonp);
            }
        } //else printf("couldnt find electrum server for (%s %s) or no retjsonp.%p\n",method,params,retjsonp);
        ep = ep->prev;
        //if ( ep != 0 )
        //    printf("using prev ep.%s\n",ep->symbol);
    }
    return(0);
}

cJSON *electrum_noargs(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *method,int32_t timeout)
{
    cJSON *retjson;
    if ( retjsonp == 0 )
        retjsonp = &retjson;
    return(electrum_submit(symbol,ep,retjsonp,method,"[]",timeout));
}

cJSON *electrum_strarg(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *method,char *arg,int32_t timeout)
{
    char params[16384]; cJSON *retjson;
    if ( strlen(arg) < sizeof(params) )
    {
        if ( retjsonp == 0 )
            retjsonp = &retjson;
        sprintf(params,"[\"%s\"]",arg);
        return(electrum_submit(symbol,ep,retjsonp,method,params,timeout));
    } else return(0);
}

cJSON *electrum_intarg(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *method,int32_t arg,int32_t timeout)
{
    char params[64]; cJSON *retjson;
    if ( retjsonp == 0 )
        retjsonp = &retjson;
    sprintf(params,"[\"%d\"]",arg);
    return(electrum_submit(symbol,ep,retjsonp,method,params,timeout));
}

cJSON *electrum_hasharg(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *method,bits256 arg,int32_t timeout)
{
    char params[128],str[65]; cJSON *retjson;
    if ( retjsonp == 0 )
        retjsonp = &retjson;
    sprintf(params,"[\"%s\"]",bits256_str(str,arg));
    return(electrum_submit(symbol,ep,retjsonp,method,params,timeout));
}

cJSON *electrum_banner(char *symbol,struct electrum_info *ep,cJSON **retjsonp) { return(electrum_noargs(symbol,ep,retjsonp,"server.banner",ELECTRUM_TIMEOUT)); }

cJSON *electrum_scripthash_cmd(char *symbol,uint8_t taddr,struct electrum_info *ep,cJSON **retjsonp,char *cmd,char *coinaddr)
{
    uint8_t addrtype,rmd160[20]; char btcaddr[64],cmdbuf[128]; //char scripthash[51],rmdstr[41],;
    bitcoin_addr2rmd160(symbol,taddr,&addrtype,rmd160,coinaddr);
    bitcoin_address("BTC",btcaddr,0,addrtype,rmd160,20);
    //init_hexbytes_noT(rmdstr,rmd160,20);
    //sprintf(scripthash,"%s",rmdstr);
    //sprintf(cmdbuf,"blockchain.scripthash.%s",cmd);
    sprintf(cmdbuf,"blockchain.address.%s",cmd);
    return(electrum_strarg(symbol,ep,retjsonp,cmdbuf,btcaddr,ELECTRUM_TIMEOUT));
}

cJSON *electrum_address_gethistory(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *addr,bits256 reftxid)
{
    char str[65]; struct LP_transaction *tx; cJSON *retjson,*txobj,*item; int32_t i,n,height; bits256 txid; struct iguana_info *coin = LP_coinfind(symbol);
    if ( coin == 0 )
        return(0);
    if ( strcmp(symbol,"BCH") == 0 )
        retjson = electrum_scripthash_cmd(symbol,coin->taddr,ep,retjsonp,"get_history",addr);
    else retjson = electrum_strarg(symbol,ep,retjsonp,"blockchain.address.get_history",addr,ELECTRUM_TIMEOUT);
    //printf("history.(%s)\n",jprint(retjson,0));
    if ( retjson != 0 && (n= cJSON_GetArraySize(retjson)) > 0 )
    {
        for (i=0; i<n; i++)
        {
            item = jitem(retjson,i);
            txid = jbits256(item,"tx_hash");
            height = jint(item,"height");
            if ( (tx= LP_transactionfind(coin,txid)) == 0 && (bits256_nonz(reftxid) == 0 || bits256_cmp(txid,reftxid) == 0) )
            {
                //char str[65]; printf("history txinit %s ht.%d\n",bits256_str(str,txid),height);
                txobj = LP_transactioninit(coin,txid,0,0);
                txobj = LP_transactioninit(coin,txid,1,txobj);
                if ( txobj != 0 )
                    free_json(txobj);
                if ( height > 0 )
                {
                    if ( (tx= LP_transactionfind(coin,txid)) != 0 )
                    {
                        if ( tx->height > 0 && tx->height != height )
                            printf("update %s height.%d <- %d\n",bits256_str(str,txid),tx->height,height);
                        tx->height = height;
                        LP_address_utxoadd(0,(uint32_t)time(NULL),"electrum history",coin,addr,txid,0,0,height,-1);
                    }
                }
            }
        }
    }
    return(retjson);
}

int32_t LP_txheight_check(struct iguana_info *coin,char *coinaddr,bits256 txid)
{
    cJSON *retjson;
    if ( coin->electrum != 0 )
    {
        if ( (retjson= electrum_address_gethistory(coin->symbol,coin->electrum,&retjson,coinaddr,txid)) != 0 )
            free_json(retjson);
    }
    return(0);
}

cJSON *electrum_address_getmempool(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *addr,bits256 reftxid,bits256 reftxid2)
{
    cJSON *retjson; struct iguana_info *coin = LP_coinfind(symbol);
    if ( coin == 0 )
        return(0);
    if ( strcmp(symbol,"BCH") == 0 )
        retjson = electrum_scripthash_cmd(symbol,coin->taddr,ep,retjsonp,"get_mempool",addr);
    else retjson = electrum_strarg(symbol,ep,retjsonp,"blockchain.address.get_mempool",addr,ELECTRUM_TIMEOUT);
    //printf("MEMPOOL.(%s)\n",jprint(retjson,0));
    electrum_process_array(coin,ep,addr,retjson,1,reftxid,reftxid2);
    return(retjson);
}

cJSON *electrum_address_listunspent(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *addr,int32_t electrumflag,bits256 txid,bits256 txid2)
{
    cJSON *retjson=0; char *retstr; struct LP_address *ap; struct iguana_info *coin; int32_t updatedflag,height,usecache=1;
    if ( (coin= LP_coinfind(symbol)) == 0 )
        return(0);
    if ( strcmp(addr,INSTANTDEX_KMD) == 0 )
        return(cJSON_Parse("[]"));
    if ( ep == 0 || ep->heightp == 0 )
        height = coin->longestchain;
    else height = *(ep->heightp);
    if ( (ap= LP_address(coin,addr)) != 0 )
    {
        if ( ap->unspenttime == 0 )
        {
            ap->unspenttime = (uint32_t)time(NULL);
            ap->unspentheight = height;
            usecache = 1;
        }
        else if ( ap->unspentheight < height )
            usecache = 0;
        else if ( G.LP_pendingswaps != 0 && time(NULL) > ap->unspenttime+13 )
            usecache = 0;
    }
    //usecache = 0; // disable unspents cache
    if ( usecache == 0 || electrumflag > 1 )
    {
        if ( strcmp(symbol,"BCH") == 0 )
            retjson = electrum_scripthash_cmd(symbol,coin->taddr,ep,retjsonp,"listunspent",addr);
        else retjson = electrum_strarg(symbol,ep,retjsonp,"blockchain.address.listunspent",addr,ELECTRUM_TIMEOUT);
        if ( retjson != 0 )
        {
            if ( jobj(retjson,"error") == 0 && is_cJSON_Array(retjson) != 0 )
            {
                if ( 0 && electrumflag > 1 )
                    printf("%s.%d u.%u/%d t.%ld %s LISTUNSPENT.(%d)\n",coin->symbol,height,ap->unspenttime,ap->unspentheight,time(NULL),addr,(int32_t)strlen(jprint(retjson,0)));
                updatedflag = 0;
                if ( electrum_process_array(coin,ep,addr,retjson,electrumflag,txid,txid2) != 0 )
                {
                    //LP_postutxos(coin->symbol,addr);
                    updatedflag = 1;
                }
                retstr = jprint(retjson,0);
                LP_unspents_cache(coin->symbol,addr,retstr,1);
                free(retstr);
            }
            else
            {
                free_json(retjson);
                retjson = 0;
            }
            if ( ap != 0 )
            {
                ap->unspenttime = (uint32_t)time(NULL);
                ap->unspentheight = height;
            }
        }
    }
    if ( retjson == 0 )
    {
        if ( (retstr= LP_unspents_filestr(symbol,addr)) != 0 )
        {
            retjson = cJSON_Parse(retstr);
            free(retstr);
        } else retjson = LP_address_utxos(coin,addr,1);
    }
    return(retjson);
}

cJSON *electrum_sendrawtransaction(char *symbol,struct electrum_info *ep,cJSON **retjsonp,char *rawtx) { return(electrum_strarg(symbol,ep,retjsonp,"blockchain.transaction.broadcast",rawtx,ELECTRUM_TIMEOUT)); }

cJSON *electrum_getheader(char *symbol,struct electrum_info *ep,cJSON **retjsonp,int32_t n)
{
    return(electrum_intarg(symbol,ep,retjsonp,"blockchain.block.get_header",n,ELECTRUM_TIMEOUT));
}

cJSON *LP_cache_transaction(struct iguana_info *coin,bits256 txid,uint8_t *serialized,int32_t len)
{
    cJSON *txobj; struct LP_transaction *tx;
    if ( (txobj= LP_transaction_fromdata(coin,txid,serialized,len)) != 0 )
    {
        if ( (tx= LP_transactionfind(coin,txid)) == 0 || tx->serialized == 0 )
        {
            txobj = LP_transactioninit(coin,txid,0,txobj);
            LP_transactioninit(coin,txid,1,txobj);
            tx = LP_transactionfind(coin,txid);
        }
        if ( tx != 0 )
        {
            tx->serialized = serialized;
            tx->len = len;
        }
        else
        {
            char str[65]; printf("unexpected couldnt find tx %s %s\n",coin->symbol,bits256_str(str,txid));
            free(serialized);
        }
    }
    return(txobj);
}

cJSON *_electrum_transaction(char *symbol,struct electrum_info *ep,cJSON **retjsonp,bits256 txid)
{
    char *hexstr,str[65]; int32_t len; cJSON *hexjson,*txobj=0; struct iguana_info *coin; uint8_t *serialized; struct LP_transaction *tx;
    //printf("electrum_transaction %s %s\n",symbol,bits256_str(str,txid));
    if ( bits256_nonz(txid) != 0 && (coin= LP_coinfind(symbol)) != 0 )
    {
        if ( (tx= LP_transactionfind(coin,txid)) != 0 && tx->serialized != 0 )
        {
            //char str[65]; printf("%s cache hit -> TRANSACTION.(%s)\n",symbol,bits256_str(str,txid));
            if ( (txobj= LP_transaction_fromdata(coin,txid,tx->serialized,tx->len)) != 0 )
            {
                *retjsonp = txobj;
                return(txobj);
            }
        }
        if ( bits256_cmp(txid,coin->cachedtxid) == 0 )
        {
            if ( (txobj= LP_transaction_fromdata(coin,txid,coin->cachedtxiddata,coin->cachedtxidlen)) != 0 )
            {
                *retjsonp = txobj;
                return(txobj);
            }
        }
        hexjson = electrum_hasharg(symbol,ep,&hexjson,"blockchain.transaction.get",txid,ELECTRUM_TIMEOUT);
        hexstr = jprint(hexjson,0);
        if ( strlen(hexstr) > 100000 )
        {
            static uint32_t counter;
            if ( counter++ < 3 )
                printf("rawtransaction %s %s too big %d\n",coin->symbol,bits256_str(str,txid),(int32_t)strlen(hexstr));
            free(hexstr);
            free_json(hexjson);
            *retjsonp = cJSON_Parse("{\"error\":\"transaction too big\"}");
            return(*retjsonp);
        }
        if ( hexstr[0] == '"' && hexstr[strlen(hexstr)-1] == '"' )
            hexstr[strlen(hexstr)-1] = 0;
        if ( (len= is_hexstr(hexstr+1,0)) > 2 )
        {
            len = (int32_t)strlen(hexstr+1) >> 1;
            serialized = malloc(len);
            if ( coin->cachedtxiddata != 0 )
                free(coin->cachedtxiddata);
            coin->cachedtxiddata = malloc(len);
            coin->cachedtxidlen = len;
            decode_hex(serialized,len,hexstr+1);
            memcpy(coin->cachedtxiddata,serialized,len);
            free(hexstr);
            //printf("DATA.(%s) from (%s)\n",hexstr+1,jprint(hexjson,0));
            *retjsonp = LP_cache_transaction(coin,txid,serialized,len); // eats serialized
            free_json(hexjson);
            //printf("return from electrum_transaction\n");
            return(*retjsonp);
        } //else printf("%s %s non-hex tx.(%s)\n",coin->symbol,bits256_str(str,txid),jprint(hexjson,0));
        free(hexstr);
        free_json(hexjson);
    }
    *retjsonp = 0;
    return(*retjsonp);
}

cJSON *electrum_transaction(int32_t *heightp,char *symbol,struct electrum_info *ep,cJSON **retjsonp,bits256 txid,char *SPVcheck)
{
    cJSON *retjson,*array; bits256 zero; struct LP_transaction *tx=0; struct iguana_info *coin;
    coin = LP_coinfind(symbol);
    if ( coin == 0 )
        return(0);
    *heightp = 0;
    if ( ep != 0 )
        portable_mutex_lock(&ep->txmutex);
    retjson = _electrum_transaction(symbol,ep,retjsonp,txid);
    if ( (tx= LP_transactionfind(coin,txid)) != 0 && ep != 0 && coin != 0 && SPVcheck != 0 && SPVcheck[0] != 0 )
    {
        if ( tx->height <= 0 )
        {
            memset(zero.bytes,0,sizeof(zero));
            if ( (array= electrum_address_listunspent(symbol,ep,&array,SPVcheck,2,txid,zero)) != 0 )
            {
                printf("SPVcheck.%s got %d unspents\n",SPVcheck,cJSON_GetArraySize(array));
                free_json(array);
            }
        }
        if ( tx->height > 0 )
        {
            if ( tx->SPV == 0 )
                tx->SPV = LP_merkleproof(coin,SPVcheck,ep,txid,tx->height);
            *heightp = tx->height;
        }
        char str[65]; printf("%s %s %s SPV height %d SPV %d\n",coin->symbol,SPVcheck,bits256_str(str,txid),tx->height,tx->SPV);
    } else if ( tx != 0 )
        *heightp = tx->height;
    if ( ep != 0 )
        portable_mutex_unlock(&ep->txmutex);
    return(retjson);
}

cJSON *electrum_getmerkle(char *symbol,struct electrum_info *ep,cJSON **retjsonp,bits256 txid,int32_t height)
{
    char params[128],str[65];
    sprintf(params,"[\"%s\", %d]",bits256_str(str,txid),height);
    if ( bits256_nonz(txid) == 0 )
        return(cJSON_Parse("{\"error\":\"null txid\"}"));
    return(electrum_submit(symbol,ep,retjsonp,"blockchain.transaction.get_merkle",params,ELECTRUM_TIMEOUT));
}
