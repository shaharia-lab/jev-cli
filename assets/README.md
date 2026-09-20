# Banner and demo

Both images in the README are generated from the sources in `src/`, so they can be remade when
the tool changes.

## Banner (`banner.png`, 2000x840)

`src/banner.html` rendered by headless Chrome:

```bash
google-chrome --headless=new --disable-gpu --hide-scrollbars \
  --window-size=2000,840 --screenshot=assets/banner.png file://$PWD/assets/src/banner.html
```

## Demo (`demo.gif`)

A real terminal session against the real API, recorded with
[asciinema](https://asciinema.org) and rendered with [agg](https://github.com/asciinema/agg).
`src/demo.sh` types each command and then runs it, so the output in the recording is genuine.

```bash
cd assets/src
export TYPESAFE_API_KEY=...       # a real key: the recording shows real answers
asciinema rec --overwrite -q --cols 100 --rows 26 -i 1.2 -c "bash demo.sh" demo.cast
agg --theme github-dark --font-size 15 --line-height 1.45 --fps-cap 12 --speed 1.15 \
  --idle-time-limit 1.0 --last-frame-duration 4 demo.cast demo-raw.gif
gifsicle -O3 demo-raw.gif -o ../demo.gif
```

Nothing in the recording prints the key: `jev` never shows it, and the script never echoes the
environment. Check the finished GIF before committing it anyway.
