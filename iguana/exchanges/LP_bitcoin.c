
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
//  LP_bitcoin.c
//  marketmaker
//

char *bitcoin_base58encode(char *coinaddr,uint8_t *data,int32_t datalen);
int32_t bitcoin_base58decode(uint8_t *data,char *coinaddr);

#define IGUANA_MAXSCRIPTSIZE 10001
#define SCRIPT_OP_IF 0x63
#define SCRIPT_OP_ELSE 0x67
#define SCRIPT_OP_ENDIF 0x68
#define SCRIPT_OP_DROP 0x75
#define SCRIPT_OP_EQUALVERIFY 0x88
#define SCRIPT_OP_HASH160 0xa9

#define SCRIPT_OP_CHECKSIG 0xac
#define SCRIPT_OP_CHECKLOCKTIMEVERIFY 0xb1
#define IGUANA_OP_SIZE 0x82

enum opcodetype
{
    // push value
    OP_0 = 0x00,
    OP_FALSE = OP_0,
    OP_PUSHDATA1 = 0x4c,
    OP_PUSHDATA2 = 0x4d,
    OP_PUSHDATA4 = 0x4e,
    OP_1NEGATE = 0x4f,
    OP_RESERVED = 0x50,
    OP_1 = 0x51,
    OP_TRUE=OP_1,
    OP_2 = 0x52,
    OP_3 = 0x53,
    OP_4 = 0x54,
    OP_5 = 0x55,
    OP_6 = 0x56,
    OP_7 = 0x57,
    OP_8 = 0x58,
    OP_9 = 0x59,
    OP_10 = 0x5a,
    OP_11 = 0x5b,
    OP_12 = 0x5c,
    OP_13 = 0x5d,
    OP_14 = 0x5e,
    OP_15 = 0x5f,
    OP_16 = 0x60,
    
    // control
    OP_NOP = 0x61,
    OP_VER = 0x62,
    OP_IF = 0x63,
    OP_NOTIF = 0x64,
    OP_VERIF = 0x65,
    OP_VERNOTIF = 0x66,
    OP_ELSE = 0x67,
    OP_ENDIF = 0x68,
    OP_VERIFY = 0x69,
    OP_RETURN = 0x6a,
    
    // stack ops
    OP_TOALTSTACK = 0x6b,
    OP_FROMALTSTACK = 0x6c,
    OP_2DROP = 0x6d,
    OP_2DUP = 0x6e,
    OP_3DUP = 0x6f,
    OP_2OVER = 0x70,
    OP_2ROT = 0x71,
    OP_2SWAP = 0x72,
    OP_IFDUP = 0x73,
    OP_DEPTH = 0x74,
    OP_DROP = 0x75,
    OP_DUP = 0x76,
    OP_NIP = 0x77,
    OP_OVER = 0x78,
    OP_PICK = 0x79,
    OP_ROLL = 0x7a,
    OP_ROT = 0x7b,
    OP_SWAP = 0x7c,
    OP_TUCK = 0x7d,
    
    // splice ops
    OP_CAT = 0x7e,
    OP_SUBSTR = 0x7f,
    OP_LEFT = 0x80,
    OP_RIGHT = 0x81,
    OP_SIZE = 0x82,
    
    // bit logic
    OP_INVERT = 0x83,
    OP_AND = 0x84,
    OP_OR = 0x85,
    OP_XOR = 0x86,
    OP_EQUAL = 0x87,
    OP_EQUALVERIFY = 0x88,
    OP_RESERVED1 = 0x89,
    OP_RESERVED2 = 0x8a,
    
    // numeric
    OP_1ADD = 0x8b,
    OP_1SUB = 0x8c,
    OP_2MUL = 0x8d,
    OP_2DIV = 0x8e,
    OP_NEGATE = 0x8f,
    OP_ABS = 0x90,
    OP_NOT = 0x91,
    OP_0NOTEQUAL = 0x92,
    
    OP_ADD = 0x93,
    OP_SUB = 0x94,
    OP_MUL = 0x95,
    OP_DIV = 0x96,
    OP_MOD = 0x97,
    OP_LSHIFT = 0x98,
    OP_RSHIFT = 0x99,
    
