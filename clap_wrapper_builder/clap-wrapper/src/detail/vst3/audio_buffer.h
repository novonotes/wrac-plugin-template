#pragma once

#include <clap/audio-buffer.h>
#include <pluginterfaces/vst/ivstaudioprocessor.h>
#include <algorithm>
#include <atomic>
#include <cstdint>
#include <vector>

namespace Clap
{
enum class AudioBusActivity
{
  Inactive,
  Active
};
static_assert(std::atomic<AudioBusActivity>::is_always_lock_free);

// VST3 permits null samples (and omitted trailing buses) for inactive buses.
// CLAP still requires valid buffers for every port. Own the replacement storage
// here so mapping never allocates and inactive outputs cannot overwrite inputs
// or alias other output channels. This is scratch storage, not a delay line.
class AudioBusBuffer
{
 public:
  // [non-realtime] Called before processing, including for currently active buses
  // so a later activation change needs no allocation on the audio thread.
  void prepare(uint32_t channels, uint32_t maxFrames, AudioBusActivity activity)
  {
    _maxFrames = maxFrames;
    _samples32.resize(size_t(channels) * maxFrames);
    _samples64.resize(size_t(channels) * maxFrames);
    _channels32.resize(channels);
    _channels64.resize(channels);
    for (uint32_t channel = 0; channel < channels; ++channel)
    {
      _channels32[channel] = maxFrames ? _samples32.data() + size_t(channel) * maxFrames : nullptr;
      _channels64[channel] = maxFrames ? _samples64.data() + size_t(channel) * maxFrames : nullptr;
    }
    setActivity(activity);
  }

  // Publish forwarded main-thread bus changes without locking the audio thread;
  // the storage prepared before activation remains unchanged.
  void setActivity(AudioBusActivity activity)
  {
    _activity.store(activity, std::memory_order_relaxed);
  }

  // [realtime-safe] A null host bus means an omitted trailing bus, not a missing
  // channel array. Only explicitly inactive buses may omit their sample buffers.
  bool map(const Steinberg::Vst::AudioBusBuffers *host, Steinberg::int32 sampleSize, uint32_t frames,
           clap_audio_buffer_t &port)
  {
    if (frames > _maxFrames) return false;
    port.data32 = nullptr;
    port.data64 = nullptr;
    port.channel_count = static_cast<uint32_t>(_channels32.size());
    port.constant_mask = 0;
    port.latency = 0;
    if (host && host->numChannels != static_cast<Steinberg::int32>(port.channel_count)) return false;

    const auto activity = _activity.load(std::memory_order_relaxed);
    if (sampleSize == Steinberg::Vst::kSample32)
      return mapSamples(host ? host->channelBuffers32 : nullptr, host != nullptr, frames, activity,
                        _channels32, port.data32, port.constant_mask);
    if (sampleSize == Steinberg::Vst::kSample64)
      return mapSamples(host ? host->channelBuffers64 : nullptr, host != nullptr, frames, activity,
                        _channels64, port.data64, port.constant_mask);
    return false;
  }

 private:
  template <typename T>
  bool mapSamples(T **hostChannels, bool hostBusPresent, uint32_t frames, AudioBusActivity activity,
                  std::vector<T *> &scratchChannels, T **&destination, uint64_t &constantMask)
  {
    const auto channels = scratchChannels.size();
    // Even an inactive, present VST3 bus must supply the channel-pointer array.
    if (hostBusPresent && channels > 0 && !hostChannels) return false;
    if (activity == AudioBusActivity::Active)
    {
      if (!hostBusPresent) return false;
      for (size_t channel = 0; channel < channels; ++channel)
        if (!hostChannels[channel]) return false;
      destination = hostChannels;
      return true;
    }

    // Reset every block: a plugin without the activation extension may have
    // written to an inactive output during the preceding process call.
    for (auto *samples : scratchChannels) std::fill_n(samples, frames, T(0));
    destination = scratchChannels.data();
    constantMask = channels >= 64 ? UINT64_MAX : (uint64_t(1) << channels) - 1;
    return true;
  }

  std::atomic<AudioBusActivity> _activity{AudioBusActivity::Inactive};
  uint32_t _maxFrames = 0;
  std::vector<float> _samples32;
  std::vector<double> _samples64;
  std::vector<float *> _channels32;
  std::vector<double *> _channels64;
};
}  // namespace Clap
