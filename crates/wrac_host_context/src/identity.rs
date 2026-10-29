use super::*;

pub(super) fn host_from_wrapper_if_needed(
    detected: DetectedHost,
    plugin_format: PluginFormat,
    clap_host_name: Option<&str>,
) -> DetectedHost {
    // The reported name identifies hosts running plugins in a helper process. Preserve
    // validator identity because product policy depends on the actual process role.
    if !detected.is_validation_or_scan_host()
        && let Some(name) = clap_host_name
    {
        let suffix = match plugin_format {
            PluginFormat::Au => " (CLAP-as-AUv2)",
            PluginFormat::Vst3 => " (CLAP-as-VST3)",
            PluginFormat::Aax => " (CLAP-as-AAX)",
            PluginFormat::Unknown => "",
        };
        let name = if suffix.is_empty() {
            Some(name)
        } else {
            name.strip_suffix(suffix)
        };
        if let Some(name) = name
            && let Some(matched) = detect_host_match_for_platform(name, name)
            && valid_apple_host_name(name, matched.family)
        {
            return DetectedHost {
                family: matched.family,
                display_name: matched.display_name.to_string(),
                // Wrapper names may omit versions that the executable path provides.
                version: matched.version.or_else(|| {
                    (matched.family == detected.family)
                        .then_some(detected.version)
                        .flatten()
                }),
                ..detected
            };
        }
    }
    detected
}

fn valid_apple_host_name(name: &str, family: HostFamily) -> bool {
    let prefix = match family {
        HostFamily::AppleGarageBand => "GarageBand",
        HostFamily::AppleLogic => "Logic Pro",
        _ => return true,
    };
    name.get(..prefix.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
        && name.get(prefix.len()..).is_some_and(|rest| {
            rest.is_empty()
                || rest
                    .strip_prefix(' ')
                    .and_then(|version| version.chars().next())
                    .is_some_and(|first| first.is_ascii_digit())
        })
}

impl HostContext {
    /// Resolves the current AU host identifier without changing the process identity.
    ///
    /// Hosts can supply this identifier after instance creation. Validation processes
    /// keep their own classification even when hosted by an application.
    pub fn host_with_au_identifier(&self, identifier: Option<&str>) -> DetectedHost {
        if self.plugin_format != PluginFormat::Au || self.host.is_validation_or_scan_host() {
            return self.host.clone();
        }
        let matched = match identifier {
            Some("com.apple.garageband") => {
                Some(host(HostFamily::AppleGarageBand, "Apple GarageBand", None))
            }
            Some("com.apple.logic.pro") => Some(host(HostFamily::AppleLogic, "Apple Logic", None)),
            Some("com.apple.mainstage") => {
                Some(host(HostFamily::AppleMainStage, "Apple MainStage", None))
            }
            _ => None,
        };
        match matched {
            Some(matched) => DetectedHost {
                family: matched.family,
                display_name: matched.display_name.to_string(),
                // The identifier does not carry a version; keep a known version for the same host.
                version: matched.version.or_else(|| {
                    (matched.family == self.host.family)
                        .then_some(self.host.version)
                        .flatten()
                }),
                ..self.host.clone()
            },
            None => self.host.clone(),
        }
    }
}
