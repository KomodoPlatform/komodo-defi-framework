
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
//  LP_transaction.c
//  marketmaker
//
bits256 LP_privkeyfind(uint8_t rmd160[20])
{
    int32_t i; static bits256 zero;
    for (i=0; i<G.LP_numprivkeys; i++)
        if ( memcmp(rmd160,G.LP_privkeys[i].rmd160,20) == 0 )
            return(G.LP_privkeys[i].privkey);
    //for (i=0; i<20; i++)
    //    printf("%02x",rmd160[i]);
    //printf(" -> no privkey\n");
    return(zero);
}

int32_t LP_privkeyadd(bits256 privkey,uint8_t rmd160[20])
{
    bits256 tmpkey;
    tmpkey = LP_privkeyfind(rmd160);
    if ( bits256_nonz(tmpkey) != 0 )
        return(-bits256_cmp(privkey,tmpkey));
    G.LP_privkeys[G.LP_numprivkeys].privkey = privkey;
    memcpy(G.LP_privkeys[G.LP_numprivkeys].rmd160,rmd160,20);
    //int32_t i; for (i=0; i<20; i++)
    //    printf("%02x",rmd160[i]);
    //char str[65]; printf(" -> add privkey.(%s)\n",bits256_str(str,privkey));
    G.LP_numprivkeys++;
    return(G.LP_numprivkeys);
}

bits256 LP_privkey(char *symbol,char *coinaddr,uint8_t taddr)
{
    bits256 privkey; uint8_t addrtype,rmd160[20];
    bitcoin_addr2rmd160(symbol,taddr,&addrtype,rmd160,coinaddr);
    privkey = LP_privkeyfind(rmd160);
    return(privkey);
}

bits256 LP_pubkey(bits256 privkey)
{
    bits256 pubkey;
    pubkey = curve25519(privkey,curve25519_basepoint9());
    return(pubkey);
}

int32_t LP_gettx_presence(int32_t *numconfirmsp,char *symbol,bits256 expectedtxid,char *coinaddr)
{
    cJSON *txobj,*retjson,*item; bits256 txid; struct iguana_info *coin; int32_t height=-1,i,n,flag = 0;
    if ( numconfirmsp != 0 )
        *numconfirmsp = -1;
    if ( (txobj= LP_gettx("LP_gettx_presence",symbol,expectedtxid,0)) != 0 )
    {
        txid = jbits256(txobj,"txid");
        if ( jobj(txobj,"error") == 0 && bits256_cmp(txid,expectedtxid) == 0 )
        {
            if ( numconfirmsp != 0 )
                *numconfirmsp = 0;
            if ( numconfirmsp != 0 && coinaddr != 0 && (coin= LP_coinfind(symbol)) != 0 && coin->electrum != 0 )
            {
                //char str[65]; printf("%s %s already in gettx (%s)\n",coinaddr,bits256_str(str,txid),jprint(txobj,0));
                if ( (retjson= electrum_address_gethistory(symbol,coin->electrum,&retjson,coinaddr,expectedtxid)) != 0 )
                {
                    if ( (n= cJSON_GetArraySize(retjson)) > 0 )
                    {
                        for (i=0; i<n; i++)
                        {
                            item = jitem(retjson,i);
                            if ( bits256_cmp(txid,jbits256(item,"tx_hash")) == 0 )
                            {
                                height = jint(item,"height");
                                //printf("found txid at height.%d\n",height);
                                if ( height <= coin->height )
                                    *numconfirmsp = (coin->height - height + 1);
                                break;
                            }
                        }
                    }
                    free_json(retjson);
                    //printf("got %s history height.%d vs coin.%d -> numconfirms.%d\n",coin->symbol,height,coin->height,*numconfirmsp);
                }
            }
            flag = 1;
        }
        free_json(txobj);
    }
    return(flag);
}

bits256 LP_broadcast(char *txname,char *symbol,char *txbytes,bits256 expectedtxid)
{
    char *retstr,*errstr; bits256 txid; uint8_t *ptr; cJSON *retjson,*errorobj; struct iguana_info *coin; int32_t i,totalretries=0,len,sentflag = 0;
    coin = LP_coinfind(symbol);
    memset(&txid,0,sizeof(txid));
    if ( txbytes == 0 || txbytes[0] == 0 )
        return(txid);
    if ( bits256_nonz(expectedtxid) == 0 )
    {
        len = (int32_t)strlen(txbytes) >> 1;
        ptr = malloc(len);
        decode_hex(ptr,len,txbytes);
        expectedtxid = bits256_calctxid(symbol,ptr,len);
        free(ptr);
    }
    for (i=0; i<2; i++)
    {
        //char str[65]; printf("LP_broadcast.%d %s (%s) %s i.%d sentflag.%d %s\n",i,txname,symbol,bits256_str(str,expectedtxid),i,sentflag,txbytes);
        if ( sentflag == 0 && LP_gettx_presence(0,symbol,expectedtxid,0) != 0 )
            sentflag = 1;
        if ( sentflag == 0 && (retstr= LP_sendrawtransaction(symbol,txbytes,0)) != 0 )
        {
            if ( is_hexstr(retstr,0) == 64 )
            {
                decode_hex(txid.bytes,32,retstr);
                if ( bits256_cmp(txid,expectedtxid) == 0 || (bits256_nonz(expectedtxid) == 0 && bits256_nonz(txid) != 0) )
                {
                    sentflag = 1;
                    expectedtxid = txid;
                }
            }
            else if ( (retjson= cJSON_Parse(retstr)) != 0 )
            {
                if ( (errorobj= jobj(retjson,"error")) != 0 )
                {
                    if ( jint(errorobj,"code") == -27 ) // "transaction already in block chain"
                    {
                        txid = expectedtxid;
                        sentflag = 1;
                    }
                    else if ( (errstr= jstr(retjson,"error")) != 0 && strcmp(errstr,"timeout") == 0 && coin != 0 && coin->electrum != 0 )
                    {
                        if ( totalretries < 4 )
                        {
                            printf("time error with electrum, retry.%d\n",totalretries);
                            totalretries++;
                            i--;
                        }
                    } else printf("broadcast error.(%s)\n",retstr);
                }
                free_json(retjson);
            }
            //char str[65]; printf("sentflag.%d [%s] %s RETSTR.(%s) %s.%s\n",sentflag,txname,txbytes,retstr,symbol,bits256_str(str,txid));
            free(retstr);
        }
        if ( sentflag != 0 )
            break;
        sleep(3);
    }
    if ( sentflag != 0 )
        return(expectedtxid);
    return(txid);
}

bits256 LP_broadcast_tx(char *name,char *symbol,uint8_t *data,int32_t datalen)
{
    bits256 txid; char *signedtx;
    memset(txid.bytes,0,sizeof(txid));
    if ( data != 0 && datalen != 0 )
    {
        signedtx = malloc(datalen*2 + 1);
        init_hexbytes_noT(signedtx,data,datalen);
        txid = bits256_calctxid(symbol,data,datalen);
#ifdef BASILISK_DISABLESENDTX
        char str[65]; printf("%s <- dont sendrawtransaction (%s) %s\n",name,bits256_str(str,txid),signedtx);
#else
        txid = LP_broadcast(name,symbol,signedtx,txid);
#endif
        free(signedtx);
    }
    return(txid);
}

int32_t iguana_msgtx_Vset(uint8_t *serialized,int32_t maxlen,struct iguana_msgtx *msgtx,struct vin_info *V)
{
    int32_t vini,j,scriptlen,p2shlen,userdatalen,siglen,plen,need_op0=0,len = 0; uint8_t *script,*redeemscript=0,*userdata=0; struct vin_info *vp;
    for (vini=0; vini<msgtx->tx_in; vini++)
    {
        vp = &V[vini];
        if ( (userdatalen= vp->userdatalen) == 0 )
        {
            userdatalen = vp->userdatalen = msgtx->vins[vini].userdatalen;
            userdata = msgtx->vins[vini].userdata;
        } else userdata = vp->userdata;
        if ( (p2shlen= vp->p2shlen) == 0 )
        {
            p2shlen = vp->p2shlen = msgtx->vins[vini].p2shlen;
            redeemscript = msgtx->vins[vini].redeemscript;
        }
        else
        {
            redeemscript = vp->p2shscript;
            msgtx->vins[vini].redeemscript = redeemscript;
        }
        if ( msgtx->vins[vini].spendlen > 33 && msgtx->vins[vini].spendscript[msgtx->vins[vini].spendlen - 1] == SCRIPT_OP_CHECKMULTISIG )
        {
            need_op0 = 1;
            printf("found multisig spendscript\n");
        }
        if ( redeemscript != 0 && p2shlen > 33 && redeemscript[p2shlen - 1] == SCRIPT_OP_CHECKMULTISIG )
        {
            need_op0 = 1;
            //printf("found multisig redeemscript\n");
        }
        msgtx->vins[vini].vinscript = script = &serialized[len];
        msgtx->vins[vini].vinscript[0] = 0;
        scriptlen = need_op0;
        for (j=0; j<vp->N; j++)
        {
            if ( (siglen= vp->signers[j].siglen) > 0 )
            {
                script[scriptlen++] = siglen;
                memcpy(&script[scriptlen],vp->signers[j].sig,siglen);
                scriptlen += siglen;
            }
        }
        msgtx->vins[vini].scriptlen = scriptlen;
        if ( vp->suppress_pubkeys == 0 && (vp->N > 1 || bitcoin_pubkeylen(&vp->spendscript[1]) != vp->spendscript[0] || vp->spendscript[vp->spendlen-1] != 0xac) )
        {
            for (j=0; j<vp->N; j++)
            {
                if ( (plen= bitcoin_pubkeylen(vp->signers[j].pubkey)) > 0 )
                {
                    script[scriptlen++] = plen;
                    memcpy(&script[scriptlen],vp->signers[j].pubkey,plen);
                    scriptlen += plen;
                }
            }
            msgtx->vins[vini].scriptlen = scriptlen;
        }
        if ( userdatalen != 0 )
        {
            memcpy(&script[scriptlen],userdata,userdatalen);
            msgtx->vins[vini].userdata = &script[scriptlen];
            msgtx->vins[vini].userdatalen = userdatalen;
            scriptlen += userdatalen;
        }
        //printf("USERDATALEN.%d scriptlen.%d redeemlen.%d\n",userdatalen,scriptlen,p2shlen);
        if ( p2shlen != 0 )
        {
            if ( p2shlen < 76 )
                script[scriptlen++] = p2shlen;
            else if ( p2shlen <= 0xff )
            {
                script[scriptlen++] = 0x4c;
                script[scriptlen++] = p2shlen;
            }
            else if ( p2shlen <= 0xffff )
            {
                script[scriptlen++] = 0x4d;
                script[scriptlen++] = (p2shlen & 0xff);
                script[scriptlen++] = ((p2shlen >> 8) & 0xff);
            } else return(-1);
            msgtx->vins[vini].p2shlen = p2shlen;
            memcpy(&script[scriptlen],redeemscript,p2shlen);
            scriptlen += p2shlen;
        }
        len += scriptlen;
    }
    if ( (0) )
    {
        int32_t i; for (i=0; i<len; i++)
            printf("%02x",script[i]);
        printf(" <-script len.%d scriptlen.%d p2shlen.%d user.%d\n",len,scriptlen,p2shlen,userdatalen);
    }
    return(len);
}

int32_t iguana_interpreter(struct iguana_info *coin,cJSON *logarray,int64_t nLockTime,struct vin_info *V,int32_t numvins)
{
    uint8_t *script,*activescript,*savescript; char *str; int32_t vini,scriptlen,activescriptlen,savelen,errs = 0; cJSON *spendscript,*item=0;
    script = calloc(1,IGUANA_MAXSCRIPTSIZE);
    savescript = calloc(1,IGUANA_MAXSCRIPTSIZE);
    str = calloc(1,IGUANA_MAXSCRIPTSIZE*2+1);
    for (vini=0; vini<numvins; vini++)
    {
        savelen = V[vini].spendlen;
        memcpy(savescript,V[vini].spendscript,savelen);
        if ( V[vini].p2shlen > 0 )
        {
            activescript = V[vini].p2shscript;
            activescriptlen = V[vini].p2shlen;
        }
        else
        {
            activescript = V[vini].spendscript;
            activescriptlen = V[vini].spendlen;
        }
        memcpy(V[vini].spendscript,activescript,activescriptlen);
        V[vini].spendlen = activescriptlen;
        spendscript = iguana_spendasm(activescript,activescriptlen);
        if ( activescriptlen < 16 )
            continue;
        //printf("interpreter.(%s)\n",jprint(spendscript,0));
        //printf("bitcoin_assembler ignore_cltverr.%d suppress.%d\n",V[vini].ignore_cltverr,V[vini].suppress_pubkeys);
        if ( (scriptlen= bitcoin_assembler(coin,logarray,script,spendscript,1,nLockTime,&V[vini])) < 0 )
        {
            //printf("bitcoin_assembler error scriptlen.%d\n",scriptlen);
            errs++;
        }
        else if ( scriptlen != activescriptlen || memcmp(script,activescript,scriptlen) != 0 )
        {
            if ( logarray != 0 )
            {
                item = cJSON_CreateObject();
                jaddstr(item,"error","script reconstruction failed");
            }
            init_hexbytes_noT(str,activescript,activescriptlen);
            //printf("activescript.(%s)\n",str);
            if ( logarray != 0 && item != 0 )
                jaddstr(item,"original",str);
            init_hexbytes_noT(str,script,scriptlen);
            //printf("reconstructed.(%s)\n",str);
            if ( logarray != 0 )
            {
                jaddstr(item,"reconstructed",str);
                jaddi(logarray,item);
            } else printf(" scriptlen mismatch.%d vs %d or miscompare\n",scriptlen,activescriptlen);
            errs++;
        }
        memcpy(V[vini].spendscript,savescript,savelen);
        V[vini].spendlen = savelen;
    }
    free(str);
    free(script);
    free(savescript);
    if ( errs != 0 )
        return(-errs);
    if ( logarray != 0 )
    {
        item = cJSON_CreateObject();
        jaddstr(item,"result","success");
        jaddi(logarray,item);
    }
    return(0);
}

