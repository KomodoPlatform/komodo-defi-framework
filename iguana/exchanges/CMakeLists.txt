include_directories(${CMAKE_CURRENT_SOURCE_DIR})
set(MM_SOURCES mm.c)

if(WIN32)
link_directories(${CMAKE_SOURCE_DIR}/marketmaker_depends/pthread-win32/bin/x64_MSVC2015.Release)
endif()
add_library(marketmaker-lib STATIC ${MM_SOURCES})
if(WIN32)
add_definitions(-DNATIVE_WINDOWS)
endif()
# Link with apple OS libraries
if (APPLE)
    find_library(CoreFoundation CoreFoundation)
    find_library(Security Security)
    target_link_libraries(marketmaker-lib ${CoreFoundation} ${Security} jemalloc)
endif()

if(NOT DEFINED MM_VERSION)
    SET(MM_VERSION UNKNOWN)
endif()

target_compile_definitions(marketmaker-lib PRIVATE -DMM_VERSION="${MM_VERSION}")
