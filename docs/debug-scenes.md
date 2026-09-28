# Debug scenes

Use `sliderino-debug` to make an image of a slide without the editor UI.
A scene file gives the changes to make to a new presentation.

The tool is for contributors. It is a cargo example, so it is not in the
releases. Run it with `cargo run --example sliderino-debug`.

## Commands

Render a slide on the CPU:

```sh
cargo run --example sliderino-debug -- render --ops debug/scenes/text.json -o out.png
```

| Option | Default | Function |
| --- | --- | --- |
| `--ops FILE` | (required) | The scene file. |
| `-o FILE` | (required) | The PNG file to write. |
| `--slide ID` | `1` | The slide to render. |
| `--scale N` | `1.0` | Pixels for each slide unit. The slide is 1600 × 900 units. |
| `--overlay` | off | Show the text frames, the line boxes, the baselines and the overflow. |

Open the scene in the editor:

```sh
cargo run --example sliderino-debug -- scene --ops debug/scenes/text.json -o shot.png
```

| Option | Default | Function |
| --- | --- | --- |
| `--ops FILE` | (required) | The scene file. |
| `-o FILE` | none | Capture the slide to this PNG file, then close the editor. Without this option, the editor stays open. |
| `--slide ID` | first slide | The slide to show. |
| `--select ID` | none | Select this element. The capture shows its outline, handles and size. |
| `--full` | off | Capture all of the window, not only the slide. |

The capture needs an X11 display, `xdotool` and `import` (ImageMagick).
Each operation of the scene is one step in the History tab.

Both commands write one line for each text box and each table of the
slide: the frame, the number of lines, the overflow and the number of
characters that the font does not have.

## Scene files

A scene file is a JSON list of operations. The `op` field gives the type of
operation. The operations are the same as the editor uses. The agent API uses
the same format (see `agent-api.md`).

```json
[
  {"op": "add_font", "face": {"family": "Inter", "weight": 600}},
  {"op": "add_element", "slide": 1, "element": {
    "id": 1, "frame": {"x": 120, "y": 80, "width": 640},
    "text": {"content": "Activation grew faster than signups", "sizing": "auto_height",
             "style": {"font": {"family": "Inter", "weight": 600}, "size": 64}}}},
  {"op": "set_text_style", "id": 1, "patch": {"underline": true}}
]
```

| `op` | Fields |
| --- | --- |
| `add_slide` | `slide` (`{"id": 2}`, optional), `index` (optional) |
| `remove_slide` | `id` |
| `move_slide` | `id`, `index` |
| `add_element` | `slide`, `parent` (optional; a group of the slide), `element`, `index` (optional; the element goes on top) |
| `remove_element` | `id` |
| `move_element` | `id`, `parent` (optional; without it, the slide), `index` (optional; the element goes on top) |
| `group` | `id` (optional), `children` (a list of element ids on one slide) |
| `ungroup` | `id` |
| `set_layer` | `id`, `patch` (`name`, `hidden`, `locked`, `opacity`; only the fields to change) |
| `set_frame` | `id`, `frame` |
| `set_text_sizing` | `id`, `sizing` |
| `set_text_style` | `id`, `patch` (only the fields to change) |
| `set_shape_style` | `id`, `patch` (`fill`, `stroke`, `corner_radius`, `start`, `end`; only the fields to change) |
| `set_line_points` | `id`, `from` (`{"x": 0, "y": 0}`), `to` |
| `replace_text` | `id`, `range` (`{"start": 0, "end": 5}`, in bytes), `text` |
| `add_font` | `face`, or `path` (from the folder of the scene) or `data` (base64) of a font file |
| `remove_font` | `face`, or `family` |
| `add_image` | `id` (optional), `path` (from the folder of the scene) or `data` (base64) |
| `remove_image` | `id` |
| `add_video` | `id` (optional), `path` (from the folder of the scene) or `data` (base64). Other formats than MP4 with H.264 change to it. |
| `remove_video` | `id` |
| `set_cell_text`, `set_cells`, `insert_rows`, `remove_rows`, `move_rows`, `insert_columns`, `remove_columns`, `move_columns`, `merge_cells`, `split_cell`, `set_table_style`, `set_cell_style`, `set_borders`, `set_table_sizing` | See "Tables" in `agent-api.md` |
| `batch` | `ops` (a list of operations; all apply or none apply) |

Rules:

- The first slide of a new presentation has the id `1`. The ids of new
  slides and elements are optional. If you do not give an id, the tool gives
  the next free id. A reference such as `"$title"` gives a new id a name,
  and later operations can use that name as the id.
