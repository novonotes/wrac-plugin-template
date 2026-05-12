# automatically build the wrapper only if this is the top level project
if (PROJECT_IS_TOP_LEVEL)
	# provide the CLAP_WRAPPER_OUTPUT_NAME to specify the matching plugin name.
	if((NOT CLAP_WRAPPER_OUTPUT_NAME ) OR (CLAP_WRAPPER_OUTPUT_NAME STREQUAL ""))
		set(CLAP_WRAPPER_OUTPUT_NAME "clapasvst3")
		message(WARNING "clap-wrapper: CLAP_WRAPPER_OUTPUT_NAME not set - continuing with a default name `clapasvst3`")
	endif()

	string(MAKE_C_IDENTIFIER ${CLAP_WRAPPER_OUTPUT_NAME} pluginname)

	if (APPLE)
		if (NOT DEFINED CLAP_WRAPPER_MACOS_EMBEDDED_CLAP_LOCATION)
			set(CLAP_WRAPPER_MACOS_EMBEDDED_CLAP_LOCATION "")
		endif()

		# AUv2 identity must be product-specific. Reusing clap-wrapper's generic
		# default makes hosts and auval resolve a different AudioComponent than
		# the one the product build script installed.
		if (NOT DEFINED CLAP_WRAPPER_AUV2_INSTRUMENT_TYPE)
			set(CLAP_WRAPPER_AUV2_INSTRUMENT_TYPE "aumu")
		endif()
		if (NOT DEFINED CLAP_WRAPPER_AUV2_MANUFACTURER_NAME)
			set(CLAP_WRAPPER_AUV2_MANUFACTURER_NAME "cleveraudio.org")
		endif()
		if (NOT DEFINED CLAP_WRAPPER_AUV2_MANUFACTURER_CODE)
			set(CLAP_WRAPPER_AUV2_MANUFACTURER_CODE "clAd")
		endif()
		if (NOT DEFINED CLAP_WRAPPER_AUV2_SUBTYPE_CODE)
			set(CLAP_WRAPPER_AUV2_SUBTYPE_CODE "gWrp")
		endif()
	endif()


	if (CLAP_WRAPPER_CAN_BUILD_AAX AND CLAP_WRAPPER_BUILD_AAX)
		    # Link the actual plugin library
			add_library(${pluginname}_as_aax MODULE)
			target_add_aax_wrapper(
					TARGET ${pluginname}_as_aax
					OUTPUT_NAME "${CLAP_WRAPPER_OUTPUT_NAME}"
					BUNDLE_IDENTIFIER "${CLAP_WRAPPER_BUNDLE_IDENTIFIER}"
					BUNDLE_VERSION "${CLAP_WRAPPER_BUNDLE_VERSION}"
			)
	endif()

	# Link the actual plugin library
	add_library(${pluginname}_as_vst3 MODULE)
	target_add_vst3_wrapper(
			TARGET ${pluginname}_as_vst3
			OUTPUT_NAME "${CLAP_WRAPPER_OUTPUT_NAME}"
			SUPPORTS_ALL_NOTE_EXPRESSIONS $<BOOL:${CLAP_SUPPORTS_ALL_NOTE_EXPRESSIONS}>
			SINGLE_PLUGIN_TUID "${CLAP_VST3_TUID_STRING}"
			BUNDLE_IDENTIFIER "${CLAP_WRAPPER_BUNDLE_IDENTIFIER}"
			BUNDLE_VERSION "${CLAP_WRAPPER_BUNDLE_VERSION}"
			MACOS_EMBEDDED_CLAP_LOCATION "${CLAP_WRAPPER_MACOS_EMBEDDED_CLAP_LOCATION}"
			)

	if (APPLE)
		if (${CLAP_WRAPPER_BUILD_AUV2})
			add_library(${pluginname}_as_auv2 MODULE)
			target_add_auv2_wrapper(
					TARGET ${pluginname}_as_auv2
					OUTPUT_NAME "${CLAP_WRAPPER_OUTPUT_NAME}"
					BUNDLE_IDENTIFIER "${CLAP_WRAPPER_BUNDLE_IDENTIFIER}"
					BUNDLE_VERSION "${CLAP_WRAPPER_BUNDLE_VERSION}"

					# The top-level helper builds one product at a time, so AUv2
					# metadata is supplied by the product build script instead of
					# using clap-wrapper's generic sample identity.
					INSTRUMENT_TYPE "${CLAP_WRAPPER_AUV2_INSTRUMENT_TYPE}"
					MANUFACTURER_NAME "${CLAP_WRAPPER_AUV2_MANUFACTURER_NAME}"
					MANUFACTURER_CODE "${CLAP_WRAPPER_AUV2_MANUFACTURER_CODE}"
					SUBTYPE_CODE "${CLAP_WRAPPER_AUV2_SUBTYPE_CODE}"
					MACOS_EMBEDDED_CLAP_LOCATION "${CLAP_WRAPPER_MACOS_EMBEDDED_CLAP_LOCATION}"
			)
		endif()
	endif()

	if (${CLAP_WRAPPER_BUILD_STANDALONE})
		add_executable(${pluginname}_as_standalone)
		target_add_standalone_wrapper(TARGET ${pluginname}_as_standalone
		    OUTPUT_NAME ${CLAP_WRAPPER_OUTPUT_NAME}
		    HOSTED_CLAP_NAME ${CLAP_WRAPPER_OUTPUT_NAME}
		    PLUGIN_INDEX 0)
	endif()
endif()
