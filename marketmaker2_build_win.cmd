@echo off
rem (c) Decker

echo Windows version and architecture:
ver
wmic os get osarchitecture

set MM_VERSION=%APPVEYOR_BUILD_VERSION%
echo MM_VERSION: %MM_VERSION%

echo [#1] Install Rust, build curl and pthreads ...
call marketmaker_build_depends.cmd

echo [#2] Build MM1 and MM2.

rem Increased verbosity here allows us to see the MM1 CMake logs.
rem And according to https://www.appveyor.com/docs/windows-images-software/#mingw-msys-cygwin
rem we can use the GNU grep from MSYS to fine-tune.
cargo build --features native -vv --color never 2>&1 | grep --line-buffered -v '     Running `rustc --'
