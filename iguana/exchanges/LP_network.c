
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
//  LP_network.c
//  marketmaker
//

struct psock
{
    uint32_t lasttime,lastping,errors;
    int32_t publicsock,sendsock,ispaired,cmdchannel;
    uint16_t publicport,sendport;
    char sendaddr[128],publicaddr[128];
} *PSOCKS;

uint16_t Numpsocks,Psockport = MIN_PSOCK_PORT,Pcmdport = MAX_PSOCK_PORT;
extern portable_mutex_t LP_commandQmutex;

char *nanomsg_transportname(int32_t bindflag,char *str,char *ipaddr,uint16_t port)
{
    sprintf(str,"tcp://%s:%u",bindflag == 0 ? ipaddr : "*",port);
    return(str);
}

/*char *nanomsg_transportname2(int32_t bindflag,char *str,char *ipaddr,uint16_t port)
 {
 sprintf(str,"ws://%s:%u",bindflag == 0 ? ipaddr : "*",port+10);
 return(str);
 }
 
 int32_t _LP_send(int32_t sock,void *msg,int32_t sendlen,int32_t freeflag)
 {
 int32_t sentbytes;
 if ( sock < 0 )
 {
 printf("LP_send.(%s) to illegal socket\n",(char *)msg);
 if ( freeflag != 0 )
 free(msg);
 return(-1);
 }
 if ( (sentbytes= nn_send(sock,msg,sendlen,0)) != sendlen )
 printf("LP_send sent %d instead of %d\n",sentbytes,sendlen);
 else printf("SENT.(%s)\n",(char *)msg);
 if ( freeflag != 0 )
 free(msg);
 return(sentbytes);
 }*/

int32_t LP_sockcheck(int32_t sock)
{
    return(1);
}

struct LP_queue
{
    struct LP_queue *next,*prev;
    int32_t sock,peerind,msglen;
    uint32_t starttime,crc32,notready;
    uint8_t msg[];
} *LP_Q;
int32_t LP_Qenqueued,LP_Qerrors,LP_Qfound;

void _LP_sendqueueadd(uint32_t crc32,int32_t sock,uint8_t *msg,int32_t msglen,int32_t peerind)
{
    struct LP_queue *ptr;
    ptr = calloc(1,sizeof(*ptr) + msglen + sizeof(bits256));
    ptr->crc32 = crc32;
    ptr->sock = sock;
    ptr->peerind = peerind;
    ptr->msglen = (int32_t)(msglen + 0*sizeof(bits256));
    memcpy(ptr->msg,msg,msglen); // sizeof(bits256) at the end all zeroes
    DL_APPEND(LP_Q,ptr);
    LP_Qenqueued++;
    //printf("Q.%p: peerind.%d msglen.%d sock.%d\n",ptr,peerind,msglen,sock);
}

int32_t LP_crc32find(int32_t *duplicatep,int32_t ind,uint32_t crc32)
{
    static uint32_t crcs[16384]; static unsigned long dup,total;
    int32_t i;
    *duplicatep = 0;
    if ( ind < 0 )
    {
        total++;
        for (i=0; i<sizeof(crcs)/sizeof(*crcs); i++)
        {
            if ( crc32 == crcs[i] )
            {
                if ( i > 0 )
                {
                    crcs[i] = crcs[i >> 1];
                    crcs[i >> 1] = crc32;
                    dup++;
                    //printf("duplicate %u in slot %d -> slot %d (%lu / %lu)\n",crc32,i,i>>1,dup,total);
                }
                *duplicatep = 1;
                break;
            }
            else if ( crcs[i] == 0 )
                break;
        }
        if ( i >= sizeof(crcs)/sizeof(*crcs) )
            i = (LP_rand() % (sizeof(crcs)/sizeof(*crcs)));
        return(i);
    }
    else
    {
        crcs[ind] = crc32;
        return(ind);
    }
}

