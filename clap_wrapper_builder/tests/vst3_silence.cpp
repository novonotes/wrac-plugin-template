// Skipped processing must clear stale/in-place buffers in both host precisions.
#include "../clap-wrapper/src/detail/vst3/silence.h"
#include <algorithm>
#include <cstdlib>
#include <iostream>
#include <type_traits>

template <typename T> void checkPrecision()
{
  using namespace Steinberg;
  using namespace Steinberg::Vst;
  T samples[2][8];
  for (auto &channel : samples) std::fill_n(channel, 8, T(0.5));
  T *first[] = {samples[0], nullptr};
  T *second[] = {samples[1]};
  AudioBusBuffers outputs[2]{};
  outputs[0].numChannels = 2;
  outputs[1].numChannels = 1;
  ProcessData data;
  data.numSamples = 7; // Do not write outside the current processing range.
  data.numOutputs = 2;
  data.outputs = outputs;
  data.inputs = outputs; // The silence path also permits in-place audio.
  data.numInputs = 2;
  if constexpr (std::is_same_v<T, double>)
  {
    data.symbolicSampleSize = kSample64;
    outputs[0].channelBuffers64 = first;
    outputs[1].channelBuffers64 = second;
  }
  else
  {
    data.symbolicSampleSize = kSample32;
    outputs[0].channelBuffers32 = first;
    outputs[1].channelBuffers32 = second;
  }
  Clap::silenceOutputBuffers(data);
  for (const auto &channel : samples)
  {
    if (!std::all_of(channel, channel + 7, [](T value) { return value == T(0); }) ||
        channel[7] != T(0.5)) std::abort();
  }
  if (outputs[0].silenceFlags != 3 || outputs[1].silenceFlags != 1) std::abort();
  data.numSamples = 0;
  data.outputs = nullptr;
  Clap::silenceOutputBuffers(data);
}

int main()
{
  checkPrecision<float>();
  checkPrecision<double>();
  std::cout << "VST3 f32/f64 silence regression passed\n";
}
