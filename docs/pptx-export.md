# PPTX export

This document tells how Sliderino writes a presentation as a PowerPoint
file (`.pptx`), where the deck is different from the editor, and how to
check the export. The code is in `src/pptx/`.

## Goal

The deck must look like the preview and stay editable in PowerPoint. Each
element keeps its position, its size, its rotation and its paint order. A
text box stays a text box, a table stays a table and a group stays a group,
when PowerPoint can show them correctly.

## Units

A slide unit is 1/120 inch. The 1600 × 900 slide is the 13.333 × 7.5 inch
(16:9) slide of PowerPoint.

| Sliderino | DrawingML |
| --- | --- |
| 1 slide unit | 7620 EMU |
| 1 slide unit of font size | 0.6 point |
| Rotation in degrees, clockwise | `rot` in 60 000ths of a degree |
| Opacity 0 to 1 | `alpha` in 100 000ths |

## Elements

| Element | Deck |
| --- | --- |
| Rectangle | `p:sp` with `rect`, or `roundRect` when it has a corner radius |
| Ellipse | `p:sp` with `ellipse` |
| Line | `p:cxnSp` with `line`, on the horizontal center axis of the frame |
| Text | `p:sp` text box (`txBox`) |
| Group | `p:grpSp` with the rotation of the group |
| Table | `p:graphicFrame` with `a:tbl`; a group of shapes when it is turned |
| Video or shader fill | `p:pic` that plays an MP4 file, with a poster |

The deck does not include hidden elements.

Opacity multiplies into each color, as in the CPU renderer. A group with
opacity 0.5 makes each of its shapes 50% transparent. It does not make the
group one transparent layer.

### Groups

A group keeps its rotation. Its children are in the axes of the group, so
PowerPoint can turn and move the group as one item.

### Shapes

- Strokes are on the center of the outline, with flat ends and miter
  joins.
- Dashes have the lengths of Sliderino: dashes are 4 stroke widths long
  with gaps of 3, and dots are 1 stroke width long with gaps of 1
  (`a:custDash`).
- Arrowheads use the PowerPoint types: `triangle`, `arrow`, `diamond` and
  `oval`. The sizes small, medium and large are `sm`, `med` and `lg`. They
  are 2, 3 and 5 stroke widths, as in Sliderino.
- A linear gradient uses `a:lin` with the angle of Sliderino,
  `scaled="0"`, and turns with the shape.
- A radial gradient uses a circle path. The focus (`a:fillToRect`) is the
  center of the gradient. The deck moves the stops so that they fall where
  the Sliderino ellipse puts them.
- An image fill stretches over the rectangle that its fit gives, as
  offsets from the box of the shape (`a:fillRect`). Cover gives negative
  offsets, contain gives positive offsets. The shape clips the image.
- The deck writes each image once. A JPEG that EXIF turns is encoded again
  upright, because PowerPoint does not read the EXIF orientation.

### Texts

The deck keeps the line breaks of Sliderino:

- Each paragraph (`\n`) is one `a:p`.
- Each line of the layout ends with a line break (`a:br`). The body does
  not wrap (`wrap="none"`) and does not fit its text (`a:noAutofit`).
- A justified paragraph is different: it is one run that wraps in its box
  (`wrap="square"`), because PowerPoint does not justify a line that ends
  with a line break.
- The spaces at the end of a line are removed.

The deck writes the style of the text on each run: size, bold, italic,
underline, strikethrough, `cap="all"` for upper case, character spacing
(`spc`) and color. Lines have an exact height (`a:spcPts`). The Auto line
height becomes the height that Sliderino gives it from the font metrics.
Paragraph spacing is the space after each paragraph, but not the last one.
The insets of the text box are 0.

The first baseline must be where Sliderino puts it. A viewer puts the
baseline of an exact line at 0.2 × the font size above the bottom of the
line, for each font. Sliderino puts the extra height of the line half
above and half below the ascent and the descent of the font. The deck
moves the text box by the difference, along the rotation of the box.

### Fonts

The deck embeds the fonts (`p:embeddedFontLst`) as Embedded OpenType files
(`.fntdata`, version 2.1, not compressed), as PowerPoint writes them.

