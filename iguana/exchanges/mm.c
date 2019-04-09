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
//  main.c
//  marketmaker
//
//  Copyright © 2017-2018 SuperNET. All rights reserved.
//

#define FROM_MARKETMAKER

#include <stdio.h>
#include <stdint.h>
#ifndef NATIVE_WINDOWS
#include "OS_portable.h"
#else
#include "../../crypto777/OS_portable.h"
#endif // !_WIN_32

uint32_t DOCKERFLAG;
uint32_t LP_ORDERBOOK_DURATION = 180;
uint32_t LP_AUTOTRADE_TIMEOUT = 30;
uint32_t LP_RESERVETIME = 90;
#include "stats.c"

#ifndef NN_WS_MSG_TYPE
#define NN_WS_MSG_TYPE 1
#endif

#include "LP_nativeDEX.c"

void LP_ports(uint16_t *pullportp,uint16_t *pubportp,uint16_t *busportp,uint16_t netid)
{
    int32_t netmod,netdiv; uint16_t otherports;
    *pullportp = *pubportp = *busportp = 0;
    if ( netid < 0 )
        netid = 0;
    else if ( netid > (65535-40-LP_RPCPORT)/4 )
    {
        printf("netid.%d overflow vs max netid.%d 14420?\n",netid,(65535-40-LP_RPCPORT)/4);
        exit(-1);
    }
    if ( netid != 0 )
    {
        netmod = (netid % 10);
        netdiv = (netid / 10);
        otherports = (netdiv * 40) + (LP_RPCPORT + netmod);
    } else otherports = LP_RPCPORT;
    *pullportp = otherports + 10;
    *pubportp = otherports + 20;
    *busportp = otherports + 30;
    printf("RPCport.%d remoteport.%d, nanoports %d %d %d\n",RPC_port,RPC_port-1,*pullportp,*pubportp,*busportp);
}

/// Useful when we want to monitor the MM output closely but piped output is buffered by default.  
/// Can also redirect the output to `log_path`, which sometimes is used in the integration tests.  
/// Eventually we should port most of the logging to Rust and this will go away.
void unbuffered_output_support(const char* log_path)
{
    if (log_path != 0)
    {
        if (freopen(log_path, "a", stdout) == 0)
        {
            printf("Can't reopen stdout to log_path %s\n", log_path);
            abort();
        }
        if (freopen(log_path, "a", stderr) == 0)
        {
            printf("Can't reopen stderr to log_path %s\n", log_path);
            abort();
        }
    }

    // For some reason this doesn't work with `freopen` on Windows.
    // TODO: Start a thread to periodically flush the streams instead.
    if (getenv("MM2_UNBUFFERED_OUTPUT") != 0)
    {
        setvbuf(stdout, 0, _IONBF, 0);
        setvbuf(stderr, 0, _IONBF, 0);
    }
}
