// AU recall must replace pending host parameters, including before the first
// render. Checking ClassInfo after rendering detects divergence hidden by the
// AU parameter cache, without depending on a plugin's editor or audio algorithm.
#include <AudioToolbox/AudioToolbox.h>
#include <CoreFoundation/CoreFoundation.h>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>
#include <functional>
#include <thread>
#include <exception>

static void require(bool condition, const char *message)
{
  if (!condition) throw std::runtime_error(message);
}
static void ok(OSStatus status)
{
  require(status == noErr, "AU operation failed");
}
static OSStatus input(void *, AudioUnitRenderActionFlags *, const AudioTimeStamp *, UInt32,
                      UInt32 frames, AudioBufferList *buffers)
{
  for (UInt32 c = 0; c < buffers->mNumberBuffers; ++c)
    for (UInt32 i = 0; i < frames; ++i) static_cast<float *>(buffers->mBuffers[c].mData)[i] = 0.1f;
  return noErr;
}
static CFPropertyListRef save(AudioUnit unit)
{
  CFPropertyListRef state = nullptr;
  UInt32 size = sizeof(state);
  ok(AudioUnitGetProperty(unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0, &state, &size));
  return state;
}
static bool sameState(AudioUnit unit, CFPropertyListRef expected)
{
  auto actual = save(unit);
  // Wrapper preset names can change independently of the CLAP state payload.
  bool same = CFEqual(CFDictionaryGetValue(static_cast<CFDictionaryRef>(expected), CFSTR("data")),
                      CFDictionaryGetValue(static_cast<CFDictionaryRef>(actual), CFSTR("data")));
  CFRelease(actual);
  return same;
}
struct RecallNotification
{
  std::function<void()> action;
  std::exception_ptr error;
  bool called = false;
};
static void duringRecall(void *context, AudioUnit, AudioUnitPropertyID, AudioUnitScope, AudioUnitElement)
{
  auto &notification = *static_cast<RecallNotification *>(context);
  if (notification.called) return;
  notification.called = true;
  // AU property listeners run inside CLAP load. A render on another thread
  // must finish before load resumes; holding its spinlock across load deadlocks.
  try
  {
    notification.action();
  }
  catch (...)
  {
    notification.error = std::current_exception();
  }
}
int main(int argc, char **argv)
{
  if (argc != 2)
  {
    std::fprintf(stderr, "usage: au-state-recall /path/to/plugin.component\n");
    return 2;
  }
  AudioUnit unit = nullptr;
  CFPropertyListRef baseline = nullptr;
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
    auto component = AudioComponentRegister(&description, CFSTR("Recall regression test"), 1, factory);
    ok(AudioComponentInstanceNew(component, &unit));
    CFRelease(bundle);
    AudioStreamBasicDescription format{};
    format.mSampleRate = 48000;
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
    // ClassInfo belongs to the constructed plugin, not render resources. Hosts
    // may recall it before AU Initialize and must not need a dummy render.
    auto beforeInitialize = save(unit);
    ok(AudioUnitSetProperty(unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0,
                            &beforeInitialize, sizeof(beforeInitialize)));
    CFRelease(beforeInitialize);
    ok(AudioUnitInitialize(unit));
    UInt32 size = 0;
    Boolean writable = false;
    ok(AudioUnitGetPropertyInfo(unit, kAudioUnitProperty_ParameterList, kAudioUnitScope_Global, 0, &size,
                                &writable));
    std::vector<AudioUnitParameterID> ids(size / sizeof(AudioUnitParameterID));
    ok(AudioUnitGetProperty(unit, kAudioUnitProperty_ParameterList, kAudioUnitScope_Global, 0,
                            ids.data(), &size));
    AudioUnitParameterID id = 0;
    float changed = 0;
    bool found = false;
    for (auto candidate : ids)
    {
      AudioUnitParameterInfo parameter{};
      size = sizeof(parameter);
      ok(AudioUnitGetProperty(unit, kAudioUnitProperty_ParameterInfo, kAudioUnitScope_Global, candidate,
                              &parameter, &size));
      float value = 0;
      ok(AudioUnitGetParameter(unit, candidate, kAudioUnitScope_Global, 0, &value));
      if ((parameter.flags & kAudioUnitParameterFlag_IsWritable) &&
          parameter.minValue < parameter.maxValue)
      {
        id = candidate;
        changed = value == parameter.maxValue ? parameter.minValue : parameter.maxValue;
        found = true;
      }
      if (parameter.flags & kAudioUnitParameterFlag_CFNameRelease) CFRelease(parameter.cfNameString);
      if (found) break;
    }
    require(found, "No writable parameter available");
    baseline = save(unit);
    auto recall = [&]
    {
      return AudioUnitSetProperty(unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0,
                                  &baseline, sizeof(baseline));
    };
    auto change = [&] { ok(AudioUnitSetParameter(unit, id, kAudioUnitScope_Global, 0, changed, 0)); };
    // Saving before the first render must include host edits once idle has run.
    change();
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, false);
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0, false);
    require(!sameState(unit, baseline), "Idle did not apply an edit before the first render");
    ok(recall());
    double sampleTime = 0;
    auto render = [&]
    {
      struct
      {
        UInt32 count;
        AudioBuffer buffers[2];
      } output{2, {{1, 0, nullptr}, {1, 0, nullptr}}};
      AudioTimeStamp timestamp{};
      timestamp.mFlags = kAudioTimeStampSampleTimeValid;
      timestamp.mSampleTime = sampleTime;
      sampleTime += 512;
      AudioUnitRenderActionFlags flags = 0;
      ok(AudioUnitRender(unit, &flags, &timestamp, 0, 512,
                         reinterpret_cast<AudioBufferList *>(&output)));
    };
    change();
    ok(recall());
    render();
    require(sameState(unit, baseline), "Pending pre-recall parameter overwrote restored state");
    change();
    render();
    require(!sameState(unit, baseline), "Post-recall parameter was discarded");
    ok(recall());
    auto invalid = CFDictionaryCreateMutableCopy(nullptr, 0, static_cast<CFDictionaryRef>(baseline));
    // Invalid AU metadata must fail before the recall attempt touches the queue.
    CFDictionaryRemoveValue(invalid, CFSTR("data"));
    change();
    auto malformedStatus = AudioUnitSetProperty(unit, kAudioUnitProperty_ClassInfo,
                                                kAudioUnitScope_Global, 0, &invalid, sizeof(invalid));
    require(malformedStatus != noErr, "Malformed AU dictionary reported success");
    render();
    require(!sameState(unit, baseline), "Malformed AU dictionary discarded pending parameters");
    ok(recall());
    auto invalidData = CFDataCreate(nullptr, reinterpret_cast<const UInt8 *>("invalid"), 7);
    CFDictionarySetValue(invalid, CFSTR("data"), invalidData);
    CFRelease(invalidData);
    change();
    auto status = AudioUnitSetProperty(unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0,
                                       &invalid, sizeof(invalid));
    CFRelease(invalid);
    require(status != noErr, "Failed CLAP recall reported success");
    render();
    require(!sameState(unit, baseline), "Rejected CLAP load discarded pending parameters");
    ok(recall());
    RecallNotification notification;
    notification.action = [&]
    {
      auto renderOnAudioThread = [&]
      {
        std::exception_ptr error;
        std::thread audio(
            [&]
            {
              try
              {
                render();
              }
              catch (...)
              {
                error = std::current_exception();
              }
            });
        audio.join();
        if (error) std::rethrow_exception(error);
      };
      renderOnAudioThread();
      change();
      renderOnAudioThread();
      change();
    };
    ok(AudioUnitAddPropertyListener(unit, kAudioUnitProperty_ClassInfo, duringRecall, &notification));
    auto concurrentStatus = recall();
    ok(AudioUnitRemovePropertyListenerWithUserData(unit, kAudioUnitProperty_ClassInfo, duringRecall,
                                                   &notification));
    ok(concurrentStatus);
    if (notification.error) std::rethrow_exception(notification.error);
    if (notification.called)
    {
      render();
      require(!sameState(unit, baseline), "Edit queued during recall was discarded");
      std::puts("PASS: render during load completed and a during-recall edit survived");
    }
    else
      std::puts("SKIP: plugin does not notify ClassInfo during load; concurrent recall not tested");
    CFRelease(baseline);
    baseline = nullptr;
    ok(AudioUnitUninitialize(unit));
    ok(AudioComponentInstanceDispose(unit));
    unit = nullptr;
    std::puts("PASS: pre-recall values replaced, subsequent edits retained, failed recall preserved");
    return 0;
  }
  catch (const std::exception &error)
  {
    std::fprintf(stderr, "FAIL: %s\n", error.what());
    if (baseline) CFRelease(baseline);
    if (unit)
    {
      AudioUnitUninitialize(unit);
      AudioComponentInstanceDispose(unit);
    }
    return 1;
  }
}
