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
//  LP_mmjson.c
//  marketmaker
//
#define MMJSON_IPADDR 255
#define MMJSON_BITS256 254
#define MMJSON_SECP33 253
#define MMJSON_SIG 252
#define MMJSON_RMD160 251
#define MMJSON_DECIMAL8 250
#define MMJSON_DECIMAL8STR 249
#define MMJSON_DECIMAL16 248
#define MMJSON_DECIMAL16STR 247
#define MMJSON_DECIMAL24 246
#define MMJSON_DECIMAL24STR 245
#define MMJSON_DECIMAL32 244
#define MMJSON_DECIMAL32STR 243
#define MMJSON_DECIMAL40 242
#define MMJSON_DECIMAL40STR 241
#define MMJSON_DECIMAL48 240
#define MMJSON_DECIMAL48STR 239
#define MMJSON_DOUBLE 238
#define MMJSON_DECIMAL64 237
#define MMJSON_DECIMAL64STR 236
#define MMJSON_TIMESTAMP 235
#define MMJSON_TIMEDIFF8 234
#define MMJSON_TIMEDIFF16 233
#define MMJSON_ZERO 232
#define MMJSON_ZEROSTR 231
#define MMJSON_COIN 230
#define MMJSON_STRING 229
#define MMJSON_ARRAY8 228
#define MMJSON_ARRAY16 227
#define MMJSON_ARRAY32 226

int32_t MM_numfields;
char *MM_fields[] =
{
    "timestamp", "getdPoW", "dPoW", "aliceid", "src", "base", "basevol", "dest", "rel", "relvol", "price", "requestid", "quoteid", "finished", "expired", "bobdeposit", "alicepayment", "bobpayment", "paymentspent", "Apaymentspent", "depositspent", "ind", "method", "swapstatus", "method2", "gettradestatus", "coin", "rmd160", "pub", "pubsecp", "sig", "session", "notify", "pubkey", "price64", "credits", "utxocoin", "n", "bal", "min", "max", "postprice", "notarized", "notarizedhash", "notarizationtxid", "wantnotify", "isLP", "gui", "nogui", "tradeid", "address", "txid", "vout", "srchash", "txfee", "quotetime", "satoshis", "desthash", "txid2", "vout2", "destaddr", "desttxid", "destvout", "feetxid", "feevout", "desttxfee", "destsatoshis", "pending", "reserved", "broadcast", "ismine", "simplegui", "request", "proof", "connect", "expiration", "iambob", "Bgui", "", "Agui", "bob", "srcamount", "bobtxfee", "alice", "destamount", "alicetxfee", "sentflags", "values", "result", "success", "status", "finishtime", "tradestatus", "pair", "connected", "warning", "critical", "endcritical", "cli", "etomic", "bobtomic", "alicetomic", "etomicsrc", "etomicdest", "hyperdex", "uuid"
};
#define MMJSON_BOUNDARY ((int32_t)(sizeof(MM_fields)/sizeof(*MM_fields)))

char *MM_coins[] =
{
    "KMD", "BTC", "CRC", "VOT", "INN", "MOON", "CRW", "EFL", "GBX", "BCO", "BLK", "BTG", "BCH", "ABY", "STAK", "XZC", "QTUM", "PURA", "DSR", "MNZ", "BTCZ", "MAGA", "BSD", "IOP", "BLOCK", "CHIPS", "888", "ARG", "GLT", "ZER", "HODLC", "UIS", "HUC", "PIVX", "BDL", "ARC", "ZCL", "VIA", "ERC", "FAIR", "FLO", "SXC", "CREA", "TRC", "BTA", "SMC", "NMC", "NAV", "EMC2", "SYS", "I0C", "DASH", "STRAT", "MUE", "MONA", "XMY", "MAC", "BTX", "XRE", "LBC", "SIB", "VTC", "REVS", "JUMBLR", "DOGE", "HUSH", "ZEC", "DGB", "ZET", "GAME", "LTC", "SUPERNET", "WLC", "PANGEA", "DEX", "BET", "CRYPTO", "HODL", "MSHARK", "BOTS", "MGW", "COQUI", "KV", "CEAL", "MESH", "ETOMIC", "BTCH", "ETH"
};

