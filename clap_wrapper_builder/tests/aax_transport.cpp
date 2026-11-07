// Verify the transport seen by CLAP across AAX playback and loop transitions.
#include "detail/aax/wrapper.h"

#include "AAX_Errors.h"
#include "AAX_ITransport.h"

#include <cmath>
#include <cstdio>
#include <vector>

struct Transport : AAX_ITransport
{
  bool playing = false;
  bool looping = false;
  bool sampleAvailable = false;
  bool tickAvailable = true;
  bool meterAvailable = true;
  int32_t numerator = 4, denominator = 4, bars = 4, beats = 1;
  int64_t sample = 12 * 48000, ticks = 12000000, displayTicks = 480;
  int64_t loopStart = 4 * 960000, loopEnd = 16 * 960000;

  AAX_Result GetCurrentTempo(double *v) const override
  {
    *v = 120.0;
    return AAX_SUCCESS;
  }
  AAX_Result GetCurrentMeter(int32_t *n, int32_t *d) const override
  {
    if (!meterAvailable) return AAX_ERROR_UNIMPLEMENTED;
    *n = numerator;
    *d = denominator;
    return AAX_SUCCESS;
  }
  AAX_Result IsTransportPlaying(bool *v) const override
  {
    *v = playing;
    return AAX_SUCCESS;
  }
  AAX_Result GetCurrentTickPosition(int64_t *v) const override
  {
    *v = ticks;
    return AAX_SUCCESS;
  }
  AAX_Result GetCurrentLoopPosition(bool *v, int64_t *start, int64_t *end) const override
  {
    *v = looping;
    *start = loopStart;
    *end = loopEnd;
    return AAX_SUCCESS;
  }
  AAX_Result GetCurrentNativeSampleLocation(int64_t *v) const override
  {
    if (!sampleAvailable) return AAX_ERROR_UNIMPLEMENTED;
    *v = sample;
    return AAX_SUCCESS;
  }
  AAX_Result GetCustomTickPosition(int64_t *v, int64_t position) const override
  {
    if (!tickAvailable || position != sample) return AAX_ERROR_UNIMPLEMENTED;
    *v = ticks;
    return AAX_SUCCESS;
  }
  AAX_Result GetBarBeatPosition(int32_t *bar, int32_t *beat, int64_t *tick,
                                int64_t position) const override
  {
    if (position != sample) return AAX_ERROR_UNIMPLEMENTED;
    *bar = bars;
    *beat = beats;
    *tick = displayTicks;
    return AAX_SUCCESS;
  }
  AAX_Result GetTicksPerQuarter(uint32_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result GetCurrentTicksPerBeat(uint32_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result GetTimelineSelectionStartPosition(int64_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result GetTimeCodeInfo(AAX_EFrameRate *, int32_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result GetFeetFramesInfo(AAX_EFeetFramesRate *, int64_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result IsMetronomeEnabled(int32_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result GetHDTimeCodeInfo(AAX_EFrameRate *, int64_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result RequestTransportStart() override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result RequestTransportStop() override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result GetTimelineSelectionEndPosition(int64_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
  AAX_Result GetKeySignature(int64_t, uint32_t *) const override
  {
    return AAX_ERROR_UNIMPLEMENTED;
  }
};

struct TransportNode : AAX_IMIDINode
{
  Transport transport;
  AAX_CMidiStream *GetNodeBuffer() override
  {
    return nullptr;
  }
  AAX_Result PostMIDIPacket(AAX_CMidiPacket *) override
  {
    return AAX_SUCCESS;
  }
  AAX_ITransport *GetTransport() override
  {
    return &transport;
  }
};

static uint32_t CLAP_ABI audioPortCount(const clap_plugin_t *, bool)
{
  return 0;
}
static clap_process_status CLAP_ABI capture(const clap_plugin_t *plugin, const clap_process_t *process)
{
  *static_cast<clap_event_transport_t *>(plugin->plugin_data) = *process->transport;
  return CLAP_PROCESS_CONTINUE;
}

static bool sameBeat(clap_beattime actual, double expected)
{
  return std::abs((double)actual / CLAP_BEATTIME_FACTOR - expected) < 1e-8;
}

int main()
{
  clap_event_transport_t captured{};
  clap_plugin_descriptor_t descriptor{};
  descriptor.name = "Transport test";
  clap_plugin_t plugin{};
  plugin.desc = &descriptor;
  plugin.plugin_data = &captured;
  plugin.process = capture;
  clap_plugin_audio_ports_t audioPorts{};
  audioPorts.count = audioPortCount;
  ParamChangeQueue queue;
  if (!queue.init(2)) return 1;
  std::vector<clap_id> gestures;
  AAXProcessAdapter adapter;
  adapter.setupProcessing(&plugin, 48000, nullptr, &audioPorts, nullptr, gestures, queue, nullptr,
                          CLAP_INVALID_ID, true, false, 0, 0, 0);
  TransportNode node;
  int32_t frames = 32;
  SAAX_Wrapper_AlgorithmicContext context;
  context.mNumSamples = &frames;
  context.mTransportNode = &node;

  node.transport.playing = true;
  node.transport.looping = true;
  node.transport.sampleAvailable = true;
  adapter.process(&context);
  const uint32_t all = CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_TIME_SIGNATURE |
                       CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | CLAP_TRANSPORT_HAS_BEATS_TIMELINE |
                       CLAP_TRANSPORT_IS_PLAYING | CLAP_TRANSPORT_IS_LOOP_ACTIVE;
  if (captured.flags != all || !sameBeat(captured.song_pos_beats, 12.5) ||
      !sameBeat(captured.bar_start, 12.0) || captured.bar_number != 3 ||
      !sameBeat(captured.loop_start_beats, 4.0) || !sameBeat(captured.loop_end_beats, 16.0) ||
      captured.loop_start_seconds != 0 || captured.loop_end_seconds != 0 ||
      (double)captured.song_pos_seconds / CLAP_SECTIME_FACTOR != 12.0)
  {
    std::fprintf(stderr, "Incorrect playing transport or loop positions\n");
    return 1;
  }

  node.transport.playing = false;
  node.transport.sampleAvailable = false;
  adapter.process(&context);
  if (captured.flags !=
      (CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_TIME_SIGNATURE | CLAP_TRANSPORT_IS_LOOP_ACTIVE))
  {
    std::fprintf(stderr, "Loop state was lost while stopped\n");
    return 1;
  }

  node.transport.looping = false;
  adapter.process(&context);
  if (captured.flags != (CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_HAS_TIME_SIGNATURE) ||
      captured.song_pos_beats != 0 || captured.song_pos_seconds != 0)
  {
    std::fprintf(stderr, "Stopped transport retained state from playback\n");
    return 1;
  }

  // Beat numbers are denominator-based, while AAX display ticks are quarter-note based.
  node.transport.playing = true;
  node.transport.sampleAvailable = true;
  node.transport.numerator = 6;
  node.transport.denominator = 8;
  node.transport.bars = 3;
  node.transport.beats = 2;
  node.transport.ticks = 6480000;
  node.transport.displayTicks = 240;
  adapter.process(&context);
  if (!sameBeat(captured.song_pos_beats, 6.75) || !sameBeat(captured.bar_start, 6.0) ||
      captured.tsig_num != 6 || captured.tsig_denom != 8)
  {
    std::fprintf(stderr, "Incorrect compound-meter position\n");
    return 1;
  }

  node.transport.meterAvailable = false;
  adapter.process(&context);
  if ((captured.flags & (CLAP_TRANSPORT_HAS_TIME_SIGNATURE | CLAP_TRANSPORT_HAS_BEATS_TIMELINE)) != 0)
  {
    std::fprintf(stderr, "Unavailable meter was reported as valid\n");
    return 1;
  }
  context.mTransportNode = nullptr;
  adapter.process(&context);
  if (captured.flags != 0)
  {
    std::fprintf(stderr, "Missing transport retained state from the previous block\n");
    return 1;
  }
  return 0;
}
