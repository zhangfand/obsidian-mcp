use serde::{Deserialize, Serialize};
use vault::Snapshot;

use crate::error::{ToolError, found};
use crate::paging::{Cursor, fit_items};
use crate::views::LinkView;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Out,
    In,
    #[default]
    Both,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetLinksArgs {
    pub note: String,
    #[serde(default)]
    pub direction: Direction,
    #[serde(skip_serializing)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GetLinksResult {
    pub path: String,
    pub outgoing: Vec<LinkView>,
    pub incoming: Vec<LinkView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<String>,
}

pub fn get_links(snap: &Snapshot, args: &GetLinksArgs) -> Result<GetLinksResult, ToolError> {
    let cursor = Cursor::new(
        "get_links",
        &serde_json::to_value(args).unwrap_or_default(),
        args.page.as_deref(),
    )?;
    let note = found(&args.note, snap.lookup(&args.note))?;
    let outgoing: Vec<LinkView> = if args.direction == Direction::In {
        Vec::new()
    } else {
        snap.outgoing(note).iter().map(LinkView::from).collect()
    };
    let incoming: Vec<LinkView> = if args.direction == Direction::Out {
        Vec::new()
    } else {
        snap.incoming(note).iter().map(LinkView::from).collect()
    };
    let (n_out, total) = (outgoing.len(), outgoing.len() + incoming.len());
    Ok(fit_items(&cursor, total, |s, e, next_page| {
        GetLinksResult {
            path: note.path.to_string(),
            outgoing: outgoing[s.min(n_out)..e.min(n_out)].to_vec(),
            incoming: incoming[s.max(n_out) - n_out..e.max(n_out) - n_out].to_vec(),
            next_page,
        }
    }))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckVaultArgs {
    #[serde(skip_serializing)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BrokenLink {
    #[serde(flatten)]
    pub link: LinkView,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CheckVaultResult {
    pub broken_links: Vec<BrokenLink>,
    pub total_broken_links: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page: Option<String>,
}

pub fn check_vault(snap: &Snapshot, args: &CheckVaultArgs) -> Result<CheckVaultResult, ToolError> {
    let cursor = Cursor::new(
        "check_vault",
        &serde_json::to_value(args).unwrap_or_default(),
        args.page.as_deref(),
    )?;
    let broken: Vec<BrokenLink> = snap
        .broken_links()
        .iter()
        .map(|r| BrokenLink {
            link: LinkView::from(r),
            reason: if !r.ambiguous.is_empty() {
                "ambiguous"
            } else if r.targets_attachment() {
                "missing_file"
            } else {
                "missing_note"
            },
        })
        .collect();
    let total = broken.len();
    Ok(fit_items(&cursor, total, |s, e, next_page| {
        CheckVaultResult {
            broken_links: broken[s..e].to_vec(),
            total_broken_links: total,
            next_page,
        }
    }))
}
