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


//#define KEEPALIVE breaks marketmaker api

#ifndef FROM_JS
#include "OS_portable.h"
//#define LIQUIDITY_PROVIDER 1

/*#define malloc(n) LP_alloc(n)
#define realloc(ptr,n) LP_realloc(ptr,n)
#define calloc(a,b) LP_alloc((uint64_t)(a) * (b))
#define free(ptr) LP_free(ptr)
#define clonestr(str) LP_clonestr(str)

void *LP_realloc(void *ptr,uint64_t len);
void *LP_alloc(uint64_t len);
void LP_free(void *ptr);
char *LP_clonestr(char *str);*/

// TODO: Use the AtomicBool `BITCOIND_RPC_INITIALIZED` instead.
int32_t bitcoind_RPC_inittime;

#if LIQUIDITY_PROVIDER
#include <curl/curl.h>
#include <curl/easy.h>

// return data from the server
#define CURL_GLOBAL_ALL (CURL_GLOBAL_SSL|CURL_GLOBAL_WIN32)
#define CURL_GLOBAL_SSL (1<<0)
#define CURL_GLOBAL_WIN32 (1<<1)

struct return_string {
    char *ptr;
    size_t len;
};

struct MemoryStruct { char *memory; size_t size,allocsize; };

size_t accumulate(void *ptr, size_t size, size_t nmemb, struct return_string *s);
void init_string(struct return_string *s);
static size_t WriteMemoryCallback(void *ptr,size_t size,size_t nmemb,void *data);


/************************************************************************
 *
 * return the current system time in milliseconds
 *
 ************************************************************************/

#define EXTRACT_BITCOIND_RESULT     // if defined, ensures error is null and returns the "result" field
#ifdef EXTRACT_BITCOIND_RESULT

/************************************************************************
 *
 * perform post processing of the results
 *
 ************************************************************************/

char *post_process_bitcoind_RPC(char *debugstr,char *command,char *rpcstr,char *params)
{
    long i,j,len;
    char *retstr = 0;
    cJSON *json,*result,*error;
#ifndef FROM_MARKETMAKER
    //usleep(1000);
#endif
    //printf("<<<<<<<<<<< bitcoind_RPC: %s post_process_bitcoind_RPC.%s.[%s]\n",debugstr,command,rpcstr);
    if ( command == 0 || rpcstr == 0 || rpcstr[0] == 0 )
    {
        if ( strcmp(command,"signrawtransaction") != 0 && strcmp(command,"getrawtransaction") != 0 )
            printf("<<<<<<<<<<< A bitcoind_RPC: %s post_process_bitcoind_RPC.%s\n",debugstr,command);
        return(rpcstr);
    }
    json = cJSON_Parse(rpcstr);
    if ( json == 0 )
    {
        printf("<<<<<<<<<<< B bitcoind_RPC: %s post_process_bitcoind_RPC.%s can't parse.(%s)\n",debugstr,command,rpcstr);
        free(rpcstr);
        return(0);
    }
    result = cJSON_GetObjectItem(json,"result");
    error = cJSON_GetObjectItem(json,"error");
    if ( error != 0 && result != 0 )
    {
        if ( (error->type&0xff) == cJSON_NULL && (result->type&0xff) != cJSON_NULL )
        {
            retstr = jprint(result,0);
            //printf("%s %s rpc retstr.%p\n",command,params,retstr);
            len = strlen(retstr);
            if ( retstr[0] == '"' && retstr[len-1] == '"' )
            {
                for (i=1,j=0; i<len-1; i++,j++)
                    retstr[j] = retstr[i];
                retstr[j] = 0;
            }
        }
        else if ( (error->type&0xff) != cJSON_NULL || (result->type&0xff) != cJSON_NULL )
        {
            if ( strcmp(command,"getrawtransaction") != 0 && strcmp(command,"signrawtransaction") != 0 && strcmp(command,"sendrawtransaction") != 0 )
                printf("<<<<<<<<<<< bitcoind_RPC: %s post_process_bitcoind_RPC (%s) error.%s\n",debugstr,command,rpcstr);
            retstr = rpcstr;
            rpcstr = 0;
        }
        if ( rpcstr != 0 )
        {
            //printf("free rpcstr.%p\n",rpcstr);
            free(rpcstr);
        }
    } else retstr = rpcstr;
    free_json(json);
    //fprintf(stderr,"<<<<<<<<<<< bitcoind_RPC: postprocess returns.(%s)\n",retstr);
    return(retstr);
}
#endif

