// Studio Pro 8.1.2 on Windows queries latency after switching to offline mode
// and before reactivation, then renders without another query. Exercise that
// sequence and latency changes with the WRAC Gain state and a VST3 SDK host.
#include "public.sdk/source/vst/hosting/module.h"
#include "public.sdk/source/vst/hosting/hostclasses.h"
#include "public.sdk/source/common/memorystream.h"
#include "pluginterfaces/base/funknownimpl.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/vstspeaker.h"

#include <cstdio>
#include <stdexcept>
#include <string>

using namespace Steinberg;
using namespace Steinberg::Vst;

struct Handler : U::Implements<U::Directly<IComponentHandler>>
{
  int32 flags = 0;
  tresult PLUGIN_API beginEdit(ParamID) override { return kResultOk; }
  tresult PLUGIN_API performEdit(ParamID, ParamValue) override { return kResultOk; }
  tresult PLUGIN_API endEdit(ParamID) override { return kResultOk; }
  tresult PLUGIN_API restartComponent(int32 value) override
  {
    flags |= value;
    return kResultOk;
  }
};

static void require(bool condition, const char *message)
{
  if (!condition) throw std::runtime_error(message);
}

static void setLatency(IComponent *component, uint32 frames)
{
  std::string state = "{\"gain\":1.0,\"bypass\":false,\"editor_page\":\"controls\",\"latency_frames\":" +
                      std::to_string(frames) + "}";
  MemoryStream stream(state.data(), static_cast<TSize>(state.size()));
  require(component->setState(&stream) == kResultOk, "state load failed");
}

int main(int argc, char **argv)
{
  try
  {
    require(argc == 2, "usage: vst3_latency_cache <WRAC Gain.vst3>");
    std::string error;
    auto module = VST3::Hosting::Module::create(argv[1], error);
    if (!module) throw std::runtime_error(error);
    auto host = owned(new HostApplication());
    auto handler = owned(new Handler());
    auto &factory = module->getFactory();
    factory.setHostContext(host);
    auto classes = factory.classInfos();
    require(!classes.empty(), "no plugin classes");
    auto component = factory.createInstance<IComponent>(classes[0].ID());
    require(component && component->initialize(host) == kResultOk, "component initialization failed");
    auto processor = U::cast<IAudioProcessor>(component);
    auto controller = U::cast<IEditController>(component);
    require(processor && controller, "missing plugin interfaces");
    require(controller->setComponentHandler(handler) == kResultOk, "handler setup failed");
    SpeakerArrangement stereo = SpeakerArr::kStereo;
    require(processor->setBusArrangements(&stereo, 1, &stereo, 1) == kResultOk, "stereo setup failed");
    require(component->activateBus(kAudio, kInput, 0, true) == kResultOk, "input activation failed");
    require(component->activateBus(kAudio, kOutput, 0, true) == kResultOk, "output activation failed");
    ProcessSetup setup{kRealtime, kSample32, 512, 44100.0};
    require(processor->setupProcessing(setup) == kResultOk, "realtime setup failed");

    // No value is known before the first activation. The host must be notified
    // once the plugin can provide its actual latency.
    setLatency(component, 256);
    require(processor->getLatencySamples() == 0, "initial inactive latency is not zero");
    handler->flags = 0;
    require(component->setActive(true) == kResultOk, "first activation failed");
    require(handler->flags & kLatencyChanged, "missing first latency notification");
    // The host never queried while active: deactivation must still retain 256.
    require(component->setActive(false) == kResultOk, "first deactivation failed");
    require(processor->getLatencySamples() == 256, "inactive latency was not cached on activation");
    setup.processMode = kOffline;
    require(processor->setupProcessing(setup) == kResultOk, "offline setup failed");
    require(processor->getLatencySamples() == 256, "offline setup lost cached latency");
    handler->flags = 0;
    require(component->setActive(true) == kResultOk, "offline activation failed");
    require(handler->flags & kLatencyChanged, "missing latency notification after inactive query");
    require(processor->getLatencySamples() == 256, "offline active latency mismatch");

    require(component->setActive(false) == kResultOk, "offline deactivation failed");
    setLatency(component, 4096);
    require(processor->getLatencySamples() == 256, "inactive query did not return the last known value");
    handler->flags = 0;
    require(component->setActive(true) == kResultOk, "changed-latency activation failed");
    require(handler->flags & kLatencyChanged, "missing changed-latency notification");
    require(component->setActive(false) == kResultOk, "changed-latency deactivation failed");
    require(processor->getLatencySamples() == 4096, "updated latency was not cached");
    setLatency(component, 0);
    require(component->setActive(true) == kResultOk, "zero-latency activation failed");
    require(processor->getLatencySamples() == 0, "zero-latency update failed");
    require(component->setActive(false) == kResultOk, "final deactivation failed");
    require(processor->getLatencySamples() == 0, "zero latency was not cached");

    require(controller->setComponentHandler(nullptr) == kResultOk, "handler removal failed");
    require(component->terminate() == kResultOk, "component termination failed");
    std::puts("VST3 latency cache regression passed");
    return 0;
  }
  catch (const std::exception &e)
  {
    std::fprintf(stderr, "FAIL: %s\n", e.what());
    return 1;
  }
}