int32_t LP_peerindsock(int32_t *peerindp)
{
    struct LP_peerinfo *peer,*tmp; int32_t peerind = 0;
    HASH_ITER(hh,LP_peerinfos,peer,tmp)
    {
        if ( peer->errors < LP_MAXPEER_ERRORS && peer->pushsock >= 0 )
        {
            if (peerind == *peerindp)
            {
                // printf("peerind.%d -> sock %d ip %s\n",peerind,peer->pushsock,peer->ipaddr);
                return (peer->pushsock);
            }
            peerind++;
        }
    }
    return(-1);
}

void _LP_queuesend(uint32_t crc32,int32_t sock0,int32_t sock1,uint8_t *msg,int32_t msglen,int32_t needack)
{
    int32_t i,maxind,flag = 0,peerind = 0; //sentbytes,
    maxind = LP_numpeers();
    //printf("%s\n", (char *)msg);
    // printf("num peers %d sock0 %d sock1 %d\n", maxind, sock0, sock1);
    if ( sock0 >= 0 ) {
        _LP_sendqueueadd(crc32, sock0, msg, msglen, 0);
    }
}

void LP_queuesend(uint32_t crc32,int32_t pubsock,char *base,char *rel,uint8_t *msg,int32_t msglen)
{
    portable_mutex_lock(&LP_networkmutex);
    if ( pubsock >= 0 )
        _LP_queuesend(crc32,pubsock,-1,msg,msglen,0);
    else _LP_queuesend(crc32,-1,-1,msg,msglen,1);
    portable_mutex_unlock(&LP_networkmutex);
}

// first 2 bytes == (crc32 & 0xffff) if encrypted, then nonce is next crypto_box_NONCEBYTES
// GENESIS_PRIVKEY is always the sender

void LP_broadcast_finish(int32_t pubsock,char *base,char *rel,uint8_t *msg,cJSON *argjson,uint32_t crc32)
{
    int32_t msglen; char *method;
    if ( (method= jstr(argjson,"method")) == 0 )
        return;
    msg = (void *)jprint(argjson,0);
    msglen = (int32_t)strlen((char *)msg) + 1;
    if ( crc32 == 0 )
        crc32 = calc_crc32(0,&msg[2],msglen - 2);
    //printf("crc32.%x IAMLP.%d pubsock.%d\n",crc32,G.LP_IAMLP,pubsock);
#ifdef FROM_MARKETMAKER
    if ( (G.LP_IAMLP == 0 || pubsock < 0) && strcmp(method,"psock") != 0 )
#else
    if ( (IAMLP == 0 || pubsock < 0) && strcmp(method,"psock") != 0 )
#endif
    {
        free(msg);
        //printf("broadcast %s\n",jstr(argjson,"method"));
        jdelete(argjson,"method");
        jaddstr(argjson,"method","broadcast");
        if ( jobj(argjson,"timestamp") == 0 )
            jaddnum(argjson,"timestamp",(uint32_t)time(NULL));
        // add signature here
        msg = (void *)jprint(argjson,0);
        msglen = (int32_t)strlen((char *)msg) + 1;
        LP_queuesend(crc32,-1,base,rel,msg,msglen);
        if ( pubsock >= 0 )
            LP_queuesend(crc32,pubsock,base,rel,msg,msglen);
    }
    else
    {
        LP_queuesend(crc32,pubsock,base,rel,msg,msglen);
    }
    free(msg);
}
    