    OP_BOOLAND = 0x9a,
    OP_BOOLOR = 0x9b,
    OP_NUMEQUAL = 0x9c,
    OP_NUMEQUALVERIFY = 0x9d,
    OP_NUMNOTEQUAL = 0x9e,
    OP_LESSTHAN = 0x9f,
    OP_GREATERTHAN = 0xa0,
    OP_LESSTHANOREQUAL = 0xa1,
    OP_GREATERTHANOREQUAL = 0xa2,
    OP_MIN = 0xa3,
    OP_MAX = 0xa4,
    
    OP_WITHIN = 0xa5,
    
    // crypto
    OP_RIPEMD160 = 0xa6,
    OP_SHA1 = 0xa7,
    OP_SHA256 = 0xa8,
    OP_HASH160 = 0xa9,
    OP_HASH256 = 0xaa,
    OP_CODESEPARATOR = 0xab,
    OP_CHECKSIG = 0xac,
    OP_CHECKSIGVERIFY = 0xad,
    OP_CHECKMULTISIG = 0xae,
    OP_CHECKMULTISIGVERIFY = 0xaf,
    
    // expansion
    OP_NOP1 = 0xb0,
    OP_CHECKLOCKTIMEVERIFY  = 0xb1,
    OP_CHECKSEQUENCEVERIFY   = 0xb2,
    OP_NOP4 = 0xb3,
    OP_NOP5 = 0xb4,
    OP_NOP6 = 0xb5,
    OP_NOP7 = 0xb6,
    OP_NOP8 = 0xb7,
    OP_NOP9 = 0xb8,
    OP_NOP10 = 0xb9,
    
    OP_COMBINEPUBKEYS = 0xc0,
    OP_CHECKSCHNORR = 0xc1,
    OP_CHECKSCHNORRVERIFY = 0xc2,
    OP_CHECKPRIVATEKEY = 0xc3,
    OP_CHECKPRIVATEKEYVERIFY = 0xc4,
    
    // template matching params
    //OP_SMALLINTEGER = 0xfa,
    //OP_PUBKEYS = 0xfb,
    //OP_PUBKEYHASH = 0xfd,
    //OP_PUBKEY = 0xfe,
    
    OP_INVALIDOPCODE = 0xff,
};

int32_t bitcoin_pubkeylen(const uint8_t *pubkey)
{
    if ( pubkey[0] == 2 || pubkey[0] == 3 )
        return(33);
    else if ( pubkey[0] == 4 )
        return(65);
    else
    {
        //printf("illegal pubkey.[%02x] %llx\n",pubkey[0],*(long long *)pubkey);
        return(-1);
    }
}

void revcalc_rmd160_sha256(uint8_t rmd160[20],bits256 revhash)
{
    bits256 hash; int32_t i;
    for (i=0; i<32; i++)
        hash.bytes[i] = revhash.bytes[31-i];
    calc_rmd160_sha256(rmd160,hash.bytes,sizeof(hash));
}

int32_t bitcoin_pubkeyspend(uint8_t *script,int32_t n,uint8_t pubkey[66])
{
    int32_t plen = bitcoin_pubkeylen(pubkey);
    script[n++] = plen;
    memcpy(&script[n],pubkey,plen);
    n += plen;
    script[n++] = SCRIPT_OP_CHECKSIG;
    return(n);
}

int32_t bitcoin_secret160verify(uint8_t *script,int32_t n,uint8_t secret160[20])
{
    script[n++] = IGUANA_OP_SIZE; // add SIZE 32 EQUALVERIFY
    script[n++] = 1;
    script[n++] = 32;
    script[n++] = SCRIPT_OP_EQUALVERIFY;
    script[n++] = SCRIPT_OP_HASH160;
    script[n++] = 0x14;
    memcpy(&script[n],secret160,0x14);
    n += 0x14;
    script[n++] = SCRIPT_OP_EQUALVERIFY;
    return(n);
}

