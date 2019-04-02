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
// derived from curve25519_donna

#include "../includes/curve25519.h"
#include <string.h>

//#undef force_inline
//#define force_inline  __attribute__((always_inline))

// Sum two numbers: output += in
static inline bits320 fsum(bits320 output,bits320 in)
{
    int32_t i;
    for (i=0; i<5; i++)
        output.ulongs[i] += in.ulongs[i];
    return(output);
}

static inline void fdifference_backwards(uint64_t *out,const uint64_t *in)
{
    static const uint64_t two54m152 = (((uint64_t)1) << 54) - 152;  // 152 is 19 << 3
    static const uint64_t two54m8 = (((uint64_t)1) << 54) - 8;
    int32_t i;
    out[0] = in[0] + two54m152 - out[0];
    for (i=1; i<5; i++)
        out[i] = in[i] + two54m8 - out[i];
}

void store_limb(uint8_t *out,uint64_t in)
{
    int32_t i;
    for (i=0; i<8; i++,in>>=8)
        out[i] = (in & 0xff);
}

static inline uint64_t load_limb(uint8_t *in)
{
    return
    ((uint64_t)in[0]) |
    (((uint64_t)in[1]) << 8) |
    (((uint64_t)in[2]) << 16) |
    (((uint64_t)in[3]) << 24) |
    (((uint64_t)in[4]) << 32) |
    (((uint64_t)in[5]) << 40) |
    (((uint64_t)in[6]) << 48) |
    (((uint64_t)in[7]) << 56);
}

// Take a little-endian, 32-byte number and expand it into polynomial form
bits320 fexpand(bits256 basepoint)
{
    bits320 out;
    out.ulongs[0] = load_limb(basepoint.bytes) & 0x7ffffffffffffLL;
    out.ulongs[1] = (load_limb(basepoint.bytes+6) >> 3) & 0x7ffffffffffffLL;
    out.ulongs[2] = (load_limb(basepoint.bytes+12) >> 6) & 0x7ffffffffffffLL;
    out.ulongs[3] = (load_limb(basepoint.bytes+19) >> 1) & 0x7ffffffffffffLL;
    out.ulongs[4] = (load_limb(basepoint.bytes+24) >> 12) & 0x7ffffffffffffLL;
    return(out);
}

#if __amd64__
// donna: special gcc mode for 128-bit integers. It's implemented on 64-bit platforms only as far as I know.
typedef unsigned uint128_t __attribute__((mode(TI)));

// Multiply a number by a scalar: output = in * scalar
static inline bits320 fscalar_product(const bits320 in,const uint64_t scalar)
{
    int32_t i; uint128_t a = 0; bits320 output;
    a = ((uint128_t)in.ulongs[0]) * scalar;
    output.ulongs[0] = ((uint64_t)a) & 0x7ffffffffffffLL;
    for (i=1; i<5; i++)
    {
        a = ((uint128_t)in.ulongs[i]) * scalar + ((uint64_t) (a >> 51));
        output.ulongs[i] = ((uint64_t)a) & 0x7ffffffffffffLL;
    }
    output.ulongs[0] += (a >> 51) * 19;
    return(output);
}

// Multiply two numbers: output = in2 * in
// output must be distinct to both inputs. The inputs are reduced coefficient form, the output is not.
// Assumes that in[i] < 2**55 and likewise for in2. On return, output[i] < 2**52
bits320 fmul(const bits320 in2,const bits320 in)
{
    uint128_t t[5]; uint64_t r0,r1,r2,r3,r4,s0,s1,s2,s3,s4,c; bits320 out;
    r0 = in.ulongs[0], r1 = in.ulongs[1], r2 = in.ulongs[2], r3 = in.ulongs[3], r4 = in.ulongs[4];
    s0 = in2.ulongs[0], s1 = in2.ulongs[1], s2 = in2.ulongs[2], s3 = in2.ulongs[3], s4 = in2.ulongs[4];
    t[0]  =  ((uint128_t) r0) * s0;
    t[1]  =  ((uint128_t) r0) * s1 + ((uint128_t) r1) * s0;
    t[2]  =  ((uint128_t) r0) * s2 + ((uint128_t) r2) * s0 + ((uint128_t) r1) * s1;
    t[3]  =  ((uint128_t) r0) * s3 + ((uint128_t) r3) * s0 + ((uint128_t) r1) * s2 + ((uint128_t) r2) * s1;
    t[4]  =  ((uint128_t) r0) * s4 + ((uint128_t) r4) * s0 + ((uint128_t) r3) * s1 + ((uint128_t) r1) * s3 + ((uint128_t) r2) * s2;
    r4 *= 19, r1 *= 19, r2 *= 19, r3 *= 19;
    t[0] += ((uint128_t) r4) * s1 + ((uint128_t) r1) * s4 + ((uint128_t) r2) * s3 + ((uint128_t) r3) * s2;
    t[1] += ((uint128_t) r4) * s2 + ((uint128_t) r2) * s4 + ((uint128_t) r3) * s3;
    t[2] += ((uint128_t) r4) * s3 + ((uint128_t) r3) * s4;
    t[3] += ((uint128_t) r4) * s4;
    r0 = (uint64_t)t[0] & 0x7ffffffffffffLL; c = (uint64_t)(t[0] >> 51);
    t[1] += c;      r1 = (uint64_t)t[1] & 0x7ffffffffffffLL; c = (uint64_t)(t[1] >> 51);
    t[2] += c;      r2 = (uint64_t)t[2] & 0x7ffffffffffffLL; c = (uint64_t)(t[2] >> 51);
    t[3] += c;      r3 = (uint64_t)t[3] & 0x7ffffffffffffLL; c = (uint64_t)(t[3] >> 51);
    t[4] += c;      r4 = (uint64_t)t[4] & 0x7ffffffffffffLL; c = (uint64_t)(t[4] >> 51);
    r0 +=   c * 19; c = r0 >> 51; r0 = r0 & 0x7ffffffffffffLL;
    r1 +=   c;      c = r1 >> 51; r1 = r1 & 0x7ffffffffffffLL;
    r2 +=   c;
    out.ulongs[0] = r0, out.ulongs[1] = r1, out.ulongs[2] = r2, out.ulongs[3] = r3, out.ulongs[4] = r4;
    return(out);
}

bits320 fsquare_times(const bits320 in,uint64_t count)
{
    uint128_t t[5]; uint64_t r0,r1,r2,r3,r4,c,d0,d1,d2,d4,d419; bits320 out;
    r0 = in.ulongs[0], r1 = in.ulongs[1], r2 = in.ulongs[2], r3 = in.ulongs[3], r4 = in.ulongs[4];
    do
    {
        d0 = r0 * 2;
        d1 = r1 * 2;
        d2 = r2 * 2 * 19;
        d419 = r4 * 19;
        d4 = d419 * 2;
        t[0] = ((uint128_t) r0) * r0 + ((uint128_t) d4) * r1 + (((uint128_t) d2) * (r3     ));
        t[1] = ((uint128_t) d0) * r1 + ((uint128_t) d4) * r2 + (((uint128_t) r3) * (r3 * 19));
        t[2] = ((uint128_t) d0) * r2 + ((uint128_t) r1) * r1 + (((uint128_t) d4) * (r3     ));
        t[3] = ((uint128_t) d0) * r3 + ((uint128_t) d1) * r2 + (((uint128_t) r4) * (d419   ));
        t[4] = ((uint128_t) d0) * r4 + ((uint128_t) d1) * r3 + (((uint128_t) r2) * (r2     ));
        
        r0 = (uint64_t)t[0] & 0x7ffffffffffffLL; c = (uint64_t)(t[0] >> 51);
        t[1] += c;      r1 = (uint64_t)t[1] & 0x7ffffffffffffLL; c = (uint64_t)(t[1] >> 51);
        t[2] += c;      r2 = (uint64_t)t[2] & 0x7ffffffffffffLL; c = (uint64_t)(t[2] >> 51);
        t[3] += c;      r3 = (uint64_t)t[3] & 0x7ffffffffffffL; c = (uint64_t)(t[3] >> 51);
        t[4] += c;      r4 = (uint64_t)t[4] & 0x7ffffffffffffLL; c = (uint64_t)(t[4] >> 51);
        r0 +=   c * 19; c = r0 >> 51; r0 = r0 & 0x7ffffffffffffLL;
        r1 +=   c;      c = r1 >> 51; r1 = r1 & 0x7ffffffffffffLL;
        r2 +=   c;
    } while( --count );
    out.ulongs[0] = r0, out.ulongs[1] = r1, out.ulongs[2] = r2, out.ulongs[3] = r3, out.ulongs[4] = r4;
    return(out);
}

static inline void fcontract_iter(uint128_t t[5],int32_t flag)
{
    int32_t i; uint64_t mask = 0x7ffffffffffffLL;
    for (i=0; i<4; i++)
        t[i+1] += t[i] >> 51, t[i] &= mask;
    if ( flag != 0 )
        t[0] += 19 * (t[4] >> 51); t[4] &= mask;
}

// donna: Take a fully reduced polynomial form number and contract it into a little-endian, 32-byte array
bits256 fcontract(const bits320 input)
{
    uint128_t t[5]; int32_t i; bits256 out;
    for (i=0; i<5; i++)
        t[i] = input.ulongs[i];
    fcontract_iter(t,1), fcontract_iter(t,1);
    // donna: now t is between 0 and 2^255-1, properly carried.
    // donna: case 1: between 0 and 2^255-20. case 2: between 2^255-19 and 2^255-1.
    t[0] += 19, fcontract_iter(t,1);
    // now between 19 and 2^255-1 in both cases, and offset by 19.
    t[0] += 0x8000000000000 - 19;
    for (i=1; i<5; i++)
        t[i] += 0x8000000000000 - 1;
    // now between 2^255 and 2^256-20, and offset by 2^255.
    fcontract_iter(t,0);
    store_limb(out.bytes,t[0] | (t[1] << 51));
    store_limb(out.bytes+8,(t[1] >> 13) | (t[2] << 38));
    store_limb(out.bytes+16,(t[2] >> 26) | (t[3] << 25));
    store_limb(out.bytes+24,(t[3] >> 39) | (t[4] << 12));
    return(out);
}

bits256 curve25519(bits256 mysecret,bits256 basepoint)
{
    bits320 bp,x,z;
    mysecret.bytes[0] &= 0xf8, mysecret.bytes[31] &= 0x7f, mysecret.bytes[31] |= 0x40;
    bp = fexpand(basepoint);
    cmult(&x,&z,mysecret,bp);
    return(fcontract(fmul(x,crecip(z))));
}

#else
// from curve25519-donna.c
typedef uint8_t u8;
typedef int32_t s32;
typedef int64_t limb;

/* Multiply a number by a scalar: output = in * scalar */
static void fscalar_product32(limb *output, const limb *in, const limb scalar) {
    unsigned i;
    for (i = 0; i < 10; ++i) {
        output[i] = in[i] * scalar;
    }
}

