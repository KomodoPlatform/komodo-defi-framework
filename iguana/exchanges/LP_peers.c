
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
//  LP_peers.c
//  marketmaker
//

struct LP_peerinfo *LP_peerfind(uint32_t ipbits,uint16_t port)
{
    struct LP_peerinfo *peer=0; uint64_t ip_port;
    ip_port = ((uint64_t)port << 32) | ipbits;
    portable_mutex_lock(&LP_peermutex);
    HASH_FIND(hh,LP_peerinfos,&ip_port,sizeof(ip_port),peer);
    portable_mutex_unlock(&LP_peermutex);
    return(peer);
}

cJSON *LP_peerjson(struct LP_peerinfo *peer)
{
    cJSON *item = cJSON_CreateObject();
    jaddstr(item,"isLP",peer->ipaddr);
    jaddnum(item,"remoteport",peer->port-1);
    jaddnum(item,"netid",peer->netid);
    if ( strcmp(peer->ipaddr,LP_myipaddr) == 0 )
    {
        jaddnum(item,"session",G.LP_sessionid);
        //if ( LP_mypeer != 0 )
        //    jaddnum(item,"numutxos",LP_mypeer->numutxos);
    } else jaddnum(item,"session",peer->sessionid);
    //jaddnum(item,"profit",peer->profitmargin);
    return(item);
}

char *LP_peers()
{
    struct LP_peerinfo *peer,*tmp; cJSON *peersjson = cJSON_CreateArray();
    HASH_ITER(hh,LP_peerinfos,peer,tmp)
    {
        //if ( peer->errors < LP_MAXPEER_ERRORS )
        if ( peer->isLP != 0 )
            jaddi(peersjson,LP_peerjson(peer));
    }
    return(jprint(peersjson,1));
}

struct LP_peerinfo *LP_addpeer(struct LP_peerinfo *mypeer,int32_t mypubsock,char *ipaddr,uint16_t port,uint16_t pushport,uint16_t subport,int32_t isLP,uint32_t sessionid,uint16_t netid)
{
    uint32_t ipbits; int32_t valid,pushsock,subsock,timeout; char checkip[64],pushaddr[128],subaddr[128]; struct LP_peerinfo *peer = 0;
#ifdef LP_STRICTPEERS
    if ( strncmp("5.9.253",ipaddr,strlen("5.9.253")) != 0 )
        return(0);
#endif
    ipbits = (uint32_t)calc_ipbits(ipaddr);
    expand_ipbits(checkip,ipbits);
    if ( strcmp(checkip,ipaddr) == 0 )
    {
        if ( (peer= LP_peerfind(ipbits,port)) != 0 )
        {
            if ( peer->netid != netid )
            {
                printf("netid mismatch for %s? %d vs %d\n",peer->ipaddr,peer->netid,G.netid);
                return(0);
            }
            if ( isLP != 0 && peer->isLP == 0 )
            {
                if ( (peer->isLP= isLP) != 0 )
                    LP_numactive_LP++;
            }
        }
        else if ( IAMLP != 0 || LP_numactive_LP < 10 )
        {
            //printf("addpeer (%s:%u) pushport.%u subport.%u\n",ipaddr,port,pushport,subport);
            peer = calloc(1,sizeof(*peer));
            peer->pairsock = -1;
            if ( strcmp(peer->ipaddr,LP_myipaddr) == 0 )
                peer->sessionid = G.LP_sessionid;
            else peer->sessionid = sessionid;
            peer->pushsock = peer->subsock = pushsock = subsock = -1;
            strcpy(peer->ipaddr,ipaddr);
            peer->netid = netid;
            //peer->profitmargin = profitmargin;
            peer->ipbits = ipbits;
            if ( (peer->isLP= isLP) != 0 )
                LP_numactive_LP++;
            peer->port = port;
            peer->ip_port = ((uint64_t)port << 32) | ipbits;
            if ( peer->pushsock >= 0 && peer->subsock >= 0 )
            {
                //printf("add peer %s isLP.%d\n",peer->ipaddr,peer->isLP);
                portable_mutex_lock(&LP_peermutex);
                HASH_ADD(hh,LP_peerinfos,ip_port,sizeof(peer->ip_port),peer);
                if ( mypeer != 0 )
                {
                    mypeer->numpeers++;
                    printf("_LPaddpeer %s -> numpeers.%d mypubsock.%d other.(%d)\n",ipaddr,mypeer->numpeers,mypubsock,isLP);
                } else peer->numpeers = 1; // will become mypeer
                portable_mutex_unlock(&LP_peermutex);
            } else printf("%s invalid pushsock.%d or subsock.%d\n",peer->ipaddr,peer->pushsock,peer->subsock);
        }
    }
    return(peer);
}

