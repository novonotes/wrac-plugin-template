cmake_minimum_required(VERSION 3.21)
project(clap_wrapper_builder)
# enable_language(OBJCXX)

# Require C++23 or later (required by clap-wrapper and AudioUnitSDK)
set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OSX_DEPLOYMENT_TARGET 10.15)

set(CLAP_SDK_ROOT "${CMAKE_CURRENT_LIST_DIR}/clap")

set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")

# Receive the static library path as a cache variable
set(CLAP_WRAPPER_BUILDER_TARGET_LIB "" CACHE FILEPATH "Path to the static library to wrap")
option(CLAP_WRAPPER_BUILDER_BUILD_STANDALONE "Build standalone app target in clap_wrapper_builder" OFF)
option(CLAP_WRAPPER_BUILDER_BUILD_VST3 "Build VST3 target in clap_wrapper_builder" ON)
if(APPLE)
    option(CLAP_WRAPPER_BUILDER_BUILD_AUV2 "Build AUv2 target in clap_wrapper_builder" ON)
else()
    option(CLAP_WRAPPER_BUILDER_BUILD_AUV2 "Build AUv2 target in clap_wrapper_builder" OFF)
endif()
set(CLAP_WRAPPER_BUILDER_STANDALONE_OUTPUT_NAME "" CACHE STRING "Output name for standalone app")
set(CLAP_WRAPPER_BUILDER_STANDALONE_PLUGIN_ID "" CACHE STRING "Plugin ID used by standalone app")

# Validate required parameters
if(NOT CLAP_WRAPPER_BUILDER_TARGET_LIB)
    message(FATAL_ERROR "CLAP_WRAPPER_BUILDER_TARGET_LIB must be specified. Use: -DCLAP_WRAPPER_BUILDER_TARGET_LIB=/path/to/your/plugin.a")
endif()

if(NOT EXISTS ${CLAP_WRAPPER_BUILDER_TARGET_LIB})
    message(FATAL_ERROR "Static library not found: ${CLAP_WRAPPER_BUILDER_TARGET_LIB}")
endif()

# Derive plugin name from the static library filename
get_filename_component(PLUGIN_NAME ${CLAP_WRAPPER_BUILDER_TARGET_LIB} NAME_WE)
string(REGEX REPLACE "^lib" "" PLUGIN_NAME ${PLUGIN_NAME})
string(REGEX REPLACE "[^A-Za-z0-9_]" "_" PLUGIN_NAME_SAFE ${PLUGIN_NAME})
string(REPLACE "_" "-" PLUGIN_NAME_BUNDLE ${PLUGIN_NAME_SAFE})

message(STATUS "CLAP Wrapper Builder Configuration:")
message(STATUS "  Target Library: ${CLAP_WRAPPER_BUILDER_TARGET_LIB}")
message(STATUS "  Plugin Name: ${PLUGIN_NAME}")
message(STATUS "  Safe Plugin Name: ${PLUGIN_NAME_SAFE}")

# Add the clap-wrapper subproject
add_subdirectory(clap-wrapper)

if(APPLE AND TARGET clap-wrapper-compile-options)
  target_compile_options(clap-wrapper-compile-options INTERFACE -Wno-shorten-64-to-32)
endif()

# Create the static library target
# Set up the external static library as an imported library
add_library(${PLUGIN_NAME_SAFE}_impl STATIC IMPORTED)

if(MSVC)
  set_target_properties(${PLUGIN_NAME_SAFE}_impl PROPERTIES
      IMPORTED_LOCATION ${CLAP_WRAPPER_BUILDER_TARGET_LIB}
      INTERFACE_LINK_LIBRARIES "ntdll.lib;ws2_32.lib;userenv.lib"
  )
endif()

if(APPLE)
  set_target_properties(${PLUGIN_NAME_SAFE}_impl PROPERTIES
      IMPORTED_LOCATION ${CLAP_WRAPPER_BUILDER_TARGET_LIB}
      INTERFACE_LINK_LIBRARIES "-framework AppKit;-framework Foundation"
  )
endif()
# Generate the source file for the CLAP entry point.
# The CLAP entry point itself is provided by clack and lives inside
# CLAP_WRAPPER_BUILDER_TARGET_LIB. Calling get_clap_entry() from here
# prevents the clap_entry symbol inside CLAP_WRAPPER_BUILDER_TARGET_LIB
# from being stripped, ensuring it is correctly retained in the DLL.
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

