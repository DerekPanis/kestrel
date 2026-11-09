# Standard vcpkg x64-windows triplet, plus one override:
# GitHub's Windows runners ship CMake 4, which refuses projects that declare
# cmake_minimum_required < 3.5 (e.g. json-c at the pinned vcpkg baseline).
# CMAKE_POLICY_VERSION_MINIMUM makes CMake 4 configure them in 3.5 mode.
set(VCPKG_TARGET_ARCHITECTURE x64)
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_LIBRARY_LINKAGE dynamic)
list(APPEND VCPKG_CMAKE_CONFIGURE_OPTIONS "-DCMAKE_POLICY_VERSION_MINIMUM=3.5")
