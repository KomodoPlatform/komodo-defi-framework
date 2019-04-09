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
//  LP_rpc.c
//  marketmaker
//

cJSON *LP_gettxout_json(bits256 txid,int32_t vout,int32_t height,char *coinaddr,uint64_t value)
{
    cJSON *retjson,*addresses,*sobj;
    retjson = cJSON_CreateObject();
    jaddnum(retjson,"value",dstr(value));
    jaddnum(retjson,"height",height);
    jaddbits256(retjson,"txid",txid);
    jaddnum(retjson,"vout",vout);
    addresses = cJSON_CreateArray();
    jaddistr(addresses,coinaddr);
    sobj = cJSON_CreateObject();
    jaddnum(sobj,"reqSigs",1);
    jaddstr(sobj,"type","pubkey");
    jadd(sobj,"addresses",addresses);
    jadd(retjson,"scriptPubKey",sobj);
    //printf("GETTXOUT.(%s)\n",jprint(retjson,0));
    return(retjson);
}

// not in electrum path
uint64_t LP_txfee(char *symbol) {
    uint64_t txfee = 0;
    if (symbol == 0 || symbol[0] == 0)
        return (LP_MIN_TXFEE);
    if (strcmp(symbol, "BTC") != 0)
        txfee = LP_MIN_TXFEE;
    return (txfee);
}
