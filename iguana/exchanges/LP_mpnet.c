
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
//  LP_mpnet.c
//  marketmaker
//

bits256 MPNET_txids[1024];
int32_t num_MPNET_txids;

int32_t LP_quoteparse(struct LP_quoteinfo *qp,cJSON *argjson);
void LP_gtc_addorder(struct LP_quoteinfo *qp);
char *LP_withdraw(struct iguana_info *coin,cJSON *argjson);

int32_t LP_mpnet_find(bits256 txid)
{
    int32_t i;
    for (i=0; i<num_MPNET_txids; i++)
        if ( bits256_cmp(txid,MPNET_txids[i]) == 0 )
            return(i);
    return(-1);
}

int32_t LP_mpnet_add(bits256 txid)
{
    if ( num_MPNET_txids < sizeof(MPNET_txids)/sizeof(*MPNET_txids) )
    {
        MPNET_txids[num_MPNET_txids++] = txid;
        return(num_MPNET_txids);
    }
    printf("MPNET_txids[] overflow\n");
    return(-1);
}

int32_t LP_mpnet_remove(bits256 txid)
{
    int32_t i;
    if ( (i= LP_mpnet_find(txid)) >= 0 )
    {
        MPNET_txids[i] = MPNET_txids[--num_MPNET_txids];
        return(i);
    }
    return(-1);
}

int32_t LP_mpnet_addorder(struct LP_quoteinfo *qp)
{
    LP_gtc_addorder(qp);
    return(0);
}

void LP_mpnet_init() // problem is coins not enabled yet
{
    char fname[1024],line[8192]; FILE *fp; struct LP_quoteinfo Q; cJSON *argjson;
    sprintf(fname,"%s/GTC/orders",GLOBAL_DBDIR), OS_compatible_path(fname);
    if ( (fp= fopen(fname,"rb+")) != 0 )
    {
        while ( fgets(line,sizeof(line),fp) > 0 )
        {
            if ( (argjson= cJSON_Parse(line)) != 0 )
            {
                if ( LP_quoteparse(&Q,argjson) == 0 )
                {
                    if ( LP_mpnet_addorder(&Q) == 0 )
                        printf("GTC %s",line);
                }
                free_json(argjson);
            }
        }
        fclose(fp);
    }
}

void LP_mpnet_send(int32_t localcopy,char *msg,int32_t sendflag,char *otheraddr)
{
    char fname[1024]; int32_t len; FILE *fp; char *hexstr,*retstr; cJSON *argjson,*outputs,*item; struct iguana_info *coin; uint8_t linebuf[8192];
    if ( localcopy != 0 )
    {
        sprintf(fname,"%s/GTC/orders",GLOBAL_DBDIR), OS_compatible_path(fname);
        if ( (fp= fopen(fname,"rb+")) == 0 )
            fp = fopen(fname,"wb+");
        else fseek(fp,0,SEEK_END);
        fprintf(fp,"%s\n",msg);
        fclose(fp);
    }
}

// 2151978
// 404bc4ac452db07ed16376b3d7e77dbfc22b4a68f7243797125bd0d3bdddf8d1
// 893b46634456034a6d5d73b67026aa157b5e2addbfc6344dfbea6bae85f7dde0
// 717c7ef9de8504bd331f3ef52ed0a16ea0e070434e12cb4d63f5f081e999c43d dup