/************************************************************************
 *
 * perform the query
 *
 ************************************************************************/

//static int32_t USE_JAY;

char *Jay_NXTrequest(char *command,char *params)
{
    char *retstr = 0;
    // issue JS Jay request
    // wait till it is done
    return(retstr);
}


static void bitcoind_init()
{
    static int32_t didinit;
    if ( didinit == 0 )
    {
        didinit = 1;
        curl_global_init(CURL_GLOBAL_ALL); //init the curl session
    }
}

char *bitcoind_RPC(char **retstrp,char *debugstr,char *url,char *userpass,char *command,char *params,int32_t timeout)
{
#ifdef KEEPALIVE
    static
#endif
    CURL *curl_handle = 0;
    static int count,count2; static double elapsedsum,elapsedsum2; extern int32_t USE_JAY;
    struct MemoryStruct chunk;
    struct curl_slist *headers = NULL; struct return_string s; CURLcode res;
    char *bracket0,*bracket1,*retstr,*databuf = 0; long len; int32_t specialcase,numretries; double starttime;
    bitcoind_init();
    numretries = 0;
    if ( debugstr != 0 && strcmp(debugstr,"BTCD") == 0 && command != 0 && strcmp(command,"SuperNET") ==  0 )
        specialcase = 1;
    else specialcase = 0;
    if ( url[0] == 0 )
        strcpy(url,"http://127.0.0.1:7776");
    if ( specialcase != 0 && (0) )
    {
        //int32_t zeroval();
        printf("<<<<<<<<<<< bitcoind_RPC: userpass.(%s) url.(%s) command.(%s) params.(%s)\n",userpass,url,command,params);
        //printf("die.%d\n",1/zeroval());
    }
try_again:
    if ( retstrp != 0 )
        *retstrp = 0;
    starttime = OS_milliseconds();
    if ( curl_handle == 0 )
        curl_handle = curl_easy_init();
    headers = curl_slist_append(0,"Expect:");
    
  	curl_easy_setopt(curl_handle,CURLOPT_USERAGENT,"mozilla/4.0");//"Mozilla/4.0 (compatible; )");
    curl_easy_setopt(curl_handle,CURLOPT_HTTPHEADER,	headers);
    curl_easy_setopt(curl_handle,CURLOPT_URL,		url);
    if ( (0) )
    {
        init_string(&s);
        curl_easy_setopt(curl_handle,CURLOPT_WRITEFUNCTION,	(void *)accumulate); 		// send all data to this function
        curl_easy_setopt(curl_handle,CURLOPT_WRITEDATA,		&s); 			// we pass our 's' struct to the callback
    }
    else
    {
        memset(&chunk,0,sizeof(chunk));
        curl_easy_setopt(curl_handle,CURLOPT_WRITEFUNCTION,WriteMemoryCallback);
        curl_easy_setopt(curl_handle,CURLOPT_WRITEDATA,(void *)&chunk);

    }
    curl_easy_setopt(curl_handle,CURLOPT_NOSIGNAL,		1L);   			// supposed to fix "Alarm clock" and long jump crash
	curl_easy_setopt(curl_handle,CURLOPT_NOPROGRESS,	1L);			// no progress callback
    if ( timeout > 0 )
    {
        if ( bitcoind_RPC_inittime != 0 )
        {
#ifndef _WIN32
            curl_easy_setopt(curl_handle,CURLOPT_TIMEOUT,1);
#else
            curl_easy_setopt(curl_handle, CURLOPT_TIMEOUT_MS, timeout*100);
#endif
        } else curl_easy_setopt(curl_handle,CURLOPT_TIMEOUT,timeout); // causes problems with iguana timeouts
    }
    if ( strncmp(url,"https",5) == 0 )
    {
        curl_easy_setopt(curl_handle,CURLOPT_SSL_VERIFYPEER,0);
        curl_easy_setopt(curl_handle,CURLOPT_SSL_VERIFYHOST,0);
    }
    if ( userpass != 0 )
        curl_easy_setopt(curl_handle,CURLOPT_USERPWD,	userpass);
    databuf = 0;
    if ( params != 0 )
    {
        if ( command != 0 && specialcase == 0 )
        {
            len = strlen(params);
            if ( len > 0 && params[0] == '[' && params[len-1] == ']' ) {
                bracket0 = bracket1 = (char *)"";
            }
            else
            {
                bracket0 = (char *)"[";
                bracket1 = (char *)"]";
            }
            char agentstr[64];
            databuf = (char *)malloc(256 + strlen(command) + strlen(params));
            if ( debugstr[0] != 0 )
                sprintf(agentstr,"\"agent\":\"%s\",",debugstr);
            else agentstr[0] = 0;
            sprintf(databuf,"{\"id\":\"jl777\",%s\"method\":\"%s\",\"params\":%s%s%s}",agentstr,command,bracket0,params,bracket1);
            //printf("url.(%s) userpass.(%s) databuf.(%s)\n",url,userpass,databuf);
            //
        } //else if ( specialcase != 0 ) fprintf(stderr,"databuf.(%s)\n",params);
        curl_easy_setopt(curl_handle,CURLOPT_POST,1L);
        if ( databuf != 0 )
            curl_easy_setopt(curl_handle,CURLOPT_POSTFIELDS,databuf);
        else curl_easy_setopt(curl_handle,CURLOPT_POSTFIELDS,params);
    }
    //laststart = milliseconds();
    res = curl_easy_perform(curl_handle);
    curl_slist_free_all(headers);
#ifndef KEEPALIVE
    curl_easy_cleanup(curl_handle);
    curl_handle = 0;
#endif
    if ( databuf != 0 ) // clean up temporary buffer
    {
        free(databuf);
        databuf = 0;
    }
    retstr = chunk.memory; // retstr = s.ptr;
    if ( res != CURLE_OK )
    {
        numretries++;
        if ( specialcase != 0 || timeout != 0 )
        {
            //printf("<<<<<<<<<<< bitcoind_RPC.(%s): BTCD.%s timeout params.(%s) s.ptr.(%s) err.%d\n",url,command,params,retstr,res);
            free(retstr);
            return(0);
        }
        else if ( numretries >= 4 )
        {
            printf( "curl_easy_perform() failed: %s %s.(%s %s), retries: %d\n",curl_easy_strerror(res),debugstr,url,command,numretries);
            //printf("Maximum number of retries exceeded!\n");
            free(retstr);
            return(0);
        }
        free(retstr);
        sleep((1<<numretries));
        goto try_again;
        
    }
    else
    {
        if ( command != 0 && specialcase == 0 )
        {
            count++;
            elapsedsum += (OS_milliseconds() - starttime);
            if ( (count % 100000) == 0)
                printf("%d: %s ave %9.6f | elapsed %.3f millis | bitcoind_RPC.(%s) url.(%s)\n",count,debugstr,elapsedsum/count,(OS_milliseconds() - starttime),command,url);
            if ( retstrp != 0 )
            {
                *retstrp = retstr;
                return(retstr);
            }
//printf("%s <- %s\n",url,command);
            return(post_process_bitcoind_RPC(debugstr,command,retstr,params));
        }
        else
        {
            if ( (0) && specialcase != 0 )
                fprintf(stderr,"<<<<<<<<<<< bitcoind_RPC: BTCD.(%s) -> (%s)\n",params,retstr);
            count2++;
            elapsedsum2 += (OS_milliseconds() - starttime);
            if ( (count2 % 10000) == 0)
                printf("%d: ave %9.6f | elapsed %.3f millis | NXT calls.(%s) cmd.(%s)\n",count2,elapsedsum2/count2,(double)(OS_milliseconds() - starttime),url,command);
            return(retstr);
        }
    }
}

