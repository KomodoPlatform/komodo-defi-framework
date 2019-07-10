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
//  DEXstats.h
//  marketmaker
//

#ifndef DEXstats_h
#define DEXstats_h

#define LEFTMARGIN 40
#define MAX_SPLINES 1024
#define MAX_LOOKAHEAD 7

struct stats_spline { char name[64]; int32_t splineid,lasti,basenum,num,firstx,dispincr,spline32[MAX_SPLINES][4]; uint32_t utc32[MAX_SPLINES]; int64_t spline64[MAX_SPLINES][4]; double dSplines[MAX_SPLINES][4],pricevals[MAX_SPLINES+MAX_LOOKAHEAD],lastutc,lastval,aveslopeabs; };

#define _extrapolate_Spline(Splines,gap) ((double)(Splines)[0] + ((gap) * ((double)(Splines)[1] + ((gap) * ((double)(Splines)[2] + ((gap) * (double)(Splines)[3]))))))
#define _extrapolate_Slope(Splines,gap) ((double)(Splines)[1] + ((gap) * ((double)(Splines)[2] + ((gap) * (double)(Splines)[3]))))

#define dto64(x) ((int64_t)((x) * (double)SATOSHIDEN * SATOSHIDEN))
#define dto32(x) ((int32_t)((x) * (double)SATOSHIDEN))
#define i64tod(x) ((double)(x) / ((double)SATOSHIDEN * SATOSHIDEN))
#define i32tod(x) ((double)(x) / (double)SATOSHIDEN)
#define _extrapolate_spline64(spline64,gap) ((double)i64tod((spline64)[0]) + ((gap) * ((double)i64tod(.001*.001*(spline64)[1]) + ((gap) * ((double)i64tod(.001*.001*.001*.001*(spline64)[2]) + ((gap) * (double)i64tod(.001*.001*.001*.001*.001*.001*(spline64)[3])))))))
#define _extrapolate_spline32(spline32,gap) ((double)i32tod((spline32)[0]) + ((gap) * ((double)i32tod(.001*.001*(spline32)[1]) + ((gap) * ((double)i32tod(.001*.001*.001*.001*(spline32)[2]) + ((gap) * (double)i32tod(.001*.001*.001*.001*.001*.001*(spline32)[3])))))))

uint32_t forex_colors[16];
double Display_scale = 0.25;

struct DEXstats_disp { double pricesum,volumesum; };

struct DEXstats_pricepoint
{
    double price,volume;
    uint32_t height;
    uint16_t seconds;
    int8_t hour,dir;
};

struct DEXstats_pairinfo
{
    char dest[128];
    int32_t numprices;
    struct DEXstats_pricepoint *prices;
};

struct DEXstats_datenuminfo
{
    int32_t numpairs,datenum;
    struct DEXstats_pairinfo *pairs;
};

struct DEXstats_priceinfo
{
    char symbol[128];
    int32_t firstdatenum,numdates;
    struct DEXstats_datenuminfo *dates;
} Prices[1024];
int32_t Num_priceinfos;

void stats_pricepoint(int32_t dir,struct DEXstats_pricepoint *ptr,uint8_t hour,uint16_t seconds,int32_t height,double volume,double price)
{
    ptr->price = price;
    ptr->volume = volume;
    ptr->height = height;
    ptr->hour = hour;
    ptr->dir = dir;
    ptr->seconds = seconds;
}

void stats_pairupdate(int32_t dir,struct DEXstats_datenuminfo *date,char *symbol,char *dest,int32_t datenum,int32_t hour,int32_t seconds,int32_t height,double volume,double price)
{
    int32_t i; struct DEXstats_pairinfo *pair = 0;
    if ( date->datenum != datenum || seconds < 0 || seconds >= 3600 || hour < 0 || hour >= 24 )
    {
        printf("date->datenum %d != %d? hour.%d seconds.%d\n",date->datenum,datenum,hour,seconds);
        return;
    }
    //printf("%d numpairs.%d %p %p\n",date->datenum,date->numpairs,date,date->pairs);
    for (i=0; i<date->numpairs; i++)
        if ( strcmp(dest,date->pairs[i].dest) == 0 )
        {
            pair = &date->pairs[i];
            break;
        }
    if ( date->pairs == 0 || i == date->numpairs )
    {
        date->pairs = realloc(date->pairs,sizeof(*date->pairs) * (date->numpairs + 1));
        pair = &date->pairs[date->numpairs++];
        memset(pair,0,sizeof(*pair));
        strcpy(pair->dest,dest);
        printf("%d new pair.%d (%s) -> dest.(%s)\n",date->datenum,date->numpairs,symbol,dest);
    }
    pair->prices = realloc(pair->prices,sizeof(*pair->prices) * (pair->numprices+1));
    stats_pricepoint(dir,&pair->prices[pair->numprices++],hour,seconds,height,volume,price);
    //printf("(%s/%s).%d numprices.%d h.%d s.%-4d %.8f %.6f\n",symbol,dest,date->datenum,pair->numprices,hour,seconds,price,volume);
}

