// A state recall must update AAX's values without recording automation.
#include "detail/aax/parameter_sync.h"
#include <AAX_CLinearTaperDelegate.h>
#include <AAX_CNumberDisplayDelegate.h>
#include <AAX_CParameter.h>
#include <AAX_IAutomationDelegate.h>
#include <cmath>
#include <cstdio>

struct Delegate : AAX_IAutomationDelegate
{
  mutable int current = 0;
  mutable int requested = 0;
  int touched = 0;
  AAX_Result RegisterParameter(AAX_CParamID) override { return AAX_SUCCESS; }
  AAX_Result UnregisterParameter(AAX_CParamID) override { return AAX_SUCCESS; }
  AAX_Result PostSetValueRequest(AAX_CParamID, double) const override
  {
    ++requested;
    return AAX_SUCCESS;
  }
  AAX_Result PostCurrentValue(AAX_CParamID, double) const override
  {
    ++current;
    return AAX_SUCCESS;
  }
  AAX_Result PostTouchRequest(AAX_CParamID) override
  {
    ++touched;
    return AAX_SUCCESS;
  }
  AAX_Result PostReleaseRequest(AAX_CParamID) override { return AAX_SUCCESS; }
  AAX_Result GetTouchState(AAX_CParamID, AAX_CBoolean *out) override
  {
    *out = false;
    return AAX_SUCCESS;
  }
  AAX_Result ParameterNameChanged(AAX_CParamID) override { return AAX_SUCCESS; }
};

static bool CLAP_ABI getValue(const clap_plugin_t *, clap_id id, double *out)
{
  if (id != 7) return false;
  *out = -6.0;
  return true;
}

int main()
{
  Delegate delegate;
  AAX_CParameterManager manager;
  manager.Initialize(&delegate);
  manager.AddParameter(new AAX_CParameter<double>(
      "output", AAX_CString("Output"), 0.0, AAX_CLinearTaperDelegate<double>(0.0, 1.0),
      AAX_CNumberDisplayDelegate<double>(), true));
  manager.AddParameter(new AAX_CParameter<double>(
      "unavailable", AAX_CString("Unavailable"), 0.7, AAX_CLinearTaperDelegate<double>(0.0, 1.0),
      AAX_CNumberDisplayDelegate<double>(), true));

  clap_plugin_t plugin{};
  clap_plugin_params_t params{};
  params.get_value = getValue;
  clap_param_info_t info{};
  info.id = 7;
  info.min_value = -24.0;
  info.max_value = 12.0;
  std::map<uint32_t, std::shared_ptr<AAXWrappedParameterInfo_t>> wrapped;
  wrapped.emplace(info.id, std::make_shared<AAXWrappedParameterInfo_t>(&plugin, info, &params, "output"));
  // A failed CLAP lookup must leave an unrelated AAX control untouched.
  info.id = 8;
  wrapped.emplace(info.id, std::make_shared<AAXWrappedParameterInfo_t>(&plugin, info, &params, "unavailable"));

  syncAAXParameterValues(&plugin, &params, wrapped, manager);
  auto *output = manager.GetParameterByID("output");
  auto *unavailable = manager.GetParameterByID("unavailable");
  if (!output || std::abs(output->GetNormalizedValue() - 0.5) > 1e-12 ||
      !unavailable || std::abs(unavailable->GetNormalizedValue() - 0.7) > 1e-12 ||
      delegate.current != 1 || delegate.requested != 0 || delegate.touched != 0)
  {
    std::fprintf(stderr, "AAX rescan did not preserve value or automation semantics\n");
    return 1;
  }
  return 0;
}
