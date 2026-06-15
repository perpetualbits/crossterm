# mullion — Programming Manual

> A terminal UI **tiling engine** in Rust. You describe a layout as a tree; mullion
> turns it into one rectangle per tile; you paint into those rectangles. This is a
> living document — sections marked **(stub)** track features still landing.
> Status: Phases 0–3b complete (rendering, borders, junctions, layout solver,
> focus, input). Carousels (Phase 4) and zoom (Phase 5) in progress.

---

## 1. The mental model

mullion has one core idea: **a tree of nodes whose leaves are tiles, resolved
against a terminal size into one `Rect` per tile.** You then draw whatever you
like into each tile's rectangle, and a double-buffered `Terminal` diffs and
flushes the frame.

```
Split(Vertical)
├─ Tile(HEADER)      Fixed(3)        // top 3 rows, full width
└─ Split(Horizontal)
   ├─ Tile(SIDEBAR)  Fixed(20)       // 20 cols on the left
   └─ Tile(MAIN)     Fill(1)         // the rest
```

A **`TileId`** (a `u64` you assign) is the stable identity of a logical pane. It
is the linchpin of the whole engine: content, focus, and (later) scroll position
all attach to the `TileId`, so a tile keeps its state even as the tree is
restructured, grown, or pruned at runtime.

The engine never learns what a "CPU graph" or a "process row" is — it only hands
each tile a rect and lets you paint. That separation is what makes mullion
reusable across programs.

---

## 2. Getting started

A minimal frame: build a tree, solve it, frame each tile, paint the interior.

```rust
use mullion::{Buffer, Node, Constraint, Size, Orientation};
use mullion::layout::solve;
use mullion::border::{frame_tiles, Borders, BorderStyle, LineWeight, CornerStyle};
use mullion::geometry::Rect;
use mullion::style::Style;

const HEADER: u64 = 1;
const SIDEBAR: u64 = 2;
const MAIN: u64 = 3;

fn build() -> Node {
    Node::Split {
        orientation: Orientation::Vertical,
        children: vec![
            (Constraint::new(Size::Fixed(3)), Node::Tile(HEADER)),
            (Constraint::new(Size::Fill(1)), Node::Split {
                orientation: Orientation::Horizontal,
                children: vec![
                    (Constraint::new(Size::Fixed(20)), Node::Tile(SIDEBAR)),
                    (Constraint::new(Size::Fill(1)), Node::Tile(MAIN)),
                ],
            }),
        ],
    }
}

fn draw(buf: &mut Buffer, root: &mut Node) {
    let style = BorderStyle { weight: LineWeight::Light, corners: CornerStyle::Square, style: Style::default() };
    let rects = solve(root, buf.area);                       // tree → [(TileId, Rect)]
    let content = frame_tiles(buf, &rects, Borders::ALL, &style); // draw borders, get interiors
    for (id, area) in content {
        match id {
            HEADER  => { buf.set_string(area.x, area.y, "mullion", Style::default()); }
            SIDEBAR => { /* paint the sidebar into `area` */ }
            MAIN    => { /* paint the main pane into `area` */ }
            _ => {}
        }
    }
}
```

Drive it with a `Terminal` (Phase 0): `term.draw(|buf| draw(buf, &mut root))`
clears a back buffer, runs your closure, diffs against the front buffer, and
flushes only the changed cells wrapped in synchronized-output markers.

---

## 3. Concepts

### 3.1 Nodes and constraints

`Node::Split { orientation, children }` divides its rect among children; each
child carries a `Constraint`:

- `Size::Fixed(n)` — exactly `n` cells (clamped to what fits).
- `Size::Percent(p)` — `p`% of the parent extent.
- `Size::Fill(weight)` — shares the leftover space by weight.
- plus optional `.with_min(n)` / `.with_max(n)` clamps; `Constraint::default()`
  is `Fill(1)`.

A split tiles its rect **exactly** when at least one `Fill` child can absorb the
remainder. `Node::Tile(id)` is a leaf.

`Orientation` is `Horizontal`, `Vertical`, or `Adaptive { margin_pct, last }`.
**Adaptive** resolves from the rect's aspect ratio each solve (lay out along the
longer dimension), with a hysteresis dead-zone so it doesn't flicker near square.
Set both a root split and an inner group to `Adaptive` and the whole layout
reorganizes when the terminal goes from wide to tall — see §3.5.

### 3.2 The buffer and Terminal

A `Buffer` is a grid of styled `Cell`s (a grapheme + `Style`), width-aware
(double-width graphemes occupy two cells). `set_string`/`set_grapheme` write
into it. `Terminal<B: Backend>` holds front/back buffers; `draw(|buf| …)` does the
diff-and-flush. Backends: `CrosstermBackend` (real terminal) and `TestBackend`
(headless, renders to a string for tests).

### 3.3 Borders

Two modes:

- **Per-tile** — `frame_tiles(buf, &rects, borders, &style)` draws a box around
  each tile and returns the interior content rect. Adjacent tiles show a doubled
  gutter; that's the intended look. `draw_box` is the underlying primitive.
- **Shared** — `render_shared(buf, &mut root, area, weight, &style, &overrides)`
  draws one outer frame and single-line dividers between sub-tiles, with correct
  `├ ┤ ┬ ┴ ┼` junctions (including mixed light/heavy). Returns content rects.

`LineWeight` is `Light`/`Heavy`/`Double`; `CornerStyle` is `Square`/`Rounded`
(rounded is light-only). The `overrides: &[(TileId, LineWeight)]` argument lets
you draw one tile's border heavier — used for the focus cue (§3.4).