bits256 iguana_str2priv(char *symbol,uint8_t wiftaddr,char *str)
{
    bits256 privkey; int32_t n; uint8_t addrtype; //struct iguana_waccount *wacct=0; struct iguana_waddress *waddr;
    memset(&privkey,0,sizeof(privkey));
    if ( str != 0 )
    {
        n = (int32_t)strlen(str) >> 1;
        if ( n == sizeof(bits256) && is_hexstr(str,sizeof(bits256)) > 0 )
            decode_hex(privkey.bytes,sizeof(privkey),str);
        else if ( bitcoin_wif2priv(symbol,wiftaddr,&addrtype,&privkey,str) != sizeof(bits256) )
        {
            //if ( (waddr= iguana_waddresssearch(&wacct,str)) != 0 )
            //    privkey = waddr->privkey;
            //else memset(privkey.bytes,0,sizeof(privkey));
        }
    }
    return(privkey);
}

int32_t iguana_vininfo_create(char *symbol,uint8_t taddr,uint8_t pubtype,uint8_t p2shtype,uint8_t isPoS,uint8_t *serialized,int32_t maxsize,struct iguana_msgtx *msgtx,cJSON *vins,int32_t numinputs,struct vin_info *V)
{
    int32_t i,plen,finalized = 1,len = 0; struct vin_info *vp; //struct iguana_waccount *wacct; struct iguana_waddress *waddr; uint32_t sigsize,pubkeysize,p2shsize,userdatalen;
    msgtx->tx_in = numinputs;
    maxsize -= (sizeof(struct iguana_msgvin) * msgtx->tx_in);
    msgtx->vins = (struct iguana_msgvin *)&serialized[maxsize];
    memset(msgtx->vins,0,sizeof(struct iguana_msgvin) * msgtx->tx_in);
    if ( msgtx->tx_in > 0 && msgtx->tx_in*sizeof(struct iguana_msgvin) < maxsize )
    {
        for (i=0; i<msgtx->tx_in; i++)
        {
            vp = &V[i];
            //printf("VINS.(%s)\n",jprint(jitem(vins,i),0));
            len += iguana_parsevinobj(&serialized[len],maxsize,&msgtx->vins[i],jitem(vins,i),vp);
            if ( msgtx->vins[i].sequence < IGUANA_SEQUENCEID_FINAL )
                finalized = 0;
            if ( msgtx->vins[i].spendscript == 0 )
            {
                /*if ( iguana_RTunspentindfind(coin,&outpt,vp->coinaddr,vp->spendscript,&vp->spendlen,&vp->amount,&vp->height,msgtx->vins[i].prev_hash,msgtx->vins[i].prev_vout,coin->bundlescount-1,0) == 0 )
                 {
                 vp->unspentind = outpt.unspentind;
                 msgtx->vins[i].spendscript = vp->spendscript;
                 msgtx->vins[i].spendlen = vp->spendlen;
                 vp->hashtype = iguana_vinscriptparse(coin,vp,&sigsize,&pubkeysize,&p2shsize,&userdatalen,vp->spendscript,vp->spendlen);
                 vp->userdatalen = userdatalen;
                 printf("V %.8f (%s) spendscript.[%d] userdatalen.%d\n",dstr(vp->amount),vp->coinaddr,vp->spendlen,userdatalen);
                 }*/
            }
            else
            {
                memcpy(vp->spendscript,msgtx->vins[i].spendscript,msgtx->vins[i].spendlen);
                vp->spendlen = msgtx->vins[i].spendlen;
                _iguana_calcrmd160(symbol,taddr,pubtype,p2shtype,vp);
                if ( (plen= bitcoin_pubkeylen(vp->signers[0].pubkey)) > 0 )
                    bitcoin_address(symbol,vp->coinaddr,taddr,pubtype,vp->signers[0].pubkey,plen);
            }
            if ( vp->M == 0 && vp->N == 0 )
                vp->M = vp->N = 1;
            /*if ( vp->coinaddr[i] != 0 && (waddr= iguana_waddresssearch(&wacct,vp->coinaddr)) != 0 )
             {
             vp->signers[0].privkey = waddr->privkey;
             if ( (plen= bitcoin_pubkeylen(waddr->pubkey)) != vp->spendscript[1] || vp->spendscript[vp->spendlen-1] != 0xac )
             {
             if ( plen > 0 && plen < sizeof(vp->signers[0].pubkey) )
             memcpy(vp->signers[0].pubkey,waddr->pubkey,plen);
             }
             }*/
        }
    }
    return(finalized);
}

int32_t bitcoin_verifyvins(void *ctx,char *symbol,uint8_t taddr,uint8_t pubtype,uint8_t p2shtype,uint8_t isPoS,int32_t height,bits256 *signedtxidp,char **signedtx,struct iguana_msgtx *msgtx,uint8_t *serialized,int32_t maxlen,struct vin_info *V,uint32_t sighash,int32_t signtx,int32_t suppress_pubkeys,int32_t zcash)
{
    bits256 sigtxid; int64_t spendamount; uint8_t *sig,*script; struct vin_info *vp; char vpnstr[64]; int32_t scriptlen,complete=0,j,vini=0,flag=0,siglen,numvouts,numsigs;
    numvouts = msgtx->tx_out;
    vpnstr[0] = 0;
    *signedtx = 0;
    memset(signedtxidp,0,sizeof(*signedtxidp));
//printf("bitcoin_verifyvins suppress.%d numvins.%d numvouts.%d signtx.%d privkey.%d M.%d N.%d\n",suppress_pubkeys,msgtx->tx_in,numvouts,signtx,bits256_nonz(V[0].signers[0].privkey),V[0].M,V[0].N);
    for (vini=0; vini<msgtx->tx_in; vini++)
    {
        if ( V->p2shscript[0] != 0 && V->p2shlen != 0 )
        {
            script = V->p2shscript;
            scriptlen = V->p2shlen;
        }
        else
        {
            script = msgtx->vins[vini].spendscript;
            scriptlen = msgtx->vins[vini].spendlen;
        }
        spendamount = LP_outpoint_amount(symbol,msgtx->vins[vini].prev_hash,msgtx->vins[vini].prev_vout);
        sigtxid = bitcoin_sigtxid(symbol,taddr,pubtype,p2shtype,isPoS,height,serialized,maxlen,msgtx,vini,script,scriptlen,spendamount,sighash,vpnstr,suppress_pubkeys,zcash);
        if ( bits256_nonz(sigtxid) != 0 )
        {
            vp = &V[vini];
            vp->sigtxid = sigtxid;
            for (j=numsigs=0; j<vp->N; j++)
            {
                sig = vp->signers[j].sig;
                siglen = vp->signers[j].siglen;
                if ( signtx != 0 && bits256_nonz(vp->signers[j].privkey) != 0 )
                {
                    siglen = bitcoin_sign(ctx,symbol,sig,sigtxid,vp->signers[j].privkey,0);
                    //if ( (plen= bitcoin_pubkeylen(vp->signers[j].pubkey)) <= 0 )
                    bitcoin_pubkey33(ctx,vp->signers[j].pubkey,vp->signers[j].privkey);
                    sig[siglen++] = sighash;
                    vp->signers[j].siglen = siglen;
                    /*char str[65]; printf("SIGTXID.(%s) ",bits256_str(str,sigtxid));
                     int32_t i; for (i=0; i<siglen; i++)
                     printf("%02x",sig[i]);
                     printf(" sig, ");
                     for (i=0; i<33; i++)
                     printf("%02x",vp->signers[j].pubkey[i]);
                     // s2 = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141 - s1;
                     printf(" SIGNEDTX.[%02x] siglen.%d priv.%s\n",sig[siglen-1],siglen,bits256_str(str,vp->signers[j].privkey));*/
                }
                if ( sig == 0 || siglen == 0 )
                {
                    memset(vp->signers[j].pubkey,0,sizeof(vp->signers[j].pubkey));
                    char str[65]; printf("no sig.%p or siglen.%d zero priv.(%s)\n",sig,siglen,bits256_str(str,vp->signers[j].privkey));
                    continue;
                }
                if ( bitcoin_verify(ctx,sig,siglen-1,sigtxid,vp->signers[j].pubkey,bitcoin_pubkeylen(vp->signers[j].pubkey)) < 0 )
                {
                    int32_t k; for (k=0; k<bitcoin_pubkeylen(vp->signers[j].pubkey); k++)
                        printf("%02x",vp->signers[j].pubkey[k]);
                    printf(" SIG.%d.%d ERROR siglen.%d\n",vini,j,siglen);
                }
                else
                {
                    flag++;
                    numsigs++;
                    /*int32_t z; char tmpaddr[64];
                    for (z=0; z<siglen-1; z++)
                        printf("%02x",sig[z]);
                    printf(" <- sig[%d]\n",j);
                    for (z=0; z<33; z++)
                        printf("%02x",vp->signers[j].pubkey[z]);
                    bitcoin_address(tmpaddr,0,0,vp->signers[j].pubkey,33);
                    printf(" <- pub, SIG.%d.%d VERIFIED numsigs.%d vs M.%d %s\n",vini,j,numsigs,vp->M,tmpaddr);*/
                }
            }
            if ( numsigs >= vp->M )
                complete = 1;
        } else if ( signtx != 0 )
            printf("bitcoin_verifyvins cant without privkey\n");
    }
    iguana_msgtx_Vset(serialized,maxlen,msgtx,V);
    cJSON *txobj = 0;//cJSON_CreateObject();
    *signedtx = iguana_rawtxbytes(symbol,taddr,pubtype,p2shtype,isPoS,height,txobj,msgtx,suppress_pubkeys,zcash);
    //printf("SIGNEDTX.(%s)\n",jprint(txobj,1));
    *signedtxidp = msgtx->txid;
    return(complete);
}

int64_t iguana_lockval(int32_t finalized,int64_t locktime)
{
    int64_t lockval = -1;
    if ( finalized == 0 )
        return(locktime);
    return(lockval);
}

int32_t iguana_signrawtransaction(void *ctx,char *symbol,uint8_t wiftaddr,uint8_t taddr,uint8_t pubtype,uint8_t p2shtype,uint8_t isPoS,int32_t height,struct iguana_msgtx *msgtx,char **signedtxp,bits256 *signedtxidp,struct vin_info *V,int32_t numinputs,char *rawtx,cJSON *vins,cJSON *privkeysjson,int32_t zcash)
{
    uint8_t *serialized,*serialized2,*serialized3,*serialized4,*extraspace,pubkeys[64][33]; int32_t finalized,i,len,n,z,plen,maxsize,complete = 0,extralen = 100000; char *privkeystr,*signedtx = 0; uint32_t sighash; bits256 privkeys[LP_MAXVINS],privkey,txid; cJSON *item; cJSON *txobj = 0;
    maxsize = 1000000;
    memset(privkey.bytes,0,sizeof(privkey));
    if ( rawtx != 0 && rawtx[0] != 0 && (len= (int32_t)strlen(rawtx)>>1) < maxsize )
    {
        serialized = malloc(maxsize);
        serialized2 = malloc(maxsize);
        serialized3 = malloc(maxsize);
        serialized4 = malloc(maxsize);
        extraspace = malloc(extralen);
        memset(msgtx,0,sizeof(*msgtx));
        decode_hex(serialized,len,rawtx);
        if ( (txobj= bitcoin_hex2json(symbol,taddr,pubtype,p2shtype,isPoS,height,&txid,msgtx,rawtx,extraspace,extralen,serialized4,vins,V->suppress_pubkeys,zcash)) != 0 )
        {
            //printf("back from bitcoin_hex2json (%s)\n",jprint(vins,0));
        } else printf("no txobj from bitcoin_hex2json\n");
        //printf("call hex2json.(%s) vins.(%s)\n",rawtx,jprint(vins,0));
        if ( (numinputs= cJSON_GetArraySize(vins)) > 0 )
        {
            //printf("numinputs.%d (%s) msgtx.%d\n",numinputs,jprint(vins,0),msgtx->tx_in);
            memset(msgtx,0,sizeof(*msgtx));
            if ( iguana_rwmsgtx(symbol,taddr,pubtype,p2shtype,isPoS,height,0,0,serialized,maxsize,msgtx,&txid,"",extraspace,extralen,vins,V->suppress_pubkeys,zcash) > 0 && numinputs == msgtx->tx_in )
            {
                memset(pubkeys,0,sizeof(pubkeys));
                memset(privkeys,0,sizeof(privkeys));
                if ( (n= cJSON_GetArraySize(privkeysjson)) > 0 )
                {
                    for (i=0; i<n; i++)
                    {
                        item = jitem(privkeysjson,i);
                        privkeystr = jstr(item,0);
                        if ( privkeystr == 0 || privkeystr[0] == 0 )
                            continue;
                        privkeys[i] = privkey = iguana_str2priv(symbol,wiftaddr,privkeystr);
                        bitcoin_pubkey33(ctx,pubkeys[i],privkey);
                        //if ( bits256_nonz(privkey) != 0 )
                        //    iguana_ensure_privkey(coin,privkey);
                    }
                }
                //printf("after privkeys tx_in.%d\n",msgtx->tx_in);
                for (i=0; i<msgtx->tx_in; i++)
                {
                    if ( msgtx->vins[i].p2shlen != 0 )
                    {
                        char coinaddr[64]; uint32_t userdatalen,hashtype,sigsize,pubkeysize; uint8_t *userdata; int32_t j,k,type,flag; struct vin_info mvin,mainvin; bits256 zero;
                        memset(zero.bytes,0,sizeof(zero));
                        coinaddr[0] = 0;
                        sigsize = 0;
                        flag = (msgtx->vins[i].vinscript[0] == 0);
                        type = bitcoin_scriptget(symbol,taddr,pubtype,p2shtype,&hashtype,&sigsize,&pubkeysize,&userdata,&userdatalen,&mainvin,msgtx->vins[i].vinscript+flag,msgtx->vins[i].scriptlen-flag,0,zcash);
                        //printf("i.%d flag.%d type.%d scriptlen.%d\n",i,flag,type,msgtx->vins[i].scriptlen);
                        if ( msgtx->vins[i].redeemscript != 0 )
                        {
                            //for (j=0; j<msgtx->vins[i].p2shlen; j++)
                            //    printf("%02x",msgtx->vins[i].redeemscript[j]);
                            bitcoin_address(symbol,coinaddr,taddr,p2shtype,msgtx->vins[i].redeemscript,msgtx->vins[i].p2shlen);
                            type = iguana_calcrmd160(symbol,taddr,pubtype,p2shtype,0,&mvin,msgtx->vins[i].redeemscript,msgtx->vins[i].p2shlen,zero,0,0);
                            for (j=0; j<mvin.N; j++)
                            {
                                if ( V->suppress_pubkeys == 0 )
                                {
                                    for (z=0; z<33; z++)
                                        V[i].signers[j].pubkey[z] = mvin.signers[j].pubkey[z];
                                }
                                if ( flag != 0 && pubkeysize == 33 && mainvin.signers[0].siglen != 0 ) // jl777: need to generalize
                                {
                                    if ( memcmp(mvin.signers[j].pubkey,mainvin.signers[0].pubkey,33) == 0 )
                                    {
                                        for (z=0; z<mainvin.signers[0].siglen; z++)
                                            V[i].signers[j].sig[z] = mainvin.signers[0].sig[z];
                                        V[i].signers[j].siglen = mainvin.signers[j].siglen;
                                        printf("[%d].signer[%d] <- from mainvin.[0]\n",i,j);
                                    }
                                }
                                for (k=0; k<n; k++)
                                {
                                    if ( V[i].signers[j].siglen == 0 && memcmp(mvin.signers[j].pubkey,pubkeys[k],33) == 0 )
                                    {
                                        V[i].signers[j].privkey = privkeys[k];
                                        if ( V->suppress_pubkeys == 0 )
                                        {
                                            for (z=0; z<33; z++)
                                                V[i].signers[j].pubkey[z] = pubkeys[k][z];
                                        }
                                        //printf("%s -> V[%d].signer.[%d] <- privkey.%d\n",mvin.signers[j].coinaddr,i,j,k);
                                        break;
                                    }
                                }
                            }
                            //printf("type.%d p2sh.[%d] -> %s M.%d N.%d\n",type,i,mvin.coinaddr,mvin.M,mvin.N);
                        }
                    }
                    if ( i < V->N )
                        V->signers[i].privkey = privkey;
                    if ( i < numinputs )
                        V[i].signers[0].privkey = privkey;
                    plen = bitcoin_pubkeylen(V->signers[i].pubkey);
                    if ( V->suppress_pubkeys == 0 && plen <= 0 )
                    {
                        if ( i < numinputs )
                        {
                            for (z=0; z<plen; z++)
                                V[i].signers[0].pubkey[z] = V->signers[i].pubkey[z];
                        }
                    }
                }
                finalized = iguana_vininfo_create(symbol,taddr,pubtype,p2shtype,isPoS,serialized2,maxsize,msgtx,vins,numinputs,V);
                //printf("finalized.%d ignore_cltverr.%d suppress.%d\n",finalized,V[0].ignore_cltverr,V[0].suppress_pubkeys);
                sighash = LP_sighash(symbol,zcash);
                if ( (complete= bitcoin_verifyvins(ctx,symbol,taddr,pubtype,p2shtype,isPoS,height,signedtxidp,&signedtx,msgtx,serialized3,maxsize,V,sighash,1,V->suppress_pubkeys,zcash)) > 0 && signedtx != 0 )
                {
                    /*int32_t tmp; //char str[65];
                    if ( (tmp= iguana_interpreter(ctx,cJSON_CreateArray(),iguana_lockval(finalized,jint(txobj,"locktime")),V,numinputs)) < 0 )
                    {
                        printf("iguana_interpreter %d error.(%s)\n",tmp,signedtx);
                        complete = 0;
                    } else printf("interpreter passed\n");*/
                } else printf("complete.%d\n",complete);
            } else printf("rwmsgtx error\n");
        } else printf("no inputs in vins.(%s)\n",vins!=0?jprint(vins,0):"null");
        free(extraspace);
        free(serialized), free(serialized2), free(serialized3), free(serialized4);
    } else return(-1);
    if ( txobj != 0 )
        free_json(txobj);
    *signedtxp = signedtx;
    return(complete);
}

