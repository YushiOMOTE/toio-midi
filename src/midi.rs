use ghakuf::messages::*;
use ghakuf::reader::*;
use log::*;
use std::path::Path;

pub fn load(file: &Path) -> Vec<Message> {
    let mut read_messages: Vec<Message> = Vec::new();

    let mut handler = HogeHandler {
        messages: &mut read_messages,
    };
    let mut reader = Reader::new(&mut handler, file).unwrap();
    let _ = reader.read();

    handler.messages.clone()
}

struct HogeHandler<'a> {
    messages: &'a mut Vec<Message>,
}

impl<'a> Handler for HogeHandler<'a> {
    fn header(&mut self, _format: u16, _track: u16, _time_base: u16) {}

    fn meta_event(&mut self, delta_time: u32, event: &MetaEvent, data: &Vec<u8>) {
        self.messages.push(Message::MetaEvent {
            delta_time: delta_time,
            event: event.clone(),
            data: data.clone(),
        });
    }

    fn midi_event(&mut self, delta_time: u32, event: &MidiEvent) {
        trace!("delta time: {:>4}, MIDI event: {}", delta_time, event);
        self.messages.push(Message::MidiEvent {
            delta_time: delta_time,
            event: event.clone(),
        });
    }

    fn sys_ex_event(&mut self, delta_time: u32, event: &SysExEvent, data: &Vec<u8>) {
        self.messages.push(Message::SysExEvent {
            delta_time: delta_time,
            event: event.clone(),
            data: data.clone(),
        });
    }

    fn track_change(&mut self) {}
}