char *bitcoind_RPCnew(void *curl_handle,char **retstrp,char *debugstr,char *url,char *userpass,char *command,char *params,int32_t timeout)
{
    static int count,count2; static double elapsedsum,elapsedsum2; extern int32_t USE_JAY;
    struct MemoryStruct chunk;
    struct curl_slist *headers = NULL; struct return_string s; CURLcode res;
    char *bracket0,*bracket1,*retstr,*databuf = 0; long len; int32_t flag=0,specialcase,numretries; double starttime;
    bitcoind_init();
    numretries = 0;
    if ( url[0] == 0 )
        strcpy(url,"http://127.0.0.1:7776");
try_again:
    if ( curl_handle == 0 )
    {
        curl_handle = curl_easy_init();
        flag = 1;
    }
    if ( retstrp != 0 )
        *retstrp = 0;
    starttime = OS_milliseconds();
    headers = curl_slist_append(0,"Expect:");
    curl_easy_setopt(curl_handle,CURLOPT_USERAGENT,"mozilla/4.0");//"Mozilla/4.0 (compatible; )");
    curl_easy_setopt(curl_handle,CURLOPT_HTTPHEADER,	headers);
    curl_easy_setopt(curl_handle,CURLOPT_URL,		url);
    if ( (0) )
    {
        init_string(&s);
        curl_easy_setopt(curl_handle,CURLOPT_WRITEFUNCTION,	(void *)accumulate); 		// send all data to this function
        curl_easy_setopt(curl_handle,CURLOPT_WRITEDATA,		&s); 			// we pass our 's' struct to the callback
    }
    else
    {
        memset(&chunk,0,sizeof(chunk));
        curl_easy_setopt(curl_handle,CURLOPT_WRITEFUNCTION,WriteMemoryCallback);
        curl_easy_setopt(curl_handle,CURLOPT_WRITEDATA,(void *)&chunk);
        
    }
    curl_easy_setopt(curl_handle,CURLOPT_NOSIGNAL,		1L);   			// supposed to fix "Alarm clock" and long jump crash
    curl_easy_setopt(curl_handle,CURLOPT_NOPROGRESS,	1L);			// no progress callback
    if ( timeout > 0 )
    {
        if ( bitcoind_RPC_inittime != 0 )
        {
#ifndef _WIN32
            curl_easy_setopt(curl_handle,CURLOPT_TIMEOUT,1);
#else
            curl_easy_setopt(curl_handle, CURLOPT_TIMEOUT_MS, timeout*100);
#endif
        } else curl_easy_setopt(curl_handle,CURLOPT_TIMEOUT,timeout); // causes problems with iguana timeouts
    }
    if ( strncmp(url,"https",5) == 0 )
    {
        curl_easy_setopt(curl_handle,CURLOPT_SSL_VERIFYPEER,0);
        curl_easy_setopt(curl_handle,CURLOPT_SSL_VERIFYHOST,0);
    }
    if ( userpass != 0 )
        curl_easy_setopt(curl_handle,CURLOPT_USERPWD,	userpass);
    databuf = 0;
    if ( params != 0 )
    {
        if ( command != 0 )
        {
            len = strlen(params);
            if ( len > 0 && params[0] == '[' && params[len-1] == ']' ) {
                bracket0 = bracket1 = (char *)"";
            }
            else
            {
                bracket0 = (char *)"[";
                bracket1 = (char *)"]";
            }
            char agentstr[64];
            databuf = (char *)malloc(256 + strlen(command) + strlen(params));
            if ( debugstr[0] != 0 )
                sprintf(agentstr,"\"agent\":\"%s\",",debugstr);
            else agentstr[0] = 0;
            sprintf(databuf,"{\"id\":\"jl777\",%s\"method\":\"%s\",\"params\":%s%s%s}",agentstr,command,bracket0,params,bracket1);
            //printf("url.(%s) userpass.(%s) databuf.(%s)\n",url,userpass,databuf);
            //
        } //else if ( specialcase != 0 ) fprintf(stderr,"databuf.(%s)\n",params);
        curl_easy_setopt(curl_handle,CURLOPT_POST,1L);
        if ( databuf != 0 )
            curl_easy_setopt(curl_handle,CURLOPT_POSTFIELDS,databuf);
        else curl_easy_setopt(curl_handle,CURLOPT_POSTFIELDS,params);
    }
    res = curl_easy_perform(curl_handle);
    curl_slist_free_all(headers);
    if ( databuf != 0 ) // clean up temporary buffer
    {
        free(databuf);
        databuf = 0;
    }
    if ( flag != 0 )
    {
        curl_easy_cleanup(curl_handle);
        curl_handle = 0;
    }
    retstr = chunk.memory; // retstr = s.ptr;
    if ( res != CURLE_OK )
    {
        numretries++;
        if ( timeout != 0 )
        {
            //printf("<<<<<<<<<<< bitcoind_RPC.(%s): BTCD.%s timeout params.(%s) s.ptr.(%s) err.%d\n",url,command,params,retstr,res);
            free(retstr);
            return(0);
        }
        else if ( numretries >= 4 )
        {
            printf( "curl_easy_perform() failed: %s %s.(%s %s), retries: %d\n",curl_easy_strerror(res),debugstr,url,command,numretries);
            //printf("Maximum number of retries exceeded!\n");
            free(retstr);
            return(0);
        }
        free(retstr);
        sleep((1<<numretries));
        goto try_again;
        
    }
    else
    {
        if ( command != 0 )
        {
            count++;
            elapsedsum += (OS_milliseconds() - starttime);
            if ( (count % 100000) == 0)
                printf("%d: ave %9.6f | elapsed %.3f millis | bitcoind_RPC.(%s) url.(%s)\n",count,elapsedsum/count,(OS_milliseconds() - starttime),command,url);
            if ( retstrp != 0 )
            {
                *retstrp = retstr;
                return(retstr);
            }
            //printf("%s <- %s\n",url,command);
            return(post_process_bitcoind_RPC(debugstr,command,retstr,params));
        }
        else
        {
            if ( (0) && specialcase != 0 )
                fprintf(stderr,"<<<<<<<<<<< bitcoind_RPC: BTCD.(%s) -> (%s)\n",params,retstr);
            count2++;
            elapsedsum2 += (OS_milliseconds() - starttime);
            if ( (count2 % 10000) == 0)
                printf("%d: ave %9.6f | elapsed %.3f millis | NXT calls.(%s) cmd.(%s)\n",count2,elapsedsum2/count2,(double)(OS_milliseconds() - starttime),url,command);
            return(retstr);
        }
    }
}