char *iguana_validaterawtx(void *ctx,struct iguana_info *coin,struct iguana_msgtx *msgtx,uint8_t *extraspace,int32_t extralen,char *rawtx,int32_t mempool,int32_t suppress_pubkeys,int32_t zcash)
{
    bits256 signedtxid; cJSON *item,*vins,*vouts,*txobj,*retjson,*sobj; char *scriptsig,*signedtx; uint32_t sighash; int32_t sigsize,slen,height,finalized = 1,i,len,maxsize,numinputs,numoutputs,complete; struct vin_info *V; uint8_t *serialized,*serialized2,scriptbuf[256]; int64_t inputsum,outputsum; struct iguana_msgvout vout;
    char *symbol; uint8_t wiftaddr,taddr,pubtype,p2shtype,isPoS;
    height = coin->longestchain;
    symbol = coin->symbol;
    wiftaddr = coin->wiftaddr;
    taddr = coin->taddr;
    pubtype = coin->pubtype;
    p2shtype = coin->p2shtype;
    isPoS = coin->isPoS;
    retjson = cJSON_CreateObject();
    inputsum = outputsum = numinputs = numoutputs = 0;
    if ( rawtx != 0 && rawtx[0] != 0 )
    {
        if ( (strlen(rawtx) & 1) != 0 )
            return(clonestr("{\"error\":\"rawtx hex has odd length\"}"));
        memset(msgtx,0,sizeof(*msgtx));
        if ( (txobj= bitcoin_hex2json(symbol,taddr,pubtype,p2shtype,isPoS,height,&msgtx->txid,msgtx,rawtx,extraspace,extralen,0,0,suppress_pubkeys,zcash)) != 0 )
        {
            maxsize = (int32_t)strlen(rawtx);
            serialized = malloc(maxsize);
            serialized2 = malloc(maxsize);
            if ( (vouts= jarray(&numoutputs,txobj,"vout")) > 0 )
            {
                for (i=0; i<numoutputs; i++)
                {
                    if ( iguana_parsevoutobj(serialized,maxsize,&vout,jitem(vouts,i)) > 0 )
                        outputsum += vout.value;
                }
            }
            if ( (vins= jarray(&numinputs,txobj,"vin")) > 0 )
            {
                V = calloc(numinputs,sizeof(*V));
                len = 0;
                for (i=0; i<numinputs; i++)
                {
                    if ( V[i].M == 0 )
                        V[i].M = 1;
                    if ( V[i].N < V[i].M )
                        V[i].N = V[i].M;
                    item = jitem(vins,i);
                    if ( strcmp(jstr(item,"txid"),"b19ce2c564f7dc57b3f95593e2b287c72d388e86de12dc562d9f8a6bea65b310") == 0 && jint(item,"vout") == 1 )
                    {
                        V[i].spendlen = 25;
                        decode_hex(V[i].spendscript,V[i].spendlen,"76a91459fdba29ea85c65ad90f6d38f7a6646476b26b1688ac");
                        msgtx->lock_time = 0;
                        V[i].amount = 2746715;
                        strcpy(V[i].coinaddr,"19Cq6MBaD8LY7trqs99ypqKAms3GcLs6J9");
                        V[i].suppress_pubkeys = 0;
                        decode_hex(msgtx->vins[i].prev_hash.bytes,32,"b19ce2c564f7dc57b3f95593e2b287c72d388e86de12dc562d9f8a6bea65b310");
                        msgtx->vins[i].prev_vout = 1;
                        msgtx->vins[i].sequence = 0xffffffff;
                        sobj = cJSON_CreateObject();
                        jaddstr(sobj,"hex","76a91459fdba29ea85c65ad90f6d38f7a6646476b26b1688ac");
                        jadd(item,"scriptPubKey",sobj);
                        printf("match special txid B\n");
                        V[i].signers[0].privkey = G.LP_privkey;
                    }
                    msgtx->vins[i].spendscript = V[i].spendscript;
                    msgtx->vins[i].spendlen = V[i].spendlen;
                    if ( (sobj= jobj(item,"scriptSig")) != 0 )
                    {
                        if ( (scriptsig= jstr(sobj,"hex")) != 0 )
                        {
                            slen = (int32_t)strlen(scriptsig) >> 1;
                            if ( slen <= sizeof(scriptbuf) )
                            {
                                msgtx->vins[i].scriptlen = slen;
                                msgtx->vins[i].vinscript = scriptbuf;
                                decode_hex(scriptbuf,slen,scriptsig);
                                if ( (sigsize= scriptbuf[0]) >= 70 && sigsize < 76 )
                                {
                                    memcpy(V[i].signers[0].sig,scriptbuf+1,sigsize-1);
                                    V[i].signers[0].siglen = sigsize - 1;
                                    V[i].hashtype = scriptbuf[1 + sigsize-1];
                                    if ( scriptbuf[sigsize+1] == 33 )
                                    {
                                        memcpy(V[i].signers[0].pubkey,&scriptbuf[sigsize+2],33);
                                        uint8_t rmd160[20]; char rmdstr[42];
                                        calc_rmd160(rmdstr,rmd160,V[i].signers[0].pubkey,33);
                                        printf("RMD160.%s\n",rmdstr);
                                    }
                                } else printf("sigsize.%d unexpected\n",sigsize);
                            }
                        }
                    }
                    inputsum += V[i].amount;
                    if ( msgtx->vins[i].sequence < IGUANA_SEQUENCEID_FINAL )
                        finalized = 0;
                }
                sighash = LP_sighash(symbol,zcash);
                complete = bitcoin_verifyvins(ctx,symbol,taddr,pubtype,p2shtype,isPoS,height,&signedtxid,&signedtx,msgtx,serialized2,maxsize,V,sighash,0,V[0].suppress_pubkeys,zcash);
                msgtx->txid = signedtxid;
                /*cJSON *log = cJSON_CreateArray();
                if ( iguana_interpreter(ctx,log,0,V,numinputs) < 0 )
                    jaddstr(retjson,"error","interpreter rejects tx");
                else complete = 1;
                jadd(retjson,"interpreter",log);*/
                jadd(retjson,"complete",complete!=0?jtrue():jfalse());
                if ( signedtx != 0 )
                    free(signedtx);
                free(V);
            }
            free(serialized), free(serialized2);
        }
        //char str[65]; printf("got txid.(%s)\n",bits256_str(str,txid));
    }
    msgtx->inputsum = inputsum;
    msgtx->numinputs = numinputs;
    msgtx->outputsum = outputsum;
    msgtx->numoutputs = numoutputs;
    msgtx->txfee = (inputsum - outputsum);
    return(jprint(retjson,1));
}

void test_validate(struct iguana_info *coin,char *signedtx)
{
    char *retstr; uint8_t extraspace[8192]; int32_t mempool=0; struct iguana_msgtx msgtx;
    retstr = iguana_validaterawtx(bitcoin_ctx(),coin,&msgtx,extraspace,sizeof(extraspace),signedtx,mempool,0,coin->zcash);
    printf("validate test.(%s)\n",retstr);
}