/* Multiply two numbers: output = in2 * in
 *
 * output must be distinct to both inputs. The inputs are reduced coefficient
 * form, the output is not.
 *
 * output[x] <= 14 * the largest product of the input limbs.
static void fproduct(limb *output, const limb *in2, const limb *in) {
    output[0] =       ((limb) ((s32) in2[0])) * ((s32) in[0]);
    output[1] =       ((limb) ((s32) in2[0])) * ((s32) in[1]) +
    ((limb) ((s32) in2[1])) * ((s32) in[0]);
    output[2] =  2 *  ((limb) ((s32) in2[1])) * ((s32) in[1]) +
    ((limb) ((s32) in2[0])) * ((s32) in[2]) +
    ((limb) ((s32) in2[2])) * ((s32) in[0]);
    output[3] =       ((limb) ((s32) in2[1])) * ((s32) in[2]) +
    ((limb) ((s32) in2[2])) * ((s32) in[1]) +
    ((limb) ((s32) in2[0])) * ((s32) in[3]) +
    ((limb) ((s32) in2[3])) * ((s32) in[0]);
    output[4] =       ((limb) ((s32) in2[2])) * ((s32) in[2]) +
    2 * (((limb) ((s32) in2[1])) * ((s32) in[3]) +
         ((limb) ((s32) in2[3])) * ((s32) in[1])) +
    ((limb) ((s32) in2[0])) * ((s32) in[4]) +
    ((limb) ((s32) in2[4])) * ((s32) in[0]);
    output[5] =       ((limb) ((s32) in2[2])) * ((s32) in[3]) +
    ((limb) ((s32) in2[3])) * ((s32) in[2]) +
    ((limb) ((s32) in2[1])) * ((s32) in[4]) +
    ((limb) ((s32) in2[4])) * ((s32) in[1]) +
    ((limb) ((s32) in2[0])) * ((s32) in[5]) +
    ((limb) ((s32) in2[5])) * ((s32) in[0]);
    output[6] =  2 * (((limb) ((s32) in2[3])) * ((s32) in[3]) +
                      ((limb) ((s32) in2[1])) * ((s32) in[5]) +
                      ((limb) ((s32) in2[5])) * ((s32) in[1])) +
    ((limb) ((s32) in2[2])) * ((s32) in[4]) +
    ((limb) ((s32) in2[4])) * ((s32) in[2]) +
    ((limb) ((s32) in2[0])) * ((s32) in[6]) +
    ((limb) ((s32) in2[6])) * ((s32) in[0]);
    output[7] =       ((limb) ((s32) in2[3])) * ((s32) in[4]) +
    ((limb) ((s32) in2[4])) * ((s32) in[3]) +
    ((limb) ((s32) in2[2])) * ((s32) in[5]) +
    ((limb) ((s32) in2[5])) * ((s32) in[2]) +
    ((limb) ((s32) in2[1])) * ((s32) in[6]) +
    ((limb) ((s32) in2[6])) * ((s32) in[1]) +
    ((limb) ((s32) in2[0])) * ((s32) in[7]) +
    ((limb) ((s32) in2[7])) * ((s32) in[0]);
    output[8] =       ((limb) ((s32) in2[4])) * ((s32) in[4]) +
    2 * (((limb) ((s32) in2[3])) * ((s32) in[5]) +
         ((limb) ((s32) in2[5])) * ((s32) in[3]) +
         ((limb) ((s32) in2[1])) * ((s32) in[7]) +
         ((limb) ((s32) in2[7])) * ((s32) in[1])) +
    ((limb) ((s32) in2[2])) * ((s32) in[6]) +
    ((limb) ((s32) in2[6])) * ((s32) in[2]) +
    ((limb) ((s32) in2[0])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[0]);
    output[9] =       ((limb) ((s32) in2[4])) * ((s32) in[5]) +
    ((limb) ((s32) in2[5])) * ((s32) in[4]) +
    ((limb) ((s32) in2[3])) * ((s32) in[6]) +
    ((limb) ((s32) in2[6])) * ((s32) in[3]) +
    ((limb) ((s32) in2[2])) * ((s32) in[7]) +
    ((limb) ((s32) in2[7])) * ((s32) in[2]) +
    ((limb) ((s32) in2[1])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[1]) +
    ((limb) ((s32) in2[0])) * ((s32) in[9]) +
    ((limb) ((s32) in2[9])) * ((s32) in[0]);
    output[10] = 2 * (((limb) ((s32) in2[5])) * ((s32) in[5]) +
                      ((limb) ((s32) in2[3])) * ((s32) in[7]) +
                      ((limb) ((s32) in2[7])) * ((s32) in[3]) +
                      ((limb) ((s32) in2[1])) * ((s32) in[9]) +
                      ((limb) ((s32) in2[9])) * ((s32) in[1])) +
    ((limb) ((s32) in2[4])) * ((s32) in[6]) +
    ((limb) ((s32) in2[6])) * ((s32) in[4]) +
    ((limb) ((s32) in2[2])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[2]);
    output[11] =      ((limb) ((s32) in2[5])) * ((s32) in[6]) +
    ((limb) ((s32) in2[6])) * ((s32) in[5]) +
    ((limb) ((s32) in2[4])) * ((s32) in[7]) +
    ((limb) ((s32) in2[7])) * ((s32) in[4]) +
    ((limb) ((s32) in2[3])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[3]) +
    ((limb) ((s32) in2[2])) * ((s32) in[9]) +
    ((limb) ((s32) in2[9])) * ((s32) in[2]);
    output[12] =      ((limb) ((s32) in2[6])) * ((s32) in[6]) +
    2 * (((limb) ((s32) in2[5])) * ((s32) in[7]) +
         ((limb) ((s32) in2[7])) * ((s32) in[5]) +
         ((limb) ((s32) in2[3])) * ((s32) in[9]) +
         ((limb) ((s32) in2[9])) * ((s32) in[3])) +
    ((limb) ((s32) in2[4])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[4]);
    output[13] =      ((limb) ((s32) in2[6])) * ((s32) in[7]) +
    ((limb) ((s32) in2[7])) * ((s32) in[6]) +
    ((limb) ((s32) in2[5])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[5]) +
    ((limb) ((s32) in2[4])) * ((s32) in[9]) +
    ((limb) ((s32) in2[9])) * ((s32) in[4]);
    output[14] = 2 * (((limb) ((s32) in2[7])) * ((s32) in[7]) +
                      ((limb) ((s32) in2[5])) * ((s32) in[9]) +
                      ((limb) ((s32) in2[9])) * ((s32) in[5])) +
    ((limb) ((s32) in2[6])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[6]);
    output[15] =      ((limb) ((s32) in2[7])) * ((s32) in[8]) +
    ((limb) ((s32) in2[8])) * ((s32) in[7]) +
    ((limb) ((s32) in2[6])) * ((s32) in[9]) +
    ((limb) ((s32) in2[9])) * ((s32) in[6]);
    output[16] =      ((limb) ((s32) in2[8])) * ((s32) in[8]) +
    2 * (((limb) ((s32) in2[7])) * ((s32) in[9]) +
         ((limb) ((s32) in2[9])) * ((s32) in[7]));
    output[17] =      ((limb) ((s32) in2[8])) * ((s32) in[9]) +
    ((limb) ((s32) in2[9])) * ((s32) in[8]);
    output[18] = 2 *  ((limb) ((s32) in2[9])) * ((s32) in[9]);
}*/

/* Reduce a long form to a short form by taking the input mod 2^255 - 19.
 *
 * On entry: |output[i]| < 14*2^54
 * On exit: |output[0..8]| < 280*2^54 */
static void freduce_degree(limb *output) {
    /* Each of these shifts and adds ends up multiplying the value by 19.
     *
     * For output[0..8], the absolute entry value is < 14*2^54 and we add, at
     * most, 19*14*2^54 thus, on exit, |output[0..8]| < 280*2^54. */
    output[8] += output[18] << 4;
    output[8] += output[18] << 1;
    output[8] += output[18];
    output[7] += output[17] << 4;
    output[7] += output[17] << 1;
    output[7] += output[17];
    output[6] += output[16] << 4;
    output[6] += output[16] << 1;
    output[6] += output[16];
    output[5] += output[15] << 4;
    output[5] += output[15] << 1;
    output[5] += output[15];
    output[4] += output[14] << 4;
    output[4] += output[14] << 1;
    output[4] += output[14];
    output[3] += output[13] << 4;
    output[3] += output[13] << 1;
    output[3] += output[13];
    output[2] += output[12] << 4;
    output[2] += output[12] << 1;
    output[2] += output[12];
    output[1] += output[11] << 4;
    output[1] += output[11] << 1;
    output[1] += output[11];
    output[0] += output[10] << 4;
    output[0] += output[10] << 1;
    output[0] += output[10];
}

#if (-1 & 3) != 3
#error "This code only works on a two's complement system"
#endif

/* return v / 2^26, using only shifts and adds.
 *
 * On entry: v can take any value. */
static inline limb
div_by_2_26(const limb v)
{
    /* High word of v; no shift needed. */
    const uint32_t highword = (uint32_t) (((uint64_t) v) >> 32);
    /* Set to all 1s if v was negative; else set to 0s. */
    const int32_t sign = ((int32_t) highword) >> 31;
    /* Set to 0x3ffffff if v was negative; else set to 0. */
    const int32_t roundoff = ((uint32_t) sign) >> 6;
    /* Should return v / (1<<26) */
    return (v + roundoff) >> 26;
}

/* return v / (2^25), using only shifts and adds.
 *
 * On entry: v can take any value. */
static inline limb
div_by_2_25(const limb v)
{
    /* High word of v; no shift needed*/
    const uint32_t highword = (uint32_t) (((uint64_t) v) >> 32);
    /* Set to all 1s if v was negative; else set to 0s. */
    const int32_t sign = ((int32_t) highword) >> 31;
    /* Set to 0x1ffffff if v was negative; else set to 0. */
    const int32_t roundoff = ((uint32_t) sign) >> 7;
    /* Should return v / (1<<25) */
    return (v + roundoff) >> 25;
}

/* Reduce all coefficients of the short form input so that |x| < 2^26.
 *
 * On entry: |output[i]| < 280*2^54 */
static void freduce_coefficients(limb *output) {
    unsigned i;
    
    output[10] = 0;
    
    for (i = 0; i < 10; i += 2) {
        limb over = div_by_2_26(output[i]);
        /* The entry condition (that |output[i]| < 280*2^54) means that over is, at
         * most, 280*2^28 in the first iteration of this loop. This is added to the
         * next limb and we can approximate the resulting bound of that limb by
         * 281*2^54. */
        output[i] -= over << 26;
        output[i+1] += over;
        
        /* For the first iteration, |output[i+1]| < 281*2^54, thus |over| <
         * 281*2^29. When this is added to the next limb, the resulting bound can
         * be approximated as 281*2^54.
         *
         * For subsequent iterations of the loop, 281*2^54 remains a conservative
         * bound and no overflow occurs. */
        over = div_by_2_25(output[i+1]);
        output[i+1] -= over << 25;
        output[i+2] += over;
    }
    /* Now |output[10]| < 281*2^29 and all other coefficients are reduced. */
    output[0] += output[10] << 4;
    output[0] += output[10] << 1;
    output[0] += output[10];
    
    output[10] = 0;
    
    /* Now output[1..9] are reduced, and |output[0]| < 2^26 + 19*281*2^29
     * So |over| will be no more than 2^16. */
    {
        limb over = div_by_2_26(output[0]);
        output[0] -= over << 26;
        output[1] += over;
    }
    
    /* Now output[0,2..9] are reduced, and |output[1]| < 2^25 + 2^16 < 2^26. The
     * bound on |output[1]| is sufficient to meet our needs. */
}

/* A helpful wrapper around fproduct: output = in * in2.
 *
 * On entry: |in[i]| < 2^27 and |in2[i]| < 2^27.
 *
 * output must be distinct to both inputs. The output is reduced degree
 * (indeed, one need only provide storage for 10 limbs) and |output[i]| < 2^26.
static void fmul32(limb *output, const limb *in, const limb *in2)
{
    limb t[19];
    fproduct(t, in, in2);
    //|t[i]| < 14*2^54
    freduce_degree(t);
    freduce_coefficients(t);
    // |t[i]| < 2^26
    memcpy(output, t, sizeof(limb) * 10);
}*/

/* Square a number: output = in**2
 *
 * output must be distinct from the input. The inputs are reduced coefficient
 * form, the output is not.
 *
 * output[x] <= 14 * the largest product of the input limbs. */
