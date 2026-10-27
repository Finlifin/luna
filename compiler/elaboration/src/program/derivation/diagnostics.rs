//! Add source provenance without guessing which identical expansion failed.
use super::*;
use crate::program::{DiagnosticNote, source};

fn contains(span: rustc_span::Span, range: nir::program::SourceRange) -> bool {
    let outer = source(span);
    outer.start < outer.end
        && range.start <= range.end
        && outer.start <= range.start
        && range.end <= outer.end
}

fn selected(source: &Package<'_>, request: &hir::item::DeriveRequest<'_>) -> Option<OwnerId> {
    let E::ResolvedDef { def_id, .. } = request.strategy?.kind else {
        return None;
    };
    let owner = source.definition_owner(def_id)?;
    matches!(source.item(owner)?.kind, ItemKind::Deriver(_)).then_some(owner)
}

pub(in crate::program) fn with_context(package: &Package<'_>, mut failure: Error) -> Error {
    // Diagnostics must not turn an exhausted expansion into unbounded work.
    let mut fuel = 100_000usize;
    let mut strategies = BTreeSet::new();
    for (owner, info) in package.owners() {
        let Some(remaining) = fuel.checked_sub(1) else {
            return failure;
        };
        fuel = remaining;
        let item = info.node.expect_item();
        if !contains(item.span, failure.source) {
            continue;
        }
        match &item.kind {
            ItemKind::Deriver(_) => {
                strategies.insert(owner);
            }
            ItemKind::Derive(request) => {
                if let Some(strategy) = selected(package, request) {
                    strategies.insert(strategy);
                }
            }
            _ => {}
        }
    }
    if strategies.is_empty() {
        return failure;
    }
    let mut notes = Vec::new();
    for strategy in &strategies {
        let item = package.item(*strategy).unwrap();
        notes.push(DiagnosticNote {
            source: source(item.span),
            message: format!("derivation strategy `{}` defined here", item.ident.name),
        });
    }
    for (owner, info) in package.owners() {
        let Some(remaining) = fuel.checked_sub(1) else {
            break;
        };
        fuel = remaining;
        if notes.len() >= 64 {
            notes.push(DiagnosticNote {
                source: failure.source,
                message: "further derivation source notes omitted".into(),
            });
            break;
        }
        let item = info.node.expect_item();
        if package
            .lexical_parents
            .get(&owner)
            .is_some_and(|parent| strategies.contains(parent))
            && contains(item.span, failure.source)
        {
            notes.push(DiagnosticNote {
                source: source(item.span),
                message: format!("generated member `{}` originates here", item.ident.name),
            });
        }
        if let ItemKind::Derive(request) = &item.kind
            && let Some(strategy) = selected(package, request)
            && strategies.contains(&strategy)
        {
            notes.push(DiagnosticNote {
                source: source(item.span),
                message: format!(
                    "derive use site of strategy `{}`",
                    package.item(strategy).unwrap().ident.name
                ),
            });
        }
    }
    failure.notes.extend(notes);
    failure
}