char *basilisk_swap_bobtxspend(bits256 *signedtxidp,uint64_t txfee,char *name,char *symbol,uint8_t wiftaddr,uint8_t taddr,uint8_t pubtype,uint8_t p2shtype,uint8_t isPoS,uint8_t wiftype,void *ctx,bits256 privkey,bits256 *privkey2p,uint8_t *redeemscript,int32_t redeemlen,uint8_t *userdata,int32_t userdatalen,bits256 utxotxid,int32_t utxovout,char *destaddr,uint8_t *pubkey33,int32_t finalseqid,uint32_t expiration,int64_t *destamountp,uint64_t satoshis,char *changeaddr,char *vinaddr,int32_t suppress_pubkeys,int32_t zcash)
{
    char *rawtxbytes=0,*signedtx=0,tmpaddr[64],hexstr[999],wifstr[128],_destaddr[64]; uint8_t spendscript[512],addrtype,rmd160[20]; cJSON *txobj,*vins,*obj,*vouts,*item,*privkeys; int32_t completed,spendlen,n,ignore_cltverr=1; struct vin_info V[8]; uint32_t timestamp,locktime = 0,sequenceid = 0xffffffff * finalseqid; bits256 txid; uint64_t value=0,change = 0; struct iguana_msgtx msgtx; struct iguana_info *coin;
    LP_mark_spent(symbol,utxotxid,utxovout);
    *destamountp = 0;
    memset(signedtxidp,0,sizeof(*signedtxidp));
    if ( finalseqid == 0 )
        locktime = expiration;
    //printf("bobtxspend.%s redeem.[%d]\n",symbol,redeemlen);
    if ( redeemlen < 0 )
        return(0);
    value = 0;
    if ( (coin= LP_coinfind(symbol)) != 0 )
    {
        if ( coin->etomic[0] != 0 )
        {
            if ( (coin= LP_coinfind("ETOMIC")) == 0 )
                return(0);
            symbol = coin->symbol;
        }
        if ( txfee > 0 && txfee < coin->txfee )
            txfee = coin->txfee;
#ifndef BASILISK_DISABLESENDTX
        if ( (txobj= LP_gettx("basilisk_swap_bobtxspend",symbol,utxotxid,0)) != 0 )
        {
            if ( (vouts= jarray(&n,txobj,"vout")) != 0 && utxovout < n )
            {
                obj = jitem(vouts,utxovout);
                value = LP_value_extract(obj,1,utxotxid);
                //printf("value in vout.%d %.8f (%s)\n",vout,dstr(value),jprint(txobj,0));
            }
            free_json(txobj);
        } else printf("cant gettx\n");
        if ( value == 0 )
        {
            //printf("basilisk_swap_bobtxspend.%s %s utxo.(%s).v%d already spent or doesnt exist\n",name,symbol,bits256_str(str,utxotxid),utxovout);
            return(0);
        }
#endif
    }
    if ( txfee > 0 && txfee < LP_MIN_TXFEE )
        txfee = LP_MIN_TXFEE;
    if ( satoshis != 0 )
    {
        if ( value < satoshis+txfee )
        {
            if ( (value-satoshis) > 3*txfee/4 )
            {
                satoshis = value - 3*txfee/4;
                printf("reduce satoshis %.8f by txfee %.8f to value %.8f\n",dstr(satoshis),dstr(txfee),dstr(value));
            }
            else if ( value == satoshis && (double)txfee/value < 0.25 )
            {
                satoshis = value - txfee;
                //printf("txfee allocation from value %.8f identical to satoshis: %.8f txfee %.8f\n",dstr(value),dstr(satoshis),dstr(txfee));
            }
            else
            {
                printf("utxo %.8f too small for %.8f + %.8f\n",dstr(value),dstr(satoshis),dstr(txfee));
                return(0);
            }
        }
        if ( value > satoshis+txfee )
            change = value - (satoshis + txfee);
        //printf("utxo %.8f, destamount %.8f change %.8f txfee %.8f\n",dstr(value),dstr(satoshis),dstr(change),dstr(txfee));
    } else if ( value > txfee )
        satoshis = value - txfee;
    else
    {
        printf("unexpected small value %.8f vs txfee %.8f\n",dstr(value),dstr(txfee));
        change = 0;
        satoshis = value >> 1;
        txfee = (value - satoshis);
        printf("unexpected small value %.8f vs txfee %.8f -> %.8f %.8f\n",dstr(value),dstr(txfee),dstr(satoshis),dstr(txfee));
    }
    if ( change < 6000 )
    {
        satoshis += change;
        change = 0;
    }
    if ( destamountp != 0 )
        *destamountp = satoshis;
    timestamp = (uint32_t)time(NULL);
    memset(V,0,sizeof(V));
    privkeys = cJSON_CreateArray();
    if ( privkey2p != 0 )
    {
        V[0].signers[1].privkey = *privkey2p;
        bitcoin_pubkey33(ctx,V[0].signers[1].pubkey,*privkey2p);
        bitcoin_priv2wif(symbol,wiftaddr,wifstr,*privkey2p,wiftype);
        jaddistr(privkeys,wifstr);
        V[0].N = V[0].M = 2;
    } else V[0].N = V[0].M = 1;
    V[0].signers[0].privkey = privkey;
    bitcoin_pubkey33(ctx,V[0].signers[0].pubkey,privkey);
    bitcoin_priv2wif(symbol,wiftaddr,wifstr,privkey,wiftype);
    jaddistr(privkeys,wifstr);
    V[0].suppress_pubkeys = suppress_pubkeys;
    V[0].ignore_cltverr = ignore_cltverr;
    if ( redeemlen != 0 )
        memcpy(V[0].p2shscript,redeemscript,redeemlen), V[0].p2shlen = redeemlen;
    txobj = bitcoin_txcreate(symbol,isPoS,locktime,coin->txversion,timestamp);
    vins = cJSON_CreateArray();
    item = cJSON_CreateObject();
    if ( userdata != 0 && userdatalen > 0 )
    {
        memcpy(V[0].userdata,userdata,userdatalen);
        V[0].userdatalen = userdatalen;
        init_hexbytes_noT(hexstr,userdata,userdatalen);
        jaddstr(item,"userdata",hexstr);
    }
    jaddbits256(item,"txid",utxotxid);
    jaddnum(item,"vout",utxovout);
    bitcoin_address(symbol,tmpaddr,taddr,pubtype,pubkey33,33);
    bitcoin_addr2rmd160(symbol,taddr,&addrtype,rmd160,tmpaddr);
    if ( redeemlen != 0 )
    {
        init_hexbytes_noT(hexstr,redeemscript,redeemlen);
        jaddstr(item,"redeemScript",hexstr);
        if ( vinaddr != 0 )
            bitcoin_addr2rmd160(symbol,taddr,&addrtype,rmd160,vinaddr);
        spendlen = bitcoin_p2shspend(spendscript,0,rmd160);
        //printf("P2SH path.%s\n",vinaddr!=0?vinaddr:0);
    } else spendlen = bitcoin_standardspend(spendscript,0,rmd160);
    init_hexbytes_noT(hexstr,spendscript,spendlen);
    jaddstr(item,"scriptPubKey",hexstr);
    jaddnum(item,"suppress",suppress_pubkeys);
    jaddnum(item,"sequence",sequenceid);
    jaddi(vins,item);
    jdelete(txobj,"vin");
    jadd(txobj,"vin",vins);
    if ( destaddr == 0 )
    {
        destaddr = _destaddr;
        bitcoin_address(symbol,destaddr,taddr,pubtype,pubkey33,33);
    }
    bitcoin_addr2rmd160(symbol,taddr,&addrtype,rmd160,destaddr);
    if ( addrtype == p2shtype )
        spendlen = bitcoin_p2shspend(spendscript,0,rmd160);
    else spendlen = bitcoin_standardspend(spendscript,0,rmd160);
    if ( change != 0 && strcmp(changeaddr,destaddr) == 0 )
    {
        printf("combine change %.8f -> %s\n",dstr(change),changeaddr);
        satoshis += change;
        change = 0;
    }
    txobj = bitcoin_txoutput(txobj,spendscript,spendlen,satoshis);
    if ( change != 0 )
    {
        int32_t changelen; uint8_t changescript[1024],changetype,changermd160[20];
        bitcoin_addr2rmd160(symbol,taddr,&changetype,changermd160,changeaddr);
        changelen = bitcoin_standardspend(changescript,0,changermd160);
        txobj = bitcoin_txoutput(txobj,changescript,changelen,change);
    }
    if ( (rawtxbytes= bitcoin_json2hex(symbol,isPoS,&txid,txobj,V)) != 0 )
    {
        char str[65];
        completed = 0;
        memset(signedtxidp,0,sizeof(*signedtxidp));
        //printf("locktime.%u sequenceid.%x rawtx.(%s) vins.(%s)\n",locktime,sequenceid,rawtxbytes,jprint(vins,0));
        if ( (completed= iguana_signrawtransaction(ctx,symbol,wiftaddr,taddr,pubtype,p2shtype,isPoS,1000000,&msgtx,&signedtx,signedtxidp,V,1,rawtxbytes,vins,privkeys,zcash)) < 0 )
        //if ( (signedtx= LP_signrawtx(symbol,signedtxidp,&completed,vins,rawtxbytes,privkeys,V)) == 0 )
            printf("couldnt sign transaction.%s %s\n",name,bits256_str(str,*signedtxidp));
        else if ( completed == 0 )
        {
            printf("incomplete signing suppress.%d %s (%s)\n",suppress_pubkeys,name,jprint(vins,0));
            if ( signedtx != 0 )
                free(signedtx), signedtx = 0;
        } // else printf("basilisk_swap_bobtxspend %s -> %s\n",name,bits256_str(str,*signedtxidp));
        free(rawtxbytes);
    } else printf("error making rawtx suppress.%d\n",suppress_pubkeys);
    free_json(privkeys);
    free_json(txobj);
    return(signedtx);
}

int32_t LP_vin_select(int32_t *aboveip,int64_t *abovep,int32_t *belowip,int64_t *belowp,struct LP_address_utxo **utxos,int32_t numunspents,uint64_t value,int32_t maxmode)
{
    int32_t i,abovei,belowi; int64_t above,below,gap,atx_value;
    abovei = belowi = -1;
    for (above=below=i=0; i<numunspents; i++)
    {
        if ( utxos[i] == 0 )
            continue;
        if ( (atx_value= utxos[i]->U.value) <= 0 )
        {
            //printf("illegal value.%d\n",i);
            continue;
        }
        if ( atx_value == value )
        {
            *aboveip = *belowip = i;
            *abovep = *belowp = 0;
            return(i);
        }
        else if ( atx_value > value )
        {
            gap = (atx_value - value);
            if ( above == 0 || gap < above )
            {
                above = gap;
                abovei = i;
            }
        }
        else
        {
            gap = (value - atx_value);
            if ( below == 0 || gap < below )
            {
                below = gap;
                belowi = i;
            }
        }
        //printf("value %.8f gap %.8f abovei.%d %.8f belowi.%d %.8f\n",dstr(value),dstr(gap),abovei,dstr(above),belowi,dstr(below));
    }
    *aboveip = abovei;
    *abovep = above;
    *belowip = belowi;
    *belowp = below;
    //printf("above.%d below.%d\n",abovei,belowi);
    if ( abovei >= 0 && belowi >= 0 )
    {
        if ( above < (below >> 1) )
            return(abovei);
        else return(belowi);
    }
    else if ( abovei >= 0 )
        return(abovei);
    else return(belowi);
    //return(abovei >= 0 && above < (below>>1) ? abovei : belowi);
}

cJSON *LP_inputjson(bits256 txid,int32_t vout,char *spendscriptstr)
{
    cJSON *sobj,*item = cJSON_CreateObject();
    jaddbits256(item,"txid",txid);
    jaddnum(item,"vout",vout);
    sobj = cJSON_CreateObject();
    jaddstr(sobj,"hex",spendscriptstr);
    jadd(item,"scriptPubKey",sobj);
    //printf("vin.%s\n",jprint(item,0));
    return(item);
}

int64_t LP_hodlcoin_interest(int32_t coinheight,int32_t utxoheight,bits256 txid,int64_t nValue)
{
    int64_t interest = 0; int32_t minutes,htdiff;
    if ( coinheight > utxoheight )
    {
        htdiff = (coinheight - utxoheight);
        if ( htdiff > 16830 )
            htdiff = 16830;
        minutes = (htdiff * 154) / 60;
        interest = nValue * htdiff * 0.000000238418;
        //interest = ((nValue * minutes) / 10743920);
    }
    return(interest);
}

uint64_t _komodo_interestnew(uint64_t nValue,uint32_t nLockTime,uint32_t tiptime)
{
    int32_t minutes; uint64_t interest = 0;
    if ( tiptime > nLockTime && (minutes= (tiptime - nLockTime) / 60) >= 60 )
    {
        //minutes.71582779 tiptime.1511292969 locktime.1511293505
        //printf("minutes.%d tiptime.%u locktime.%u\n",minutes,tiptime,nLockTime);
        if ( minutes > 365 * 24 * 60 )
            minutes = 365 * 24 * 60;
        if ( nLockTime > 1536000000 && minutes > 31*24*60 )
            minutes = 31 * 24 * 60;
        minutes -= 59;
        interest = ((nValue / 10512000) * minutes);
    }
    return(interest);
}

int64_t LP_komodo_interest(bits256 txid,int64_t value)
{
    uint32_t nLockTime; uint32_t tiptime; int64_t interest = 0;
    if ( value >= 10*SATOSHIDEN )
    {
        if ( (nLockTime= LP_locktime("KMD",txid)) >= 500000000 )
        {
            tiptime = (uint32_t)time(NULL) - 777;
            interest = _komodo_interestnew(value,nLockTime,tiptime);
        }
    }
    return(interest);
}

int32_t LP_vins_select(void *ctx,struct iguana_info *coin,int64_t *totalp,int64_t amount,struct vin_info *V,struct LP_address_utxo **utxos,int32_t numunspents,int32_t suppress_pubkeys,int32_t ignore_cltverr,bits256 privkey,cJSON *privkeys,cJSON *vins,uint8_t *script,int32_t scriptlen,bits256 utxotxid,int32_t utxovout,int32_t dustcombine)
{
    char wifstr[128],spendscriptstr[128],str[65]; int32_t i,j,maxiters,n,numpre,ind,abovei,belowi,maxmode=0; struct vin_info *vp; cJSON *txobj; struct LP_address_utxo *up,*min0,*min1,*preselected[3]; int64_t value,interest,interestsum,above,below,remains = amount,total = 0;
    *totalp = 0;
    interestsum = 0;
    init_hexbytes_noT(spendscriptstr,script,scriptlen);
    bitcoin_priv2wif(coin->symbol,coin->wiftaddr,wifstr,privkey,coin->wiftype);
    n = 0;
    min0 = min1 = 0;
    memset(preselected,0,sizeof(preselected));
    for (j=numpre=0; j<numunspents; j++)
    {
        up = utxos[j];
        if ( utxovout == up->U.vout && bits256_cmp(utxotxid,up->U.txid) == 0 )
        {
            preselected[numpre++] = up;
            //printf("found utxotxid in slot.%d\n",j);
            utxos[j] = 0;
            continue;
        }
        if ( up->spendheight <= 0 && up->U.height > 0 && up->U.value != 0 )
        {
            if ( (txobj= LP_gettxout(coin->symbol,coin->smartaddr,up->U.txid,up->U.vout)) == 0 )
            {
                up->spendheight = 1;
                utxos[j] = 0;
            }
            else
            {
                if ( LP_inventory_prevent(1,coin->symbol,up->U.txid,up->U.vout) == 0 )
                {
                    if ( min1 == 0 || up->U.value < min1->U.value )
                    {
                        if ( min0 == 0 || up->U.value < min0->U.value )
                        {
                            min1 = min0;
                            min0 = up;
                        } else min1 = up;
                    }
                } else utxos[j] = 0;
                if ( 0 && utxos[j] != 0 )
                    printf("gettxout j.%d %s/v%d (%s)\n",j,bits256_str(str,up->U.txid),up->U.vout,jprint(txobj,0));
                free_json(txobj);
            }
        } else utxos[j] = 0;
    }
    if ( bits256_nonz(utxotxid) != 0 && numpre == 0 )
    {
        up = LP_address_utxofind(coin,coin->smartaddr,utxotxid,utxovout);
        //printf("have utxotxid but wasnt found up.%p\n",up);
        if ( up == 0 )
        {
            value = LP_txvalue(0,coin->symbol,utxotxid,utxovout);
            LP_address_utxoadd(0,(uint32_t)time(NULL),"withdraw",coin,coin->smartaddr,utxotxid,utxovout,value,1,-1);
            //printf("added after not finding\n");
        }
        if ( (up= LP_address_utxofind(coin,coin->smartaddr,utxotxid,utxovout)) != 0 )
            preselected[numpre++] = up;
        else
        {
            //printf("couldnt add address_utxo %s/v%d after not finding\n",bits256_str(str,utxotxid),utxovout);
            sleep(1);
            value = LP_txvalue(0,coin->symbol,utxotxid,utxovout);
            LP_address_utxoadd(0,(uint32_t)time(NULL),"withdraw",coin,coin->smartaddr,utxotxid,utxovout,value,1,-1);
            if ( (up= LP_address_utxofind(coin,coin->smartaddr,utxotxid,utxovout)) != 0 )
                preselected[numpre++] = up;
            else printf("second couldnt add address_utxo %s/v%d after not finding\n",bits256_str(str,utxotxid),utxovout);
            //return(0);
        }
    }
    if ( dustcombine >= 1 && min0 != 0 && min0->U.value < LP_DUSTCOMBINE_THRESHOLD && (coin->electrum == 0 || min0->SPV > 0) )
    {
        for (j=0; j<numpre; j++)
            if ( min0 == preselected[j] )
                break;
        if ( j == numpre )
            preselected[numpre++] = min0;
    }
    else min0 = 0;
    if ( dustcombine >= 2 && min1 != 0 && min1->U.value < LP_DUSTCOMBINE_THRESHOLD && (coin->electrum == 0 || min1->SPV > 0) )
    {
        for (j=0; j<numpre; j++)
            if ( min1 == preselected[j] )
                break;
        if ( j == numpre )
            preselected[numpre++] = min1;
    }
    else min1 = 0;
    
    printf("dustcombine.%d numpre.%d min0.%p min1.%p numutxos.%d amount %.8f\n",dustcombine,numpre,min0,min1,numunspents,dstr(amount));
    maxiters = numunspents+numpre;
    for (i=0; i<maxiters; i++)
    {
        if ( i < numpre )
        {
            up = preselected[i];
            char str[65]; printf("preselected[%d]: %s/v%d %.8f\n",i,bits256_str(str,up->U.txid),up->U.vout,dstr(up->U.value));
        }
        else
        {
            below = above = 0;
            abovei = belowi = -1;
            if ( LP_vin_select(&abovei,&above,&belowi,&below,utxos,numunspents,remains,maxmode) < 0 )
            {
                printf("error finding unspent i.%d of %d, %.8f vs %.8f\n",i,numunspents,dstr(remains),dstr(amount));
                return(0);
            }
            if ( belowi < 0 || abovei >= 0 )
                ind = abovei;
            else ind = belowi;
            if ( ind < 0 )
            {
                printf("error finding unspent i.%d of %d, %.8f vs %.8f, abovei.%d belowi.%d ind.%d\n",i,numunspents,dstr(remains),dstr(amount),abovei,belowi,ind);
                return(0);
            }
            up = utxos[ind];
            utxos[ind] = utxos[--numunspents];
            utxos[numunspents] = 0;
            for (j=0; j<numpre; j++)
                if ( up == preselected[j] )
                    break;
            if ( j < numpre )
                continue;
            if ( LP_validSPV(coin->symbol,coin->smartaddr,up->U.txid,up->U.vout) < 0 )
                continue;
        }
        if ( bits256_cmp(utxotxid,up->U.txid) != 0 && LP_allocated(up->U.txid,up->U.vout) != 0 )
            continue;
        up->spendheight = 1;
        total += up->U.value;
        remains -= up->U.value;
        interest = 0;
        if ( up->U.height < 7777777 && strcmp(coin->symbol,"KMD") == 0 )
        {
            if ( (interest= LP_komodo_interest(up->U.txid,up->U.value)) > 0 )
            {
                interestsum += interest;
                char str[65]; printf("%s/%d %.8f interest %.8f -> sum %.8f\n",bits256_str(str,up->U.txid),up->U.vout,dstr(up->U.value),dstr(interest),dstr(interestsum));
            }
        }
        else if ( strcmp(coin->symbol,"HODLC") == 0 )
        {
            if ( (interest= LP_hodlcoin_interest(coin->height,up->U.height,up->U.txid,up->U.value)) > 0 )
            {
                interestsum += interest;
                char str[65]; printf("%s/%d %.8f hodl interest %.8f -> sum %.8f\n",bits256_str(str,up->U.txid),up->U.vout,dstr(up->U.value),dstr(interest),dstr(interestsum));
            }
        }
        //printf("suppress.%d numunspents.%d vini.%d value %.8f, total %.8f remains %.8f interest %.8f sum %.8f %s/v%d\n",suppress_pubkeys,numunspents,n,dstr(up->U.value),dstr(total),dstr(remains),dstr(interest),dstr(interestsum),bits256_str(str,up->U.txid),up->U.vout);
        vp = &V[n++];
        vp->N = vp->M = 1;
        vp->signers[0].privkey = privkey;
        jaddistr(privkeys,wifstr);
        bitcoin_pubkey33(ctx,vp->signers[0].pubkey,privkey);
        vp->suppress_pubkeys = suppress_pubkeys;
        vp->ignore_cltverr = ignore_cltverr;
        jaddi(vins,LP_inputjson(up->U.txid,up->U.vout,spendscriptstr));
        LP_unavailableset(up->U.txid,up->U.vout,(uint32_t)time(NULL)+LP_RESERVETIME*2,G.LP_mypub25519);
        if ( remains <= 0 && i >= numpre-1 )
            break;
        if ( numunspents < 0 || n >= LP_MAXVINS )
        {
            printf("total %.8f not enough for amount %.8f\n",dstr(total),dstr(amount));
            return(0);
        }
    }
    *totalp = total + interestsum;
    return(n);
}