/************************************************************************
 *
 * Initialize the string handler so that it is thread safe
 *
 ************************************************************************/

void init_string(struct return_string *s)
{
    s->len = 0;
    s->ptr = (char *)calloc(1,s->len+1);
    if ( s->ptr == NULL )
    {
        fprintf(stderr,"init_string malloc() failed\n");
        exit(-1);
    }
    s->ptr[0] = '\0';
}

/************************************************************************
 *
 * Use the "writer" to accumulate text until done
 *
 ************************************************************************/

size_t accumulate(void *ptr,size_t size,size_t nmemb,struct return_string *s)
{
    size_t new_len = s->len + size*nmemb;
    s->ptr = (char *)realloc(s->ptr,new_len+1);
    if ( s->ptr == NULL )
    {
        fprintf(stderr, "accumulate realloc() failed\n");
        exit(-1);
    }
    memcpy(s->ptr+s->len,ptr,size*nmemb);
    s->ptr[new_len] = '\0';
    s->len = new_len;
    return(size * nmemb);
}

static size_t WriteMemoryCallback(void *ptr,size_t size,size_t nmemb,void *data)
{
    size_t needed,realsize = (size * nmemb);
    struct MemoryStruct *mem = (struct MemoryStruct *)data;
    needed = mem->size + realsize + 1;
    if ( ptr == 0 && needed < 256 )
    {
        mem->allocsize = 256;
        mem->memory = malloc(mem->allocsize);
    }
    if ( mem->allocsize < needed )
    {
        //printf("curl needs %d more\n",(int32_t)realsize);
        mem->memory = (ptr != 0) ? realloc(mem->memory,needed) : malloc(needed);
        //printf("mem->memory.%p len.%d\n",mem->memory,(int32_t)needed);
        mem->allocsize = needed;
    }
    //mem->memory = (ptr != 0) ? realloc(mem->memory,mem->size + realsize + 1) : malloc(mem->size + realsize + 1);
    if ( mem->memory != 0 )
    {
        if ( ptr != 0 )
            memcpy(&(mem->memory[mem->size]),ptr,realsize);
        mem->size += realsize;
        mem->memory[mem->size] = 0;
    }
    //printf("got %d bytes\n",(int32_t)(size*nmemb));
    return(realsize);
}

