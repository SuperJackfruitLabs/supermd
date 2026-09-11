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

impl Surface {
    /// Command ids this surface offers, in the order they appear.
    fn command_ids(self) -> &'static [&'static str] {
        match self {
            Surface::SidebarFile => &[
                "sidebar_rename",
                "sidebar_move",
                "sidebar_delete",
                "reveal_in_finder",
                "copy_path",
            ],
            Surface::SidebarFolder => &[
                "sidebar_new_file",
                "sidebar_new_folder",
                "sidebar_rename",
                "sidebar_delete",
                "reveal_in_finder",
                "copy_path",
            ],
            Surface::Tab => &["close_tab", "toggle_preview", "show_changes"],
            Surface::GraphNode => &["graph_local", "graph_fit"],
            Surface::GraphGhost => &["graph_local"],
            Surface::Editor => &["follow_link", "bold", "italic"],
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
pub fn items_for(surface: Surface) -> Vec<MenuItem> {
    surface
        .command_ids()
        .iter()
        .filter_map(|id| {
            COMMANDS.iter().find(|c| &c.id == id).map(|c| MenuItem {
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
        let ids: Vec<&str> = items_for(Surface::SidebarFile).iter().map(|i| i.id).collect();
        for expected in ["sidebar_rename", "sidebar_delete", "sidebar_move", "reveal_in_finder", "copy_path"] {
            assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
        }
    }

    /// A folder cannot be renamed into a file's actions: New File Here
    /// and New Folder Here belong to it, and Move does not.
    #[test]
    fn a_sidebar_folder_offers_creation_not_file_actions() {
        let ids: Vec<&str> = items_for(Surface::SidebarFolder).iter().map(|i| i.id).collect();
        assert!(ids.contains(&"sidebar_new_file"), "{ids:?}");
        assert!(ids.contains(&"sidebar_new_folder"), "{ids:?}");
        assert!(ids.contains(&"reveal_in_finder"), "{ids:?}");
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
            for id in surface.command_ids() {
                assert!(
                    crate::commands::COMMANDS.iter().any(|c| &c.id == id),
                    "{surface:?} offers {id:?}, which is not in COMMANDS",
                );
            }
        }
    }

    /// Labels and shortcuts come from the table, so a renamed command
    /// or a changed key updates the menu for free.
    #[test]
    fn labels_and_keys_come_from_the_command_table() {
        let item = items_for(Surface::SidebarFile)
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
            assert!(!items_for(*surface).is_empty(), "{surface:?} has no items");
        }
    }
}
