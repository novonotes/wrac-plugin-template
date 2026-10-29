// A CLAP plugin changes its latency through host.request_restart(), which the AUv2
// wrapper serves by reactivating the plugin from its idle timer while the AU stays
// initialized. This checks that path end to end: the AU reports the new latency and
// notifies listeners, rendering continues on another thread throughout the restart,
// and the audio afterwards is delayed by exactly the latency the AU reports.
//
// The latency change is driven by a state restore rather than a parameter, because the
// template plugin keeps its delay setting out of the host parameter list.
#include <AudioToolbox/AudioToolbox.h>
#include <CoreFoundation/CoreFoundation.h>
#include <atomic>
#include <cmath>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

static constexpr double kSampleRate = 48000;
static constexpr UInt32 kBlockFrames = 512;

static void require(bool condition, const char *message)
{
  if (!condition) throw std::runtime_error(message);
}
static void ok(OSStatus status)
{
  require(status == noErr, "AU operation failed");
}

// The input is silence with a single unit impulse at gImpulseAt (in the render sample
// timeline), so the output position of the impulse is the plugin's actual delay.
static std::atomic<int64_t> gImpulseAt{-1};
static OSStatus input(void *, AudioUnitRenderActionFlags *, const AudioTimeStamp *timeStamp, UInt32,
                      UInt32 frames, AudioBufferList *buffers)
{
  const auto start = static_cast<int64_t>(timeStamp->mSampleTime);
  const auto impulseAt = gImpulseAt.load();
  for (UInt32 c = 0; c < buffers->mNumberBuffers; ++c)
    for (UInt32 i = 0; i < frames; ++i)
      static_cast<float *>(buffers->mBuffers[c].mData)[i] = (start + i == impulseAt) ? 1.0f : 0.0f;
  return noErr;
}

static std::atomic<int> gLatencyNotifications{0};
static void latencyListener(void *, AudioUnit, AudioUnitPropertyID, AudioUnitScope, AudioUnitElement)
{
  ++gLatencyNotifications;
}

struct Renderer
{
  AudioUnit unit;
  int64_t sampleTime = 0;
  float left[kBlockFrames]{};
  float right[kBlockFrames]{};

  // Renders one block and returns the output sample time of the loudest sample in it,
  // or -1 when the block is silent.
  int64_t renderBlock()
  {
    AudioBufferList *buffers =
        static_cast<AudioBufferList *>(alloca(sizeof(AudioBufferList) + sizeof(AudioBuffer)));
    buffers->mNumberBuffers = 2;
    buffers->mBuffers[0] = {1, kBlockFrames * sizeof(float), left};
    buffers->mBuffers[1] = {1, kBlockFrames * sizeof(float), right};
    AudioTimeStamp timeStamp{};
    timeStamp.mFlags = kAudioTimeStampSampleTimeValid;
    timeStamp.mSampleTime = static_cast<Float64>(sampleTime);
    AudioUnitRenderActionFlags flags = 0;
    ok(AudioUnitRender(unit, &flags, &timeStamp, 0, kBlockFrames, buffers));
    int64_t peakAt = -1;
    float peak = 0.5f;
    for (UInt32 i = 0; i < kBlockFrames; ++i)
    {
      require(std::isfinite(left[i]) && std::isfinite(right[i]), "Non-finite output");
      if (std::fabs(left[i]) > peak)
      {
        peak = std::fabs(left[i]);
        peakAt = sampleTime + i;
      }
    }
    sampleTime += kBlockFrames;
    return peakAt;
  }
};

static Float64 latencySeconds(AudioUnit unit)
{
  Float64 latency = -1;
  UInt32 size = sizeof(latency);
  ok(AudioUnitGetProperty(unit, kAudioUnitProperty_Latency, kAudioUnitScope_Global, 0, &latency, &size));
  return latency;
}
static long latencyFrames(AudioUnit unit)
{
  return std::lround(latencySeconds(unit) * kSampleRate);
}

// Returns the ClassInfo of the unit with the plugin's delay setting replaced. The
// template plugin saves its state as JSON with a "latency_frames" field.
static CFPropertyListRef stateWithLatency(AudioUnit unit, int frames)
{
  CFPropertyListRef state = nullptr;
  UInt32 size = sizeof(state);
  ok(AudioUnitGetProperty(unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0, &state, &size));
  auto data = static_cast<CFDataRef>(CFDictionaryGetValue(static_cast<CFDictionaryRef>(state), CFSTR("data")));
  require(data != nullptr, "ClassInfo has no data");
  std::string json(reinterpret_cast<const char *>(CFDataGetBytePtr(data)), CFDataGetLength(data));
  const std::string key = "\"latency_frames\":";
  auto at = json.find(key);
  require(at != std::string::npos, "Plugin state has no latency_frames field");
  auto valueEnd = json.find_first_of(",}", at + key.size());
  json.replace(at + key.size(), valueEnd - (at + key.size()), std::to_string(frames));

  auto edited = CFDictionaryCreateMutableCopy(nullptr, 0, static_cast<CFDictionaryRef>(state));
  auto newData = CFDataCreate(nullptr, reinterpret_cast<const UInt8 *>(json.data()), json.size());
  CFDictionarySetValue(edited, CFSTR("data"), newData);
  CFRelease(newData);
  CFRelease(state);
  return edited;
}

