cmake_minimum_required(VERSION 3.25)
project(wrapper_regression_tests LANGUAGES CXX)

set(WRAC_TEST_PLUGIN "" CACHE PATH "Built WRAC Gain bundle used by the tests")
if(NOT IS_DIRECTORY "${WRAC_TEST_PLUGIN}")
  message(FATAL_ERROR "WRAC_TEST_PLUGIN must point to a built WRAC Gain bundle: ${WRAC_TEST_PLUGIN}")
endif()

enable_testing()

set(WRAC_TEST_AAX_SDK "" CACHE PATH "Optional AAX SDK root for the AAX regression tests")
if(WRAC_TEST_AAX_SDK)
  set(AAX_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
  set(AAX_BUILD_PTSL_EXAMPLES OFF CACHE BOOL "" FORCE)
  add_subdirectory("${WRAC_TEST_AAX_SDK}" aaxsdk EXCLUDE_FROM_ALL)

  function(add_aax_process_test test)
    add_executable(${test} ${test}.cpp
      "${CMAKE_CURRENT_LIST_DIR}/../clap-wrapper/src/detail/aax/process.cpp")
    target_compile_features(${test} PRIVATE cxx_std_23)
    target_compile_definitions(${test} PRIVATE CLAP_WRAPPER_LOGLEVEL=0)
    if(APPLE)
      target_compile_definitions(${test} PRIVATE MAC=1 MACOS_USE_STD_FILESYSTEM=1)
    elseif(WIN32)
      target_compile_definitions(${test} PRIVATE WIN=1)
    endif()
    target_include_directories(${test} PRIVATE
      "${CMAKE_CURRENT_LIST_DIR}/../clap-wrapper/src"
      "${CMAKE_CURRENT_LIST_DIR}/../clap-wrapper/include"
      "${CMAKE_CURRENT_LIST_DIR}/../clap-wrapper/libs/fmt"
      "${CMAKE_CURRENT_LIST_DIR}/../clap/include")
    target_link_libraries(${test} PRIVATE AAX_SDK::AAXLibrary)
    add_test(NAME ${test} COMMAND ${test})
  endfunction()

  add_executable(aax_parameter_rescan aax_parameter_rescan.cpp)
  target_compile_features(aax_parameter_rescan PRIVATE cxx_std_17)
  target_include_directories(aax_parameter_rescan PRIVATE
    "${CMAKE_CURRENT_LIST_DIR}/../clap-wrapper/src"
    "${CMAKE_CURRENT_LIST_DIR}/../clap/include")
  target_link_libraries(aax_parameter_rescan PRIVATE AAX_SDK::AAXLibrary)
  add_test(NAME aax_parameter_rescan COMMAND aax_parameter_rescan)
  add_aax_process_test(aax_midi_input)
  add_aax_process_test(aax_audio_input)
endif()

if(APPLE)
  find_library(AUDIO_TOOLBOX AudioToolbox REQUIRED)
  find_library(CORE_FOUNDATION CoreFoundation REQUIRED)
  foreach(test IN ITEMS au_state_recall au_latency_restart)
    add_executable(${test} "${test}.cpp")
    target_compile_features(${test} PRIVATE cxx_std_17)
    target_compile_options(${test} PRIVATE -Wall -Wextra -Werror)
    target_link_libraries(${test} PRIVATE "${AUDIO_TOOLBOX}" "${CORE_FOUNDATION}")
    add_test(NAME ${test} COMMAND ${test} "${WRAC_TEST_PLUGIN}")
    # A state-recall regression can deadlock, so CTest must bound execution.
    set_tests_properties(${test} PROPERTIES TIMEOUT 30)
  endforeach()
elseif(WIN32)
  set(SMTG_ENABLE_VST3_PLUGIN_EXAMPLES OFF CACHE BOOL "" FORCE)
  set(SMTG_ENABLE_VST3_HOSTING_EXAMPLES OFF CACHE BOOL "" FORCE)
  set(SMTG_ENABLE_VSTGUI_SUPPORT OFF CACHE BOOL "" FORCE)
  set(SDK_ROOT "${CMAKE_CURRENT_LIST_DIR}/../vst3sdk")
  add_subdirectory("${SDK_ROOT}" vst3sdk EXCLUDE_FROM_ALL)

  add_executable(vst3_latency_cache
    vst3_latency_cache/main.cpp
    "${SDK_ROOT}/public.sdk/source/common/memorystream.cpp"
    "${SDK_ROOT}/public.sdk/source/vst/hosting/module_win32.cpp"
    "${SDK_ROOT}/public.sdk/samples/vst-hosting/validator/source/usediids.cpp"
  )
  target_compile_features(vst3_latency_cache PRIVATE cxx_std_17)
  target_include_directories(vst3_latency_cache PRIVATE "${SDK_ROOT}")
  target_link_libraries(vst3_latency_cache PRIVATE sdk_hosting sdk_common base pluginterfaces)
  add_test(NAME vst3_latency_cache COMMAND vst3_latency_cache "${WRAC_TEST_PLUGIN}")
  set_tests_properties(vst3_latency_cache PROPERTIES TIMEOUT 30)
else()
  message(FATAL_ERROR "Wrapper regression tests are supported on Windows and macOS")
endif()
