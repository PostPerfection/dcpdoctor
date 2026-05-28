cmake_minimum_required(VERSION 3.25)
project(dcpdoctor
    VERSION 0.5.0
    DESCRIPTION "DCP validator and verifier"
    LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# Dependencies
if(WIN32)
    # Windows: use find_package (vcpkg provides these)
    if(NOT TARGET LibXml2::LibXml2)
      find_package(LibXml2 REQUIRED)
    endif()
    if(NOT TARGET OpenSSL::SSL)
      find_package(OpenSSL REQUIRED)
    endif()
    if(NOT TARGET XercesC::XercesC)
      find_package(XercesC REQUIRED)
    endif()
    find_package(unofficial-sqlite3 CONFIG QUIET)
    if(NOT unofficial-sqlite3_FOUND)
        find_package(SQLite3 REQUIRED)
    endif()
else()
    find_package(PkgConfig REQUIRED)
    if(NOT TARGET PkgConfig::LIBXML2)
      pkg_check_modules(LIBXML2 REQUIRED IMPORTED_TARGET libxml-2.0)
    endif()
    if(NOT TARGET PkgConfig::OPENSSL)
      pkg_check_modules(OPENSSL REQUIRED IMPORTED_TARGET openssl)
    endif()
    if(NOT TARGET PkgConfig::XERCES)
      pkg_check_modules(XERCES REQUIRED IMPORTED_TARGET xerces-c)
    endif()
    if(NOT TARGET PkgConfig::SQLITE3)
      pkg_check_modules(SQLITE3 REQUIRED IMPORTED_TARGET sqlite3)
    endif()
endif()

# CLI11 (header-only)
if(NOT TARGET CLI11::CLI11)
  add_subdirectory(extern/CLI11 EXCLUDE_FROM_ALL)
endif()

# spdlog
if(NOT TARGET spdlog::spdlog)
  add_subdirectory(extern/spdlog EXCLUDE_FROM_ALL)
endif()

# asdcplib
if(NOT TARGET libasdcp)
  set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
  add_subdirectory(extern/asdcplib EXCLUDE_FROM_ALL)
endif()

# Core library
add_library(dcpdoctor_core STATIC
    src/dcp.cpp
    src/assetmap.cpp
    src/pkl.cpp
    src/cpl.cpp
    src/hash.cpp
    src/mxf.cpp
    src/j2k.cpp
    src/bitrate.cpp
    src/audio.cpp
    src/subtitle.cpp
    src/isdcf.cpp
    src/validators.cpp
    src/compliance.cpp
    src/schema_validator.cpp
    src/signature.cpp
    src/report.cpp
    src/note.cpp
    src/preferences.cpp
    src/timeline.cpp
    src/facility_check.cpp
    src/conformance.cpp
    src/dci_ctp.cpp
    src/advanced.cpp
    src/server.cpp
    src/kdm.cpp
    src/diff.cpp
    src/cache.cpp
    src/theater.cpp
    src/fixes.cpp
    src/mxf_advanced.cpp
    src/kdm_advanced.cpp
    src/hfr_stereo.cpp
    src/imf.cpp
    src/quality.cpp
    src/photon.cpp
    src/premium.cpp
    src/studio.cpp
    src/checksum_verify.cpp
    src/mxf_extract.cpp
    src/auto_qc.cpp
    src/validate.cpp
    src/schema_validate.cpp
    src/imf_compliance.cpp
    src/qc.cpp
    src/qc_report.cpp
    src/loudness.cpp
    src/av_sync.cpp
    src/hdr_validate.cpp
    src/frame_compare.cpp
    src/info.cpp)

get_target_property(_ASDCP_SOURCE_DIR libasdcp SOURCE_DIR)
target_include_directories(dcpdoctor_core PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
    ${_ASDCP_SOURCE_DIR})

if(WIN32)
    target_compile_definitions(dcpdoctor_core PUBLIC KM_WIN32)
    target_link_libraries(dcpdoctor_core PUBLIC
        LibXml2::LibXml2
        OpenSSL::SSL OpenSSL::Crypto
        XercesC::XercesC
        $<IF:$<TARGET_EXISTS:unofficial::sqlite3::sqlite3>,unofficial::sqlite3::sqlite3,SQLite::SQLite3>
        spdlog::spdlog
        libasdcp)
else()
    target_link_libraries(dcpdoctor_core PUBLIC
        PkgConfig::LIBXML2
        PkgConfig::OPENSSL
        PkgConfig::XERCES
        PkgConfig::SQLITE3
        spdlog::spdlog
        libasdcp)
endif()

# CLI tool (only when building standalone)
if(CMAKE_PROJECT_NAME STREQUAL PROJECT_NAME)
  add_executable(dcpdoctor src/main.cpp)
  target_link_libraries(dcpdoctor PRIVATE dcpdoctor_core CLI11::CLI11)
endif()

# Tests (only when building standalone)
if(CMAKE_PROJECT_NAME STREQUAL PROJECT_NAME)
  option(BUILD_TESTING "Build tests" ON)
  if(BUILD_TESTING)
    enable_testing()
    add_executable(dcpdoctor_test tests/test_main.cpp)
    target_link_libraries(dcpdoctor_test PRIVATE dcpdoctor_core)
    add_test(NAME dcpdoctor_test COMMAND dcpdoctor_test)
  endif()
endif()

# Python bindings (optional)
option(BUILD_PYTHON_BINDINGS "Build Python/SWIG bindings" OFF)
if(BUILD_PYTHON_BINDINGS)
  add_subdirectory(bindings/python)
endif()
