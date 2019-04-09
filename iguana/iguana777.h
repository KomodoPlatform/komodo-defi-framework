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

/*
 adding assetchain coin: copy genCOIN to SuperNET/iguana/coins, make a _7776 variant with RELAY=-1 and VALIDATE=0
 copy that into basilisk as coin, changing RELAY -> 0
 */

/*
 To add a new dPoW'ed assetchain with DEX* API support:
 1. add to komodo/src: assetchains, assetchains.old, dpowassets, fiat-cli
 2. add to end of NOTARY_CURRENCIES[] array in fundnotaries (iguana_notary.c)
 3. create fiat/<ac_name>
 4. add to m_notary coins/<ac_name> get gen_acname  from where komodod was launched, change RELAY:-1 and port to 7776 and make <ac_name>_7776 variant
 5. make coins/basilisk/<ac_name>
 6. launch from a single node with -gen, launch a second node using -addnode=<ipaddr of 1st node> but without -gen
 7. from a single node, fundnotaries <ac_name> to get notaries able to dPoW
 8. m_splitfunds
 
 */

#ifndef iguana777_net_h
#define iguana777_net_h

#if defined(_WIN32) || defined(_WIN64)
#ifndef WIN32
#define WIN32
#endif
#endif

#if (defined(_WIN32) || defined(__WIN32__)) && \
!defined(WIN32) && !defined(__SYMBIAN32__)
#ifndef WIN32
#define WIN32
#endif
#endif

#ifdef WIN32
#define __MINGW


#else
#ifndef __MINGW
#include <arpa/inet.h>
#endif
#endif

#define LOCKTIME_THRESHOLD 500000000
#define KOMODO_INTEREST ((uint64_t)(0.05 * SATOSHIDEN))   // 5% CANNOT CHANGE as komodo_interest.h div 20

//#define BTC2_VERSION
#define BTC2_HARDFORK_HEIGHT 444444
#define BTC2_SIGHASH_FORKID 0xcf
#define BTC2_NETMAGIC 0xaabbccdd
#define BTC2_DEFAULT_PORT 8222
#define BTC2_DIFF_WINDOW 60

/*#ifdef __APPLE__
#define ISNOTARYNODE 1
#include "nn.h"
#include "bus.h"
#else*/
//#ifdef __APPLE__
struct supernet_info;
struct exchange_info;

#include "../crypto777/OS_portable.h"

#include "../includes/iguana_defines.h"
#include "../includes/iguana_types.h"
#include "../includes/iguana_structs.h"
#include "../includes/iguana_funcs.h"
#include "../includes/iguana_globals.h"

#ifndef MAX
#define MAX(a,b) ((a) >= (b) ? (a) : (b))
#endif
#ifndef MIN
#define MIN(a,b) ((a) < (b) ? (a) : (b))
#endif

#endif