PowerPoint has four styles for each family: regular, bold, italic and bold
italic. A face with a weight of 600 or more uses the bold style. For each
style, the deck embeds the face that the texts use most. The embedded file
has the names and the weight class of that style, so PowerPoint finds it.
Thus a SemiBold text shows the SemiBold face. When two faces of a family
use one style, the other face shows as the embedded face, and the export
gives a warning.

The deck does not embed:

- A font whose license does not permit embedding (OS/2 `fsType`).
- A font with CFF outlines. PowerPoint embeds only TrueType outlines.

The export gives a warning for each of them.

### Tables

A table is an editable PowerPoint table:

- The column widths and the row heights are those of the Sliderino layout.
- Merged cells use `gridSpan`, `rowSpan`, `hMerge` and `vMerge`.
- Each cell has its fill, its four borders, its vertical alignment and its
  text. The deck uses no table style.
- The margins are the padding of the table. The top and bottom margins
  also move the text by the baseline difference (see "Texts").
- Sliderino makes a column as wide as its widest line. A viewer that
  measures the text a little wider wraps it. Thus the margin where the
  lines end is smaller by 0.25 × the font size, but not more than the
  padding.

PowerPoint cannot turn a table. A turned table becomes a group of its cell
fills, its borders and its texts, and the export gives a warning.

### Videos and shaders

A shape with a video or a shader fill becomes a picture (`p:pic`) that
plays an MP4 file. The first frame is the poster.

- Cover crops the picture (`a:srcRect`). The picture has the geometry and
  the outline of the shape.
- Stretch fills the box of the shape.
- Contain makes the picture the size of the video in the box. The outline
  of the shape is a shape of its own.
- A shader becomes an MP4 file (H.264, no sound) from time 0 to its
  `duration`, at 30 frames per second. Without a GPU, the shape has no
  fill, and the export gives a warning.

The slide timing (`p:timing`) starts the `auto` videos with the slide, all
together. Each `on_click` video starts on its own click, in layer order.
Videos that loop repeat until the slide ends. Muted videos have no sound.

PowerPoint plays videos opaque. When a video or shader fill has an opacity
less than 1, only the poster is transparent, and the export gives a
warning.

## Known differences

These differences are in the design of the deck:

| Feature | Difference |
| --- | --- |
| Radial gradient that is not a circle on the slide | The deck uses a circle with the mean of the two radii. |
| Radial gradient larger than the half diagonal of its box | The colors past the half diagonal show as the color at the half diagonal. |
| Thick horizontal table border | Sliderino extends it by half the stroke width at each end. PowerPoint does not. |
| Contained video in an ellipse or a rounded rectangle | The video shows in a rectangle. |
| Transparent video or shader | The video plays opaque. |
| Turned table | A group of shapes, not an editable table. |

## Assumptions not verified in PowerPoint

The export follows ECMA-376 and the files that PowerPoint writes. The rules
below were measured in LibreOffice 26.8 only. Check them in PowerPoint when
you can. Use `sliderino-debug export-pptx --guides`: the blue guides show
where Sliderino puts the text frames and the baselines.

1. The baseline of a line of exact height is 0.2 × the font size above the
   bottom of the line. LibreOffice does this for Inter, Liberation Serif and
   Noto Sans, whose descents are 0.24, 0.22 and 0.29 of the size.
2. A circle path gradient is a circle around its focus with a radius of
   half the diagonal of the box. LibreOffice does not use `a:tileRect`.
3. PowerPoint reads uncompressed EOT fonts in `.fntdata` parts, and finds
   a face by the family and style names in the file.
4. PowerPoint turns an image fill with its shape (`rotWithShape="1"`).
   LibreOffice does not.
5. PowerPoint applies the transparency of a table border. LibreOffice does
   not.
6. PowerPoint crops, clips, outlines and turns a video picture.
   LibreOffice shows the whole video in its box, without rotation.

## Check the export

`docs/debug-scenes.md` gives the `sliderino-debug` commands for each step:
`export-pptx`, `pptx-dump`, `pptx-check`, `pptx-roundtrip`, `pptx-render`
and `pptx-compare`.

The tests are in `src/pptx/parity_tests.rs`:

- `cargo test pptx` exports the debug scenes, reads each deck again
  (`inspect.rs`) and compares each shape with its element (`parity.rs`).
- `cargo test -- --ignored pptx` renders each deck with LibreOffice and
  compares it with the CPU renderer. It also validates each deck against the
  Open XML schema. These tests need LibreOffice, `pdftoppm` and `dotnet`.
