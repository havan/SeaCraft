# Building from a SeaCraft hull

This page is for someone who wants to build the boat: station moulds to set up on a strongback, a profile
(backbone) pattern, a transom pattern, and a table of offsets to loft from by hand. It says what the program
draws, how to print it, and what it does not check.

## What a mould is, and why the skin comes off it

A station mould is a cross-section of the hull cut out of sheet material and set up at its station, so the skin can
be bent over the row of moulds. The designed hull is the outside of the boat. If a mould were cut to that line,
the skin laid over it would end up one skin thickness too big all round. So the program cuts each section from the
hull and moves it inward by the skin thickness: plank, plywood, or strips plus glass and fairing compound.

Where the hull is not square to the station (towards the bow and stern), the skin meets the mould at an angle and
the program takes more off, the thickness divided by the cosine of that angle. Where the surface runs almost along
the station plane it stops at five times the thickness and says on the sheet how many pieces it held back.

The program cannot know how thick your skin is. Give it: `--skin-mm 6` for 6 mm plywood, say. With no skin given
the cut line is the hull surface, and every sheet says "no skin deducted".

## Choosing stations

By default there are 11 stations at the middles of equal slices of the hull, the same stations as the table of
offsets. `--stations 15` gives more; `--spacing 0.5` puts one at every 0.5 m of model x; `--at 0.6,1.2,2.4` puts
them exactly where you want them, in metres along the model's x axis. A station that misses the hull is left out
and named in a warning.

## Making the patterns

On the command line:

```console
seacraft templates hull.fef -o moulds.pdf --skin-mm 6 --profile --transom 0.0,12
```

The table it prints has one row per pattern: where it is, its half-breadth, which face of the mould goes on the
station line, and the largest bevel. Read the warnings it prints after the table.

In the desktop app: **File ▸ Builder templates…** opens a window with the same options. It works out the page
count in the background and shows it with any warnings; **Export…** writes the file. **File ▸ Export ▸ Offsets
table PDF…** writes the table of offsets.

An AI agent can do the same through the MCP server with `export_templates` and `export_offsets_pdf`; the
[`seacraft-boatbuilding`](../skills/seacraft-boatbuilding/SKILL.md) skill tells it how.

## Printing at full size

The PDF tiles each pattern over ordinary paper (A4 by default; `--paper a3`, `--paper letter`, or `--paper 610x914`
in millimetres).

1. Print at **100 %**, "actual size". Never "fit to page".
2. Measure both 100 mm rules on the first page (and the 4 inch rules with `--imperial`). If either is not 100 mm,
   do not cut: fix the printer setting and print again.
3. Each pattern starts with a map page showing which tiles were printed and their page numbers (A1, A2, B1, …).
   Tiles with nothing on them are not printed.
4. Neighbouring tiles overlap by 10 mm (`--overlap-mm`). Lay them so the join lines and crosses on both pages sit
   on top of each other, and tape them.
5. Cut on the centre of the heavy line. The dashed grey line is the hull surface, there for reference.

`--single-page` gives one page a pattern instead, sized to fit it, for a plotter or a print shop; every such page
carries its own rules. A pattern bigger than a printer can take is easier as a DXF.

A big hull at full size can need more pages than the program will tile (1000); it says so and suggests larger
paper, single pages or a DXF. `--scale 1:10` makes a small copy to look at, but you cannot cut moulds from it.

## Which face goes on the line

A mould has thickness too. Set it on the wrong side of its station line and its edge stands proud of the skin.
Each sheet says which face goes on the line, worked out from the hull's shape there:

- "Station line on the FORWARD face of the mould": the hull gets smaller going forward, so the mould stands aft of
  its line.
- "Station line on the AFT face of the mould": the other way, near the stern.
- "Either face on the station line" where the hull is parallel.

Where part of a section narrows one way and part the other, as when the sides narrow forward while the sheer rises,
no face clears the skin everywhere. The face is then chosen for the greater length of the section, and the sheet
says which face to bevel where the rest narrows the other way, and by up to how much: "Bevel the aft face where the
hull narrows aft, up to 4.1°, or it stands proud of the skin".

The sheet also gives the largest bevel along the section. That is the most any part of the edge needs, not what
the whole edge needs: the bevel changes along the section. Find it at each part of the edge by laying a batten
across several moulds, and plane each part until the batten lies fair, before planking.

## The strongback

Moulds are usually set up upside down on a strongback. `--strongback 2.0` gives the strongback's height as a
model z, in metres, at or above the sheer; each mould then gets legs running up to it, and a line marks the level.
A level below the top of the hull is refused. `--half` draws the starboard half only, to be flipped about the
centreline for the other side.

## Profile and transom

`--profile` adds the backbone pattern: the hull cut on its centreline, bow to the right, with a numbered mark at
each station, the baseline and the waterlines.

`--transom X,RAKE` adds the transom: X is where its plane meets the baseline, in metres along the model, and RAKE
how far its top leans aft of upright, in degrees. It is drawn in its own sloping plane, so the shape is true;
heights on it run up the slope. The cut is taken 1 mm forward of the plane given, because a plane lying exactly in
the transom face has no section; the sheet says so.

## DXF for a CNC shop

`-o moulds.dxf` writes every pattern into one drawing in millimetres, on layers `CUT` (the mould line), `MARK`
(centreline, waterlines, baseline, station marks), `REFERENCE` (the hull surface) and `TEXT` (names and notes).

## The table of offsets

```console
seacraft offsets hull.fef --pdf offsets.pdf --imperial
```

writes half-breadths on waterlines and heights on buttocks, in millimetres above the keel, and with `--imperial`
the same tables again in feet-inches-eighths (`6-6-6` is 6 ft 6 6/8 in; a `+` is one sixteenth more). A dash means
the section does not reach that line.

## Smoother lines

The patterns are cut from the refined hull, a mesh of flat pieces. Each sheet gives the longest straight piece
along its section. If that is more than a few tens of millimetres on a curved part, raise the refinement:
`--level 4` or `--level 5`.

## What the program does not check

The patterns are geometry computed from the hull you gave it. The program does not know your real plywood or plank
thickness, how your printer scales, how the material will bend, or whether the design itself is sound. Nobody has
yet printed a set of these patterns and measured it, or built a boat on them. Measure the rules, check a few
dimensions against the table of offsets before cutting, and read [`DISCLAIMER.md`](../DISCLAIMER.md).