void LP_broadcast_message(int32_t pubsock,char *base,char *rel,bits256 destpub25519,char *msgstr)
{
    uint8_t encoded[LP_ENCRYPTED_MAXSIZE],space[sizeof(encoded)],*msg,*nonce,*cipher; int32_t encrypted=0,msglen; uint32_t crc32=0; cJSON *argjson; char *methodstr,method[64],cipherstr[LP_ENCRYPTED_MAXSIZE*2+1];
    msglen = (int32_t)strlen(msgstr) + 1;
    msg = (void *)msgstr;
    if ( bits256_nonz(destpub25519) != 0 )
    {
        nonce = &encoded[2];
        OS_randombytes(nonce,crypto_box_NONCEBYTES);
        cipher = &encoded[2 + crypto_box_NONCEBYTES];
        msglen = _SuperNET_cipher(nonce,&encoded[2 + crypto_box_NONCEBYTES],msg,msglen,destpub25519,GENESIS_PRIVKEY,space);
        msglen += crypto_box_NONCEBYTES;
        crc32 = calc_crc32(0,&encoded[2],msglen);
        encoded[0] = crc32 & 0xff;
        encoded[1] = (crc32 >> 8) & 0xff;
        msg = encoded;
        msglen += 2;
        encrypted = 1;
        //printf("msgstr.(%s)\n",msgstr);
        free(msgstr), msgstr = 0;
    }
    if ( encrypted == 0 )
    {
        if ( (argjson= cJSON_Parse(msgstr)) != 0 )
        {
            if ( (methodstr= jstr(argjson,"method")) != 0 && strlen(methodstr) <= sizeof(method) )
            {
                strcpy(method,methodstr);
                jdelete(argjson,"method");
                if ( jobj(argjson,"method2") != 0 )
                    jdelete(argjson,"method2");
                jaddstr(argjson,"method2",method);
                jaddstr(argjson,"method",method);
                //if ( strncmp(method,"connect",7) == 0 || strcmp(method,"reserved") == 0 )
                //    printf("CRC32.%u (%s)\n",crc32,msgstr);
                LP_broadcast_finish(pubsock,base,rel,msg,argjson,0);
                //if ( strncmp(method,"connect",7) == 0 || strcmp(method,"reserved") == 0 )
                //    printf("finished %u\n",crc32);
            } // else printf("no valid method in (%s)\n",msgstr);
            free_json(argjson);
        } else printf("couldnt parse %p (%s)\n",msgstr,msgstr);
    }
    else
    {
        argjson = cJSON_CreateObject();
        init_hexbytes_noT(cipherstr,msg,msglen);
        jaddstr(argjson,"cipher",cipherstr);
        jaddstr(argjson,"method2","encrypted");
        jaddstr(argjson,"method","encrypted");
        LP_broadcast_finish(pubsock,base,rel,msg,argjson,crc32);
        free_json(argjson);
    }
    if ( msgstr != 0 )
        free(msgstr);
}

void LP_psockadd(int32_t ispaired,int32_t publicsock,uint16_t recvport,int32_t sendsock,uint16_t sendport,char *subaddr,char *publicaddr,int32_t cmdchannel)
{
    struct psock *ptr;
    portable_mutex_lock(&LP_psockmutex);
    PSOCKS = realloc(PSOCKS,sizeof(*PSOCKS) * (Numpsocks + 1));
    ptr = &PSOCKS[Numpsocks++];
    memset(ptr,0,sizeof(*ptr));
    ptr->ispaired = ispaired;
    ptr->cmdchannel = cmdchannel;
    ptr->publicsock = publicsock;
    ptr->publicport = recvport;
    ptr->sendsock = sendsock;
    ptr->sendport = sendport;
    safecopy(ptr->sendaddr,subaddr,sizeof(ptr->sendaddr));
    safecopy(ptr->publicaddr,publicaddr,sizeof(ptr->publicaddr));
    ptr->lasttime = (uint32_t)time(NULL);
    portable_mutex_unlock(&LP_psockmutex);
}
    
