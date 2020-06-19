use anyhow::{anyhow, Context, Error, Result};
use ghakuf::messages::*;
use ghakuf::reader::*;
use log::*;
use std::collections::{HashMap, VecDeque};
use std::convert::TryInto;
use std::path::Path;
use toio::Note;

pub type EventMap = HashMap<u8, Vec<Event>>;

#[derive(Clone, Debug)]
pub struct Event {
    pub note: Note,
    pub time: u64,
    pub at: u64,
}

#[derive(Clone, Debug)]
pub struct Rule {
    channels: Vec<u8>,
    as_channel: u8,
}

impl Rule {
    fn new(channels: Vec<u8>, as_channel: u8) -> Self {
        Self {
            channels,
            as_channel,
        }
    }
}

impl std::str::FromStr for Rule {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.contains("=") {
            let mut iter = s.splitn(2, "=");
            let as_channel = iter.next().ok_or_else(|| anyhow!("Invalid rule: {}", s))?;
            let channels = iter.next().ok_or_else(|| anyhow!("Invalid rule: {}", s))?;

            let as_channel = as_channel.parse().context(format!("Invalid rule: {}", s))?;
            let channels: Result<Vec<_>> = channels
                .split(",")
                .map(|channel| Ok(channel.parse().context(format!("Invalid rule: {}", s))?))
                .collect();

            Ok(Rule::new(channels?, as_channel))
        } else {
            Err(anyhow!("Invalid rule: {}", s))
        }
    }
}

impl Event {
    fn new(note: Note, time: u64, at: u64) -> Self {
        Self { note, time, at }
    }
}

pub fn list(file: &Path) -> Result<Vec<u8>> {
    let mut handler = MessageHandler::new(1000);
    let mut reader = Reader::new(&mut handler, file).map_err(|e| anyhow!("{}", e))?;
    let _ = reader.read();

    let mut channels: Vec<_> = handler.default().keys().cloned().collect();
    channels.sort();
    Ok(channels)
}

pub fn load(file: &Path, unit: u64, tempo: u64, rules: &[Rule]) -> Result<EventMap> {
    if tempo == 0 {
        return Err(anyhow!("Tempo must be non-zero"));
    }

    let mut handler = MessageHandler::new(tempo);
    let mut reader = Reader::new(&mut handler, file).map_err(|e| anyhow!("{}", e))?;
    let _ = reader.read();

    if rules.is_empty() {
        return Ok(handler.default());
    }

    for rule in rules {
        info!(
            "Assign channels {:?} to cube {}",
            rule.channels, rule.as_channel
        );
        handler.merge(unit, &rule)?;
    }

    Ok(handler.merged().clone())
}

struct MessageHandler {
    tempo: u64,
    map: HashMap<u8, VecDeque<Event>>,
    merged: EventMap,
}

impl MessageHandler {
    fn new(tempo: u64) -> Self {
        Self {
            tempo,
            map: HashMap::new(),
            merged: HashMap::new(),
        }
    }

    fn default(&self) -> EventMap {
        self.map
            .clone()
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect()
    }

    fn merged(&self) -> &EventMap {
        &self.merged
    }

    fn merge(&mut self, unit: u64, rule: &Rule) -> Result<()> {
        let mut merged = vec![];
        let seqs: Result<Vec<_>> = rule
            .channels
            .iter()
            .map(|ch| {
                self.map
                    .get(ch)
                    .cloned()
                    .ok_or_else(|| anyhow!("No such channel: {}", ch))
            })
            .collect();
        let mut seqs = seqs?;
        let mut at = 0;

        let slice = 1;

        for i in 0.. {
            let mut notes = HashMap::new();
            let mut nodata = true;

            trace!("---");

            for (ch, seq) in seqs.iter_mut().enumerate() {
                let e = match seq.front() {
                    Some(e) => e.clone(),
                    None => continue,
                };
                nodata = false;

                trace!("{}-{}: ch={}: {:?}", at, at + slice, ch, e);

                if !(e.at + e.time < at || at + slice <= e.at) && e.note != Note::NoSound {
                    notes.insert(ch, Event::new(e.note, slice, at));
                }

                if e.at + e.time <= at + slice {
                    seq.pop_front();
                }
            }

            if nodata {
                break;
            }

            if notes.is_empty() {
                trace!("* {}-{}: no sound", at, at + slice);
                merged.push(Event::new(Note::NoSound, slice, at))
            } else if notes.len() == 1 {
                let e = notes.values().next().unwrap().clone();
                trace!("* {}-{}: {:?}", at, at + slice, e);
                merged.push(e);
            } else {
                let mut chs: Vec<_> = notes.keys().collect();
                chs.sort();
                let ch = chs[(i / unit as usize) % chs.len()];
                let e = notes.get(ch).unwrap().clone();
                trace!("* {}-{}: {:?} (ch={})", at, at + slice, e, ch);
                merged.push(e);
            }

            notes.clear();

            at += slice;
        }

        let mut squashed: Vec<Event> = vec![];

        for e in merged {
            if let Some(last) = squashed.last_mut() {
                if last.note == e.note {
                    last.time += e.time;
                    continue;
                }
            }
            squashed.push(e);
        }

        self.merged.insert(rule.as_channel, squashed);

        Ok(())
    }
}

impl Handler for MessageHandler {
    fn header(&mut self, _format: u16, _track: u16, time_base: u16) {
        debug!("time_base: {:04x} {}", time_base, time_base);
    }

    fn meta_event(&mut self, _delta_time: u32, _event: &MetaEvent, _data: &Vec<u8>) {}

    fn midi_event(&mut self, delta: u32, event: &MidiEvent) {
        trace!("delta time: {:>4}, MIDI event: {}", delta, event);

        let delta = (delta as u64) * 1000 / self.tempo;

        match event {
            MidiEvent::NoteOn { ch, note, velocity } => {
                let events = self.map.entry(*ch).or_insert_with(|| VecDeque::new());

                if let Some(last) = events.back_mut() {
                    last.time = delta;
                }

                let at = match events.back() {
                    Some(e) => e.at,
                    None => 0,
                } + delta as u64;

                events.push_back(Event::new(
                    if *velocity > 0 {
                        (*note - 12).try_into().unwrap()
                    } else {
                        Note::NoSound
                    },
                    0,
                    at,
                ));
            }
            _ => {}
        }
    }

    fn sys_ex_event(&mut self, _delta_time: u32, _event: &SysExEvent, _data: &Vec<u8>) {}

    fn track_change(&mut self) {}
}
