use core::borrow::Borrow;
use core::cmp::Ordering;

use super::node::{Handle, NodeRef, marker, ForceResult::*};

use SearchResult::*;

use crate::alloc::Alloc;
use core::fmt::Debug;

pub enum SearchResult<BorrowType, K, V, FoundType, GoDownType, A> where A: Alloc + Default, A::Err: Debug {
    Found(Handle<NodeRef<BorrowType, K, V, FoundType, A>, marker::KV>),
    GoDown(Handle<NodeRef<BorrowType, K, V, GoDownType, A>, marker::Edge>)
}

pub fn search_tree<BorrowType, K, V, Q: ?Sized, A>(
    mut node: NodeRef<BorrowType, K, V, marker::LeafOrInternal, A>,
    key: &Q
) -> SearchResult<BorrowType, K, V, marker::LeafOrInternal, marker::Leaf, A>
        where Q: Ord, K: Borrow<Q>, A: Alloc + Default, A::Err: Debug {

    loop {
        match search_node(node, key) {
            Found(handle) => return Found(handle),
            GoDown(handle) => match handle.force() {
                Leaf(leaf) => return GoDown(leaf),
                Internal(internal) => {
                    node = internal.descend();
                    continue;
                }
            }
        }
    }
}

pub fn search_node<BorrowType, K, V, Type, Q: ?Sized, A>(
    node: NodeRef<BorrowType, K, V, Type, A>,
    key: &Q
) -> SearchResult<BorrowType, K, V, Type, Type, A>
        where Q: Ord, K: Borrow<Q>, A: Alloc + Default, A::Err: Debug {

    match search_linear(&node, key) {
        (idx, true) => Found(
            Handle::new_kv(node, idx)
        ),
        (idx, false) => SearchResult::GoDown(
            Handle::new_edge(node, idx)
        )
    }
}

pub fn search_linear<BorrowType, K, V, Type, Q: ?Sized, A>(
    node: &NodeRef<BorrowType, K, V, Type, A>,
    key: &Q
) -> (usize, bool)
        where Q: Ord, K: Borrow<Q>, A: Alloc + Default, A::Err: Debug {

    for (i, k) in node.keys().iter().enumerate() {
        match key.cmp(k.borrow()) {
            Ordering::Greater => {},
            Ordering::Equal => return (i, true),
            Ordering::Less => return (i, false)
        }
    }
    (node.keys().len(), false)
}
