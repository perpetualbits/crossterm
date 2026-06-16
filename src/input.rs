// SPDX-License-Identifier: LGPL-3.0-or-later
// Copyright (C) 2026  Epsilon Null Operation
//! Input router: modal navigation via a prefix key.
//!
//! ## Coupling note
//!
//! This module re-exports [`KeyCode`], [`KeyEvent`], and [`KeyModifiers`]
//! directly from crossterm.  That coupling is intentional — the engine already
//! depends on crossterm's event infrastructure and the key types are stable.
//! If a non-crossterm backend ever appears, this is the one seam to replace.
//!
//! ## Collision model
//!
//! Consumer code (e.g. apptop) binds plain keys for in-tile actions.  To avoid
//! ambiguity the engine reserves a **navigation namespace** behind a prefix key.
//! Default prefix: **`Ctrl-w`** (vim-window style).  Everything else is
//! forwarded to the caller to deliver to the focused tile.
//!
//! The prefix and bindings are held in a replaceable [`Keymap`], so a consumer
//! can choose a different scheme (e.g. a `Ctrl-b` prefix for tmux-style nav).

pub use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::geometry::Rect;
use crate::layout::{solve, TileId};
use crate::mouse::{carousel_at, tile_at};
use crate::tree::{Dir, Tree};

// ── NavCommand ────────────────────────────────────────────────────────────────

/// A navigation action, already decoded from a key.  Executed against the [`Tree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavCommand {
    /// Move focus to the next leaf in DFS order (wrapping).
    FocusNext,
    /// Move focus to the previous leaf in DFS order (wrapping).
    FocusPrev,
    /// Move focus to the first leaf in DFS order.
    FocusFirst,
    /// Move focus to the last leaf in DFS order.
    FocusLast,
    /// Flip the orientation of the focused leaf's parent split.
    Flip,
    /// Swap the focused leaf with its next sibling.
    SwapNext,
    /// Swap the focused leaf with its previous sibling.
    SwapPrev,
    /// Zoom into the currently focused leaf (tmux-style fullscreen).
    ///
    /// Executes [`Tree::zoom_focus`].  A no-op if already zoomed to the focus.
    ZoomIn,
    /// Pop one zoom level, returning to the previous view.
    ///
    /// Executes [`Tree::zoom_out`].  A no-op when not zoomed.
    ZoomOut,
}

// ── KeyOutcome ────────────────────────────────────────────────────────────────

/// Result of feeding one key event to the [`InputRouter`].
#[derive(Debug)]
pub enum KeyOutcome {
    /// A [`NavCommand`] was recognised and already executed on the tree.
    Nav(NavCommand),
    /// The prefix was consumed (entering PendingNav), or a PendingNav was
    /// cancelled.  Nothing to forward.
    Consumed,
    /// Not a navigation key — the app should deliver this event to the focused
    /// tile's content handler.
    Forward(KeyEvent),
}

// ── MouseOutcome ─────────────────────────────────────────────────────────────

/// Result of feeding one mouse event to the [`InputRouter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseOutcome {
    /// A left-button press landed on a tile; focus has already been updated.
    Focused(TileId),
    /// A scroll event landed on a carousel; its scroll offset has been updated.
    Scrolled(TileId),
    /// The event did not match any interactive element (no click target, no
    /// carousel under the cursor, or an event kind the router ignores).
    Ignored,
}

// ── Keymap ────────────────────────────────────────────────────────────────────

/// Maps keys to nav commands while in the PendingNav state.
///
/// ## Default bindings
///
/// Prefix: **`Ctrl-w`**
///
/// | Key | Command |
/// |-----|---------|
/// | `Tab` / `j` | `FocusNext` |
/// | `BackTab` / `k` | `FocusPrev` |
/// | `g` | `FocusFirst` |
/// | `G` | `FocusLast` |
/// | `f` | `Flip` |
/// | `n` | `SwapNext` |
/// | `p` | `SwapPrev` |
/// | `z` | `ZoomIn` |
/// | `Z` | `ZoomOut` |
pub struct Keymap {
    prefix: KeyEvent,
    bindings: Vec<(KeyEvent, NavCommand)>,
}

