cmake_minimum_required(VERSION 3.21)
project(clap_wrapper_builder)
# enable_language(OBJCXX)

# ---------------------------------------------------------------------------
# clap_wrapper_builder
#
# Wraps a prebuilt CLAP plugin static library into VST3 / AUv2 / AAX /
# Standalone bundles. The caller supplies a static library plus a few cache
# variables; this file owns everything from there (entry-point shim, per-OS
# system-library linkage, bundle identifiers, artifact staging).
#
# Input contract (pass with -D on the cmake command line):
#
#   Required
#     CLAP_WRAPPER_BUILDER_TARGET_LIB   Path to the static library to wrap.
#                                       Plugin name is derived from its file
#                                       name (leading "lib" stripped) unless
#                                       overridden below.
#
#   Naming / metadata (optional)
#     CLAP_WRAPPER_BUILDER_TARGET_NAME      Internal CMake target name. Defaults
#                                           to the derived plugin name.
#     CLAP_WRAPPER_BUILDER_OUTPUT_NAME      Display/bundle name. Overrides the
#                                           name derived from TARGET_LIB.
#     CLAP_WRAPPER_BUILDER_BUNDLE_VERSION   Version written into bundles.
#                                           Default "1.0.0".
#     CLAP_WRAPPER_BUILDER_STAGE_DIR        If set, finished artifacts are
#                                           copied here under fixed names.
#                                           Leave empty to skip staging.
#
#   Products
#     CLAP_WRAPPER_BUILDER_PRODUCT_COUNT    Number of plugin products exposed by
#                                           the wrapped static library. Default 1.
#     CLAP_WRAPPER_BUILDER_PRODUCT_<N>_OUTPUT_NAME
#                                           Product name used for AUv2 output.
#     CLAP_WRAPPER_BUILDER_PRODUCT_<N>_PLUGIN_ID
#                                           CLAP plugin ID used by standalone.
#     CLAP_WRAPPER_BUILDER_PRODUCT_<N>_STANDALONE_NAME
#                                           Standalone app output name.
#     CLAP_WRAPPER_BUILDER_PRODUCT_<N>_AUV2_TYPE
#     CLAP_WRAPPER_BUILDER_PRODUCT_<N>_AUV2_SUBTYPE
#                                           Product AUv2 type/subtype codes.
#
#   Format toggles
#     CLAP_WRAPPER_BUILDER_BUILD_VST3        Default ON.
#     CLAP_WRAPPER_BUILDER_BUILD_AUV2        Default ON on macOS, OFF elsewhere.
#     CLAP_WRAPPER_BUILDER_BUILD_AAX         Default OFF. Requires the Avid AAX
#                                            SDK; only meaningful on macOS/Win.
#     CLAP_WRAPPER_BUILDER_BUILD_STANDALONE  Default OFF.
#
# clap-wrapper is vendored as a git subtree. The clap, vst3sdk, and
# AudioUnitSDK dependency SDKs live alongside this file as git submodules.
# ---------------------------------------------------------------------------

# Require C++23 or later (required by clap-wrapper and AudioUnitSDK)
set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# The wrapped library ends up inside shared plugin bundles (.vst3/.so/.dylib),
# so every object on the link line must be position-independent.
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
# 11.0 is the floor the Rust plugin staticlib is built against; keep the C++
# side from targeting a newer minimum and producing a mismatched bundle.
if(APPLE AND (NOT DEFINED CMAKE_OSX_DEPLOYMENT_TARGET OR "${CMAKE_OSX_DEPLOYMENT_TARGET}" STREQUAL ""))
    if(DEFINED ENV{MACOSX_DEPLOYMENT_TARGET} AND NOT "$ENV{MACOSX_DEPLOYMENT_TARGET}" STREQUAL "")
        set(CMAKE_OSX_DEPLOYMENT_TARGET "$ENV{MACOSX_DEPLOYMENT_TARGET}" CACHE STRING "macOS deployment target" FORCE)
    else()
        set(CMAKE_OSX_DEPLOYMENT_TARGET 11.0 CACHE STRING "macOS deployment target" FORCE)
    endif()
