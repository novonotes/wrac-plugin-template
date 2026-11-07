#pragma once

#include "parameter.h"
#include <AAX_CParameterManager.h>
#include <map>
#include <memory>

// A CLAP value rescan is a state observation, not an automation gesture.
// Refresh AAX's parameter cache and its listeners without writing automation.
inline void syncAAXParameterValues(
    const clap_plugin_t *plugin, const clap_plugin_params_t *params,
    const std::map<uint32_t, std::shared_ptr<AAXWrappedParameterInfo_t>> &wrappedParameters,
    AAX_CParameterManager &manager)
{
  if (!plugin || !params || !params->get_value) return;
  for (const auto &[id, wrapped] : wrappedParameters)
  {
    double value = 0.0;
    if (!params->get_value(plugin, id, &value)) continue;
    auto *parameter = manager.GetParameterByID(wrapped->_aax_identifier.c_str());
    if (parameter) parameter->UpdateNormalizedValue(wrapped->asAAXValue(value));
  }
}
