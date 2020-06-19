use anyhow::{anyhow, Context, Error, Result};
use ghakuf::messages::*;
use ghakuf::reader::*;
use log::*;
use std::collections::{BTreeMap, HashMap, VecDeque};
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

impl Event {
    fn new(note: Note, time: u64, at: u64) -> Self {
        Self { note, time, at }
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    tracks: Vec<u8>,
    as_track: u8,
}

impl Rule {
    fn new(tracks: Vec<u8>, as_track: u8) -> Self {
        Self { tracks, as_track }
    }
}

impl std::str::FromStr for Rule {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.contains("=") {
            let mut iter = s.splitn(2, "=");
            let as_track = iter.next().ok_or_else(|| anyhow!("Invalid rule: {}", s))?;
            let tracks = iter.next().ok_or_else(|| anyhow!("Invalid rule: {}", s))?;

            let as_track = as_track.parse().context(format!("Invalid rule: {}", s))?;
            let tracks: Result<Vec<_>> = tracks
                .split(",")
                .map(|track| Ok(track.parse().context(format!("Invalid rule: {}", s))?))
                .collect();

            Ok(Rule::new(tracks?, as_track))
        } else {
            Err(anyhow!("Invalid rule: {}", s))
        }
    }
}

pub fn list(file: &Path) -> Result<Vec<u8>> {
    let mut handler = MessageHandler::new();
    let mut reader = Reader::new(&mut handler, file).map_err(|e| anyhow!("{}", e))?;
    let _ = reader.read();
    handler.adjust();

    let mut tracks: Vec<_> = handler.default().keys().cloned().collect();
    tracks.sort();
    Ok(tracks)
}

pub fn load(file: &Path, unit: u64, rules: &[Rule]) -> Result<EventMap> {
    let mut handler = MessageHandler::new();
    let mut reader = Reader::new(&mut handler, file).map_err(|e| anyhow!("{}", e))?;
    let _ = reader.read();
    handler.adjust();

    if rules.is_empty() {
        info!("Use default track assignment rule");
        return Ok(handler.default().clone());
    }

    for rule in rules {
        info!("Assign tracks {:?} to cube {}", rule.tracks, rule.as_track);
        handler.merge(unit, &rule)?;
    }

    Ok(handler.merged().clone())
}

#[derive(Clone)]
struct Track {
    id: u8,
    at: u64,
    events: VecDeque<Event>,
    tempo: BTreeMap<u64, u64>,
}

impl Track {
    fn new(id: u8) -> Self {
        Self {
            id,
            at: 0,
            events: VecDeque::new(),
            tempo: BTreeMap::new(),
        }
    }

    fn delta(&mut self, delta: u32) {
        let delta = delta as u64;
        self.at += delta;

        if let Some(last) = self.events.back_mut() {
            last.time = delta;
        }
    }

    fn void(&mut self, delta: u32) {
        self.delta(delta);
    }

    fn tempo(&mut self, delta: u32, data: &[u8]) {
        self.delta(delta);

        let mut tempo = 0;
        for d in data {
            tempo *= 256;
            tempo += *d as u64;
        }

        self.tempo.insert(self.at, tempo);
    }

    fn note(&mut self, delta: u32, note: u8, vel: u8) {
        self.delta(delta);

        self.events.push_back(Event::new(
            if vel > 0 {
                (note - 12).try_into().unwrap()
            } else {
                Note::NoSound
            },
            0,
            self.at,
        ));
    }

    fn adjust(&mut self, tempo: &BTreeMap<u64, u64>, time_base: u64) {
        let mut new_at = 0;
        self.events.iter_mut().for_each(|e| {
            let mut s = e.at;

            let (_, ctempo) = tempo.range(0..=s).last().expect("Unknown tempo");
            let mut ctempo = *ctempo;
            let mut t = tempo
                .range(e.at..(e.at + e.time))
                .fold(0, move |t, (at, tempo)| {
                    let tt = (at - s) * ctempo / 1000 / time_base;
                    s = *at;
                    ctempo = *tempo;
                    t + tt
                });
            t += (e.at + e.time - s) * ctempo / 1000 / time_base;

            e.time = t;
            e.at = new_at;
            new_at += t;
        });
    }
}