endif()

set(CLAP_SDK_ROOT "${CMAKE_CURRENT_LIST_DIR}/clap")

# Rust staticlibs link the static MSVC CRT; the C++ side must match it or the
# final wrapper hits duplicate/conflicting CRT symbols at link time.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")

# Receive the static library path as a cache variable
set(CLAP_WRAPPER_BUILDER_TARGET_LIB "" CACHE FILEPATH "Path to the static library to wrap")
set(CLAP_WRAPPER_BUILDER_TARGET_NAME "" CACHE STRING "Internal CMake target name for the wrapper")
set(CLAP_WRAPPER_BUILDER_OUTPUT_NAME "" CACHE STRING "Output plugin display/bundle name")
set(CLAP_WRAPPER_BUILDER_STAGE_DIR "" CACHE PATH "Directory where built artifacts are staged")
set(CLAP_WRAPPER_BUILDER_BUNDLE_VERSION "1.0.0" CACHE STRING "Version written into wrapped plugin bundles")
set(CLAP_WRAPPER_BUILDER_PRODUCT_COUNT "1" CACHE STRING "Number of plugin products exposed by the wrapped library")
option(CLAP_WRAPPER_BUILDER_BUILD_STANDALONE "Build standalone app target in clap_wrapper_builder" OFF)
option(CLAP_WRAPPER_BUILDER_BUILD_VST3 "Build VST3 target in clap_wrapper_builder" ON)
if(APPLE)
    option(CLAP_WRAPPER_BUILDER_BUILD_AUV2 "Build AUv2 target in clap_wrapper_builder" ON)
else()
    option(CLAP_WRAPPER_BUILDER_BUILD_AUV2 "Build AUv2 target in clap_wrapper_builder" OFF)
endif()
if(APPLE OR WIN32)
    option(CLAP_WRAPPER_BUILDER_BUILD_AAX "Build AAX target in clap_wrapper_builder" OFF)
else()
    option(CLAP_WRAPPER_BUILDER_BUILD_AAX "Build AAX target in clap_wrapper_builder" OFF)
endif()

# Validate required parameters
if(NOT CLAP_WRAPPER_BUILDER_TARGET_LIB)
    message(FATAL_ERROR "CLAP_WRAPPER_BUILDER_TARGET_LIB must be specified. Use: -DCLAP_WRAPPER_BUILDER_TARGET_LIB=/path/to/your/plugin.a")
endif()

if(NOT EXISTS "${CLAP_WRAPPER_BUILDER_TARGET_LIB}")
    message(FATAL_ERROR "Static library not found: ${CLAP_WRAPPER_BUILDER_TARGET_LIB}")
endif()

# Derive plugin name from the static library filename
get_filename_component(PLUGIN_NAME "${CLAP_WRAPPER_BUILDER_TARGET_LIB}" NAME_WE)
string(REGEX REPLACE "^lib" "" PLUGIN_NAME "${PLUGIN_NAME}")
if(CLAP_WRAPPER_BUILDER_OUTPUT_NAME)
    set(PLUGIN_NAME "${CLAP_WRAPPER_BUILDER_OUTPUT_NAME}")
endif()
string(REGEX REPLACE "[^A-Za-z0-9_]" "_" PLUGIN_NAME_SAFE "${PLUGIN_NAME}")
string(REPLACE "_" "-" PLUGIN_NAME_BUNDLE ${PLUGIN_NAME_SAFE})
if(CLAP_WRAPPER_BUILDER_TARGET_NAME)
    string(REGEX REPLACE "[^A-Za-z0-9_]" "_" PLUGIN_NAME_SAFE "${CLAP_WRAPPER_BUILDER_TARGET_NAME}")