void stats_datenumupdate(int32_t dir,struct DEXstats_priceinfo *pp,int32_t datenum,int32_t hour,int32_t seconds,int32_t height,double volume,char *dest,double price)
{
    int32_t offset,i,n; struct DEXstats_datenuminfo *date;
    if ( (offset= datenum - pp->firstdatenum) < 0 )
    {
        printf("illegal datenum.%d for %s when 1st.%d\n",datenum,pp->symbol,pp->firstdatenum);
        return;
    }
    if ( offset == 0 || offset > pp->numdates )
    {
        pp->dates = realloc(pp->dates,sizeof(*pp->dates) * (offset+1));
        n = (offset - pp->numdates);
        printf("allocate %s.[%d to %d]\n",pp->symbol,pp->numdates,pp->numdates+n);
        for (i=0; i<=n; i++)
        {
            date = &pp->dates[pp->numdates + i];
            if ( date->datenum != pp->firstdatenum + pp->numdates + i )
            {
                memset(date,0,sizeof(*date));
                date->datenum = pp->firstdatenum + pp->numdates + i;
            }
        }
        pp->numdates = offset;
    }
    stats_pairupdate(dir,&pp->dates[offset],pp->symbol,dest,datenum,hour,seconds,height,volume,price);
}

struct DEXstats_priceinfo *stats_priceinfo(char *symbol,int32_t datenum)
{
    int32_t i; struct DEXstats_priceinfo *pp = 0;
    if ( Num_priceinfos >= sizeof(Prices)/sizeof(*Prices) )
        return(0);
    for (i=0; i<Num_priceinfos; i++)
        if ( strcmp(Prices[i].symbol,symbol) == 0 )
        {
            pp = &Prices[i];
            break;
        }
    if ( i == Num_priceinfos )
    {
        pp = &Prices[Num_priceinfos++];
        strcpy(pp->symbol,symbol);
        pp->firstdatenum = datenum;
    }
    return(pp);
}

void stats_LPpubkeyupdate(char *LPpubkey,uint32_t timestamp)
{
    printf("LP.(%s) t.%u\n",LPpubkey,timestamp);
}

void stats_priceupdate(int32_t datenum,int32_t hour,int32_t seconds,uint32_t timestamp,int32_t height,char *key,char *LPpubkey,cJSON *tradejson)
{
    int32_t dir = 0; uint64_t srcamount,destamount; char *source,*dest; double price; struct DEXstats_priceinfo *pp;
    if ( LPpubkey != 0 )
        stats_LPpubkeyupdate(LPpubkey,timestamp);
    if ( tradejson != 0 )
    {
        source = jstr(jitem(tradejson,0),0);
        srcamount = SATOSHIDEN * jdouble(jitem(tradejson,1),0);
        dest = jstr(jitem(tradejson,2),0);
        destamount = SATOSHIDEN * jdouble(jitem(tradejson,3),0);
        if ( srcamount != 0 && destamount != 0 )
        {
            price = (double)destamount / srcamount;
            if ( key != 0 )
            {
                dir = 1;
                if ( (pp= stats_priceinfo(source,datenum)) != 0 )
                    stats_datenumupdate(-1,pp,datenum,hour,seconds,height,dstr(srcamount),dest,price);
                if ( (pp= stats_priceinfo(dest,datenum)) != 0 )
                    stats_datenumupdate(1,pp,datenum,hour,seconds,height,dstr(destamount),source,1. / price);
            }
            else if ( (pp= stats_priceinfo(source,datenum)) != 0 )
                stats_datenumupdate(0,pp,datenum,hour,seconds,height,dstr(srcamount),dest,price);
        } else price = 0.;
        if ( dir != 0 )
            printf("dir.%-2d %d.%02d.%04d ht.%-4d %s (%s %12.8f) -> (%s %12.8f) %16.8f %16.8f\n",dir,datenum,hour,seconds,height,key!=0?key:"",source,dstr(srcamount),dest,dstr(destamount),price,1./price);
    }
}

double _pairaved(double valA,double valB)
{
    if ( valA != 0. && valB != 0. )
        return((valA + valB) / 2.);
    else if ( valA != 0. ) return(valA);
    else return(valB);
}

double calc_loganswer(double pastlogprice,double futurelogprice)
{
    if ( fabs(pastlogprice) < .0000001 || fabs(futurelogprice) < .0000001 )
        return(0);
    return(10000. * (exp(futurelogprice - pastlogprice)-1.));
}

double _pairdiff(register double valA,register double valB)
{
    if ( valA != 0. && valB != 0. )
        return((valA - valB));
    else return(0.);
}

double balanced_ave(double buf[],int32_t i,int32_t width)
{
    register int32_t nonz,j; register double sum,price;
    nonz = 0;
    sum = 0.0;
    for (j=-width; j<=width; j++)
    {
        price = buf[i + j];
        if ( price != 0.0 )
        {
            sum += price;
            nonz++;
        }
    }
    if ( nonz != 0 )
        sum /= nonz;
    return(sum);
}

