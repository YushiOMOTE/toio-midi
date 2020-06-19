mod midi;

use anyhow::{anyhow, Error, Result};
use log::*;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use structopt::StructOpt;
use toio::{Cube, Note, SoundOp};
use tokio::{sync::Barrier, time::delay_for};

use crate::midi::{EventMap, Rule};

#[derive(StructOpt)]
struct Opt {
    /// MIDI file name
    #[structopt(name = "file")]
    file: PathBuf,
    /// List tracks
    #[structopt(short = "l", long = "list")]
    list: bool,
    /// Rules to assign tracks to cube
    #[structopt(short = "r", long = "rule", parse(try_from_str))]
    rules: Vec<Rule>,
    /// Speed
    #[structopt(short = "s", long = "speed", default_value = "100")]
    speed: u64,
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

    async fn add(&mut self, note: Note, msec: u64) {
        let mut msec = msec / 10 * 10;

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

async fn play(inst: &mut Instrument, track: u8, map: EventMap, speed: u64) {
    let events = map.get(&track).cloned().unwrap_or_else(|| vec![]);

    for e in events {
        inst.add(e.note, e.time * 100 / speed).await;
    }

    inst.play().await;
}

#[tokio::main]
async fn main() -> Result<()> {
    let opt = Opt::from_args();

    env_logger::from_env(
        env_logger::Env::default().default_filter_or(format!("{}=info", module_path!())),
    )
    .init();

    if opt.list {
        let tracks = midi::list(&opt.file)?;
        info!("Available tracks: {:?}", tracks);
        return Ok(());
    }

    if opt.speed == 0 {
        return Err(anyhow!("Speed must be non-zero"));
    }

    let events = midi::load(&opt.file, opt.unit, &opt.rules)?;

    let mut cubes = Cube::search().all().await?;

    if cubes.is_empty() {
        return Err(anyhow!("No cube found"));
    }

    for cube in cubes.iter_mut() {
        cube.connect().await?;
    }

    let begin = Arc::new(Barrier::new(cubes.len() + 1));
    let end = Arc::new(Barrier::new(cubes.len()));

    let begin0 = begin.clone();
    tokio::spawn(async move {
        info!("Starts playing in 3 seconds...");
        delay_for(Duration::from_secs(3)).await;
        begin0.wait().await;
    });

    let tracks: Vec<_> = cubes
        .into_iter()
        .enumerate()
        .map(|(track, mut cube)| {
            let events = events.clone();
            let begin = begin.clone();
            let end = end.clone();
            let speed = opt.speed;

            tokio::spawn(async move {
                let track = track as u8;

                info!("Cube {} is ready", track);

                // Turn on the light.
                cube.light_on(
                    ((track % 7 + 1) & 1u8) * 255,
                    ((track % 7 + 1) >> 1u8 & 1u8) * 255,
                    ((track % 7 + 1) >> 2u8 & 1u8) * 255,
                    None,
                )
                .await?;

                begin.wait().await;

                let mut inst = Instrument::new(cube);
                play(&mut inst, track, events, speed).await;

                info!("Cube {} finishes playing", track);

                if end.wait().await.is_leader() {
                    info!("Shutdown down in 5 seconds...");
                }

                delay_for(Duration::from_secs(5)).await;

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
