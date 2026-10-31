//! Complete a success projection when its generic payload acquires an error
//! qualifier. User bodies receive the reconstructed payload through a single
//! initializer; their calls, captures and exits remain unchanged. The projection
//! removes definition-time labels, not qualifiers hidden in its abstract payload.
use super::Conversion;
use crate::Term;
use crate::program::{Expr, ExprKind, MatchArm, Pattern};

/// An ignored success payload needs no reconstructed value. Extend its tag
/// test in place, retaining whole-subject aliases and the surrounding guard/or
/// structure. In particular, a final guard must not run once per alternative.
pub(crate) fn complete_ignored_success_pattern(
    pattern: &mut Pattern,
    known_errors: &[Term],
    payload: &Term,
) {
    let Term::ErrorResult { errors, .. } = payload else {
        return;
    };
    let additional = errors
        .iter()
        .filter(|label| !known_errors.contains(label))
        .collect::<Vec<_>>();
    if additional.is_empty() {
        return;
    }
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        match pattern {
            Pattern::ErrorResult { label: None, inner } if ignores_subject(inner) => {
                let mut restored = std::mem::replace(inner, Box::new(Pattern::Wild));
                let mut leaves = vec![restored.as_mut()];
                while let Some(leaf) = leaves.pop() {
                    match leaf {
                        Pattern::Wild => {
                            let mut alternatives = vec![Pattern::ErrorResult {
                                label: None,
                                inner: Box::new(Pattern::Wild),
                            }];
                            alternatives.extend(additional.iter().map(|label| {
                                Pattern::ErrorResult {
                                    label: Some((*label).clone()),
                                    inner: Box::new(Pattern::Wild),
                                }
                            }));
                            *leaf = Pattern::Or(alternatives);
                        }
                        Pattern::Guard { pattern, .. }
                        | Pattern::Remap { pattern, .. }
                        | Pattern::Then { first: pattern, .. } => leaves.push(pattern),
                        Pattern::Or(alternatives) => leaves.extend(alternatives),
                        _ => {
                            unreachable!("subject-independent pattern was checked before rewriting")
                        }
                    }
                }
                *pattern = *restored;
            }
            Pattern::Binding { inner, .. }
            | Pattern::Observe { inner, .. }
            | Pattern::Guard { pattern: inner, .. }
            | Pattern::Remap { pattern: inner, .. }
            | Pattern::Then { first: inner, .. } => pending.push(inner),
            Pattern::Or(alternatives) => pending.extend(alternatives),
            // Products, error payloads and computed next-patterns have a
            // different subject domain. Do not apply the outer row there.
            _ => {}
        }
    }
}

fn ignores_subject(pattern: &Pattern) -> bool {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        match pattern {
            Pattern::Wild => {}
            Pattern::Guard { pattern, .. }
            | Pattern::Remap { pattern, .. }
            | Pattern::Then { first: pattern, .. } => pending.push(pattern),
            Pattern::Or(alternatives) => pending.extend(alternatives),
            // Binding/Observe aliases expose the payload's type even when
            // their nested test is a wildcard. They need value restoration.
            _ => return false,
        }
    }
    true
}

pub(crate) fn projection_operand(value: &Expr) -> Option<&Expr> {
    conversion_operand(value, false)
}

fn conversion_operand(mut value: &Expr, allow_failure: bool) -> Option<&Expr> {
    loop {
        match &value.kind {
            ExprKind::Local(_) => return Some(value),
            ExprKind::Block {
                statements,
                result: Some(_),
            } if statements.len() == 1 => {
                let ExprKind::LetPattern {
                    value: initializer,
                    guard: None,
                    otherwise: None,
                    ..
                } = &statements[0].kind
                else {
                    return None;
                };
                value = initializer;
            }
            ExprKind::ErrorConvert {
                conversion: Conversion::Success | Conversion::Embed,
                value: inner,
            } => value = inner,
            ExprKind::ErrorConvert {
                conversion: Conversion::Failure,
                value: inner,
            } if allow_failure => value = inner,
            _ => return None,
        }
    }
}