endif()

message(STATUS "CLAP Wrapper Builder Configuration:")
message(STATUS "  Target Library: ${CLAP_WRAPPER_BUILDER_TARGET_LIB}")
message(STATUS "  Plugin Name: ${PLUGIN_NAME}")
message(STATUS "  Safe Plugin Name: ${PLUGIN_NAME_SAFE}")
message(STATUS "  Stage Dir: ${CLAP_WRAPPER_BUILDER_STAGE_DIR}")
message(STATUS "  Build AAX: ${CLAP_WRAPPER_BUILDER_BUILD_AAX}")
message(STATUS "  Product Count: ${CLAP_WRAPPER_BUILDER_PRODUCT_COUNT}")
if(CLAP_WRAPPER_BUILDER_PRODUCT_COUNT LESS 1)
    message(FATAL_ERROR "CLAP_WRAPPER_BUILDER_PRODUCT_COUNT must be at least 1")
endif()
math(EXPR PRODUCT_LAST_INDEX "${CLAP_WRAPPER_BUILDER_PRODUCT_COUNT} - 1")

# Add the clap-wrapper subproject
add_subdirectory(clap-wrapper)

# clap-wrapper builds with -Werror on Apple; its own AUv2 code trips
# -Wshorten-64-to-32, so relax just that warning to keep the build green.
if(APPLE AND TARGET clap-wrapper-compile-options)
  target_compile_options(clap-wrapper-compile-options INTERFACE -Wno-shorten-64-to-32)
endif()

function(clap_wrapper_builder_product_value INDEX FIELD OUT_VAR)
    set(var_name "CLAP_WRAPPER_BUILDER_PRODUCT_${INDEX}_${FIELD}")
    if(DEFINED ${var_name} AND NOT "${${var_name}}" STREQUAL "")
        set(${OUT_VAR} "${${var_name}}" PARENT_SCOPE)
    else()
        set(${OUT_VAR} "" PARENT_SCOPE)
    endif()
endfunction()

function(clap_wrapper_builder_product_output_name INDEX OUT_VAR)
    clap_wrapper_builder_product_value(${INDEX} "OUTPUT_NAME" value)
    if(value)
        set(${OUT_VAR} "${value}" PARENT_SCOPE)
    else()
        message(FATAL_ERROR "CLAP_WRAPPER_BUILDER_PRODUCT_${INDEX}_OUTPUT_NAME must be specified")
    endif()
endfunction()

function(clap_wrapper_builder_product_target_name INDEX OUTPUT_NAME OUT_VAR)
    string(MAKE_C_IDENTIFIER "${OUTPUT_NAME}" product_name_safe)
    # Product display names are user-facing and not guaranteed to be unique after
    # CMake identifier sanitization. Keep the metadata order in target names so
    # two products cannot collapse into the same wrapper target.
    set(${OUT_VAR} "${PLUGIN_NAME_SAFE}_product_${INDEX}_${product_name_safe}" PARENT_SCOPE)
endfunction()

# Create the static library target
# Set up the external static library as an imported library
add_library(${PLUGIN_NAME_SAFE}_impl STATIC IMPORTED)
set_target_properties(${PLUGIN_NAME_SAFE}_impl PROPERTIES
    IMPORTED_LOCATION "${CLAP_WRAPPER_BUILDER_TARGET_LIB}"
)

if(MSVC)
  set_target_properties(${PLUGIN_NAME_SAFE}_impl PROPERTIES
      # Rust staticlibs do not carry native system-library dependencies into
      # the final wrapper link, so keep the common Windows runtime deps explicit.
      INTERFACE_LINK_LIBRARIES "ntdll.lib;ws2_32.lib;userenv.lib;bcrypt.lib;runtimeobject.lib;psapi.lib"
  )
endif()