impl Keymap {
    /// Construct a keymap with a custom prefix and binding list.
    pub fn new(prefix: KeyEvent, bindings: Vec<(KeyEvent, NavCommand)>) -> Self {
        Self { prefix, bindings }
    }

    fn is_prefix(&self, key: &KeyEvent) -> bool {
        key.code == self.prefix.code && key.modifiers == self.prefix.modifiers
    }

    fn lookup(&self, key: &KeyEvent) -> Option<NavCommand> {
        self.bindings
            .iter()
            .find(|(k, _)| k.code == key.code && k.modifiers == key.modifiers)
            .map(|(_, cmd)| *cmd)
    }
}

impl Default for Keymap {
    fn default() -> Self {
        use KeyCode::{BackTab, Char, Tab};
        Self::new(
            KeyEvent::new(Char('w'), KeyModifiers::CONTROL),
            vec![
                (KeyEvent::new(Tab, KeyModifiers::NONE), NavCommand::FocusNext),
                (KeyEvent::new(Char('j'), KeyModifiers::NONE), NavCommand::FocusNext),
                (KeyEvent::new(BackTab, KeyModifiers::NONE), NavCommand::FocusPrev),
                (KeyEvent::new(Char('k'), KeyModifiers::NONE), NavCommand::FocusPrev),
                (KeyEvent::new(Char('g'), KeyModifiers::NONE), NavCommand::FocusFirst),
                (KeyEvent::new(Char('G'), KeyModifiers::NONE), NavCommand::FocusLast),
                (KeyEvent::new(Char('f'), KeyModifiers::NONE), NavCommand::Flip),
                (KeyEvent::new(Char('n'), KeyModifiers::NONE), NavCommand::SwapNext),
                (KeyEvent::new(Char('p'), KeyModifiers::NONE), NavCommand::SwapPrev),
                (KeyEvent::new(Char('z'), KeyModifiers::NONE), NavCommand::ZoomIn),
                (KeyEvent::new(Char('Z'), KeyModifiers::NONE), NavCommand::ZoomOut),
            ],
        )
    }
}

// ── RouterMode ────────────────────────────────────────────────────────────────

/// Internal state of the router's two-mode state machine.
enum RouterMode {
    /// Ordinary passthrough; the prefix key triggers the transition.
    Normal,
    /// One key is pending; it is interpreted as a nav command (or cancels).
    PendingNav,
}

// ── InputRouter ───────────────────────────────────────────────────────────────

/// Modal input router: translates raw key and mouse events into typed outcomes.
///
/// ## State machine (keyboard)
///
/// ```text
/// Normal ──[prefix]──► PendingNav ──[nav key]──► Normal (fires Nav)
///                              └──[Esc / unknown]──► Normal (fires Consumed)
/// Normal ──[other key]──► Normal (fires Forward)
/// ```
///
/// The prefix is single-shot: one prefix → one command.  A sticky repeat mode
/// can be added later without changing this API.
///
/// ## Mouse handling
///
/// [`handle_mouse`](InputRouter::handle_mouse) is stateless with respect to the
/// key state machine: it always resolves the event directly against the tree
/// regardless of whether the router is in `Normal` or `PendingNav` mode.  Mouse
/// events do not cancel a pending key prefix.
pub struct InputRouter {
    /// Current state of the key prefix state machine.
    mode: RouterMode,
    /// Active key bindings.
    keymap: Keymap,
    /// Number of scroll steps fired per wheel tick.  Default: 1.
    wheel_scroll_step: u16,
}

impl InputRouter {
    /// Construct with the default [`Keymap`] in Normal mode and a wheel step of 1.
    pub fn new() -> Self {
        Self { mode: RouterMode::Normal, keymap: Keymap::default(), wheel_scroll_step: 1 }
    }

    /// Construct with a custom [`Keymap`] in Normal mode and a wheel step of 1.
    pub fn with_keymap(km: Keymap) -> Self {
        Self { mode: RouterMode::Normal, keymap: km, wheel_scroll_step: 1 }
    }