pub(crate) fn complete_success_projection(
    input: &Term,
    known_errors: &[Term],
    arm: &mut MatchArm,
) -> Vec<MatchArm> {
    let Term::ErrorResult { result: leaf, .. } = input else {
        return Vec::new();
    };
    let Pattern::ErrorResult { label: None, inner } = &arm.pattern else {
        return Vec::new();
    };
    let Pattern::Binding { binding, inner } = inner.as_ref() else {
        return Vec::new();
    };
    let Some(operand) = projection_operand(&arm.body) else {
        return Vec::new();
    };
    if !matches!(inner.as_ref(), Pattern::Wild)
        || arm.guard.is_some()
        || !matches!(operand.kind, ExprKind::Local(id) if id == *binding)
    {
        return Vec::new();
    }
    let Term::ErrorResult { errors, result } = &operand.ty else {
        return Vec::new();
    };
    if result != leaf {
        return Vec::new();
    }
    let binding = *binding;
    let target = operand.ty.clone();
    let source = operand.source;
    let template = arm.body.clone();
    let remaining = errors
        .iter()
        .filter(|label| !known_errors.contains(label))
        .cloned()
        .collect::<Vec<_>>();
    let convert = |ty, conversion| {
        let replacement = Expr {
            source,
            ty: target.clone(),
            kind: ExprKind::ErrorConvert {
                conversion,
                value: Box::new(Expr {
                    source,
                    ty,
                    kind: ExprKind::Local(binding),
                }),
            },
        };
        let mut body = template.clone();
        let mut operand = &mut body;
        while !matches!(operand.kind, ExprKind::Local(_)) {
            operand = match &mut operand.kind {
                ExprKind::ErrorConvert { value, .. } => value,
                ExprKind::Block { statements, .. } => {
                    let ExprKind::LetPattern { value, .. } = &mut statements[0].kind else {
                        unreachable!()
                    };
                    value
                }
                _ => unreachable!(),
            };
        }
        *operand = replacement;
        body
    };
    arm.body = convert((**leaf).clone(), Conversion::Success);
    let mut additional = Vec::with_capacity(remaining.len());
    for label in remaining {
        additional.push(MatchArm {
            pattern: Pattern::ErrorResult {
                label: Some(label.clone()),
                inner: Box::new(Pattern::Binding {
                    binding,
                    inner: Box::new(Pattern::Wild),
                }),
            },
            guard: None,
            body: convert(label, Conversion::Failure),
        });
    }
    additional
}

/// Type substitution has no package source table. Before publishing executable
/// IR, give the newly assembled projection branches distinct cleanup identities,
/// including copied user bodies and captures, retaining original provenance and
/// intentional lexical source sharing. Other match bodies are untouched.
pub fn anchor_completed_projections(
    expression: &mut Expr,
    sources: &mut Vec<crate::program::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    crate::visit::expression_mut(expression, fuel, |node, fuel| {
        let ExprKind::Match { arms, .. } = &mut node.kind else {
            return Ok(());
        };
        let needs_anchors = arms.iter().enumerate().any(|(i, arm)| {
            conversion_operand(&arm.body, true).is_some()
                && (arms[..i]
                    .iter()
                    .any(|other| other.body.source == arm.body.source)
                    || shared_projection_source(&arm.body))
        });
        if !needs_anchors {
            return Ok(());
        }
        for arm in arms {
            if conversion_operand(&arm.body, true).is_none() {
                continue;
            }
            anchor_expression_copy(&mut arm.body, node.source, sources, fuel)?;
        }
        Ok(())
    })
}

/// Give a copied decision subtree private executable identities while retaining
/// lexical sharing within that copy and links to its original source locations.
pub fn anchor_expression_copy(
    expression: &mut Expr,
    site: crate::program::SourceRange,
    sources: &mut Vec<crate::program::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut remapped = std::collections::BTreeMap::new();
    crate::visit::expression_mut(expression, fuel, |value, _| {
        let key = (value.source.start, value.source.end);
        // An inserted wrapper initially shares its operand's source.
        // Split that boundary, but preserve intentional source sharing
        // elsewhere (for example a discarded void expression).
        let separate = matches!(&value.kind, ExprKind::ErrorConvert { value: inner, .. }
                    if value.source == inner.source);
        let source = if !separate && let Some(source) = remapped.get(&key) {
            *source
        } else {
            let index = u32::try_from(sources.len())
                .map_err(|_| "error projection source space exhausted")?;
            sources.push(crate::program::ExpansionSource {
                original: value.source,
                site,
            });
            let source = crate::program::SourceRange::generated(index);
            if !separate {
                remapped.insert(key, source);
            }
            source
        };
        value.source = source;
        Ok(())
    })
}

fn shared_projection_source(mut value: &Expr) -> bool {
    loop {
        match &value.kind {
            ExprKind::ErrorConvert { value: inner, .. } => {
                if value.source == inner.source {
                    return true;
                }
                value = inner;
            }
            ExprKind::Block {
                statements,
                result: Some(_),
            } if statements.len() == 1 => {
                let ExprKind::LetPattern {
                    value: initializer,
                    guard: None,
                    otherwise: None,
                    ..
                } = &statements[0].kind
                else {
                    return false;
                };
                value = initializer;
            }
            _ => return false,
        }
    }
}
