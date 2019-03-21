
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

use common::{free_c_ptr, nn, slice_to_malloc, HyRes, CJSON, CORE};
use common::mm_ctx::MmArc;
use crossbeam::channel;
use futures::{future, Future, Stream};
use libc::{c_char, c_void, strlen};
use serde_json::{self as json, Value as Json};
use std::ffi::{CStr, CString};
use std::fmt;
use std::ptr::null_mut;
use std::time::Duration;

use crate::lp_native_dex::lp_command_process;
use crate::lp_swap::save_stats_swap_status;
use crate::rpc::{dispatcher, DispatcherRes};

/*
struct psock
{
    uint32_t lasttime,lastping,errors;
    int32_t publicsock,sendsock,ispaired,cmdchannel;
    uint16_t publicport,sendport;
    char sendaddr[128],publicaddr[128];
} *PSOCKS;

uint16_t Numpsocks,Psockport = MIN_PSOCK_PORT,Pcmdport = MAX_PSOCK_PORT;
extern portable_mutex_t LP_commandQmutex;

#ifdef FROM_JS

int32_t nn_socket(int domain, int protocol)
{
    return(0);
}

int32_t nn_close(int s)
{
    return(0);
}

int32_t nn_setsockopt(int s, int level, int option, const void *optval,size_t optvallen)
{
    return(0);
}

int32_t nn_getsockopt(int s, int level, int option, void *optval,size_t *optvallen)
{
    return(0);
}

int32_t nn_bind(int s, const char *addr)
{
    return(-1);
}

int32_t nn_connect(int s, const char *addr)
{
    if ( strncmp("ws://",addr,strlen("ws://")) != 0 )
        return(-1);
    return(0);
}

int32_t nn_shutdown(int s, int how)
{
    return(0);
}

int32_t nn_send(int s, const void *buf, size_t len, int flags)
{
    printf("JS cant nn_send (%s)\n",(char *)buf);
    return(0);
}

int32_t nn_recv(int s, void *buf, size_t len, int flags)
{
    return(0);
}

int32_t nn_errno(void)
{
    return(-11);
}

const char *nn_strerror(int errnum)
{
    return("nanomsg error");
}

int32_t nn_poll(struct nn_pollfd *fds, int nfds, int timeout)
{
    return(0);
}


#endif
*/

pub fn nanomsg_transportname (bindflag: i32, ipaddr: &fmt::Display, port: u16) -> String {
    fomat! ("tcp://" if bindflag == 0 {(ipaddr)} else {"*"} ':' (port))
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
 }

int32_t LP_sockcheck(int32_t sock)
{
    struct nn_pollfd pfd;
    pfd.fd = sock;
    pfd.events = NN_POLLOUT;
    if ( nn_poll(&pfd,1,1) > 0 )
        return(1);
    else return(-1);
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

uint32_t _LP_magic_check(bits256 hash,bits256 magic)
{
    bits256 pubkey,shared;
    pubkey = curve25519(magic,curve25519_basepoint9());
    shared = curve25519(hash,pubkey);
    return(shared.uints[1] & ((1 << LP_MAGICBITS)-1));
}

bits256 LP_calc_magic(uint8_t *msg,int32_t len)
{
    static uint32_t maxn,counter,nsum; static double sum;
    bits256 magic,hash; int32_t n = 0; double millis;
    vcalc_sha256(0,hash.bytes,msg,len);
    millis = OS_milliseconds();
    while ( 1 )
    {
        magic = rand256(1);
        if ( _LP_magic_check(hash,magic) == LP_BARTERDEX_VERSION )
            break;
        n++;
    }
    sum += (OS_milliseconds() - millis);
    nsum += n;
    counter++;
    if ( n > maxn || (LP_rand() % 10000) == 0 )
    {
        if ( n > maxn )
        {
            printf("LP_calc_magic maxn.%d <- %d   | ",maxn,n);
            maxn = n;
        }
        printf("millis %.3f ave %.3f, aveiters %.1f\n",OS_milliseconds() - millis,sum/counter,(double)nsum/counter);
    }
    return(magic);
}

int32_t LP_magic_check(uint8_t *msg,int32_t recvlen,char *remoteaddr)
{
    bits256 magic,hash; uint32_t val;
    recvlen -= sizeof(bits256);
    if ( recvlen > 0 )
    {
        vcalc_sha256(0,hash.bytes,msg,recvlen);
        memcpy(magic.bytes,&msg[recvlen],sizeof(magic));
        val = _LP_magic_check(hash,magic);
        if ( val != LP_BARTERDEX_VERSION )
            printf("magicval = %x from %s\n",val,remoteaddr);
        return(val == LP_BARTERDEX_VERSION);
    }
    return(-1);
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
        peerind++;
        if ( peer->errors < LP_MAXPEER_ERRORS && peer->pushsock >= 0 )
        {
            if ( peerind < *peerindp )
                continue;
            *peerindp = peerind;
            //printf("peerind.%d -> sock %d\n",peerind,peer->pushsock);
            return(peer->pushsock);
        }
    }
    return(-1);
}

