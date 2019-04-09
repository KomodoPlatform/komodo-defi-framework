@echo off
rem [ Decker] Automatically download and build depends script for marketmaker.
rem
rem 1. Requires installed CMake for Windows (!)
rem 2. Currently build only 64-bit release versions of .lib and .dll
rem 3. Libraries available: pthreads, curl

if exist "c:\Program Files (x86)\Microsoft Visual Studio\2017\BuildTools\VC\Auxiliary\Build\vcvars64.bat" (
 call "c:\Program Files (x86)\Microsoft Visual Studio\2017\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
)

@REM set up the environment, https://www.appveyor.com/docs/lang/cpp/#visual-studio-2017
if exist "C:\Program Files (x86)\Microsoft Visual Studio\2017\Community\VC\Auxiliary\Build\vcvars64.bat" (
 call "C:\Program Files (x86)\Microsoft Visual Studio\2017\Community\VC\Auxiliary\Build\vcvars64.bat"
)

rem MSBuild /help
echo.
echo Decker will automatically download and build all needed *.dll and *.lib for you ;)

mkdir marketmaker_depends
mkdir x64

rem TODO: Download automatically from build.rs.
rem NB: "marketmaker_depends" is cached between the AppVeyor builds!
rem --- pthreads ---
:compile_pthreads
if not exist marketmaker_depends\pthreadVC2.dll (
    rem NB: This library depends on `msvcr100.dll`.
    rem     In the future we should probably get rid of it entirely, by starting all threads from Rust.
    curl ftp://sourceware.org/pub/pthreads-win32/prebuilt-dll-2-9-1-release/dll/x64/pthreadVC2.dll -o marketmaker_depends/pthreadVC2.dll
    curl ftp://sourceware.org/pub/pthreads-win32/prebuilt-dll-2-9-1-release/lib/x64/pthreadVC2.lib -o marketmaker_depends/pthreadVC2.lib
)
copy marketmaker_depends\pthreadVC2.dll x64\pthreadVC2.dll
copy marketmaker_depends\pthreadVC2.lib x64\pthreadVC2.lib

rem Show what we've got here, in case we'd want to clear the cached folder, etc.
echo marketmaker_build_depends] dir marketmaker_depends
dir marketmaker_depends