# The Rust plugin renders its UI through a system webview. Rust staticlibs do
# not carry these native system deps into the wrapper link (same reason as the
# MSVC block above), so declare the webview/UI frameworks per platform here.
if(APPLE)
  set_target_properties(${PLUGIN_NAME_SAFE}_impl PROPERTIES
      INTERFACE_LINK_LIBRARIES "-framework AppKit;-framework Foundation;-framework WebKit"
  )
endif()

if(UNIX AND NOT APPLE)
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(LINUX_PLUGIN_DEPS REQUIRED IMPORTED_TARGET webkit2gtk-4.1 gtk+-3.0 gdk-x11-3.0)
  set_target_properties(${PLUGIN_NAME_SAFE}_impl PROPERTIES
      INTERFACE_LINK_LIBRARIES PkgConfig::LINUX_PLUGIN_DEPS
  )
endif()
# Generate the source file for the CLAP entry point.
# The CLAP entry point itself is provided by the plugin binary and lives
# inside CLAP_WRAPPER_BUILDER_TARGET_LIB. Calling get_clap_entry() from here
# prevents the clap_entry symbol inside CLAP_WRAPPER_BUILDER_TARGET_LIB from
# being stripped, ensuring it is correctly retained in the DLL.
set(ENTRY_SOURCE_FILE "${CMAKE_CURRENT_BINARY_DIR}/${PLUGIN_NAME_SAFE}_entry.cpp")
file(WRITE ${ENTRY_SOURCE_FILE} "
#include <clap/clap.h>
extern \"C\" {
  clap_plugin_entry_t get_clap_entry();
  const clap_plugin_entry_t internal_clap_entry = get_clap_entry();
}

// for Windows only
#if defined(_MSC_VER)
#pragma comment(linker,  \"/export:clap_entry\")
#endif
")

# Configure bundle plugin formats to build. CLAP, VST3, and AUv2 can expose
# every product in one bundle through the CLAP factory descriptors.
set(PLUGIN_FORMATS "CLAP")
if(CLAP_WRAPPER_BUILDER_BUILD_VST3)
    list(APPEND PLUGIN_FORMATS "VST3")
endif()
if(APPLE AND CLAP_WRAPPER_BUILDER_BUILD_AUV2)
    list(APPEND PLUGIN_FORMATS "AUV2")
endif()
if(CLAP_WRAPPER_BUILDER_BUILD_AAX)
    list(APPEND PLUGIN_FORMATS "AAX")
endif()

# Plugin bundle metadata
set(BUNDLE_ID "org.wrapper.${PLUGIN_NAME_BUNDLE}")
set(BUNDLE_VERSION "${CLAP_WRAPPER_BUILDER_BUNDLE_VERSION}")

# Create bundle-level plugins using make_clapfirst_plugins. CLAP and VST3 can
# expose every product in one bundle through the CLAP factory descriptors.
set(CLAPFIRST_COMMON_ARGS
    TARGET_NAME ${PLUGIN_NAME_SAFE}
    IMPL_TARGET ${PLUGIN_NAME_SAFE}_impl
    OUTPUT_NAME "${PLUGIN_NAME}"
    ENTRY_SOURCE ${ENTRY_SOURCE_FILE}
    PLUGIN_FORMATS ${PLUGIN_FORMATS}
    BUNDLE_IDENTIFIER "${BUNDLE_ID}"
    BUNDLE_VERSION "${BUNDLE_VERSION}"
    # We stage artifacts ourselves into STAGE_DIR below, so suppress
    # clap-wrapper's own install-to-system-folders step.
    COPY_AFTER_BUILD FALSE
    WINDOWS_FOLDER_VST3 TRUE
)