void *curl_post(CURL **cHandlep,char *url,char *userpass,char *postfields,char *hdr0,char *hdr1,char *hdr2,char *hdr3)
{
    struct MemoryStruct chunk; CURL *cHandle; long code; struct curl_slist *headers = 0;
    if ( (cHandle= *cHandlep) == NULL )
		*cHandlep = cHandle = curl_easy_init();
    else curl_easy_reset(cHandle);
    //#ifdef DEBUG
	//curl_easy_setopt(cHandle,CURLOPT_VERBOSE, 1);
    //#endif
	curl_easy_setopt(cHandle,CURLOPT_USERAGENT,"mozilla/4.0");//"Mozilla/4.0 (compatible; )");
	curl_easy_setopt(cHandle,CURLOPT_SSL_VERIFYPEER,0);
	//curl_easy_setopt(cHandle,CURLOPT_SSLVERSION,1);
	curl_easy_setopt(cHandle,CURLOPT_URL,url);
  	curl_easy_setopt(cHandle,CURLOPT_CONNECTTIMEOUT,10);
    if ( userpass != 0 && userpass[0] != 0 )
        curl_easy_setopt(cHandle,CURLOPT_USERPWD,userpass);
	if ( postfields != 0 && postfields[0] != 0 )
    {
        curl_easy_setopt(cHandle,CURLOPT_POST,1);
		curl_easy_setopt(cHandle,CURLOPT_POSTFIELDS,postfields);
    }
    if ( hdr0 != NULL && hdr0[0] != 0 )
    {
        //printf("HDR0.(%s) HDR1.(%s) HDR2.(%s) HDR3.(%s)\n",hdr0!=0?hdr0:"",hdr1!=0?hdr1:"",hdr2!=0?hdr2:"",hdr3!=0?hdr3:"");
        headers = curl_slist_append(headers,hdr0);
        if ( hdr1 != 0 && hdr1[0] != 0 )
            headers = curl_slist_append(headers,hdr1);
        if ( hdr2 != 0 && hdr2[0] != 0 )
            headers = curl_slist_append(headers,hdr2);
        if ( hdr3 != 0 && hdr3[0] != 0 )
            headers = curl_slist_append(headers,hdr3);
    } //headers = curl_slist_append(0,"Expect:");
    if ( headers != 0 )
        curl_easy_setopt(cHandle,CURLOPT_HTTPHEADER,headers);
    //res = curl_easy_perform(cHandle);
    memset(&chunk,0,sizeof(chunk));
    curl_easy_setopt(cHandle,CURLOPT_WRITEFUNCTION,WriteMemoryCallback);
    curl_easy_setopt(cHandle,CURLOPT_WRITEDATA,(void *)&chunk);
    curl_easy_perform(cHandle);
    curl_easy_getinfo(cHandle,CURLINFO_RESPONSE_CODE,&code);
    if ( headers != 0 )
        curl_slist_free_all(headers);
    if ( code != 200 )
        printf("(%s) server responded with code %ld (%s)\n",url,code,chunk.memory);
    return(chunk.memory);
}

void curlhandle_free(void *curlhandle)
{
    curl_easy_cleanup(curlhandle);
}

#else
char *bitcoind_RPC(char **retstrp,char *debugstr,char *url,char *userpass,char *command,char *params,int32_t timeout)
{
    return(clonestr("{\"error\":\"curl is disabled\"}"));
}

void *curl_post(void **cHandlep,char *url,char *userpass,char *postfields,char *hdr0,char *hdr1,char *hdr2,char *hdr3)
{
    return(clonestr("{\"error\":\"curl is disabled\"}"));
}
#endif
#endif