int32_t bitcoin_checklocktimeverify(uint8_t *script,int32_t n,uint32_t locktime)
{
    script[n++] = 4;
    script[n++] = locktime & 0xff, locktime >>= 8;
    script[n++] = locktime & 0xff, locktime >>= 8;
    script[n++] = locktime & 0xff, locktime >>= 8;
    script[n++] = locktime & 0xff;
    script[n++] = SCRIPT_OP_CHECKLOCKTIMEVERIFY;
    script[n++] = SCRIPT_OP_DROP;
    return(n);
}

bits256 bits256_calcaddrhash(char *symbol,uint8_t *serialized,int32_t  len)
{
    bits256 hash;
    memset(hash.bytes,0,sizeof(hash));
    if ( strcmp(symbol,"GRS") != 0 )
    {
        if ( strcmp(symbol,"SMART") != 0 )
            hash = bits256_doublesha256(0,serialized,len);
        else HashKeccak(hash.bytes,serialized,len);
    }
    else
    {
        HashGroestl(hash.bytes,serialized,len);
        /*int32_t i; char str[65];
        for (i=0; i<len; i++)
            printf("%02x",serialized[i]);
        printf(" HashGroestl %d -> %s\n",len,bits256_str(str,hash));*/
    }
    return(hash);
}

int32_t bitcoin_addr2rmd160(char *symbol,uint8_t taddr,uint8_t *addrtypep,uint8_t rmd160[20],char *coinaddr)
{
    bits256 hash; uint8_t *buf,_buf[26],data5[128],rmd21[21]; char prefixaddr[64],hrp[64]; int32_t len,len5,offset;
    *addrtypep = 0;
    memset(rmd160,0,20);
    if ( coinaddr == 0 || coinaddr[0] == 0 )
        return(0);
    if ( coinaddr[0] == '0' && coinaddr[1] == 'x' && is_hexstr(coinaddr+2,0) == 40 ) // for ETH
    {
        decode_hex(rmd160,20,coinaddr+2); // not rmd160 hash but hopefully close enough;
        return(20);
    }
    if ( strcmp(symbol,"BCH") == 0 )//&& strlen(coinaddr) == 42 )
    {
        char *bchprefix = "bitcoincash:";
        if ( strncmp(coinaddr,bchprefix,strlen(bchprefix)) != 0 )
        {
            strcpy(prefixaddr,bchprefix);
            strcat(prefixaddr,coinaddr);
        } else strcpy(prefixaddr,coinaddr);
        if ( bech32_decode(hrp,data5,&len5,prefixaddr) == 0 )
        {
            printf("bitcoin_addr2rmd160 bech32_decode error.(%s)\n",prefixaddr);
            return(0);
        }
        len = 0;
        if ( bech32_convert_bits(rmd21,&len,8,data5,len5,5,0) == 0 )
            printf("error converting data5\n");
        *addrtypep = rmd21[0] == 0 ? 0 : 5;
        memcpy(rmd160,&rmd21[1],20);
        return(20);
    }
    offset = 1 + (taddr != 0);
    memset(rmd160,0,20);
    *addrtypep = 0;
    buf = _buf;
    if ( (len= bitcoin_base58decode(buf,coinaddr)) >= 4 )
    {
        // validate with trailing hash, then remove hash
        hash = bits256_calcaddrhash(symbol,buf,20+offset);
        *addrtypep = (taddr == 0) ? *buf : buf[1];
        memcpy(rmd160,buf+offset,20);
        if ( (buf[20+offset]&0xff) == hash.bytes[31] && (buf[21+offset]&0xff) == hash.bytes[30] && (buf[22+offset]&0xff) == hash.bytes[29] && (buf[23+offset]&0xff) == hash.bytes[28] )
        {
            //printf("coinaddr.(%s) valid checksum addrtype.%02x\n",coinaddr,*addrtypep);
            return(20);
        }
        else if ( (strcmp(symbol,"GRS") == 0 || strcmp(symbol,"SMART") == 0) && (buf[20+offset]&0xff) == hash.bytes[0] && (buf[21+offset]&0xff) == hash.bytes[1] && (buf[22+offset]&0xff) == hash.bytes[2] && (buf[23+offset]&0xff) == hash.bytes[3] )
            return(20);
        else if ( strcmp(symbol,"BTC") != 0 || *addrtypep == 0 || *addrtypep == 5 )
        {
            int32_t i;
            //if ( len > 20 )
            //    hash = bits256_calcaddrhash(symbol,buf,len);
            for (i=0; i<len; i++)
                printf("%02x ",hash.bytes[i]);
            char str[65]; printf("\n%s addrtype.%d taddr.%02x checkhash.(%s) len.%d mismatch %02x %02x %02x %02x vs %02x %02x %02x %02x (%s)\n",symbol,*addrtypep,taddr,coinaddr,len,buf[len-1]&0xff,buf[len-2]&0xff,buf[len-3]&0xff,buf[len-4]&0xff,hash.bytes[31],hash.bytes[30],hash.bytes[29],hash.bytes[28],bits256_str(str,hash));
        }
    }
    return(0);
}

