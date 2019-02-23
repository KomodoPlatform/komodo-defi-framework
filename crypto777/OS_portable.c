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

#include "OS_portable.h"
//#include <sys/stat.h>

#ifdef __unix__
# include <pthread.h>
#endif

struct OS_memspace
{
    portable_mutex_t mutex; long used,totalsize; struct OS_mappedptr M; char name[64]; void *ptr;
    int32_t alignflag,counter,maxheight,openfiles,lastcounter,threadsafe,allocated:1,mapped:1,external:1;
#ifdef IGUANA_PEERALLOC
    int32_t outofptrs,numptrs,availptrs;
    void *ptrs[4096]; int32_t allocsizes[4096],maxsizes[4096];
#endif
};

void *OS_filealloc(struct OS_mappedptr *M,char *fname,struct OS_memspace *mem,long size);

#ifndef MAP_FILE
#define MAP_FILE        0
#endif

void OS_portable_init()
{
#ifdef _WIN32
    OS_nonportable_init();
#endif
}

// from tweetnacl
void OS_portable_randombytes(unsigned char *x,long xlen)
{
#ifdef _WIN32
    OS_nonportable_randombytes(x,xlen);
#else
    static int fd = -1;
    int32_t i;
    if (fd == -1) {
        for (;;) {
            fd = open("/dev/urandom",O_RDONLY);
            if (fd != -1) break;
            sleep(1);
        }
    }
    while (xlen > 0) {
        if (xlen < 1048576) i = (int32_t)xlen; else i = 1048576;
        i = (int32_t)read(fd,x,i);
        if (i < 1) {
            sleep(1);
            continue;
        }
        if ( (0) )
        {
            int32_t j;
            for (j=0; j<i; j++)
                printf("%02x ",x[j]);
            printf("-> %p\n",x);
        }
        x += i;
        xlen -= i;
    }
#endif
}

int32_t OS_portable_truncate(char *fname,long filesize)
{
#ifdef _WIN32
    printf("need to implement truncate()\n");
    return(-1);
#else
    return(truncate(fname,filesize));
#endif
}

