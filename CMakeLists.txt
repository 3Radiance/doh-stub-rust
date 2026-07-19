cmake_minimum_required(VERSION 3.16)
project(dohstub CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_executable(dohstub src/main.cpp)

if(WIN32)
    target_link_libraries(dohstub ws2_32 winhttp)
endif()
