// Exercise AAX-to-CLAP MIDI and note conversion, including event size and velocity.
#include "detail/aax/wrapper.h"

#include <array>
#include <cmath>
#include <cstdio>
#include <cstdint>
#include <vector>

struct MidiNode : AAX_IMIDINode
{
  AAX_CMidiStream stream{};
  AAX_CMidiStream *GetNodeBuffer() override
  {
    return &stream;
  }
  AAX_Result PostMIDIPacket(AAX_CMidiPacket *) override
  {
    return AAX_SUCCESS;
  }
  AAX_ITransport *GetTransport() override
  {
    return nullptr;
  }
};

struct CapturedMidi
{
  clap_event_header_t header;
  uint16_t port;
  std::array<uint8_t, 3> data;
};

struct CapturedEvents
{
  std::vector<CapturedMidi> midi;
  std::vector<clap_event_note_t> notes;
};

static uint32_t CLAP_ABI audioPortCount(const clap_plugin_t *, bool)
{
  return 0;
}

static clap_process_status CLAP_ABI captureEvents(const clap_plugin_t *plugin,
                                                  const clap_process_t *process)
{
  auto &events = *static_cast<CapturedEvents *>(plugin->plugin_data);
  for (uint32_t i = 0; i < process->in_events->size(process->in_events); ++i)
  {
    const auto *header = process->in_events->get(process->in_events, i);
    if (header->type == CLAP_EVENT_MIDI)
    {
      const auto *midi = reinterpret_cast<const clap_event_midi_t *>(header);
      events.midi.push_back({*header, midi->port_index, {midi->data[0], midi->data[1], midi->data[2]}});
    }
    else if (header->type == CLAP_EVENT_NOTE_ON || header->type == CLAP_EVENT_NOTE_OFF)
      events.notes.push_back(*reinterpret_cast<const clap_event_note_t *>(header));
  }
  return CLAP_PROCESS_CONTINUE;
}

static bool run(bool preferMidi, bool declared = true)
{
  std::array<AAX_CMidiPacket, 4> packets{{
      {4, 3, {0x92, 64, 111, 0}, false},
      {8, 3, {0x82, 64, 3, 0}, true},
      {10, 3, {0x92, 64, 0, 0}, false},
      {12, 3, {0xB2, 7, 99, 0}, false},
  }};
  MidiNode input;
  input.stream = {static_cast<uint32_t>(packets.size()), packets.data()};
  MidiNode transport;

  CapturedEvents events;
  clap_plugin_descriptor_t descriptor{};
  descriptor.name = "MIDI input test";
  clap_plugin_t plugin{};
  plugin.desc = &descriptor;
  plugin.plugin_data = &events;
  plugin.process = captureEvents;
  clap_plugin_audio_ports_t audioPorts{};
  audioPorts.count = audioPortCount;
  ParamChangeQueue queue;
  if (!queue.init(2)) return false;
  std::vector<clap_id> gestures;
  AAXProcessAdapter adapter;
  adapter.setupProcessing(&plugin, 48000, nullptr, &audioPorts, nullptr, gestures, queue, nullptr,
                          declared ? 0 : CLAP_INVALID_ID, preferMidi, false, 0, 0, 0);
  int32_t frames = 32;
  SAAX_Wrapper_AlgorithmicContext context;
  context.mNumSamples = &frames;
  // An undeclared field is private data, not an AAX MIDI node.
  context.mInputNode = declared ? &input : reinterpret_cast<AAX_IMIDINode *>(std::uintptr_t{0xcdcdcdcd});
  context.mTransportNode = &transport;
  adapter.process(&context);

  if (!declared) return events.midi.empty() && events.notes.empty();

  const size_t firstMidi = preferMidi ? 0 : 3;
  if (events.midi.size() != packets.size() - firstMidi || events.notes.size() != firstMidi)
  {
    std::fprintf(stderr, "Unexpected MIDI/note event counts: %zu/%zu\n", events.midi.size(),
                 events.notes.size());
    return false;
  }
  for (size_t i = 0; i < events.midi.size(); ++i)
  {
    const auto &event = events.midi[i];
    const auto &packet = packets[i + firstMidi];
    if (event.header.size != sizeof(clap_event_midi_t) || event.header.time != packet.mTimestamp ||
        event.header.space_id != CLAP_CORE_EVENT_SPACE_ID ||
        event.header.flags != (packet.mIsImmediate ? CLAP_EVENT_IS_LIVE : 0) || event.port != 0 ||
        event.data != std::array<uint8_t, 3>{packet.mData[0], packet.mData[1], packet.mData[2]})
    {
      std::fprintf(stderr, "Invalid CLAP MIDI event %zu: size=%u port=%u\n", i, event.header.size,
                   event.port);
      return false;
    }
  }
  for (size_t i = 0; i < events.notes.size(); ++i)
  {
    const auto &note = events.notes[i];
    const auto &packet = packets[i];
    if (note.header.type != (i == 0 ? CLAP_EVENT_NOTE_ON : CLAP_EVENT_NOTE_OFF) ||
        note.header.size != sizeof(clap_event_note_t) || note.header.time != packet.mTimestamp ||
        note.header.flags != (packet.mIsImmediate ? CLAP_EVENT_IS_LIVE : 0) || note.port_index != 0 ||
        note.channel != 2 || note.key != 64 || note.note_id != -1 ||
        std::abs(note.velocity - packet.mData[2] / 127.0) > 1e-12)
    {
      std::fprintf(stderr, "Invalid CLAP note %zu: port=%d velocity=%f\n", i, note.port_index,
                   note.velocity);
      return false;
    }
  }
  return true;
}

int main()
{
  return run(true) && run(false) && run(true, false) ? 0 : 1;
}