void buf_trioave(double dest[],double src[],int32_t n)
{
    register int32_t i,j,width = 3;
    for (i=0; i<128; i++)
        src[i] = 0;
    //for (i=n-width-1; i>width; i--)
    //	dest[i] = balanced_ave(src,i,width);
    //for (i=width; i>0; i--)
    //	dest[i] = balanced_ave(src,i,i);
    for (i=1; i<width; i++)
        dest[i] = balanced_ave(src,i,i);
    for (i=width; i<1024-width; i++)
        dest[i] = balanced_ave(src,i,width);
    dest[0] = _pairaved(dest[0],dest[1] - _pairdiff(dest[2],dest[1]));
    j = width-1;
    for (i=1024-width; i<1023; i++,j--)
        dest[i] = balanced_ave(src,i,j);
    if ( dest[1021] != 0. && dest[1021] != 0. )
        dest[1023] = ((2.0 * dest[1022]) - dest[1021]);
    else dest[1023] = 0.;
}

void smooth1024(double dest[],double src[],int32_t smoothiters)
{
    double smoothbufA[1024],smoothbufB[1024]; int32_t i;
    buf_trioave(smoothbufA,src,1024);
    for (i=0; i<smoothiters; i++)
    {
        buf_trioave(smoothbufB,smoothbufA,1024);
        buf_trioave(smoothbufA,smoothbufB,1024);
    }
    buf_trioave(dest,smoothbufA,1024);
}

float _calc_pricey(register double price,register double weekave)
{
    if ( price != 0. && weekave != 0. )
        return(0.1 * calc_loganswer(weekave,price));
    else return(0.f);
}

float pixelwt(register int32_t color)
{
    return(((float)((color>>16)&0x0ff) + (float)((color>>8)&0x0ff) + (float)((color>>0)&0x0ff))/0x300);
}

int32_t pixel_ratios(uint32_t red,uint32_t green,uint32_t blue)
{
    float max;
    /*if ( red > green )
     max = red;
     else
     max = green;
     if ( blue > max )
     max = blue;*/
    max = (red + green + blue);
    if ( max == 0. )
        return(0);
    if ( max > 0xff )
    {
        red = (uint32_t)(((float)red / max) * 0xff);
        green = (uint32_t)(((float)green / max) * 0xff);
        blue = (uint32_t)(((float)blue / max) * 0xff);
    }
    
    if ( red > 0xff )
        red = 0xff;
    if ( green > 0xff )
        green = 0xff;
    if ( blue > 0xff )
        blue = 0xff;
    return((red << 16) | (green << 8) | blue);
}

int32_t conv_yval_to_y(register float yval,register int32_t height)
{
    register int32_t y;
    height = (height>>1) - 2;
    y = (int32_t)-yval;
    if ( y > height )
        y = height;
    else if ( y < -height )
        y = -height;
    
    y += height;
    if ( y < 0 )
        y = 0;
    height <<= 1;
    if ( y >= height-1 )
        y = height-1;
    return(y);
}

uint32_t scale_color(uint32_t color,float strength)
{
    int32_t red,green,blue;
    if ( strength < 0. )
        strength = -strength;
    red = (color>>16) & 0xff;
    green = (color>>8) & 0xff;
    blue = color & 0xff;
    
    red = (int32_t)((float)red * (strength/100.f));
    green = (int32_t)((float)green * (strength/100.f));
    blue = (int32_t)((float)blue * (strength/100.f));
    if ( red > 0xff )
        red = 0xff;
    if ( green > 0xff )
        green = 0xff;
    if ( blue > 0xff )
        blue = 0xff;
    return((red<<16) | (green<<8) | blue);
}

uint32_t pixel_blend(uint32_t pixel,uint32_t color)//,int32_t groupsize)
{
    int32_t red,green,blue,sum,n,n2,groupsize = 1;
    float red2,green2,blue2,sum2;
    if ( color == 0 )
        return(pixel);
    if ( pixel == 0 )
    {
        return((1<<24) | scale_color(color,100.f/(float)groupsize));
    }
    n = (pixel>>24) & 0xff;
    if ( n == 0 )
        n = 1;
    pixel &= 0xffffff;
    red = (pixel>>16) & 0xff;
    green = (pixel>>8) & 0xff;
    blue = pixel & 0xff;
    sum = red + green + blue;
    
    n2 = (color>>24) & 0xff;
    if ( n2 == 0 )
        n2 = 1;
    red2 = ((float)((color>>16) & 0xff)) / groupsize;
    green2 = ((float)((color>>8) & 0xff)) / groupsize;
    blue2 = ((float)(color & 0xff)) / groupsize;
    sum2 = (red2 + green2 + blue2);
    
    //printf("gs %d (%d x %d,%d,%d: %d) + (%d x %.1f,%.1f,%.1f: %.1f) = ",groupsize,n,red,green,blue,sum,n2,red2,green2,blue2,sum2);
    red = (uint32_t)(((((((float)red / (float) sum) * n) + (((float)red2 / (float) sum2) * n2)) / (n+n2)) * ((sum+sum2)/2)));
    green = (uint32_t)(((((((float)green / (float) sum) * n) + (((float)green2 / (float) sum2) * n2)) / (n+n2)) * ((sum+sum2)/2)));
    blue = (uint32_t)(((((((float)blue / (float) sum) * n) + (((float)blue2 / (float) sum2) * n2)) / (n+n2)) * ((sum+sum2)/2)));
    
    n += n2;
    if ( n > 0xff )
        n = 0xff;
    ///printf("%x (%d,%d,%d) ",color,red,green,blue);
    color = (n<<24) | pixel_ratios(red,green,blue);//pixel_overflow(&red,&green,&blue);
    
    //printf("%x (%d,%d,%d)\n",color,(color>>16)&0xff,(color>>8)&0xff,color&0xff);
    return(color);
}

