
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
//  LP_cache.c
//  marketmaker
//

cJSON *LP_transaction_fromdata(struct iguana_info *coin,bits256 txid,uint8_t *serialized,int32_t len)
{
    uint8_t *extraspace; cJSON *txobj; char str[65],str2[65]; struct iguana_msgtx msgtx; bits256 checktxid;
    extraspace = calloc(1,4000000);
    memset(&msgtx,0,sizeof(msgtx));
    txobj = bitcoin_data2json(coin->symbol,coin->taddr,coin->pubtype,coin->p2shtype,coin->isPoS,coin->height,&checktxid,&msgtx,extraspace,4000000,serialized,len,0,0,coin->zcash);
    //printf("TX.(%s) match.%d\n",jprint(txobj,0),bits256_cmp(txid,checktxid));
    free(extraspace);
    if ( bits256_cmp(txid,checktxid) != 0 )
    {
        printf("%s LP_transaction_fromdata mismatched txid %s vs %s\n",coin->symbol,bits256_str(str,txid),bits256_str(str2,checktxid));
        free_json(txobj);
        txobj = 0;
    }
    return(txobj);
}

struct LP_transaction *LP_create_transaction(struct iguana_info *coin,bits256 txid,uint8_t *serialized,int32_t len,int32_t height,long fpos)
{
    cJSON *txobj; bits256 spenttxid; int32_t i,spentvout,numvins,numvouts; cJSON *vout,*vin,*vins,*vouts; struct LP_transaction *tx; char str[65];
    if ( (tx= LP_transactionfind(coin,txid)) != 0 )
        return(tx);
    if ( (txobj= LP_transaction_fromdata(coin,txid,serialized,len)) != 0 )
    {
        vins = jarray(&numvins,txobj,"vin");
        vouts = jarray(&numvouts,txobj,"vout");
        tx = LP_transactionadd(coin,txid,height,numvouts,numvins);
        tx->serialized = serialized, tx->len = len;
        // free(serialized), tx->len = 0;
        tx->fpos = fpos;
        tx->SPV = tx->height = height;
        //printf("tx.%s numvins.%d numvouts.%d\n",bits256_str(str,txid),numvins,numvouts);
        for (i=0; i<numvouts; i++)
        {
            vout = jitem(vouts,i);
            tx->outpoints[i].value = LP_value_extract(vout,0,txid);
            tx->outpoints[i].interest = SATOSHIDEN * jdouble(vout,"interest");
            LP_destaddr(tx->outpoints[i].coinaddr,vout);
            //printf("from transaction init %s %s %s/v%d <- %.8f\n",coin->symbol,tx->outpoints[i].coinaddr,bits256_str(str,txid),i,dstr(tx->outpoints[i].value));
            LP_address_utxoadd(0,(uint32_t)time(NULL),"LP_create_transaction",coin,tx->outpoints[i].coinaddr,txid,i,tx->outpoints[i].value,height,-1);
        }
        for (i=0; i<numvins; i++)
        {
            vin = jitem(vins,i);
            spenttxid = jbits256(vin,"txid");
            spentvout = jint(vin,"vout");
            if ( i == 0 && bits256_nonz(spenttxid) == 0 )
                continue;
            if ( (tx= LP_transactionfind(coin,spenttxid)) != 0 )
            {
                if ( spentvout < tx->numvouts )
                {
                    if ( tx->outpoints[spentvout].spendheight <= 0 )
                    {
                        tx->outpoints[spentvout].spendtxid = txid;
                        tx->outpoints[spentvout].spendvini = i;
                        tx->outpoints[spentvout].spendheight = height > 0 ? height : 1;
                        LP_address_utxoadd(0,(uint32_t)time(NULL),"LP_transactioninit iter1",coin,tx->outpoints[spentvout].coinaddr,spenttxid,spentvout,tx->outpoints[spentvout].value,-1,height>0?height:1);
                        if ( 0 && strcmp(coin->symbol,"REVS") == 0 )
                            printf("spend %s %s/v%d at ht.%d\n",coin->symbol,bits256_str(str,tx->txid),spentvout,height);
                    }
                } else printf("LP_transactioninit: %s spentvout.%d < numvouts.%d spendheight.%d\n",bits256_str(str,spenttxid),spentvout,tx->numvouts,tx->outpoints[spentvout].spendheight);
            } //else printf("LP_transactioninit: couldnt find (%s) ht.%d %s\n",bits256_str(str,spenttxid),height,jprint(vin,0));
            if ( bits256_cmp(spenttxid,txid) == 0 )
                printf("spending same tx's %p vout ht.%d %s.[%d] s%d\n",tx,height,bits256_str(str,txid),tx!=0?tx->numvouts:0,spentvout);
        }
        free_json(txobj);
    }
    return(tx);
}