### 3.4 Focus and input

`Tree` owns the root plus focus state:

```rust
use mullion::tree::{Tree, Dir};
let mut tree = Tree::new(build());          // focus starts on the first leaf
tree.focus_next();                          // DFS-order traversal (wraps)
tree.focus_set(MAIN);                       // focus a specific id
tree.flip_focused_parent();                 // flip the focused tile's parent H↔V
tree.swap_focused(Dir::Next);               // swap with the next sibling
```

Focus follows the **`TileId`**, not a position — adding, removing, or reordering
*other* leaves never disturbs it; `ensure_focus_valid()` re-resolves only if the
focused leaf itself disappears.

Input goes through an `InputRouter` that resolves the key-collision problem: a
prefix (default `Ctrl-w`) enters a navigation mode for one key; everything else is
forwarded to the focused tile.

```rust
use mullion::input::{InputRouter, KeyOutcome};
let mut router = InputRouter::new();
match router.handle(key, &mut tree) {
    KeyOutcome::Nav(_cmd)   => { /* focus/flip/swap already applied to the tree */ }
    KeyOutcome::Consumed    => { /* prefix entered, or nav cancelled */ }
    KeyOutcome::Forward(k)  => { /* deliver `k` to the focused tile's content */ }
}
```

Highlight the focused tile by passing `focus_override(&tree, LineWeight::Heavy)`
as the `overrides` to `render_shared`.

### 3.5 Dynamic trees (grow, prune, reconcile)

The tree is plain owned data — grow it with `Vec::push`, prune with `remove`/
`retain`, rearrange by swapping subtrees, then re-solve. Two disciplines keep a
runtime-discovered, churning layout stable:

1. **Derive `TileId`s from durable domain identity** (hash of a VM id, a cgroup
   path) — never positional indices. Then a vanishing item simply isn't in the
   next snapshot, and every surviving item keeps its node (and its focus/scroll/
   history).
2. **Reconcile, don't rebuild.** Diff the new snapshot against the current
   children and mutate in place, reusing surviving subtrees:

```rust
// App-level: make a container's children match `desired`, preserving survivors.
fn reconcile(children: &mut Vec<(Constraint, Node)>, desired: &[(TileId, Constraint)]) {
    let mut old: std::collections::HashMap<u64, (Constraint, Node)> = children
        .drain(..).filter_map(|(c, n)| mullion::tree::tile_id_of(&n).map(|id| (id, (c, n)))).collect();
    for &(id, c) in desired {
        match old.remove(&id) {
            Some((_, node)) => children.push((c, node)),         // survivor: keep its state
            None            => children.push((c, Node::Tile(id))), // newly appeared
        }
    } // leftovers in `old` are gone — dropped here
}
```

Unbounded, runtime-populated collections belong in a **`Carousel`** (§3.6), which
virtualizes; the fixed skeleton (header / main / side) stays a `Split`.

### 3.6 Carousels — scrollable groups **(stub — Phase 4)**

`Node::Carousel { id, orientation, scroll, children }` holds more tiles than fit
and scrolls; only on-screen children are solved and rendered (virtualization).
Addressed by its `id` via `node_by_id_mut` for scrolling and reconciliation.
*Scroll operations and scroll↔focus coupling land in Phase 4b; this section will
expand then.*

### 3.7 Zoom **(stub — Phase 5)**

Re-root the view at the focused subtree (tmux-style), with a zoom stack that
preserves the rest of the tree's focus and scroll state. apptop's drill-down
(host → VM → process, Esc to return) is the same abstraction. *Expands in Phase 5.*

### 3.8 Border labels **(stub — Phase 6)**

Scrolling text in any of a tile's four borders — horizontal marquees on top/bottom,
upright-stacked vertical text on the sides.

---

## 4. API reference by module

| Module | Key items |
|--------|-----------|
| `geometry` | `Rect` (+ `intersection`, `contains`, `area`) |
| `style` | `Style`, `Color`, `Modifier` |
| `buffer` | `Buffer`, `Cell` |
| `backend` | `Backend`, `CrosstermBackend`, `TestBackend` |
| `terminal` | `Terminal` |
| `layout` | `solve`, `Node`, `Constraint`, `Size`, `Orientation`, `Axis` |
| `tree` | `Tree`, `Dir`, `tile_id_of`, `leaves`, `focus_path`, `focus_override`, `node_by_id`/`node_by_id_mut` |
| `input` | `InputRouter`, `KeyOutcome`, `NavCommand`, `Keymap` (+ re-exported `KeyEvent`/`KeyCode`/`KeyModifiers`) |
| `border` | `draw_box`, `frame_tiles`, `render_shared`, `BorderStyle`, `Borders`, `LineWeight`, `CornerStyle` |
| `junction` | `EdgeGrid`, `EdgeCell`, `resolve` (box-drawing junction resolver) |

Common re-exports at the crate root: `Buffer`, `Cell`, `Node`, `Constraint`,
`Size`, `Orientation`, `LineWeight`. Module-scoped: `Axis` (`layout`),
`Dir` (`tree`).

---

## 5. A worked example **(stub — fills out with Phase 8 / apptop integration)**

A small monitor: a `Fixed(3)` header, a sidebar list, and a main pane, with `Tab`
focus traversal, a focus highlight, and keys forwarded to the focused pane. To be
expanded into a runnable example once carousels and zoom land.

---

## 6. Status & roadmap

See `docs/tiling-engine-roadmap.md` for the phase plan and open design questions.
This manual tracks the public API as each phase is reviewed and merged.
