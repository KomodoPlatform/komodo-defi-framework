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

#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <errno.h>
#include <sys/types.h>
#include <sys/stat.h>
#include "LP_include.h"

char *OS_nonportable_path(char *str)
{
    int32_t i;
    for (i=0; str[i]!=0; i++)
        if ( str[i] == '/' )
            str[i] = '\\';
    return(str);
}

char *OS_portable_path(char *str)
{
#ifdef _WIN32
    return(OS_nonportable_path(str));
#else
#ifdef __PNACL
    /*int32_t i,n;
    if ( str[0] == '/' )
        return(str);
    else
    {
        n = (int32_t)strlen(str);
        for (i=n; i>0; i--)
            str[i] = str[i-1];
        str[0] = '/';
        str[n+1] = 0;
    }*/
#endif
    return(str);
#endif
}

char *OS_compatible_path(char *str)
{
    return(OS_portable_path(str));
}

void OS_ensure_directory(char *dirname)
{
    FILE *fp; int32_t retval; char fname[512];
    sprintf(fname,"%s/.tmpmarker",dirname);
    if ( (fp= fopen(OS_compatible_path(fname),"rb")) == 0 )
    {
        if ( (fp= fopen(OS_compatible_path(dirname),"rb")) == 0 )
        {
            retval = mkdir(dirname
#ifndef _WIN32
                           ,511
#endif
                           );
            //printf("mkdir.(%s) retval.%d errno.%d %s\n",dirname,retval,errno,strerror(errno));
        } else fclose(fp);//, printf("dirname.(%s) exists\n",dirname);
        if ( (fp= fopen(fname,"wb")) != 0 )
            fclose(fp);//, printf("created.(%s)\n",fname);
        else printf("cant create.(%s) errno.%d %s\n",fname,errno,strerror(errno));
    } else fclose(fp);//, printf("%s exists\n",fname);
}