char *LP_createrawtransaction(cJSON **txobjp,int32_t *numvinsp,struct iguana_info *coin,struct vin_info *V,int32_t max,bits256 privkey,cJSON *outputs,cJSON *vins,cJSON *privkeys,int64_t txfee,bits256 utxotxid,int32_t utxovout,int32_t onevin,uint32_t locktime,char *opretstr,char *passphrase)
{
    static void *ctx;
    cJSON *txobj,*item; uint8_t addrtype,rmd160[20],data[8192+64],script[8192],spendscript[256]; char *coinaddr,*rawtxbytes,*scriptstr; bits256 txid; uint32_t crc32,timestamp; int64_t change=0,adjust=0,total,value,amount = 0; int32_t origspendlen=0,i,offset,len,dustcombine,scriptlen,spendlen,suppress_pubkeys,ignore_cltverr,numvouts=0,numvins=0,numutxos=0; struct LP_address_utxo *utxos[LP_MAXVINS*256]; struct LP_address *ap;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    *numvinsp = 0;
    *txobjp = 0;
    /*if ( sizeof(utxos)/sizeof(*utxos) != max )
    {
        printf("LP_createrawtransaction: internal error %d != max.%d\n",(int32_t)(sizeof(utxos)/sizeof(*utxos)),max);
        return(0);
    }*/
    if ( coin == 0 || outputs == 0 || (numvouts= cJSON_GetArraySize(outputs)) <= 0 )
    {
        printf("LP_createrawtransaction: illegal coin.%p outputs.%p or arraysize.%d, error\n",coin,outputs,numvouts);
        return(0);
    }
    if ( coin->numutxos < LP_MINDESIRED_UTXOS )
        dustcombine = 0;
    else if ( coin->numutxos >= LP_MAXDESIRED_UTXOS )
        dustcombine = 2;
    else dustcombine = 1;
#ifdef LP_DISABLE_DISTCOMBINE
    dustcombine = 0;
#endif
    amount = txfee;
    for (i=0; i<numvouts; i++)
    {
        item = jitem(outputs,i);
        if ( (coinaddr= jfieldname(item)) != 0 )
        {
            if ( LP_address_isvalid(coin->symbol,coinaddr) <= 0 )
            {
                printf("%s LP_createrawtransaction %s i.%d of %d is invalid\n",coin->symbol,coinaddr,i,numvouts);
                return(0);
            }
            if ( (value= SATOSHIDEN * jdouble(item,coinaddr)) <= 0 )
            {
                printf("cant get value %s i.%d of %d %s\n",coinaddr,i,numvouts,jprint(outputs,0));
                return(0);
            }
            amount += value;
            //printf("vout.%d %.8f -> total %.8f\n",i,dstr(value),dstr(amount));
        }
        else
        {
            printf("cant get fieldname.%d of %d %s\n",i,numvouts,jprint(outputs,0));
            return(0);
        }
    }
    if ( (ap= LP_address(coin,coin->smartaddr)) == 0 )
    {
        printf("LP_createrawtransaction LP_address null?\n");
        return(0);
    }
    memset(utxos,0,sizeof(utxos));
    //char str[65];
    if ( onevin != 0 )
    {
        if ( (txobj= LP_gettxout(coin->symbol,coin->smartaddr,utxotxid,utxovout)) != 0 )
        {
            struct LP_address_utxo U;
            memset(&U,0,sizeof(U));
            utxos[0] = &U;
            utxos[0]->U.txid = utxotxid;
            utxos[0]->U.vout = utxovout;
            utxos[0]->U.value = LP_value_extract(txobj,0,utxotxid);
            free_json(txobj);
            //char str[65]; printf("add onevin %s/v%d %.8f\n",bits256_str(str,utxotxid),utxovout,dstr(utxos[0]->U.value));
            numutxos = 1;
        }
        else
        {
            printf("LP_createrawtransaction: onevin spent already\n");
            return(0);
        }
    }
    else
    {
        if ( (numutxos= LP_address_utxo_ptrs(coin,0,utxos,(int32_t)(sizeof(utxos)/sizeof(*utxos)),ap,coin->smartaddr)) <= 0 )
        {
            if ( bits256_nonz(utxotxid) == 0 )
            {
                printf("LP_createrawtransaction: address_utxo_ptrs %d, error\n",numutxos);
                return(0);
            }
        }
    }
    ignore_cltverr = 0;
    suppress_pubkeys = 1;
    scriptlen = bitcoin_standardspend(script,0,G.LP_myrmd160);
    numvins = LP_vins_select(ctx,coin,&total,amount,V,utxos,numutxos,suppress_pubkeys,ignore_cltverr,privkey,privkeys,vins,script,scriptlen,utxotxid,utxovout,dustcombine);
    if ( numvins <= 0 || total < amount )
    {
        printf("change %.8f = total %.8f - amount %.8f, adjust %.8f numvouts.%d, txfee %.8f\n",dstr(change),dstr(total),dstr(amount),dstr(adjust),numvouts,dstr(txfee));
        printf("not enough inputs  %.8f < for amount %.8f txfee %.8f\n",dstr(total),dstr(amount),dstr(txfee));
        return(0);
    }
    change = (total - amount);
    timestamp = (uint32_t)time(NULL);
    if ( locktime == 0 && strcmp("KMD",coin->symbol) == 0 )
        locktime = timestamp - 777;
    txobj = bitcoin_txcreate(coin->symbol,coin->isPoS,locktime,coin->txversion,timestamp);
    jdelete(txobj,"vin");
    jadd(txobj,"vin",jduplicate(vins));
    //printf("change %.8f = total %.8f - amount %.8f, adjust %.8f numvouts.%d\n",dstr(change),dstr(total),dstr(amount),dstr(adjust),numvouts);
    for (i=0; i<numvouts; i++)
    {
        item = jitem(outputs,i);
        if ( (coinaddr= jfieldname(item)) != 0 )
        {
            if ( (value= SATOSHIDEN * jdouble(item,coinaddr)) <= 0 )
            {
                printf("cant get value i.%d of %d %s\n",i,numvouts,jprint(outputs,0));
                free_json(txobj);
                return(0);
            }
            if ( (scriptstr= jstr(item,"script")) != 0 )
            {
                spendlen = (int32_t)strlen(scriptstr) >> 1;
                if ( spendlen < sizeof(script) )
                {
                    decode_hex(spendscript,spendlen,scriptstr);
                    //printf("i.%d using external script.(%s) %d\n",i,scriptstr,spendlen);
                }
                else
                {
                    printf("custom script.%d too long %d\n",i,spendlen);
                    free_json(txobj);
                    return(0);
                }
            }
            else
            {
                bitcoin_addr2rmd160(coin->symbol,coin->taddr,&addrtype,rmd160,coinaddr);
                if ( addrtype == coin->pubtype )
                    spendlen = bitcoin_standardspend(spendscript,0,rmd160);
                else spendlen = bitcoin_p2shspend(spendscript,0,rmd160);
                if ( i == numvouts-1 && strcmp(coinaddr,coin->smartaddr) == 0 && change != 0 )
                {
                    //printf("combine last vout %.8f with change %.8f\n",dstr(value+adjust),dstr(change));
                    value += change;
                    change = 0;
                }
            }
            txobj = bitcoin_txoutput(txobj,spendscript,spendlen,value + adjust);
        }
        else
        {
            printf("cant get fieldname.%d of %d %s\n",i,numvouts,jprint(outputs,0));
            free_json(txobj);
            return(0);
        }
    }
    if ( change < 6000 )
    {
        //adjust = change / numvouts; adjust messes up vout encoding!
        change = 0;
    }
    if ( change != 0 )
        txobj = bitcoin_txoutput(txobj,script,scriptlen,change);
    if ( opretstr != 0 )
    {
        spendlen = (int32_t)strlen(opretstr) >> 1;
        if ( spendlen < sizeof(script)-60 )
        {
            if ( passphrase != 0 && passphrase[0] != 0 )
            {
                decode_hex(data,spendlen,opretstr);
                offset = 2 + (spendlen >= 16);
                origspendlen = spendlen;
                crc32 = calc_crc32(0,data,spendlen);
                spendlen = LP_opreturn_encrypt(&script[offset],(int32_t)sizeof(script)-offset,data,spendlen,passphrase,crc32&0xffff);
                if ( spendlen < 0 )
                {
                    printf("error encrpting opreturn data\n");
                    free_json(txobj);
                    return(0);
                }
            } else offset = crc32 = 0;
            len = 0;
            script[len++] = SCRIPT_OP_RETURN;
            if ( spendlen < 76 )
                script[len++] = spendlen;
            else if ( spendlen <= 0xff )
            {
                script[len++] = 0x4c;
                script[len++] = spendlen;
            }
            else if ( spendlen <= 0xffff )
            {
                script[len++] = 0x4d;
                script[len++] = (spendlen & 0xff);
                script[len++] = ((spendlen >> 8) & 0xff);
            }
            if ( passphrase != 0 && passphrase[0] != 0 )
            {
                if ( offset != len )
                {
                    printf("offset.%d vs len.%d, reencrypt\n",offset,len);
                    spendlen = LP_opreturn_encrypt(&script[len],(int32_t)sizeof(script)-len,data,origspendlen,passphrase,crc32&0xffff);
                    if ( spendlen < 0 )
                    {
                        printf("error encrpting opreturn data\n");
                        free_json(txobj);
                        return(0);
                    }
                } //else printf("offset.%d already in right place\n",offset);
            } else decode_hex(&script[len],spendlen,opretstr);
            txobj = bitcoin_txoutput(txobj,script,len + spendlen,0);
            //printf("OP_RETURN.[%d, %d] script.(%s)\n",len,spendlen,opretstr);
        }
        else
        {
            printf("custom script.%d too long %d\n",i,spendlen);
            free_json(txobj);
            return(0);
        }
    }
    if ( (rawtxbytes= bitcoin_json2hex(coin->symbol,coin->isPoS,&txid,txobj,V)) != 0 )
    {
    } else printf("error making rawtx suppress.%d\n",suppress_pubkeys);
    *txobjp = txobj;
    *numvinsp = numvins;
    return(rawtxbytes);
}