// Restores a state that selects `frames` of delay, renders on another thread the whole
// time the wrapper restarts the plugin, and waits for the AU to report the new latency.
static void changeLatency(Renderer &renderer, int frames)
{
  auto state = stateWithLatency(renderer.unit, frames);
  const int notificationsBefore = gLatencyNotifications.load();
  ok(AudioUnitSetProperty(renderer.unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0, &state,
                          sizeof(state)));
  CFRelease(state);

  // The wrapper never blocks the render thread for a restart; renders during it must
  // keep succeeding (the wrapper outputs silence for those blocks).
  std::atomic<bool> rendering{true};
  std::exception_ptr renderError;
  std::thread renderThread(
      [&]
      {
        try
        {
          while (rendering) renderer.renderBlock();
        }
        catch (...)
        {
          renderError = std::current_exception();
        }
      });

  // The restart runs from the wrapper's idle timer on this thread's run loop.
  for (int i = 0; i < 100 && latencyFrames(renderer.unit) != frames; ++i)
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, false);
  rendering = false;
  renderThread.join();
  if (renderError) std::rethrow_exception(renderError);

  require(latencyFrames(renderer.unit) == frames, "AU did not report the new latency");
  require(gLatencyNotifications.load() > notificationsBefore, "AU did not notify the latency change");
}

// Sends one impulse through the plugin and returns how many samples later it came out.
static int64_t measuredDelay(Renderer &renderer)
{
  const int64_t impulseAt = renderer.sampleTime + 100;
  gImpulseAt = impulseAt;
  int64_t peakAt = -1;
  for (int block = 0; block < 32 && peakAt < 0; ++block) peakAt = renderer.renderBlock();
  gImpulseAt = -1;
  require(peakAt >= 0, "Impulse did not come out of the plugin");
  return peakAt - impulseAt;
}

int main(int argc, char **argv)
{
  if (argc != 2)
  {
    std::fprintf(stderr, "usage: au-latency-restart /path/to/plugin.component\n");
    return 2;
  }
  AudioUnit unit = nullptr;
  try
  {
    auto url = CFURLCreateFromFileSystemRepresentation(nullptr, reinterpret_cast<const UInt8 *>(argv[1]),
                                                       std::char_traits<char>::length(argv[1]), true);
    auto bundle = CFBundleCreate(nullptr, url);
    CFRelease(url);
    require(bundle && CFBundleLoadExecutable(bundle), "Cannot load AU bundle");
    auto info =
        static_cast<CFArrayRef>(CFBundleGetValueForInfoDictionaryKey(bundle, CFSTR("AudioComponents")));
    auto componentInfo = static_cast<CFDictionaryRef>(CFArrayGetValueAtIndex(info, 0));
    auto code = [&](CFStringRef key)
    {
      char text[5]{};
      require(CFStringGetCString(static_cast<CFStringRef>(CFDictionaryGetValue(componentInfo, key)),
                                 text, sizeof(text), kCFStringEncodingASCII),
              "Invalid AU code");
      return (UInt32(text[0]) << 24) | (UInt32(text[1]) << 16) | (UInt32(text[2]) << 8) |
             UInt32(text[3]);
    };
    AudioComponentDescription description{code(CFSTR("type")), code(CFSTR("subtype")),
                                          code(CFSTR("manufacturer")), 0, 0};
    auto factory = reinterpret_cast<AudioComponentFactoryFunction>(CFBundleGetFunctionPointerForName(
        bundle,
        static_cast<CFStringRef>(CFDictionaryGetValue(componentInfo, CFSTR("factoryFunction")))));
    require(factory != nullptr, "Cannot find AU factory");
    auto component = AudioComponentRegister(&description, CFSTR("Latency restart test"), 1, factory);
    ok(AudioComponentInstanceNew(component, &unit));
    CFRelease(bundle);

    AudioStreamBasicDescription format{};
    format.mSampleRate = kSampleRate;
    format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags =
        kAudioFormatFlagIsFloat | kAudioFormatFlagIsNonInterleaved | kAudioFormatFlagIsPacked;
    format.mBytesPerPacket = format.mBytesPerFrame = 4;
    format.mFramesPerPacket = 1;
    format.mChannelsPerFrame = 2;
    format.mBitsPerChannel = 32;
    for (auto scope : {kAudioUnitScope_Input, kAudioUnitScope_Output})
      ok(AudioUnitSetProperty(unit, kAudioUnitProperty_StreamFormat, scope, 0, &format, sizeof(format)));
    AURenderCallbackStruct callback{input, nullptr};
    ok(AudioUnitSetProperty(unit, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0,
                            &callback, sizeof(callback)));
    ok(AudioUnitAddPropertyListener(unit, kAudioUnitProperty_Latency, latencyListener, nullptr));
    ok(AudioUnitInitialize(unit));

    Renderer renderer{unit};
    for (int block = 0; block < 8; ++block) renderer.renderBlock();
    require(latencyFrames(unit) == 0, "Plugin must start without latency");
    require(measuredDelay(renderer) == 0, "Audio is delayed although no latency is reported");

    for (int frames : {1024, 0})
    {
      changeLatency(renderer, frames);
      const auto delay = measuredDelay(renderer);
      if (delay != frames)
      {
        std::fprintf(stderr, "reported latency %d, measured delay %lld\n", frames,
                     static_cast<long long>(delay));
        throw std::runtime_error("Audio delay does not match the reported latency");
      }
    }

    std::printf("PASS: latency reported, notified and applied across restarts\n");
    AudioUnitUninitialize(unit);
    AudioComponentInstanceDispose(unit);
    return 0;
  }
  catch (const std::exception &error)
  {
    std::fprintf(stderr, "FAIL: %s\n", error.what());
    if (unit) AudioComponentInstanceDispose(unit);
    return 1;
  }
}