static void fsquare_inner(limb *output, const limb *in) {
    output[0] =       ((limb) ((s32) in[0])) * ((s32) in[0]);
    output[1] =  2 *  ((limb) ((s32) in[0])) * ((s32) in[1]);
    output[2] =  2 * (((limb) ((s32) in[1])) * ((s32) in[1]) +
                      ((limb) ((s32) in[0])) * ((s32) in[2]));
    output[3] =  2 * (((limb) ((s32) in[1])) * ((s32) in[2]) +
                      ((limb) ((s32) in[0])) * ((s32) in[3]));
    output[4] =       ((limb) ((s32) in[2])) * ((s32) in[2]) +
    4 *  ((limb) ((s32) in[1])) * ((s32) in[3]) +
    2 *  ((limb) ((s32) in[0])) * ((s32) in[4]);
    output[5] =  2 * (((limb) ((s32) in[2])) * ((s32) in[3]) +
                      ((limb) ((s32) in[1])) * ((s32) in[4]) +
                      ((limb) ((s32) in[0])) * ((s32) in[5]));
    output[6] =  2 * (((limb) ((s32) in[3])) * ((s32) in[3]) +
                      ((limb) ((s32) in[2])) * ((s32) in[4]) +
                      ((limb) ((s32) in[0])) * ((s32) in[6]) +
                      2 *  ((limb) ((s32) in[1])) * ((s32) in[5]));
    output[7] =  2 * (((limb) ((s32) in[3])) * ((s32) in[4]) +
                      ((limb) ((s32) in[2])) * ((s32) in[5]) +
                      ((limb) ((s32) in[1])) * ((s32) in[6]) +
                      ((limb) ((s32) in[0])) * ((s32) in[7]));
    output[8] =       ((limb) ((s32) in[4])) * ((s32) in[4]) +
    2 * (((limb) ((s32) in[2])) * ((s32) in[6]) +
         ((limb) ((s32) in[0])) * ((s32) in[8]) +
         2 * (((limb) ((s32) in[1])) * ((s32) in[7]) +
              ((limb) ((s32) in[3])) * ((s32) in[5])));
    output[9] =  2 * (((limb) ((s32) in[4])) * ((s32) in[5]) +
                      ((limb) ((s32) in[3])) * ((s32) in[6]) +
                      ((limb) ((s32) in[2])) * ((s32) in[7]) +
                      ((limb) ((s32) in[1])) * ((s32) in[8]) +
                      ((limb) ((s32) in[0])) * ((s32) in[9]));
    output[10] = 2 * (((limb) ((s32) in[5])) * ((s32) in[5]) +
                      ((limb) ((s32) in[4])) * ((s32) in[6]) +
                      ((limb) ((s32) in[2])) * ((s32) in[8]) +
                      2 * (((limb) ((s32) in[3])) * ((s32) in[7]) +
                           ((limb) ((s32) in[1])) * ((s32) in[9])));
    output[11] = 2 * (((limb) ((s32) in[5])) * ((s32) in[6]) +
                      ((limb) ((s32) in[4])) * ((s32) in[7]) +
                      ((limb) ((s32) in[3])) * ((s32) in[8]) +
                      ((limb) ((s32) in[2])) * ((s32) in[9]));
    output[12] =      ((limb) ((s32) in[6])) * ((s32) in[6]) +
    2 * (((limb) ((s32) in[4])) * ((s32) in[8]) +
         2 * (((limb) ((s32) in[5])) * ((s32) in[7]) +
              ((limb) ((s32) in[3])) * ((s32) in[9])));
    output[13] = 2 * (((limb) ((s32) in[6])) * ((s32) in[7]) +
                      ((limb) ((s32) in[5])) * ((s32) in[8]) +
                      ((limb) ((s32) in[4])) * ((s32) in[9]));
    output[14] = 2 * (((limb) ((s32) in[7])) * ((s32) in[7]) +
                      ((limb) ((s32) in[6])) * ((s32) in[8]) +
                      2 *  ((limb) ((s32) in[5])) * ((s32) in[9]));
    output[15] = 2 * (((limb) ((s32) in[7])) * ((s32) in[8]) +
                      ((limb) ((s32) in[6])) * ((s32) in[9]));
    output[16] =      ((limb) ((s32) in[8])) * ((s32) in[8]) +
    4 *  ((limb) ((s32) in[7])) * ((s32) in[9]);
    output[17] = 2 *  ((limb) ((s32) in[8])) * ((s32) in[9]);
    output[18] = 2 *  ((limb) ((s32) in[9])) * ((s32) in[9]);
}

/* fsquare sets output = in^2.
 *
 * On entry: The |in| argument is in reduced coefficients form and |in[i]| <
 * 2^27.
 *
 * On exit: The |output| argument is in reduced coefficients form (indeed, one
 * need only provide storage for 10 limbs) and |out[i]| < 2^26. */
static void
fsquare32(limb *output, const limb *in) {
    limb t[19];
    fsquare_inner(t, in);
    /* |t[i]| < 14*2^54 because the largest product of two limbs will be <
     * 2^(27+27) and fsquare_inner adds together, at most, 14 of those
     * products. */
    freduce_degree(t);
    freduce_coefficients(t);
    /* |t[i]| < 2^26 */
    memcpy(output, t, sizeof(limb) * 10);
}

#if (-32 >> 1) != -16
#error "This code only works when >> does sign-extension on negative numbers"
#endif

/* s32_eq returns 0xffffffff iff a == b and zero otherwise. */
static s32 s32_eq(s32 a, s32 b) {
    a = ~(a ^ b);
    a &= a << 16;
    a &= a << 8;
    a &= a << 4;
    a &= a << 2;
    a &= a << 1;
    return a >> 31;
}

/* s32_gte returns 0xffffffff if a >= b and zero otherwise, where a and b are
 * both non-negative. */
static s32 s32_gte(s32 a, s32 b) {
    a -= b;
    /* a >= 0 iff a >= b. */
    return ~(a >> 31);
}

/* Take a fully reduced polynomial form number and contract it into a
 * little-endian, 32-byte array.
 *
 * On entry: |input_limbs[i]| < 2^26 */
static void fcontract32(u8 *output, limb *input_limbs)
{
    int i;
    int j;
    s32 input[10];
    s32 mask;
    
    /* |input_limbs[i]| < 2^26, so it's valid to convert to an s32. */
    for (i = 0; i < 10; i++)
        input[i] = (s32)input_limbs[i];
    
    for (j = 0; j < 2; ++j) {
        for (i = 0; i < 9; ++i) {
            if ((i & 1) == 1) {
                /* This calculation is a time-invariant way to make input[i]
                 * non-negative by borrowing from the next-larger limb. */
                const s32 mask = input[i] >> 31;
                const s32 carry = -((input[i] & mask) >> 25);
                input[i] = input[i] + (carry << 25);
                input[i+1] = input[i+1] - carry;
            } else {
                const s32 mask = input[i] >> 31;
                const s32 carry = -((input[i] & mask) >> 26);
                input[i] = input[i] + (carry << 26);
                input[i+1] = input[i+1] - carry;
            }
        }
        
        /* There's no greater limb for input[9] to borrow from, but we can multiply
         * by 19 and borrow from input[0], which is valid mod 2^255-19. */
        {
            const s32 mask = input[9] >> 31;
            const s32 carry = -((input[9] & mask) >> 25);
            input[9] = input[9] + (carry << 25);
            input[0] = input[0] - (carry * 19);
        }
        
        /* After the first iteration, input[1..9] are non-negative and fit within
         * 25 or 26 bits, depending on position. However, input[0] may be
         * negative. */
    }
    
    /* The first borrow-propagation pass above ended with every limb
     except (possibly) input[0] non-negative.
     
     If input[0] was negative after the first pass, then it was because of a
     carry from input[9]. On entry, input[9] < 2^26 so the carry was, at most,
     one, since (2**26-1) >> 25 = 1. Thus input[0] >= -19.
     
     In the second pass, each limb is decreased by at most one. Thus the second
     borrow-propagation pass could only have wrapped around to decrease
     input[0] again if the first pass left input[0] negative *and* input[1]
     through input[9] were all zero.  In that case, input[1] is now 2^25 - 1,
     and this last borrow-propagation step will leave input[1] non-negative. */
    {
        const s32 mask = input[0] >> 31;
        const s32 carry = -((input[0] & mask) >> 26);
        input[0] = input[0] + (carry << 26);
        input[1] = input[1] - carry;
    }
    
    /* All input[i] are now non-negative. However, there might be values between
     * 2^25 and 2^26 in a limb which is, nominally, 25 bits wide. */
    for (j = 0; j < 2; j++) {
        for (i = 0; i < 9; i++) {
            if ((i & 1) == 1) {
                const s32 carry = input[i] >> 25;
                input[i] &= 0x1ffffff;
                input[i+1] += carry;
            } else {
                const s32 carry = input[i] >> 26;
                input[i] &= 0x3ffffff;
                input[i+1] += carry;
            }
        }
        
        {
            const s32 carry = input[9] >> 25;
            input[9] &= 0x1ffffff;
            input[0] += 19*carry;
        }
    }
    
    /* If the first carry-chain pass, just above, ended up with a carry from
     * input[9], and that caused input[0] to be out-of-bounds, then input[0] was
     * < 2^26 + 2*19, because the carry was, at most, two.
     *
     * If the second pass carried from input[9] again then input[0] is < 2*19 and
     * the input[9] -> input[0] carry didn't push input[0] out of bounds. */
    
    /* It still remains the case that input might be between 2^255-19 and 2^255.
     * In this case, input[1..9] must take their maximum value and input[0] must
     * be >= (2^255-19) & 0x3ffffff, which is 0x3ffffed. */
    mask = s32_gte(input[0], 0x3ffffed);
    for (i = 1; i < 10; i++) {
        if ((i & 1) == 1) {
            mask &= s32_eq(input[i], 0x1ffffff);
        } else {
            mask &= s32_eq(input[i], 0x3ffffff);
        }
    }
    
    /* mask is either 0xffffffff (if input >= 2^255-19) and zero otherwise. Thus
     * this conditionally subtracts 2^255-19. */
    input[0] -= mask & 0x3ffffed;
    
    for (i = 1; i < 10; i++) {
        if ((i & 1) == 1) {
            input[i] -= mask & 0x1ffffff;
        } else {
            input[i] -= mask & 0x3ffffff;
        }
    }
    
    input[1] <<= 2;
    input[2] <<= 3;
    input[3] <<= 5;
    input[4] <<= 6;
    input[6] <<= 1;
    input[7] <<= 3;
    input[8] <<= 4;
    input[9] <<= 6;
#define F(i, s) \
output[s+0] |=  input[i] & 0xff; \
output[s+1]  = (input[i] >> 8) & 0xff; \
output[s+2]  = (input[i] >> 16) & 0xff; \
output[s+3]  = (input[i] >> 24) & 0xff;
    output[0] = 0;
    output[16] = 0;
    F(0,0);
    F(1,3);
    F(2,6);
    F(3,9);
    F(4,12);
    F(5,16);
    F(6,19);
    F(7,22);
    F(8,25);
    F(9,28);
#undef F
}

bits320 bits320_limbs(limb limbs[10])
{
    bits320 output; int32_t i;
    for (i=0; i<10; i++)
        output.uints[i] = limbs[i];
    return(output);
}

static inline bits320 fscalar_product(const bits320 in,const uint64_t scalar)
{
    limb output[10],input[10]; int32_t i;
    for (i=0; i<10; i++)
        input[i] = in.uints[i];
    fscalar_product32(output,input,scalar);
    return(bits320_limbs(output));
}

static inline bits320 fsquare_times(const bits320 in,uint64_t count)
{
    limb output[10],input[10]; int32_t i;
    for (i=0; i<10; i++)
        input[i] = in.uints[i];
    for (i=0; i<count; i++)
    {
        fsquare32(output,input);
        memcpy(input,output,sizeof(input));
    }
    return(bits320_limbs(output));
}

bits256 fmul_donna(bits256 a,bits256 b);
bits256 crecip_donna(bits256 a);

bits256 fcontract(const bits320 in)
{
    bits256 contracted; limb input[10]; int32_t i;
    for (i=0; i<10; i++)
        input[i] = in.uints[i];
    fcontract32(contracted.bytes,input);
    return(contracted);
}

