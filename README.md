# toio-midi

Let [toio](https://toio.io/) cube play music.

## Usage

```
./toio-midi battle.mid
```

Each cube plays a channel in the specified MIDI file.
If there're three connected cubes, three channels are played at the same time.

```
toio-midi 0.1.0

USAGE:
    toio-midi [OPTIONS] <file>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -c, --channel <channel>...    Channels to assign to cubes

ARGS:
    <file>    MIDI file name
```