    /// Set the number of scroll steps fired per wheel tick.
    ///
    /// The wheel step applies to both `ScrollUp` and `ScrollDown` events handled
    /// by [`handle_mouse`](InputRouter::handle_mouse).  The default is 1.
    pub fn set_wheel_scroll_step(&mut self, step: u16) -> &mut Self {
        self.wheel_scroll_step = step;
        self
    }

    /// Feed one key event.  Mutates `tree` when a [`NavCommand`] fires.
    pub fn handle(&mut self, key: KeyEvent, tree: &mut Tree) -> KeyOutcome {
        match self.mode {
            RouterMode::Normal => {
                if self.keymap.is_prefix(&key) {
                    self.mode = RouterMode::PendingNav;
                    KeyOutcome::Consumed
                } else {
                    KeyOutcome::Forward(key)
                }
            }
            RouterMode::PendingNav => {
                self.mode = RouterMode::Normal;
                if key.code == KeyCode::Esc {
                    return KeyOutcome::Consumed;
                }
                match self.keymap.lookup(&key) {
                    Some(cmd) => {
                        execute_nav(cmd, tree);
                        KeyOutcome::Nav(cmd)
                    }
                    None => KeyOutcome::Consumed,
                }
            }
        }
    }

    /// Drive focus and carousel scroll from one mouse event.
    ///
    /// Hit-tests the **effective** (zoom-aware) subtree of `tree` laid out in
    /// `area`.  Three event kinds are handled:
    ///
    /// - **Left-button press** — [`solve`]s the effective root, then calls
    ///   [`tile_at`] to find the tile under the cursor.  If found, calls
    ///   [`Tree::focus_set`] and returns `Focused(id)`.
    /// - **Scroll up / scroll down** — calls [`carousel_at`] on the effective
    ///   root to find the innermost carousel under the cursor, then calls
    ///   [`Tree::scroll_by`] with `−step` or `+step` respectively (where `step`
    ///   is [`wheel_scroll_step`](InputRouter::set_wheel_scroll_step)).  Returns
    ///   `Scrolled(id)`.
    /// - **Everything else** — `Ignored`.
    ///
    /// ## Zoom behaviour
    ///
    /// Because both `solve` and `carousel_at` operate on `effective_root_mut()`,
    /// a click on a tile that is outside the zoom window returns `Ignored`
    /// (the tile does not appear in the effective subtree's solved rects).
    /// Likewise a wheel event when the effective root is a single `Tile` returns
    /// `Ignored` (no carousel exists under the point).
    ///
    /// ## Composed layouts
    ///
    /// For applications that render separate regions with independent trees (e.g.
    /// a header split and a body carousel), call [`tile_at`] and [`carousel_at`]
    /// directly on each region's rect list rather than using this method with a
    /// combined area.
    ///
    /// # Parameters
    /// - `ev`: The crossterm [`MouseEvent`] to process.
    /// - `tree`: The layout tree; mutated on click (focus) or wheel (scroll).
    /// - `area`: The same `Rect` passed to `solve` / `render_carousel` for this
    ///   tree, so the solve geometry matches the render geometry.
    ///
    /// # Returns
    /// A [`MouseOutcome`] indicating what, if anything, was updated.
    pub fn handle_mouse(&mut self, ev: MouseEvent, tree: &mut Tree, area: Rect) -> MouseOutcome {
        let (x, y) = (ev.column, ev.row);
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Solve the effective subtree to obtain on-screen rects, then
                // hit-test.  The Vec is owned so the mutable borrow of tree ends
                // before focus_set takes a new borrow.
                let rects = solve(tree.effective_root_mut(), area);
                match tile_at(&rects, x, y) {
                    Some(id) => {
                        tree.focus_set(id);
                        MouseOutcome::Focused(id)
                    }
                    None => MouseOutcome::Ignored,
                }
            }
            MouseEventKind::ScrollUp => {
                // carousel_at returns an owned TileId (Copy); the mutable borrow
                // of tree ends before scroll_by takes a new one.
                match carousel_at(tree.effective_root_mut(), area, x, y) {
                    Some(id) => {
                        // Scroll backward: negative delta, saturating at 0.
                        tree.scroll_by(id, -(self.wheel_scroll_step as i32));
                        MouseOutcome::Scrolled(id)
                    }
                    None => MouseOutcome::Ignored,
                }
            }
            MouseEventKind::ScrollDown => {
                match carousel_at(tree.effective_root_mut(), area, x, y) {
                    Some(id) => {
                        tree.scroll_by(id, self.wheel_scroll_step as i32);
                        MouseOutcome::Scrolled(id)
                    }
                    None => MouseOutcome::Ignored,
                }
            }
            _ => MouseOutcome::Ignored,
        }
    }
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new()
    }
}