# Configure plugin formats to build
set(PLUGIN_FORMATS "CLAP")
if(CLAP_WRAPPER_BUILDER_BUILD_VST3)
    list(APPEND PLUGIN_FORMATS "VST3")
endif()
if(APPLE AND CLAP_WRAPPER_BUILDER_BUILD_AUV2)
    list(APPEND PLUGIN_FORMATS "AUV2")
endif()

# Plugin bundle metadata
set(BUNDLE_ID "org.wrapper.${PLUGIN_NAME_BUNDLE}")
set(BUNDLE_VERSION "1.0.0")

# Create multi-format plugins using make_clapfirst_plugins
set(CLAPFIRST_COMMON_ARGS
    TARGET_NAME ${PLUGIN_NAME_SAFE}
    IMPL_TARGET ${PLUGIN_NAME_SAFE}_impl
    OUTPUT_NAME "${PLUGIN_NAME}"
    ENTRY_SOURCE ${ENTRY_SOURCE_FILE}
    PLUGIN_FORMATS ${PLUGIN_FORMATS}
    BUNDLE_IDENTIFIER "${BUNDLE_ID}"
    BUNDLE_VERSION "${BUNDLE_VERSION}"
    COPY_AFTER_BUILD FALSE
    WINDOWS_FOLDER_VST3 FALSE
)

if(CLAP_WRAPPER_BUILDER_BUILD_STANDALONE)
    if(NOT CLAP_WRAPPER_BUILDER_STANDALONE_PLUGIN_ID)
        message(FATAL_ERROR "CLAP_WRAPPER_BUILDER_STANDALONE_PLUGIN_ID must be specified when CLAP_WRAPPER_BUILDER_BUILD_STANDALONE=ON")
    endif()

    if(NOT CLAP_WRAPPER_BUILDER_STANDALONE_OUTPUT_NAME)
        set(CLAP_WRAPPER_BUILDER_STANDALONE_OUTPUT_NAME "${PLUGIN_NAME} Standalone")
    endif()

    message(STATUS "  Standalone Enabled: ON")
    message(STATUS "  Standalone Output Name: ${CLAP_WRAPPER_BUILDER_STANDALONE_OUTPUT_NAME}")
    message(STATUS "  Standalone Plugin ID: ${CLAP_WRAPPER_BUILDER_STANDALONE_PLUGIN_ID}")

    make_clapfirst_plugins(
        ${CLAPFIRST_COMMON_ARGS}
        STANDALONE_CONFIGURATIONS standalone "${CLAP_WRAPPER_BUILDER_STANDALONE_OUTPUT_NAME}" "${CLAP_WRAPPER_BUILDER_STANDALONE_PLUGIN_ID}"
    )
else()
    make_clapfirst_plugins(${CLAPFIRST_COMMON_ARGS})
endif()

if(APPLE)
    set(CLAP_BUNDLE_ID "${BUNDLE_ID}.clap")
    set(VST3_BUNDLE_ID "${BUNDLE_ID}.vst3")
    set(AUV2_BUNDLE_ID "${BUNDLE_ID}.auv2")
    set(STANDALONE_BUNDLE_ID "${BUNDLE_ID}.standalone")
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

    if(TARGET ${PLUGIN_NAME_SAFE}_auv2)
        set_target_properties(${PLUGIN_NAME_SAFE}_auv2 PROPERTIES
            MACOSX_BUNDLE_GUI_IDENTIFIER "${AUV2_BUNDLE_ID}"
            XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "${AUV2_BUNDLE_ID}"
        )
    endif()

    if(TARGET ${PLUGIN_NAME_SAFE}_standalone)
        set_target_properties(${PLUGIN_NAME_SAFE}_standalone PROPERTIES
            MACOSX_BUNDLE_GUI_IDENTIFIER "${STANDALONE_BUNDLE_ID}"
            XCODE_ATTRIBUTE_PRODUCT_BUNDLE_IDENTIFIER "${STANDALONE_BUNDLE_ID}"
            MACOSX_BUNDLE_INFO_PLIST "${STANDALONE_INFO_PLIST}"
        )
    endif()
endif()

if(MSVC AND TARGET ${PLUGIN_NAME_SAFE}_standalone)
    target_link_libraries(${PLUGIN_NAME_SAFE}_standalone PRIVATE runtimeobject.lib)
endif()

target_include_directories(${PLUGIN_NAME_SAFE}_clap
  PRIVATE
  ${CLAP_SDK_ROOT}/include
)