set(STANDALONE_TARGETS)
if(CLAP_WRAPPER_BUILDER_BUILD_STANDALONE)
    message(STATUS "  Standalone Enabled: ON")
    set(STANDALONE_CONFIGURATIONS)
    # Standalone hosts load exactly one CLAP plugin ID, so each product needs its
    # own app target even when all products come from the same static library.
    foreach(PRODUCT_INDEX RANGE 0 ${PRODUCT_LAST_INDEX})
        clap_wrapper_builder_product_output_name(${PRODUCT_INDEX} PRODUCT_OUTPUT_NAME)
        clap_wrapper_builder_product_value(${PRODUCT_INDEX} "PLUGIN_ID" PRODUCT_PLUGIN_ID)
        clap_wrapper_builder_product_value(${PRODUCT_INDEX} "STANDALONE_NAME" PRODUCT_STANDALONE_NAME)
        if(NOT PRODUCT_PLUGIN_ID)
            message(FATAL_ERROR "CLAP_WRAPPER_BUILDER_PRODUCT_${PRODUCT_INDEX}_PLUGIN_ID must be specified when CLAP_WRAPPER_BUILDER_BUILD_STANDALONE=ON")
        endif()
        if(NOT PRODUCT_STANDALONE_NAME)
            message(FATAL_ERROR "CLAP_WRAPPER_BUILDER_PRODUCT_${PRODUCT_INDEX}_STANDALONE_NAME must be specified when CLAP_WRAPPER_BUILDER_BUILD_STANDALONE=ON")
        endif()
        set(STANDALONE_POSTFIX "product_${PRODUCT_INDEX}_standalone")
        set(STANDALONE_TARGET "${PLUGIN_NAME_SAFE}_${STANDALONE_POSTFIX}")
        list(APPEND STANDALONE_TARGETS "${STANDALONE_TARGET}")
        list(APPEND STANDALONE_CONFIGURATIONS "${STANDALONE_POSTFIX}" "${PRODUCT_STANDALONE_NAME}" "${PRODUCT_PLUGIN_ID}")
        message(STATUS "  Standalone Product: ${PRODUCT_STANDALONE_NAME} -> ${PRODUCT_PLUGIN_ID}")
    endforeach()

    make_clapfirst_plugins(
        ${CLAPFIRST_COMMON_ARGS}
        STANDALONE_CONFIGURATIONS ${STANDALONE_CONFIGURATIONS}
    )
else()
    make_clapfirst_plugins(${CLAPFIRST_COMMON_ARGS})
endif()

set(AUV2_PRODUCT_TARGETS)
if(APPLE AND CLAP_WRAPPER_BUILDER_BUILD_AUV2)
    set(AUV2_PRODUCT_TARGETS "${PLUGIN_NAME_SAFE}_auv2")
endif()