char *bitcoin_address(char *symbol,char *coinaddr,uint8_t taddr,uint8_t addrtype,uint8_t *pubkey_or_rmd160,int32_t len)
{
    static void *ctx;
    int32_t offset,i,len5; char prefixed[64]; uint8_t data[64],data5[64],bigpubkey[65]; bits256 hash; struct iguana_info *coin;
    coinaddr[0] = 0;
    offset = 1 + (taddr != 0);
    if ( len != 20 )
    {
        calc_rmd160_sha256(data+offset,pubkey_or_rmd160,len);
        //for (i=0; i<20; i++)
        //    printf("%02x",data[offset+i]);
        //printf(" rmd160\n");
    }
    else memcpy(data+offset,pubkey_or_rmd160,20);
    if ( strcmp(symbol,"BCH") == 0 )
    {
        len5 = 0;
        if ( addrtype == 0 )
            data[0] = (0 << 3);
        else data[0] = (1 << 3);
        bech32_convert_bits(data5,&len5,5,data,21,8,1);
        if ( bech32_encode(prefixed,"bitcoincash",data5,len5) == 0 )
            return(0);
        for (i=0; prefixed[i]!=0; i++)
            if ( prefixed[i] == ':' )
                break;
        if ( prefixed[i] != ':' )
            return(0);
        strcpy(coinaddr,&prefixed[i+1]);
        return(coinaddr);
    }
    if ( taddr != 0 )
    {
        data[0] = taddr;
        data[1] = addrtype;
    } else data[0] = addrtype;
    hash = bits256_calcaddrhash(symbol,data,20+offset);
    if ( strcmp(symbol,"GRS") != 0 && strcmp(symbol,"SMART") != 0 )
    {
        for (i=0; i<4; i++)
            data[20+offset+i] = hash.bytes[31-i];
    }
    else
    {
        for (i=0; i<4; i++)
            data[20+offset+i] = hash.bytes[i];
    }
    if ( (coinaddr= bitcoin_base58encode(coinaddr,data,24+offset)) != 0 )
    {
        //printf("coinaddr.%p %s\n",coinaddr,coinaddr!=0?coinaddr:"null");
    } else printf("null coinaddr taddr.%02x\n",taddr);
    return(coinaddr);
}

void bitcoin_priv2pub(void *ctx,char *symbol,uint8_t *pubkey33,char *coinaddr,bits256 privkey,uint8_t taddr,uint8_t addrtype)
{
    bits256 pub;
    memset(pubkey33,0,33);
    coinaddr[0] = 0;
    crypto_box_priv2pub(pub.bytes,privkey.bytes);
    bitcoin_pubkey33(ctx,pubkey33,privkey);
    bitcoin_address(symbol,coinaddr,taddr,addrtype,pubkey33,33);
}

int32_t base58encode_checkbuf(char *symbol,uint8_t taddr,uint8_t addrtype,uint8_t *data,int32_t data_len)
{
    uint8_t i,offset; bits256 hash;
    offset = 1 + (taddr != 0);
    if ( taddr != 0 )
    {
        data[0] = taddr;
        data[1] = addrtype;
    } else data[0] = addrtype;
    //for (i=0; i<data_len+1; i++)
    //    printf("%02x",data[i]);
    //printf(" extpriv -> ");
    hash = bits256_calcaddrhash(symbol,data,(int32_t)data_len+offset);
    //for (i=0; i<32; i++)
    //    printf("%02x",hash.bytes[i]);
    //printf(" checkhash\n");
    if ( strcmp(symbol,"GRS") != 0 && strcmp(symbol,"SMART") != 0 )
    {
        for (i=0; i<4; i++)
            data[data_len+i+offset] = hash.bytes[31-i];
    }
    else
    {
        for (i=0; i<4; i++)
            data[data_len+i+offset] = hash.bytes[i];
    }
    return(data_len + 4 + offset);
}

