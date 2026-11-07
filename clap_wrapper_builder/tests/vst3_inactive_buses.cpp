// Inactive optional buses must not silence valid main audio. Exercise the real
// ProcessAdapter with a strict CLAP processor, including in-place and f64 audio.
#include "../clap-wrapper/src/detail/vst3/process.h"
#include <pluginterfaces/vst/vstspeaker.h>
#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <new>
#include <type_traits>

namespace
{
thread_local bool processing = false;

void require(bool condition, const char *message)
{
  if (!condition)
  {
    std::fprintf(stderr, "%s\n", message);
    std::abort();
  }
}
}  // namespace

// This regression guards the audio callback itself against new allocations.
void *operator new(std::size_t size)
{
  require(!processing, "Allocation in the process callback");
  if (void *memory = std::malloc(size ? size : 1)) return memory;
  throw std::bad_alloc();
}
void *operator new[](std::size_t size)
{
  return ::operator new(size);
}
void operator delete(void *memory) noexcept
{
  std::free(memory);
}
void operator delete[](void *memory) noexcept
{
  std::free(memory);
}
void operator delete(void *memory, std::size_t) noexcept
{
  std::free(memory);
}
void operator delete[](void *memory, std::size_t) noexcept
{
  std::free(memory);
}

namespace
{
using namespace Steinberg;
using namespace Steinberg::Vst;

template <typename T>
T **samples(const clap_audio_buffer_t &buffer)
{
  if constexpr (std::is_same_v<T, double>)
  {
    require(!buffer.data32, "Only the requested precision may be supplied");
    return buffer.data64;
  }
  else
  {
    require(!buffer.data64, "Only the requested precision may be supplied");
    return buffer.data32;
  }
}

template <typename T>
void setSamples(AudioBusBuffers &bus, T **channels)
{
  if constexpr (std::is_same_v<T, double>)
    bus.channelBuffers64 = channels;
  else
    bus.channelBuffers32 = channels;
}

template <typename T>
class Fixture
{
 public:
  explicit Fixture(bool inPlace) : _inPlace(inPlace)
  {
    for (auto *buses : {&_inputs, &_outputs})
    {
      buses->push_back(owned(new AudioBus(STR16("Main"), kMain, 0, SpeakerArr::kStereo)));
      buses->push_back(owned(new AudioBus(STR16("Aux"), kAux, 0, SpeakerArr::kStereo)));
      buses->at(0)->setActive(true);
      buses->at(1)->setActive(false);
    }
    _plugin.plugin_data = this;
    _plugin.process = process;
    _paramsExtension.flush = flush;
    _adapter.setupProcessing(&_plugin, &_paramsExtension, _inputs, _outputs, 8, 0, {}, _params, nullptr,
                             nullptr, _gestures, false, false);
    data.numSamples = 7;
    data.symbolicSampleSize = std::is_same_v<T, double> ? kSample64 : kSample32;
    data.numInputs = data.numOutputs = 2;
    data.inputs = _hostInputs;
    data.outputs = _hostOutputs;
    for (int channel = 0; channel < 2; ++channel)
    {
      _mainInput[channel] = _inputSamples[channel];
      _mainOutput[channel] = inPlace ? _inputSamples[channel] : _outputSamples[channel];
      _auxInput[channel] = _sidechainSamples[channel];
      _auxOutput[channel] = _discardedSamples[channel];
    }
    for (int bus = 0; bus < 2; ++bus) _hostInputs[bus].numChannels = _hostOutputs[bus].numChannels = 2;
    setSamples(_hostInputs[0], _mainInput);
    setSamples(_hostOutputs[0], _mainOutput);
    nullAuxSamples();
  }

