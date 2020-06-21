use anyhow::{anyhow, Result};
use derive_new::new;
use ghakuf::{messages::*, reader::*};
use log::*;
use std::{
    collections::{BTreeMap, HashMap},
    convert::TryInto,
    path::Path,
};
use toio::Note;

pub type EventMap = BTreeMap<(Time, Channel), Play>;
pub type Channel = u8;
pub type Time = u64;

#[derive(Clone, Debug, PartialEq, Eq, new)]
pub struct Play {
    pub ch: Channel,
    pub at: Time,
    pub len: Time,
    pub note: Note,
}

#[derive(Clone, Debug, PartialEq, Eq, new)]
pub struct PlaySet {
    pub ch: Channel,
    pub at: Time,
    #[new(default)]
    pub len: Time,
    #[new(default)]
    pub plays: Vec<Play>,
}

#[derive(Clone, Debug, PartialEq, Eq, new)]
pub enum Event {
    Start(Start),
    Stop(Stop),
    Tempo(Tempo),
}

#[derive(Clone, Debug, PartialEq, Eq, new)]
pub struct Start {
    ch: Channel,
    note: Note,
}

#[derive(Clone, Debug, PartialEq, Eq, new)]
pub struct Stop {
    ch: Channel,
}

#[derive(Clone, Debug, PartialEq, Eq, new)]
pub struct Tempo {
    ch: Channel,
    tempo: u64,
}

#[derive(Clone, Debug, Default, new)]
struct Raw {
    #[new(default)]
    at: Time,
    #[new(default)]
    notes: HashMap<Note, Time>,
    #[new(default)]
    events: BTreeMap<(Time, Channel), Event>,
}

impl Raw {
    fn update(&mut self, delta: Time) {
        self.at += delta;
    }

    fn on(&mut self, ch: Channel, delta: Time, note: Note) {
        self.update(delta);
        self.onoff(ch, note, true);
    }

    fn off(&mut self, ch: Channel, delta: Time, note: Note) {
        self.update(delta);
        self.onoff(ch, note, false);
    }

    fn tempo(&mut self, ch: Channel, delta: Time, tempo: u64) {
        self.update(delta);
        self.events
            .insert((self.at, ch), Event::Tempo(Tempo::new(ch, tempo)));
    }

    fn end(&mut self, ch: Channel) {
        if !self.notes.is_empty() {
            self.events
                .insert((self.at, ch), Event::Stop(Stop::new(ch)));
        }
        self.notes.clear();
        self.at = 0;
    }

    fn onoff(&mut self, ch: Channel, note: Note, on: bool) {
        let old = self.note();
        if on {
            self.notes.insert(note, self.at);
        } else {
            self.notes.remove(&note);
        }
        let new = self.note();

        if old != new {
            if let Some(_) = old {
                self.events
                    .insert((self.at, ch), Event::Stop(Stop::new(ch)));
            }
            if let Some(note) = new {
                self.events
                    .insert((self.at, ch), Event::Start(Start::new(ch, note)));
            }
        }
    }

    fn note(&self) -> Option<Note> {
        self.notes
            .iter()
            .min_by(|p, q| p.1.cmp(q.1))
            .map(|(k, _)| *k)
    }

    fn tempoed(&self, time_base: u64) -> Tempoed {
        let mut tempo = 500000;
        let mut events = BTreeMap::new();
        let mut old_tempo_at = 0;
        let mut new_tempo_at = 0;
        let mut new_at = 0;
        let mut notes = HashMap::new();

        for ((at, _), event) in &self.events {
            new_at = ((at - old_tempo_at) * tempo / 1000 / time_base + new_tempo_at) / 10 * 10;

            match event {
                Event::Start(s) => {
                    if let Some((start_at, note)) = notes.remove(&s.ch) {
                        events.insert(
                            (start_at, s.ch),
                            Play::new(s.ch, start_at, new_at - start_at, note),
                        );
                    }
                    notes.insert(s.ch, (new_at, s.note));
                }
                Event::Stop(s) => {
                    if let Some((start_at, note)) = notes.remove(&s.ch) {
                        events.insert(
                            (start_at, s.ch),
                            Play::new(s.ch, start_at, new_at - start_at, note),
                        );
                    }
                }
                Event::Tempo(t) => {
                    old_tempo_at = *at;
                    new_tempo_at = new_at;
                    tempo = t.tempo;
                }
            }
        }

        for (ch, (start_at, note)) in notes {
            events.insert(
                (start_at, ch),
                Play::new(ch, start_at, new_at - start_at, note),
            );
        }

        Tempoed(events)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, new)]
struct Tempoed(EventMap);

