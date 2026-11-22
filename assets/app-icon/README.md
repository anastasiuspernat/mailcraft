# MailCraft app icon

**Harbor**: three envelopes stacked on a rounded tile, the front one white with its flap as one
bold blue V, the two behind it fainter the further back, on a tile that runs from cyan at its top
to deep blue: a full inbox.

The design is the project owner's. The SVG was written as vectors for MailCraft (not traced, and
nothing in it comes from any other product's icon). It is original work of the MailCraft
contributors, under the project's licence (MIT OR Apache-2.0). The earlier icon, Martin Ball's
envelope that is also a calendar page, is kept in `alternates/`.

## Files

| Path | What |
|---|---|
| `source/mailcraft.svg` | The source: everything else here is written from it |
| `alternates/calmail-{blue,teal,violet,sunset,green,graphite}.svg` | The earlier icon in each of its colour schemes, kept and not used |
| `alternates/gen.py` | Writes the earlier icon's schemes from one template and a palette per scheme |
| `hicolor/` | Linux: PNGs from 16 to 512 px, and the SVG as the scalable icon |
| `mailcraft.ico` | Windows: 16 to 256 px |
| `mailcraft.icns`, `mailcraft-macos-512.png` | macOS, on Apple's 824/1024 grid |
| `mailcraft-1024.png` | The icon at 1024 px |

## Geometry

The tile is fitted to the canvas, whatever margin the SVG keeps round it (the bounding box of
what it draws, made square about its centre); the scalable icon's `viewBox` is set to the same
square. The macOS files keep Apple's margin (an 824 px body on a 1024 px canvas). The 16, 24, 32
and 48 px files show the drawing a little closer inside the same rounded tile (1.14, 1.08, 1.08
and 1.04 times), so the envelopes still read.

## Changing it

Edit `source/mailcraft.svg`, then:

```sh
cargo xtask icon
```

It renders the SVG with `resvg`, a pure-Rust renderer that only xtask depends on, and writes every
file above. Commit what changed; each file has its row in `ATTRIBUTION.md`.
