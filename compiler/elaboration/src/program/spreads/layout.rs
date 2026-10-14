//! Read-only packet layout hints. Hints select element contexts, never establish
//! a type equality or replace checking the packet expression itself.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn spread_packet_contexts(
        &mut self,
        args: &[Arg<'_>],
        types: &[Term],
        span: Span,
    ) -> Result<Vec<Option<Term>>, Error> {
        self.spread_packet_layout(args, types, span)?.into_iter()
            .map(|fields| fields.map(|fields| self.tuple_value_type(fields, span)).transpose())
            .collect()
    }

    pub(in crate::program) fn spread_packet_layout<T: Clone>(
        &mut self,
        args: &[Arg<'_>],
        types: &[T],
        span: Span,
    ) -> Result<Vec<Option<Vec<T>>>, Error> {
        self.spread_packet_layout_with_widths(args, types, span, &BTreeMap::new())
    }

    pub(in crate::program) fn spread_packet_layout_with_widths<T: Clone>(
        &mut self,
        args: &[Arg<'_>],
        types: &[T],
        span: Span,
        settled: &BTreeMap<usize, usize>,
    ) -> Result<Vec<Option<Vec<T>>>, Error> {
        let mut fuel = 10_000;
        let mut widths = Vec::with_capacity(args.len());
        for (index, argument) in args.iter().enumerate() {
            widths.push(match argument {
                Arg::Positional(_) => Some(1),
                Arg::Named(..) | Arg::Implicit(_) | Arg::PostLambda(_) => Some(0),
                Arg::Expand(value) => match settled.get(&index) {
                    Some(width) => Some(*width),
                    None => self.spread_width(value, &mut fuel, 0)?,
                },
            });
        }
        let known = widths.iter().flatten().try_fold(0usize, |sum, width| {
            sum.checked_add(*width)
                .ok_or_else(|| error(span, "spread layout size exhausted"))
        })?;
        if known > types.len() {
            return Err(error(
                span,
                "argument count mismatch: spread tuple widths exceed positional slots",
            ));
        }
        let unknown: Vec<_> = widths
            .iter()
            .enumerate()
            .filter_map(|(index, width)| width.is_none().then_some(index))
            .collect();
        if let [index] = unknown.as_slice() {
            widths[*index] = Some(types.len() - known);
        }
        // Both ends can provide unambiguous contexts even when two interior
        // widths remain unknown. Never choose a partition for those interiors.
        let mut contexts = vec![None; args.len()];
        let mut start = 0usize;
        for (index, width) in widths.iter().enumerate() {
            let Some(width) = width else { break };
            let end = start + width;
            if let Arg::Expand(_) = args[index] {
                contexts[index] = Some(types[start..end].to_vec());
            }
            start = end;
        }
        let mut end = types.len();
        for (index, width) in widths.iter().enumerate().rev() {
            let Some(width) = width else { break };
            let start = end - width;
            if contexts[index].is_none() && matches!(args[index], Arg::Expand(_)) {
                contexts[index] = Some(types[start..end].to_vec());
            }
            end = start;
        }
        Ok(contexts)
    }

    pub(in crate::program) fn spread_width(
        &mut self,
        value: &Expr<'_>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<usize>, Error> {
        if *fuel == 0 || depth >= 128 {
            return Err(error(
                value.span,
                "spread layout inference budget exhausted",
            ));
        }
        *fuel -= 1;
        match &value.kind {
            H::Tuple(items) => Ok(Some(items.len())),
            H::Block(block) => match block.expr {
                Some(result) => self.spread_width(result, fuel, depth + 1),
                None => Ok(None),
            },
            H::If(_, yes, Some(no)) => {
                let Some(yes) = yes.expr else { return Ok(None) };
                let left = self.spread_width(yes, fuel, depth + 1)?;
                let right = self.spread_width(no, fuel, depth + 1)?;
                Ok(left.filter(|_| left == right))
            }
            H::Match(_, arms) => {
                self.spread_branch_width(arms.iter().map(|arm| arm.body), fuel, depth + 1)
            }
            H::When(arms) => {
                self.spread_branch_width(arms.iter().map(|arm| arm.body), fuel, depth + 1)
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let Some(ty) = self.locals.get(&local(*id)).cloned() else {
                    return Ok(None);
                };
                self.tuple_width(&ty, value.span)
            }
            H::Application(callee, _) => {
                let head = if let H::NFApplication(head, _) = callee.kind {
                    head
                } else {
                    callee
                };
                let (signature, generic) = match head.kind {
                    H::ResolvedDef { def_id, .. } => {
                        let Some(owner) = self.package.definition_owner(def_id) else {
                            return Ok(None);
                        };
                        if !matches!(
                            self.package.item(owner).map(|item| &item.kind),
                            Some(ItemKind::Fn(..))
                        ) {
                            return Ok(None);
                        }
                        self.ensure_signature(owner, value.span)?;
                        let Some(signature) = self.signatures.get(&owner) else {
                            return Ok(None);
                        };
                        (
                            signature.clone(),
                            self.type_parameters
                                .get(&owner)
                                .is_some_and(|types| !types.is_empty()),
                        )
                    }
                    H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                        let Some(ty) = self.locals.get(&local(id)).cloned() else {
                            return Ok(None);
                        };
                        (self.structural_type(&ty, value.span)?, false)
                    }
                    _ => return Ok(None),
                };
                let signature = if let Term::Closure { signature, .. } = &signature {
                    signature.as_ref()
                } else {
                    &signature
                };
                let Term::Function { result, .. } = signature.callable_signature() else {
                    return Ok(None);
                };
                if let Term::Tuple(fields) = result.as_ref() {
                    Ok(Some(fields.len()))
                } else if let Some(fields) = self.bound_tuple_elements(result) {
                    // The explicit list fixes arity even while its element
                    // types contain this callee's uninstantiated binders.
                    Ok(Some(fields.len()))
                } else if generic {
                    // An uninstantiated result binder is not the caller's type
                    // argument. Never use its body or a caller-side equation to
                    // invent the shape of an application.
                    Ok(None)
                } else {
                    self.tuple_width(result, value.span)
                }
            }
            _ => Ok(None),
        }
    }

    fn spread_branch_width<'h>(
        &mut self,
        branches: impl Iterator<Item = &'h Expr<'h>>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<usize>, Error> {
        let mut width = None;
        for branch in branches {
            let Some(next) = self.spread_width(branch, fuel, depth)? else {
                return Ok(None);
            };
            if width.is_some_and(|previous| previous != next) {
                return Ok(None);
            }
            width = Some(next);
        }
        Ok(width)
    }

    fn tuple_width(&self, ty: &Term, span: Span) -> Result<Option<usize>, Error> {
        Ok(match self.structural_type(ty, span)? {
            Term::Tuple(fields) => Some(fields.len()),
            _ => None,
        })
    }
}