char *LP_opreturndecrypt(void *ctx,char *symbol,bits256 utxotxid,char *passphrase)
{
    cJSON *txjson,*vouts,*opret,*sobj,*retjson; uint16_t utxovout; char *opretstr,*hexstr; uint8_t *opretdata,*databuf,*decoded; uint16_t ind16; uint32_t crc32; int32_t i,len,numvouts,opretlen,datalen; struct iguana_info *coin;
    if ( (coin= LP_coinfind(symbol)) == 0 )
        return(clonestr("{\"error\":\"cant find coin\"}"));
    retjson = cJSON_CreateObject();
    utxovout = 0;
    if ( (txjson= LP_gettx("LP_opreturn_decrypt",coin->symbol,utxotxid,1)) != 0 )
    {
        if ( (vouts= jarray(&numvouts,txjson,"vout")) != 0 && numvouts >= 1 )
        {
            opret = jitem(vouts,numvouts - 1);
            jaddstr(retjson,"coin",symbol);
            jaddbits256(retjson,"opreturntxid",utxotxid);
            if ( (sobj= jobj(opret,"scriptPubKey")) != 0 )
            {
                if ( (opretstr= jstr(sobj,"hex")) != 0 )
                {
                    jaddstr(retjson,"opreturn",opretstr);
                    opretlen = (int32_t)strlen(opretstr) >> 1;
                    opretdata = malloc(opretlen);
                    decode_hex(opretdata,opretlen,opretstr);
                    databuf = &opretdata[2];
                    datalen = 0;
                    if ( opretdata[0] != 0x6a )
                        jaddstr(retjson,"error","not opreturn data");
                    else if ( (datalen= opretdata[1]) < 76 )
                    {
                        if ( &databuf[datalen] != &opretdata[opretlen] )
                            databuf = 0, jaddstr(retjson,"error","mismatched short opretlen");
                    }
                    else if ( opretdata[1] == 0x4c )
                    {
                        datalen = opretdata[2];
                        databuf++;
                        if ( &databuf[datalen] != &opretdata[opretlen] )
                            databuf = 0, jaddstr(retjson,"error","mismatched opretlen");
                    }
                    else if ( opretdata[1] == 0x4d )
                    {
                        datalen = opretdata[3];
                        datalen <<= 8;
                        datalen |= opretdata[2];
                        databuf += 2;
                        if ( &databuf[datalen] != &opretdata[opretlen] )
                            databuf = 0, jaddstr(retjson,"error","mismatched big opretlen");
                    }
                    else databuf = 0, jaddstr(retjson,"error","unexpected opreturn data type");
                    if ( databuf != 0 )
                    {
                        decoded = calloc(1,opretlen+1);
                        if ( (len= LP_opreturn_decrypt(&ind16,decoded,databuf,datalen,passphrase)) < 0 )
                            jaddstr(retjson,"error","decrypt error");
                        else
                        {
                            crc32 = calc_crc32(0,decoded,len);
                            if ( (crc32 & 0xffff) == ind16 )
                            {
                                jaddstr(retjson,"result","success");
                                hexstr = malloc(len*2+1);
                                init_hexbytes_noT(hexstr,decoded,len);
                                jaddstr(retjson,"decrypted",hexstr);
                                for (i=0; i<len; i++)
                                    if ( isprint(decoded[i]) == 0 )
                                        break;
                                if ( i == len )
                                {
                                    memcpy(hexstr,decoded,len);
                                    hexstr[len] = 0;
                                    jaddstr(retjson,"original",hexstr);
                                }
                                free(hexstr);
                            } else jaddstr(retjson,"error","decrypt crc16 error");
                        }
                        free(decoded);
                    }
                    free(opretdata);
                }
            }
        }
        free_json(txjson);
    }
    return(jprint(retjson,1));
}

char *LP_createblasttransaction(uint64_t *changep,int32_t *changeoutp,cJSON **txobjp,cJSON **vinsp,struct vin_info *V,struct iguana_info *coin,bits256 utxotxid,int32_t utxovout,uint64_t utxovalue,bits256 privkey,cJSON *outputs,int64_t txfee)
{
    static void *ctx;
    cJSON *txobj,*item,*vins; uint8_t addrtype,rmd160[20],pubkey33[33],tmptype,data[8192+64],script[8192],spendscript[256]; char *coinaddr,*rawtxbytes,*scriptstr,spendscriptstr[128],blastaddr[64],wifstr[64]; bits256 txid; uint32_t locktime,crc32,timestamp; int64_t change=0,adjust=0,total,value,amount = 0; int32_t i,offset,len,scriptlen,spendlen,suppress_pubkeys,ignore_cltverr,numvouts=0;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    *txobjp = *vinsp = 0;
    *changep = 0;
    *changeoutp = -1;
    if ( coin == 0 || outputs == 0 || (numvouts= cJSON_GetArraySize(outputs)) <= 0 )
    {
        fprintf(stderr,"LP_createblasttransaction: illegal coin.%p outputs.%p or arraysize.%d, error\n",coin,outputs,numvouts);
        return(0);
    }
    amount = txfee;
    for (i=0; i<numvouts; i++)
    {
        item = jitem(outputs,i);
        if ( (coinaddr= jfieldname(item)) != 0 )
        {
            if ( 0 && LP_address_isvalid(coin->symbol,coinaddr) <= 0 )
            {
                fprintf(stderr,"%s LP_createblasttransaction %s i.%d of %d is invalid\n",coin->symbol,coinaddr,i,numvouts);
                return(0);
            }
            if ( (value= SATOSHIDEN * jdouble(item,coinaddr)) <= 0 )
            {
                fprintf(stderr,"LP_createblasttransaction: cant get value %s i.%d of %d %s\n",coinaddr,i,numvouts,jprint(outputs,0));
                return(0);
            }
            amount += value;
            //printf("vout.%d %.8f -> total %.8f\n",i,dstr(value),dstr(amount));
        }
        else
        {
            fprintf(stderr,"LP_createblasttransaction: cant get fieldname.%d of %d %s\n",i,numvouts,jprint(outputs,0));
            return(0);
        }
    }
    ignore_cltverr = 0;
    suppress_pubkeys = 1;
    memset(V,0,sizeof(*V));
    V->N = V->M = 1;
    V->signers[0].privkey = privkey;
    bitcoin_priv2wif(coin->symbol,coin->wiftaddr,wifstr,privkey,coin->wiftype);
    bitcoin_priv2pub(ctx,coin->symbol,pubkey33,blastaddr,privkey,coin->taddr,coin->pubtype);
    bitcoin_addr2rmd160("KMD",coin->taddr,&tmptype,rmd160,blastaddr);
    V->suppress_pubkeys = suppress_pubkeys;
    V->ignore_cltverr = ignore_cltverr;
    change = (utxovalue - amount);
    timestamp = (uint32_t)time(NULL);
    locktime = 0;
    txobj = bitcoin_txcreate(coin->symbol,coin->isPoS,locktime,coin->txversion,timestamp);
    scriptlen = bitcoin_standardspend(script,0,rmd160);
    init_hexbytes_noT(spendscriptstr,script,scriptlen);
    vins = cJSON_CreateArray();
    jaddi(vins,LP_inputjson(utxotxid,utxovout,spendscriptstr));
    jdelete(txobj,"vin");
    jadd(txobj,"vin",jduplicate(vins));
    *vinsp = vins;
    for (i=0; i<numvouts; i++)
    {
        item = jitem(outputs,i);
        if ( (coinaddr= jfieldname(item)) != 0 )
        {
            if ( (value= SATOSHIDEN * jdouble(item,coinaddr)) <= 0 )
            {
                fprintf(stderr,"LP_createblasttransaction: cant get value i.%d of %d %s\n",i,numvouts,jprint(outputs,0));
                free_json(txobj);
                return(0);
            }
            if ( (scriptstr= jstr(item,"script")) != 0 )
            {
                spendlen = (int32_t)strlen(scriptstr) >> 1;
                if ( spendlen < sizeof(script) )
                {
                    decode_hex(spendscript,spendlen,scriptstr);
                    //printf("i.%d using external script.(%s) %d\n",i,scriptstr,spendlen);
                }
                else
                {
                    fprintf(stderr,"LP_createblasttransaction: custom script.%d too long %d\n",i,spendlen);
                    free_json(txobj);
                    return(0);
                }
            }
            else
            {
                bitcoin_addr2rmd160(coin->symbol,coin->taddr,&addrtype,rmd160,coinaddr);
                if ( addrtype == coin->pubtype )
                    spendlen = bitcoin_standardspend(spendscript,0,rmd160);
                else spendlen = bitcoin_p2shspend(spendscript,0,rmd160);
                if ( i == numvouts-1 && strcmp(coinaddr,coin->smartaddr) == 0 && change != 0 )
                {
                    value += change;
                    change = 0;
                }
            }
            txobj = bitcoin_txoutput(txobj,spendscript,spendlen,value + adjust);
        }
        else
        {
            fprintf(stderr,"LP_createblasttransaction: cant get fieldname.%d of %d %s\n",i,numvouts,jprint(outputs,0));
            free_json(txobj);
            return(0);
        }
    }
    if ( change < 6000 || change < txfee )
        change = 0;
    *changep = change;
    if ( change != 0 )
    {
        txobj = bitcoin_txoutput(txobj,script,scriptlen,change);
        *changeoutp = numvouts;
    }
    if ( (rawtxbytes= bitcoin_json2hex(coin->symbol,coin->isPoS,&txid,txobj,V)) == 0 )
        fprintf(stderr,"LP_createblasttransaction: error making rawtx suppress.%d\n",suppress_pubkeys);
    *txobjp = txobj;
    return(rawtxbytes);
}

char *bitcoin_signrawtransaction(int32_t *completedp,bits256 *signedtxidp,struct iguana_info *coin,char *rawtx,char *wifstr)
{
    char *retstr,*paramstr,*hexstr,*signedtx = 0; int32_t len; uint8_t *data; cJSON *signedjson;
    *completedp = 0;
    memset(signedtxidp,0,sizeof(*signedtxidp));
    paramstr = calloc(1,200000+1);
    sprintf(paramstr,"[\"%s\", null, [\"%s\"]]",rawtx,wifstr);
    if ( (retstr= bitcoind_passthru(coin->symbol,coin->serverport,coin->userpass,"signrawtransaction",paramstr)) != 0 )
    {
        //printf("%s signed -> %s\n",coin->symbol,retstr);
        if ( (signedjson= cJSON_Parse(retstr)) != 0 )
        {
            if ( (hexstr= jstr(signedjson,"hex")) != 0 )
            {
                len = (int32_t)strlen(hexstr);
                signedtx = calloc(1,len+1);
                strcpy(signedtx,hexstr);
                *completedp = is_cJSON_True(jobj(signedjson,"complete"));
                len >>= 1;
                data = malloc(len);
                decode_hex(data,len,hexstr);
                *signedtxidp = bits256_calctxid(coin->symbol,data,len);
                free(data);
            }
            free_json(signedjson);
        }
        free(retstr);
    }
    free(paramstr);
    return(signedtx);
}

char *LP_txblast(struct iguana_info *coin,cJSON *argjson)
{
    static void *ctx;
    int32_t broadcast,i,num,numblast,utxovout,completed=0,numvouts,changeout; char *passphrase,changeaddr[64],vinaddr[64],wifstr[65],blastaddr[65],str[65],*signret,*signedtx=0,*rawtx=0; struct vin_info V; uint32_t locktime,starttime; uint8_t pubkey33[33]; cJSON *retjson,*item,*outputs,*vins=0,*txobj=0,*privkeys=0; struct iguana_msgtx msgtx; bits256 privkey,pubkey,checktxid,utxotxid,signedtxid; uint64_t txfee,utxovalue,change;
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    if ( (passphrase= jstr(argjson,"password")) == 0 )
        return(clonestr("{\"error\":\"need password\"}"));
    outputs = jarray(&numvouts,argjson,"outputs");
    utxotxid = jbits256(argjson,"utxotxid");
    utxovout = jint(argjson,"utxovout");
    if ( (numblast= jint(argjson,"numblast")) == 0 )
        numblast = 1000000;
    utxovalue = j64bits(argjson,"utxovalue");
    txfee = juint(argjson,"txfee");
    broadcast = juint(argjson,"broadcast");
    conv_NXTpassword(privkey.bytes,pubkey.bytes,(uint8_t *)passphrase,(int32_t)strlen(passphrase));
    privkey.bytes[0] &= 248, privkey.bytes[31] &= 127, privkey.bytes[31] |= 64;
    bitcoin_priv2wif(coin->symbol,coin->wiftaddr,wifstr,privkey,coin->wiftype);
    bitcoin_priv2pub(ctx,coin->symbol,pubkey33,blastaddr,privkey,coin->taddr,coin->pubtype);
    safecopy(vinaddr,blastaddr,sizeof(vinaddr));
    safecopy(changeaddr,blastaddr,sizeof(changeaddr));
    privkeys = cJSON_CreateArray();
    jaddistr(privkeys,wifstr);
    starttime = (uint32_t)time(NULL);
    for (i=0; i<numblast; i++)
    {
        if ( (rawtx= LP_createblasttransaction(&change,&changeout,&txobj,&vins,&V,coin,utxotxid,utxovout,utxovalue,privkey,outputs,txfee)) != 0 )
        {
            completed = 0;
            memset(&msgtx,0,sizeof(msgtx));
            memset(signedtxid.bytes,0,sizeof(signedtxid));
            if ( (signedtx= bitcoin_signrawtransaction(&completed,&signedtxid,coin,rawtx,wifstr)) == 0 )
            //if ( (completed= iguana_signrawtransaction(ctx,coin->symbol,coin->wiftaddr,coin->taddr,coin->pubtype,coin->p2shtype,coin->isPoS,coin->longestchain,&msgtx,&signedtx,&signedtxid,&V,1,rawtx,vins,privkeys,coin->zcash)) < 0 )
                printf("LP_txblast: couldnt sign blast tx %s\n",bits256_str(str,signedtxid));
            else if ( completed == 0 )
            {
                printf("LP_txblast incomplete signing blast tx (%s)\n",jprint(vins,0));
                break;
            }
            else
            {
                if ( broadcast != 0 )
                {
                    if ( (signret= LP_sendrawtransaction(coin->symbol,signedtx,0)) != 0 )
                    {
                        printf("LP_txblast.%s broadcast (%s) vs %s\n",coin->symbol,bits256_str(str,signedtxid),signret);
                        if ( is_hexstr(signret,0) == 64 )
                        {
                            decode_hex(checktxid.bytes,32,signret);
                            if ( bits256_cmp(checktxid,signedtxid) == 0 )
                            {
                                printf("blaster i.%d of %d: %s/v%d %.8f\n",i,numblast,bits256_str(str,signedtxid),changeout,dstr(change));
                            } else break;
                        }
                        else
                        {
                            printf("error sending tx:\n \"%s\" null '[\"%s\"]'\n",rawtx,wifstr);
                            break;
                        }
                        free(signret);
                    }
                    else
                    {
                        fprintf(stderr,"null return from LP_sendrawtransaction\n");
                        break;
                    }
                } else printf("blaster i.%d of %d: %s/v%d %.8f %s\n",i,numblast,bits256_str(str,signedtxid),changeout,dstr(change),signedtx);
            }
        }
        else
        {
            fprintf(stderr,"error creating txblast rawtransaction\n");
            break;
        }
        if ( txobj != 0 )
            free_json(txobj), txobj = 0;
        if ( rawtx != 0 )
            free(rawtx), rawtx = 0;
        if ( signedtx != 0 )
            free(signedtx), signedtx = 0;
        if ( changeout < 0 || change == 0 )
            break;
        utxotxid = signedtxid;
        utxovout = changeout;
        utxovalue = change;
        // good place to update outputs[] for a fully programmable blast
    }
    free_json(privkeys), privkeys = 0;
    retjson = cJSON_CreateObject();
    jaddstr(retjson,"result","success");
    jaddstr(retjson,"blastaddr",blastaddr);
    jaddnum(retjson,"broadcast",broadcast);
    jaddnum(retjson,"numblast",numblast);
    jaddnum(retjson,"completed",i);
    jaddbits256(retjson,"lastutxo",utxotxid);
    jaddnum(retjson,"lastutxovout",utxovout);
    jaddnum(retjson,"lastutxovalue",dstr(utxovalue));
    jaddnum(retjson,"elapsed",(uint32_t)time(NULL) - starttime);
    jaddnum(retjson,"tx/sec",(double)i / ((uint32_t)time(NULL) - starttime + 1));
    return(jprint(retjson,1));
}

