use std::{path::Path, sync::OnceLock};

#[cfg(target_os = "macos")]
use std::ffi::CStr;

/// Host process identity plus the wrapper format inferred from CLAP metadata.
///
/// The host family is intentionally diagnostic/mechanical context, not policy. Callers
/// decide which workaround or compatibility decision to apply from this value so product
/// behavior does not accumulate inside the detection table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostContext {
    pub host: DetectedHost,
    pub plugin_format: PluginFormat,
    pub system: SystemContext,
}

impl HostContext {
    pub fn detect_current(clap_host_name: Option<&str>) -> Self {
        Self {
            host: detect_host(),
            plugin_format: PluginFormat::detect(clap_host_name.unwrap_or_default()),
            system: SystemContext::detect(),
        }
    }

    /// Keeps validation / scanner process classification as the WRAC-owned source of truth.
    ///
    /// Product code should consume this result from the `PluginCoreContext::host_context`
    /// passed during plugin instance creation instead of duplicating process-name checks.
    pub fn is_validation_or_scan_host(&self) -> bool {
        self.host.is_validation_or_scan_host()
    }
}

/// Operating-system context captured outside the plugin entry DSO lifecycle.
///
/// Products use this for diagnostics and support bundles. Keeping it in `HostContext`
/// lets plugin entry init stay lightweight while preserving the environment details
/// that are useful once a real plugin instance is created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemContext {
    pub os: &'static str,
    pub arch: &'static str,
    pub os_version: Option<String>,
}

impl SystemContext {
    pub fn detect() -> Self {
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            os_version: cached_os_version(),
        }
    }
}

/// Parsed host identity from the process executable.
///
/// `display_name` preserves the human-readable names historically used in logs, while
/// `family` and `version` are stable fields for tests and host-specific quirk selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedHost {
    pub display_name: String,
    pub process_name: String,
    pub process_path: String,
    pub family: HostFamily,
    pub version: Option<HostVersion>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostVersion {
    pub major: u16,
    pub minor: Option<u16>,
}

impl HostVersion {
    pub const fn major(major: u16) -> Self {
        Self { major, minor: None }
    }