bits320 fmul(const bits320 in,const bits320 in2)
{
    /*limb output[11],input[10],input2[10]; int32_t i;
    for (i=0; i<10; i++)
    {
        input[i] = in.uints[i];
        input2[i] = in2.uints[i];
    }
    fmul32(output,input,input2);
    return(bits320_limbs(output));*/
    bits256 mulval;
    mulval = fmul_donna(fcontract(in),fcontract(in2));
    return(fexpand(mulval));
}

bits256 curve25519(bits256 mysecret,bits256 theirpublic)
{
    int32_t curve25519_donna(uint8_t *mypublic,const uint8_t *secret,const uint8_t *basepoint);
    bits256 rawkey;
    mysecret.bytes[0] &= 0xf8, mysecret.bytes[31] &= 0x7f, mysecret.bytes[31] |= 0x40;
    curve25519_donna(&rawkey.bytes[0],&mysecret.bytes[0],&theirpublic.bytes[0]);
    return(rawkey);
}

#endif


// Input: Q, Q', Q-Q' -> Output: 2Q, Q+Q'
// x2 z2: long form && x3 z3: long form
// x z: short form, destroyed && xprime zprime: short form, destroyed
// qmqp: short form, preserved
static inline void
fmonty(bits320 *x2, bits320 *z2, // output 2Q
       bits320 *x3, bits320 *z3, // output Q + Q'
       bits320 *x, bits320 *z,   // input Q
       bits320 *xprime, bits320 *zprime, // input Q'
       const bits320 qmqp) // input Q - Q'
{
    bits320 origx,origxprime,zzz,xx,zz,xxprime,zzprime;
    origx = *x;
    *x = fsum(*x,*z), fdifference_backwards(z->ulongs,origx.ulongs);  // does x - z
    origxprime = *xprime;
    *xprime = fsum(*xprime,*zprime), fdifference_backwards(zprime->ulongs,origxprime.ulongs);
    xxprime = fmul(*xprime,*z), zzprime = fmul(*x,*zprime);
    origxprime = xxprime;
    xxprime = fsum(xxprime,zzprime), fdifference_backwards(zzprime.ulongs,origxprime.ulongs);
    *x3 = fsquare_times(xxprime,1), *z3 = fmul(fsquare_times(zzprime,1),qmqp);
    xx = fsquare_times(*x,1), zz = fsquare_times(*z,1);
    *x2 = fmul(xx,zz);
    fdifference_backwards(zz.ulongs,xx.ulongs);  // does zz = xx - zz
    zzz = fscalar_product(zz,121665);
    *z2 = fmul(zz,fsum(zzz,xx));
}

// -----------------------------------------------------------------------------
// Maybe swap the contents of two limb arrays (@a and @b), each @len elements
// long. Perform the swap iff @swap is non-zero.
// This function performs the swap without leaking any side-channel information.
// -----------------------------------------------------------------------------
static inline void swap_conditional(bits320 *a,bits320 *b,uint64_t iswap)
{
    int32_t i; const uint64_t swap = -iswap;
    for (i=0; i<5; ++i)
    {
        const uint64_t x = swap & (a->ulongs[i] ^ b->ulongs[i]);
        a->ulongs[i] ^= x, b->ulongs[i] ^= x;
    }
}

// Calculates nQ where Q is the x-coordinate of a point on the curve
// resultx/resultz: the x coordinate of the resulting curve point (short form)
// n: a little endian, 32-byte number
// q: a point of the curve (short form)
void cmult(bits320 *resultx,bits320 *resultz,bits256 secret,const bits320 q)
{
    int32_t i,j; bits320 a,b,c,d,e,f,g,h,*t;
    bits320 Zero320bits,One320bits, *nqpqx = &a,*nqpqz = &b,*nqx = &c,*nqz = &d,*nqpqx2 = &e,*nqpqz2 = &f,*nqx2 = &g,*nqz2 = &h;
    memset(&Zero320bits,0,sizeof(Zero320bits));
    memset(&One320bits,0,sizeof(One320bits)), One320bits.ulongs[0] = 1;
    a = d = e = g = Zero320bits, b = c = f = h = One320bits;
    *nqpqx = q;
    for (i=0; i<32; i++)
    {
        uint8_t byte = secret.bytes[31 - i];
        for (j=0; j<8; j++)
        {
            const uint64_t bit = byte >> 7;
            swap_conditional(nqx,nqpqx,bit), swap_conditional(nqz,nqpqz,bit);
            fmonty(nqx2,nqz2,nqpqx2,nqpqz2,nqx,nqz,nqpqx,nqpqz,q);
            swap_conditional(nqx2,nqpqx2,bit), swap_conditional(nqz2,nqpqz2,bit);
            t = nqx, nqx = nqx2, nqx2 = t;
            t = nqz, nqz = nqz2, nqz2 = t;
            t = nqpqx, nqpqx = nqpqx2, nqpqx2 = t;
            t = nqpqz, nqpqz = nqpqz2, nqpqz2 = t;
            byte <<= 1;
        }
    }
    *resultx = *nqx, *resultz = *nqz;
}

// Shamelessly copied from donna's code that copied djb's code, changed a little
inline bits320 crecip(const bits320 z)
{
    bits320 a,t0,b,c;
    /* 2 */ a = fsquare_times(z, 1); // a = 2
    /* 8 */ t0 = fsquare_times(a, 2);
    /* 9 */ b = fmul(t0, z); // b = 9
    /* 11 */ a = fmul(b, a); // a = 11
    /* 22 */ t0 = fsquare_times(a, 1);
    /* 2^5 - 2^0 = 31 */ b = fmul(t0, b);
    /* 2^10 - 2^5 */ t0 = fsquare_times(b, 5);
    /* 2^10 - 2^0 */ b = fmul(t0, b);
    /* 2^20 - 2^10 */ t0 = fsquare_times(b, 10);
    /* 2^20 - 2^0 */ c = fmul(t0, b);
    /* 2^40 - 2^20 */ t0 = fsquare_times(c, 20);
    /* 2^40 - 2^0 */ t0 = fmul(t0, c);
    /* 2^50 - 2^10 */ t0 = fsquare_times(t0, 10);
    /* 2^50 - 2^0 */ b = fmul(t0, b);
    /* 2^100 - 2^50 */ t0 = fsquare_times(b, 50);
    /* 2^100 - 2^0 */ c = fmul(t0, b);
    /* 2^200 - 2^100 */ t0 = fsquare_times(c, 100);
    /* 2^200 - 2^0 */ t0 = fmul(t0, c);
    /* 2^250 - 2^50 */ t0 = fsquare_times(t0, 50);
    /* 2^250 - 2^0 */ t0 = fmul(t0, b);
    /* 2^255 - 2^5 */ t0 = fsquare_times(t0, 5);
    /* 2^255 - 21 */ return(fmul(t0, a));
}

void OS_randombytes(unsigned char *x,long xlen);

bits256 rand256(int32_t privkeyflag)
{
    bits256 randval;
    OS_randombytes(randval.bytes,sizeof(randval));
    if ( privkeyflag != 0 )
        randval.bytes[0] &= 0xf8, randval.bytes[31] &= 0x7f, randval.bytes[31] |= 0x40;
    return(randval);
}

bits256 curve25519_basepoint9()
{
    bits256 basepoint;
    memset(&basepoint,0,sizeof(basepoint));
    basepoint.bytes[0] = 9;
    return(basepoint);
}

bits256 curve25519_keypair(bits256 *pubkeyp)
{
    bits256 privkey;
    privkey = rand256(1);
    *pubkeyp = curve25519(privkey,curve25519_basepoint9());
    //printf("[%llx %llx] ",privkey.txid,(*pubkeyp).txid);
    return(privkey);
}

// following is ported from libtom

#define STORE32L(x, y)                                                                     \
{ (y)[3] = (uint8_t)(((x)>>24)&255); (y)[2] = (uint8_t)(((x)>>16)&255);   \
(y)[1] = (uint8_t)(((x)>>8)&255); (y)[0] = (uint8_t)((x)&255); }

#define LOAD32L(x, y)                            \
{ x = (uint32_t)(((uint64_t)((y)[3] & 255)<<24) | \
((uint32_t)((y)[2] & 255)<<16) | \
((uint32_t)((y)[1] & 255)<<8)  | \
((uint32_t)((y)[0] & 255))); }

#define STORE64L(x, y)                                                                     \
{ (y)[7] = (uint8_t)(((x)>>56)&255); (y)[6] = (uint8_t)(((x)>>48)&255);   \
(y)[5] = (uint8_t)(((x)>>40)&255); (y)[4] = (uint8_t)(((x)>>32)&255);   \
(y)[3] = (uint8_t)(((x)>>24)&255); (y)[2] = (uint8_t)(((x)>>16)&255);   \
(y)[1] = (uint8_t)(((x)>>8)&255); (y)[0] = (uint8_t)((x)&255); }

#define LOAD64L(x, y)                                                       \
{ x = (((uint64_t)((y)[7] & 255))<<56)|(((uint64_t)((y)[6] & 255))<<48)| \
(((uint64_t)((y)[5] & 255))<<40)|(((uint64_t)((y)[4] & 255))<<32)| \
(((uint64_t)((y)[3] & 255))<<24)|(((uint64_t)((y)[2] & 255))<<16)| \
(((uint64_t)((y)[1] & 255))<<8)|(((uint64_t)((y)[0] & 255))); }

#define STORE32H(x, y)                                                                     \
{ (y)[0] = (uint8_t)(((x)>>24)&255); (y)[1] = (uint8_t)(((x)>>16)&255);   \
(y)[2] = (uint8_t)(((x)>>8)&255); (y)[3] = (uint8_t)((x)&255); }

#define LOAD32H(x, y)                            \
{ x = (uint32_t)(((uint64_t)((y)[0] & 255)<<24) | \
((uint32_t)((y)[1] & 255)<<16) | \
((uint32_t)((y)[2] & 255)<<8)  | \
((uint32_t)((y)[3] & 255))); }

#define STORE64H(x, y)                                                                     \
{ (y)[0] = (uint8_t)(((x)>>56)&255); (y)[1] = (uint8_t)(((x)>>48)&255);     \
(y)[2] = (uint8_t)(((x)>>40)&255); (y)[3] = (uint8_t)(((x)>>32)&255);     \
(y)[4] = (uint8_t)(((x)>>24)&255); (y)[5] = (uint8_t)(((x)>>16)&255);     \
(y)[6] = (uint8_t)(((x)>>8)&255); (y)[7] = (uint8_t)((x)&255); }

#define LOAD64H(x, y)                                                      \
{ x = (((uint64_t)((y)[0] & 255))<<56)|(((uint64_t)((y)[1] & 255))<<48) | \
(((uint64_t)((y)[2] & 255))<<40)|(((uint64_t)((y)[3] & 255))<<32) | \
(((uint64_t)((y)[4] & 255))<<24)|(((uint64_t)((y)[5] & 255))<<16) | \
(((uint64_t)((y)[6] & 255))<<8)|(((uint64_t)((y)[7] & 255))); }

// Various logical functions
#define RORc(x, y) ( ((((uint32_t)(x)&0xFFFFFFFFUL)>>(uint32_t)((y)&31)) | ((uint32_t)(x)<<(uint32_t)(32-((y)&31)))) & 0xFFFFFFFFUL)
#define Ch(x,y,z)       (z ^ (x & (y ^ z)))
#define Maj(x,y,z)      (((x | y) & z) | (x & y))
#define S(x, n)         RORc((x),(n))
#define R(x, n)         (((x)&0xFFFFFFFFUL)>>(n))
#define Sigma0(x)       (S(x, 2) ^ S(x, 13) ^ S(x, 22))
#define Sigma1(x)       (S(x, 6) ^ S(x, 11) ^ S(x, 25))
#define Gamma0(x)       (S(x, 7) ^ S(x, 18) ^ R(x, 3))
#define Gamma1(x)       (S(x, 17) ^ S(x, 19) ^ R(x, 10))
#define MIN(x, y) ( ((x)<(y))?(x):(y) )