/*int32_t LP_coinbus(uint16_t coin_busport)
{
    struct LP_peerinfo *peer,*tmp; char busaddr[64]; int32_t timeout,bussock = -1;
    return(-1);
    if ( IAMLP != 0 && LP_mypeer != 0 && (bussock= nn_socket(AF_SP,NN_BUS)) >= 0 )
    {
        timeout = 1;
        nn_setsockopt(bussock,NN_SOL_SOCKET,NN_SNDTIMEO,&timeout,sizeof(timeout));
        nn_setsockopt(bussock,NN_SOL_SOCKET,NN_RCVTIMEO,&timeout,sizeof(timeout));
        nanomsg_transportname(0,busaddr,LP_mypeer->ipaddr,coin_busport);
        if ( nn_bind(bussock,busaddr) < 0 )
        {
            printf("error binding to coin_busport.%s\n",busaddr);
            nn_close(bussock);
        }
        else
        {
            HASH_ITER(hh,LP_peerinfos,peer,tmp)
            {
                if ( LP_mypeer->port != peer->port || strcmp(LP_mypeer->ipaddr,peer->ipaddr) != 0 )
                {
                    nanomsg_transportname(0,busaddr,peer->ipaddr,coin_busport);
                    nn_connect(bussock,busaddr);
                }
            }
        }
    }
    return(bussock);
}*/

void LP_peer_recv(char *ipaddr,int32_t ismine,struct LP_pubkey_info *pubp)
{
    struct LP_peerinfo *peer;
    if ( (peer= LP_peerfind((uint32_t)calc_ipbits(ipaddr),RPC_port)) != 0 )
    {
        peer->numrecv++;
        if ( ismine != 0 && bits256_cmp(G.LP_mypub25519,pubp->pubkey) != 0 && (bits256_cmp(peer->pubkey,pubp->pubkey) != 0 || pubp->pairsock != peer->pairsock) )
        {
            peer->pubkey = pubp->pubkey;
            pubp->pairsock = peer->pairsock;
            char str[65]; printf("set pubkey for %s <- %s, pairsock.%d\n",ipaddr,bits256_str(str,pubp->pubkey),pubp->pairsock);
        }
        peer->recvtime = (uint32_t)time(NULL);
    }
}

int32_t LP_numpeers()
{
    struct LP_peerinfo *peer,*tmp; int32_t numpeers = 0;
    HASH_ITER(hh,LP_peerinfos,peer,tmp)
    {
        if ( peer->isLP != 0 )
            numpeers++;
    }
    return(numpeers);
}

uint16_t LP_randpeer(char *destip)
{
    struct LP_peerinfo *peer,*tmp; uint16_t port = 0; int32_t n,r,numpeers = 0;
    destip[0] = 0;
    numpeers = LP_numpeers();
    if ( numpeers > 0 )
    {
        r = LP_rand() % numpeers;
        n = 0;
        HASH_ITER(hh,LP_peerinfos,peer,tmp)
        {
            if ( peer->isLP != 0 )
            {
                if ( n++ == r )
                {
                    strcpy(destip,peer->ipaddr);
                    port = peer->port;
                    break;
                }
            }
        }
    }
    return(port);
}

uint16_t LP_rarestpeer(char *destip)
{
    struct LP_peerinfo *peer,*tmp,*rarest = 0; int32_t iter; uint32_t now;
    now = (uint32_t)time(NULL);
    destip[0] = 0;
    for (iter=0; iter<2; iter++)
    {
        HASH_ITER(hh,LP_peerinfos,peer,tmp)
        {
            if ( strcmp(peer->ipaddr,LP_myipaddr) != 0 && iter == 0 && peer->recvtime < now-3600 )
                continue;
            if ( peer->isLP != 0 )
            {
                if ( rarest == 0 || peer->numrecv < rarest->numrecv )
                    rarest = peer;
            }
        }
        if ( rarest != 0 )
            break;
    }
    if ( rarest == 0 )
        LP_randpeer(destip);
    else strcpy(destip,rarest->ipaddr);
    return(rarest != 0 ? rarest->port : RPC_port);
}

