#pragma once

#include "clap/private/macros.h"

#ifndef CLAP_ABI
#define CLAP_ABI
#endif

#ifdef __cplusplus
extern "C"
{
#endif

  static const CLAP_CONSTEXPR char WRAC_PLUGIN_FACTORY_RUN_LOOP[] =
      "com.novonotes.wrac.plugin-factory-run-loop/0";

  typedef struct wrac_plugin_factory_run_loop
  {
    // Called by format wrappers on the host/UI thread before creating a CLAP
    // plugin instance. This lets Rust bind novonotes_run_loop to the same
    // thread that JUCE would initialise as the message thread for the wrapper.
    bool(CLAP_ABI *bind_current_thread)(const wrac_plugin_factory_run_loop *factory);

    // Releases one bind reference. Wrappers call this from the matching format
    // lifecycle teardown point after the CLAP plugin instance has been destroyed.
    void(CLAP_ABI *unbind_current_thread)(const wrac_plugin_factory_run_loop *factory);
  } wrac_plugin_factory_run_loop_t;

#ifdef __cplusplus
}
#endif