char *OS_portable_path(char *str)
{
#ifdef _WIN32
    char *OS_nonportable_path(char *str);
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

FILE *OS_appendfile(char *origfname)
{
    char fname[1024]; FILE *fp;
    strcpy(fname,origfname);
    OS_portable_path(fname);
    if ( (fp= fopen(fname,"rb+")) == 0 )
        fp = fopen(fname,"wb");
    else fseek(fp,0,SEEK_END);
    return(fp);
}

int32_t OS_portable_renamefile(char *fname,char *newfname)
{
#ifdef _WIN32
	return(OS_nonportable_renamefile(fname,newfname));
#else
    return(rename(fname,newfname));
#endif
}

int32_t OS_portable_removefile(char *fname)
{
#ifdef _WIN32
    return(OS_nonportable_removefile(fname));
#else
    return(remove(fname));
#endif
    return(-1);
}

int32_t OS_portable_rmdir(char *dirname,int32_t diralso)
{
    char cmdstr[1024],tmp[512]; //int32_t i;
    strcpy(tmp,dirname);
    OS_portable_path(tmp);
#ifdef _WIN32
    sprintf(cmdstr,"rmdir %s",tmp);
    if ( system(cmdstr) != 0 )
        printf("error deleting dir.(%s)\n",cmdstr);
    else return(1);
#else
    if ( diralso != 0 )
    {
        sprintf(cmdstr,"rm -rf %s",tmp);
        if ( system(cmdstr) != 0 )
            printf("error deleting dir.(%s)\n",cmdstr);
        //sprintf(cmdstr,"rmdir %s",tmp);
        //if ( system(cmdstr) != 0 )
        //    printf("error deleting dir.(%s)\n",cmdstr);
    }
    else
    {
        //for (i=0; i<=16; i++)
        {
            //if ( i < 16 )
            //    sprintf(cmdstr,"rm %s/%c*",tmp,i<10?'0'+i:'a'-10+i);
            //else sprintf(cmdstr,"rm %s/*",tmp);
            sprintf(cmdstr,"rm -rf %s",tmp);
            if ( system(cmdstr) != 0 )
                printf("error deleting dir.(%s)\n",cmdstr);
        }
    }
    return(0);
#endif
    return(-1);
}

void *OS_portable_mapfile(char *fname,long *filesizep,int32_t enablewrite)
{
#ifdef _WIN32
    void *OS_nonportable_mapfile(char *fname,long *filesizep,int32_t enablewrite);
    return(OS_nonportable_mapfile(fname,filesizep,enablewrite));
#else
	int32_t fd,rwflags,flags = MAP_FILE|MAP_SHARED;
	long filesize;
    void *ptr = 0;
	*filesizep = 0;
	if ( enablewrite != 0 )
		fd = open(fname,O_RDWR);
	else fd = open(fname,O_RDONLY);
	if ( fd < 0 )
	{
		//printf("map_file: error opening enablewrite.%d %s\n",enablewrite,fname);
        return(0);
	}
    if ( *filesizep == 0 )
        filesize = (long)lseek(fd,0,SEEK_END);
    else filesize = *filesizep;
	rwflags = PROT_READ;
	if ( enablewrite != 0 )
		rwflags |= PROT_WRITE;
//#if __i386__ || _WIN32 || __PNACL
	ptr = mmap(0,filesize,rwflags,flags,fd,0);
//#else
//	void *mmap64(void *addr,size_t len,int32_t prot,int32_t flags,int32_t fildes,off_t off);
//	ptr = mmap64(0,filesize,rwflags,flags,fd,0);
//#endif
	close(fd);
    if ( ptr == 0 || ptr == MAP_FAILED )
	{
		printf("map_file.write%d: mapping %s failed? mp %p\n",enablewrite,fname,ptr);
		return(0);
	}
	*filesizep = filesize;
    //printf("mapped rw.%d %ld -> %s\n",enablewrite,(long)filesize,fname);
	return(ptr);
#endif
}

int32_t OS_portable_syncmap(struct OS_mappedptr *mp,long len)
{
/*#ifndef __PNACL
	int32_t err = -1;
	if ( mp->actually_allocated != 0 )
		return(0);
    if ( mp->fileptr != 0 && mp->dirty != 0 )
    {
        if ( len == 0 )
            len = mp->allocsize;
        err = msync(mp->fileptr,len,MS_SYNC);
        if ( err != 0 )
            printf("sync (%s) len %llu, err %d errno.%d\n",mp->fname,(long long)len,err,errno);
        //Sync_total += len;
        mp->dirty = 0;
    }
	return(err);
#else
    return(OS_nonportable_syncmap(mp,len));
#endif*/
    return(-1);
}

void *OS_portable_tmpalloc(char *dirname,char *name,struct OS_memspace *mem,long origsize)
{
#ifdef __PNACL
    return(OS_nonportable_tmpalloc(dirname,name,mem,origsize));
#else
    char fname[1024]; void *ptr; long size;
    if ( mem->threadsafe != 0 )
        portable_mutex_lock(&mem->mutex);
    if ( origsize != 0 && (mem->M.fileptr == 0 || (mem->used + origsize) > mem->totalsize) )
    {
        //coin->TMPallocated += origsize;
        memset(&mem->M,0,sizeof(mem->M));
        sprintf(fname,"tmp/%s/%s.%d",dirname,name,mem->counter), OS_compatible_path(fname);
        mem->counter++;
        if ( mem->totalsize == 0 )
        {
            mem->totalsize = (1024 * 1024 * 16);
        }
        //if ( coin->R.RSPACE.size == 0 )
        //    coin->R.RSPACE.size = mem->size;
        if ( mem->totalsize > origsize )
            size = mem->totalsize;
        else size = origsize;
        printf("filealloc.(%s) -> ",fname);
        if ( OS_filealloc(&mem->M,fname,mem,size) == 0 )
        {
            printf("couldnt map tmpfile %s\n",fname);
            return(0);
        }
        printf("created\n");
    }
    ptr = iguana_memalloc(mem,origsize,1);
    if ( mem->threadsafe != 0 )
        portable_mutex_unlock(&mem->mutex);
    return(ptr);
#endif
}
