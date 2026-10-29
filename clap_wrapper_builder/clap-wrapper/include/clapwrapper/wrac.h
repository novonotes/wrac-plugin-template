#pragma once

#include <stdbool.h>

#include <clap/private/macros.h>

#ifdef __cplusplus
extern "C"
{
#endif

  static const CLAP_CONSTEXPR char WRAC_PLUGIN_MAIN_THREAD_HOOK[] =
      "com.novonotes.wrac.plugin-main-thread-hook/0";

  typedef struct wrac_plugin_main_thread_hook
  {
    void(CLAP_ABI *attach_main_thread)(const struct wrac_plugin_main_thread_hook *hook);
    void(CLAP_ABI *detach_main_thread)(const struct wrac_plugin_main_thread_hook *hook);
  } wrac_plugin_main_thread_hook_t;

  // Plugin extension that the AUv2 wrapper queries through clap_plugin.get_extension.
  static const CLAP_CONSTEXPR char WRAC_AU_HOST_IDENTIFIER[] =
      "com.novonotes.wrac.au-host-identifier/1";

  typedef struct wrac_au_host_identifier
  {
    // Called when the host sets kAudioUnitProperty_AUHostIdentifier, which hosts may do
    // after instance creation. `host_name` is the host-owned CFStringRef, or null to clear
    // the identifier; the plugin copies it before returning. Returns false if the name
    // cannot be read.
    bool(CLAP_ABI *set)(const struct clap_plugin *plugin, const void *host_name);
  } wrac_au_host_identifier_t;

#ifdef __cplusplus
}
#endif
