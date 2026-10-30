cmake_minimum_required(VERSION 3.25)
project(vst3_latency_cache_test LANGUAGES CXX)

if(NOT WIN32)
  message(FATAL_ERROR "This regression host currently supports Windows only")
endif()

set(SMTG_ENABLE_VST3_PLUGIN_EXAMPLES OFF CACHE BOOL "" FORCE)
set(SMTG_ENABLE_VST3_HOSTING_EXAMPLES OFF CACHE BOOL "" FORCE)
set(SMTG_ENABLE_VSTGUI_SUPPORT OFF CACHE BOOL "" FORCE)
set(SDK_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../vst3sdk")
add_subdirectory("${SDK_ROOT}" vst3sdk EXCLUDE_FROM_ALL)

add_executable(vst3_latency_cache
  main.cpp
  "${SDK_ROOT}/public.sdk/source/common/memorystream.cpp"
  "${SDK_ROOT}/public.sdk/source/vst/hosting/module_win32.cpp"
  "${SDK_ROOT}/public.sdk/samples/vst-hosting/validator/source/usediids.cpp"
)
target_compile_features(vst3_latency_cache PRIVATE cxx_std_17)
target_include_directories(vst3_latency_cache PRIVATE "${SDK_ROOT}")
target_link_libraries(vst3_latency_cache PRIVATE sdk_hosting sdk_common base pluginterfaces)
