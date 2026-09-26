# Debug scenes

Use `sliderino-debug` to make an image of a slide without the editor UI.
A scene file gives the changes to make to a new presentation.

## Commands

Render a slide on the CPU:

```sh
cargo run --bin sliderino-debug -- render --ops debug/scenes/text.json -o out.png
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
cargo run --bin sliderino-debug -- scene --ops debug/scenes/text.json -o shot.png
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

Both commands write one line for each text box of the slide: the frame, the
number of lines, the overflow and the number of characters that the font
does not have.

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
| `set_layer` | `id`, `patch` (`name`, `hidden`, `locked`; only the fields to change) |
| `set_frame` | `id`, `frame` |
| `set_text_sizing` | `id`, `sizing` |
| `set_text_style` | `id`, `patch` (only the fields to change) |
| `replace_text` | `id`, `range` (`{"start": 0, "end": 5}`, in bytes), `text` |
| `add_font` | `face` |
| `remove_font` | `face` |
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
- `sizing` is `auto_width`, `auto_height` or `fixed`.
- An element has a `text` or a `group`. A group has `children`, a list of
  elements. The last child is on top. Frames of children are in slide
  units, as all frames.
- An element can have a `name`, `hidden` and `locked`. Set them with
  `set_layer`. `"name": null` removes the name.
- In `style` and `patch`, all fields are optional: `font`, `size`,
  `line_height` (`"auto"` or `{"percent": 120}`), `letter_spacing` (percent of
  the size), `align` (`left`, `center`, `right`, `justify`),
  `vertical_align` (`top`, `middle`, `bottom`), `paragraph_spacing`,
  `underline`, `strikethrough`, `case` (`original`, `upper`), `color`
  (`"1A1A1A"`), `opacity` (0 to 1).
- If an operation fails, the tool stops and shows the number of the
  operation (from 0).

The folder `debug/scenes` has examples.
