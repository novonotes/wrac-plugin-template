use std::sync::Arc;

use parking_lot::RwLock;
use wrac_clap_adapter::{
    AudioPortConfigurationRequest, AudioPortFlags, AudioPortInfo, AudioPortType, PluginAudioPorts,
    PluginConfigurableAudioPorts, PluginError, PluginResult,
};

/// host と交渉した audio layout の SoT。**non-realtime 専用**。
///
/// host の port query と configurable-audio-ports apply がここを読み書きするが、
/// `Processor::process()` は読まない。audio thread から RwLock を読むと priority
/// inversion を招くため、layout は「次に activate する processor の設定」として扱い、
/// `activate()` で snapshot して渡す ([`WracGainAudioProcessor`](crate::audio::WracGainAudioProcessor) 参照)。sidechain や
/// ambisonics など複雑な layout でも、この「store に記録 → activate で snapshot」の
/// 形は同じ。
pub(super) struct AudioLayoutStore {
    channel_count: RwLock<u32>,
}

impl AudioLayoutStore {
    pub(super) fn new(channel_count: u32) -> Self {
        Self {
            channel_count: RwLock::new(channel_count),
        }
    }

    pub(super) fn channel_count(&self) -> u32 {
        *self.channel_count.read()
    }

    fn set_channel_count(&self, channel_count: u32) {
        *self.channel_count.write() = channel_count;
    }
}

pub(super) struct WracGainAudioPorts {
    layout: Arc<AudioLayoutStore>,
}

impl WracGainAudioPorts {
    pub(super) fn new(layout: Arc<AudioLayoutStore>) -> Self {
        Self { layout }
    }
}

// gain なので main in / main out が 1 つずつ。channel 数は configurable audio
// ports 経由で host が変更できる。
impl PluginAudioPorts for WracGainAudioPorts {
    fn audio_port_count(&self, _is_input: bool) -> u32 {
        1
    }

    fn audio_port_info(&self, index: u32, is_input: bool) -> Option<AudioPortInfo> {
        let channel_count = self.layout.channel_count();
        (index == 0).then_some(if is_input {
            AudioPortInfo {
                id: 1,
                name: "Main In",
                flags: AudioPortFlags {
                    is_main: true,
                    ..AudioPortFlags::default()
                },
                channel_count,
                port_type: audio_port_type(channel_count),
                in_place_pair: None,
            }
        } else {
            AudioPortInfo {
                id: 2,
                name: "Main Out",
                flags: AudioPortFlags {
                    is_main: true,
                    ..AudioPortFlags::default()
                },
                channel_count,
                port_type: audio_port_type(channel_count),
                in_place_pair: None,
            }
        })
    }
}

/// host からの layout 変更要求を [`AudioLayoutStore`] に反映する capability。
///
/// `&self` で更新するのは、adapter が `&mut self` lock を通らずに呼ぶため
/// ([`WracGainPlugin`](super::WracGainPlugin) 参照)。active 中に変えてよい訳ではなく、adapter が
/// Processor 不在 (inactive) のときだけ呼ぶことで安全を保証している。
pub(super) struct WracGainConfigurableAudioPorts {
    layout: Arc<AudioLayoutStore>,
}

impl WracGainConfigurableAudioPorts {
    pub(super) fn new(layout: Arc<AudioLayoutStore>) -> Self {
        Self { layout }
    }
}

// 例: host が stereo→mono を提案 → 受理可否を `can_apply_*` で答え、
// 実反映を `apply_*` で行う。
impl PluginConfigurableAudioPorts for WracGainConfigurableAudioPorts {
    fn can_apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> bool {
        let accepted = resolve_audio_channel_count(self.layout.channel_count(), requests);
        accepted.is_some()
    }

    fn apply_audio_port_configuration(
        &self,
        requests: &[AudioPortConfigurationRequest],
    ) -> PluginResult<()> {
        // adapter 側が Processor の存在中は configuration apply を拒否する。ここは非 RT
        // query 専用 store だけを更新し、audio thread は activate 時の snapshot を使う。
        let previous_channel_count = self.layout.channel_count();
        let channel_count =
            resolve_audio_channel_count(previous_channel_count, requests).ok_or_else(|| {
                log::warn!(
                    "rejecting unsupported audio port configuration: request_count={}, current_channel_count={}",
                    requests.len(),
                    previous_channel_count
                );
                PluginError::InvalidState
            })?;
        log::debug!(
            "applying audio port configuration: previous_channel_count={previous_channel_count}, channel_count={channel_count}"
        );
        self.layout.set_channel_count(channel_count);
        Ok(())
    }
}

fn audio_port_type(channel_count: u32) -> AudioPortType {
    match channel_count {
        1 => AudioPortType::Mono,
        2 => AudioPortType::Stereo,
        _ => AudioPortType::Unspecified,
    }
}

/// port 構成要求を解析し、受理できるなら新しい channel 数を返す。
///
/// 入出力が対称な main port のみ受理する。sidechain のような非対称構成は
/// 製品固有の routing 意味論が必要で、汎用 gain サンプルでは定義できないため。
fn resolve_audio_channel_count(
    current_channel_count: u32,
    requests: &[AudioPortConfigurationRequest],
) -> Option<u32> {
    let mut input_channel_count = current_channel_count;
    let mut output_channel_count = current_channel_count;
    for request in requests {
        if request.port_index != 0 {
            return None;
        }
        if !is_supported_audio_port_request(request) {
            return None;
        }
        if request.is_input {
            input_channel_count = request.channel_count;
        } else {
            output_channel_count = request.channel_count;
        }
    }

    // 入出力で channel 数が一致しているときだけ受理する。
    (input_channel_count == output_channel_count).then_some(input_channel_count)
}

fn is_supported_audio_port_request(request: &AudioPortConfigurationRequest) -> bool {
    matches!(
        (request.channel_count, request.port_type),
        (1, AudioPortType::Mono | AudioPortType::Unspecified)
            | (2, AudioPortType::Stereo | AudioPortType::Unspecified)
    )
}

#[cfg(test)]
mod tests {
    // host や CLAP runtime 無しで検証できる純粋ロジックの単体テスト例。

    use wrac_clap_adapter::{AudioPortConfigurationRequest, AudioPortType};

    use super::resolve_audio_channel_count;

    #[test]
    fn accepts_matching_mono_configuration() {
        let requests = [
            AudioPortConfigurationRequest {
                is_input: true,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
            AudioPortConfigurationRequest {
                is_input: false,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
        ];

        assert_eq!(resolve_audio_channel_count(2, &requests), Some(1));
    }

    #[test]
    fn rejects_mismatched_input_output_configuration() {
        let requests = [
            AudioPortConfigurationRequest {
                is_input: true,
                port_index: 0,
                channel_count: 1,
                port_type: AudioPortType::Mono,
            },
            AudioPortConfigurationRequest {
                is_input: false,
                port_index: 0,
                channel_count: 2,
                port_type: AudioPortType::Stereo,
            },
        ];

        assert_eq!(resolve_audio_channel_count(2, &requests), None);
    }
}