    pub const fn major_minor(major: u16, minor: u16) -> Self {
        Self {
            major,
            minor: Some(minor),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFormat {
    Vst3,
    Au,
    Aax,
    Unknown,
}

impl PluginFormat {
    /// Detects the outer wrapper format from clap-wrapper's host name suffix.
    ///
    /// Native CLAP hosts do not carry this suffix, so absence is reported as `Unknown`
    /// instead of guessing from the process name.
    pub fn detect(clap_host_name: &str) -> Self {
        let normalized = clap_host_name.to_ascii_lowercase();
        if normalized.contains("clap-as-vst3") {
            Self::Vst3
        } else if normalized.contains("clap-as-au") {
            Self::Au
        } else if normalized.contains("clap-as-aax") {
            Self::Aax
        } else {
            Self::Unknown
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vst3 => "vst3",
            Self::Au => "au",
            Self::Aax => "aax",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFamily {
    AbletonLive,
    AdobeAudition,
    AdobePremiere,
    AppleAuLab,
    AppleAuval,
    AppleFinalCut,
    AppleGarageBand,
    AppleInfoHelper,
    AppleLogic,
    AppleMainStage,
    Ardour,
    BitwigStudio,
    CakewalkByBandlab,
    CakewalkSonar,
    DaVinciResolve,
    DigitalPerformer,
    FlStudio,
    JuceAudioPluginHost,
    Luna,
    MagixSamplitude,
    MagixSequoia,
    MuseReceptor,
    NiMaschine,
    Pluginval,
    ProTools,
    Pyramix,
    Reason,
    Renoise,
    Reaper,
    Sadie,
    SteinbergCubase,
    SteinbergCubaseBridged,
    SteinbergNuendo,
    SteinbergTestHost,
    SteinbergWavelab,
    StudioOne,
    Tracktion,
    TracktionWaveform,
    VbVstScanner,
    ViennaEnsemblePro,
    WaveBurner,
    Unknown,
}

impl DetectedHost {
    /// Hosts that represent descriptor scans or validators, not regular DAW sessions.
    pub fn is_validation_or_scan_host(&self) -> bool {
        matches!(
            self.family,
            HostFamily::AppleAuval
                | HostFamily::Pluginval
                | HostFamily::SteinbergTestHost
                | HostFamily::VbVstScanner
        ) || contains_ignore_case(&self.process_name, "xtask")
            || contains_ignore_case(&self.process_name, "validator")
            || contains_ignore_case(&self.process_name, "clap-validator")
            || starts_with_ignore_case(&self.process_name, "aaxval_")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HostMatch {
    family: HostFamily,
    display_name: &'static str,
    version: Option<HostVersion>,
}

pub fn detect_host() -> DetectedHost {
    let process_path = host_process_path();
    detect_host_from_path(&process_path)
}

pub fn detect_host_from_path(process_path: &str) -> DetectedHost {
    let process_name = file_name_or_empty(process_path);
    let matched = detect_host_match_for_platform(process_path, &process_name);
    let (family, display_name, version) = matched
        .map(|matched| (matched.family, matched.display_name, matched.version))
        .unwrap_or((HostFamily::Unknown, "Unknown", None));
    DetectedHost {
        display_name: display_name.to_string(),
        process_name,
        process_path: process_path.to_string(),
        family,
        version,
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn _NSGetExecutablePath(buf: *mut std::os::raw::c_char, bufsize: *mut u32) -> i32;
}

#[cfg(target_os = "macos")]
fn host_process_path() -> String {
    let mut size = 8192u32;
    let mut buffer = vec![0; size as usize + 8];
    let result = unsafe { _NSGetExecutablePath(buffer.as_mut_ptr(), &mut size) };
    if result == 0 {
        return unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned();
    }

    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn host_process_path() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn file_name_or_empty(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn cached_os_version() -> Option<String> {
    static OS_VERSION: OnceLock<Option<String>> = OnceLock::new();
    OS_VERSION.get_or_init(detect_os_version).clone()
}

fn detect_os_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        detect_macos_version()
    }

    #[cfg(target_os = "windows")]
    {
        detect_windows_version()
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        detect_unix_version()
    }
}

#[cfg(target_os = "macos")]
fn detect_macos_version() -> Option<String> {
    let plist = plist::Value::from_file("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    plist_string_for_key(&plist, "ProductVersion")
}

#[cfg(any(target_os = "macos", test))]
fn plist_string_for_key(plist: &plist::Value, key: &str) -> Option<String> {
    let value = plist
        .as_dictionary()
        .and_then(|dictionary| dictionary.get(key))
        .and_then(plist::Value::as_string)?;
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct RtlOsVersionInfoW {
    os_version_info_size: u32,
    major_version: u32,
    minor_version: u32,
    build_number: u32,
    platform_id: u32,
    csd_version: [u16; 128],
}

#[cfg(target_os = "windows")]
#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version_info: *mut RtlOsVersionInfoW) -> i32;
}

#[cfg(target_os = "windows")]
fn detect_windows_version() -> Option<String> {
    let mut info = RtlOsVersionInfoW {
        os_version_info_size: std::mem::size_of::<RtlOsVersionInfoW>() as u32,
        major_version: 0,
        minor_version: 0,
        build_number: 0,
        platform_id: 0,
        csd_version: [0; 128],
    };
    let status = unsafe { RtlGetVersion(&mut info) };
    (status >= 0).then(|| {
        format!(
            "{}.{}.{}",
            info.major_version, info.minor_version, info.build_number
        )
    })
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn detect_unix_version() -> Option<String> {
    let os_release = std::fs::read_to_string("/etc/os-release").ok()?;
    os_release_value(&os_release, "PRETTY_NAME")
        .or_else(|| os_release_value(&os_release, "VERSION_ID"))
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn os_release_value(source: &str, key: &str) -> Option<String> {
    for line in source.lines() {
        let Some((candidate_key, value)) = line.split_once('=') else {
            continue;
        };
        if candidate_key != key {
            continue;
        }
        let value = value.trim().trim_matches('"').to_string();
        return (!value.is_empty()).then_some(value);
    }
    None
}

fn contains_ignore_case(value: &str, pattern: &str) -> bool {
    value
        .as_bytes()
        .windows(pattern.len())
        .any(|window| window.eq_ignore_ascii_case(pattern.as_bytes()))
}

fn starts_with_ignore_case(value: &str, pattern: &str) -> bool {
    value
        .as_bytes()
        .get(..pattern.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(pattern.as_bytes()))
}

fn host(family: HostFamily, display_name: &'static str, version: Option<HostVersion>) -> HostMatch {
    HostMatch {
        family,
        display_name,
        version,
    }
}

fn detect_host_match_for_platform(host_path: &str, host_filename: &str) -> Option<HostMatch> {
    #[cfg(target_os = "macos")]
    {
        detect_host_macos(host_path, host_filename)
    }

    #[cfg(target_os = "windows")]
    {
        detect_host_windows(host_path, host_filename)
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = host_path;
        detect_host_unix(host_filename)
    }
}

#[cfg(target_os = "macos")]
fn detect_host_macos(host_path: &str, host_filename: &str) -> Option<HostMatch> {
    if contains_ignore_case(host_path, "Final Cut Pro.app")
        || contains_ignore_case(host_path, "Final Cut Pro Trial.app")
    {
        return Some(host(HostFamily::AppleFinalCut, "Final Cut", None));
    }
    if contains_ignore_case(host_path, "Live 6") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 6",
            Some(HostVersion::major(6)),
        ));
    }
    if contains_ignore_case(host_path, "Live 7") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 7",
            Some(HostVersion::major(7)),
        ));
    }
    if contains_ignore_case(host_path, "Live 8") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 8",
            Some(HostVersion::major(8)),
        ));
    }
    if contains_ignore_case(host_path, "Live 9") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 9",
            Some(HostVersion::major(9)),
        ));
    }
    if contains_ignore_case(host_path, "Live 10") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 10",
            Some(HostVersion::major(10)),
        ));
    }
    if contains_ignore_case(host_path, "Live 11") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 11",
            Some(HostVersion::major(11)),
        ));
    }
    if contains_ignore_case(host_filename, "Live") {
        return Some(host(HostFamily::AbletonLive, "Ableton Live", None));
    }
    if contains_ignore_case(host_filename, "Audition") {
        return Some(host(HostFamily::AdobeAudition, "Adobe Audition", None));
    }
    if contains_ignore_case(host_filename, "Adobe Premiere") {
        return Some(host(HostFamily::AdobePremiere, "Adobe Premiere", None));
    }
    if contains_ignore_case(host_filename, "GarageBand") {
        return Some(host(HostFamily::AppleGarageBand, "Apple GarageBand", None));
    }
    if contains_ignore_case(host_filename, "Logic") {
        return Some(host(HostFamily::AppleLogic, "Apple Logic", None));
    }
    if contains_ignore_case(host_filename, "MainStage") {
        return Some(host(HostFamily::AppleMainStage, "Apple MainStage", None));
    }
    if contains_ignore_case(host_filename, "AU Lab") {
        return Some(host(HostFamily::AppleAuLab, "AU Lab", None));
    }
    if contains_ignore_case(host_filename, "Pro Tools") {
        return Some(host(HostFamily::ProTools, "ProTools", None));
    }
    if contains_ignore_case(host_filename, "Nuendo 3") {
        return Some(host(
            HostFamily::SteinbergNuendo,
            "Steinberg Nuendo 3",
            Some(HostVersion::major(3)),
        ));
    }
    if contains_ignore_case(host_filename, "Nuendo 4") {
        return Some(host(
            HostFamily::SteinbergNuendo,
            "Steinberg Nuendo 4",
            Some(HostVersion::major(4)),
        ));
    }
    if contains_ignore_case(host_filename, "Nuendo 5") {
        return Some(host(
            HostFamily::SteinbergNuendo,
            "Steinberg Nuendo 5",
            Some(HostVersion::major(5)),
        ));
    }
    if contains_ignore_case(host_filename, "Nuendo") {
        return Some(host(HostFamily::SteinbergNuendo, "Steinberg Nuendo", None));
    }
    detect_cubase_macos(host_path, host_filename).or_else(|| {
        if contains_ignore_case(host_path, "Wavelab 7") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab 7",
                Some(HostVersion::major(7)),
            ))
        } else if contains_ignore_case(host_path, "Wavelab 8") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab 8",
                Some(HostVersion::major(8)),
            ))
        } else if contains_ignore_case(host_filename, "Wavelab") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab",
                None,
            ))
        } else if contains_ignore_case(host_filename, "WaveBurner") {
            Some(host(HostFamily::WaveBurner, "WaveBurner", None))
        } else if contains_ignore_case(host_path, "Digital Performer") {
            Some(host(HostFamily::DigitalPerformer, "DigitalPerformer", None))
        } else if contains_ignore_case(host_filename, "reaper") {
            Some(host(HostFamily::Reaper, "Reaper", None))
        } else if contains_ignore_case(host_filename, "Reason") {
            Some(host(HostFamily::Reason, "Reason", None))
        } else if contains_ignore_case(host_path, "Studio One") {
            Some(host(HostFamily::StudioOne, "Studio One", None))
        } else if starts_with_ignore_case(host_filename, "Waveform") {
            Some(host(
                HostFamily::TracktionWaveform,
                "Tracktion Waveform",
                None,
            ))
        } else if contains_ignore_case(host_path, "Tracktion 3") {
            Some(host(
                HostFamily::Tracktion,
                "Tracktion 3",
                Some(HostVersion::major(3)),
            ))
        } else if contains_ignore_case(host_filename, "Tracktion") {
            Some(host(HostFamily::Tracktion, "Tracktion", None))
        } else if contains_ignore_case(host_filename, "Renoise") {
            Some(host(HostFamily::Renoise, "Renoise", None))
        } else if contains_ignore_case(host_filename, "Resolve") {
            Some(host(HostFamily::DaVinciResolve, "DaVinci Resolve", None))
        } else if starts_with_ignore_case(host_filename, "Bitwig") {
            Some(host(HostFamily::BitwigStudio, "Bitwig Studio", None))
        } else if contains_ignore_case(host_filename, "OsxFL") {
            Some(host(HostFamily::FlStudio, "FL Studio", None))
        } else if contains_ignore_case(host_filename, "pluginval") {
            Some(host(HostFamily::Pluginval, "pluginval", None))
        } else if contains_ignore_case(host_filename, "AudioPluginHost") {
            Some(host(
                HostFamily::JuceAudioPluginHost,
                "JUCE AudioPluginHost",
                None,
            ))
        } else if contains_ignore_case(host_path, "LUNA.app")
            || contains_ignore_case(host_filename, "LUNA")
        {
            Some(host(HostFamily::Luna, "LUNA", None))
        } else if contains_ignore_case(host_filename, "Maschine") {
            Some(host(HostFamily::NiMaschine, "NI Maschine", None))
        } else if contains_ignore_case(host_filename, "Vienna Ensemble Pro") {
            Some(host(
                HostFamily::ViennaEnsemblePro,
                "Vienna Ensemble Pro",
                None,
            ))
        } else if contains_ignore_case(host_filename, "auvaltool") {
            Some(host(HostFamily::AppleAuval, "auval", None))
        } else if contains_ignore_case(host_filename, "com.apple.audio.infohelper") {
            Some(host(
                HostFamily::AppleInfoHelper,
                "com.apple.audio.InfoHelper",
                None,
            ))
        } else {
            None
        }
    })
}