void init_forex_colors(uint32_t *forex_colors)
{
    int32_t i;
    forex_colors[0] = 0x00ff00;
    forex_colors[1] = 0x0033ff;
    forex_colors[2] = 0xff0000;
    forex_colors[3] = 0x00ffff;
    forex_colors[4] = 0xffff00;
    forex_colors[5] = 0xff00ff;
    forex_colors[6] = 0xffffff;
    forex_colors[7] = 0xff8800;
    forex_colors[8] = 0xff88ff;
    for (i=9; i<16; i++)
        forex_colors[i] = pixel_blend(forex_colors[i-8],0xffffff);
}

int32_t is_primary_color(register uint32_t color)
{
    static uint32_t forex_colors[16];
    register int32_t i;
    if ( forex_colors[0] == 0 )
        init_forex_colors(forex_colors);
    for (i=0; i<8; i++)
        if ( color == forex_colors[i] )
            return(1);
    return(0);
}

void disp_yval(register int32_t color,register float yval,register uint32_t *bitmap,register int32_t x,register int32_t rowwidth,register int32_t height)
{
    register int32_t y;
    if ( forex_colors[0] == 0 )
        init_forex_colors(forex_colors);
    x += LEFTMARGIN;
    if ( x < 0 || x >= rowwidth )
        return;
    //y = conv_yval_to_y(yval,height/Display_scale) * Display_scale;
    y = conv_yval_to_y(yval * Display_scale,height);
    if ( 1 && is_primary_color(color) != 0 )
    {
        bitmap[y*rowwidth + x] = color;
        //printf("(%d, %d) <- %x, ",x,y,color);
        return;
    }
    //if ( pixelwt(color) > pixelwt(bitmap[y*rowwidth + x]) )
    bitmap[y*rowwidth + x] = pixel_blend(bitmap[y*rowwidth + x],color);
    return;
    //if ( is_primary_color(color) != 0 || (is_primary_color(bitmap[y*rowwidth+x]) == 0 && pixelwt(color) > pixelwt(bitmap[y*rowwidth + x])) )
    //	bitmap[y*rowwidth + x] = color;
}

void disp_yvalsum(register int32_t color,register float yval,register uint32_t *bitmap,register int32_t x,register int32_t rowwidth,register int32_t height)
{
    int32_t y,red,green,blue,dispcolor;
    x += LEFTMARGIN;
    if ( x < 0 || x >= rowwidth )
        return;
    y = conv_yval_to_y(yval * Display_scale,height);
    red = (color>>16) & 0xff;
    green = (color>>8) & 0xff;
    blue = color & 0xff;
    dispcolor = bitmap[y*rowwidth + x];
    red += (dispcolor>>16) & 0xff;
    green += (dispcolor>>8) & 0xff;
    blue += dispcolor & 0xff;
    bitmap[y*rowwidth + x] = pixel_ratios(red,green,blue);
}

void disp_dot(register float radius,register int32_t color,register float yval,register uint32_t *bitmap,register int32_t x,register int32_t rowwidth,register int32_t height)
{
    register float i,j,sq,val;
    if ( radius > 1 )
    {
        sq = radius * radius;
        for (i=-radius; i<=radius; i++)
        {
            for (j=-radius; j<=radius; j++)
            {
                val = ((j*j + i*i) / sq);
                if ( val <= 1. )
                {
                    val = 1. - val;
                    disp_yval(scale_color(color,(100 * val * val * val * val)),yval+j,bitmap,x+i,rowwidth,height);
                }
            }
        }
    }
    else disp_yval(color,yval,bitmap,x,rowwidth,height);
}

void horizline(int32_t calclogflag,int32_t rowwidth,int32_t height,uint32_t *bitmap,double rawprice,double ave)
{
    int32_t x;
    double yval;
    if ( calclogflag != 0 )
        yval = _calc_pricey(log(rawprice),log(ave));
    else yval = _calc_pricey(rawprice,ave);
    for (x=0; x<rowwidth; x++)
        disp_yval(0x888888,yval,bitmap,x,rowwidth,height);
}

void rescale_floats(float *line,int32_t width,double scale)
{
    int32_t i;
    for (i=0; i<width; i++)
        line[i] *= scale;
}

void rescale_doubles(double *line,int32_t width,double scale)
{
    int32_t i;
    for (i=0; i<width; i++)
        line[i] *= scale;
}

