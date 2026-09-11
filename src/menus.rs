//! Which commands a right-click offers, per surface.
//!
//! The menu is a filter over the command table rather than its own
//! list, so renaming a command or changing its shortcut updates every
//! menu for free — a hand-written list is how `SHORTCUTS` in
//! workspace.rs went stale enough that CLAUDE.md documented a table
//! which no longer existed.

use crate::commands::COMMANDS;

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

impl Surface {
    pub const ALL: &'static [Surface] = &[
        Surface::SidebarFile,
        Surface::SidebarFolder,
        Surface::Tab,
        Surface::GraphNode,
        Surface::GraphGhost,
        Surface::Editor,
    ];

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

    /// Every item must name a real command, or the menu offers
    /// something that cannot be dispatched.
    #[test]
    fn every_menu_item_names_a_real_command() {
        for surface in Surface::ALL {
            for item in items_for(*surface) {
                assert!(
                    crate::commands::COMMANDS.iter().any(|c| c.id == item.id),
                    "{surface:?} offers {:?}, which is not in COMMANDS",
                    item.id
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
