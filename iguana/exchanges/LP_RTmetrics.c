
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
//  LP_RTmetrics.c
//  marketmaker
//

#define LP_NUMRT 1024
struct LP_RTmetrics_pendings
{
    char refbase[128],refrel[128];
    int64_t pending_kmdvalue[LP_NUMRT];
    int32_t numswaps,numavoidtxids,numwhitelist,numblacklist,numpendings,pending_swaps[LP_NUMRT];
    bits256 avoidtxids[8192],whitelist[LP_NUMRT],blacklist[LP_NUMRT],pending_pubkeys[LP_NUMRT];
} LP_RTmetrics;

int32_t LP_bits256_find(bits256 *list,int32_t num,bits256 val)
{
    int32_t i;
    if ( bits256_nonz(val) != 0 )
    {
        for (i=0; i<num; i++)
            if ( bits256_cmp(list[i],val) == 0 )
                return(i);
    }
    return(-1);
}

int32_t LP_RTmetrics_avoidtxid(bits256 txid)
{
    return(LP_bits256_find(LP_RTmetrics.avoidtxids,LP_RTmetrics.numavoidtxids,txid));
}

int32_t LP_RTmetrics_blacklisted(bits256 pubkey)
{
    return(LP_bits256_find(LP_RTmetrics.blacklist,LP_RTmetrics.numblacklist,pubkey));
}