static inline int32_t sha256_vcompress(struct sha256_vstate * md,uint8_t *buf)
{
    uint32_t S[8],W[64],t0,t1,i;
    for (i=0; i<8; i++) // copy state into S
        S[i] = md->state[i];
    for (i=0; i<16; i++) // copy the state into 512-bits into W[0..15]
        LOAD32H(W[i],buf + (4*i));
    for (i=16; i<64; i++) // fill W[16..63]
        W[i] = Gamma1(W[i - 2]) + W[i - 7] + Gamma0(W[i - 15]) + W[i - 16];
    
#define RND(a,b,c,d,e,f,g,h,i,ki)                    \
t0 = h + Sigma1(e) + Ch(e, f, g) + ki + W[i];   \
t1 = Sigma0(a) + Maj(a, b, c);                  \
d += t0;                                        \
h  = t0 + t1;
    
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],0,0x428a2f98);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],1,0x71374491);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],2,0xb5c0fbcf);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],3,0xe9b5dba5);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],4,0x3956c25b);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],5,0x59f111f1);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],6,0x923f82a4);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],7,0xab1c5ed5);
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],8,0xd807aa98);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],9,0x12835b01);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],10,0x243185be);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],11,0x550c7dc3);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],12,0x72be5d74);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],13,0x80deb1fe);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],14,0x9bdc06a7);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],15,0xc19bf174);
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],16,0xe49b69c1);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],17,0xefbe4786);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],18,0x0fc19dc6);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],19,0x240ca1cc);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],20,0x2de92c6f);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],21,0x4a7484aa);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],22,0x5cb0a9dc);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],23,0x76f988da);
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],24,0x983e5152);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],25,0xa831c66d);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],26,0xb00327c8);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],27,0xbf597fc7);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],28,0xc6e00bf3);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],29,0xd5a79147);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],30,0x06ca6351);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],31,0x14292967);
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],32,0x27b70a85);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],33,0x2e1b2138);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],34,0x4d2c6dfc);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],35,0x53380d13);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],36,0x650a7354);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],37,0x766a0abb);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],38,0x81c2c92e);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],39,0x92722c85);
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],40,0xa2bfe8a1);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],41,0xa81a664b);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],42,0xc24b8b70);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],43,0xc76c51a3);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],44,0xd192e819);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],45,0xd6990624);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],46,0xf40e3585);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],47,0x106aa070);
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],48,0x19a4c116);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],49,0x1e376c08);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],50,0x2748774c);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],51,0x34b0bcb5);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],52,0x391c0cb3);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],53,0x4ed8aa4a);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],54,0x5b9cca4f);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],55,0x682e6ff3);
    RND(S[0],S[1],S[2],S[3],S[4],S[5],S[6],S[7],56,0x748f82ee);
    RND(S[7],S[0],S[1],S[2],S[3],S[4],S[5],S[6],57,0x78a5636f);
    RND(S[6],S[7],S[0],S[1],S[2],S[3],S[4],S[5],58,0x84c87814);
    RND(S[5],S[6],S[7],S[0],S[1],S[2],S[3],S[4],59,0x8cc70208);
    RND(S[4],S[5],S[6],S[7],S[0],S[1],S[2],S[3],60,0x90befffa);
    RND(S[3],S[4],S[5],S[6],S[7],S[0],S[1],S[2],61,0xa4506ceb);
    RND(S[2],S[3],S[4],S[5],S[6],S[7],S[0],S[1],62,0xbef9a3f7);
    RND(S[1],S[2],S[3],S[4],S[5],S[6],S[7],S[0],63,0xc67178f2);
#undef RND
    for (i=0; i<8; i++) // feedback
        md->state[i] = md->state[i] + S[i];
    return(0);
}

#undef RORc
#undef Ch
#undef Maj
#undef S
#undef R
#undef Sigma0
#undef Sigma1
#undef Gamma0
#undef Gamma1

static inline void sha256_vinit(struct sha256_vstate * md)
{
    md->curlen = 0;
    md->length = 0;
    md->state[0] = 0x6A09E667UL;
    md->state[1] = 0xBB67AE85UL;
    md->state[2] = 0x3C6EF372UL;
    md->state[3] = 0xA54FF53AUL;
    md->state[4] = 0x510E527FUL;
    md->state[5] = 0x9B05688CUL;
    md->state[6] = 0x1F83D9ABUL;
    md->state[7] = 0x5BE0CD19UL;
}

static inline int32_t sha256_vprocess(struct sha256_vstate *md,const uint8_t *in,uint64_t inlen)
{
    uint64_t n; int32_t err;
    if ( md->curlen > sizeof(md->buf) )
        return(-1);
    while ( inlen > 0 )
    {
        if ( md->curlen == 0 && inlen >= 64 )
        {
            if ( (err= sha256_vcompress(md,(uint8_t *)in)) != 0 )
                return(err);
            md->length += 64 * 8, in += 64, inlen -= 64;
        }
        else
        {
            n = MIN(inlen,64 - md->curlen);
            memcpy(md->buf + md->curlen,in,(size_t)n);
            md->curlen += n, in += n, inlen -= n;
            if ( md->curlen == 64 )
            {
                if ( (err= sha256_vcompress(md,md->buf)) != 0 )
                    return(err);
                md->length += 8*64;
                md->curlen = 0;
            }
        }
    }
    return(0);
}

static inline int32_t sha256_vdone(struct sha256_vstate *md,uint8_t *out)
{
    int32_t i;
    if ( md->curlen >= sizeof(md->buf) )
        return(-1);
    md->length += md->curlen * 8; // increase the length of the message
    md->buf[md->curlen++] = (uint8_t)0x80; // append the '1' bit
    // if len > 56 bytes we append zeros then compress.  Then we can fall back to padding zeros and length encoding like normal.
    if ( md->curlen > 56 )
    {
        while ( md->curlen < 64 )
            md->buf[md->curlen++] = (uint8_t)0;
        sha256_vcompress(md,md->buf);
        md->curlen = 0;
    }
    while ( md->curlen < 56 ) // pad upto 56 bytes of zeroes
        md->buf[md->curlen++] = (uint8_t)0;
    STORE64H(md->length,md->buf+56); // store length
    sha256_vcompress(md,md->buf);
    for (i=0; i<8; i++) // copy output
        STORE32H(md->state[i],out+(4*i));
    return(0);
}

int32_t init_hexbytes_noT(char *hexbytes,uint8_t *message,long len);

void vcalc_sha256(char hashstr[(256 >> 3) * 2 + 1],uint8_t hash[256 >> 3],uint8_t *src,int32_t len)
{
    struct sha256_vstate md;
    sha256_vinit(&md);
    sha256_vprocess(&md,src,len);
    sha256_vdone(&md,hash);
    if ( hashstr != 0 )
        init_hexbytes_noT(hashstr,hash,256 >> 3);
}

void vcalc_sha256cat(uint8_t hash[256 >> 3],uint8_t *src,int32_t len,uint8_t *src2,int32_t len2)
{
    struct sha256_vstate md;
    sha256_vinit(&md);
    sha256_vprocess(&md,src,len);
    if ( src2 != 0 )
        sha256_vprocess(&md,src2,len2);
    sha256_vdone(&md,hash);
}

void vupdate_sha256(uint8_t hash[256 >> 3],struct sha256_vstate *state,uint8_t *src,int32_t len)
{
    struct sha256_vstate md;
    memset(&md,0,sizeof(md));
    if ( src == 0 )
        sha256_vinit(&md);
    else
    {
        md = *state;
        sha256_vprocess(&md,src,len);
    }
    *state = md;
    sha256_vdone(&md,hash);
}


// rmd160: the five basic functions F(), G() and H()
#define F(x, y, z)        ((x) ^ (y) ^ (z))
#define G(x, y, z)        (((x) & (y)) | (~(x) & (z)))
#define H(x, y, z)        (((x) | ~(y)) ^ (z))
#define I(x, y, z)        (((x) & (z)) | ((y) & ~(z)))
#define J(x, y, z)        ((x) ^ ((y) | ~(z)))
#define ROLc(x, y) ( (((unsigned long)(x)<<(unsigned long)((y)&31)) | (((unsigned long)(x)&0xFFFFFFFFUL)>>(unsigned long)(32-((y)&31)))) & 0xFFFFFFFFUL)

/* the ten basic operations FF() through III() */
#define FF(a, b, c, d, e, x, s)        \
(a) += F((b), (c), (d)) + (x);\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define GG(a, b, c, d, e, x, s)        \
(a) += G((b), (c), (d)) + (x) + 0x5a827999UL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define HH(a, b, c, d, e, x, s)        \
(a) += H((b), (c), (d)) + (x) + 0x6ed9eba1UL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define II(a, b, c, d, e, x, s)        \
(a) += I((b), (c), (d)) + (x) + 0x8f1bbcdcUL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define JJ(a, b, c, d, e, x, s)        \
(a) += J((b), (c), (d)) + (x) + 0xa953fd4eUL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define FFF(a, b, c, d, e, x, s)        \
(a) += F((b), (c), (d)) + (x);\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define GGG(a, b, c, d, e, x, s)        \
(a) += G((b), (c), (d)) + (x) + 0x7a6d76e9UL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define HHH(a, b, c, d, e, x, s)        \
(a) += H((b), (c), (d)) + (x) + 0x6d703ef3UL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define III(a, b, c, d, e, x, s)        \
(a) += I((b), (c), (d)) + (x) + 0x5c4dd124UL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

#define JJJ(a, b, c, d, e, x, s)        \
(a) += J((b), (c), (d)) + (x) + 0x50a28be6UL;\
(a) = ROLc((a), (s)) + (e);\
(c) = ROLc((c), 10);