int32_t bitcoin_wif2priv(char *symbol,uint8_t wiftaddr,uint8_t *addrtypep,bits256 *privkeyp,char *wifstr)
{
    int32_t offset,len = -1; bits256 hash; uint8_t buf[256],*ptr;
    offset = 1 + (wiftaddr != 0);
    memset(buf,0,sizeof(buf));
    memset(privkeyp,0,sizeof(*privkeyp));
    if ( (len= bitcoin_base58decode(buf,wifstr)) >= 4 )
    {
        if ( len >= 32+offset )
        {
            memcpy(privkeyp,buf+offset,32);
            /*if ( len > 32+offset )
                printf("wif %s: extra byte %d len.%d vs %d addrtype.%d\n",wifstr,buf[32+offset],len,32+offset,(wiftaddr == 0) ? buf[0] : buf[1]);
            else printf("%s is for uncompressed\n",wifstr);*/
        }
        else
        {
            printf("wif %s -> buf too short len.%d\n",wifstr,len);
            return(-1);
        }
        ptr = buf;
        hash = bits256_calcaddrhash(symbol,ptr,len - 4);
        *addrtypep = (wiftaddr == 0) ? *ptr : ptr[1];
        if ( (ptr[len - 4]&0xff) == hash.bytes[31] && (ptr[len - 3]&0xff) == hash.bytes[30] &&(ptr[len - 2]&0xff) == hash.bytes[29] && (ptr[len - 1]&0xff) == hash.bytes[28] )
        {
            //int32_t i; for (i=0; i<len; i++)
            //    printf("%02x ",ptr[i]);
            //printf(" ptr, hash.%02x %02x %02x %02x ",hash.bytes[28],hash.bytes[29],hash.bytes[30],hash.bytes[31]);
            //printf("wifstr.(%s) valid len.%d\n",wifstr,len);
            return(32);
        }
        else if ( (strcmp(symbol,"GRS") == 0 || strcmp(symbol,"SMART") == 0) && (ptr[len - 4]&0xff) == hash.bytes[0] && (ptr[len - 3]&0xff) == hash.bytes[1] &&(ptr[len - 2]&0xff) == hash.bytes[2] && (ptr[len - 1]&0xff) == hash.bytes[3] )
            return(32);
    }
    return(-1);
}

int32_t bitcoin_priv2wif(char *symbol,uint8_t wiftaddr,char *wifstr,bits256 privkey,uint8_t addrtype)
{
    uint8_t data[128]; int32_t offset,len = 32;
    if ( wiftaddr != 0 )
    {
        //data[0] = wiftaddr;
        //data[1] = addrtype;
        offset = 2;
    }
    else
    {
        //data[0] = addrtype;
        offset = 1;
    }
    memcpy(data+offset,privkey.bytes,len);
    data[offset + len++] = 1;
    len = base58encode_checkbuf(symbol,wiftaddr,addrtype,data,len);
    if ( bitcoin_base58encode(wifstr,data,len) == 0 ) // skips last byte?
    {
        char str[65]; printf("error making wif from %s\n",bits256_str(str,privkey));
        return(-1);
    }
    return((int32_t)strlen(wifstr));
}

int32_t bitcoin_priv2wiflong(char *symbol,uint8_t wiftaddr,char *wifstr,bits256 privkey,uint8_t addrtype)
{
    uint8_t data[128]; int32_t offset,len = 32;
    offset = 1 + (wiftaddr != 0);
    memcpy(data+offset,privkey.bytes,sizeof(privkey));
    len = base58encode_checkbuf(symbol,wiftaddr,addrtype,data,len);
    if ( bitcoin_base58encode(wifstr,data,len) == 0 )
        return(-1);
    return((int32_t)strlen(wifstr));
}