if(APPLE)
    set(CLAP_BUNDLE_ID "${BUNDLE_ID}.clap")
    set(VST3_BUNDLE_ID "${BUNDLE_ID}.vst3")
    # Keep the AAX bundle ID format-specific. AAX hosts load `.aaxplugin`
    # bundles from the Avid folder, and sharing the CLAP/VST3 ID would make
    # macOS bundle metadata ambiguous during local validation.
    set(AAX_BUNDLE_ID "${BUNDLE_ID}.aaxplugin")
    set(STANDALONE_INFO_PLIST "${CMAKE_CURRENT_LIST_DIR}/StandaloneInfo.plist.in")

    if(TARGET ${PLUGIN_NAME_SAFE}_clap)
        set_target_properties(${PLUGIN_NAME_SAFE}_clap PROPERTIES
            MACOSX_BUNDLE_GUI_IDENTIFIER "${CLAP_BUNDLE_ID}"
            XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "${CLAP_BUNDLE_ID}"
        )
    endif()

    if(TARGET ${PLUGIN_NAME_SAFE}_vst3)
        set_target_properties(${PLUGIN_NAME_SAFE}_vst3 PROPERTIES
            MACOSX_BUNDLE_GUI_IDENTIFIER "${VST3_BUNDLE_ID}"
            XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "${VST3_BUNDLE_ID}"
        )
    endif()

    if(TARGET ${PLUGIN_NAME_SAFE}_aax)
        set_target_properties(${PLUGIN_NAME_SAFE}_aax PROPERTIES
            MACOSX_BUNDLE_GUI_IDENTIFIER "${AAX_BUNDLE_ID}"
            XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "${AAX_BUNDLE_ID}"
        )
    endif()

    foreach(AUV2_TARGET IN LISTS AUV2_PRODUCT_TARGETS)
        string(REPLACE "_" "-" AUV2_TARGET_BUNDLE_ID "${AUV2_TARGET}")
        set_target_properties(${AUV2_TARGET} PROPERTIES
            MACOSX_BUNDLE_GUI_IDENTIFIER "${BUNDLE_ID}.${AUV2_TARGET_BUNDLE_ID}.auv2"
            XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "${BUNDLE_ID}.${AUV2_TARGET_BUNDLE_ID}.auv2"
        )
    endforeach()

    foreach(STANDALONE_TARGET IN LISTS STANDALONE_TARGETS)
        string(REPLACE "_" "-" STANDALONE_TARGET_BUNDLE_ID "${STANDALONE_TARGET}")
        # The standalone app has its own Info.plist template, so set the bundle version
        # properties explicitly instead of relying on Xcode's default 1.0/1 values.
        set_target_properties(${STANDALONE_TARGET} PROPERTIES
            MACOSX_BUNDLE_GUI_IDENTIFIER "${BUNDLE_ID}.${STANDALONE_TARGET_BUNDLE_ID}.standalone"
            XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "${BUNDLE_ID}.${STANDALONE_TARGET_BUNDLE_ID}.standalone"
            MACOSX_BUNDLE_SHORT_VERSION_STRING "${BUNDLE_VERSION}"
            MACOSX_BUNDLE_BUNDLE_VERSION "${BUNDLE_VERSION}"
            MACOSX_BUNDLE_INFO_PLIST "${STANDALONE_INFO_PLIST}"
        )
    endforeach()
endif()

# The standalone exe links the impl lib directly rather than through the
# wrapper, so it needs runtimeobject.lib stated again on its own link line.
if(MSVC)
    foreach(STANDALONE_TARGET IN LISTS STANDALONE_TARGETS)
        target_link_libraries(${STANDALONE_TARGET} PRIVATE runtimeobject.lib)
    endforeach()
endif()

# Only the CLAP target compiles the generated entry shim, which includes
# <clap/clap.h>; the other formats get the header transitively.
target_include_directories(${PLUGIN_NAME_SAFE}_clap
  PRIVATE
  ${CLAP_SDK_ROOT}/include
)

