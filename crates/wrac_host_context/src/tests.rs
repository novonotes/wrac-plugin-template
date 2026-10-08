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
    let live = detect_host_from_path("/Applications/Ableton Live 11 Suite.app/Contents/MacOS/Live");
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