double _output_line(int32_t calclogflag,double ave,double *output,double *buf,int32_t n,int32_t color,uint32_t *bitmap,int32_t rowwidth,int32_t height)
{
    int32_t x,nonz = 0;
    double yval,val,aveabs = 0.;
    if ( ave == 0. )
        return(0.);
    if ( calclogflag != 0 )
        ave = log(ave);
    for (x=0; x<n; x++)
    {
        if ( (val= buf[x]) != 0. )
        {
            //if ( calclogflag != 0 )
            {
                val = log(buf[x]);
                if ( ave != 1. )
                    yval = _calc_pricey(val,ave);
                else yval = val;
            } //else yval = (val / ave) * height / 3;
            //printf("(%f -> %f) ",val,yval);
            if ( fabs(yval) > .0000000001 )
            {
                aveabs += fabs(yval);
                nonz++;
                if ( color != 0 )
                    disp_yval(color,yval,bitmap,x,rowwidth,height);
            }
        } else yval = 0.;
        output[x] = yval;
    }
    if ( nonz != 0 )
        aveabs /= nonz;
    return(aveabs);
    //
    //printf("ave %f rowwidth.%d\n",ave,rowwidth);
}

double stats_splineval(struct stats_spline *spline,uint32_t timestamp,int32_t lookahead)
{
    int32_t i,gap,ind = (spline->num - 1);
    if ( timestamp >= spline->utc32[ind] )
    {
        gap = (timestamp - spline->utc32[ind]);
        if ( gap < lookahead )
            return(_extrapolate_spline64(spline->spline64[ind],gap));
        else return(0.);
    }
    else if ( timestamp <= spline->utc32[0] )
    {
        gap = (spline->utc32[0] - timestamp);
        if ( gap < lookahead )
            return(_extrapolate_spline64(spline->spline64[0],gap));
        else return(0.);
    }
    for (i=0; i<spline->num-1; i++)
    {
        ind = (i + spline->lasti) % (spline->num - 1);
        if ( timestamp >= spline->utc32[ind] && timestamp < spline->utc32[ind+1] )
        {
            spline->lasti = ind;
            return(_extrapolate_spline64(spline->spline64[ind],timestamp - spline->utc32[ind]));
        }
    }
    return(0.);
}

