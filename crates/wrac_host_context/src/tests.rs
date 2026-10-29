use super::*;
// Only the macOS host tables exercise wrapper-reported host names.
#[cfg(target_os = "macos")]
use crate::identity::host_from_wrapper_if_needed;

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
    let live = detect_host_from_path("/Applications/Ableton Live 11 Suite.app/Contents/MacOS/Live");
    assert_eq!(live.display_name, "Ableton Live 11");
    assert_eq!(live.family, HostFamily::AbletonLive);
    assert_eq!(live.version, Some(HostVersion::major(11)));

    let luna = detect_host_from_path("/Applications/LUNA.app/Contents/MacOS/LUNA");
    assert_eq!(luna.display_name, "LUNA");
    assert_eq!(luna.family, HostFamily::Luna);
}

#[cfg(target_os = "macos")]
#[test]
fn wrapper_name_preserves_a_version_detected_from_the_same_host_process() {
    let detected = detect_host_from_path("/Applications/Cubase 10.app/Contents/MacOS/Cubase");
    let resolved =
        host_from_wrapper_if_needed(detected, PluginFormat::Vst3, Some("Cubase (CLAP-as-VST3)"));
    assert_eq!(resolved.family, HostFamily::SteinbergCubase);
    assert_eq!(resolved.version, Some(HostVersion::major(10)));
}

#[cfg(target_os = "macos")]
#[test]
fn recognizes_apple_daws_hosted_by_the_au_service() {
    let path = "/System/Library/Frameworks/AudioToolbox.framework/XPCServices/AUHostingServiceXPC_arrow.xpc/Contents/MacOS/AUHostingServiceXPC_arrow";
    for (name, expected) in [
        (
            "GarageBand 10.4.8 (CLAP-as-AUv2)",
            HostFamily::AppleGarageBand,
        ),
        ("Logic Pro 11.0 (CLAP-as-AUv2)", HostFamily::AppleLogic),
        ("MainStage 3 (CLAP-as-AUv2)", HostFamily::AppleMainStage),
        ("GarageBand Helper 1 (CLAP-as-VST3)", HostFamily::Unknown),
        ("GarageBandHelper 1 (CLAP-as-AUv2)", HostFamily::Unknown),
        ("GarageBand Helper 1 (CLAP-as-AUv2)", HostFamily::Unknown),
        ("CLAP-as-AUv2", HostFamily::Unknown),
    ] {
        let detected = host_from_wrapper_if_needed(
            detect_host_from_path(path),
            PluginFormat::detect(name),
            Some(name),
        );
        assert_eq!(detected.family, expected, "{name}");
        assert_eq!(detected.process_path, path);
        assert_eq!(detected.process_name, "AUHostingServiceXPC_arrow");
    }

    // Scanner and other processes must retain their original identity.
    let detected = host_from_wrapper_if_needed(
        detect_host_from_path("/usr/bin/auvaltool"),
        PluginFormat::Au,
        Some("GarageBand 10.4.8 (CLAP-as-AUv2)"),
    );
    assert_eq!(detected.family, HostFamily::AppleAuval);
}

#[test]
fn host_identifier_can_arrive_after_initial_detection() {
    let (context, writer) = HostContext::new(
        detect_host_from_path("/some/unknown-host"),
        PluginFormat::Au,
        SystemContext::detect(),
    );
    // Products keep clones from instance creation; they must observe later identifiers.
    let product_clone = context.clone();
    assert_eq!(product_clone.host().family, HostFamily::Unknown);
    writer.set(Some("com.apple.garageband".into()));
    assert_eq!(product_clone.host().family, HostFamily::AppleGarageBand);
    writer.set(Some("com.apple.logic.pro".into()));
    assert_eq!(context.host().family, HostFamily::AppleLogic);
    writer.set(Some("unrecognized".into()));
    assert_eq!(context.host().family, HostFamily::Unknown);
    writer.set(None);
    assert_eq!(context.host().family, HostFamily::Unknown);

    let (other_format, writer) = HostContext::new(
        detect_host_from_path("/some/unknown-host"),
        PluginFormat::Vst3,
        SystemContext::detect(),
    );
    writer.set(Some("com.apple.garageband".into()));
    assert_eq!(other_format.host().family, HostFamily::Unknown);

    // A validator hosted by an application keeps its process role. Build the host directly
    // because the executable-path tables only recognize auval on macOS.
    let (validator, writer) = HostContext::new(
        DetectedHost {
            display_name: "Apple auval".into(),
            process_name: "auvaltool".into(),
            process_path: "/usr/bin/auvaltool".into(),
            family: HostFamily::AppleAuval,
            version: None,
        },
        PluginFormat::Au,
        SystemContext::detect(),
    );
    writer.set(Some("com.apple.garageband".into()));
    assert_eq!(validator.host().family, HostFamily::AppleAuval);
}

#[cfg(target_os = "windows")]
#[test]
fn detects_windows_hosts_like_existing_adapter() {
    let cubase = detect_host_from_path(r"C:\Program Files\Steinberg\Cubase 10\vst2xscanner.exe");
    assert_eq!(cubase.display_name, "Steinberg Cubase 10");
    assert_eq!(cubase.family, HostFamily::SteinbergCubase);
    assert_eq!(cubase.version, Some(HostVersion::major(10)));

    let live = detect_host_from_path(r"C:\Program Files\Ableton\Live 11 Suite\Program\Live 11.exe");
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