- A text can only use a font face that the scene adds with `add_font`.
  `add_font` gives only the name of the face. The tool gets the font data
  from the fonts of Sliderino (Inter) or from the fonts of the system. If the
  license of the font does not permit embedding, the tool shows a warning.
  Scenes do not add fonts automatically, but the agent API does.
- `add_font` with `path` adds the faces of a TTF, OTF or TTC file. See
  `debug/scenes/custom-font.json` and "Fonts" in `agent-api.md`.
- `sizing` is `auto_width`, `auto_height` or `fixed`.
- An element has a `text`, a `group`, a `rectangle`, an `ellipse`, a
  `line` or a `table`. See `debug/scenes/tables.json` and "Tables" in
  `agent-api.md`. A group has `children`, a list of elements. The last child is on
  top. Frames of children are in slide units, as all frames.
- Shapes have the same fields as in the agent API. See "Shapes" in
  `agent-api.md`.
- An element can have a `name`, `hidden`, `locked` and `opacity` (0 to 1).
  Set them with `set_layer`. `"name": null` removes the name.
- In `style` and `patch`, all fields are optional: `font`, `size`,
  `line_height` (`"auto"` or `{"percent": 120}`), `letter_spacing` (percent of
  the size), `align` (`left`, `center`, `right`, `justify`),
  `vertical_align` (`top`, `middle`, `bottom`), `paragraph_spacing`,
  `underline`, `strikethrough`, `case` (`original`, `upper`), `color`
  (`"1A1A1A"`). To make a text transparent, set the `opacity` of the element.
- If an operation fails, the tool stops and shows the number of the
  operation (from 0).

The folder `debug/scenes` has examples: `shapes.json` shows the shapes,
fills, strokes and arrowheads, `images.json` shows image fills, and
`shaders.json` shows shader fills, a shader channel and a shader that does
not compile.

The `scene` command waits until the editor decodes all images and video
frames and renders all shaders before it captures the slide. The capture
shows the first frame of videos and shaders at time 0.

## PPTX export

These commands check each step of the PPTX export. See `pptx-export.md` for
the rules of the export.

| Command | Function |
| --- | --- |
| `export-pptx --ops FILE -o OUT.pptx [--guides]` | Export a scene to PPTX without the editor. `--guides` adds thin blue lines on the text frames and on the baselines of the Sliderino layout. Use it to see where a viewer puts the text. |
| `pptx-dump FILE [--part PART]` | Without `--part`, list the parts of the file with their size and content type. With `--part`, show one part as indented XML, for example `ppt/slides/slide1.xml`. |
| `pptx-check FILE [--schema]` | Check the package: the content types, the relationships, the XML and the shape ids. `--schema` also validates the XML against the Open XML schema. |
| `pptx-roundtrip --ops FILE [-o OUT.pptx]` | Export a scene, read the deck, and compare each shape with its element. The command fails when they are different. |
| `pptx-render FILE -o DIR [--scale N]` | Render each slide of a PPTX file to PNG with LibreOffice. |
| `pptx-compare --ops FILE -o DIR [--slide ID] [--scale N] [--threshold F]` | Render a slide on the CPU and its deck with LibreOffice. Write `reference.png`, `pptx.png`, `diff.png` (the different pixels in red) and `deck.pptx`. Show the fraction of different pixels and where the ink of each element moves. With `--threshold`, fail when more than this fraction of the pixels is different. |

`pptx-render` and `pptx-compare` need LibreOffice (`soffice`) and
`pdftoppm`. `--schema` needs `dotnet`. The first run builds the tool in
`tools/pptx-validate` and gets the Open XML SDK from NuGet.

Example: check a scene in all the steps.

```sh
cargo run --example sliderino-debug -- pptx-roundtrip --ops debug/scenes/text.json -o text.pptx
cargo run --example sliderino-debug -- pptx-check --schema text.pptx
cargo run --example sliderino-debug -- pptx-compare --ops debug/scenes/text.json -o compare --scale 2
```

The folder `debug/scenes/pptx` has scenes for the export: `groups.json`
(groups in groups, turned groups and hidden elements) and `media.json`
(video fills with each fit and start, and a shader).

The tests check the parity of the export:

- `cargo test pptx` exports the debug scenes and compares each deck with
  its presentation. It needs no other programs.
- `cargo test -- --ignored pptx` also compares each deck with LibreOffice,
  pixel by pixel, and validates it against the Open XML schema. Each scene
  has a limit of different pixels. The test fails when a scene becomes
  worse.