double stats_calcspline(struct stats_spline *spline,double *outputs,double *slopes,int32_t dispwidth,uint32_t *utc32,double *splinevals,int32_t num)
{
    static double errsums[3]; static int errcount;
    double c[MAX_SPLINES],f[MAX_SPLINES],dd[MAX_SPLINES],dl[MAX_SPLINES],du[MAX_SPLINES],gaps[MAX_SPLINES];
    int32_t n,i,lasti,x,numsplines,nonz; double vx,vy,vw,vz,gap,sum,xval,yval,abssum,lastval,lastxval,yval64,yval32,yval3; uint32_t gap32;
    sum = lastxval = n = lasti = nonz = 0;
    for (i=0; i<MAX_SPLINES&&i<num; i++)
    {
        if ( (f[n]= splinevals[i]) != 0. && utc32[i] != 0 )
        {
            //printf("i%d.(%u %f) ",i,utc32[i],splinevals[i]);
            //printf("%f ",splinevals[i]);
            if ( n > 0 )
            {
                if ( (gaps[n-1]= utc32[i] - lastxval) < 0 )
                {
                    printf("illegal gap %f to t%d\n",lastxval,utc32[i]);
                    return(0);
                }
            }
            spline->utc32[n] = lastxval = utc32[i];
            n++;
        }
    }
    if ( (numsplines= n) < 4 )
        return(0);
    for (i=0; i<n-3; i++)
        dl[i] = du[i] = gaps[i+1];
    for (i=0; i<n-2; i++)
    {
        dd[i] = 2.0 * (gaps[i] + gaps[i+1]);
        c[i]  = (3.0 / (double)gaps[i+1]) * (f[i+2] - f[i+1]) - (3.0 / (double)gaps[i]) * (f[i+1] - f[i]);
    }
    //for (i=0; i<n; i++)
    //    printf("%f ",f[i]);
    //printf("F2[%d]\n",n);
    dd[0] += (gaps[0] + (double)gaps[0]*gaps[0] / gaps[1]);
    du[0] -= ((double)gaps[0]*gaps[0] / gaps[1]);
    dd[n-3] += (gaps[n-2] + (double)gaps[n-2]*gaps[n-2] / gaps[n-3]);
    dl[n-4] -= ((double)gaps[n-2]*gaps[n-2] / gaps[n-3]);
    
    //tridiagonal(n-2, dl, dd, du, c);
    for (i=0; i<n-1-2; i++)
    {
        du[i] /= dd[i];
        dd[i+1] -= dl[i]*du[i];
    }
    c[0] /= dd[0];
    for (i=1; i<n-2; i++)
        c[i] = (c[i] - dl[i-1] * c[i-1]) / dd[i];
    for (i=n-2-4; i>=0; i--)
        c[i] -= c[i+1] * du[i];
    //tridiagonal(n-2, dl, dd, du, c);
    
    for (i=n-3; i>=0; i--)
        c[i+1] = c[i];
    c[0] = (1.0 + (double)gaps[0] / gaps[1]) * c[1] - ((double)gaps[0] / gaps[1] * c[2]);
    c[n-1] = (1.0 + (double)gaps[n-2] / gaps[n-3] ) * c[n-2] - ((double)gaps[n-2] / gaps[n-3] * c[n-3]);
    //printf("c[n-1] %f, n-2 %f, n-3 %f\n",c[n-1],c[n-2],c[n-3]);
    abssum = nonz = lastval = 0;
    outputs[spline->firstx] = f[0];
    spline->num = numsplines;
    for (i=0; i<n; i++)
    {
        vx = f[i];
        vz = c[i];
        if ( i < n-1 )
        {
            gap = gaps[i];
            vy = ((f[i+1] - f[i]) / gap) - (gap * (c[i+1] + 2.*c[i]) / 3.);
            vw = (c[i+1] - c[i]) / (3. * gap);
        }
        else
        {
            vy = 0;
            vw = 0;
        }
        //printf("%3d: t%u [%14.11f %14.11f %14.11f %14.11f] gap %f | %d\n",i,spline->utc32[i],(vx),vy*1000*1000,vz*1000*1000*1000*1000,vw*1000*1000*1000*1000*1000*1000,gap,conv_unixtime(&tmp,spline->utc32[i]));
        spline->dSplines[i][0] = vx, spline->dSplines[i][1] = vy, spline->dSplines[i][2] = vz, spline->dSplines[i][3] = vw;
        spline->spline64[i][0] = dto64(vx), spline->spline64[i][1] = dto64(vy*1000*1000), spline->spline64[i][2] = dto64(vz*1000*1000*1000*1000), spline->spline64[i][3] = dto64(vw*1000*1000*1000*1000*1000*1000);
        spline->spline32[i][0] = dto32(vx), spline->spline32[i][1] = dto32(vy*1000*1000), spline->spline32[i][2] = dto32(vz*1000*1000*1000*1000), spline->spline32[i][3] = dto32(vw*1000*1000*1000*1000*1000*1000);
        gap32 = gap = spline->dispincr;
        xval = spline->utc32[i] + gap;
        lastval = vx;
        while ( i < n-1 )
        {
            x = spline->firstx + ((xval - spline->utc32[0]) / spline->dispincr);
            if ( x > dispwidth-1 ) x = dispwidth-1;
            if ( x < 0 ) x = 0;
            if ( (i < n-2 && gap > gaps[i] + spline->dispincr) )
                break;
            if ( i == n-2 && xval > spline->utc32[n-1] + MAX_LOOKAHEAD*spline->dispincr )
            {
                //printf("x.%d dispwidth.%d xval %f > utc[n-1] %f + %f\n",x,dispwidth,xval,utc[n-1],MAX_LOOKAHEAD*incr);
                break;
            }
            if ( x >= 0 )
            {
                yval = _extrapolate_Spline(spline->dSplines[i],gap);
                yval64 = _extrapolate_spline64(spline->spline64[i],gap32);
                if ( (yval3 = stats_splineval(spline,gap32 + spline->utc32[i],MAX_LOOKAHEAD*spline->dispincr)) != 0 )
                {
                    yval32 = _extrapolate_spline32(spline->spline32[i],gap32);
                    errsums[0] += fabs(yval - yval64), errsums[1] += fabs(yval - yval32), errsums[2] += fabs(yval - yval3), errcount++;
                    if ( fabs(yval - yval3) > SMALLVAL )
                        printf("(%.10f vs %.10f %.10f %.10f [%.16f %.16f %.16f]) ",yval,yval64,yval32,yval3, errsums[0]/errcount,errsums[1]/errcount,errsums[2]/errcount);
                }
                if ( yval > 5000. ) yval = 5000.;
                else if ( yval < -5000. ) yval = -5000.;
                if ( isnan(yval) == 0 )
                {
                    outputs[x] = yval;
                    spline->lastval = outputs[x], spline->lastutc = xval;
                    if ( 1 && fabs(lastval) > SMALLVAL )
                    {
                        if ( lastval != 0 && outputs[x] != 0 )
                        {
                            if ( slopes != 0 )
                                slopes[x] = (outputs[x] - lastval), abssum += fabs(slopes[x]);
                            nonz++;
                        }
                    }
                }
                //else outputs[x] = 0.;
                //printf("x.%-4d %d %f %f %f i%-4d: gap %9.6f %9.6f last %9.6f slope %9.6f | %9.1f [%9.1f %9.6f %9.6f %9.6f %9.6f]\n",x,firstx,xval,utc[0],incr,i,gap,yval,lastval,slopes[x],xval,utc[i+1],dSplines[i][0],dSplines[i][1]*1000*1000,dSplines[i][2]*1000*1000*1000*1000,dSplines[i][3]*1000*1000*1000*1000*1000*1000);
            }
            gap32 += spline->dispincr, gap += spline->dispincr, xval += spline->dispincr;
        }
        //double pred = (i>0) ? _extrapolate_Spline(dSplines[i-1],gaps[i-1]) : 0.;
        //printf("%2d: w%8.1f [gap %f -> %9.6f | %9.6f %9.6f %9.6f %9.6f %9.6f]\n",i,weekinds[i],gap,pred,f[i],dSplines[i].x,1000000*dSplines[i].y,1000000*1000000*dSplines[i].z,1000000*1000000*1000*dSplines[i].w);
    }
    if ( nonz != 0 )
        abssum /= nonz;
    spline->aveslopeabs = abssum;
    return(lastval);
}

