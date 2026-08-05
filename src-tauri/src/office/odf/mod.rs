//! OpenDocument (odt/ods/odp) rendering.
//!
//! ## Package layout
//!
//! An ODF package is a zip whose parts live at *fixed* paths, so there is no
//! `[Content_Types].xml`, no `_rels` graph, and no use for `office::opc`:
//!
//! - `mimetype` — required, uncompressed, first member; the document's media type.
//! - `META-INF/manifest.xml` — every part with its media type, and, for a
//!   password-protected document, the `manifest:encryption-data` that says the
//!   part is ciphertext.
//! - `content.xml` — `office:body`, plus the *automatic* (one-off) styles.
//! - `styles.xml` — the named styles, list and number styles, page layouts and
//!   master pages. ODF's real stylesheet: a document without it loses its
//!   formatting, not just its overrides.
//! - `meta.xml`, `settings.xml` — document properties and view state.
//! - `Pictures/…` — embedded media, referenced by `xlink:href`.
//! - `Object N/`, `ObjectReplacements/Object N` — embedded charts and OLE
//!   objects, stored as a whole sub-document plus a pre-rendered replacement
//!   image. Sub-documents are **never** rendered recursively; the shape adapter
//!   draws the replacement image, or a placeholder when there is none.
//!
//! Prefixes (`office:`, `fo:`, `text:`) are conventional, not guaranteed, so
//! every lookup goes through the local-name helpers in `office::xml`.
//!
//! ## Where the rest lands
//!
//! - [`pkg`] — opening a package, the encryption check, `xlink:href` validation,
//!   and the document class.
//! - [`length`] — measurements, the `fo:border` shorthand, colour values.
//! - Style cascade — the `style:style` graph (`style:parent-style-name`, with the
//!   automatic styles from `content.xml` layered over the named styles from
//!   `styles.xml`) resolved into the property bags the renderers read.
//! - List and number styles — `text:list-style` and `number:*-style`, feeding
//!   `office::listnum` and `office::numfmt`.
//! - Three shape adapters — one per document class, each mapping ODF's
//!   `draw:frame` geometry onto the same emission scaffolding (`office::emit`,
//!   `office::html`) the OOXML renderers already use.
//!
//! ## Dispatch
//!
//! [`render`] is the one entry point for all three classes: the package is opened
//! once, and [`pkg::Class`] — read from `office:body`'s child, not from the file
//! extension — picks the renderer. A `.odt` holding `office:spreadsheet` therefore
//! renders as the spreadsheet it is rather than as a text document with no
//! paragraphs.
//!
//! [`text`] is live; the spreadsheet and presentation arms are filled in by later
//! passes and until then say so rather than rendering an empty page.
//!
//! ## What the classes share
//!
//! [`style`], [`list`], [`length`], [`numstyle`] and [`pkg`] are class-agnostic by
//! construction. [`draw`] and [`table`] are the odt spellings of a frame and a
//! table: the geometry vocabulary is shared with odp and ods, but the *placement*
//! is not — a text document's frame joins a reflowing line, a slide's is absolute —
//! so a later pass reuses the pieces it needs rather than the emitters whole.

mod draw;
mod length;
mod list;
mod numstyle;
mod pkg;
mod sheet;
mod slide;
mod style;
mod table;
mod text;

use super::emit::{self, Notes};
use super::OfficeDoc;

/// Renders one section of an ODF package.
pub fn render(path: &str, section: Option<u32>, terms: &[String]) -> Result<OfficeDoc, String> {
    // Notes start before the package does: `pkg::open` reports a missing
    // `styles.xml` and the other optional parts, and those degradations belong in
    // the same footer as the renderer's own.
    let mut notes = Notes::new();
    let package = pkg::open(path, &mut notes).map_err(fatal)?;
    match package.class {
        pkg::Class::Text => text::render(package, notes, section, terms),
        pkg::Class::Spreadsheet => sheet::render(package, notes, section, terms),
        pkg::Class::Presentation => slide::render(package, notes, section, terms),
    }
}

/// An error that cost the whole document, as something a reader can act on.
///
/// The two marker errors are the ones with a cause worth naming: a
/// password-protected package has nothing to render without the password, and a
/// budget stop means the package is far larger than the document in it. Everything
/// else is already a sentence about this file.
fn fatal(err: String) -> String {
    if err == pkg::ENCRYPTED {
        "This document is password-protected: the preview cannot open it.".to_string()
    } else if err == super::pkg::BUDGET_EXCEEDED {
        emit::degrade_msg(&err, "document")
    } else {
        err
    }
}
