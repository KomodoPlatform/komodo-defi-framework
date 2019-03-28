cmake_minimum_required(VERSION 3.9.2)

project(SuperNET)
set(DEPS_INSTALL_PREFIX ${CMAKE_BINARY_DIR}/install)
set(CMAKE_CXX_STANDARD 11)
set(CMAKE_C_STANDARD 99)
set(CMAKE_CXX_STANDARD_REQUIRED True)
set(CMAKE_CXX_EXTENSIONS Off)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
# We're using a static version of nanomsg, so tell the headers not to try linking with the DLL.
add_definitions(-DNN_STATIC_LIB=ON)

include(cmake/DownloadProject.cmake)

if(NOT(WIN32))
    # Download and install nanomsg at CMake configure time
    download_project(PROJ nanomsg
        GIT_REPOSITORY  https://github.com/nanomsg/nanomsg.git
        GIT_TAG         1.1.2
        GIT_SHALLOW     1
        GIT_PROGRESS    1
        CMAKE_ARGS      "-DCMAKE_POSITION_INDEPENDENT_CODE=ON -DNN_STATIC_LIB=ON -DNN_ENABLE_GETADDRINFO_A=OFF -DNN_ENABLE_DOC=OFF -DNN_TESTS=OFF -DNN_TOOLS=OFF -DNN_ENABLE_NANOCAT=OFF -DCMAKE_INSTALL_PREFIX=${DEPS_INSTALL_PREFIX}"
        UPDATE_DISCONNECTED 1
    )
endif(NOT(WIN32))

find_library(
    NANOMSG_LIBRARY NAMES nanomsg PATHS ${DEPS_INSTALL_PREFIX}/lib NO_DEFAULT_PATH
    HINTS "${CMAKE_SOURCE_DIR}/x64"
)
include_directories("${CMAKE_SOURCE_DIR}")
add_subdirectory(iguana/exchanges)
add_subdirectory(crypto777)
add_subdirectory(crypto777/jpeg)