int32_t stats_genspline(double output[2048],double slopes[2048],struct stats_spline *spline,int32_t splineid,char *name,uint32_t *utc32,double *splinevals,int32_t numsplines,double *refvals)
{
    int32_t i; double origvals[MAX_SPLINES];
    if ( numsplines > MAX_SPLINES )
    {
        printf("numsplines.%d > MAX_SPLINES.%d\n",numsplines,MAX_SPLINES);
        return(-1);
    }
    memset(spline,0,sizeof(*spline)), memset(output,0,sizeof(*output)*2048), memset(slopes,0,sizeof(*slopes)*2048);
    spline->dispincr = 3600, spline->basenum = splineid, strcpy(spline->name,name);
    memcpy(origvals,splinevals,sizeof(*splinevals) * MAX_SPLINES);
    spline->lastval = stats_calcspline(spline,output,slopes,2048,utc32,splinevals,numsplines);
    if ( refvals != 0 )
    {
        for (i=0; i<spline->num; i++)
        {
            if ( i < spline->num )
            {
                if ( 0 && refvals[i] != 0 && output[i * 24] != refvals[i] )
                    printf("{%.8f != %.8f}.%d ",output[i * 24],refvals[i],i);
                spline->pricevals[i] = output[i * 24];
            }
        }
    }
    //printf("spline.%s num.%d\n",name,spline->num);
    return(spline->num);
}

void output_line(int32_t calclogflag,double ave,double *buf,int32_t n,int32_t color,uint32_t *bitmap,int32_t rowwidth,int32_t height)
{
    double src[1024],dest[1024]; int32_t i;
    memset(src,0,sizeof(src));
    memset(dest,0,sizeof(dest));
    if ( (1) )
    {
        for (i=0; i<1024; i++)
            src[1023-i] = dest[1023-i] = buf[i];
        smooth1024(dest,src,3);
        for (i=0; i<1024; i++)
            src[1023-i] = dest[i];
    }
    else
    {
        for (i=0; i<1024; i++)
            src[i] = buf[i];
    }
    _output_line(calclogflag,ave,buf,src,1024,color,bitmap,rowwidth,height);
}

void stats_updatedisp(struct DEXstats_disp *disp,double price,double volume)
{
    if ( price > SMALLVAL && volume > SMALLVAL )
    {
        disp->pricesum += (price * volume);
        disp->volumesum += volume;
    }
}

void stats_dispprices(struct DEXstats_disp *prices,int32_t leftdatenum,int32_t numdates,struct DEXstats_datenuminfo *date,char *dest,int32_t current_daysecond)
{
    int32_t i,j,offset,datenum = date->datenum; struct DEXstats_pairinfo *pair; struct DEXstats_pricepoint *ptr; uint32_t timestamp,lefttimestamp,righttimestamp;
    offset = datenum - leftdatenum;
    lefttimestamp = OS_conv_datenum(leftdatenum,0,0,0);
    righttimestamp = OS_conv_datenum(leftdatenum+numdates,0,0,0);
    //printf("search dest.%s datenum.%d vs leftdatenum.%d numdates.%d offset.%d numpairs.%d\n",dest,datenum,leftdatenum,numdates,offset,date->numpairs);
    for (i=0; i<date->numpairs; i++)
    {
        if ( strcmp(dest,date->pairs[i].dest) == 0 )
        {
            pair = &date->pairs[i];
            //printf("found dest.(%s) numprices.%d\n",dest,pair->numprices);
            for (j=0; j<pair->numprices; j++)
            {
                ptr = &pair->prices[j];
                timestamp = OS_conv_datenum(date->datenum,ptr->hour,ptr->seconds/60,ptr->seconds%60);
                timestamp += (24*3600 - current_daysecond);
                offset = (timestamp - lefttimestamp) / (24*3600);
                if ( offset >= 0 && offset < numdates )
                {
                    //printf("found dest.(%s) numprices.%d offset.%d (%.8f %.6f)\n",dest,pair->numprices,offset,ptr->price,ptr->volume);
                    stats_updatedisp(&prices[offset],ptr->price,ptr->volume);
                }
            }
            break;
        }
    }
}

#include "../../crypto777/jpeg/jinclude.h"
#include "../../crypto777/jpeg/jpeglib.h"
#include "../../crypto777/jpeg/jerror.h"

void gen_jpegfile(char *fname,int32_t quality,uint8_t *bitmap,int32_t width,int32_t height)
{
    struct jpeg_compress_struct cinfo;
    struct jpeg_error_mgr jerr;
    FILE * outfile;		/* target file */
    JSAMPROW row_pointer[1];	/* pointer to JSAMPLE row[s] */
    int row_stride;		/* physical row width in image buffer */
    cinfo.err = jpeg_std_error(&jerr);
    jpeg_create_compress(&cinfo);
    if ( (outfile= fopen(fname,"wb")) == NULL)
    {
        fprintf(stderr, "can't open %s\n", fname);
        return;
    }
    jpeg_stdio_dest(&cinfo, outfile);
    cinfo.image_width = width; 	/* image width and height, in pixels */
    cinfo.image_height = height;
    cinfo.input_components = 3;		/* # of color components per pixel */
    cinfo.in_color_space = JCS_RGB; 	/* colorspace of input image */
    jpeg_set_defaults(&cinfo);
    jpeg_set_quality(&cinfo, quality, TRUE /* limit to baseline-JPEG values */);
    jpeg_start_compress(&cinfo, TRUE);
    row_stride = width * 3;	/* JSAMPLEs per row in image_buffer */
    while (cinfo.next_scanline < cinfo.image_height)
    {
        row_pointer[0] = &bitmap[cinfo.next_scanline * row_stride];
        (void) jpeg_write_scanlines(&cinfo, row_pointer, 1);
    }
    jpeg_finish_compress(&cinfo);
    fclose(outfile);
    jpeg_destroy_compress(&cinfo);
}

