//! What Obsidian means by the files in a vault: notes, properties, headings
//! and sections, links and embeds, tags, aliases and block ids, and how a
//! name, alias or path reaches a note. Every answer comes from the files on
//! disk at the moment of the call.

mod edit;
mod files;
mod moves;
mod note;
mod parse;
mod path;
mod vault;

pub use note::{
    Alias, BlockId, Heading, Link, LinkKind, Note, Properties, Property, PropertyValue, Section,
    SectionLookup, Tag,
};
pub use files::{TmpPlacement, WriteError};
pub use edit::{PropertyEditError, SectionEdit, new_note_text};
pub use moves::{FileRewrite, MovePlan, MoveError};
pub use path::{PathError, VaultPath};
pub use vault::{NoteLookup, ResolvedLink, Snapshot, Vault};