#[cfg(target_os = "macos")]
fn detect_cubase_macos(host_path: &str, host_filename: &str) -> Option<HostMatch> {
    if contains_ignore_case(host_filename, "Cubase 4") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 4",
            Some(HostVersion::major(4)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase 5") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 5",
            Some(HostVersion::major(5)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase 6") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 6",
            Some(HostVersion::major(6)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase 7") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 7",
            Some(HostVersion::major(7)),
        ));
    }
    if contains_ignore_case(host_path, "Cubase 8.5.app") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 8.5",
            Some(HostVersion::major_minor(8, 5)),
        ));
    }
    if contains_ignore_case(host_path, "Cubase 8.app") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 8",
            Some(HostVersion::major(8)),
        ));
    }
    if contains_ignore_case(host_path, "Cubase 9.5.app") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 9.5",
            Some(HostVersion::major_minor(9, 5)),
        ));
    }
    if contains_ignore_case(host_path, "Cubase 9.app") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 9",
            Some(HostVersion::major(9)),
        ));
    }
    if contains_ignore_case(host_path, "Cubase 10.5.app") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 10.5",
            Some(HostVersion::major_minor(10, 5)),
        ));
    }
    if contains_ignore_case(host_path, "Cubase 10.app") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 10",
            Some(HostVersion::major(10)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase") {
        return Some(host(HostFamily::SteinbergCubase, "Steinberg Cubase", None));
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_host_windows(host_path: &str, host_filename: &str) -> Option<HostMatch> {
    if contains_ignore_case(host_filename, "Live 6") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 6",
            Some(HostVersion::major(6)),
        ));
    }
    if contains_ignore_case(host_filename, "Live 7") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 7",
            Some(HostVersion::major(7)),
        ));
    }
    if contains_ignore_case(host_filename, "Live 8") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 8",
            Some(HostVersion::major(8)),
        ));
    }
    if contains_ignore_case(host_filename, "Live 9") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 9",
            Some(HostVersion::major(9)),
        ));
    }
    if contains_ignore_case(host_filename, "Live 10") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 10",
            Some(HostVersion::major(10)),
        ));
    }
    if contains_ignore_case(host_filename, "Live 11") {
        return Some(host(
            HostFamily::AbletonLive,
            "Ableton Live 11",
            Some(HostVersion::major(11)),
        ));
    }
    if contains_ignore_case(host_filename, "Live ") {
        return Some(host(HostFamily::AbletonLive, "Ableton Live", None));
    }
    if contains_ignore_case(host_filename, "Audition") {
        return Some(host(HostFamily::AdobeAudition, "Adobe Audition", None));
    }
    if contains_ignore_case(host_filename, "Adobe Premiere") {
        return Some(host(HostFamily::AdobePremiere, "Adobe Premiere", None));
    }
    if contains_ignore_case(host_filename, "ProTools") {
        return Some(host(HostFamily::ProTools, "ProTools", None));
    }
    if contains_ignore_case(host_path, "SONAR 8") {
        return Some(host(
            HostFamily::CakewalkSonar,
            "Cakewalk Sonar 8",
            Some(HostVersion::major(8)),
        ));
    }
    if contains_ignore_case(host_filename, "SONAR") {
        return Some(host(HostFamily::CakewalkSonar, "Cakewalk Sonar", None));
    }
    if contains_ignore_case(host_filename, "Cakewalk.exe") {
        return Some(host(
            HostFamily::CakewalkByBandlab,
            "Cakewalk by Bandlab",
            None,
        ));
    }
    if contains_ignore_case(host_filename, "GarageBand") {
        return Some(host(HostFamily::AppleGarageBand, "Apple GarageBand", None));
    }
    if contains_ignore_case(host_filename, "Logic") {
        return Some(host(HostFamily::AppleLogic, "Apple Logic", None));
    }
    if contains_ignore_case(host_filename, "MainStage") {
        return Some(host(HostFamily::AppleMainStage, "Apple MainStage", None));
    }
    if starts_with_ignore_case(host_filename, "Waveform") {
        return Some(host(
            HostFamily::TracktionWaveform,
            "Tracktion Waveform",
            None,
        ));
    }
    if contains_ignore_case(host_path, "Tracktion 3") {
        return Some(host(
            HostFamily::Tracktion,
            "Tracktion 3",
            Some(HostVersion::major(3)),
        ));
    }
    if contains_ignore_case(host_filename, "Tracktion") {
        return Some(host(HostFamily::Tracktion, "Tracktion", None));
    }
    if contains_ignore_case(host_filename, "reaper") {
        return Some(host(HostFamily::Reaper, "Reaper", None));
    }
    detect_cubase_windows(host_path, host_filename).or_else(|| {
        if contains_ignore_case(host_filename, "VSTBridgeApp") {
            Some(host(
                HostFamily::SteinbergCubaseBridged,
                "Steinberg Cubase 5 Bridged",
                Some(HostVersion::major(5)),
            ))
        } else if contains_ignore_case(host_path, "Wavelab 5") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab 5",
                Some(HostVersion::major(5)),
            ))
        } else if contains_ignore_case(host_path, "Wavelab 6") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab 6",
                Some(HostVersion::major(6)),
            ))
        } else if contains_ignore_case(host_path, "Wavelab 7") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab 7",
                Some(HostVersion::major(7)),
            ))
        } else if contains_ignore_case(host_path, "Wavelab 8") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab 8",
                Some(HostVersion::major(8)),
            ))
        } else if contains_ignore_case(host_path, "Nuendo") {
            Some(host(HostFamily::SteinbergNuendo, "Steinberg Nuendo", None))
        } else if contains_ignore_case(host_filename, "Wavelab") {
            Some(host(
                HostFamily::SteinbergWavelab,
                "Steinberg Wavelab",
                None,
            ))
        } else if contains_ignore_case(host_filename, "TestHost") {
            Some(host(
                HostFamily::SteinbergTestHost,
                "Steinberg TestHost",
                None,
            ))
        } else if contains_ignore_case(host_filename, "rm-host") {
            Some(host(HostFamily::MuseReceptor, "Muse Receptor", None))
        } else if contains_ignore_case(host_filename, "Maschine") {
            Some(host(HostFamily::NiMaschine, "NI Maschine", None))
        } else if starts_with_ignore_case(host_filename, "FL")
            || contains_ignore_case(host_filename, "ilbridge.")
        {
            Some(host(HostFamily::FlStudio, "FL Studio", None))
        } else if contains_ignore_case(host_path, "Studio One") {
            Some(host(HostFamily::StudioOne, "Studio One", None))
        } else if contains_ignore_case(host_path, "Digital Performer") {
            Some(host(HostFamily::DigitalPerformer, "DigitalPerformer", None))
        } else if contains_ignore_case(host_filename, "VST_Scanner") {
            Some(host(HostFamily::VbVstScanner, "VBVSTScanner", None))
        } else if contains_ignore_case(host_path, "Merging Technologies") {
            Some(host(HostFamily::Pyramix, "Pyramix", None))
        } else if starts_with_ignore_case(host_filename, "Sam") {
            Some(host(HostFamily::MagixSamplitude, "Magix Samplitude", None))
        } else if starts_with_ignore_case(host_filename, "Sequoia") {
            Some(host(HostFamily::MagixSequoia, "Magix Sequoia", None))
        } else if contains_ignore_case(host_filename, "Reason") {
            Some(host(HostFamily::Reason, "Reason", None))
        } else if contains_ignore_case(host_filename, "Renoise") {
            Some(host(HostFamily::Renoise, "Renoise", None))
        } else if contains_ignore_case(host_filename, "Resolve") {
            Some(host(HostFamily::DaVinciResolve, "DaVinci Resolve", None))
        } else if contains_ignore_case(host_path, "Bitwig Studio") {
            Some(host(HostFamily::BitwigStudio, "Bitwig Studio", None))
        } else if contains_ignore_case(host_filename, "Sadie") {
            Some(host(HostFamily::Sadie, "SADiE", None))
        } else if contains_ignore_case(host_filename, "pluginval") {
            Some(host(HostFamily::Pluginval, "pluginval", None))
        } else if contains_ignore_case(host_filename, "AudioPluginHost") {
            Some(host(
                HostFamily::JuceAudioPluginHost,
                "JUCE AudioPluginHost",
                None,
            ))
        } else if contains_ignore_case(host_filename, "Vienna Ensemble Pro") {
            Some(host(
                HostFamily::ViennaEnsemblePro,
                "Vienna Ensemble Pro",
                None,
            ))
        } else {
            None
        }
    })
}