int32_t mmjson_coinfind(char *symbol)
{
    int32_t i;
    for (i=0; i<sizeof(MM_coins)/sizeof(*MM_coins); i++)
    {
        if ( MM_coins[i] == 0 )
            return(-1);
        if ( strcmp(MM_coins[i],symbol) == 0 )
            return(i);
    }
    return(-1);
};

int32_t mmadd(char *field)
{
    MM_fields[MM_numfields] = calloc(1,strlen(field)+1);
    strcpy(MM_fields[MM_numfields],field);
    return(MM_numfields++);
}

int32_t mmfind(char *field)
{
    int32_t i;
    if ( MM_numfields == 0 )
    {
        for (i=0; i<sizeof(MM_fields)/sizeof(*MM_fields); i++)
            if ( MM_fields[i] == 0 )
                break;
        MM_numfields = i;
    }
    for (i=0; i<MM_numfields; i++)
    {
        if ( strcmp(MM_fields[i],field) == 0 )
            return(i);
    }
    return(-1);
}

int32_t MMJSON_rwnum(int32_t rwflag,uint8_t *buf,uint64_t *longp,int32_t n)
{
    int32_t i; uint64_t l = 0;
    if ( rwflag != 0 )
    {
        l = *longp;
        for (i=0; i<n; i++)
        {
            buf[i] = (uint8_t)l;
            l >>= 8;
        }
    }
    else
    {
        for (i=n-1; i>=0; i--)
        {
            l <<= 8;
            l |= buf[i];
        }
        *longp = l;
    }
    return(n);
}

