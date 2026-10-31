cmake_minimum_required(VERSION 3.16)
project(homa_ffi_client C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

# Locations, overridable:
#   cmake -DHOMA_FFI_INCLUDE_DIR=... -DHOMA_FFI_LIB_DIR=...
if(NOT HOMA_FFI_INCLUDE_DIR)
    set(HOMA_FFI_INCLUDE_DIR
        "${CMAKE_CURRENT_LIST_DIR}/../../crates/homa-ffi/include")
endif()
if(NOT HOMA_FFI_LIB_DIR)
    if(EXISTS "${CMAKE_CURRENT_LIST_DIR}/../../target/release/libhoma_ffi.so"
       OR EXISTS "${CMAKE_CURRENT_LIST_DIR}/../../target/release/libhoma_ffi.dylib")
        set(HOMA_FFI_LIB_DIR "${CMAKE_CURRENT_LIST_DIR}/../../target/release")
    else()
        set(HOMA_FFI_LIB_DIR "${CMAKE_CURRENT_LIST_DIR}/../../target/debug")
    endif()
endif()

add_executable(ffi-client main.c)
target_include_directories(ffi-client PRIVATE "${HOMA_FFI_INCLUDE_DIR}")
target_link_directories(ffi-client PRIVATE "${HOMA_FFI_LIB_DIR}")
target_link_libraries(ffi-client PRIVATE homa_ffi)
# rpath so the binary finds libhoma_ffi.so next to the cargo output
# without requiring LD_LIBRARY_PATH.
set_target_properties(ffi-client PROPERTIES
    BUILD_RPATH "${HOMA_FFI_LIB_DIR}"
    INSTALL_RPATH "${HOMA_FFI_LIB_DIR}")

if(UNIX AND NOT APPLE)
    target_link_libraries(ffi-client PRIVATE dl pthread m)
endif()

message(STATUS "homa_ffi include: ${HOMA_FFI_INCLUDE_DIR}")
message(STATUS "homa_ffi lib dir: ${HOMA_FFI_LIB_DIR}")