char *_LP_psock_create(int32_t *pullsockp,int32_t *pubsockp,char *ipaddr,uint16_t publicport,uint16_t subport,int32_t ispaired,int32_t cmdchannel,bits256 pubkey)
{
    int32_t i,pullsock,bindflag=(IAMLP != 0),pubsock,arg; struct LP_pubkey_info *pubp; char pushaddr[128],subaddr[128]; cJSON *retjson = 0;
    pullsock = pubsock = -1;
    *pullsockp = *pubsockp = -1;
    if ( cmdchannel != 0 && bits256_nonz(pubkey) == 0 )
    {
        printf("ignore cmdchannel request without pubkey\n");
        return(clonestr("{\"error\":\"cmdchannel needs pubkey\"}"));
    }
    if ( IAMLP != 0 && bits256_nonz(pubkey) != 0 )
    {
        if ( (pubp= LP_pubkeyadd(pubkey)) != 0 )
        {
            if ( pubp->pairsock >= 0 )
            {
                //printf("%s already has pairsock.%d\n",bits256_str(str,pubkey),pubp->pairsock);
                portable_mutex_lock(&LP_psockmutex);
                for (i=0; i<Numpsocks; i++)
                    if ( PSOCKS[i].publicsock == pubp->pairsock )
                    {
                        //PSOCKS[i].lasttime = (uint32_t)time(NULL) - PSOCK_KEEPALIVE - 1;
                        retjson = cJSON_CreateObject();
                        jaddstr(retjson,"result","success");
                        jaddstr(retjson,"LPipaddr",ipaddr);
                        jaddstr(retjson,"connectaddr",PSOCKS[i].sendaddr);
                        jaddnum(retjson,"connectport",PSOCKS[i].sendport);
                        jaddnum(retjson,"ispaired",PSOCKS[i].ispaired);
                        jaddnum(retjson,"cmdchannel",PSOCKS[i].cmdchannel);
                        jaddstr(retjson,"publicaddr",PSOCKS[i].publicaddr);
                        jaddnum(retjson,"publicport",PSOCKS[i].publicport);
                        //printf("cmd.%d publicaddr.(%s) for subaddr.(%s), pullsock.%d pubsock.%d\n",cmdchannel,pushaddr,subaddr,pullsock,pubsock);
                        *pullsockp = pullsock;
                        *pubsockp = pubsock;
                        portable_mutex_unlock(&LP_psockmutex);
                        return(jprint(retjson,1));
                    }
                portable_mutex_unlock(&LP_psockmutex);
            }
            //printf("pairsock for %s <- %d\n",bits256_str(str,pubkey),pullsock);
            //pubp->pairsock = pullsock;
        }
    }
    nanomsg_transportname(bindflag,pushaddr,ipaddr,publicport);
    nanomsg_transportname(bindflag,subaddr,ipaddr,subport);
    return(0);
}
    
char *LP_psock(int32_t *pullsockp,char *ipaddr,int32_t ispaired,int32_t cmdchannel,bits256 pubkey)
{
    char *retstr=0; uint16_t i,publicport,subport,maxport; int32_t pubsock=-1;
    *pullsockp = -1;
    //printf("LP_psock ipaddr.%s ispaird.%d cmdchannel.%d\n",ipaddr,ispaired,cmdchannel);
    if ( cmdchannel == 0 )
    {
        maxport = MAX_PSOCK_PORT;
        publicport = Psockport++;
        subport = Psockport++;
    }
    else
    {
        if ( cmdchannel != 0 && bits256_nonz(pubkey) == 0 )
            return(clonestr("{\"error\",\"cant do pairsock for null pubkey\"}"));
        maxport = 65534;
        publicport = subport = Pcmdport++;
    }
    for (i=0; i<maxport; i++)
    {
        if ( publicport < MIN_PSOCK_PORT )
            publicport = MIN_PSOCK_PORT+1;
        if ( cmdchannel == 0 && subport <= publicport )
            subport = publicport +  1;
        if ( (retstr= _LP_psock_create(pullsockp,&pubsock,ipaddr,publicport,subport,ispaired,cmdchannel,pubkey)) != 0 )
        {
            //printf("LP_psock returns.(%s)\n",retstr);
            return(retstr);
        }
        if ( cmdchannel == 0 )
            publicport+=2, subport+=2;
        else publicport++, subport++;
    }
    if ( Psockport >= MAX_PSOCK_PORT )
        Psockport = MIN_PSOCK_PORT;
    if ( Pcmdport >= 65534 )
        Pcmdport = MAX_PSOCK_PORT;
    return(clonestr("{\"error\",\"cant find psock ports\"}"));
}
