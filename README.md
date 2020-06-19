# toio-midi

Let [toio](https://toio.io/) cube play music.

## Usage

You can assign tracks in the MIDI file to each cube. For example,

```
./toio-midi ./battle.mid -r 0=2 1=3
```

Cube 0 plays track 2, while cube 1 plays track 3.

```
./toio-midi ./battle.mid -r 0=2,4 1=3
```

Cube 0 plays both track 2 and 4, while cube 1 plays track 3.

To list the available tracks,

```
./toio-midi ./battle.mid -l
```

See the help for more details,

```
toio-midi 0.1.0

USAGE:
    toio-midi [FLAGS] [OPTIONS] <file>

FLAGS:
    -h, --help       Prints help information
    -l, --list       List tracks
    -V, --version    Prints version information

OPTIONS:
    -r, --rule <rules>...    Rules to assign tracks to cube
    -s, --speed <speed>      Speed [default: 100]
    -u, --unit <unit>        Time-slice size used on merge [default: 40]

ARGS:
    <file>    MIDI file name
```