fn mix(mixed: &mut EventMap, orig: &EventMap, unit: u64, as_ch: u8, chs: &[u8]) {
    if chs.len() == 1 {
        for ((at, ch), play) in orig {
            if chs.contains(ch) {
                let mut p = play.clone();
                p.ch = as_ch;
                mixed.insert((*at, as_ch), p);
            }
        }
        return;
    }

    let mut on: Vec<Play> = vec![];
    let mut last = None::<Play>;
    let mut iter = orig.iter().peekable();

    for at in 0.. {
        if !on.is_empty() {
            let mut play = on[(at / unit) as usize % on.len()].clone();
            play.at = at;
            play.len = 1;

            if let Some(mut l) = last.take() {
                // If the item is same as the previous one, merge.
                if l.ch == play.ch && l.at + l.len == play.at && l.note == play.note {
                    l.len += 1;
                    last = Some(l);
                } else {
                    l.ch = as_ch;
                    mixed.insert((l.at, as_ch), l);
                    last = Some(play);
                }
            } else {
                last = Some(play);
            }
        }

        on.retain(|play| at < play.at + play.len);

        let play = match iter.peek() {
            Some(((play_at, ch), play)) if *play_at <= at => {
                if !chs.contains(ch) {
                    iter.next();
                    continue;
                } else {
                    play
                }
            }
            Some(_) => continue,
            None => break,
        };

        on.push((*play).clone());

        iter.next();
    }

    if let Some(mut l) = last.take() {
        l.ch = as_ch;
        mixed.insert((l.at, as_ch), l);
    }
}

impl Tempoed {
    fn mixed(&self, unit: u64, rules: &[(u8, Vec<u8>)]) -> Tempoed {
        let mut mixed = BTreeMap::new();

        for (as_ch, chs) in rules {
            mix(&mut mixed, &self.0, unit, *as_ch, &chs);
        }

        Tempoed(mixed)
    }

    fn merged(&self, size: usize, maxlen: Time) -> Merged {
        let mut merged = BTreeMap::new();
        let mut chs = HashMap::new();

        for ((at, _), play) in &self.0 {
            let mut play = play.clone();
            let mut rem = play.len;

            while rem > 0 {
                play.len = rem.min(maxlen);

                // Operations on gaps
                {
                    enum Op {
                        Flush,
                        Fill(Time, Time),
                        None,
                    }

                    let set = chs
                        .entry(play.ch)
                        .or_insert_with(|| PlaySet::new(play.ch, *at));

                    let op = if let Some(last) = set.plays.last() {
                        if last.at + last.len + maxlen < play.at {
                            // Gap is longer than maxlen, flush.
                            Op::Flush
                        } else if last.at + last.len < play.at {
                            // Gap is shorter than maxlen, fill.
                            Op::Fill(last.at + last.len, play.at - (last.at + last.len))
                        } else {
                            // No gap.
                            Op::None
                        }
                    } else {
                        // No last.
                        Op::None
                    };

                    match op {
                        Op::Flush => {
                            merged.insert((set.at, set.ch), set.clone());

                            let ch = set.ch;
                            chs.remove(&ch);
                        }
                        Op::Fill(at, len) => {
                            set.plays.push(Play::new(play.ch, at, len, Note::NoSound));
                            if set.plays.len() == size {
                                merged.insert((set.at, set.ch), set.clone());

                                let ch = set.ch;
                                chs.remove(&ch);
                            }
                        }
                        Op::None => {}
                    }
                }

                let set = chs
                    .entry(play.ch)
                    .or_insert_with(|| PlaySet::new(play.ch, *at));
                set.len += play.len;
                set.plays.push(play.clone());
                if set.plays.len() == size {
                    merged.insert((set.at, set.ch), set.clone());

                    let ch = set.ch;
                    chs.remove(&ch);
                }

                play.at += play.len;
                rem -= play.len;
            }
        }

        for (_, set) in chs {
            merged.insert((set.at, set.ch), set.clone());
        }

        Merged(merged)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, new)]
pub struct Merged(BTreeMap<(Time, Channel), PlaySet>);

#[derive(Clone, Debug, new)]
struct Processor {
    #[new(default)]
    time_base: u64,
    #[new(default)]
    ch: u8,
    #[new(default)]
    raw: Raw,
}

impl Processor {
    fn finalize(&self, size: usize, maxlen: Time) -> Merged {
        self.raw.tempoed(self.time_base).merged(size, maxlen)
    }

    fn finalize_mixed(
        &self,
        size: usize,
        maxlen: Time,
        unit: u64,
        rules: &[(u8, Vec<u8>)],
    ) -> Merged {
        self.raw
            .tempoed(self.time_base)
            .mixed(unit, rules)
            .merged(size, maxlen)
    }
}

impl Handler for Processor {
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
        debug!(
            "{}: delta time: {:>4}, meta event: {}",
            self.ch, delta, event
        );

