---
id: core-widgets
title: Core widgets
section: Components
order: 30
keywords: buttons, chips, switches, fields, rows, colors, states
context: core
---

# Core widgets

The Core page lays every control out in panels: buttons, chips, switches,
segmented controls, sliders, text and search fields, list rows, the inspector
and the color swatches.

## How to try a control

1. Point at a control to see its hover fill.
2. Click a chip or a switch to change its state.
3. Drag a slider or type in a field.
4. Press <kbd>Tab</kbd> to move the focus ring between controls.

Each cell is labelled with the component name in `type.caption`. The states
that cannot be forced side by side have their own cell: hover, pressed,
focused and disabled.

## What the states mean

- **Idle** is the resting look: `bg.control` or a transparent surface.
- **Hover** lifts the fill to `bg.hover`.
- **Selected** takes the accent color and its tint.
- **Disabled** drops the text to `text.disabled` and ignores clicks.

The focused copies rotate through the page until you click somewhere, so you
can see the focus ring without pressing <kbd>Tab</kbd>.

> Tip: The color swatches at the bottom of the page read the live theme, so a
> new accent in **Appearance** repaints them at once.

See [Appearance](topic:appearance).
