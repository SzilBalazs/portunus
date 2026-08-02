#![allow(dead_code)]

pub mod color;
pub mod fill;
pub mod geom;
pub mod line;
pub mod theme;

// Node lookup shared by the colour/fill/line/geom parsers. Only the *local* name
// is matched: the `a:` prefix is conventional, not guaranteed, and theme override
// parts bind the DrawingML namespace to a different prefix.

/// First element child with this local name.
pub fn child_elem<'a>(
    node: roxmltree::Node<'a, 'a>,
    local: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == local)
}

/// Element children in document order. Transform and stop lists are
/// order-sensitive, so callers must never collect these into a set.
pub fn elems<'a>(
    node: roxmltree::Node<'a, 'a>,
) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> + 'a {
    node.children().filter(|n| n.is_element())
}