  void check()
  {
    expectAudio();  // Null samples in present, inactive buses.
    expectAudio();  // Scratch outputs must be reset after a plugin writes to them.

    data.numInputs = data.numOutputs = 1;
    expectAudio();  // VST3 also permits omitted trailing inactive buses.
    data.numInputs = data.numOutputs = 2;

    setSamples(_hostInputs[1], _auxInput);
    setSamples(_hostOutputs[1], _auxOutput);
    expectAudio();  // Inactive host buffers must be ignored even when non-null.
    require(_discardedSamples[0][0] == T(0.75), "Inactive host output was overwritten");

    _auxActive = true;
    _adapter.activateAudioBus(kInput, 1, true);
    _adapter.activateAudioBus(kOutput, 1, true);
    expectAudio();  // Reactivation must restore the host's input/output buffers.
    require(_discardedSamples[0][0] == T(0.625), "Reactivated output was not written");

    setSamples(_hostInputs[1], _nullChannels);
    expectRejected();  // Active null input samples are errors, not silence inputs.
    setSamples(_hostInputs[1], _auxInput);
    setSamples(_hostOutputs[1], _nullChannels);
    expectRejected();  // Active null output samples must not reach the processor.
    setSamples(_hostOutputs[1], _auxOutput);
    data.numOutputs = 1;
    expectRejected();  // Missing an active trailing output is also an error.
    data.numOutputs = 2;
    expectAudio();  // Recovery also clears stale VST3 silence flags.

    _auxActive = false;
    _adapter.activateAudioBus(kInput, 1, false);
    _adapter.activateAudioBus(kOutput, 1, false);
    nullAuxSamples();
    expectAudio();

    _hostOutputs[1].numChannels = 1;
    expectRejected();  // Inactive does not excuse a malformed bus descriptor.
    _hostOutputs[1].numChannels = 2;
    setSamples(_hostOutputs[1], static_cast<T **>(nullptr));
    expectRejected();  // Only sample pointers, not the channel array, may be null.
    nullAuxSamples();
    data.numInputs = 1;
    data.inputs = nullptr;
    expectRejected();
    data.inputs = _hostInputs;
    data.numInputs = 2;
    data.numSamples = 9;
    expectRejected();  // Never exceed the preallocated maximum (8 frames).
    data.numSamples = 7;

    _status = CLAP_PROCESS_ERROR;
    expectAudio();  // A real processor failure still clears even partially written audio.
    for (auto status : {CLAP_PROCESS_CONTINUE, CLAP_PROCESS_SLEEP, CLAP_PROCESS_TAIL,
                        CLAP_PROCESS_CONTINUE_IF_NOT_QUIET})
    {
      _status = status;
      expectAudio();  // Successful statuses must preserve the current output.
    }

    data.numSamples = 0;
    data.numInputs = data.numOutputs = 0;
    data.inputs = data.outputs = nullptr;
    const auto calls = _calls;
    const auto flushes = _flushes;
    run();
    require(_calls == calls && _flushes == flushes + 1, "Zero-frame parameter flush changed");
    require(_inputSamples[0][0] == T(0.25), "Zero-frame flush touched audio");
  }

  ProcessData data;

 private:
  static clap_process_status CLAP_ABI process(const clap_plugin_t *plugin, const clap_process_t *block)
  {
    auto &self = *static_cast<Fixture *>(plugin->plugin_data);
    ++self._calls;
    require(block->audio_inputs_count == 2 && block->audio_outputs_count == 2,
            "CLAP port count changed when VST3 omitted a bus");
    for (uint32_t bus = 0; bus < 2; ++bus)
    {
      const auto &input = block->audio_inputs[bus];
      const auto &output = block->audio_outputs[bus];
      require(input.channel_count == 2 && output.channel_count == 2, "Channel count changed");
      require(samples<T>(input) && samples<T>(output), "Missing CLAP channel array");
      const bool inactive = bus == 1 && !self._auxActive;
      require(input.constant_mask == (inactive ? 3u : 0u), "Incorrect input constant mask");
      require(output.constant_mask == (inactive ? 3u : 0u), "Stale output constant mask");
      for (uint32_t channel = 0; channel < 2; ++channel)
      {
        require(samples<T>(input)[channel] && samples<T>(output)[channel], "Null CLAP samples");
        for (uint32_t frame = 0; frame < block->frames_count; ++frame)
          if (inactive)
          {
            require(samples<T>(input)[channel][frame] == T(0), "Inactive input is not silent");
            require(samples<T>(output)[channel][frame] == T(0), "Scratch output was not reset");
          }
        for (uint32_t other = 0; other < bus * 2 + channel; ++other)
          require(samples<T>(output)[channel] != samples<T>(block->audio_outputs[other / 2])[other % 2],
                  "Output channels alias each other");
      }
    }
    for (uint32_t channel = 0; channel < 2; ++channel)
      for (uint32_t frame = 0; frame < block->frames_count; ++frame)
      {
        const auto value = samples<T>(block->audio_inputs[0])[channel][frame] * T(2) +
                           samples<T>(block->audio_inputs[1])[channel][frame];
        samples<T>(block->audio_outputs[0])[channel][frame] = value;
        samples<T>(block->audio_outputs[1])[channel][frame] = value;
      }
    return self._status;
  }