void LP_SPV_store(struct iguana_info *coin,bits256 txid,int32_t height)
{
    FILE *fp; char fname[512]; struct LP_transaction *tx = 0;
    if ( (tx= LP_transactionfind(coin,txid)) != 0 && tx->serialized != 0 && tx->len > 0 && tx->fpos == 0 )
    {
        sprintf(fname,"%s/UNSPENTS/%s.SPV",GLOBAL_DBDIR,coin->symbol), OS_portable_path(fname);
        if ( (fp= OS_appendfile(fname)) != 0 )
        {
            fwrite(&tx->txid,1,sizeof(tx->txid),fp);
            fwrite(&tx->len,1,sizeof(tx->len),fp);
            fwrite(&tx->height,1,sizeof(tx->height),fp);
            tx->fpos = ftell(fp);
            fwrite(tx->serialized,1,tx->len,fp);
            fclose(fp);
        }
    } //else printf("cant store %s %s tx.%p [%d] fpos.%ld SPV.%d\n",coin->symbol,bits256_str(str,txid),tx,tx!=0?tx->len:-1,tx!=0?tx->fpos:-1,tx!=0?tx->SPV:-1);
}

int32_t LP_cacheitem(struct iguana_info *coin,FILE *fp)
{
    bits256 txid,hash; long fpos; int32_t offset,retval,height,len; uint8_t *serialized; char str[65],str2[65];
    fpos = ftell(fp);
    if ( fread(&txid,1,sizeof(txid),fp) == sizeof(txid) && fread(&len,1,sizeof(len),fp) == sizeof(len) && fread(&height,1,sizeof(height),fp) == sizeof(height) && len < 100000 )
    {
        offset = (int32_t)(sizeof(txid) + sizeof(len) + sizeof(height));
        serialized = malloc(len);
        if ( (retval= (int32_t)fread(serialized,1,len,fp)) == len )
        {
            hash = bits256_calctxid(coin->symbol,serialized,len);
            if ( bits256_cmp(hash,txid) == 0 )
            {
                //printf("%s validated in cache\n",bits256_str(str,hash));
                LP_create_transaction(coin,txid,serialized,len,height,fpos+offset);
                return((int32_t)(ftell(fp) - fpos));
            }
            printf("%s vs %s did not validated in cache\n",bits256_str(str,hash),bits256_str(str2,txid));
        } else printf("retval.%d vs len.%d\n",retval,len);
    } else printf("fread error\n");
    return(-1);
}

bits256 validate_merkle(int32_t pos,bits256 txid,cJSON *proofarray,int32_t proofsize)
{
    int32_t i; uint8_t serialized[sizeof(bits256) * 2]; bits256 hash,proof;
    hash = txid;
    for (i=0; i<proofsize; i++)
    {
        proof = jbits256i(proofarray,i);
        if ( (pos & 1) == 0 )
        {
            iguana_rwbignum(1,&serialized[0],sizeof(hash),hash.bytes);
            iguana_rwbignum(1,&serialized[sizeof(hash)],sizeof(proof),proof.bytes);
        }
        else
        {
            iguana_rwbignum(1,&serialized[0],sizeof(proof),proof.bytes);
            iguana_rwbignum(1,&serialized[sizeof(hash)],sizeof(hash),hash.bytes);
        }
        hash = bits256_doublesha256(0,serialized,sizeof(serialized));
        pos >>= 1;
    }
    return(hash);
}

bits256 LP_merkleroot(struct iguana_info *coin,struct electrum_info *ep,int32_t height)
{
    cJSON *hdrobj; bits256 merkleroot;
    memset(merkleroot.bytes,0,sizeof(merkleroot));
    if ( coin->cachedmerkleheight == height )
        return(coin->cachedmerkle);
    if ( (hdrobj= electrum_getheader(coin->symbol,ep,&hdrobj,height)) != 0 )
    {
        if ( jobj(hdrobj,"merkle_root") != 0 )
        {
            merkleroot = jbits256(hdrobj,"merkle_root");
            if ( bits256_nonz(merkleroot) != 0 )
            {
                coin->cachedmerkle = merkleroot;
                coin->cachedmerkleheight = height;
            }
        }
        free_json(hdrobj);
    } else printf("couldnt get header for ht.%d\n",height);
    return(merkleroot);
}

