#pragma once

#include <pluginterfaces/vst/ivstaudioprocessor.h>
#include <cstring>

namespace Clap
{
// Hosts may supply stale output or alias inputs and outputs. On skipped/error
// paths, explicitly clear every supplied output in the negotiated precision.
inline void silenceOutputBuffers(Steinberg::Vst::ProcessData &data)
{
  if (data.numSamples <= 0 || !data.outputs) return;
  if (data.symbolicSampleSize != Steinberg::Vst::kSample32 &&
      data.symbolicSampleSize != Steinberg::Vst::kSample64) return;

  for (Steinberg::int32 bus = 0; bus < data.numOutputs; ++bus)
  {
    auto &out = data.outputs[bus];
    for (Steinberg::int32 ch = 0; ch < out.numChannels; ++ch)
    {
      if (data.symbolicSampleSize == Steinberg::Vst::kSample64)
      {
        if (out.channelBuffers64 && out.channelBuffers64[ch])
          std::memset(out.channelBuffers64[ch], 0, sizeof(double) * (size_t)data.numSamples);
      }
      else if (out.channelBuffers32 && out.channelBuffers32[ch])
        std::memset(out.channelBuffers32[ch], 0, sizeof(float) * (size_t)data.numSamples);
    }
    out.silenceFlags = out.numChannels >= 64 ? ~Steinberg::uint64(0)
                      : out.numChannels > 0 ? ((Steinberg::uint64(1) << out.numChannels) - 1)
                                            : 0;
  }
}
}
