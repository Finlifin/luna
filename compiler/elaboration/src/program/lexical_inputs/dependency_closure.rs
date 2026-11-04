//! Close source dependencies without confusing a callee's binders with captures.
use super::*;
use std::collections::VecDeque;

pub(super) struct ScopeReads<K, B> {
    pub bindings: BTreeSet<B>,
    pub direct: BTreeSet<B>,
    pub callees: BTreeSet<K>,
}

impl<K, B> Default for ScopeReads<K, B> {
    fn default() -> Self {
        Self {
            bindings: BTreeSet::new(),
            direct: BTreeSet::new(),
            callees: BTreeSet::new(),
        }
    }
}

fn charge(fuel: &mut usize, span: Span) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "lexical dependency closure budget exhausted"))?;
    Ok(())
}

/// Explicit argument expressions belong to the caller. Only free inputs flow
/// back from a referenced declaration. Monotone propagation closes recursive
/// groups without expanding their bodies repeatedly or publishing partial sets.
pub(super) fn close<K: Copy + Ord, B: Copy + Ord>(
    graph: &BTreeMap<K, ScopeReads<K, B>>,
    root: K,
    fuel: &mut usize,
    span: Span,
) -> Result<BTreeSet<B>, Error> {
    let mut reverse = BTreeMap::<K, BTreeSet<K>>::new();
    let mut needed = BTreeMap::<K, BTreeSet<B>>::new();
    for (&owner, reads) in graph {
        for input in &reads.direct {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "lexical dependency closure budget exhausted"))?;
            if !reads.bindings.contains(input) {
                needed.entry(owner).or_default().insert(*input);
            }
        }
        needed.entry(owner).or_default();
        for callee in &reads.callees {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "lexical dependency closure budget exhausted"))?;
            if !graph.contains_key(callee) {
                return Err(error(span, "lexical callee observation is incomplete"));
            }
            reverse.entry(*callee).or_default().insert(owner);
        }
    }
    let mut queued = needed.keys().copied().collect::<BTreeSet<_>>();
    let mut pending = VecDeque::from_iter(queued.iter().copied());
    while let Some(callee) = pending.pop_front() {
        queued.remove(&callee);
        charge(fuel, span)?;
        *fuel = (*fuel)
            .checked_sub(needed[&callee].len())
            .ok_or_else(|| error(span, "lexical dependency closure budget exhausted"))?;
        let outgoing = needed[&callee].clone();
        for caller in reverse.get(&callee).into_iter().flatten() {
            let mut changed = false;
            for input in &outgoing {
                charge(fuel, span)?;
                if !graph[caller].bindings.contains(input) {
                    changed |= needed.get_mut(caller).unwrap().insert(*input);
                }
            }
            if changed && queued.insert(*caller) {
                pending.push_back(*caller);
            }
        }
    }
    needed
        .remove(&root)
        .ok_or_else(|| error(span, "missing lexical root observation"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_span::DUMMY_SP;

    fn scope(bound: &[u8], direct: &[u8], callees: &[u8]) -> ScopeReads<u8, u8> {
        ScopeReads {
            bindings: bound.iter().copied().collect(),
            direct: direct.iter().copied().collect(),
            callees: callees.iter().copied().collect(),
        }
    }

    #[test]
    fn recursive_captures_exclude_each_declarations_own_inputs() {
        let graph = BTreeMap::from([
            (0, scope(&[1], &[2], &[1])),
            (1, scope(&[3], &[1, 3], &[2])),
            (2, scope(&[4], &[4, 5], &[1])),
        ]);
        // 1 is captured by the recursive group but bound at the caller;
        // 3 and 4 belong to individual callees. Only 2 and 5 escape root 0.
        assert_eq!(
            close(&graph, 0, &mut 1000, DUMMY_SP).unwrap(),
            BTreeSet::from([2, 5])
        );
        assert_eq!(
            close(&graph, 1, &mut 1000, DUMMY_SP).unwrap(),
            BTreeSet::from([1, 5])
        );
    }

    #[test]
    fn an_expression_and_its_referenced_owner_have_distinct_scopes() {
        let graph = BTreeMap::from([(0, scope(&[], &[1], &[1])), (1, scope(&[1], &[1, 2], &[1]))]);
        assert_eq!(
            close(&graph, 0, &mut 1000, DUMMY_SP).unwrap(),
            BTreeSet::from([1, 2])
        );
        assert_eq!(
            close(&graph, 1, &mut 1000, DUMMY_SP).unwrap(),
            BTreeSet::from([2])
        );
    }

    #[test]
    fn exhausted_closure_does_not_publish_partial_dependencies() {
        let graph = BTreeMap::from([(0, scope(&[], &[1], &[1])), (1, scope(&[], &[2], &[]))]);
        let mut fuel = 2;
        let failure = close(&graph, 0, &mut fuel, DUMMY_SP).unwrap_err();
        assert!(failure.to_string().contains("budget exhausted"));
        assert!(fuel < 2);
        assert_eq!(
            close(&graph, 0, &mut 1000, DUMMY_SP).unwrap(),
            BTreeSet::from([1, 2])
        );
    }

    #[test]
    fn incomplete_graph_is_not_an_empty_capture_set() {
        let graph = BTreeMap::from([(0, scope(&[], &[], &[1]))]);
        assert!(
            close(&graph, 0, &mut 1000, DUMMY_SP)
                .unwrap_err()
                .to_string()
                .contains("observation is incomplete")
        );
    }
}
