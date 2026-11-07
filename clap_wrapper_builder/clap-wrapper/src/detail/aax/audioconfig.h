#pragma once

#include <vector>
#include <string>

#include "clap_proxy.h"
#include "factory.h"

namespace CLAPAAX
{

typedef struct stemformat_combi
{
  std::string name;
  uint32_t format_in;
  uint32_t format_out;
  uint32_t plugin_id = 0;  // AAX_eProperty_PlugInID_Native override; 0 = auto-generate
} stemformat_combi_t;

typedef struct sAAXStemIndexToClapMap
{
  const char *identifier;
  uint32_t aaxStemformat;
  const uint8_t *clapmap;
  size_t mapsize;
} sAAXStemIndexToClapMap_t;

struct plugin_bus_info_t
{
  std::vector<stemformat_combi_t> stemformats;
  bool has_midi_in = false;
  bool has_midi_out = false;
  // Zero means no sidechain; CLAP main input is always port zero.
  uint32_t sidechain_input_port = 0;
  std::string midi_in_name;
  std::string midi_out_name;
};

plugin_bus_info_t getAvailableBusConfigs(Clap::Library *factory, uint32_t index);

// Query the same factory-time declaration when describing and running an instance.
uint32_t sidechainInputPort(const Clap::Library *factory, const char *pluginid);

// The MIDI nodes the AAX description registers for a plugin. A context field that is not
// registered as a MIDI node holds placeholder private data, so the running instance must use
// exactly the nodes declared here: the description and the instance both call this function.
struct midi_node_declaration_t
{
  bool has_midi_in = false;
  bool has_midi_out = false;
  std::string midi_in_name;
  std::string midi_out_name;
};

// With AAX info from the plugin factory, only its MIDI names declare nodes. Without it, a note
// port that supports the MIDI dialect does. `plugin` and `noteports` may be null.
midi_node_declaration_t declaredMidiNodes(const clap_plugin_info_as_aax_t *aaxinfo,
                                          const clap_plugin_t *plugin,
                                          const clap_plugin_note_ports_t *noteports);

// AAX info the plugin factory provides for the plugin with this CLAP id, or nullptr.
const clap_plugin_info_as_aax_t *findAaxInfo(const Clap::Library *factory, const char *pluginid);

}  // namespace CLAPAAX