if(CLAP_WRAPPER_BUILDER_STAGE_DIR)
    file(MAKE_DIRECTORY "${CLAP_WRAPPER_BUILDER_STAGE_DIR}")

    # Stage artifacts to xtask's fixed output paths. xtask deliberately avoids
    # finding outputs from generator-specific directories after the build.
    if(TARGET ${PLUGIN_NAME_SAFE}_vst3)
        if(APPLE)
            add_custom_command(TARGET ${PLUGIN_NAME_SAFE}_vst3 POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.vst3"
                COMMAND ${CMAKE_COMMAND} -E copy_directory "$<TARGET_BUNDLE_DIR:${PLUGIN_NAME_SAFE}_vst3>" "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.vst3"
                VERBATIM
            )
        elseif(WIN32)
            add_custom_command(TARGET ${PLUGIN_NAME_SAFE}_vst3 POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.vst3"
                COMMAND ${CMAKE_COMMAND} -E copy_directory "$<TARGET_FILE_DIR:${PLUGIN_NAME_SAFE}_vst3>/../.." "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.vst3"
                VERBATIM
            )
        else()
            add_custom_command(TARGET ${PLUGIN_NAME_SAFE}_vst3 POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.vst3"
                COMMAND ${CMAKE_COMMAND} -E copy_directory "$<TARGET_FILE_DIR:${PLUGIN_NAME_SAFE}_vst3>/../.." "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.vst3"
                VERBATIM
            )
        endif()
    endif()

    if(TARGET ${PLUGIN_NAME_SAFE}_aax)
        # clap-wrapper leaves AAX artifacts in generator-specific locations.
        # Stage them beside the other plugin formats so xtask can install,
        # validate, and print one deterministic path per target.
        if(APPLE)
            add_custom_command(TARGET ${PLUGIN_NAME_SAFE}_aax POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.aaxplugin"
                COMMAND ${CMAKE_COMMAND} -E copy_directory "$<TARGET_BUNDLE_DIR:${PLUGIN_NAME_SAFE}_aax>" "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.aaxplugin"
                VERBATIM
            )
        elseif(WIN32)
            add_custom_command(TARGET ${PLUGIN_NAME_SAFE}_aax POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.aaxplugin"
                COMMAND ${CMAKE_COMMAND} -E copy_directory "$<TARGET_FILE_DIR:${PLUGIN_NAME_SAFE}_aax>/../.." "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PLUGIN_NAME}.aaxplugin"
                VERBATIM
            )
        endif()
    endif()

    foreach(AUV2_TARGET IN LISTS AUV2_PRODUCT_TARGETS)
        get_target_property(AUV2_OUTPUT_NAME ${AUV2_TARGET} LIBRARY_OUTPUT_NAME)
        set(AUV2_GENERATED_PLIST "${CMAKE_CURRENT_BINARY_DIR}/${AUV2_TARGET}-build-helper-output/auv2_Info.plist")
        # clap-wrapper's AUv2 helper generates the AudioComponents metadata that
        # auval needs; copy it back before staging/signing the component bundle.
        add_custom_command(TARGET ${AUV2_TARGET} POST_BUILD
            COMMAND ${CMAKE_COMMAND} -E copy_if_different "${AUV2_GENERATED_PLIST}" "$<TARGET_BUNDLE_DIR:${AUV2_TARGET}>/Contents/Info.plist"
            COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${AUV2_OUTPUT_NAME}.component"
            COMMAND ${CMAKE_COMMAND} -E copy_directory "$<TARGET_BUNDLE_DIR:${AUV2_TARGET}>" "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${AUV2_OUTPUT_NAME}.component"
            VERBATIM
        )
    endforeach()

    foreach(STANDALONE_TARGET IN LISTS STANDALONE_TARGETS)
        get_target_property(PRODUCT_STANDALONE_OUTPUT_NAME ${STANDALONE_TARGET} OUTPUT_NAME)
        if(APPLE)
            add_custom_command(TARGET ${STANDALONE_TARGET} POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E rm -rf "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PRODUCT_STANDALONE_OUTPUT_NAME}.app"
                COMMAND ${CMAKE_COMMAND} -E copy_directory "$<TARGET_BUNDLE_DIR:${STANDALONE_TARGET}>" "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PRODUCT_STANDALONE_OUTPUT_NAME}.app"
                VERBATIM
            )
        elseif(WIN32)
            add_custom_command(TARGET ${STANDALONE_TARGET} POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E copy_if_different "$<TARGET_FILE:${STANDALONE_TARGET}>" "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PRODUCT_STANDALONE_OUTPUT_NAME}.exe"
                VERBATIM
            )
        else()
            add_custom_command(TARGET ${STANDALONE_TARGET} POST_BUILD
                COMMAND ${CMAKE_COMMAND} -E copy_if_different "$<TARGET_FILE:${STANDALONE_TARGET}>" "${CLAP_WRAPPER_BUILDER_STAGE_DIR}/${PRODUCT_STANDALONE_OUTPUT_NAME}"
                VERBATIM
            )
        endif()
    endforeach()
endif()
