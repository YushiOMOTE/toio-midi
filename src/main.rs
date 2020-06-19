mod midi;

use anyhow::{anyhow, Error, Result};
use futures::prelude::*;
use log::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use structopt::StructOpt;
use toio::{Cube, Note, SoundOp};
use tokio::time::delay_for;

use crate::midi::{EventMap, Rule};

#[derive(StructOpt)]
struct Opt {
    /// MIDI file name
    #[structopt(name = "file")]
    file: PathBuf,
    /// Rules to assign channels to cube
    #[structopt(short = "r", long = "rule", parse(try_from_str))]
    rules: Vec<Rule>,
    /// Tempo
    #[structopt(short = "t", long = "tempo", default_value = "1000")]
    tempo: u64,
    /// Time-slice size used on merge
    #[structopt(short = "u", long = "unit", default_value = "40")]
    unit: u64,
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

async fn play(mut inst: Instrument, channel: u8, map: EventMap) {
    let events = map.get(&channel).cloned().unwrap_or_else(|| vec![]);

    for e in events {
        inst.add(e.note, e.time).await;
    }

    inst.play().await;

    info!("Shutdown down in 5 seconds...");
    delay_for(Duration::from_secs(5)).await;
}

#[tokio::main]
async fn main() -> Result<()> {
    let opt = Opt::from_args();

    env_logger::init();

    let events = midi::load(&opt.file, opt.unit, opt.tempo, &opt.rules)?;

    let mut cubes = Cube::search().all().await?;

    if cubes.is_empty() {
        return Err(anyhow!("No cube found"));
    }

    for cube in cubes.iter_mut() {
        cube.connect().await?;
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
        .map(|(channel, mut cube)| {
            let events = events.clone();
            let mut rx = tx.subscribe();

            tokio::spawn(async move {
                let channel = channel as u8;

                info!("Cube {} is ready", channel);

                // Turn on the light.
                cube.light_on(
                    ((channel % 7 + 1) & 1u8) * 255,
                    ((channel % 7 + 1) >> 1u8 & 1u8) * 255,
                    ((channel % 7 + 1) >> 2u8 & 1u8) * 255,
                    None,
                )
                .await?;

                // Fence to start at the same time.
                rx.next().await.ok_or_else(|| anyhow!("Broken barrier"))??;

                let inst = Instrument::new(cube);
                play(inst, channel, events).await;

                Ok::<_, Error>(())
            })
        })
        .collect();

    for res in futures::future::join_all(tracks).await {
        res??;
    }

    info!("Finish");

    Ok(())
}