int32_t LP_merkleproof(struct iguana_info *coin,char *coinaddr,struct electrum_info *ep,bits256 txid,int32_t height)
{
    struct LP_transaction *tx=0; cJSON *merkobj,*merkles,*retjson; bits256 roothash,merkleroot; int32_t m,ht=0,SPV = 0;
    if ( height <= 0 )
        return(0);
    if ( (tx= LP_transactionfind(coin,txid)) == 0 && strcmp(coinaddr,coin->smartaddr) == 0 )
    {
        if ( (retjson= electrum_transaction(&ht,coin->symbol,ep,&retjson,txid,0)) != 0 )
            free_json(retjson);
    }
    if ( tx != 0 )
    {
        if ( tx->height == 0 )
        {
            if ( height != 0 )
                tx->height = height;
            else if ( ht != 0 )
                tx->height = ht;
            height = tx->height;
        }
        if ( tx->SPV > 0 )
            return(tx->SPV);
    }
    if ( (merkobj= electrum_getmerkle(coin->symbol,ep,&merkobj,txid,height)) != 0 )
    {
        char str[65],str2[65],str3[65];
        SPV = 0;
        memset(roothash.bytes,0,sizeof(roothash));
        if ( (merkles= jarray(&m,merkobj,"merkle")) != 0 )
        {
            roothash = validate_merkle(jint(merkobj,"pos"),txid,merkles,m);
            merkleroot = LP_merkleroot(coin,ep,height);
            if ( bits256_nonz(merkleroot) != 0 )
            {
                if ( bits256_cmp(merkleroot,roothash) == 0 )
                {
                    SPV = height;
                    LP_SPV_store(coin,txid,height);
                    if ( tx != 0 )
                    {
                        tx->SPV = height;
                        if ( strcmp(coinaddr,coin->smartaddr) != 0 && tx->serialized != 0 )
                        {
                            free(tx->serialized);
                            tx->serialized = 0;
                            tx->len = 0;
                        }
                    }
                    //printf("validated MERK %s ht.%d -> %s root.(%s)\n",bits256_str(str,txid),height,jprint(merkobj,0),bits256_str(str2,roothash));
                }
                else
                {
                    SPV = -1;
                    printf("ERROR MERK %s ht.%d -> %s root.(%s) vs %s\n",bits256_str(str,txid),height,jprint(merkobj,0),bits256_str(str2,roothash),bits256_str(str3,merkleroot));
                }
            } else SPV = 0;
        }
        if ( SPV < 0 )
        {
            printf("MERKLE DIDNT VERIFY.%s %s ht.%d (%s)\n",coin->symbol,bits256_str(str,txid),height,jprint(merkobj,0));
            if ( jobj(merkobj,"error") != 0 )
                SPV = 0; // try again later
        }
        free_json(merkobj);
    }
    return(SPV);
}

char *LP_unspents_filestr(char *symbol,char *addr)
{
    char fname[1024]; long fsize;
    sprintf(fname,"%s/UNSPENTS/%s_%s",GLOBAL_DBDIR,symbol,addr), OS_portable_path(fname);
    return(OS_filestr(&fsize,fname));
}

void LP_unspents_cache(char *symbol,char *addr,char *arraystr,int32_t updatedflag)
{
    char fname[1024]; FILE *fp=0;
    sprintf(fname,"%s/UNSPENTS/%s_%s",GLOBAL_DBDIR,symbol,addr), OS_portable_path(fname);
    //printf("unspents cache.(%s) for %s %s, updated.%d\n",fname,symbol,addr,updatedflag);
    if ( updatedflag == 0 && (fp= fopen(fname,"rb")) == 0 )
        updatedflag = 1;
    else if ( fp != 0 )
        fclose(fp);
    if ( updatedflag != 0 && (fp= fopen(fname,"wb")) != 0 )
    {
        fwrite(arraystr,1,strlen(arraystr),fp);
        fclose(fp);
    }
}

uint64_t LP_unspents_load(char *symbol,char *addr)
{
    char *arraystr; uint64_t balance = 0; int32_t i,n; bits256 zero; cJSON *retjson,*item; struct iguana_info *coin;
    if ( (coin= LP_coinfind(symbol)) != 0 )
    {
        if ( (arraystr= LP_unspents_filestr(symbol,addr)) != 0 )
        {
            if ( (retjson= cJSON_Parse(arraystr)) != 0 )
            {
                //printf("PROCESS UNSPENTS %s\n",arraystr);
                if ( (n= cJSON_GetArraySize(retjson)) > 0 )
                {
                    for (i=0; i<n; i++)
                    {
                        item = jitem(retjson,i);
                        balance += j64bits(item,"value");
                    }
                }
                memset(zero.bytes,0,sizeof(zero));
                electrum_process_array(coin,coin->electrum,addr,retjson,1,zero,zero);
                free_json(retjson);
            }
            free(arraystr);
        }
    }
    return(balance);
}