char *LP_withdraw(struct iguana_info *coin,cJSON *argjson)
{
    static void *ctx;
    int32_t broadcast,allocated_outputs=0,iter,i,num,utxovout,autofee,completed=0,maxV,numvins,numvouts,datalen,suppress_pubkeys; bits256 privkey; struct LP_address *ap; char changeaddr[64],vinaddr[64],str[65],wifstr[64],*signret,*signedtx=0,*rawtx=0; struct vin_info *V; uint32_t locktime; cJSON *retjson,*item,*outputs,*vins=0,*txobj=0,*privkeys=0; struct iguana_msgtx msgtx; bits256 utxotxid,signedtxid; uint64_t txfee=0,newtxfee=10000;
//printf("withdraw.%s %s\n",coin->symbol,jprint(argjson,0));
    if ( coin->etomic[0] != 0 )
    {
        if ( (coin= LP_coinfind("ETOMIC")) == 0 )
            return(clonestr("{\"error\":\"use LP_eth_withdraw for ETH or ERC20\"}"));
    }
    if ( ctx == 0 )
        ctx = bitcoin_ctx();
    broadcast = jint(argjson,"broadcast");
    if ( (outputs= jarray(&numvouts,argjson,"outputs")) == 0 )
    {
        if ( jstr(argjson,"opreturn") == 0 )
        {
            printf("no outputs in argjson (%s)\n",jprint(argjson,0));
            return(clonestr("{\"error\":\"no outputs specified\"}"));
        }
        else
        {
            outputs = cJSON_CreateArray();
            item = cJSON_CreateObject();
            jaddnum(item,coin->smartaddr,0.0001);
            jaddi(outputs,item);
            numvouts = 1;
            allocated_outputs = 1;
        }
    }
    utxotxid = jbits256(argjson,"utxotxid");
    utxovout = jint(argjson,"utxovout");
    locktime = juint(argjson,"locktime");
    txfee = juint(argjson,"txfee");
    autofee = (strcmp(coin->symbol,"BTC") == 0);
    //printf("LP_withdraw: %s/v%d %s\n",bits256_str(str,utxotxid),utxovout,jprint(outputs,0));
    if ( txfee == 0 )
    {
        autofee = 1;
        txfee = coin->txfee;
        if ( txfee > 0 && txfee < LP_MIN_TXFEE )
            txfee = LP_MIN_TXFEE;
    } else autofee = 0;
    suppress_pubkeys = 0;
    memset(signedtxid.bytes,0,sizeof(signedtxid));
    safecopy(changeaddr,coin->smartaddr,sizeof(changeaddr));
    safecopy(vinaddr,coin->smartaddr,sizeof(vinaddr));
    privkey = LP_privkey(coin->symbol,vinaddr,coin->taddr);
    maxV = LP_MAXVINS;
    V = malloc(maxV * sizeof(*V));
    for (iter=0; iter<2; iter++)
    {
        privkeys = cJSON_CreateArray();
        vins = cJSON_CreateArray();
        memset(V,0,sizeof(*V) * maxV);
        numvins = 0;
        if ( (rawtx= LP_createrawtransaction(&txobj,&numvins,coin,V,maxV,privkey,outputs,vins,privkeys,iter == 0 ? txfee : newtxfee,utxotxid,utxovout,jint(argjson,"onevin"),locktime,jstr(argjson,"opreturn"),jstr(argjson,"passphrase"))) != 0 )
        {
            completed = 0;
            memset(&msgtx,0,sizeof(msgtx));
            memset(signedtxid.bytes,0,sizeof(signedtxid));
            if ( jint(argjson,"onevin") != 0 )
            {
                V[0].suppress_pubkeys = 1;
                bitcoin_priv2wif(coin->symbol,coin->wiftaddr,wifstr,privkey,coin->wiftype);
                if ( (signedtx= bitcoin_signrawtransaction(&completed,&signedtxid,coin,rawtx,wifstr)) != 0 && completed == 0 )
                {
                    printf("incomplete signing\n");
                    free(signedtx);
                    signedtx = 0;
                }
            }
            else
            {
                //printf("created V[0].suppress %d\n",V[0].suppress_pubkeys);
                if ( (completed= iguana_signrawtransaction(ctx,coin->symbol,coin->wiftaddr,coin->taddr,coin->pubtype,coin->p2shtype,coin->isPoS,coin->longestchain,&msgtx,&signedtx,&signedtxid,V,numvins,rawtx,vins,privkeys,coin->zcash)) < 0 )
                    printf("couldnt sign withdraw %s\n",bits256_str(str,signedtxid));
                else if ( completed == 0 )
                {
                    printf("incomplete signing withdraw (%s)\n",jprint(vins,0));
                    if ( signedtx != 0 )
                        free(signedtx), signedtx = 0;
                }
            }
            if ( signedtx == 0 )
                break;
            datalen = (int32_t)strlen(signedtx) / 2;
            if ( autofee != 0 && iter == 0 && strcmp(coin->symbol,"BTC") == 0 )
            {
                txfee = newtxfee = LP_txfeecalc(coin,0,datalen);
                printf("txfee %.8f -> newtxfee %.8f, numvins.%d datalen.%d\n",dstr(txfee),dstr(newtxfee),numvins,datalen);
                for (i=0; i<numvins; i++)
                {
                    item = jitem(vins,i);
//printf("set available %s\n",jprint(item,0));
                    LP_availableset(jbits256(item,"txid"),jint(item,"vout"));
                }
                free_json(vins), vins = 0;
                free_json(txobj), txobj = 0;
                free_json(privkeys), privkeys = 0;
                if ( rawtx != 0 )
                    free(rawtx), rawtx = 0;
                if ( signedtx != 0 )
                    free(signedtx), signedtx = 0;
            } else break;
        } else break;
    }
    free(V);
    if ( vins != 0 )
    {
        if ( completed == 0 && (numvins= cJSON_GetArraySize(vins)) > 0 )
        {
            for (i=0; i<numvins; i++)
            {
                item = jitem(vins,i);
                LP_availableset(jbits256(item,"txid"),jint(item,"vout"));
            }
        }
        free_json(vins);
    }
    if ( privkeys != 0 )
        free_json(privkeys);
    retjson = cJSON_CreateObject();
    if ( rawtx != 0 )
    {
        jaddstr(retjson,"rawtx",rawtx);
        free(rawtx);
    }
    if ( signedtx != 0 )
    {
        jaddstr(retjson,"hex",signedtx);
        if ( broadcast != 0 )
        {
            if ( (signret= LP_sendrawtransaction(coin->symbol,signedtx,0)) != 0 )
            {
                printf("LP_withdraw.%s %s -> %s (%s)\n",coin->symbol,jprint(argjson,0),bits256_str(str,signedtxid),signret);
                free(signret);
            }
        }
        free(signedtx);
    }
    if ( txobj != 0 )
        jadd(retjson,"tx",txobj);
    jaddbits256(retjson,"txid",signedtxid);
    jaddnum(retjson,"txfee",txfee);
    jadd(retjson,"complete",completed!=0?jtrue():jfalse());
    if ( allocated_outputs != 0 )
        free_json(outputs);
    return(jprint(retjson,1));
}

char *LP_autofillbob(struct iguana_info *coin,uint64_t satoshis)
{
    char *retstr; cJSON *argjson,*withdrawjson,*outputs,*item; int64_t total,balance,txfee;
    if ( coin->etomic[0] == 0 )
    {
        if ( coin->electrum != 0 )
            balance = LP_unspents_load(coin->symbol,coin->smartaddr);
        else balance = LP_RTsmartbalance(coin);
        if ( strcmp("BTC",coin->symbol) == 0 )
            txfee = LP_txfeecalc(coin,0,1000);
        balance -= (txfee + 1000000);
        if ( balance < (satoshis<<2) )
            return(clonestr("{\"error\":\"couldnt autofill balance too small\"}"));
        if ( balance > satoshis+3*txfee && balance >= (txfee + 1000000) )
        {
            argjson = cJSON_CreateObject();
            outputs = cJSON_CreateArray();
            item = cJSON_CreateObject();
            jaddnum(item,coin->smartaddr,dstr(satoshis + 3000000));
            jaddi(outputs,item);
            item = cJSON_CreateObject();
            jaddnum(item,coin->smartaddr,dstr(LP_DEPOSITSATOSHIS(satoshis) + 3000000));
            jaddi(outputs,item);
            item = cJSON_CreateObject();
            jaddnum(item,coin->smartaddr,0.0001);
            jaddi(outputs,item);
            jadd(argjson,"outputs",outputs);
            jaddnum(argjson,"broadcast",0);
            jaddstr(argjson,"coin",coin->symbol);
            retstr = LP_withdraw(coin,argjson);
            free_json(argjson);
            return(retstr);
        } else return(clonestr("{\"error\":\"balance too small to autosplit, please make more deposits\"}"));
    }
    return(clonestr("{\"error\":\"couldnt autofill etomic needs separate support\"}"));
}

char *LP_movecoinbases(char *symbol)
{
    static bits256 zero; bits256 utxotxid,txid; struct iguana_info *coin; cJSON *retjson,*outputs,*argjson,*txids,*unspents,*item,*gen,*output; int32_t i,n,utxovout; char *retstr,*hexstr;
    if ( (coin= LP_coinfind(symbol)) != 0 )
    {
        if ( coin->electrum == 0 )
        {
            txids = cJSON_CreateArray();
            if ( (unspents= LP_listunspent(symbol,coin->smartaddr,zero,zero)) != 0 )
            {
                if ( (n= cJSON_GetArraySize(unspents)) > 0 )
                {
                    for (i=0; i<n; i++)
                    {
                        item = jitem(unspents,i);
                        if ( is_cJSON_True(jobj(item,"generated")) != 0 )
                        {
                            utxotxid = jbits256(item,"txid");
                            utxovout = jint(item,"vout");
                            argjson = cJSON_CreateObject();
                            outputs = cJSON_CreateArray();
                            output = cJSON_CreateObject();
                            jaddnum(output,coin->smartaddr,0.0001);
                            jaddi(outputs,output);
                            jadd(argjson,"outputs",outputs);
                            jaddnum(argjson,"broadcast",1);
                            jaddstr(argjson,"coin",coin->symbol);
                            jaddbits256(argjson,"utxotxid",utxotxid);
                            jaddnum(argjson,"utxovout",utxovout);
                            jaddnum(argjson,"onevin",1);
                            if ( (retstr= LP_withdraw(coin,argjson)) != 0 )
                            {
                                if ( (retjson= cJSON_Parse(retstr)) != 0 )
                                {
                                    txid = jbits256(retjson,"txid");
                                    hexstr = jstr(retjson,"hex");
                                    if ( bits256_nonz(txid) != 0 && hexstr != 0 )
                                    {
                                        printf("%s -> %s\n",jprint(item,0),hexstr);
                                        jaddibits256(txids,txid);
                                    }
                                    free_json(retjson);
                                }
                                free(retstr);
                            }
                            free_json(argjson);
                        }
                    }
                }
                free_json(unspents);
            }
            return(jprint(txids,1));
        }
        return(clonestr("{\"error\":\"LP_movecoinbases cant be electrum\"}"));
    }
    return(clonestr("{\"error\":\"LP_movecoinbases couldnt find coin\"}"));
}

