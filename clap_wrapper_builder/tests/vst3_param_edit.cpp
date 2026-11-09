// A host may query the controller during performEdit. Drive the wrapper's queued
// CLAP events directly so this checks the real host callback without a GUI.
#include "public.sdk/source/vst/hosting/module.h"
#include "public.sdk/source/vst/hosting/hostclasses.h"
#include "pluginterfaces/base/funknownimpl.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "detail/clap/automation.h"
#include "detail/os/osutil.h"

#include <cmath>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <tuple>

using namespace Steinberg;
using namespace Steinberg::Vst;

static void require(bool condition, const char *message)
{
  if (!condition) throw std::runtime_error(message);
}

struct Handler : U::Implements<U::Directly<IComponentHandler>>
{
  IEditController *controller = nullptr;
  int phase = 0;
  ParamID expectedId = 0;
  double expectedValue = 0;
  bool coherent = true;
  tresult PLUGIN_API beginEdit(ParamID id) override
  {
    coherent &= phase == 0 && id == expectedId;
    phase = 1;
    return kResultOk;
  }
  tresult PLUGIN_API performEdit(ParamID id, ParamValue value) override
  {
    coherent &= phase == 1 && id == expectedId && std::abs(value - expectedValue) < 1e-12;
    coherent &= std::abs(controller->getParamNormalized(id) - value) < 1e-12;
    phase = 2;
    return kResultOk;
  }
  tresult PLUGIN_API endEdit(ParamID id) override
  {
    coherent &= phase == 2 && id == expectedId;
    phase = 3;
    return kResultOk;
  }
  tresult PLUGIN_API restartComponent(int32) override { return kResultOk; }
};

struct ComponentCleanup
{
  IComponent *component;
  IEditController *controller;
  ~ComponentCleanup()
  {
    // Detach the host even when an assertion fails, before unloading the plugin module.
    controller->setComponentHandler(nullptr);
    component->terminate();
  }
};

int main(int argc, char **argv)
{
  try
  {
    require(argc == 2, "usage: param_edit <plugin.vst3>");
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
    require(component && component->initialize(host) == kResultOk, "initialization failed");
    auto controller = U::cast<IEditController>(component);
    require(controller, "missing controller");
    ComponentCleanup cleanup{component, controller};
    handler->controller = controller;
    require(controller->setComponentHandler(handler) == kResultOk, "handler setup failed");
    auto *automation = dynamic_cast<Clap::IAutomation *>(component.get());
    auto *idle = dynamic_cast<os::IPlugObject *>(component.get());
    require(automation && idle, "missing wrapper interfaces");

    ParamID toggle = kNoParamId, continuous = kNoParamId;
    for (int32 i = 0; i < controller->getParameterCount(); ++i)
    {
      ParameterInfo info{};
      require(controller->getParameterInfo(i, info) == kResultOk, "parameter info failed");
      if (!(info.flags & ParameterInfo::kCanAutomate)) continue;
      if (info.stepCount == 1 && toggle == kNoParamId) toggle = info.id;
      if (info.stepCount == 0 && continuous == kNoParamId) continuous = info.id;
    }
    require(toggle != kNoParamId && continuous != kNoParamId, "missing parameters");
    // WRAC Gain publishes both parameters to CLAP over 0..1; cover stepped and continuous edits.
    for (const auto &[id, first, second] : {std::tuple<ParamID, double, double>{toggle, 0., 1.},
                                             std::tuple<ParamID, double, double>{continuous, .25, .75}})
    {
      require(controller->setParamNormalized(id, second) == kResultOk, "set initial value failed");
      for (double value : {first, second})
      {
        handler->phase = 0;
        handler->expectedId = id;
        handler->expectedValue = value;
        clap_event_param_value_t event{};
        event.param_id = id;
        event.value = value;
        automation->onBeginEdit(id);
        automation->onPerformEdit(&event);
        automation->onEndEdit(id);
        idle->onIdle();
        require(handler->coherent && handler->phase == 3, "host saw stale value or incorrect gesture");
        require(std::abs(controller->getParamNormalized(id) - value) < 1e-12, "controller value stale");
      }
    }
    std::puts("VST3 parameter edit regression passed");
    return 0;
  }
  catch (const std::exception &e)
  {
    std::fprintf(stderr, "FAIL: %s\n", e.what());
    return 1;
  }
}