void _LP_queuesend(uint32_t crc32,int32_t sock0,int32_t sock1,uint8_t *msg,int32_t msglen,int32_t needack)
{
    int32_t i,maxind,flag = 0,peerind = 0; //sentbytes,
    for (i=0; i<2; i++)
    {
        if ( sock0 < 0 && sock1 < 0 )
        {
            if ( (maxind= LP_numpeers()) > 0 )
                peerind = (LP_rand() % maxind) + 1;
            else peerind = 1;
            sock0 = LP_peerindsock(&peerind);
            if ( (maxind= LP_numpeers()) > 0 )
                peerind = (LP_rand() % maxind) + 1;
            else peerind = 1;
            sock1 = LP_peerindsock(&peerind);
            flag = 1;
        }
        if ( sock0 >= 0 )
            _LP_sendqueueadd(crc32,sock0,msg,msglen,needack * peerind);
        if ( sock1 >= 0 )
            _LP_sendqueueadd(crc32,sock1,msg,msglen,needack);
        if ( flag == 0 )
            break;
        sock0 = sock1 = -1;
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
    
uint32_t LP_swapsend(int32_t pairsock,struct basilisk_swap *swap,uint32_t msgbits,uint8_t *data,int32_t datalen,uint32_t nextbits,uint32_t crcs[2])
{
    uint8_t *buf; int32_t sentbytes,offset=0,i;
    buf = malloc(datalen + sizeof(msgbits) + sizeof(swap->I.req.quoteid) + sizeof(bits256)*2);
    for (i=0; i<32; i++)
        buf[offset++] = swap->I.myhash.bytes[i];
    for (i=0; i<32; i++)
        buf[offset++] = swap->I.otherhash.bytes[i];
    offset += iguana_rwnum(1,&buf[offset],sizeof(swap->I.req.quoteid),&swap->I.req.quoteid);
    offset += iguana_rwnum(1,&buf[offset],sizeof(msgbits),&msgbits);
    if ( datalen > 0 )
        memcpy(&buf[offset],data,datalen), offset += datalen;
    if ( (sentbytes= nn_send(pairsock,buf,offset,0)) != offset )
    {
        printf("sentbytes.%d vs offset.%d\n",sentbytes,offset);
        if ( sentbytes < 0 )
        {
        }
    }
    //printf("sent %d bytes\n",sentbytes);
    //else printf("send.[%d] %x offset.%d datalen.%d [%llx]\n",sentbytes,msgbits,offset,datalen,*(long long *)data);
    free(buf);
    return(nextbits);
}
    
struct LP_queuedcommand
{
    struct LP_queuedcommand *next,*prev;
    char **retstrp;
    int32_t responsesock,msglen,stats_JSONonly;
    uint32_t queueid;
    char msg[];
} *LP_commandQ;
*/

#[derive(Debug)]
struct QueuedCommand {
    response_sock: i32,
    stats_json_only: i32,
    queue_id: u32,
    msg: String,
    // retstrp: *mut *mut c_char,
}

#[derive(Serialize)]
struct CommandForNn {
    result: Json,
    #[serde(rename="queueid")]
    queue_id: u32
}

lazy_static! {
    // TODO: Move to `MmCtx`.
    static ref COMMAND_QUEUE: (channel::Sender<QueuedCommand>, channel::Receiver<QueuedCommand>) = channel::unbounded();
}

/// Sends a reply to the `cmd.response_sock` peer.
fn reply_to_peer (cmd: QueuedCommand, mut reply: Vec<u8>) -> Result<(), String> {
    if cmd.response_sock >= 0 {
        if cmd.queue_id != 0 {
            let result = try_s! (json::from_slice (&reply));
            let nn_command = CommandForNn {
                queue_id: cmd.queue_id,
                result
            };

            reply = try_s! (json::to_vec (&nn_command))
        }

        // See also commits ce09bcd and 62f3cba: looks like we need the wired string to be zero-terminated.
        reply.push (0);

        let _size = unsafe {nn::nn_send (
            cmd.response_sock,
            slice_to_malloc (&reply) as *mut c_void,  // Officially it should be `nn_allocmsg` but looks like `malloc` will do.
            reply.len() as usize,
            0,
        )};
    }
    Ok(())
}

/// Run the RPC handler and send it's reply to a peer.
fn rpc_reply_to_peer (handler: HyRes, cmd: QueuedCommand) {
    let f = handler.then (move |r| -> Box<Future<Item=(), Error=()> + Send> {
        let res = match r {Ok (r) => r, Err (err) => {
            log! ("rpc_reply_to_peer] handler error: " (err));
            return Box::new (future::err(()))
        }};
        let body_f = res.into_body().concat2();
        Box::new (body_f.then (move |body| -> Result<(), ()> {
            let body = match body {Ok (r) => r, Err (err) => {
                log! ("rpc_reply_to_peer] error getting the body from the RPC handler: " (err));
                return Err(())
            }};
            if let Err (err) = reply_to_peer (cmd, body.to_vec()) {
                log! ("reply_to_peer error: " (err));
                return Err(())
            }
            Ok(())
        }))
    });
    CORE.spawn (|_| f)
}

/// The thread processing the peer-to-peer messaging bus.
pub unsafe fn lp_command_q_loop(ctx: MmArc) {
    loop {
        if ctx.is_stopping() { break }

        let cmd = match (*COMMAND_QUEUE).1.recv_timeout (Duration::from_millis (100)) {
            Ok (cmd) => cmd,
            Err (channel::RecvTimeoutError::Timeout) => continue,  // And check `is_stopping`.
            Err (channel::RecvTimeoutError::Disconnected) => break
        };

        let msg_json = unwrap!(json::from_str(&cmd.msg));
        if cmd.stats_json_only < 0 { // broadcast passthrough
            if cmd.response_sock >= 0 {
                let nn_command = CommandForNn {
                    queue_id: 0,
                    result: msg_json
                };
                let json_str = unwrap!(json::to_string(&nn_command));
                let c_str = unwrap!(CString::new(json_str));
                let _size = nn::nn_send(
                    cmd.response_sock,
                    c_str.as_ptr() as *const c_void,
                    strlen(c_str.as_ptr()) as usize,
                    0,
                );
            }
        } else {
            let json: Json = unwrap!(json::from_str(&cmd.msg));
            let method = json["method"].as_str();
            if let Some(m) = method {
                if m == "swapstatus" {
                    let handler = save_stats_swap_status(json["data"].clone());
                    rpc_reply_to_peer (handler, cmd);
                    continue;
                }
            }

            let json = match dispatcher (json, None, ctx.clone()) {
                DispatcherRes::Match (handler) => {
                    rpc_reply_to_peer (handler, cmd);
                    continue
                },
                DispatcherRes::NoMatch (req) => req
            };

            // Invokes `lp_trade_command` and the older `stats_JSON` code.
            let c_json = unwrap!(CJSON::from_str(&cmd.msg));
            let retstr = lp_command_process(
                ctx.clone(),
                cmd.response_sock,
                json,
                c_json,
                cmd.stats_json_only,
            );
            if !retstr.is_null() {
                let retvec = CStr::from_ptr (retstr) .to_bytes().to_vec();
                free_c_ptr(retstr as *mut c_void);

                if let Err (err) = reply_to_peer (cmd, retvec) {
                    log! ("reply_to_peer error: " (err))
                }
            }
        }
    }
}

/// Register an RPC command that came internally or from the peer-to-peer bus.
#[no_mangle]
pub extern "C" fn lp_queue_command (retstrp: *mut *mut c_char, buf: *mut c_char, response_sock: i32,
                                    stats_json_only: i32, queue_id: u32) -> () {
    if retstrp != null_mut() {
        unsafe { *retstrp = null_mut() }
    }

    if buf == null_mut() {panic! ("!buf")}
    let msg = String::from (unwrap! (unsafe {CStr::from_ptr (buf)} .to_str()));
    let cmd = QueuedCommand {
        msg,
        queue_id,
        response_sock,
        stats_json_only
    };
    unwrap! ((*COMMAND_QUEUE).0.send (cmd))
}
/*
void mynn_close(int32_t sock)
{
    struct nn_pollfd pfd; int32_t n; void *buf;
    if ( sock >= 0 )
    {
        while ( (n= nn_recv(sock,&buf,NN_MSG,0)) > 0 )
            printf("got n.%d bytes from nn_close(%d)\n",n,sock);
        pfd.fd = sock;
        pfd.events = POLLOUT;
        while ( nn_poll(&pfd,1,100) > 0 )
        {
            printf("cant send to nn_close(%d)\n",sock);
            sleep(1);
        }
        if ( IAMLP != 0 )
            nn_close(sock);
    }
}
    
void LP_psockloop(void *_ptr)
{
    static struct nn_pollfd *pfds;
    int32_t nexti=0,j,i,n,nonz,iter,retval,sentbytes,size=0,sendsock = -1; uint32_t now; struct psock *ptr=0; void *buf=0; char keepalive[512];
    strcpy(LP_psockloop_stats.name,"LP_psockloop");
    LP_psockloop_stats.threshold = 1000.;
    while ( LP_STOP_RECEIVED == 0 )
    {
        LP_millistats_update(&LP_psockloop_stats);
        now = (uint32_t)time(NULL);
        if ( buf != 0 && ptr != 0 && sendsock >= 0 )
        {
            if ( size > 0 )
            {
                if ( (sentbytes= nn_send(sendsock,buf,size,0)) != size ) // need tight loop
                    printf("LP_psockloop sent %d instead of %d\n",sentbytes,size);
                if ( buf != 0 )
                {
                    if ( buf != keepalive )
                        nn_freemsg(buf);
                    buf = 0;
                    size = 0;
                    ptr = 0;
                    sendsock = -1;
                }
            }
        }
        else if ( Numpsocks > 0 )
        {
            if ( pfds == 0 )
                pfds = calloc(MAX_PSOCK_PORT,sizeof(*pfds));
            nexti = (rand() % Numpsocks);
            portable_mutex_lock(&LP_psockmutex);
            memset(pfds,0,sizeof(*pfds) * ((Numpsocks*2 <= MAX_PSOCK_PORT) ? Numpsocks*2 : MAX_PSOCK_PORT));
            for (iter=j=0; iter<2; iter++)
            {
                for (j=n=0; j<Numpsocks; j++)
                {
                    i = (j + nexti) % Numpsocks;
                    ptr = &PSOCKS[i];
                    if ( iter == 0 )
                    {
                        pfds[n].fd = ptr->publicsock;
                        //printf("check sock.%d\n",ptr->publicsock);
                        pfds[n].events = POLLIN;
                    }
                    else
                    {
                        if ( pfds[n].fd != ptr->publicsock )
                        {
                            printf("unexpected fd.%d mismatched publicsock.%d\n",pfds[n].fd,ptr->publicsock);
                            break;
                        }
                        else if ( (pfds[n].revents & POLLIN) != 0 )
                        {
                            //printf("cmd.%d publicsock.%d %s has pollin\n",ptr->cmdchannel,ptr->publicsock,ptr->publicaddr);
                            buf = 0;
                            if ( (size= nn_recv(ptr->publicsock,&buf,NN_MSG,0)) > 0 )
                            {
                                ptr->lasttime = now;
                                if ( ptr->cmdchannel == 0 )
                                {
                                    sendsock = ptr->sendsock;
                                    break;
                                } else LP_queuecommand(0,(char *)buf,ptr->publicsock,0,0);
                            }
                            if ( buf != 0 )
                            {
                                nn_freemsg(buf);
                                buf = 0;
                                size = 0;
                            }
                        }
                    }
                    n++;
                    if ( ptr->sendsock > 0 )
                    {
                        if ( iter == 0 )
                        {
                            pfds[n].fd = ptr->sendsock;
                            pfds[n].events = POLLIN;
                        }
                        else
                        {
                            if ( pfds[n].fd != ptr->sendsock )
                            {
                                printf("unexpected fd.%d mismatched sendsock.%d\n",pfds[n].fd,ptr->sendsock);
                                break;
                            }
                            else if ( (pfds[n].revents & POLLIN) != 0 )
                            {
                                if ( (size= nn_recv(ptr->sendsock,&buf,NN_MSG,0)) > 0 )
                                {
                                    //printf("%s paired has pollin (%s)\n",ptr->sendaddr,(char *)buf);
                                    ptr->lasttime = now;
                                    if ( ptr->ispaired != 0 )
                                    {
                                        sendsock = ptr->publicsock;
                                        break;
                                    }
                                }
                                if ( buf != 0 )
                                {
                                    nn_freemsg(buf);
                                    buf = 0;
                                    size = 0;
                                }
                            }
                        }
                        n++;
                    }
                }
                if ( iter == 0 )
                {
                    if ( (retval= nn_poll(pfds,n,1)) <= 0 )
                    {
                        //if ( retval != 0 )
                        //    printf("nn_poll retval.%d\n",retval);
                        break;
                    } // else printf("num pfds.%d retval.%d\n",n,retval);
                }
            }
            if ( sendsock < 0 )
                usleep(10000);
            if ( 0 && IAMLP != 0 && sendsock < 0 )
            {
                usleep(30000);
                for (i=nonz=0; i<Numpsocks; i++)
                {
                    ptr = &PSOCKS[i];
                    if ( ptr->cmdchannel == 0 && now > ptr->lasttime+PSOCK_KEEPALIVE )
                    {
                        printf("PSOCKS[%d] of %d (%u %u) lag.%d IDLETIMEOUT\n",i,Numpsocks,ptr->publicport,ptr->sendport,now - ptr->lasttime);
                        if ( ptr->sendsock != ptr->publicsock && ptr->sendsock >= 0 )
                            nn_close(ptr->sendsock), ptr->sendsock = -1;
                        if ( ptr->publicsock >= 0 )
                            nn_close(ptr->publicsock), ptr->publicsock = -1;
                        nonz++;
                    }
                }
                if ( nonz > 0 )
                {
                    n = Numpsocks;
                    for (i=0; i<n; i++)
                    {
                        ptr = &PSOCKS[i];
                        if ( ptr->sendsock < 0 && ptr->publicsock < 0 )
                        {
                            for (j=i; j<n-1; j++)
                                PSOCKS[j] = PSOCKS[j+1];
                            n--;
                        }
                    }
                    printf("PSOCKS purge nonz.%d n.%d vs Numpsocks.%d\n",nonz,n,Numpsocks);
                    Numpsocks = n;
                }
            }
            portable_mutex_unlock(&LP_psockmutex);
        } else usleep(100000);
    }
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
    
int32_t LP_psockmark(char *publicaddr)
{
    int32_t i,retval = -1; struct psock *ptr;
    portable_mutex_lock(&LP_psockmutex);
    for (i=0; i<Numpsocks; i++)
    {
        ptr = &PSOCKS[i];
        if ( strcmp(publicaddr,ptr->publicaddr) == 0 )
        {
            printf("mark PSOCKS[%d] %s for deletion\n",i,publicaddr);
            ptr->lasttime = 0;
            retval = i;
            break;
        }
    }
    portable_mutex_unlock(&LP_psockmutex);
    return(retval);
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
    if ( (pullsock= nn_socket(AF_SP,ispaired!=0?NN_PAIR:NN_PULL)) >= 0 && (cmdchannel != 0 ||(pubsock= nn_socket(AF_SP,ispaired!=0?NN_PAIR:NN_PAIR)) >= 0) )
    {
        if ( nn_bind(pullsock,pushaddr) >= 0 && (cmdchannel != 0 || nn_bind(pubsock,subaddr) >= 0) )
        {
            arg = 100;
            nn_setsockopt(pullsock,NN_SOL_SOCKET,NN_SNDTIMEO,&arg,sizeof(arg));
            if ( pubsock >= 0 )
                nn_setsockopt(pubsock,NN_SOL_SOCKET,NN_SNDTIMEO,&arg,sizeof(arg));
            arg = 1;
            nn_setsockopt(pullsock,NN_SOL_SOCKET,NN_RCVTIMEO,&arg,sizeof(arg));
            if ( pubsock >= 0 )
                nn_setsockopt(pubsock,NN_SOL_SOCKET,NN_RCVTIMEO,&arg,sizeof(arg));
            //arg = 2;
            //nn_setsockopt(pullsock,NN_SOL_SOCKET,NN_MAXTTL,&arg,sizeof(arg));
            //if ( pubsock >= 0 )
            //    nn_setsockopt(pubsock,NN_SOL_SOCKET,NN_MAXTTL,&arg,sizeof(arg));
            nanomsg_transportname(0,pushaddr,ipaddr,publicport);
            nanomsg_transportname(0,subaddr,ipaddr,subport);
            LP_psockadd(ispaired,pullsock,publicport,pubsock,subport,subaddr,pushaddr,cmdchannel);
            retjson = cJSON_CreateObject();
            jaddstr(retjson,"result","success");
            jaddstr(retjson,"LPipaddr",ipaddr);
            jaddstr(retjson,"connectaddr",subaddr);
            jaddnum(retjson,"connectport",subport);
            jaddnum(retjson,"ispaired",ispaired);
            jaddnum(retjson,"cmdchannel",cmdchannel);
            jaddstr(retjson,"publicaddr",pushaddr);
            jaddnum(retjson,"publicport",publicport);
            if ( bits256_nonz(pubkey) != 0 && (pubp= LP_pubkeyadd(pubkey)) != 0 )
                pubp->pairsock = pullsock;
            char str[65]; printf("PSOCK %s cmd.%d publicaddr.(%s) for subaddr.(%s), pullsock.%d pubsock.%d\n",bits256_str(str,pubkey),cmdchannel,pushaddr,subaddr,pullsock,pubsock);
            *pullsockp = pullsock;
            *pubsockp = pubsock;
            return(jprint(retjson,1));
        } //else printf("bind error on %s or %s\n",pushaddr,subaddr);
        if ( pullsock >= 0 )
            nn_close(pullsock);
        if ( pubsock >= 0 )
            nn_close(pubsock);
    }
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
    
/*
 LP_pushaddr_get makes transparent the fact that most nodes cannot bind()!
 
 The idea is to create an LP node NN_PAIR sock that the LP node binds to and client node connects to. Additionally, the LP node creates an NN_PULL that other nodes can NN_PUSH to and returns this address in pushaddr/retval for the client node to register with. The desired result is that other than the initial LP node, all the other nodes do a normal NN_PUSH, requiring no change to the NN_PUSH/NN_PULL logic. Of course, the initial LP node needs to autoforward all packets from the public NN_PULL to the NN_PUB
 
 similar to LP_pushaddr_get, create an NN_PAIR for DEX atomic data, can be assumed to have a max lifetime of 2*INSTANTDEX_LOCKTIME
 
 both are combined in LP_psock_get
 
 */
    
char *issue_LP_psock(char *destip,uint16_t destport,int32_t ispaired,int32_t cmdchannel)
{
    char str[65],url[512],*retstr; bits256 pub25519; int32_t netid;
#ifdef FROM_MARKETMAKER
    pub25519 = G.LP_mypub25519;
    netid = G.netid;
#else
    extern bits256 BET_mypub25519;
    pub25519 = BET_mypub25519;
    netid = 0;
#endif
    sprintf(url,"http://%s:%u/api/stats/psock?ispaired=%d&cmdchannel=%d&pubkey=%s&netid=%d",destip,destport-1,ispaired,cmdchannel,bits256_str(str,pub25519),netid);
    //return(LP_issue_curl("psock",destip,destport,url));
    retstr = issue_curlt(url,LP_HTTP_TIMEOUT*10);
    printf("issue_LP_psock got (%s) from %s\n",retstr,url); // this is needed?!
    return(retstr);
}
    
uint16_t LP_psock_get(char *connectaddr,char *publicaddr,int32_t ispaired,int32_t cmdchannel,char *ipaddr)
{
    uint16_t publicport = 0; char *retstr,*addr; cJSON *retjson; struct LP_peerinfo *peer,*tmp;
    connectaddr[0] = publicaddr[0] = 0;
    HASH_ITER(hh,LP_peerinfos,peer,tmp)
    {
        if ( ipaddr != 0 && strcmp(ipaddr,peer->ipaddr) != 0 )
            continue;
        connectaddr[0] = publicaddr[0] = 0;
        if ( peer->errors < LP_MAXPEER_ERRORS && (retstr= issue_LP_psock(peer->ipaddr,peer->port,ispaired,cmdchannel)) != 0 )
        {
            if ( (retjson= cJSON_Parse(retstr)) != 0 )
            {
                //printf("from %s:%u (%s)\n",peer->ipaddr,peer->port,retstr);
                if ( (addr= jstr(retjson,"publicaddr")) != 0 )
                    safecopy(publicaddr,addr,128);
                if ( (addr= jstr(retjson,"connectaddr")) != 0 )
                    safecopy(connectaddr,addr,128);
                if ( publicaddr[0] != 0 && connectaddr[0] != 0 )
                    publicport = juint(retjson,"publicport");
                free_json(retjson);
            }
            //printf("got.(%s) connect.%s public.%s publicport.%u\n",retstr,connectaddr,publicaddr,publicport);
            free(retstr);
            return(publicport);
        } //else printf("error psock from %s:%u\n",peer->ipaddr,peer->port);
    }
    return(0);
}
    
int32_t LP_initpublicaddr(void *ctx,uint16_t *mypullportp,char *publicaddr,char *myipaddr,uint16_t mypullport,int32_t ispaired)
{
    int32_t nntype,pullsock,timeout; char bindaddr[128],connectaddr[128];
    *mypullportp = mypullport;
    if ( ispaired == 0 )
    {
        if ( LP_canbind != 0 )
            nntype = LP_COMMAND_RECVSOCK;
        else nntype = NN_PAIR;//NN_SUB;
    } else nntype = NN_PAIR;
    if ( LP_canbind != 0 )
    {
        nanomsg_transportname(0,publicaddr,myipaddr,mypullport);
        nanomsg_transportname(1,bindaddr,myipaddr,mypullport);
    }
    else
    {
        *mypullportp = 0;
        if ( ispaired == 0 )
        {
            sprintf(publicaddr,"127.0.0.1:%u",mypullport);
            return(-1);
        }
        while ( *mypullportp == 0 )
        {
            if ( (*mypullportp= LP_psock_get(connectaddr,publicaddr,ispaired,0,0)) != 0 )
                break;
            sleep(10);
            printf("try to get publicaddr again\n");
        }
    }
    while ( 1 )
    {
        if ( (pullsock= nn_socket(AF_SP,nntype)) >= 0 )
        {
            if ( LP_canbind == 0 )
            {
                if ( nn_connect(pullsock,connectaddr) < 0 )
                {
                    printf("bind to %s error for %s: %s\n",connectaddr,publicaddr,nn_strerror(nn_errno()));
                    exit(-1);
                }
                else
                {
                    printf("nntype.%d NN_PAIR.%d connect to %s connectsock.%d\n",nntype,NN_PAIR,connectaddr,pullsock);
                }
            }
            else
            {
                if ( nn_bind(pullsock,bindaddr) < 0 )
                {
                    printf("bind to %s error for %s: %s\n",bindaddr,publicaddr,nn_strerror(nn_errno()));
                    exit(-1);
                }
            }
            timeout = 100;
            nn_setsockopt(pullsock,NN_SOL_SOCKET,NN_RCVTIMEO,&timeout,sizeof(timeout));
            nn_setsockopt(pullsock,NN_SOL_SOCKET,NN_SNDTIMEO,&timeout,sizeof(timeout));
            if ( nntype == NN_SUB )
                nn_setsockopt(pullsock,NN_SUB,NN_SUB_SUBSCRIBE,"",0);
        }
        //if ( LP_canbind != 0 || ispaired != 0 || nn_tests(ctx,pullsock,publicaddr,NN_PUSH) >= 0 )
        //    break;
        //printf("nn_tests failed, try again\n");
        //sleep(3);
        break;
        if ( pullsock >= 0 )
            nn_close(pullsock);
    }
    return(pullsock);
}
*/