static int32_t rmd160_vcompress(struct rmd160_vstate *md,uint8_t *buf)
{
    uint32_t aa,bb,cc,dd,ee,aaa,bbb,ccc,ddd,eee,X[16];
    int i;
    
    /* load words X */
    for (i = 0; i < 16; i++){
        LOAD32L(X[i], buf + (4 * i));
    }
    
    /* load state */
    aa = aaa = md->state[0];
    bb = bbb = md->state[1];
    cc = ccc = md->state[2];
    dd = ddd = md->state[3];
    ee = eee = md->state[4];
    
    /* round 1 */
    FF(aa, bb, cc, dd, ee, X[ 0], 11);
    FF(ee, aa, bb, cc, dd, X[ 1], 14);
    FF(dd, ee, aa, bb, cc, X[ 2], 15);
    FF(cc, dd, ee, aa, bb, X[ 3], 12);
    FF(bb, cc, dd, ee, aa, X[ 4],  5);
    FF(aa, bb, cc, dd, ee, X[ 5],  8);
    FF(ee, aa, bb, cc, dd, X[ 6],  7);
    FF(dd, ee, aa, bb, cc, X[ 7],  9);
    FF(cc, dd, ee, aa, bb, X[ 8], 11);
    FF(bb, cc, dd, ee, aa, X[ 9], 13);
    FF(aa, bb, cc, dd, ee, X[10], 14);
    FF(ee, aa, bb, cc, dd, X[11], 15);
    FF(dd, ee, aa, bb, cc, X[12],  6);
    FF(cc, dd, ee, aa, bb, X[13],  7);
    FF(bb, cc, dd, ee, aa, X[14],  9);
    FF(aa, bb, cc, dd, ee, X[15],  8);
    
    /* round 2 */
    GG(ee, aa, bb, cc, dd, X[ 7],  7);
    GG(dd, ee, aa, bb, cc, X[ 4],  6);
    GG(cc, dd, ee, aa, bb, X[13],  8);
    GG(bb, cc, dd, ee, aa, X[ 1], 13);
    GG(aa, bb, cc, dd, ee, X[10], 11);
    GG(ee, aa, bb, cc, dd, X[ 6],  9);
    GG(dd, ee, aa, bb, cc, X[15],  7);
    GG(cc, dd, ee, aa, bb, X[ 3], 15);
    GG(bb, cc, dd, ee, aa, X[12],  7);
    GG(aa, bb, cc, dd, ee, X[ 0], 12);
    GG(ee, aa, bb, cc, dd, X[ 9], 15);
    GG(dd, ee, aa, bb, cc, X[ 5],  9);
    GG(cc, dd, ee, aa, bb, X[ 2], 11);
    GG(bb, cc, dd, ee, aa, X[14],  7);
    GG(aa, bb, cc, dd, ee, X[11], 13);
    GG(ee, aa, bb, cc, dd, X[ 8], 12);
    
    /* round 3 */
    HH(dd, ee, aa, bb, cc, X[ 3], 11);
    HH(cc, dd, ee, aa, bb, X[10], 13);
    HH(bb, cc, dd, ee, aa, X[14],  6);
    HH(aa, bb, cc, dd, ee, X[ 4],  7);
    HH(ee, aa, bb, cc, dd, X[ 9], 14);
    HH(dd, ee, aa, bb, cc, X[15],  9);
    HH(cc, dd, ee, aa, bb, X[ 8], 13);
    HH(bb, cc, dd, ee, aa, X[ 1], 15);
    HH(aa, bb, cc, dd, ee, X[ 2], 14);
    HH(ee, aa, bb, cc, dd, X[ 7],  8);
    HH(dd, ee, aa, bb, cc, X[ 0], 13);
    HH(cc, dd, ee, aa, bb, X[ 6],  6);
    HH(bb, cc, dd, ee, aa, X[13],  5);
    HH(aa, bb, cc, dd, ee, X[11], 12);
    HH(ee, aa, bb, cc, dd, X[ 5],  7);
    HH(dd, ee, aa, bb, cc, X[12],  5);
    
    /* round 4 */
    II(cc, dd, ee, aa, bb, X[ 1], 11);
    II(bb, cc, dd, ee, aa, X[ 9], 12);
    II(aa, bb, cc, dd, ee, X[11], 14);
    II(ee, aa, bb, cc, dd, X[10], 15);
    II(dd, ee, aa, bb, cc, X[ 0], 14);
    II(cc, dd, ee, aa, bb, X[ 8], 15);
    II(bb, cc, dd, ee, aa, X[12],  9);
    II(aa, bb, cc, dd, ee, X[ 4],  8);
    II(ee, aa, bb, cc, dd, X[13],  9);
    II(dd, ee, aa, bb, cc, X[ 3], 14);
    II(cc, dd, ee, aa, bb, X[ 7],  5);
    II(bb, cc, dd, ee, aa, X[15],  6);
    II(aa, bb, cc, dd, ee, X[14],  8);
    II(ee, aa, bb, cc, dd, X[ 5],  6);
    II(dd, ee, aa, bb, cc, X[ 6],  5);
    II(cc, dd, ee, aa, bb, X[ 2], 12);
    
    /* round 5 */
    JJ(bb, cc, dd, ee, aa, X[ 4],  9);
    JJ(aa, bb, cc, dd, ee, X[ 0], 15);
    JJ(ee, aa, bb, cc, dd, X[ 5],  5);
    JJ(dd, ee, aa, bb, cc, X[ 9], 11);
    JJ(cc, dd, ee, aa, bb, X[ 7],  6);
    JJ(bb, cc, dd, ee, aa, X[12],  8);
    JJ(aa, bb, cc, dd, ee, X[ 2], 13);
    JJ(ee, aa, bb, cc, dd, X[10], 12);
    JJ(dd, ee, aa, bb, cc, X[14],  5);
    JJ(cc, dd, ee, aa, bb, X[ 1], 12);
    JJ(bb, cc, dd, ee, aa, X[ 3], 13);
    JJ(aa, bb, cc, dd, ee, X[ 8], 14);
    JJ(ee, aa, bb, cc, dd, X[11], 11);
    JJ(dd, ee, aa, bb, cc, X[ 6],  8);
    JJ(cc, dd, ee, aa, bb, X[15],  5);
    JJ(bb, cc, dd, ee, aa, X[13],  6);
    
    /* parallel round 1 */
    JJJ(aaa, bbb, ccc, ddd, eee, X[ 5],  8);
    JJJ(eee, aaa, bbb, ccc, ddd, X[14],  9);
    JJJ(ddd, eee, aaa, bbb, ccc, X[ 7],  9);
    JJJ(ccc, ddd, eee, aaa, bbb, X[ 0], 11);
    JJJ(bbb, ccc, ddd, eee, aaa, X[ 9], 13);
    JJJ(aaa, bbb, ccc, ddd, eee, X[ 2], 15);
    JJJ(eee, aaa, bbb, ccc, ddd, X[11], 15);
    JJJ(ddd, eee, aaa, bbb, ccc, X[ 4],  5);
    JJJ(ccc, ddd, eee, aaa, bbb, X[13],  7);
    JJJ(bbb, ccc, ddd, eee, aaa, X[ 6],  7);
    JJJ(aaa, bbb, ccc, ddd, eee, X[15],  8);
    JJJ(eee, aaa, bbb, ccc, ddd, X[ 8], 11);
    JJJ(ddd, eee, aaa, bbb, ccc, X[ 1], 14);
    JJJ(ccc, ddd, eee, aaa, bbb, X[10], 14);
    JJJ(bbb, ccc, ddd, eee, aaa, X[ 3], 12);
    JJJ(aaa, bbb, ccc, ddd, eee, X[12],  6);
    
    /* parallel round 2 */
    III(eee, aaa, bbb, ccc, ddd, X[ 6],  9);
    III(ddd, eee, aaa, bbb, ccc, X[11], 13);
    III(ccc, ddd, eee, aaa, bbb, X[ 3], 15);
    III(bbb, ccc, ddd, eee, aaa, X[ 7],  7);
    III(aaa, bbb, ccc, ddd, eee, X[ 0], 12);
    III(eee, aaa, bbb, ccc, ddd, X[13],  8);
    III(ddd, eee, aaa, bbb, ccc, X[ 5],  9);
    III(ccc, ddd, eee, aaa, bbb, X[10], 11);
    III(bbb, ccc, ddd, eee, aaa, X[14],  7);
    III(aaa, bbb, ccc, ddd, eee, X[15],  7);
    III(eee, aaa, bbb, ccc, ddd, X[ 8], 12);
    III(ddd, eee, aaa, bbb, ccc, X[12],  7);
    III(ccc, ddd, eee, aaa, bbb, X[ 4],  6);
    III(bbb, ccc, ddd, eee, aaa, X[ 9], 15);
    III(aaa, bbb, ccc, ddd, eee, X[ 1], 13);
    III(eee, aaa, bbb, ccc, ddd, X[ 2], 11);
    
    /* parallel round 3 */
    HHH(ddd, eee, aaa, bbb, ccc, X[15],  9);
    HHH(ccc, ddd, eee, aaa, bbb, X[ 5],  7);
    HHH(bbb, ccc, ddd, eee, aaa, X[ 1], 15);
    HHH(aaa, bbb, ccc, ddd, eee, X[ 3], 11);
    HHH(eee, aaa, bbb, ccc, ddd, X[ 7],  8);
    HHH(ddd, eee, aaa, bbb, ccc, X[14],  6);
    HHH(ccc, ddd, eee, aaa, bbb, X[ 6],  6);
    HHH(bbb, ccc, ddd, eee, aaa, X[ 9], 14);
    HHH(aaa, bbb, ccc, ddd, eee, X[11], 12);
    HHH(eee, aaa, bbb, ccc, ddd, X[ 8], 13);
    HHH(ddd, eee, aaa, bbb, ccc, X[12],  5);
    HHH(ccc, ddd, eee, aaa, bbb, X[ 2], 14);
    HHH(bbb, ccc, ddd, eee, aaa, X[10], 13);
    HHH(aaa, bbb, ccc, ddd, eee, X[ 0], 13);
    HHH(eee, aaa, bbb, ccc, ddd, X[ 4],  7);
    HHH(ddd, eee, aaa, bbb, ccc, X[13],  5);
    
    /* parallel round 4 */
    GGG(ccc, ddd, eee, aaa, bbb, X[ 8], 15);
    GGG(bbb, ccc, ddd, eee, aaa, X[ 6],  5);
    GGG(aaa, bbb, ccc, ddd, eee, X[ 4],  8);
    GGG(eee, aaa, bbb, ccc, ddd, X[ 1], 11);
    GGG(ddd, eee, aaa, bbb, ccc, X[ 3], 14);
    GGG(ccc, ddd, eee, aaa, bbb, X[11], 14);
    GGG(bbb, ccc, ddd, eee, aaa, X[15],  6);
    GGG(aaa, bbb, ccc, ddd, eee, X[ 0], 14);
    GGG(eee, aaa, bbb, ccc, ddd, X[ 5],  6);
    GGG(ddd, eee, aaa, bbb, ccc, X[12],  9);
    GGG(ccc, ddd, eee, aaa, bbb, X[ 2], 12);
    GGG(bbb, ccc, ddd, eee, aaa, X[13],  9);
    GGG(aaa, bbb, ccc, ddd, eee, X[ 9], 12);
    GGG(eee, aaa, bbb, ccc, ddd, X[ 7],  5);
    GGG(ddd, eee, aaa, bbb, ccc, X[10], 15);
    GGG(ccc, ddd, eee, aaa, bbb, X[14],  8);
    
    /* parallel round 5 */
    FFF(bbb, ccc, ddd, eee, aaa, X[12] ,  8);
    FFF(aaa, bbb, ccc, ddd, eee, X[15] ,  5);
    FFF(eee, aaa, bbb, ccc, ddd, X[10] , 12);
    FFF(ddd, eee, aaa, bbb, ccc, X[ 4] ,  9);
    FFF(ccc, ddd, eee, aaa, bbb, X[ 1] , 12);
    FFF(bbb, ccc, ddd, eee, aaa, X[ 5] ,  5);
    FFF(aaa, bbb, ccc, ddd, eee, X[ 8] , 14);
    FFF(eee, aaa, bbb, ccc, ddd, X[ 7] ,  6);
    FFF(ddd, eee, aaa, bbb, ccc, X[ 6] ,  8);
    FFF(ccc, ddd, eee, aaa, bbb, X[ 2] , 13);
    FFF(bbb, ccc, ddd, eee, aaa, X[13] ,  6);
    FFF(aaa, bbb, ccc, ddd, eee, X[14] ,  5);
    FFF(eee, aaa, bbb, ccc, ddd, X[ 0] , 15);
    FFF(ddd, eee, aaa, bbb, ccc, X[ 3] , 13);
    FFF(ccc, ddd, eee, aaa, bbb, X[ 9] , 11);
    FFF(bbb, ccc, ddd, eee, aaa, X[11] , 11);
    
    /* combine results */
    ddd += cc + md->state[1];               /* final result for md->state[0] */
    md->state[1] = md->state[2] + dd + eee;
    md->state[2] = md->state[3] + ee + aaa;
    md->state[3] = md->state[4] + aa + bbb;
    md->state[4] = md->state[0] + bb + ccc;
    md->state[0] = ddd;
    
    return 0;
}

/**
 Initialize the hash state
 @param md   The hash state you wish to initialize
 @return 0 if successful
 */
int rmd160_vinit(struct rmd160_vstate * md)
{
    md->state[0] = 0x67452301UL;
    md->state[1] = 0xefcdab89UL;
    md->state[2] = 0x98badcfeUL;
    md->state[3] = 0x10325476UL;
    md->state[4] = 0xc3d2e1f0UL;
    md->curlen   = 0;
    md->length   = 0;
    return 0;
}
#define HASH_PROCESS(func_name, compress_name, state_var, block_size)                       \
int func_name (struct rmd160_vstate * md, const unsigned char *in, unsigned long inlen)               \
{                                                                                           \
unsigned long n;                                                                        \
int           err;                                                                      \
if (md->curlen > sizeof(md->buf)) {                             \
return -1;                                                            \
}                                                                                       \
while (inlen > 0) {                                                                     \
if (md->curlen == 0 && inlen >= block_size) {                           \
if ((err = compress_name (md, (unsigned char *)in)) != 0) {               \
return err;                                                                   \
}                                                                                \
md->length += block_size * 8;                                        \
in             += block_size;                                                    \
inlen          -= block_size;                                                    \
} else {                                                                            \
n = MIN(inlen, (block_size - md->curlen));                           \
memcpy(md->buf + md->curlen, in, (size_t)n);              \
md->curlen += n;                                                     \
in             += n;                                                             \
inlen          -= n;                                                             \
if (md->curlen == block_size) {                                      \
if ((err = compress_name (md, md->buf)) != 0) {            \
return err;                                                                \
}                                                                             \
md->length += 8*block_size;                                       \
md->curlen = 0;                                                   \
}                                                                                \
}                                                                                    \
}                                                                                       \
return 0;                                                                        \
}

