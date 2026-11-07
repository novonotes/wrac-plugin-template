// An AAX sidechain must reach its CLAP port without exposing undeclared input
// fields.
#include "detail/aax/wrapper.h"

#include <array>
#include <cstdio>
#include <vector>

struct TransportNode : AAX_IMIDINode
{
  AAX_CMidiStream *GetNodeBuffer() override
  {
    return nullptr;
  }
  AAX_Result PostMIDIPacket(AAX_CMidiPacket *) override
  {
    return AAX_SUCCESS;
  }
  AAX_ITransport *GetTransport() override
  {
    return nullptr;
  }
};

enum class Sidechain
{
  Undeclared,
  Disconnected,
  NullIndex,
  InvalidIndex,
  NullBuffer,
  Connected,
  ConnectedWithPadding
};

struct Layout
{
  uint32_t inputChannels;
  uint32_t outputChannels;
  uint32_t auxChannels;
  bool expectSidechain;
  bool processed = false;
};

static uint32_t CLAP_ABI audioPortCount(const clap_plugin_t *, bool isInput)
{
  return isInput ? 3 : 1;
}

static bool CLAP_ABI audioPortInfo(const clap_plugin_t *plugin, uint32_t index, bool isInput,
                                   clap_audio_port_info_t *info)
{
  const auto &layout = *static_cast<const Layout *>(plugin->plugin_data);
  if (index >= (isInput ? 3u : 1u)) return false;
  *info = {};
  info->channel_count =
      isInput ? (index == 0 ? layout.inputChannels : layout.auxChannels) : layout.outputChannels;
  return true;
}

static clap_process_status CLAP_ABI processAudio(const clap_plugin_t *plugin,
                                                 const clap_process_t *process)
{
  auto &layout = *static_cast<Layout *>(plugin->plugin_data);
  const auto &main = process->audio_inputs[0];
  const auto &sidechain = process->audio_inputs[1];
  const auto &unmapped = process->audio_inputs[2];
  const auto &out = process->audio_outputs[0];
  for (uint32_t c = 0; c < sidechain.channel_count; ++c)
  {
    if (!sidechain.data32[c] || !unmapped.data32[c]) return CLAP_PROCESS_ERROR;
    for (uint32_t i = 0; i < process->frames_count; ++i)
      if (sidechain.data32[c][i] != (layout.expectSidechain ? 0.75f : 0.0f) ||
          unmapped.data32[c][i] != 0.0f)
        return CLAP_PROCESS_ERROR;
  }
  for (uint32_t c = 0; c < out.channel_count; ++c)
    for (uint32_t i = 0; i < process->frames_count; ++i)
      out.data32[c][i] = main.data32[c % main.channel_count][i];
  layout.processed = true;
  return CLAP_PROCESS_CONTINUE;
}

static bool run(uint32_t inChannels, uint32_t outChannels, uint32_t auxChannels, Sidechain state)
{
  Layout layout{inChannels, outChannels, auxChannels,
                state == Sidechain::Connected || state == Sidechain::ConnectedWithPadding};
  clap_plugin_descriptor_t descriptor{};
  descriptor.name = "Audio input test";
  clap_plugin_t plugin{};
  plugin.desc = &descriptor;
  plugin.plugin_data = &layout;
  plugin.process = processAudio;
  clap_plugin_audio_ports_t audioPorts{};
  audioPorts.count = audioPortCount;
  audioPorts.get = audioPortInfo;
  ParamChangeQueue queue;
  if (!queue.init(2)) return false;
  std::vector<clap_id> gestures;
  AAXProcessAdapter adapter;
  const uint32_t sidechainPort = state == Sidechain::Undeclared ? 0 : 1;
  adapter.setupProcessing(&plugin, 48000, nullptr, &audioPorts, nullptr, gestures, queue, nullptr,
                          CLAP_INVALID_ID, true, false, sidechainPort, 0, 0);

  std::array<float, 32> left, right, sidechain, outLeft{}, outRight{};
  left.fill(0.25f);
  right.fill(-0.5f);
  sidechain.fill(0.75f);
  std::array<float *, 4> inputs{left.data(), inChannels == 2 ? right.data() : nullptr, nullptr};
  std::array<float *, 2> outputs{outLeft.data(), outRight.data()};
  int32_t frames = static_cast<int32_t>(left.size());
  int32_t sidechainIndex = state == Sidechain::Disconnected ? 0 : static_cast<int32_t>(inChannels);
  if (state == Sidechain::InvalidIndex) sidechainIndex = -1;
  if (state == Sidechain::ConnectedWithPadding) ++sidechainIndex;
  if (layout.expectSidechain) inputs[sidechainIndex] = sidechain.data();
  TransportNode transport;
  SAAX_Wrapper_AlgorithmicContext context;
  context.mAudioInputs = inputs.data();
  context.mAudioOutputs = outputs.data();
  context.mNumSamples = &frames;
  context.mTransportNode = &transport;
  // An unregistered AAX field is private data, not a pointer to an index.
  context.mSideChainInput = state == Sidechain::Undeclared
                                ? reinterpret_cast<int32_t *>(uintptr_t{0xcdcdcdcd})
                            : state == Sidechain::NullIndex ? nullptr
                                                            : &sidechainIndex;
  adapter.process(&context);
  if (!layout.processed || outLeft != left ||
      (outChannels == 2 && outRight != (inChannels == 2 ? right : left)))
  {
    std::fprintf(stderr, "Audio failed: %u/%u, aux %u, sidechain state %u\n", inChannels, outChannels,
                 auxChannels, static_cast<unsigned>(state));
    return false;
  }
  return true;
}

int main()
{
  constexpr std::array states{Sidechain::Undeclared,          Sidechain::Disconnected,
                              Sidechain::NullIndex,           Sidechain::InvalidIndex,
                              Sidechain::NullBuffer,          Sidechain::Connected,
                              Sidechain::ConnectedWithPadding};
  for (auto state : states)
    if (!run(1, 1, 1, state) || !run(1, 2, 2, state) || !run(2, 2, 2, state)) return 1;
  return 0;
}
