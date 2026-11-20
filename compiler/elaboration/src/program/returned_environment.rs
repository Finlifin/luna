//! Expose acquired dependencies of a call result in the consuming region.
//! Other operands keep their original owners and their call cleanup boundary.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn is_prepared_call_region(&self, value: &n::Expr) -> bool {
        match &value.kind {
            K::CallPreparation {
                result: Some(_), ..
            } => true,
            K::Block {
                result: Some(_), ..
            } => self
                .local_runtime_origins
                .call_regions
                .contains_key(&value.source),
            _ => false,
        }
    }

    pub(super) fn result_dependencies(
        &mut self,
        ty: &Term,
        span: Span,
    ) -> Result<BTreeSet<nir::RuntimeId>, Error> {
        nominal_fields::metered(&mut self.required_steps, |fuel| {
            nir::type_computations::charge_term(ty, fuel)?;
            Ok(ty.runtime_dependencies())
        })
        .map_err(|e| error(span, e.to_string()))
    }

    pub(super) fn dependency_root(&self, id: nir::RuntimeId) -> nir::RuntimeId {
        self.local_runtime_deliveries
            .get(&id)
            .and_then(|entry| entry.projection.as_ref())
            .map_or(id, |path| path.root)
    }

    pub(super) fn take_returned_environment(
        &mut self,
        value: &mut n::Expr,
        span: Span,
    ) -> Result<Vec<n::Expr>, Error> {
        let dependencies = self.result_dependencies(&value.ty, span)?;
        let mut escaping = BTreeSet::new();
        for id in dependencies {
            let root = self.dependency_root(id);
            if let Some(entry) = self.local_runtime_deliveries.get(&root)
                && let K::Local(binding) = entry.expr.kind
                && self
                    .local_runtime_origins
                    .acquired_bindings
                    .contains_key(&binding)
            {
                escaping.insert(binding);
            }
        }
        if escaping.is_empty() {
            return Ok(Vec::new());
        }

        // Record only cuts through compiler-owned preparations. A nested user
        // block may own the referent or have its own full-expression boundary.
        let mut regions = Vec::new();
        let mut cursor: &n::Expr = value;
        let mut last = None;
        while self.is_prepared_call_region(cursor) {
            let (K::CallPreparation {
                statements,
                result: Some(body),
            }
            | K::Block {
                statements,
                result: Some(body),
            }) = &cursor.kind
            else {
                unreachable!()
            };
            self.required_steps = self
                .required_steps
                .checked_sub(1 + statements.len() as u64)
                .ok_or_else(|| error(span, "result environment inspection budget exhausted"))?;
            for (i, stmt) in statements.iter().enumerate() {
                if matches!(&stmt.kind, K::Let { binding, .. } if escaping.contains(binding)) {
                    last = Some((regions.len(), i + 1));
                }
            }
            regions.push(statements.as_slice());
            cursor = body;
        }
        let Some((depth, cut)) = last else {
            return Ok(Vec::new());
        };
        regions.truncate(depth + 1);
        regions[depth] = &regions[depth][..cut];

        // Earlier arguments must still run before the supplier. A transit
        // binding carries their values until their original preparation takes
        // ownership again. Never relocate storage borrowed by another prefix.
        let mut transit = BTreeSet::new();
        for stmt in regions.iter().flat_map(|statements| statements.iter()) {
            let K::Let {
                binding,
                value: operand,
            } = &stmt.kind
            else {
                return Ok(Vec::new());
            };
            if !transit.is_empty() {
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let start = fuel;
                let reads = nir::free_bindings::collect(operand, &mut fuel);
                self.required_steps -= (start - fuel) as u64;
                if !reads
                    .map_err(|e| error(span, e.to_string()))?
                    .is_disjoint(&transit)
                {
                    return Ok(Vec::new());
                }
            }
            if !escaping.contains(binding) {
                if self
                    .local_runtime_origins
                    .acquired_bindings
                    .contains_key(binding)
                {
                    return Ok(Vec::new());
                }
                transit.insert(*binding);
            }
        }

        let cuts: Vec<_> = regions.into_iter().map(<[_]>::len).collect();
        let mut prefix = Vec::new();
        let mut cursor = value;
        for cut in cuts {
            let (K::CallPreparation {
                statements,
                result: Some(body),
            }
            | K::Block {
                statements,
                result: Some(body),
            }) = &mut cursor.kind
            else {
                unreachable!()
            };
            let tail = statements.split_off(cut);
            for mut stmt in std::mem::take(statements) {
                let K::Let {
                    binding,
                    value: operand,
                } = &mut stmt.kind
                else {
                    unreachable!()
                };
                let ty = declaration_inputs::copy_input_domain(
                    &operand.ty,
                    &mut self.required_steps,
                    span,
                )?;
                if escaping.contains(binding) {
                    self.locals.insert(*binding, ty);
                } else {
                    let original = *binding;
                    *binding = self.spread_binding(span)?;
                    statements.push(n::Expr {
                        ty: stmt.ty.clone(),
                        source: stmt.source,
                        kind: K::Let {
                            binding: original,
                            value: Box::new(n::Expr {
                                ty,
                                source: stmt.source,
                                kind: K::Local(*binding),
                            }),
                        },
                    });
                }
                prefix.push(stmt);
            }
            statements.extend(tail);
            cursor = body;
        }
        Ok(prefix)
    }
}
