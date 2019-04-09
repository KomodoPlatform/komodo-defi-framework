
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