int32_t basilisk_rawtx_gen(void *ctx,char *str,uint32_t swapstarted,uint8_t *pubkey33,int32_t iambob,int32_t lockinputs,struct basilisk_rawtx *rawtx,uint32_t locktime,uint8_t *script,int32_t scriptlen,int64_t txfee,int32_t minconf,int32_t delay,bits256 privkey,uint8_t *changermd160,char *vinaddr)
{
    struct iguana_info *coin; int32_t len,retval=-1; char *retstr,*hexstr; cJSON *argjson,*outputs,*item,*retjson,*obj;
    if ( (coin= LP_coinfind(rawtx->symbol)) == 0 )
        return(-1);
    if ( coin->etomic[0] != 0 )
    {
        if ( (coin= LP_coinfind("ETOMIC")) == 0 )
            return(-1);
    }
    if ( strcmp(coin->smartaddr,vinaddr) != 0 )
    {
        printf("???????????????????????? basilisk_rawtx_gen mismatched %s %s vinaddr.%s != (%s)\n",rawtx->symbol,coin->symbol,vinaddr,coin->smartaddr);
        return(-1);
    }
    argjson = cJSON_CreateObject();
    jaddbits256(argjson,"utxotxid",rawtx->utxotxid);
    jaddnum(argjson,"utxovout",rawtx->utxovout);
    jaddnum(argjson,"locktime",locktime);
    jadd64bits(argjson,"txfee",txfee);
    outputs = cJSON_CreateArray();
    item = cJSON_CreateObject();
    jaddnum(item,rawtx->I.destaddr,dstr(rawtx->I.amount));
    jaddi(outputs,item);
    jadd(argjson,"outputs",outputs);
    //printf("call LP_withdraw.(%s)\n",jprint(argjson,0));
    if ( (retstr= LP_withdraw(coin,argjson)) != 0 )
    {
        if ( (retjson= cJSON_Parse(retstr)) != 0 )
        {
            if ( (obj= jobj(retjson,"complete")) != 0 && is_cJSON_True(obj) != 0 && (hexstr= jstr(retjson,"hex")) != 0 && (len= is_hexstr(hexstr,0)) > 16 )
            {
                rawtx->I.datalen = len >> 1;
                decode_hex(rawtx->txbytes,rawtx->I.datalen,hexstr);
                rawtx->I.completed = 1;
                rawtx->I.signedtxid = jbits256(retjson,"txid");
                retval = 0;
            } else printf("rawtx withdraw error? (%s)\n",jprint(argjson,0));
            free_json(retjson);
        }
        free(retstr);
    }
    free_json(argjson);
    return(retval);
}

int32_t basilisk_alicescript(char *symbol,uint8_t *redeemscript,int32_t *redeemlenp,uint8_t *script,int32_t n,char *msigaddr,uint8_t taddr,uint8_t altps2h,uint8_t *pubAm,uint8_t *pubBn)
{
    uint8_t p2sh160[20]; struct vin_info V;
    memset(&V,0,sizeof(V));
    memcpy(&V.signers[0].pubkey[0],pubAm,33);
    memcpy(&V.signers[1].pubkey[0],pubBn,33);
    V.M = V.N = 2;
    *redeemlenp = bitcoin_MofNspendscript(p2sh160,redeemscript,n,&V);
    bitcoin_address(symbol,msigaddr,taddr,altps2h,p2sh160,sizeof(p2sh160));
    n = bitcoin_p2shspend(script,0,p2sh160);
    //for (i=0; i<*redeemlenp; i++)
    //    printf("%02x",redeemscript[i]);
    //printf(" <- redeemscript alicetx\n");
    return(n);
}

char *basilisk_swap_Aspend(char *name,char *symbol,uint64_t Atxfee,uint8_t wiftaddr,uint8_t taddr,uint8_t pubtype,uint8_t p2shtype,uint8_t isPoS,uint8_t wiftype,void *ctx,bits256 privAm,bits256 privBn,bits256 utxotxid,int32_t utxovout,uint8_t pubkey33[33],uint32_t expiration,int64_t *destamountp,char *vinaddr,int32_t zcash)
{
    char msigaddr[64],*signedtx = 0; int32_t spendlen,redeemlen; uint8_t tmp33[33],redeemscript[512],spendscript[128]; uint8_t pubAm[33],pubBn[33]; bits256 signedtxid; uint64_t txfee;
    if ( bits256_nonz(privAm) != 0 && bits256_nonz(privBn) != 0 )
    {
        bitcoin_pubkey33(ctx,pubAm,privAm);
        bitcoin_pubkey33(ctx,pubBn,privBn);
        //char str[65];
        //printf("pubAm.(%s)\n",bits256_str(str,pubAm));
        //printf("pubBn.(%s)\n",bits256_str(str,pubBn));
        spendlen = basilisk_alicescript(symbol,redeemscript,&redeemlen,spendscript,0,msigaddr,taddr,p2shtype,pubAm,pubBn);
        if ( (txfee= Atxfee) == 0 )
        {
            if ( (txfee= LP_getestimatedrate(LP_coinfind(symbol)) * LP_AVETXSIZE) < LP_MIN_TXFEE )
                txfee = LP_MIN_TXFEE;
        }
        signedtx = basilisk_swap_bobtxspend(&signedtxid,txfee,name,symbol,wiftaddr,taddr,pubtype,p2shtype,isPoS,wiftype,ctx,privAm,&privBn,redeemscript,redeemlen,0,0,utxotxid,utxovout,0,pubkey33,1,expiration,destamountp,0,0,vinaddr,1,zcash);
        LP_mark_spent(symbol,utxotxid,utxovout);
    }
    return(signedtx);
}

int32_t LP_swap_getcoinaddr(char *symbol,char *coinaddr,bits256 txid,int32_t vout)
{
    cJSON *retjson;
    coinaddr[0] = 0;
    if ( (retjson= LP_gettx("LP_swap_getcoinaddr",symbol,txid,1)) != 0 )
    {
        LP_txdestaddr(coinaddr,txid,vout,retjson);
        free_json(retjson);
    }
    return(coinaddr[0] != 0);
}

int32_t basilisk_swap_getsigscript(char *symbol,uint8_t *script,int32_t maxlen,bits256 txid,int32_t vini)
{
    cJSON *retjson,*vins,*item,*skey; int32_t n,scriptlen = 0; char *hexstr;
    //char str[65]; printf("getsigscript %s %s/v%d\n",symbol,bits256_str(str,txid),vini);
    if ( bits256_nonz(txid) != 0 && (retjson= LP_gettx("basilisk_swap_getsigscript",symbol,txid,0)) != 0 )
    {
        //printf("gettx.(%s)\n",jprint(retjson,0));
        if ( (vins= jarray(&n,retjson,"vin")) != 0 && vini < n )
        {
            item = jitem(vins,vini);
            if ( (skey= jobj(item,"scriptSig")) != 0 && (hexstr= jstr(skey,"hex")) != 0 && (scriptlen= (int32_t)strlen(hexstr)) < maxlen*2 )
            {
                scriptlen >>= 1;
                decode_hex(script,scriptlen,hexstr);
                //char str[65]; printf("%s %s/v%d sigscript.(%s)\n",symbol,bits256_str(str,txid),vini,hexstr);
            }
        }
        free_json(retjson);
    }
    return(scriptlen);
}

#ifdef notnow
bits256 _LP_swap_spendtxid(char *symbol,char *destaddr,char *coinaddr,bits256 utxotxid,int32_t vout)
{
    char *retstr,*addr; cJSON *array,*item,*array2; int32_t i,n,m; bits256 spendtxid,txid;
    memset(&spendtxid,0,sizeof(spendtxid));
    if ( (retstr= blocktrail_listtransactions(symbol,coinaddr,100,0)) != 0 )
    {
        if ( (array= cJSON_Parse(retstr)) != 0 )
        {
            if ( (n= cJSON_GetArraySize(array)) > 0 )
            {
                for (i=0; i<n; i++)
                {
                    if ( (item= jitem(array,i)) == 0 )
                        continue;
                    txid = jbits256(item,"txid");
                    if ( bits256_nonz(txid) == 0 )
                    {
                        if ( (array2= jarray(&m,item,"inputs")) != 0 && m == 1 )
                        {
                            //printf("found inputs with %s\n",bits256_str(str,spendtxid));
                            txid = jbits256(jitem(array2,0),"output_hash");
                            if ( bits256_cmp(txid,utxotxid) == 0 )
                            {
                                //printf("matched %s\n",bits256_str(str,txid));
                                if ( (array2= jarray(&m,item,"outputs")) != 0 && m == 1 && (addr= jstr(jitem(array2,0),"address")) != 0 )
                                {
                                    spendtxid = jbits256(item,"hash");
                                    strcpy(destaddr,addr);
                                    //printf("set spend addr.(%s) <- %s\n",addr,jprint(item,0));
                                    break;
                                }
                            }
                        }
                    }
                    else if ( bits256_cmp(txid,utxotxid) == 0 )
                    {
                        spendtxid = jbits256(item,"spendtxid");
                        if ( bits256_nonz(spendtxid) != 0 )
                        {
                            LP_swap_getcoinaddr(symbol,destaddr,spendtxid,0);
                            //char str[65]; printf("found spendtxid.(%s) -> %s\n",bits256_str(str,spendtxid),destaddr);
                            break;
                        }
                    }
                }
            }
            free_json(array);
        }
        free(retstr);
    }
    return(spendtxid);
}
#endif

bits256 LP_swap_spendtxid(char *symbol,char *destaddr,bits256 utxotxid,int32_t utxovout)
{
    bits256 spendtxid,txid,vintxid; int32_t spendvin,i,j,m,n; char coinaddr[64]; cJSON *array,*vins,*vin,*txobj; struct iguana_info *coin;
    // listtransactions or listspents
    coinaddr[0] = 0;
    memset(&spendtxid,0,sizeof(spendtxid));
    if ( LP_spendsearch(destaddr,&spendtxid,&spendvin,symbol,utxotxid,utxovout) > 0 )
    {
        //char str[65]; printf("%s dest.%s spend of %s/v%d detected\n",symbol,destaddr,bits256_str(str,utxotxid),utxovout);
    }
    else if ( (coin= LP_coinfind(symbol)) != 0 && coin->electrum == 0 )
    {
        //printf("get received by %s\n",destaddr);
        if ( (array= LP_listreceivedbyaddress(symbol,destaddr)) != 0 )
        {
            if ( (n= cJSON_GetArraySize(array)) > 0 )
            {
                for (i=0; i<n; i++)
                {
                    txid = jbits256i(array,i);
                    if ( (txobj= LP_gettx("LP_swap_spendtxid",symbol,txid,1)) != 0 )
                    {
                        if ( (vins= jarray(&m,txobj,"vin")) != 0 )
                        {
//printf("vins.(%s)\n",jprint(vins,0));
                            for (j=0; j<m; j++)
                            {
                                vin = jitem(vins,j);
                                vintxid = jbits256(vin,"txid");
                                if ( utxovout == jint(vin,"vout") && bits256_cmp(vintxid,utxotxid) == 0 )
                                {
                                    LP_txdestaddr(destaddr,txid,0,txobj);
                                    //char str[65],str2[65],str3[65]; printf("LP_swap_spendtxid: found %s/v%d spends %s vs %s/v%d found.%d destaddr.(%s)\n",bits256_str(str,txid),j,bits256_str(str2,vintxid),bits256_str(str3,utxotxid),utxovout,bits256_cmp(vintxid,utxotxid) == 0,destaddr);
                                    spendtxid = txid;
                                    break;
                                }
                            }
                        }
                        free_json(txobj);
                    }
                }
            }
            free_json(array);
        }
    }
    /*   if ( (retjson= LP_gettxout(symbol,coinaddr,utxotxid,vout)) == 0 )
     {
     decode_hex(spendtxid.bytes,32,"deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
     printf("couldnt find spend of %s/v%d, but no gettxout\n",bits256_str(str,utxotxid),vout);
     } else free_json(retjson);
     */
    return(spendtxid);
    //char str[65]; printf("swap %s spendtxid.(%s)\n",symbol,bits256_str(str,utxotxid));
}

int32_t basilisk_swap_bobredeemscript(int32_t depositflag,int32_t *secretstartp,uint8_t *redeemscript,uint32_t locktime,uint8_t *pubA0,uint8_t *pubB0,uint8_t *pubB1,bits256 privAm,bits256 privBn,uint8_t *secretAm,uint8_t *secretAm256,uint8_t *secretBn,uint8_t *secretBn256)
{
    int32_t i,n=0; uint8_t *cltvpub,*destpub; bits256 privkey; uint8_t pubkeyA[33],pubkeyB[33],secret160[20],secret256[32];
    if ( depositflag != 0 )
    {
        cltvpub = pubA0;
        destpub = pubB0;
        privkey = privBn;
        memcpy(secret160,secretBn,20);
        memcpy(secret256,secretBn256,32);
    }
    else
    {
        cltvpub = pubB1;
        destpub = pubA0;
        privkey = privAm;
        memcpy(secret160,secretAm,20);
        memcpy(secret256,secretAm256,32);
    }
    for (i=0; i<20; i++)
        if ( secret160[i] != 0 )
            break;
    if ( i == 20 )
        return(-1);
    memcpy(pubkeyA,cltvpub,33);
    memcpy(pubkeyB,destpub,33);
    redeemscript[n++] = SCRIPT_OP_IF;
    n = bitcoin_checklocktimeverify(redeemscript,n,locktime);
    if ( depositflag != 0 )
    {
        //for (i=0; i<20; i++)
        //    printf("%02x",secretAm[i]);
        //printf(" <- secretAm depositflag.%d nonz.%d\n",depositflag,bits256_nonz(privkey));
        n = bitcoin_secret160verify(redeemscript,n,secretAm);
    }
    n = bitcoin_pubkeyspend(redeemscript,n,pubkeyA);
    redeemscript[n++] = SCRIPT_OP_ELSE;
    if ( secretstartp != 0 )
        *secretstartp = n + 2;
    if ( bits256_nonz(privkey) != 0 )
    {
        uint8_t bufA[20],bufB[20];
        revcalc_rmd160_sha256(bufA,privkey);
        calc_rmd160_sha256(bufB,privkey.bytes,sizeof(privkey));
        /*if ( memcmp(bufA,secret160,sizeof(bufA)) == 0 )
         printf("MATCHES BUFA\n");
         else if ( memcmp(bufB,secret160,sizeof(bufB)) == 0 )
         printf("MATCHES BUFB\n");
         else printf("secret160 matches neither\n");
         for (i=0; i<20; i++)
         printf("%02x",bufA[i]);
         printf(" <- revcalc\n");
         for (i=0; i<20; i++)
         printf("%02x",bufB[i]);
         printf(" <- calc\n");*/
        memcpy(secret160,bufB,20);
    }
    n = bitcoin_secret160verify(redeemscript,n,secret160);
    n = bitcoin_pubkeyspend(redeemscript,n,pubkeyB);
    redeemscript[n++] = SCRIPT_OP_ENDIF;
    return(n);
}

int32_t basilisk_swapuserdata(uint8_t *userdata,bits256 privkey,int32_t ifpath,bits256 signpriv,uint8_t *redeemscript,int32_t redeemlen)
{
    int32_t i,len = 0;
    if ( bits256_nonz(privkey) != 0 )
    {
        userdata[len++] = sizeof(privkey);
        for (i=0; i<sizeof(privkey); i++)
            userdata[len++] = privkey.bytes[i];
    }
    userdata[len++] = 0x51 * ifpath; // ifpath == 1 -> if path, 0 -> else path
    return(len);
}
