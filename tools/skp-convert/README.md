# skp-convert: SketchUp models for Daedalus

SketchUp's `.skp` format is closed. [OpenSKP](https://github.com/iamahsanmehmood/openskp) (MIT, by Ahsan Mehmood; made by
studying the format, not affiliated with Trimble) reads it; this helper turns its reading into files Daedalus opens. It
only reads the `.skp` files.

Install (once), in an environment of its own:

```sh
python3 -m venv ~/.local/share/daedalus/openskp
~/.local/share/daedalus/openskp/bin/pip install openskp
cp skp-convert.py ~/.local/share/daedalus/
install -m755 skp-convert ~/.local/bin/
```

- `skp-convert [FOLDER]` (default `~/3D Objects`): every `.skp` under it into `FOLDER/Converted from SketchUp/`: `.dskp`
  (groups and components, for Daedalus), `.obj` + `.mtl` (flattened; metres, Z up), `.json` (OpenSKP's full reading), and
  `conversion report.txt`.
- `skp-convert --one FILE.skp OUT.dskp`: what Daedalus runs when a `.skp` is opened in it.

The `.dskp` format is described in `crates/daedalus-io/src/skp.rs`.