        match event {
            MetaEvent::SetTempo => {
                let mut tempo = 0u64;
                for d in data {
                    tempo *= 256;
                    tempo += *d as u64;
                }
                self.raw.tempo(self.ch, delta as u64, tempo);
            }
            _ => {
                self.raw.update(delta as u64);
            }
        }
    }

    fn midi_event(&mut self, delta: u32, event: &MidiEvent) {
        debug!(
            "{}: delta time: {:>4}, MIDI event: {}",
            self.ch, delta, event
        );

        match event {
            MidiEvent::NoteOn {
                ch: _,
                note,
                velocity,
            } => {
                if *velocity > 0 {
                    self.raw
                        .on(self.ch, delta as u64, (*note - 12).try_into().unwrap());
                } else {
                    self.raw
                        .off(self.ch, delta as u64, (*note - 12).try_into().unwrap());
                }
            }
            MidiEvent::NoteOff {
                ch: _,
                note,
                velocity: _,
            } => {
                self.raw
                    .off(self.ch, delta as u64, (*note - 12).try_into().unwrap());
            }
            _ => {
                self.raw.update(delta as u64);
            }
        }
    }

    fn sys_ex_event(&mut self, delta: u32, _event: &SysExEvent, _data: &Vec<u8>) {
        debug!("{}: ex event: {:>4}", self.ch, delta);
        self.raw.update(delta as u64);
    }

    fn track_change(&mut self) {
        self.raw.end(self.ch);
        self.ch += 1;
    }
}

fn proc<P: AsRef<Path>>(p: P) -> Result<Processor> {
    let mut proc = Processor::new();
    let mut reader = Reader::new(&mut proc, p.as_ref()).map_err(|e| anyhow!("{}", e))?;
    let _ = reader.read();
    Ok(proc)
}

pub fn load<P: AsRef<Path>>(p: P) -> Result<BTreeMap<(Time, Channel), PlaySet>> {
    Ok(proc(p)?.finalize(59, 2550).0)
}

pub fn load_mixed<P: AsRef<Path>>(
    p: P,
    unit: u64,
    rules: &[(u8, Vec<u8>)],
) -> Result<BTreeMap<(Time, Channel), PlaySet>> {
    Ok(proc(p)?.finalize_mixed(59, 2550, unit, rules).0)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn raw() {
        let mut r = Raw::new();
        r.on(0, 100, Note::C3);
        r.on(0, 200, Note::D3);
        r.on(0, 100, Note::E3);
        r.off(0, 300, Note::C3);
        r.off(0, 0, Note::D3);
        r.off(0, 0, Note::E3);

        let es: Vec<_> = r.events.into_iter().map(|((at, _), v)| (at, v)).collect();
        assert_eq!(
            es,
            vec![
                (100u64, Event::Start(Start::new(0, Note::C3))),
                (300u64, Event::Start(Start::new(0, Note::D3))),
                (400u64, Event::Start(Start::new(0, Note::E3))),
                (700u64, Event::Stop(Stop::new(0)))
            ]
        );
    }

    #[test]
    fn tempoed() {
        let mut r = Raw::new();
        r.tempo(0, 0, 500000);
        r.on(0, 100, Note::C3);
        r.on(0, 200, Note::D3);
        r.on(0, 100, Note::E3);
        r.off(0, 300, Note::C3);
        r.off(0, 0, Note::D3);
        r.off(0, 0, Note::E3);

        // 500msec / 100 = 5msec <=> 1
        let t = r.tempoed(100);

        let es: Vec<_> = t.0.into_iter().map(|((at, _), v)| (at, v)).collect();
        assert_eq!(
            es,
            vec![
                (500u64, Play::new(0, 500, 1000, Note::C3)),
                (1500u64, Play::new(0, 1500, 500, Note::D3)),
                (2000u64, Play::new(0, 2000, 1500, Note::E3)),
            ]
        );
    }

    fn p(ch: Channel, at: Time, len: Time, plays: Vec<Play>) -> PlaySet {
        PlaySet { ch, at, len, plays }
    }

    #[test]
    fn merged() {
        let mut r = Raw::new();

        r.tempo(0, 0, 500000);

        for i in 0..3 {
            r.on(i, 100, Note::C3);
            r.on(i, 200, Note::D3);
            r.on(i, 100, Note::E3);
            r.off(i, 300, Note::C3);
            r.off(i, 0, Note::D3);
            r.off(i, 0, Note::E3);
        }

        // 1 = 5msec
        // Max is large enough
        let t = r.tempoed(100).merged(1000, 2500);

        let es: Vec<_> = t.0.into_iter().map(|((at, _), v)| (at, v)).collect();
        assert_eq!(
            es,
            (0..3)
                .map(|i| (
                    500,
                    p(
                        i,
                        500,
                        3000,
                        vec![
                            Play::new(i, 500, 1000, Note::C3),
                            Play::new(i, 1500, 500, Note::D3),
                            Play::new(i, 2000, 1500, Note::E3)
                        ]
                    )
                ))
                .collect::<Vec<_>>()
        );
    }
}
