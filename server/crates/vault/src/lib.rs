//! What Obsidian means by the files in a vault: notes, properties, headings
//! and sections, links and embeds, tags, aliases and block ids, and how a
//! name, alias or path reaches a note. Every answer comes from the files on
//! disk at the moment of the call.

mod note;
mod parse;
mod path;
mod vault;

pub use note::{
    Alias, BlockId, Heading, Link, LinkKind, Note, Properties, Property, PropertyValue, Section,
    SectionLookup, Tag,
};
pub use path::{PathError, VaultPath};
pub use vault::{NoteLookup, ResolvedLink, Snapshot, Vault};
