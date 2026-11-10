# Replaces the AudioComponents tags in an AUv2 bundle Info.plist with the tags
# each product declares.
#
# Usage: cmake -DINFO_PLIST=<path> -DTAGS_CONFIG=<path> -P apply_auv2_tags.cmake
#
# TAGS_CONFIG is written by CMakeLists.txt at configure time and defines
# AUV2_PRODUCT_COUNT plus AUV2_PRODUCT_<N>_AUV2_TYPE/_AUV2_SUBTYPE/_AUV2_TAGS.
#
# Entries are matched to products by type/subtype because that pair is the
# identity a product declares, whereas the entry order is an implementation
# detail of clap-wrapper's build helper. An entry that matches no product (for
# example a legacy identity) fails the build instead of shipping the helper's
# non-standard tags.

include("${TAGS_CONFIG}")
math(EXPR product_last_index "${AUV2_PRODUCT_COUNT} - 1")

function(read_component_field entry field out_var)
    execute_process(
        COMMAND /usr/bin/plutil -extract "AudioComponents.${entry}.${field}" raw -o - "${INFO_PLIST}"
        RESULT_VARIABLE result
        OUTPUT_VARIABLE value
        OUTPUT_STRIP_TRAILING_WHITESPACE
        ERROR_QUIET
    )
    if(result EQUAL 0)
        set(${out_var} "${value}" PARENT_SCOPE)
    else()
        set(${out_var} "" PARENT_SCOPE)
    endif()
endfunction()

set(entry 0)
while(TRUE)
    read_component_field(${entry} "type" component_type)
    if(component_type STREQUAL "")
        # plutil fails past the last AudioComponents entry.
        break()
    endif()
    read_component_field(${entry} "subtype" component_subtype)

    set(component_tags "")
    foreach(product_index RANGE 0 ${product_last_index})
        if(component_type STREQUAL "${AUV2_PRODUCT_${product_index}_AUV2_TYPE}"
           AND component_subtype STREQUAL "${AUV2_PRODUCT_${product_index}_AUV2_SUBTYPE}")
            set(component_tags "${AUV2_PRODUCT_${product_index}_AUV2_TAGS}")
            break()
        endif()
    endforeach()
    if(component_tags STREQUAL "")
        message(FATAL_ERROR "No product declares AUv2 tags for AudioComponents entry ${component_type}/${component_subtype} in ${INFO_PLIST}")
    endif()

    set(tags_json "")
    foreach(tag IN LISTS component_tags)
        if(NOT tags_json STREQUAL "")
            string(APPEND tags_json ",")
        endif()
        # Custom tags are free text, so escape the characters JSON reserves.
        string(REPLACE "\\" "\\\\" tag "${tag}")
        string(REPLACE "\"" "\\\"" tag "${tag}")
        string(APPEND tags_json "\"${tag}\"")
    endforeach()
    execute_process(
        COMMAND /usr/bin/plutil -replace "AudioComponents.${entry}.tags" -json "[${tags_json}]" "${INFO_PLIST}"
        RESULT_VARIABLE result
    )
    if(NOT result EQUAL 0)
        message(FATAL_ERROR "Failed to write AUv2 tags for AudioComponents entry ${entry} in ${INFO_PLIST}")
    endif()
    math(EXPR entry "${entry} + 1")
endwhile()

if(entry EQUAL 0)
    message(FATAL_ERROR "No AudioComponents entries found in ${INFO_PLIST}")
endif()