/**
 Process a block of memory though the hash
 @param md     The hash state
 @param in     The data to hash
 @param inlen  The length of the data (octets)
 @return 0 if successful
 */
HASH_PROCESS(rmd160_vprocess, rmd160_vcompress, rmd160, 64)

/**
 Terminate the hash to get the digest
 @param md  The hash state
 @param out [out] The destination of the hash (20 bytes)
 @return 0 if successful
 */
int rmd160_vdone(struct rmd160_vstate * md, unsigned char *out)
{
    int i;
    if (md->curlen >= sizeof(md->buf)) {
        return -1;
    }
    /* increase the length of the message */
    md->length += md->curlen * 8;
    
    /* append the '1' bit */
    md->buf[md->curlen++] = (unsigned char)0x80;
    
    /* if the length is currently above 56 bytes we append zeros
     * then compress.  Then we can fall back to padding zeros and length
     * encoding like normal.
     */
    if (md->curlen > 56) {
        while (md->curlen < 64) {
            md->buf[md->curlen++] = (unsigned char)0;
        }
        rmd160_vcompress(md, md->buf);
        md->curlen = 0;
    }
    /* pad upto 56 bytes of zeroes */
    while (md->curlen < 56) {
        md->buf[md->curlen++] = (unsigned char)0;
    }
    /* store length */
    STORE64L(md->length, md->buf+56);
    rmd160_vcompress(md, md->buf);
    /* copy output */
    for (i = 0; i < 5; i++) {
        STORE32L(md->state[i], out+(4*i));
    }
    return 0;
}

void calc_rmd160(char hexstr[41],uint8_t buf[20],uint8_t *msg,int32_t len)
{
    struct rmd160_vstate md;
    rmd160_vinit(&md);
    rmd160_vprocess(&md,msg,len);
    rmd160_vdone(&md, buf);
    if ( hexstr != 0 )
        init_hexbytes_noT(hexstr,buf,20);
}

bits256 bits256_sha256(bits256 data)
{
    bits256 hash;
    vcalc_sha256(0,hash.bytes,data.bytes,sizeof(data));
    return(hash);
}

void bits256_rmd160(uint8_t rmd160[20],bits256 data)
{
    calc_rmd160(0,rmd160,data.bytes,sizeof(data));
}

void bits256_rmd160_sha256(uint8_t rmd160[20],bits256 data)
{
    bits256 hash;
    hash = bits256_sha256(data);
    bits256_rmd160(rmd160,hash);
}

#ifdef ENABLE_RMDTEST
int rmd160_test(void)
{
    static const struct {
        char *msg;
        unsigned char md[20];
    } tests[] = {
        { "",
            { 0x9c, 0x11, 0x85, 0xa5, 0xc5, 0xe9, 0xfc, 0x54, 0x61, 0x28,
                0x08, 0x97, 0x7e, 0xe8, 0xf5, 0x48, 0xb2, 0x25, 0x8d, 0x31 }
        },
        { "a",
            { 0x0b, 0xdc, 0x9d, 0x2d, 0x25, 0x6b, 0x3e, 0xe9, 0xda, 0xae,
                0x34, 0x7b, 0xe6, 0xf4, 0xdc, 0x83, 0x5a, 0x46, 0x7f, 0xfe }
        },
        { "abc",
            { 0x8e, 0xb2, 0x08, 0xf7, 0xe0, 0x5d, 0x98, 0x7a, 0x9b, 0x04,
                0x4a, 0x8e, 0x98, 0xc6, 0xb0, 0x87, 0xf1, 0x5a, 0x0b, 0xfc }
        },
        { "message digest",
            { 0x5d, 0x06, 0x89, 0xef, 0x49, 0xd2, 0xfa, 0xe5, 0x72, 0xb8,
                0x81, 0xb1, 0x23, 0xa8, 0x5f, 0xfa, 0x21, 0x59, 0x5f, 0x36 }
        },
        { "abcdefghijklmnopqrstuvwxyz",
            { 0xf7, 0x1c, 0x27, 0x10, 0x9c, 0x69, 0x2c, 0x1b, 0x56, 0xbb,
                0xdc, 0xeb, 0x5b, 0x9d, 0x28, 0x65, 0xb3, 0x70, 0x8d, 0xbc }
        },
        { "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            { 0x12, 0xa0, 0x53, 0x38, 0x4a, 0x9c, 0x0c, 0x88, 0xe4, 0x05,
                0xa0, 0x6c, 0x27, 0xdc, 0xf4, 0x9a, 0xda, 0x62, 0xeb, 0x2b }
        }
    };
    int x;
    unsigned char buf[20]; char hexstr[41];
    
    for (x = 0; x < (int)(sizeof(tests)/sizeof(tests[0])); x++) {
        calc_rmd160(hexstr,buf,(unsigned char *)tests[x].msg,(int32_t) strlen(tests[x].msg));
        if (memcmp(buf, tests[x].md, 20) != 0) {
            printf("Failed test %d\n", x);
            return -1;
        }
        else printf("rmd160(%s) -> (%s)\n",tests[x].msg,hexstr);
    }
    return 0;
}
#endif

#undef FF
#undef GG
#undef HH
#undef II
#undef FFF
#undef GGG
#undef HHH
#undef III
#undef F
#undef G
#undef H
#undef I
#undef J
#undef ROLc

static const uint32_t crc32_tab[] = {
	0x00000000, 0x77073096, 0xee0e612c, 0x990951ba, 0x076dc419, 0x706af48f,
	0xe963a535, 0x9e6495a3,	0x0edb8832, 0x79dcb8a4, 0xe0d5e91e, 0x97d2d988,
	0x09b64c2b, 0x7eb17cbd, 0xe7b82d07, 0x90bf1d91, 0x1db71064, 0x6ab020f2,
	0xf3b97148, 0x84be41de,	0x1adad47d, 0x6ddde4eb, 0xf4d4b551, 0x83d385c7,
	0x136c9856, 0x646ba8c0, 0xfd62f97a, 0x8a65c9ec,	0x14015c4f, 0x63066cd9,
	0xfa0f3d63, 0x8d080df5,	0x3b6e20c8, 0x4c69105e, 0xd56041e4, 0xa2677172,
	0x3c03e4d1, 0x4b04d447, 0xd20d85fd, 0xa50ab56b,	0x35b5a8fa, 0x42b2986c,
	0xdbbbc9d6, 0xacbcf940,	0x32d86ce3, 0x45df5c75, 0xdcd60dcf, 0xabd13d59,
	0x26d930ac, 0x51de003a, 0xc8d75180, 0xbfd06116, 0x21b4f4b5, 0x56b3c423,
	0xcfba9599, 0xb8bda50f, 0x2802b89e, 0x5f058808, 0xc60cd9b2, 0xb10be924,
	0x2f6f7c87, 0x58684c11, 0xc1611dab, 0xb6662d3d,	0x76dc4190, 0x01db7106,
	0x98d220bc, 0xefd5102a, 0x71b18589, 0x06b6b51f, 0x9fbfe4a5, 0xe8b8d433,
	0x7807c9a2, 0x0f00f934, 0x9609a88e, 0xe10e9818, 0x7f6a0dbb, 0x086d3d2d,
	0x91646c97, 0xe6635c01, 0x6b6b51f4, 0x1c6c6162, 0x856530d8, 0xf262004e,
	0x6c0695ed, 0x1b01a57b, 0x8208f4c1, 0xf50fc457, 0x65b0d9c6, 0x12b7e950,
	0x8bbeb8ea, 0xfcb9887c, 0x62dd1ddf, 0x15da2d49, 0x8cd37cf3, 0xfbd44c65,
	0x4db26158, 0x3ab551ce, 0xa3bc0074, 0xd4bb30e2, 0x4adfa541, 0x3dd895d7,
	0xa4d1c46d, 0xd3d6f4fb, 0x4369e96a, 0x346ed9fc, 0xad678846, 0xda60b8d0,
	0x44042d73, 0x33031de5, 0xaa0a4c5f, 0xdd0d7cc9, 0x5005713c, 0x270241aa,
	0xbe0b1010, 0xc90c2086, 0x5768b525, 0x206f85b3, 0xb966d409, 0xce61e49f,
	0x5edef90e, 0x29d9c998, 0xb0d09822, 0xc7d7a8b4, 0x59b33d17, 0x2eb40d81,
	0xb7bd5c3b, 0xc0ba6cad, 0xedb88320, 0x9abfb3b6, 0x03b6e20c, 0x74b1d29a,
	0xead54739, 0x9dd277af, 0x04db2615, 0x73dc1683, 0xe3630b12, 0x94643b84,
	0x0d6d6a3e, 0x7a6a5aa8, 0xe40ecf0b, 0x9309ff9d, 0x0a00ae27, 0x7d079eb1,
	0xf00f9344, 0x8708a3d2, 0x1e01f268, 0x6906c2fe, 0xf762575d, 0x806567cb,
	0x196c3671, 0x6e6b06e7, 0xfed41b76, 0x89d32be0, 0x10da7a5a, 0x67dd4acc,
	0xf9b9df6f, 0x8ebeeff9, 0x17b7be43, 0x60b08ed5, 0xd6d6a3e8, 0xa1d1937e,
	0x38d8c2c4, 0x4fdff252, 0xd1bb67f1, 0xa6bc5767, 0x3fb506dd, 0x48b2364b,
	0xd80d2bda, 0xaf0a1b4c, 0x36034af6, 0x41047a60, 0xdf60efc3, 0xa867df55,
	0x316e8eef, 0x4669be79, 0xcb61b38c, 0xbc66831a, 0x256fd2a0, 0x5268e236,
	0xcc0c7795, 0xbb0b4703, 0x220216b9, 0x5505262f, 0xc5ba3bbe, 0xb2bd0b28,
	0x2bb45a92, 0x5cb36a04, 0xc2d7ffa7, 0xb5d0cf31, 0x2cd99e8b, 0x5bdeae1d,
	0x9b64c2b0, 0xec63f226, 0x756aa39c, 0x026d930a, 0x9c0906a9, 0xeb0e363f,
	0x72076785, 0x05005713, 0x95bf4a82, 0xe2b87a14, 0x7bb12bae, 0x0cb61b38,
	0x92d28e9b, 0xe5d5be0d, 0x7cdcefb7, 0x0bdbdf21, 0x86d3d2d4, 0xf1d4e242,
	0x68ddb3f8, 0x1fda836e, 0x81be16cd, 0xf6b9265b, 0x6fb077e1, 0x18b74777,
	0x88085ae6, 0xff0f6a70, 0x66063bca, 0x11010b5c, 0x8f659eff, 0xf862ae69,
	0x616bffd3, 0x166ccf45, 0xa00ae278, 0xd70dd2ee, 0x4e048354, 0x3903b3c2,
	0xa7672661, 0xd06016f7, 0x4969474d, 0x3e6e77db, 0xaed16a4a, 0xd9d65adc,
	0x40df0b66, 0x37d83bf0, 0xa9bcae53, 0xdebb9ec5, 0x47b2cf7f, 0x30b5ffe9,
	0xbdbdf21c, 0xcabac28a, 0x53b39330, 0x24b4a3a6, 0xbad03605, 0xcdd70693,
	0x54de5729, 0x23d967bf, 0xb3667a2e, 0xc4614ab8, 0x5d681b02, 0x2a6f2b94,
	0xb40bbe37, 0xc30c8ea1, 0x5a05df1b, 0x2d02ef8d
};

