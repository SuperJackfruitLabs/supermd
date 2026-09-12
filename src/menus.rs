//! Which commands a right-click offers, per surface.
//!
//! The menu is a filter over the command table rather than its own
//! list, so renaming a command or changing its shortcut updates every
//! menu for free — a hand-written list is how `SHORTCUTS` in
//! workspace.rs went stale enough that CLAUDE.md documented a table
//! which no longer existed.

use crate::commands::COMMANDS;

/// Declares `Surface` and `Surface::ALL` from one variant list, so a
/// variant cannot be added to the enum without also landing in `ALL`
/// — there is no second place to forget it. Before this, `ALL` was a
/// hand-written array beside the enum: adding a variant only forced a
/// `command_ids` match arm (the match is exhaustive over `Surface`),
/// nothing forced the variant into `ALL`, and every test that walks
/// `Surface::ALL` — including `every_menu_item_names_a_real_command`
/// — would have silently skipped it. Fusing the two removes the
/// second list rather than merely testing that it stayed in sync.
macro_rules! surfaces {
    ($(#[$doc:meta])* pub enum Surface { $($variant:ident),* $(,)? }) => {
        $(#[$doc])*
        pub enum Surface {
            $($variant),*
        }

        impl Surface {
            /// Every surface, in declaration order. Derived from the
            /// same variant list as the enum above — see `surfaces!`.
            pub const ALL: &'static [Surface] = &[$(Surface::$variant),*];
        }
    };
}

surfaces! {
    /// Where a right-click happened.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Surface {
        SidebarFile,
        SidebarFolder,
        Tab,
        GraphNode,
        GraphGhost,
        Editor,
    }
}

/// What the editor knew at the point a right-click landed. Every
/// field is a plain fact the GPUI layer can read off the document;
/// which items those facts earn is decided here, so the rule is pure
/// and under test and `editor/mod.rs` holds no item list.
///
/// The facts are taken at the caret *after* the click has applied its
/// cursor policy (a click outside the selection moves the caret to it,
/// a click inside preserves the selection). That is deliberate: every
/// command this menu offers acts on `selection.head`, so reading the
/// facts from the same place is what makes "the menu offers it" and
/// "the command will do something" the same condition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditorContext {
    /// The caret is in a table cell — `table_cursor()`'s own test, and
    /// so the precondition of all four table commands.
    pub in_table: bool,
    /// The caret's run of non-blank lines holds an ordered-list item —
    /// Renumber List's precondition (a fenced code block's numbers do
    /// not count, exactly as `lists::renumber_block` does not count
    /// them).
    pub in_ordered_list: bool,
    /// The caret is on a link.
    pub on_link: bool,
    /// This document takes formatting edits at all: Markdown, and not
    /// the read-only diff view. Every editor command is gated on
    /// `can_format()`, so when this is false the menu is empty and the
    /// editor opens none.
    pub can_format: bool,
}

impl EditorContext {
    /// Every fact true — the superset of what the editor menu can ever
    /// offer. `Surface::ALL`'s walkers use it so that no editor id
    /// escapes `every_menu_item_names_a_real_command` merely because no
    /// context in a test happened to earn it.
    pub fn permissive() -> Self {
        Self { in_table: true, in_ordered_list: true, on_link: true, can_format: true }
    }
}

/// Every id the editor menu can offer, in order, each paired with the
/// fact that earns it. One list, not two: `Surface::Editor`'s
/// `command_ids` runs it through the permissive context, so the
/// exhaustiveness tests walk exactly the ids this table holds and a new
/// row cannot be added without them checking it names a real command.
type Gate = fn(&EditorContext) -> bool;
const EDITOR_MENU: &[(&str, Gate)] = &[
    ("follow_link", |c| c.on_link),
    ("bold", |_| true),
    ("italic", |_| true),
    ("table_insert_row", |c| c.in_table),
    ("table_delete_row", |c| c.in_table),
    ("table_insert_column", |c| c.in_table),
    ("table_delete_column", |c| c.in_table),
    ("renumber_list", |c| c.in_ordered_list),
];