char *stats_prices(char *symbol,char *dest,struct DEXstats_disp *prices,int32_t leftdatenum,int32_t numdates)
{
    int32_t i,j,n; struct DEXstats_priceinfo *pp; uint32_t *utc32,tmp,timestamp,lefttimestamp,righttimestamp; double *splinevals,total; char fname[1024]; cJSON *retjson,*array,*item;
    timestamp = (uint32_t)time(NULL);
    if ( Num_priceinfos >= sizeof(Prices)/sizeof(*Prices) )
        return(0);
    lefttimestamp = OS_conv_datenum(leftdatenum-1,0,0,0);
    righttimestamp = OS_conv_datenum(leftdatenum+numdates,0,0,0);
    for (i=0; i<Num_priceinfos; i++)
        if ( strcmp(Prices[i].symbol,symbol) == 0 )
        {
            pp = &Prices[i];
            for (j=0; j<=pp->numdates; j++)
            {
                timestamp = OS_conv_datenum(pp->firstdatenum+j,0,0,0);
                if ( timestamp < lefttimestamp ) // can speed up by calculating offset 0
                {
                    //printf("skip (%s) datenums %d %d %d\n",symbol,datenum,pp->firstdatenum,pp->firstdatenum+pp->numdates);
                    continue;
                }
                stats_dispprices(prices,leftdatenum,numdates,&pp->dates[j],dest,timestamp % (3600*24));
            }
            break;
        }
    tmp = OS_conv_datenum(leftdatenum,0,0,0);
    utc32 = calloc(sizeof(*utc32),numdates);
    splinevals = calloc(sizeof(*splinevals),numdates);
    for (total=i=n=0; i<numdates; i++,tmp+=24*3600)
    {
        if ( prices[i].volumesum != 0. )
        {
            total += prices[i].volumesum;
            splinevals[n] = (prices[i].pricesum / prices[i].volumesum);
            utc32[n] = tmp;
            //printf("offset.%d splineval %.8f t%u n.%d\n",i,splinevals[n],tmp,n);
            n++;
        }
    }
    retjson = cJSON_CreateObject();
    jaddstr(retjson,"source",symbol);
    jaddstr(retjson,"dest",dest);
    jaddnum(retjson,"totalvolume",total);
    jaddnum(retjson,"start",leftdatenum);
    jaddnum(retjson,"numdates",numdates);
    if ( n > 3 )
    {
        double output[2048],slopes[2048],sum = 0.; struct stats_spline spline; int32_t splineid = 0;
        memset(&spline,0,sizeof(spline));
        stats_genspline(output,slopes,&spline,splineid,"spline",utc32,splinevals,n,0);
        array = cJSON_CreateArray();
        for (i=0; i<n; i++)
        {
            item = cJSON_CreateArray();
            jaddinum(item,utc32[i]);
            jaddinum(item,splinevals[i]);
            jaddi(array,item);
        }
        jadd(retjson,"splinevals",array);
        array = cJSON_CreateArray();
        for (i=0; i<2048; i++)
        {
            if ( output[i] == 0. )
                break;
            jaddinum(array,output[i]);
            sum += output[i];
        }
        if ( i != 2048 )
            i++;
        sum /= i;
        uint32_t val,height = 400,*bitmap = calloc(sizeof(*bitmap),height * numdates*24);
        uint8_t red,green,blue,*tmpptr,*bytemap = calloc(sizeof(*bytemap),3 * height * numdates*24);
        horizline(1,numdates*24,height,bitmap,sum,sum);
        output_line(1,sum,output,i,0x00ff00,bitmap,numdates*24,height);
        tmpptr = bytemap;
        for (j=0; j<height*numdates*24; j++)
        {
            val = bitmap[j];
            red = val & 0xff;
            green = (val >> 8) & 0xff;
            blue = (val >> 16) & 0xff;
            *tmpptr++ = red;
            *tmpptr++ = green;
            *tmpptr++ = blue;
        }
        sprintf(fname,"%s/bitmaps/%s_%s.jpg",STATS_DESTDIR,symbol,dest), OS_portable_path(fname);
        gen_jpegfile(fname,100,bytemap,numdates*24,height);
        free(bitmap), free(bytemap);
        jaddstr(retjson,"bitmap",fname);
        jadd(retjson,"hourly",array);
        jaddnum(retjson,"average",sum);
    }
    free(utc32);
    free(splinevals);
    return(jprint(retjson,1));
}

#endif /* DEXstats_h */
