# SPDX-License-Identifier: AGPL-3.0-only
#
# Builds kbridge as ONE shared library (kbridge.dll / libkbridge.so) that
# statically contains libchiaki and its bundled third-party code. Rust then
# links a single library and never deals with libchiaki's dependency graph.

cmake_minimum_required(VERSION 3.20)
project(kbridge C)

set(CHIAKI_NG_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../vendor/chiaki-ng" CACHE PATH "Path to the chiaki-ng source tree")
if(NOT EXISTS "${CHIAKI_NG_DIR}/lib/include/chiaki/session.h")
	message(FATAL_ERROR "chiaki-ng not found at ${CHIAKI_NG_DIR}. Run: git submodule update --init --recursive")
endif()

# libchiaki is linked into a shared library, so everything must be PIC.
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# Only the protocol library is needed; chiaki-ng's own GUI, CLI and decoders are not.
set(CHIAKI_ENABLE_GUI OFF CACHE BOOL "" FORCE)
set(CHIAKI_ENABLE_CLI OFF CACHE BOOL "" FORCE)
set(CHIAKI_ENABLE_TESTS OFF CACHE BOOL "" FORCE)
set(CHIAKI_ENABLE_FFMPEG_DECODER OFF CACHE STRING "" FORCE)
set(CHIAKI_ENABLE_PI_DECODER OFF CACHE STRING "" FORCE)
set(CHIAKI_ENABLE_SETSU OFF CACHE STRING "" FORCE)
set(CHIAKI_ENABLE_STEAMDECK_NATIVE OFF CACHE STRING "" FORCE)
set(CHIAKI_ENABLE_SPEEX OFF CACHE STRING "" FORCE)
set(CHIAKI_ENABLE_STEAM_SHORTCUT OFF CACHE BOOL "" FORCE)
set(CHIAKI_LIB_ENABLE_OPUS ON CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)

add_subdirectory("${CHIAKI_NG_DIR}" chiaki-ng EXCLUDE_FROM_ALL)

add_library(kbridge SHARED kbridge.c kbridge.h)
target_compile_definitions(kbridge PRIVATE KBRIDGE_BUILDING=1)
target_link_libraries(kbridge PRIVATE chiaki-lib)
set_target_properties(kbridge PROPERTIES C_VISIBILITY_PRESET hidden)
if(MSVC)
	target_compile_options(kbridge PRIVATE /W3)
else()
	target_compile_options(kbridge PRIVATE -Wall -Wextra -Wno-unused-parameter)
endif()

install(TARGETS kbridge RUNTIME DESTINATION bin LIBRARY DESTINATION lib ARCHIVE DESTINATION lib)
install(FILES kbridge.h DESTINATION include)