impl Surface {
    /// Command ids this surface offers, in the order they appear. Only
    /// `Surface::Editor` reads `ctx`; every other surface offers the
    /// same rows wherever it is clicked.
    fn command_ids(self, ctx: EditorContext) -> Vec<&'static str> {
        match self {
            Surface::SidebarFile => vec![
                "sidebar_rename",
                "sidebar_move",
                "sidebar_delete",
                "reveal_in_finder",
                "copy_path",
            ],
            Surface::SidebarFolder => vec![
                "sidebar_new_file",
                "sidebar_new_folder",
                "sidebar_rename",
                "sidebar_delete",
                "reveal_in_finder",
                "copy_path",
            ],
            Surface::Tab => vec!["close_tab", "toggle_preview", "show_changes"],
            Surface::GraphNode => vec!["graph_local", "graph_fit"],
            Surface::GraphGhost => vec!["graph_local"],
            Surface::Editor => {
                if !ctx.can_format {
                    return Vec::new();
                }
                EDITOR_MENU
                    .iter()
                    .filter(|(_, earns)| earns(&ctx))
                    .map(|(id, _)| *id)
                    .collect()
            }
        }
    }
}

/// One row of a context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: &'static str,
    pub label: &'static str,
    /// Shortcut as written in the command table, or "" when unbound.
    pub keys: &'static str,
}