// CRC-32-IEEE
uint32_t calc_crc32(uint32_t crc,const void *buf,size_t size)
{
	const uint8_t *p;
    
	p = (const uint8_t *)buf;
	crc = crc ^ ~0U;
    
	while (size--)
		crc = crc32_tab[(crc ^ *p++) & 0xFF] ^ (crc >> 8);
    
	return crc ^ ~0U;
}

bits256 curve25519_shared(bits256 privkey,bits256 otherpub)
{
    bits256 shared,hash;
    shared = curve25519(privkey,otherpub);
    vcalc_sha256(0,hash.bytes,shared.bytes,sizeof(shared));
    //printf("priv.%llx pub.%llx shared.%llx -> hash.%llx\n",privkey.txid,pubkey.txid,shared.txid,hash.txid);
    //hash.bytes[0] &= 0xf8, hash.bytes[31] &= 0x7f, hash.bytes[31] |= 64;
    return(hash);
}

int32_t curve25519_donna(uint8_t *mypublic,const uint8_t *secret,const uint8_t *basepoint);
/*{
    bits256 val,p,bp;
    memcpy(p.bytes,secret,sizeof(p));
    memcpy(bp.bytes,basepoint,sizeof(bp));
    val = curve25519(p,bp);
    memcpy(mypublic,val.bytes,sizeof(val));
    return(0);
}*/

uint64_t conv_NXTpassword(unsigned char *mysecret,unsigned char *mypublic,uint8_t *pass,int32_t passlen)
{
    static uint8_t basepoint[32] = {9};
    uint64_t addr; uint8_t hash[32];
    if ( pass != 0 && passlen != 0 )
        vcalc_sha256(0,mysecret,pass,passlen);
    mysecret[0] &= 248, mysecret[31] &= 127, mysecret[31] |= 64;
    curve25519_donna(mypublic,mysecret,basepoint);
    vcalc_sha256(0,hash,mypublic,32);
    memcpy(&addr,hash,sizeof(addr));
    return(addr);
}

#include <stdio.h>

bits256 GENESIS_PUBKEY,GENESIS_PRIVKEY;

bits256 acct777_pubkey(bits256 privkey)
{
    static uint8_t basepoint[32] = {9};
    bits256 pubkey;
    privkey.bytes[0] &= 248, privkey.bytes[31] &= 127, privkey.bytes[31] |= 64;
    curve25519_donna(pubkey.bytes,privkey.bytes,basepoint);
    return(pubkey);
}

uint64_t acct777_nxt64bits(bits256 pubkey)
{
    bits256 acct;
    vcalc_sha256(0,acct.bytes,pubkey.bytes,sizeof(pubkey));
    return(acct.txid);
}

bits256 acct777_msgprivkey(uint8_t *data,int32_t datalen)
{
    bits256 hash;
    vcalc_sha256(0,hash.bytes,data,datalen);
    return(hash);
}

bits256 acct777_msgpubkey(uint8_t *data,int32_t datalen)
{
    return(acct777_pubkey(acct777_msgprivkey(data,datalen)));
}

bits256 acct777_hashiter(bits256 privkey,bits256 pubkey,int32_t lockdays,uint8_t chainlen)
{
    uint16_t lockseed,signlen = 0; uint8_t signbuf[16]; bits256 shared,lockhash;
    lockseed = (chainlen & 0x7f) | (lockdays << 7);
    signlen = 0, signbuf[signlen++] = lockseed & 0xff, signbuf[signlen++] = (lockseed >> 8) & 0xff;
    
    privkey.bytes[0] &= 248, privkey.bytes[31] &= 127, privkey.bytes[31] |= 64;
    shared = curve25519(privkey,pubkey);
    vcalc_sha256cat(lockhash.bytes,shared.bytes,sizeof(shared),signbuf,signlen);
    return(lockhash);
}

bits256 acct777_lockhash(bits256 pubkey,int32_t lockdays,uint8_t chainlen)
{
    bits256 lockhash = GENESIS_PRIVKEY;
    while ( chainlen > 0 )
        lockhash = acct777_hashiter(lockhash,pubkey,lockdays,chainlen--);
    return(lockhash);
}

bits256 acct777_invoicehash(bits256 *invoicehash,uint16_t lockdays,uint8_t chainlen)
{
    int32_t i; bits256 lockhash,privkey;
    OS_randombytes(privkey.bytes,sizeof(privkey)); // both privkey and pubkey are sensitive. pubkey allows verification, privkey proves owner
    lockhash = privkey;
    for (i=0; i<chainlen; i++)
        lockhash = acct777_hashiter(lockhash,GENESIS_PUBKEY,chainlen - i,lockdays);
    *invoicehash = lockhash;
    return(privkey);
}

//char *bits256_str();
//struct acct777_sig { bits256 sigbits,pubkey; uint64_t signer64bits; uint32_t timestamp,allocsize; };

void acct777_rwsig(int32_t rwflag,uint8_t *serialized,struct acct777_sig *sig)
{
    int32_t len = 0;
    iguana_rwbignum(rwflag,&serialized[len],sizeof(bits256),sig->sigbits.bytes), len += sizeof(bits256);
    iguana_rwbignum(rwflag,&serialized[len],sizeof(bits256),sig->pubkey.bytes),len += sizeof(bits256);
    iguana_rwnum(rwflag,&serialized[len],sizeof(sig->signer64bits),&sig->signer64bits),len += sizeof(sig->signer64bits);
    iguana_rwnum(rwflag,&serialized[len],sizeof(sig->timestamp),&sig->timestamp),len += sizeof(sig->timestamp);
    iguana_rwnum(rwflag,&serialized[len],sizeof(sig->allocsize),&sig->allocsize),len += sizeof(sig->allocsize);
}

int32_t acct777_sigcheck(struct acct777_sig *sig)
{
#define IGUANA_GENESIS 1453075200
#define IGUANA_MAXPACKETSIZE (1024 * 1024 * 4)
#define TEN_YEARS (10 * 365 * 24 * 3600)
   if ( sig->allocsize < sizeof(*sig) || sig->allocsize > IGUANA_MAXPACKETSIZE )
    {
        //printf("acct777_sign: invalid datalen.%d hex.%08x\n",sig->allocsize,sig->allocsize);
        return(-1);
    }
    if ( sig->timestamp < IGUANA_GENESIS || sig->timestamp > (long)IGUANA_GENESIS+TEN_YEARS )
    {
        printf("acct777_sign: invalid timestamp.%u (%u %u)\n",sig->timestamp,IGUANA_GENESIS,IGUANA_GENESIS+TEN_YEARS);
        return(-1);
    }
    return(sig->allocsize);
}

uint64_t acct777_sign(struct acct777_sig *sig,bits256 privkey,bits256 otherpubkey,uint32_t timestamp,uint8_t *serialized,int32_t datalen)
{
    bits256 pubkey; bits256 shared; uint8_t buf[sizeof(*sig)];
    pubkey = acct777_pubkey(privkey);
    if ( memcmp(sig->pubkey.bytes,otherpubkey.bytes,sizeof(bits256)) != 0 )
    {
        //char str[65],str2[65];
        //printf("set sig fields.(%s) != (%s)\n",bits256_str(str,sig->pubkey),bits256_str(str2,otherpubkey));
        sig->pubkey = pubkey;
        sig->timestamp = timestamp;
        sig->allocsize = (int32_t)(datalen + sizeof(*sig));
        sig->signer64bits = acct777_nxt64bits(sig->pubkey);
    }
    sig->sigbits = shared = curve25519(privkey,otherpubkey);
    if ( acct777_sigcheck(sig) < 0 )
        return(0);
    memset(buf,0,sizeof(buf));
    acct777_rwsig(1,buf,sig);
    //int32_t i; for (i=0; i<sizeof(buf); i++)
    //    printf("%02x ",buf[i]);
    //printf(" <<<<<<<<< SIGN.%d allocsize.%d signer.%llu t%u\n",datalen,sig->allocsize,(long long)sig->signer64bits,sig->timestamp);
    //char str[65]; printf("shared.(%s) crc.%u datalen.%d\n",bits256_str(str,shared),calc_crc32(0,buf,sizeof(buf)),datalen);
    vcalc_sha256cat(sig->sigbits.bytes,buf,sizeof(buf),serialized,datalen);
    //printf(" calcsig.%llx pubkey.%llx signer.%llu | t%u crc.%08x len.%d shared.%llx <- %llx * %llx\n",(long long)sig->sigbits.txid,(long long)sig->pubkey.txid,(long long)sig->signer64bits,timestamp,calc_crc32(0,serialized,datalen),datalen,(long long)shared.txid,(long long)privkey.txid,(long long)otherpubkey.txid);
    return(sig->signer64bits);
}

uint64_t acct777_validate(struct acct777_sig *sig,bits256 privkey,bits256 pubkey)
{
    struct acct777_sig checksig; uint64_t signerbits; int32_t datalen; uint8_t *serialized;
    datalen = (int32_t)(sig->allocsize - sizeof(*sig));
    checksig = *sig;
#if defined(_M_X64)
	serialized = (uint8_t *)((unsigned char *)sig + sizeof(*sig));
#else
    serialized = (uint8_t *)((long)sig + sizeof(*sig));
#endif
    //{ int32_t i; for (i=0; i<datalen; i++) printf("%02x",serialized[i]); printf(" VALIDATE.%d?\n",datalen); }
    acct777_sign(&checksig,privkey,pubkey,sig->timestamp,serialized,datalen);
    if ( memcmp(checksig.sigbits.bytes,sig->sigbits.bytes,sizeof(checksig.sigbits)) != 0 )
    {
        //char *bits256_str();
        //char str[65],str2[65]; printf("sig compare error using sig->pub from %llu\n>>>>>>>> sig.(%s) vs (%s)",(long long)acct777_nxt64bits(sig->pubkey),bits256_str(str,checksig.sigbits),bits256_str(str2,sig->sigbits));
        return(0);
    }
    signerbits = acct777_nxt64bits(sig->pubkey);
    if ( signerbits == checksig.signer64bits )
        return(signerbits);
    else return(0);
}

uint64_t acct777_signtx(struct acct777_sig *sig,bits256 privkey,uint32_t timestamp,uint8_t *data,int32_t datalen)
{
    return(acct777_sign(sig,privkey,acct777_msgpubkey(data,datalen),timestamp,data,datalen));
}

/*uint64_t acct777_swaptx(bits256 privkey,struct acct777_sig *sig,uint32_t timestamp,uint8_t *data,int32_t datalen)
{
    uint64_t othernxt;
    if ( (othernxt= acct777_validate(sig)) != sig->signer64bits )
        return(0);
    return(acct777_sign(sig,privkey,acct777_msgpubkey(data,datalen),timestamp,data,datalen));
}*/

int32_t _SuperNET_cipher(uint8_t nonce[crypto_box_NONCEBYTES],uint8_t *cipher,uint8_t *message,int32_t len,bits256 destpub,bits256 srcpriv,uint8_t *buf)
{
    memset(cipher,0,len+crypto_box_ZEROBYTES);
    memset(buf,0,crypto_box_ZEROBYTES);
    memcpy(buf+crypto_box_ZEROBYTES,message,len);
    crypto_box(cipher,buf,len+crypto_box_ZEROBYTES,nonce,destpub.bytes,srcpriv.bytes);
    return(len + crypto_box_ZEROBYTES);
}

uint8_t *_SuperNET_decipher(uint8_t nonce[crypto_box_NONCEBYTES],uint8_t *cipher,uint8_t *message,int32_t len,bits256 srcpub,bits256 mypriv)
{
    int32_t err;
    if ( (err= crypto_box_open(message,cipher,len,nonce,srcpub.bytes,mypriv.bytes)) == 0 )
    {
        message += crypto_box_ZEROBYTES;
        len -= crypto_box_ZEROBYTES;
        return(message);
    }
    return(0);
}

//#undef force_inline