fn execute_nav(cmd: NavCommand, tree: &mut Tree) {
    match cmd {
        NavCommand::FocusNext  => tree.focus_next(),
        NavCommand::FocusPrev  => tree.focus_prev(),
        NavCommand::FocusFirst => tree.focus_first(),
        NavCommand::FocusLast  => tree.focus_last(),
        NavCommand::Flip       => tree.flip_focused_parent(),
        NavCommand::SwapNext   => tree.swap_focused(Dir::Next),
        NavCommand::SwapPrev   => tree.swap_focused(Dir::Prev),
        NavCommand::ZoomIn     => { tree.zoom_focus(); }
        NavCommand::ZoomOut    => tree.zoom_out(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::layout::{Constraint, Node, Orientation, Size};
    use crate::tree::{leaves, node_by_id};

    fn tile(id: u64) -> Node {
        Node::Tile(id)
    }

    fn h_split(kids: Vec<Node>) -> Node {
        Node::Split {
            orientation: Orientation::Horizontal,
            children: kids.into_iter()
                .map(|n| (Constraint::new(Size::Fill(1)), n))
                .collect(),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn two_tile_tree() -> Tree {
        Tree::new(h_split(vec![tile(0), tile(1)]))
    }

    // ── Router state machine ──────────────────────────────────────────────

    #[test]
    fn prefix_in_normal_enters_pending_nav() {
        let mut router = InputRouter::new();
        let mut tree = two_tile_tree();
        let outcome = router.handle(ctrl('w'), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Consumed));
        // Mode is now PendingNav — next nav key should fire.
        let outcome = router.handle(key(KeyCode::Tab), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Nav(NavCommand::FocusNext)));
    }

    #[test]
    fn non_prefix_in_normal_forwards_key() {
        let mut router = InputRouter::new();
        let mut tree = two_tile_tree();
        let outcome = router.handle(key(KeyCode::Char('a')), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Forward(_)));
        // Still in Normal — next key also forwards.
        let outcome = router.handle(key(KeyCode::Enter), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Forward(_)));
    }

    #[test]
    fn prefix_tab_fires_focus_next_and_returns_to_normal() {
        let mut router = InputRouter::new();
        let mut tree = two_tile_tree();
        assert_eq!(tree.focus(), Some(0));
        router.handle(ctrl('w'), &mut tree);
        let outcome = router.handle(key(KeyCode::Tab), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Nav(NavCommand::FocusNext)));
        assert_eq!(tree.focus(), Some(1));
        // Back in Normal — arbitrary key forwards.
        assert!(matches!(router.handle(key(KeyCode::Char('x')), &mut tree), KeyOutcome::Forward(_)));
    }

    #[test]
    fn prefix_esc_cancels_no_tree_change() {
        let mut router = InputRouter::new();
        let mut tree = two_tile_tree();
        let before = tree.focus();
        router.handle(ctrl('w'), &mut tree);
        let outcome = router.handle(key(KeyCode::Esc), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Consumed));
        assert_eq!(tree.focus(), before);
        // Back in Normal.
        assert!(matches!(router.handle(key(KeyCode::Char('a')), &mut tree), KeyOutcome::Forward(_)));
    }

    #[test]
    fn prefix_unmapped_key_cancels_no_tree_change() {
        let mut router = InputRouter::new();
        let mut tree = two_tile_tree();
        let before = tree.focus();
        router.handle(ctrl('w'), &mut tree);
        // 'Q' is not in the default keymap → Consumed, no tree change.
        let outcome = router.handle(key(KeyCode::Char('Q')), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Consumed));
        assert_eq!(tree.focus(), before);
        // Back in Normal.
        assert!(matches!(router.handle(key(KeyCode::Char('a')), &mut tree), KeyOutcome::Forward(_)));
    }

    #[test]
    fn custom_keymap_routes_with_different_prefix_and_bindings() {
        let km = Keymap::new(
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            vec![
                (key(KeyCode::Char('n')), NavCommand::FocusNext),
                (key(KeyCode::Char('p')), NavCommand::FocusPrev),
            ],
        );
        let mut router = InputRouter::with_keymap(km);
        let mut tree = two_tile_tree();

        // Default prefix (Ctrl-w) is no longer the prefix — forwards.
        assert!(matches!(router.handle(ctrl('w'), &mut tree), KeyOutcome::Forward(_)));

        // Custom prefix (Ctrl-b) enters PendingNav.
        let outcome = router.handle(
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            &mut tree,
        );
        assert!(matches!(outcome, KeyOutcome::Consumed));

        // 'n' fires FocusNext.
        let outcome = router.handle(key(KeyCode::Char('n')), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Nav(NavCommand::FocusNext)));
    }

    #[test]
    fn all_default_bindings_fire() {
        use NavCommand::*;
        let cases: &[(KeyEvent, NavCommand)] = &[
            (key(KeyCode::Tab), FocusNext),
            (key(KeyCode::Char('j')), FocusNext),
            (key(KeyCode::BackTab), FocusPrev),
            (key(KeyCode::Char('k')), FocusPrev),
            (key(KeyCode::Char('g')), FocusFirst),
            (key(KeyCode::Char('G')), FocusLast),
            (key(KeyCode::Char('f')), Flip),
            (key(KeyCode::Char('n')), SwapNext),
            (key(KeyCode::Char('p')), SwapPrev),
            (key(KeyCode::Char('z')), ZoomIn),
            (key(KeyCode::Char('Z')), ZoomOut),
        ];

        let root = Node::Split {
            orientation: Orientation::Horizontal,
            children: (0u64..4)
                .map(|i| (Constraint::new(Size::Fill(1)), Node::Tile(i)))
                .collect(),
        };

        for (key_event, expected) in cases {
            let mut router = InputRouter::new();
            let mut tree = Tree::new(root.clone());
            router.handle(ctrl('w'), &mut tree);
            match router.handle(*key_event, &mut tree) {
                KeyOutcome::Nav(cmd) => assert_eq!(cmd, *expected),
                other => panic!("expected Nav({:?}), got {:?}", expected, other),
            }
        }
    }

    #[test]
    fn swap_next_via_router_reorders_siblings() {
        let mut router = InputRouter::new();
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1), tile(2)]));
        router.handle(ctrl('w'), &mut tree);
        router.handle(key(KeyCode::Char('n')), &mut tree); // SwapNext
        assert_eq!(leaves(tree.root()), vec![1, 0, 2]);
        assert_eq!(tree.focus(), Some(0));
    }

    #[test]
    fn flip_via_router_changes_orientation() {
        let mut router = InputRouter::new();
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1)]));
        router.handle(ctrl('w'), &mut tree);
        router.handle(key(KeyCode::Char('f')), &mut tree); // Flip
        assert!(matches!(tree.root(), Node::Split { orientation: Orientation::Vertical, .. }));
    }

    #[test]
    fn zoom_in_via_router_drives_zoom_focus() {
        let mut router = InputRouter::new();
        // focus starts at tile 0; Ctrl-w z → ZoomIn → zoom_focus() into tile 0.
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1)]));
        router.handle(ctrl('w'), &mut tree);
        let outcome = router.handle(key(KeyCode::Char('z')), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Nav(NavCommand::ZoomIn)));
        assert!(tree.is_zoomed(), "tree must be zoomed after ZoomIn");
        assert_eq!(tree.zoom_depth(), 1);
        assert!(matches!(tree.effective_root(), Node::Tile(0)));
    }

    #[test]
    fn zoom_out_via_router_drives_zoom_out() {
        let mut router = InputRouter::new();
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1)]));
        tree.zoom_focus(); // zoom in manually so we can test zoom-out
        assert!(tree.is_zoomed());

        router.handle(ctrl('w'), &mut tree);
        let outcome = router.handle(key(KeyCode::Char('Z')), &mut tree);
        assert!(matches!(outcome, KeyOutcome::Nav(NavCommand::ZoomOut)));
        assert!(!tree.is_zoomed(), "tree must not be zoomed after ZoomOut");
    }

    // ── handle_mouse ─────────────────────────────────────────────────────

    fn make_mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
    }

    fn v_carousel(id: u64, scroll: u16, child_h: u16, n: u64) -> Node {
        Node::Carousel {
            id,
            orientation: Orientation::Vertical,
            scroll,
            children: (0..n).map(|i| (child_h, Node::Tile(i))).collect(),
        }
    }

    #[test]
    fn mouse_left_press_focuses_tile_under_cursor() {
        // H-split [Tile(0) | Tile(1)] in 40×10.  Tile(0) = x[0,20), Tile(1) = x[20,40).
        let area = Rect::new(0, 0, 40, 10);
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1)]));
        let mut router = InputRouter::new();
        assert_eq!(tree.focus(), Some(0), "focus starts at tile 0");

        // Click in the right half → should focus Tile(1).
        let ev = make_mouse(MouseEventKind::Down(MouseButton::Left), 25, 5);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Focused(1)), "expected Focused(1)");
        assert_eq!(tree.focus(), Some(1));
    }

    #[test]
    fn mouse_left_press_in_gap_returns_ignored() {
        // Fixed(10) + Fixed(10) in a 40-col area: x=20..40 is empty.
        let area = Rect::new(0, 0, 40, 10);
        let mut tree = Tree::new(Node::Split {
            orientation: Orientation::Horizontal,
            children: vec![
                (Constraint { size: Size::Fixed(10), min: 0, max: u16::MAX }, Node::Tile(0)),
                (Constraint { size: Size::Fixed(10), min: 0, max: u16::MAX }, Node::Tile(1)),
            ],
        });
        let mut router = InputRouter::new();
        let ev = make_mouse(MouseEventKind::Down(MouseButton::Left), 30, 5);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Ignored));
        // Focus unchanged.
        assert_eq!(tree.focus(), Some(0));
    }

    #[test]
    fn mouse_wheel_down_increments_carousel_scroll() {
        // Vertical carousel id=99, 5 children × 10 rows each; scroll starts at 5.
        let area = Rect::new(0, 0, 20, 10);
        let mut tree = Tree::new(v_carousel(99, 5, 10, 5));
        let mut router = InputRouter::new();

        let ev = make_mouse(MouseEventKind::ScrollDown, 10, 5);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Scrolled(99)));
        if let Some(Node::Carousel { scroll, .. }) = node_by_id(tree.root(), 99) {
            assert_eq!(*scroll, 6, "scroll should have incremented by 1");
        } else {
            panic!("carousel not found");
        }
    }

    #[test]
    fn mouse_wheel_up_decrements_carousel_scroll_saturating() {
        let area = Rect::new(0, 0, 20, 10);
        let mut tree = Tree::new(v_carousel(99, 3, 10, 5));
        let mut router = InputRouter::new();

        let ev = make_mouse(MouseEventKind::ScrollUp, 10, 5);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Scrolled(99)));
        if let Some(Node::Carousel { scroll, .. }) = node_by_id(tree.root(), 99) {
            assert_eq!(*scroll, 2, "scroll should have decremented by 1");
        } else {
            panic!("carousel not found");
        }
    }

    #[test]
    fn mouse_wheel_up_at_zero_saturates_not_wraps() {
        let area = Rect::new(0, 0, 20, 10);
        let mut tree = Tree::new(v_carousel(99, 0, 10, 5));
        let mut router = InputRouter::new();

        let ev = make_mouse(MouseEventKind::ScrollUp, 10, 5);
        router.handle_mouse(ev, &mut tree, area);
        if let Some(Node::Carousel { scroll, .. }) = node_by_id(tree.root(), 99) {
            assert_eq!(*scroll, 0, "scroll must not wrap below 0");
        } else {
            panic!("carousel not found");
        }
    }

    #[test]
    fn mouse_wheel_with_no_carousel_under_cursor_returns_ignored() {
        // Pure H-split — no carousel anywhere in the tree.
        let area = Rect::new(0, 0, 40, 10);
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1)]));
        let mut router = InputRouter::new();

        let ev = make_mouse(MouseEventKind::ScrollDown, 10, 5);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Ignored));
    }

    #[test]
    fn mouse_click_zoom_aware_outside_zoom_returns_ignored() {
        // H-split [Tile(0) | Tile(1)]; zoom into Tile(1).
        // In the zoomed view, the effective root is Tile(1) filling the whole area.
        // A click at x=5 would hit Tile(0) in the unzoomed tree but should return
        // Ignored from handle_mouse since effective_root is just Tile(1), and
        // solve([Tile(1)], area) = [(1, area)] — (5,5) is inside area → Focused(1).
        // Actually zoomed into Tile(1) means the whole area resolves to Tile(1).
        // Let's instead test that a point outside the area returns Ignored.
        let area = Rect::new(5, 5, 30, 20); // non-zero origin to test containment
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1)]));
        tree.focus_set(1);
        tree.zoom_focus(); // effective root = Tile(1)
        let mut router = InputRouter::new();

        // Click at (0,0) — outside the area rect (area starts at (5,5)).
        let ev = make_mouse(MouseEventKind::Down(MouseButton::Left), 0, 0);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Ignored), "out-of-area click must be Ignored");
    }

    #[test]
    fn mouse_click_zoom_aware_in_area_hits_zoomed_tile() {
        // When zoomed into Tile(1), solve on effective root in area yields [(1, area)].
        // Any in-area click therefore focuses Tile(1).
        let area = Rect::new(0, 0, 40, 10);
        let mut tree = Tree::new(h_split(vec![tile(0), tile(1)]));
        tree.focus_set(1);
        tree.zoom_focus();
        let mut router = InputRouter::new();

        // Click in the left portion — would be Tile(0) unzoomed, but Tile(1) when zoomed.
        let ev = make_mouse(MouseEventKind::Down(MouseButton::Left), 5, 5);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Focused(1)));
    }

    #[test]
    fn mouse_wheel_ignored_when_zoomed_to_single_tile() {
        // Zoom into a leaf tile — no carousel exists in the effective subtree.
        let area = Rect::new(0, 0, 20, 10);
        let mut tree = Tree::new(v_carousel(1, 0, 5, 4));
        tree.zoom_focus(); // focus is Tile(0); effective root becomes Tile(0)
        let mut router = InputRouter::new();

        let ev = make_mouse(MouseEventKind::ScrollDown, 10, 5);
        let outcome = router.handle_mouse(ev, &mut tree, area);
        assert!(matches!(outcome, MouseOutcome::Ignored));
    }

    #[test]
    fn wheel_scroll_step_is_respected() {
        let area = Rect::new(0, 0, 20, 10);
        let mut tree = Tree::new(v_carousel(99, 0, 5, 10));
        let mut router = InputRouter::new();
        router.set_wheel_scroll_step(3);

        let ev = make_mouse(MouseEventKind::ScrollDown, 10, 5);
        router.handle_mouse(ev, &mut tree, area);
        if let Some(Node::Carousel { scroll, .. }) = node_by_id(tree.root(), 99) {
            assert_eq!(*scroll, 3, "step=3 should advance scroll by 3");
        } else {
            panic!("carousel not found");
        }
    }
}