/// The menu for a surface, resolved against the command table. An id
/// with no matching command is dropped rather than shown as a dead row.
///
/// Belt-and-braces, not a gap-filler: `every_menu_item_names_a_real_command`
/// walks every `command_ids()` id for every `Surface::ALL` entry, and
/// `Surface::ALL` is now derived from the same variant list as the enum
/// (`surfaces!`, above), so there is no id and no surface this function
/// can be called with that the test does not already check on every
/// `cargo test`. The filter has no live failure mode left to catch — it
/// stays because a `String`-vs-`&'static str` id match costs nothing at
/// this size, and "an unrecognised id renders nothing" is a cheaper
/// invariant to keep true by construction than to keep re-justifying.
pub fn items_for(surface: Surface, ctx: EditorContext) -> Vec<MenuItem> {
    surface
        .command_ids(ctx)
        .iter()
        .filter_map(|id| {
            COMMANDS.iter().find(|c| c.id == *id).map(|c| MenuItem {
                id: c.id,
                label: c.label,
                keys: c.keys.first().copied().unwrap_or(""),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sidebar's actions all exist already and are keyboard-only.
    /// A right-click is where a Mac user looks for them first.
    #[test]
    fn a_sidebar_file_offers_the_file_actions() {
        let ids: Vec<&str> = items_for(Surface::SidebarFile, EditorContext::default()).iter().map(|i| i.id).collect();
        for expected in ["sidebar_rename", "sidebar_delete", "sidebar_move", "reveal_in_finder", "copy_path"] {
            assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
        }
    }

    /// A folder cannot be renamed into a file's actions: New File Here
    /// and New Folder Here belong to it, and Move does not.
    #[test]
    fn a_sidebar_folder_offers_creation_not_file_actions() {
        let ids: Vec<&str> = items_for(Surface::SidebarFolder, EditorContext::default()).iter().map(|i| i.id).collect();
        assert!(ids.contains(&"sidebar_new_file"), "{ids:?}");
        assert!(ids.contains(&"sidebar_new_folder"), "{ids:?}");
        assert!(ids.contains(&"reveal_in_finder"), "{ids:?}");
    }

    /// Ids offered for one editor click, for readability in the tests
    /// below.
    fn editor_ids(ctx: EditorContext) -> Vec<&'static str> {
        items_for(Surface::Editor, ctx).into_iter().map(|i| i.id).collect()
    }

    /// The bug this menu exists for: a user clicks into a table, gets
    /// the raw Markdown as designed, and has no way to add or remove a
    /// row — the five commands shipped with `keys: []`, reachable only
    /// from the Format menu. A right-click in a table must offer all
    /// four table commands.
    #[test]
    fn a_click_in_a_table_offers_the_table_commands() {
        let ids = editor_ids(EditorContext { in_table: true, can_format: true, ..Default::default() });
        for expected in [
            "table_insert_row",
            "table_delete_row",
            "table_insert_column",
            "table_delete_column",
        ] {
            assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
        }
        assert!(!ids.contains(&"renumber_list"), "a table is not an ordered list: {ids:?}");
    }

    /// Renumber List belongs to an ordered list and nothing else.
    #[test]
    fn a_click_in_an_ordered_list_offers_renumber() {
        let ids = editor_ids(EditorContext {
            in_ordered_list: true,
            can_format: true,
            ..Default::default()
        });
        assert!(ids.contains(&"renumber_list"), "{ids:?}");
        assert!(!ids.contains(&"table_insert_row"), "no table here: {ids:?}");
    }

    /// Follow Link is the one row that depends on what is under the
    /// pointer rather than on the block around it.
    #[test]
    fn follow_link_appears_only_on_a_link() {
        let on = editor_ids(EditorContext { on_link: true, can_format: true, ..Default::default() });
        assert!(on.contains(&"follow_link"), "{on:?}");
        let off = editor_ids(EditorContext { can_format: true, ..Default::default() });
        assert!(!off.contains(&"follow_link"), "prose is not a link: {off:?}");
    }

    /// Ordinary prose gets the two formatting toggles and nothing that
    /// would be a dead row: a menu full of commands that do nothing is
    /// how the Format menu already failed this user.
    #[test]
    fn prose_offers_only_the_formatting_toggles() {
        let ids = editor_ids(EditorContext { can_format: true, ..Default::default() });
        assert_eq!(ids, vec!["bold", "italic"], "prose offers exactly the toggles");
    }

    /// Every command in the editor menu is gated on `can_format()` —
    /// a code file or the read-only diff view takes none of them, so
    /// the menu is empty and (see `editor/mod.rs`) never opens.
    #[test]
    fn a_document_that_takes_no_edits_offers_nothing() {
        for ctx in [
            EditorContext { can_format: false, ..EditorContext::permissive() },
            EditorContext::default(),
        ] {
            assert!(editor_ids(ctx).is_empty(), "{ctx:?} still offered rows");
        }
    }

    /// Every id in `EDITOR_MENU` is reachable: a gate that can never be
    /// true is a row the user can never see, and the exhaustiveness
    /// tests above (which use the permissive context) would not notice.
    /// Checked against the real gate, one fact at a time.
    #[test]
    fn every_editor_row_is_reachable_from_some_click() {
        let each = [
            EditorContext { can_format: true, ..Default::default() },
            EditorContext { in_table: true, can_format: true, ..Default::default() },
            EditorContext { in_ordered_list: true, can_format: true, ..Default::default() },
            EditorContext { on_link: true, can_format: true, ..Default::default() },
        ];
        for (id, _) in EDITOR_MENU {
            assert!(
                each.iter().any(|ctx| editor_ids(*ctx).contains(id)),
                "{id} can never appear in the editor menu",
            );
        }
    }

    /// Every id in `command_ids` must name a real command. This checks
    /// the *source* list, not `items_for`'s output: `items_for` filters
    /// out ids that fail to resolve, so asserting over its result is a
    /// tautology (every returned item was, by construction, a matched
    /// `COMMANDS` entry) and would pass no matter what a surface's
    /// `command_ids` says. Walking `command_ids` directly is the only
    /// way a typo'd id can fail this test.
    #[test]
    fn every_menu_item_names_a_real_command() {
        for surface in Surface::ALL {
            for id in surface.command_ids(EditorContext::permissive()) {
                assert!(
                    crate::commands::COMMANDS.iter().any(|c| c.id == id),
                    "{surface:?} offers {id:?}, which is not in COMMANDS",
                );
            }
        }
    }

    /// Labels and shortcuts come from the table, so a renamed command
    /// or a changed key updates the menu for free.
    #[test]
    fn labels_and_keys_come_from_the_command_table() {
        let item = items_for(Surface::SidebarFile, EditorContext::default())
            .into_iter()
            .find(|i| i.id == "sidebar_rename")
            .expect("rename is offered");
        let cmd = crate::commands::COMMANDS.iter().find(|c| c.id == "sidebar_rename").unwrap();
        assert_eq!(item.label, cmd.label);
        assert!(!item.keys.is_empty(), "a bound command shows its shortcut");
    }

    /// No surface offers nothing: an empty menu is worse than none.
    #[test]
    fn no_surface_is_empty() {
        for surface in Surface::ALL {
            assert!(
                !items_for(*surface, EditorContext::permissive()).is_empty(),
                "{surface:?} has no items"
            );
        }
    }
}
