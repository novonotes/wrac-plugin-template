#pragma once

#include <clap/private/macros.h>

#ifdef __cplusplus
extern "C" {
#endif

static const CLAP_CONSTEXPR char WRAC_PLUGIN_MAIN_THREAD_HOOK[] =
    "com.novonotes.wrac.plugin-main-thread-hook/0";

typedef struct wrac_plugin_main_thread_hook
{
  void(CLAP_ABI *attach_main_thread)(const struct wrac_plugin_main_thread_hook *hook);
  void(CLAP_ABI *detach_main_thread)(const struct wrac_plugin_main_thread_hook *hook);
} wrac_plugin_main_thread_hook_t;

#ifdef __cplusplus
}
#endif
