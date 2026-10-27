//! Enumerate successful pattern outputs without replacing ordinary pattern checks.
use super::*;
use function_templates::GenerationBinding;
use hir::pattern::{BitSegment, PatternKind as P};

pub(super) fn collect<'h>(
    root: &'h hir::Pattern<'h>,
    annotation: Option<&'h hir::Expr<'h>>,
    outputs: &mut Vec<GenerationBinding<'h>>,
    fuel: &mut usize,
) -> Result<(), Error> {
    let mut pending = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(pattern) = pending.pop() {
        charge(fuel, pattern.span)?;
        match &pattern.kind {
            P::Binding(mode, name, inner) => {
                if seen.insert(pattern.hir_id) {
                    outputs.push(GenerationBinding {
                        id: pattern.hir_id,
                        name: *name,
                        // A whole-pattern annotation is not the type of each
                        // component, nor of a reference-mode binding.
                        annotation: if pattern.hir_id == root.hir_id
                            && *mode == hir::common::BindingMode::ByValue
                        {
                            annotation
                        } else {
                            None
                        },
                        span: pattern.span,
                    });
                }
                pending.extend(*inner);
            }
            P::Tuple(patterns) | P::Or(patterns) => {
                pending.extend(patterns.iter().rev());
            }
            P::List(patterns, rest) => {
                pending.extend(*rest);
                pending.extend(patterns.iter().rev());
            }
            P::Struct(head, fields, _) | P::AppStruct(head, fields) => {
                pending.extend(fields.iter().rev().map(|field| &field.pat));
                pending.push(head);
            }
            P::AppTuple(head, patterns) => {
                pending.extend(patterns.iter().rev());
                pending.push(head);
            }
            P::Then(left, _, right) => {
                pending.push(right);
                pending.push(left);
            }
            P::Guard(inner, _)
            | P::Projection(inner, _)
            | P::OptionSome(inner)
            | P::ErrorOk(inner)
            | P::ErrorErr(inner)
            | P::Not(inner)
            | P::Ref(inner) => pending.push(inner),
            P::NormalForm(_, arguments) => {
                pending.extend(arguments.iter().rev().map(|argument| &argument.pattern));
            }
            P::BitVec { segments, .. } => {
                for segment in segments.iter().rev() {
                    charge(fuel, pattern.span)?;
                    if let BitSegment::Typed { pattern, .. } = segment {
                        pending.push(pattern);
                    }
                }
            }
            P::CodeSplice(_, bindings) => {
                for binding in bindings {
                    charge(fuel, pattern.span)?;
                    if seen.insert(binding.hir_id) {
                        outputs.push(GenerationBinding {
                            id: binding.hir_id,
                            name: binding.name,
                            annotation: None,
                            span: binding.name.span,
                        });
                    }
                }
            }
            P::Wild
            | P::Const(_)
            | P::Comptime(_)
            | P::OptionNull
            | P::Range(..)
            | P::Async
            | P::Invalid => {}
        }
        if pending.len() > *fuel {
            return Err(error(
                pattern.span,
                "strategy binding traversal budget exhausted",
            ));
        }
    }
    Ok(())
}
