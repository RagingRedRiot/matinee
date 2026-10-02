# matinee

**Scripted, reproducible demos of your software, recorded from the real thing.**

A matinee is a showing put on for an audience. This tool puts one on for
your project: you write the demo once as a script, and matinee performs it
against the real program, with real, seeded data, and records it as a GIF or
video, with a camera that eases in on whatever is being clicked, typed, or
dragged so small controls read clearly in a large window. Change the program,
run matinee again, and the demo is current.

```sh
matinee record demo.toml        # perform and record one demo
matinee record                  # every demo in the project
```

> **Status:** not built yet. This repository holds the [vision](VISION.md) and
> a working [prototype](prototypes/) that recorded callboard's README demo.
> Nothing here is published.

## Why

Hand-recorded demos go stale the day the interface changes, take a steady
hand to redo, and are never quite the same twice. Terminal tools have
[VHS](https://github.com/charmbracelet/vhs) for this. Desktop apps, and
projects that are part terminal, part GUI, have nothing comparable. matinee
aims to be that: one storyboard format, one command, for terminals and GUIs
alike.

## Read next

- [VISION.md](VISION.md): philosophy, goals, and the plan.
- [prototypes/](prototypes/): what has already been proven, and what it taught us.

## License

[MIT](LICENSE)