struct MessageHandler {
    track: u8,
    tracks: HashMap<u8, Track>,
    time_base: u64,
    raw: EventMap,
    merged: EventMap,
}

impl MessageHandler {
    fn new() -> Self {
        Self {
            track: 0,
            tracks: HashMap::new(),
            time_base: 0,
            raw: HashMap::new(),
            merged: HashMap::new(),
        }
    }

    fn default(&self) -> &EventMap {
        &self.raw
    }

    fn merged(&self) -> &EventMap {
        &self.merged
    }

    /// Adjust events timing based on tempo
    fn adjust(&mut self) {
        let tempo: BTreeMap<_, _> = self
            .tracks
            .iter()
            .map(|(_, track)| track.tempo.iter().map(|(k, v)| (k.clone(), v.clone())))
            .flatten()
            .collect();

        for (_, track) in &mut self.tracks {
            track.adjust(&tempo, self.time_base);
        }

        self.raw = self
            .tracks
            .clone()
            .into_iter()
            .map(|(k, v)| (k, v.events.into_iter().collect()))
            .collect()
    }

    /// Merge some tracks into one track
    fn merge(&mut self, unit: u64, rule: &Rule) -> Result<()> {
        let mut merged = vec![];
        let tracks: Result<Vec<_>> = rule
            .tracks
            .iter()
            .map(|t| {
                self.tracks
                    .get(t)
                    .cloned()
                    .ok_or_else(|| anyhow!("No such track: {}", t))
            })
            .collect();
        let mut tracks = tracks?;
        let mut at = 0;

        let slice = 1;

        for i in 0.. {
            let mut notes = HashMap::new();
            let mut nodata = true;

            trace!("---");

            for (ch, track) in tracks.iter_mut().enumerate() {
                let e = match track.events.front() {
                    Some(e) => e.clone(),
                    None => continue,
                };
                nodata = false;

                trace!("{}-{}: ch={}: {:?}", at, at + slice, ch, e);

                if !(e.at + e.time < at || at + slice <= e.at) && e.note != Note::NoSound {
                    notes.insert(ch, Event::new(e.note, slice, at));
                }

                if e.at + e.time <= at + slice {
                    track.events.pop_front();
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

        self.merged.insert(rule.as_track, squashed);

        Ok(())
    }

    fn track(&mut self) -> &mut Track {
        let track = self.track;
        self.tracks
            .entry(self.track)
            .or_insert_with(|| Track::new(track))
    }
}

impl Handler for MessageHandler {
    fn header(&mut self, _format: u16, _track: u16, time_base: u16) {
        debug!("time_base: {:04x} {}", time_base, time_base);
        if time_base & 0x8000 > 0 {
            warn!("Unsupported time base");
            self.time_base = 480;
        } else {
            self.time_base = time_base as u64;
        }
    }

    fn meta_event(&mut self, delta: u32, event: &MetaEvent, data: &Vec<u8>) {
        trace!("delta time: {:>4}, meta event: {}", delta, event);

        match event {
            MetaEvent::SetTempo => {
                self.track().tempo(delta, &data);
            }
            _ => {
                self.track().void(delta);
            }
        }
    }

    fn midi_event(&mut self, delta: u32, event: &MidiEvent) {
        trace!("delta time: {:>4}, MIDI event: {}", delta, event);

        match event {
            MidiEvent::NoteOn {
                ch: _,
                note,
                velocity,
            } => {
                self.track().note(delta, *note, *velocity);
            }
            MidiEvent::NoteOff {
                ch: _,
                note: _,
                velocity: _,
            } => {
                self.track().note(delta, 0, 0);
            }
            _ => {
                self.track().void(delta);
            }
        }
    }

    fn sys_ex_event(&mut self, _delta_time: u32, _event: &SysExEvent, _data: &Vec<u8>) {}

    fn track_change(&mut self) {
        self.track += 1;
    }
}
