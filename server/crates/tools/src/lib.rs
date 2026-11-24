//! How an agent talks to the vault: each tool as a plain function with its
//! own argument and result types, the 8 KB reply limit and paging, the
//! stale-edit check, and the wording of errors. What any of it means in
//! Obsidian is the `vault` crate's business.

mod error;
mod find;
mod links;
mod paging;
mod read;
mod specs;
mod views;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use vault::{Snapshot, Vault};

pub use error::{ErrorKind, ToolError};
pub use find::{FindNotesArgs, FindNotesResult, FoundNote, PropertyFilter, Snippet, find_notes};
pub use links::{
    BrokenLink, CheckVaultArgs, CheckVaultResult, Direction, GetLinksArgs, GetLinksResult, check_vault, get_links,
};
pub use paging::REPLY_LIMIT;
pub use read::{OutlineEntry, ReadMode, ReadNoteArgs, ReadNoteResult, read_note};
pub use specs::{ToolSpec, specs};
pub use views::{LinkView, NoteRef};

/// What a tool call gives back: one JSON text, and whether it is an error.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: String,
    pub is_error: bool,
}

/// The tools over one vault. Every call reads the vault afresh.
#[derive(Debug, Clone)]
pub struct Tools {
    vault: Vault,
}

impl Tools {
    pub fn new(vault: Vault) -> Tools {
        Tools { vault }
    }

    /// Runs the tool `name` with JSON `arguments` (`None` counts as `{}`).
    pub fn call(&self, name: &str, arguments: Option<Value>) -> Reply {
        let args = arguments.unwrap_or_else(|| Value::Object(Default::default()));
        let result = match name {
            "find_notes" => self.run(args, find_notes),
            "read_note" => self.run(args, read_note),
            "get_links" => self.run(args, get_links),
            "check_vault" => self.run(args, check_vault),
            "create_note" | "edit_section" | "set_property" | "move_note" | "delete_note" => Err(ToolError::invalid(
                format!("`{name}` is not available on this server yet. Only find_notes, read_note, get_links and check_vault are."),
            )),
            other => Err(ToolError::invalid(format!(
                "There is no tool `{other}`. The tools are find_notes, read_note, get_links and check_vault."
            ))),
        };
        match result {
            Ok(text) => Reply { text, is_error: false },
            Err(e) => Reply { text: e.to_json().to_string(), is_error: true },
        }
    }

    fn run<A, R>(&self, args: Value, tool: fn(&Snapshot, &A) -> Result<R, ToolError>) -> Result<String, ToolError>
    where
        A: DeserializeOwned,
        R: Serialize,
    {
        let args: A = serde_json::from_value(args)
            .map_err(|e| ToolError::invalid(format!("The arguments do not fit this tool: {e}. See the tool's input schema.")))?;
        let snap = self.vault.snapshot().map_err(ToolError::io)?;
        let result = tool(&snap, &args)?;
        Ok(serde_json::to_string(&result).expect("tool results serialize"))
    }
}