  static void CLAP_ABI flush(const clap_plugin_t *plugin, const clap_input_events_t *,
                             const clap_output_events_t *)
  {
    ++static_cast<Fixture *>(plugin->plugin_data)->_flushes;
  }

  void nullAuxSamples()
  {
    setSamples(_hostInputs[1], _nullChannels);
    setSamples(_hostOutputs[1], _nullChannels);
  }

  void run()
  {
    for (int channel = 0; channel < 2; ++channel)
    {
      std::fill_n(_inputSamples[channel], 16, T(0.25));
      std::fill_n(_outputSamples[channel], 16, T(-0.75));
      std::fill_n(_sidechainSamples[channel], 16, T(0.125));
      std::fill_n(_discardedSamples[channel], 16, T(0.75));
    }
    processing = true;
    _adapter.process(data);
    processing = false;
  }

  void checkMain(T expected)
  {
    for (auto *channel : _mainOutput)
    {
      for (int frame = 0; frame < data.numSamples; ++frame)
        require(channel[frame] == expected, "Main output is incorrect");
      require(channel[data.numSamples] == (_inPlace ? T(0.25) : T(-0.75)),
              "Wrote past the current block");
    }
  }

  void expectAudio()
  {
    const auto calls = _calls;
    run();
    require(_calls == calls + 1, "Valid main audio did not reach CLAP processing");
    const bool failed = _status == CLAP_PROCESS_ERROR;
    checkMain(failed ? T(0) : (_auxActive ? T(0.625) : T(0.5)));
    require(_hostOutputs[0].silenceFlags == (failed ? 3u : 0u), "Incorrect VST3 silence flags");
  }

  void expectRejected()
  {
    const auto calls = _calls;
    run();
    require(_calls == calls, "Invalid buffers reached CLAP processing");
    checkMain(T(0));
    require(_hostOutputs[0].silenceFlags == 3, "Rejected block is not marked silent");
  }

  bool _inPlace;
  bool _auxActive = false;
  int _calls = 0;
  int _flushes = 0;
  clap_process_status _status = CLAP_PROCESS_CONTINUE;
  BusList _inputs{kAudio, kInput};
  BusList _outputs{kAudio, kOutput};
  ParameterContainer _params;
  std::vector<clap_id> _gestures;
  clap_plugin_t _plugin{};
  clap_plugin_params_t _paramsExtension{};
  Clap::ProcessAdapter _adapter;
  AudioBusBuffers _hostInputs[2]{};
  AudioBusBuffers _hostOutputs[2]{};
  T _inputSamples[2][16]{};
  T _outputSamples[2][16]{};
  T _sidechainSamples[2][16]{};
  T _discardedSamples[2][16]{};
  T *_mainInput[2]{};
  T *_mainOutput[2]{};
  T *_auxInput[2]{};
  T *_auxOutput[2]{};
  T *_nullChannels[2]{};
};
}  // namespace

int main()
{
  for (bool inPlace : {false, true})
  {
    Fixture<float>(inPlace).check();
    Fixture<double>(inPlace).check();
  }
  std::puts("VST3 inactive-bus processing regression passed (f32/f64, separate/in-place)");
}