#[cfg(target_os = "windows")]
fn detect_cubase_windows(host_path: &str, host_filename: &str) -> Option<HostMatch> {
    if contains_ignore_case(host_filename, "Cubase4") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 4",
            Some(HostVersion::major(4)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase5") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 5",
            Some(HostVersion::major(5)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase6") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 6",
            Some(HostVersion::major(6)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase7") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 7",
            Some(HostVersion::major(7)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase8.5.exe") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 8.5",
            Some(HostVersion::major_minor(8, 5)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase8.exe") {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 8",
            Some(HostVersion::major(8)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase9.5.exe")
        || contains_ignore_case(host_path, "Cubase 9.5")
    {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 9.5",
            Some(HostVersion::major_minor(9, 5)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase9.exe")
        || contains_ignore_case(host_path, "Cubase 9")
    {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 9",
            Some(HostVersion::major(9)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase10.5.exe")
        || contains_ignore_case(host_path, "Cubase 10.5")
    {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 10.5",
            Some(HostVersion::major_minor(10, 5)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase10.exe")
        || contains_ignore_case(host_path, "Cubase 10")
    {
        return Some(host(
            HostFamily::SteinbergCubase,
            "Steinberg Cubase 10",
            Some(HostVersion::major(10)),
        ));
    }
    if contains_ignore_case(host_filename, "Cubase") {
        return Some(host(HostFamily::SteinbergCubase, "Steinberg Cubase", None));
    }
    None
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn detect_host_unix(host_filename: &str) -> Option<HostMatch> {
    if contains_ignore_case(host_filename, "Ardour") {
        return Some(host(HostFamily::Ardour, "Ardour", None));
    }
    if starts_with_ignore_case(host_filename, "Waveform") {
        return Some(host(
            HostFamily::TracktionWaveform,
            "Tracktion Waveform",
            None,
        ));
    }
    if contains_ignore_case(host_filename, "Tracktion") {
        return Some(host(HostFamily::Tracktion, "Tracktion", None));
    }
    if starts_with_ignore_case(host_filename, "Bitwig") {
        return Some(host(HostFamily::BitwigStudio, "Bitwig Studio", None));
    }
    if contains_ignore_case(host_filename, "pluginval") {
        return Some(host(HostFamily::Pluginval, "pluginval", None));
    }
    if contains_ignore_case(host_filename, "AudioPluginHost") {
        return Some(host(
            HostFamily::JuceAudioPluginHost,
            "JUCE AudioPluginHost",
            None,
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wrapper_format_from_clap_host_name() {
        assert_eq!(
            PluginFormat::detect("Cubase LE AI Elements (CLAP-as-VST3)"),
            PluginFormat::Vst3
        );
        assert_eq!(
            PluginFormat::detect("Logic Pro (CLAP-as-AU)"),
            PluginFormat::Au
        );
        assert_eq!(PluginFormat::detect("Native CLAP"), PluginFormat::Unknown);
    }

    #[test]
    fn detects_runtime_os_version() {
        let os_version = SystemContext::detect()
            .os_version
            .expect("runtime OS version should be available");
        let os_version = os_version.trim();
        assert!(!os_version.is_empty());

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            assert!(os_version.chars().any(|ch| ch.is_ascii_digit()));
            assert!(os_version.contains('.'));
        }

        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            assert!(os_version.chars().any(|ch| ch.is_ascii_alphanumeric()));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn detects_macos_hosts_like_existing_adapter() {
        let live =
            detect_host_from_path("/Applications/Ableton Live 11 Suite.app/Contents/MacOS/Live");
        assert_eq!(live.display_name, "Ableton Live 11");
        assert_eq!(live.family, HostFamily::AbletonLive);
        assert_eq!(live.version, Some(HostVersion::major(11)));

        let luna = detect_host_from_path("/Applications/LUNA.app/Contents/MacOS/LUNA");
        assert_eq!(luna.display_name, "LUNA");
        assert_eq!(luna.family, HostFamily::Luna);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn detects_windows_hosts_like_existing_adapter() {
        let cubase =
            detect_host_from_path(r"C:\Program Files\Steinberg\Cubase 10\vst2xscanner.exe");
        assert_eq!(cubase.display_name, "Steinberg Cubase 10");
        assert_eq!(cubase.family, HostFamily::SteinbergCubase);
        assert_eq!(cubase.version, Some(HostVersion::major(10)));

        let live =
            detect_host_from_path(r"C:\Program Files\Ableton\Live 11 Suite\Program\Live 11.exe");
        assert_eq!(live.display_name, "Ableton Live 11");
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    #[test]
    fn detects_unix_hosts_like_existing_adapter() {
        let pluginval = detect_host_from_path("/usr/bin/pluginval");
        assert_eq!(pluginval.display_name, "pluginval");
        assert_eq!(pluginval.family, HostFamily::Pluginval);
    }

    #[test]
    fn unknown_hosts_stay_unknown() {
        let detected = detect_host_from_path("/Applications/SomeHost.app/Contents/MacOS/SomeHost");
        assert_eq!(detected.display_name, "Unknown");
        assert_eq!(detected.family, HostFamily::Unknown);
    }

    #[test]
    fn reads_os_version_from_plist_value() {
        let plist = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "ProductVersion".to_string(),
            plist::Value::String("26.4.1".to_string()),
        )]));

        assert_eq!(
            plist_string_for_key(&plist, "ProductVersion"),
            Some("26.4.1".to_string())
        );
    }

    #[test]
    fn missing_or_empty_os_version_is_unknown() {
        let missing = plist::Value::Dictionary(plist::Dictionary::new());
        assert_eq!(plist_string_for_key(&missing, "ProductVersion"), None);

        let empty = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "ProductVersion".to_string(),
            plist::Value::String(String::new()),
        )]));
        assert_eq!(plist_string_for_key(&empty, "ProductVersion"), None);
    }
}