int32_t MMJSON_decodeitem(cJSON *lineobj,uint8_t *linebuf,int32_t i,int32_t len,char *fieldstr,uint32_t *timestampp)
{
    int32_t c,valind,j; char tmpstr[64],ipaddr[64],hexstr[256],arbstr[8192]; uint64_t l;
    switch ( (valind= linebuf[i++]) )
    {
        case MMJSON_IPADDR:
            i += MMJSON_rwnum(0,&linebuf[i],&l,4);
            expand_ipbits(ipaddr,(uint32_t)l);
            jaddstr(lineobj,fieldstr,ipaddr);
            break;
        case MMJSON_BITS256:
            init_hexbytes_noT(hexstr,&linebuf[i],32);
            i += 32;
            jaddstr(lineobj,fieldstr,hexstr);
            break;
        case MMJSON_SECP33:
            init_hexbytes_noT(hexstr,&linebuf[i],33);
            i += 33;
            jaddstr(lineobj,fieldstr,hexstr);
            break;
        case MMJSON_SIG:
            init_hexbytes_noT(hexstr,&linebuf[i],65);
            i += 65;
            jaddstr(lineobj,fieldstr,hexstr);
            break;
        case MMJSON_RMD160:
            init_hexbytes_noT(hexstr,&linebuf[i],20);
            i += 20;
            jaddstr(lineobj,fieldstr,hexstr);
            break;
        case MMJSON_DECIMAL8:
            l = linebuf[i++];
            jaddnum(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL8STR:
            l = linebuf[i++];
            jadd64bits(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL16:
            i += MMJSON_rwnum(0,&linebuf[i],&l,2);
            jaddnum(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL16STR:
            i += MMJSON_rwnum(0,&linebuf[i],&l,2);
            jadd64bits(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL24:
            i += MMJSON_rwnum(0,&linebuf[i],&l,3);
            jaddnum(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL24STR:
            i += MMJSON_rwnum(0,&linebuf[i],&l,3);
            sprintf(tmpstr,"%llu",(long long)l);
            jaddstr(lineobj,fieldstr,tmpstr);
            break;
        case MMJSON_DECIMAL32:
            i += MMJSON_rwnum(0,&linebuf[i],&l,4);
            //printf("decimal32.%u %08x\n",(uint32_t)l,(uint32_t)l);
            jaddnum(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL32STR:
            i += MMJSON_rwnum(0,&linebuf[i],&l,4);
            //printf("decimal32.%u %08x\n",(uint32_t)l,(uint32_t)l);
            jadd64bits(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL40:
            i += MMJSON_rwnum(0,&linebuf[i],&l,5);
            jaddnum(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL40STR:
            i += MMJSON_rwnum(0,&linebuf[i],&l,5);
            jadd64bits(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL48:
            i += MMJSON_rwnum(0,&linebuf[i],&l,6);
            jaddnum(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL48STR:
            i += MMJSON_rwnum(0,&linebuf[i],&l,6);
            jadd64bits(lineobj,fieldstr,l);
            break;
        case MMJSON_DOUBLE:
            i += MMJSON_rwnum(0,&linebuf[i],&l,8);
            //printf("double %llu -> %.8f\n",(long long)l,dstr(l));
            jaddnum(lineobj,fieldstr,dstr(l));
            break;
        case MMJSON_DECIMAL64:
            i += MMJSON_rwnum(0,&linebuf[i],&l,8);
            jadd64bits(lineobj,fieldstr,l);
            break;
        case MMJSON_DECIMAL64STR:
            i += MMJSON_rwnum(0,&linebuf[i],&l,8);
            jadd64bits(lineobj,fieldstr,l);
            break;
        case MMJSON_TIMESTAMP:
            if ( *timestampp == 0 )
            {
                i += MMJSON_rwnum(0,&linebuf[i],&l,4);
                *timestampp = (uint32_t)l;
                jaddnum(lineobj,fieldstr,l);
            }
            else
            {
                printf("timestamp %u already exists\n",*timestampp);
                free_json(lineobj);
                return(-1);
            }
            break;
        case MMJSON_TIMEDIFF8:
            jaddnum(lineobj,fieldstr,*timestampp + linebuf[i++]);
            break;
        case MMJSON_TIMEDIFF16:
            i += MMJSON_rwnum(0,&linebuf[i],&l,2);
            jaddnum(lineobj,fieldstr,*timestampp + l);
            break;
        case MMJSON_ZERO:
            jaddnum(lineobj,fieldstr,0);
            break;
        case MMJSON_ZEROSTR:
            //printf("%s.zerostr\n",fieldstr);
            jadd64bits(lineobj,fieldstr,0);
            break;
        case MMJSON_COIN:
            jaddstr(lineobj,fieldstr,MM_coins[linebuf[i++]]);
            break;
        case MMJSON_STRING:
            j = 0;
            while ( (c= linebuf[i++]) != 0 )
            {
                if ( i > len )
                {
                    printf("string overflow i.%d vs len.%d\n",i,len);
                    free_json(lineobj);
                    return(-1);
                }
                arbstr[j++] = c;
            }
            arbstr[j] = 0;
            if (strcmp(fieldstr, "txChain") == 0) {
                cJSON_AddItemToObject(lineobj, fieldstr, cJSON_Parse(arbstr));
            } else {
                jaddstr(lineobj, fieldstr, arbstr);
            }
            break;
        default:
            if ( valind < MMJSON_BOUNDARY )
                jaddstr(lineobj,fieldstr,MM_fields[valind]);
            else
            {
                printf("%s unhandled valind.%d k.%d len.%d (%s)\n",fieldstr,valind,i,len,jprint(lineobj,0));
                free_json(lineobj);
                return(-1);
            }
            break;
    }
    return(i);
}

char *MMJSON_decode(uint8_t *linebuf,int32_t len)
{
    uint32_t timestamp = 0; char *fieldstr; uint64_t l; int32_t ind,i=0,j,m=-1; cJSON *obj,*item,*array,*lineobj = cJSON_CreateObject();
    while ( i+1 < len )
    {
        //printf("ind.%d i.%d vs len.%d\n",linebuf[i],i,len);
        if ( (ind= linebuf[i++]) >= MMJSON_BOUNDARY )
        {
            if ( ind != MMJSON_STRING )
            {
                printf("illegal field i.%d ind.%d (%s) boundary.%d\n",i,ind,jprint(lineobj,0),MMJSON_BOUNDARY);
                free_json(lineobj);
                return(0);
            }
            else
            {
                fieldstr = (char *)&linebuf[i++];
                while ( linebuf[i] != 0 )
                    i++;
                i++;
            }
        } else fieldstr = MM_fields[ind];
        if ( linebuf[i] == MMJSON_ARRAY8 )
        {
            i++;
            m = linebuf[i++];
        }
        else if ( linebuf[i] == MMJSON_ARRAY16 )
        {
            i++;
            i += MMJSON_rwnum(0,&linebuf[i],&l,2);
            m = (int32_t)l;
        }
        else if ( linebuf[i] == MMJSON_ARRAY32 )
        {
            i++;
            i += MMJSON_rwnum(0,&linebuf[i],&l,4);
            m = (int32_t)l;
        } else m = -1;
        if ( m >= 0 )
        {
            //printf("%s i.%d m.%d\n",fieldstr,i,m);
            array = cJSON_CreateArray();
            for (j=0; j<m; j++)
            {
                item = cJSON_CreateObject();
                if ( (i= MMJSON_decodeitem(item,linebuf,i,len,fieldstr,&timestamp)) < 0 )
                {
                    printf("error decoding item ind.%s (%s)\n",fieldstr,jprint(lineobj,0));
                    free_json(array);
                    free_json(lineobj);
                    return(0);
                }
                obj = jobj(item,fieldstr);
                jaddi(array,jduplicate(obj));
                free_json(item);
            }
            jadd(lineobj,fieldstr,array);
        }
        else if ( (i= MMJSON_decodeitem(lineobj,linebuf,i,len,fieldstr,&timestamp)) < 0 )
        {
            printf("error decoding item ind.%s (%s)\n",fieldstr,jprint(lineobj,0));
            free_json(lineobj);
            return(0);
        }
        //printf("i.%d/%d ind.%d %s valind.%d\n",i,len,ind,MM_fields[ind],linebuf[i]);
    }
    return(jprint(lineobj,1));
}

int32_t MMJSON_encodeval(uint8_t *linebuf,int32_t k,int32_t ind,char *v,uint32_t *timestampp,cJSON *ptr,char *fieldstr)
{
    double val; char checkstr[512]; uint64_t l; int32_t valind,len,isstr=0,coinind,j,dots,diff;
    if ( ind >= 0 )
    {
        fieldstr = MM_fields[ind];
        if ( strcmp("utxocoin",fieldstr) == 0 || strcmp("alice",fieldstr) == 0 || strcmp("bob",fieldstr) == 0 || strcmp("base",fieldstr) == 0 || strcmp("rel",fieldstr) == 0 || strcmp("coin",fieldstr) == 0 || strcmp("txfee",fieldstr) == 0 || strcmp("desttxfee",fieldstr) == 0 || strcmp("price64",fieldstr) == 0 || strcmp("satoshis",fieldstr) == 0 || strcmp("destsatoshis",fieldstr) == 0 )
            isstr = 1;
        else isstr = 0;
    }
    //printf("%s.(%s) k.%d\n",fieldstr,v,k);
    if ( (valind= mmfind(v)) >= 0 )
    {
        linebuf[k++] = valind;
        return(k);
    }
    else if ( strcmp("0",v) == 0 )
    {
        if ( isstr != 0 )
            linebuf[k++] = MMJSON_ZEROSTR;
        else linebuf[k++] = MMJSON_ZERO;
        return(k);
    }
    for (j=dots=0; v[j]!=0; j++)
    {
        if ( (v[j] < '0' || v[j] > '9') && v[j] != '.' )
            break;
        else if ( v[j] == '.' )
            dots++;
    }
    if ( dots == 3 && v[j] == 0 && strlen(v) < 17 && is_ipaddr(v) != 0 )
    {
        //printf("<ipaddr> ");
        linebuf[k++] = MMJSON_IPADDR;
        l = calc_ipbits(v);
        k += MMJSON_rwnum(1,&linebuf[k],&l,4);
    }
    else if ( dots == 1 && v[j] == 0 )
    {
        if ( (val= atof(v)) > SMALLVAL )
        {
            l = SATOSHIDEN * (val + 0.000000005);
            sprintf(checkstr,"%.8f",dstr(l));
            if ( strcmp(checkstr,v) == 0 )
            {
                //printf("<double> ");
                linebuf[k++] = MMJSON_DOUBLE;
                k += MMJSON_rwnum(1,&linebuf[k],&l,8);
            } else printf("ERR.<%s %s> ",v,checkstr);
        }
    }
    else if ( (len= is_hexstr(v,0)) == 64 )
    {
        //printf("<bits256> ");
        linebuf[k++] = MMJSON_BITS256;
        decode_hex(&linebuf[k],32,v), k += 32;
    }
    else if ( len == 66 )
    {
        //printf("<secp33> ");
        linebuf[k++] = MMJSON_SECP33;
        decode_hex(&linebuf[k],33,v), k += 33;
    }
    else if ( len == 65*2 )
    {
        //printf("<sig> ");
        linebuf[k++] = MMJSON_SIG;
        decode_hex(&linebuf[k],65,v), k += 65;
    }
    else if ( len == 40 )
    {
        //printf("<rmd160> ");
        linebuf[k++] = MMJSON_RMD160;
        decode_hex(&linebuf[k],20,v), k += 20;
    }
    else if ( len > 40 )
    {
        printf("ERR.<hex.%d> ",len/2);
    }
    else if ( is_decimalstr(v) != 0 && (l= calc_nxt64bits(v)) > 0 )
    {
        if ( l < 0x100 )
        {
            //printf("<decimal8> ");
            if ( l == 0 )
            {
                linebuf[k++] = isstr != 0 ? MMJSON_ZEROSTR : MMJSON_ZERO;
            }
            else
            {
                linebuf[k++] = isstr != 0 ? MMJSON_DECIMAL8STR : MMJSON_DECIMAL8;
                linebuf[k++] = (uint8_t)l;
            }
        }
        else if ( l < 0x10000 )
        {
            //printf("<decimal16> ");
            linebuf[k++] = isstr != 0 ? MMJSON_DECIMAL16STR : MMJSON_DECIMAL16;
            k += MMJSON_rwnum(1,&linebuf[k],&l,2);
        }
        else if ( l < 0x1000000 )
        {
            linebuf[k++] = isstr != 0 ? MMJSON_DECIMAL24STR : MMJSON_DECIMAL24;
            //printf("decimal24 %llu %s (%s) %d\n",(long long)l,v,fieldstr,linebuf[k-1]);
            k += MMJSON_rwnum(1,&linebuf[k],&l,3);
        }
        else if ( l < 0x100000000LL )
        {
            if ( v[0] != '"' && *timestampp == 0 && strcmp(fieldstr,"timestamp") == 0 )
            {
                *timestampp = (uint32_t)atol(v);
                //printf("<timestamp> ");
                linebuf[k++] = MMJSON_TIMESTAMP;
                l = *timestampp;
                k += MMJSON_rwnum(1,&linebuf[k],&l,4);
            }
            else if ( v[0] != '"' && *timestampp != 0 && (diff= ((uint32_t)atol(v)-*timestampp)) < 0x100 && diff >= 0 )
            {
                //printf("<timediff.8> ");
                linebuf[k++] = MMJSON_TIMEDIFF8;
                linebuf[k++] = (uint8_t)diff;
            }
            else if ( v[0] != '"' && *timestampp != 0 && (diff= ((uint32_t)atol(v)-*timestampp)) < 0x10000 && diff >= 0 )
            {
                //printf("<timediff.16> ");
                linebuf[k++] = MMJSON_TIMEDIFF16;
                l = diff;
                k += MMJSON_rwnum(1,&linebuf[k],&l,2);
            }
            else
            {
                //printf("<decimal32>.%u %08x\n",(uint32_t)l,(uint32_t)l);
                linebuf[k++] = isstr != 0 ? MMJSON_DECIMAL32STR : MMJSON_DECIMAL32;
                k += MMJSON_rwnum(1,&linebuf[k],&l,4);
            }
        }
        else if ( l < 0x10000000000LL )
        {
            //printf("<decimal40> ");
            linebuf[k++] = isstr != 0 ? MMJSON_DECIMAL40STR : MMJSON_DECIMAL40;
            k += MMJSON_rwnum(1,&linebuf[k],&l,5);
        }
        else if ( l < 0x1000000000000LL )
        {
            //printf("<decimal48> ");
            linebuf[k++] = isstr != 0 ? MMJSON_DECIMAL48STR : MMJSON_DECIMAL48;
            k += MMJSON_rwnum(1,&linebuf[k],&l,6);
        }
        //else if ( l < 0x100000000000000LL )
        //    printf("<decimal56> ");
        else
        {
            //printf("<decimal64> ");
            linebuf[k++] = isstr != 0 ? MMJSON_DECIMAL64STR : MMJSON_DECIMAL64;
            k += MMJSON_rwnum(1,&linebuf[k],&l,8);
        }
    }
    else
    {
        if ( (ind= mmfind(v)) >= 0 )
            linebuf[k++] = ind;
        else
        {
            for (j=0; v[j]!=0; j++)
            {
                if ( v[j] >= 'a' && v[j] <= 'z' )
                    continue;
                else break;
            }
            if ( v[j] == 0 )
            {
                static uint32_t counter;
                if ( counter++ < 3 )
                    printf("unexpected missing string value.(%s)\n",v);
                //ind = mmadd(v);
                //printf("%s.<%s>.%d ",s,v,ind);
                //linebuf[k++] = ind;
                linebuf[k++] = MMJSON_STRING;
                memcpy(&linebuf[k],v,strlen(v)+1);
                k += (int32_t)strlen(v) + 1;
            }
            else
            {
                for (j=0; v[j]!=0; j++)
                {
                    if ( v[j] >= 'A' && v[j] <= 'Z' )
                        continue;
                    else break;
                }
                if ( v[j] == 0 && (coinind= mmjson_coinfind(v)) >= 0 )
                {
                    //printf("<coin> ");
                    linebuf[k++] = MMJSON_COIN;
                    linebuf[k++] = coinind;
                }
                /*else if ( strlen(v) == 34 )
                 {
                 printf("<coinaddr> ");
                 k += 22;
                 }*/
                else
                {
                    linebuf[k++] = MMJSON_STRING;
                    if ( v[0] == '"' )
                    {
                        v++;
                        v[strlen(v)-1] = 0;
                    }
                    //printf("str.<%s> ",v);
                    memcpy(&linebuf[k],v,strlen(v)+1);
                    k += (int32_t)strlen(v) + 1;
                }
            }
        }
    }
    return(k);
}

int32_t MMJSON_encode(uint8_t *linebuf,char *line)
{
    uint32_t timestamp; uint64_t l; char *decodestr,*s,*v=0; cJSON *lineobj,*array,*ptr; int32_t k=0,m,i,asize,ind,z,allocv_flag;
    timestamp = 0;
    if ( (lineobj= cJSON_Parse(line)) != 0 && strcmp(jstr(lineobj, "method2"), "swapstatus") != 0)
    {
        if ( line[strlen(line)-1] == '\n' )
            line[strlen(line)-1] = 0;
        //printf("%s\n",jprint(lineobj,0));
        if ( (m= cJSON_GetArraySize(lineobj)) > 0 )
        {
            ptr = lineobj->child;
            for (i=0; i<m; i++,ptr=ptr->next)
            {
                allocv_flag = 0;
                s = jfieldname(ptr);
                if ( (ind= mmfind(s)) < 0 )
                {
                    static uint32_t counter;
                    if ( counter++ < 3 )
                        printf("missing field.(%s) add to MM_fields[]\n",s);
                    linebuf[k++] = MMJSON_STRING;
                    memcpy(&linebuf[k],s,strlen(s)+1);
                    k += (int32_t)strlen(s) + 1;
                    //ind = mmadd(s);
                } else linebuf[k++] = ind;
                //printf("%s ",s);
                if ( (array= jobj(lineobj,s)) != 0 && is_cJSON_Array(array) != 0 )
                {
                    asize = cJSON_GetArraySize(array);
                    if ( asize < 0x100 )
                    {
                        linebuf[k++] = MMJSON_ARRAY8;
                        linebuf[k++] = asize;
                    }
                    else if ( asize < 0x10000 )
                    {
                        linebuf[k++] = MMJSON_ARRAY16;
                        l = asize;
                        k += MMJSON_rwnum(1,&linebuf[k],&l,2);
                    }
                    else
                    {
                        linebuf[k++] = MMJSON_ARRAY32;
                        l = asize;
                        k += MMJSON_rwnum(1,&linebuf[k],&l,4);
                    }
                    for (z=0; z<asize; z++)
                    {
                        if ( (v= jprint(jitem(array,z),0)) != 0 )
                        {
                            //printf("%d.(%s k.%d).%d ",z,v,k,asize);
                            k = MMJSON_encodeval(linebuf,k,ind,v,&timestamp,ptr,s);
                            free(v);
                        } else printf("ERROR.(%s) ",jprint(jitem(array,z),0));
                    }
                    //printf("%s array.%d k.%d\n",fieldstr,asize,k);
                    continue;
                }
                else if ( (v= jstr(lineobj,s)) == 0 )
                {
                    v = jprint(jobj(lineobj,s),0);
                    //printf("allocate v.%p\n",v);
                    allocv_flag = 1;
                }
                if ( v != 0 )
                {
                    //printf("%s\n",v);
                    k = MMJSON_encodeval(linebuf,k,ind,v,&timestamp,ptr,s);
                }
                else printf("ERROR.(%s) ",jprint(jobj(lineobj,s),0));
                if ( allocv_flag != 0 && v != 0 )
                {
                    //printf("free allocated v\n");
                    free(v);
                }
                //printf("m.%d values\n",m);
            }
        }
        free_json(lineobj);
        if ( (decodestr= MMJSON_decode(linebuf,k)) == 0 || strcmp(decodestr,line) != 0 )
        {
            printf(" k.%d error decoding (%s) -> (%s)\n",k,line,decodestr==0?"":decodestr);
            if ( decodestr != 0 )
                free(decodestr);
            return(-1);
        } //else printf("decoded\n");
        free(decodestr);
    }
    return(k);
}

#ifndef FROM_MARKETMAKER
#define packetout "/Users/mac/mmjson/packet.out"
#define packetlog "/Users/mac/mmjson/packet.log"

int main(int argc, const char * argv[])
{
    FILE *fp,*outfp; uint8_t linebuf[8192]; char line[8192],str[65]; int32_t i,k,compressed=0,n=0,total = 0;
    outfp = fopen(packetout,"wb");
    if ( (fp= fopen(packetlog,"rb")) != 0 )
    {
        while ( fgets(line,sizeof(line),fp) > 0 )
        {
            n++;
            total += strlen(line);
            if ( (k= MMJSON_encode(linebuf,line)) > 0 )
            {
                //printf("\n");
                if ( outfp != 0 )
                    fwrite(linebuf,1,k,outfp);
                compressed += k;
            }
            else
            {
                compressed += strlen(line);
                //printf("error parsing.(%s)\n",line);
            }
        }
        fclose(fp);
        if ( outfp != 0 )
        {
            uint8_t *data,*bits; int32_t numbits; bits256 seed; long fsize = ftell(outfp);
            fclose(outfp);
            if ( (0) && (outfp= fopen(packetout,"rb")) != 0 )
            {
                data = calloc(1,fsize);
                bits = calloc(1,fsize);
                if ( fread(data,1,fsize,outfp) == fsize )
                {
                    memset(seed.bytes,0,sizeof(seed));
                    decode_hex(seed.bytes,32,"ffffff070000810478800084000800b200101400002001400404844402d29fc4");
                    numbits = ramcoder_compress(bits,(int32_t)fsize,data,(int32_t)fsize,seed);
                    fclose(outfp);
                    printf("numbits.%d %d bytes %.1f seed.%s\n",numbits,numbits/8+1,(double)compressed/(numbits/8),bits256_str(str,seed));
                }
            }
        }
    } else printf("cant find packet.log\n");
    printf("char *MM_fields[256] = \n{\n");
    for (i=0; i<MM_numfields; i++)
        printf("\"%s\", ",MM_fields[i]);
    printf("\n};\nnumlines.%d size %d compressed.%d %.3f maxind.%d\n",n,total,compressed,(double)total/compressed,MM_numfields);
}
#endif
