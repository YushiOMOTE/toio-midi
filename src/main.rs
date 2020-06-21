mod midi;

use anyhow::{anyhow, Context, Error, Result};
use futures::prelude::*;
use log::*;
use std::path::PathBuf;
use structopt::StructOpt;
use toio::{Cube, SoundOp};
use tokio::time::{delay_for, delay_until, Duration, Instant};

use crate::midi::PlaySet;

#[derive(Clone, Debug)]
pub struct Rule {
    chs: Vec<u8>,
    as_ch: u8,
}

impl Rule {
    fn new(chs: Vec<u8>, as_ch: u8) -> Self {
        Self { chs, as_ch }
    }
}

impl std::str::FromStr for Rule {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.contains("=") {
            let mut iter = s.splitn(2, "=");
            let as_ch = iter.next().ok_or_else(|| anyhow!("Invalid rule: {}", s))?;
            let chs = iter.next().ok_or_else(|| anyhow!("Invalid rule: {}", s))?;

            let as_ch = as_ch.parse().context(format!("Invalid rule: {}", s))?;
            let chs: Result<Vec<_>> = chs
                .split(",")
                .map(|ch| Ok(ch.parse().context(format!("Invalid rule: {}", s))?))
                .collect();

            Ok(Rule::new(chs?, as_ch))
        } else {
            Err(anyhow!("Invalid rule: {}", s))
        }
    }
}

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

fn ops(set: &PlaySet) -> Vec<SoundOp> {
    assert!(set.plays.len() <= 59);
    set.plays
        .iter()
        .map(|p| {
            assert!(p.len <= 2550);
            SoundOp::new(p.note, Duration::from_millis(p.len))
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let opt = Opt::from_args();

    env_logger::from_env(
        env_logger::Env::default().default_filter_or(format!("{}=info", module_path!())),
    )
    .init();

    if opt.speed == 0 {
        return Err(anyhow!("Speed must be non-zero"));
    }

    if opt.list {
        let events = midi::load(&opt.file)?;

        let mut set = vec![];
        for ((_, ch), _) in events {
            set.push(ch);
        }
        set.sort();
        set.dedup();
        info!("Available tracks: {:?}", set);
        return Ok(());
    }

    let events = if opt.rules.is_empty() {
        midi::load(&opt.file)?
    } else {
        info!("Parsing file {}...", opt.file.display());
        let rules: Vec<_> = opt.rules.iter().map(|r| (r.as_ch, r.chs.clone())).collect();
        midi::load_mixed(&opt.file, opt.unit, &rules)?
    };

    let mut cubes = Cube::search().all().await?;

    if cubes.is_empty() {
        return Err(anyhow!("No cube found"));
    }

    for (i, cube) in cubes.iter_mut().enumerate() {
        cube.connect().await?;
        info!("Cube {} connected", i);

        let p = opt
            .rules
            .iter()
            .find(|p| p.as_ch == i as u8)
            .map(|r| r.chs.iter().sum())
            .unwrap_or(i as u8);
        cube.light_on(
            ((p % 7 + 1) & 1u8) * 255,
            ((p % 7 + 1) >> 1u8 & 1u8) * 255,
            ((p % 7 + 1) >> 2u8 & 1u8) * 255,
            None,
        )
        .await?;
    }

    let cubes: Vec<_> = cubes
        .into_iter()
        .enumerate()
        .map(|(i, mut cube)| {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Some(p) = rx.next().await {
                    cube.play(1, ops(&p))
                        .await
                        .context(format!("error on cube {}", i))?;
                }
                Ok::<_, Error>(())
            });
            tx
        })
        .collect();

    info!("Start playing in 3 seconds...");
    delay_for(Duration::from_secs(3)).await;
    info!("Started");

    let start = Instant::now();
    let mut last_at = 0;
    for ((at, _), playset) in events {
        debug!("At {}: {:?}", at, playset);

        if last_at != at {
            delay_until(start + Duration::from_millis(at)).await;
        }
        last_at = at;

        if let Some(cube) = cubes.get(playset.ch as usize) {
            let _ = cube.send(playset);
        }
    }

    info!("Shutting down in 3 seconds...");
    delay_for(Duration::from_secs(3)).await;
    info!("Done");

    Ok(())
}
