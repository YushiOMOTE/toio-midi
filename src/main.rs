mod midi;

use futures::prelude::*;
use ghakuf::messages::{Message, MidiEvent};
use log::*;
use std::{
    convert::TryInto,
    path::PathBuf,
    time::{Duration, Instant},
};
use structopt::StructOpt;
use toio::{Cube, Note, SoundOp};
use tokio::time::delay_for;

#[derive(StructOpt)]
struct Opt {
    /// MIDI file name
    #[structopt(name = "file")]
    file: PathBuf,
    /// Channels to assign to cubes
    #[structopt(short = "c", long = "channel")]
    channel: Vec<u8>,
}

struct Instrument {
    cube: Cube,
    ops: Vec<SoundOp>,
    inst: Instant,
}

impl Instrument {
    fn new(cube: Cube) -> Self {
        Self {
            cube,
            ops: vec![],
            inst: Instant::now(),
        }
    }

    async fn add(&mut self, note: Note, mut msec: u64) {
        while msec > 0 {
            let d = msec.min(2550);

            let op = SoundOp::new(note, Duration::from_millis(d));
            debug!("add {:?}", op);
            self.ops.push(op);
            if self.ops.len() == 59 {
                self.play().await;
            }

            msec -= d;
        }
    }

    async fn play(&mut self) {
        let ops = self.ops.split_off(0);

        if ops.is_empty() {
            return;
        }

        let d = ops
            .iter()
            .fold(0u64, |s, op| s + op.duration.as_millis() as u64);

        debug!(
            "{}: play {:?} (len={}, delay={})",
            self.inst.elapsed().as_millis(),
            ops,
            ops.len(),
            d
        );
        let play = self.cube.play(1, ops);
        let delay = delay_for(Duration::from_millis(d));

        let (p, _) = futures::join!(play, delay);
        p.unwrap();

        debug!("{}", self.inst.elapsed().as_millis());
    }
}

#[derive(Debug)]
struct Sound {
    time: u64,
    note: Note,
}

async fn play(mut inst: Instrument, channel: u8, messages: Vec<Message>) {
    let iter = messages.iter().peekable();

    let convert = |e: &Message| match e {
        Message::MidiEvent { delta_time, event } => match &event {
            MidiEvent::NoteOn { ch, note, velocity } if *ch == channel => Some(Sound {
                time: ((*delta_time as u64) * 3 / 5) / 10 * 10,
                note: if *velocity > 0 {
                    (*note).try_into().unwrap()
                } else {
                    Note::NoSound
                },
            }),
            _ => None,
        },
        _ => None,
    };

    let mut iter = iter.filter_map(convert).peekable();

    if let Some(e0) = iter.peek() {
        delay_for(Duration::from_millis(e0.time)).await;
    }

    while let Some(e1) = iter.next() {
        let e2 = if let Some(e2) = iter.peek() {
            e2
        } else {
            break;
        };

        inst.add(e1.note, e2.time).await;
    }

    inst.play().await;

    info!("Shutdown down in 5 seconds...");
    delay_for(Duration::from_secs(5)).await;
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();

    env_logger::init();

    let messages = midi::load(&opt.file);

    let mut cubes = Cube::search().all().await.unwrap();

    for cube in cubes.iter_mut() {
        cube.connect().await.unwrap();
    }

    let (tx, _) = tokio::sync::broadcast::channel(16);

    info!("Starts in 3 seconds");

    tokio::spawn({
        let tx = tx.clone();
        async move {
            delay_for(Duration::from_secs(3)).await;
            let _ = tx.send(());
        }
    });

    let tracks: Vec<_> = cubes
        .into_iter()
        .enumerate()
        .map(|(i, mut cube)| {
            let messages = messages.clone();
            let mut rx = tx.subscribe();
            let channel = opt.channel.clone();

            tokio::spawn(async move {
                let track = channel.get(i).cloned().unwrap_or(i as u8);

                info!("Start playing track: {}", track);

                // Turn on the light.
                cube.light_on(
                    ((track % 7 + 1) & 1u8) * 255,
                    ((track % 7 + 1) >> 1u8 & 1u8) * 255,
                    ((track % 7 + 1) >> 2u8 & 1u8) * 255,
                    None,
                )
                .await
                .unwrap();

                // Fence to start at the same time.
                rx.next().await.unwrap().unwrap();

                let inst = Instrument::new(cube);
                play(inst, track, messages).await;
            })
        })
        .collect();

    let _ = futures::future::select_all(tracks).await;

    info!("Finish");
}
