use super::*;

/// Borrowed identities in stable IIR traversal order, collected once per dump.
/// Domains follow graph.templates; ordinary function tags occupy each prefix.
#[derive(Default)]
pub(super) struct TemplateIdentities<'a> {
    methods: Vec<Vec<&'a Expr>>,
}
impl<'a> TemplateIdentities<'a> {
    pub(super) fn new(domains: usize) -> Self {
        Self {
            methods: vec![vec![]; domains],
        }
    }
    pub(super) fn observe(
        &mut self,
        graph: &'a iir::Package,
        expr: &'a Expr,
        fuel: &mut usize,
    ) -> Result<(), String> {
        if !matches!(expr.kind, K::TraitMethod { .. }) {
            return Ok(());
        }
        *fuel = fuel
            .checked_sub(graph.templates.len())
            .ok_or("host trait identity domain budget exhausted")?;
        let Some(domain) = graph
            .templates
            .iter()
            .position(|table| table.logical_type() == &expr.ty)
        else {
            return Ok(());
        };
        self.insert(domain, expr, fuel)
    }
    fn insert(&mut self, domain: usize, expr: &'a Expr, fuel: &mut usize) -> Result<(), String> {
        let identities = &mut self.methods[domain];
        *fuel = fuel
            .checked_sub(identities.len().saturating_add(1))
            .ok_or("host trait identity collection budget exhausted")?;
        if !identities.iter().any(|previous| previous.kind == expr.kind) {
            identities.push(expr);
        }
        Ok(())
    }
    fn index(&self, domain: usize, expr: &Expr, fuel: &mut usize) -> Result<usize, String> {
        let identities = self
            .methods
            .get(domain)
            .ok_or("host trait identity domain missing")?;
        *fuel = fuel
            .checked_sub(identities.len().saturating_add(1))
            .ok_or("host trait identity lookup budget exhausted")?;
        identities
            .iter()
            .position(|candidate| candidate.kind == expr.kind)
            .ok_or_else(|| "host trait identity is absent from the runtime graph".into())
    }
}

impl Emitter<'_> {
    pub(super) fn callable(&mut self, expr: &Expr) -> Result<Value, String> {
        let key = match &expr.kind {
            K::Function(declaration) => iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![].into(),
            },
            K::FunctionInstance {
                declaration,
                arguments,
            } => iir::InstanceKey {
                declaration: *declaration,
                arguments: arguments.clone().into(),
            },
            K::TraitMethod {
                interface,
                interface_arguments,
                subject,
                member,
                method_arguments,
            } => {
                let method = self
                    .graph
                    .dispatch
                    .iter()
                    .find(|dispatch| {
                        dispatch.interface == *interface
                            && dispatch.interface_arguments == *interface_arguments
                            && dispatch.subject == *subject
                    })
                    .and_then(|dispatch| {
                        dispatch.methods.iter().find(|method| {
                            method.member == *member && method.method_arguments == *method_arguments
                        })
                    })
                    .ok_or("host trait callable has no selected method")?;
                iir::InstanceKey {
                    declaration: method.body,
                    arguments: method.arguments.clone().into(),
                }
            }
            _ => {
                return Err(
                    "host callable requires a completed function or method identity".into(),
                );
            }
        };
        if let Some((domain, table)) = self
            .graph
            .templates
            .iter()
            .enumerate()
            .find(|(_, table)| table.logical_type() == &expr.ty)
        {
            // Validate the selected target even for trait identities, whose
            // identity is more precise than the implementation body address.
            let selected = table.encode(&key, self.fuel)?;
            let tag = if matches!(expr.kind, K::TraitMethod { .. }) {
                let index = self.template_identities.index(domain, expr, self.fuel)?;
                u64::try_from(table.targets().len())
                    .ok()
                    .and_then(|base| {
                        u64::try_from(index)
                            .ok()
                            .and_then(|index| base.checked_add(index))
                    })
                    .ok_or("host trait identity tag overflow")?
            } else {
                selected
            };
            return Ok(Value {
                ty: Ty::TemplateIdentity(std::sync::Arc::new(expr.ty.clone())),
                text: tag.to_string(),
            });
        }
        let index = self
            .graph
            .functions
            .iter()
            .position(|function| function.key == key)
            .ok_or("host function address is missing from the instance graph")?;
        if repr(self.graph, &expr.ty, self.checked)? != Ty::FunctionPointer {
            return Err("host function value lacks an ordinary function signature".into());
        }
        Ok(Value {
            ty: Ty::FunctionPointer,
            text: format!("@flurry_{index}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_identity_index_deduplicates_references_without_merging_domains() {
        let method = Expr {
            source: nir::program::SourceRange { start: 10, end: 20 },
            ty: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Bool)),
            },
            kind: K::TraitMethod {
                interface: nir::DeclarationId {
                    package: 0,
                    local: 1,
                },
                interface_arguments: vec![],
                member: nir::DeclarationId {
                    package: 0,
                    local: 2,
                },
                subject: Term::Primitive(P::Bool),
                method_arguments: vec![],
            },
        };
        let mut alias = method.clone();
        alias.source = nir::program::SourceRange { start: 30, end: 40 };
        let mut other = method.clone();
        let K::TraitMethod { subject, .. } = &mut other.kind else {
            unreachable!()
        };
        *subject = Term::Primitive(P::Char);
        let mut index = TemplateIdentities::new(2);
        let mut fuel = 100;
        index.insert(0, &method, &mut fuel).unwrap();
        index.insert(0, &alias, &mut fuel).unwrap();
        assert!(index.insert(0, &other, &mut 0).is_err());
        assert_eq!(index.methods[0].len(), 1);
        index.insert(0, &other, &mut fuel).unwrap();
        index.insert(1, &other, &mut fuel).unwrap();
        assert_eq!(index.index(0, &alias, &mut fuel).unwrap(), 0);
        assert_eq!(index.index(0, &other, &mut fuel).unwrap(), 1);
        assert_eq!(index.index(1, &other, &mut fuel).unwrap(), 0);
        assert!(index.index(1, &method, &mut fuel).is_err());
        assert!(index.index(0, &method, &mut 0).is_err());
    }
}

pub(super) struct MethodFrame<'a> {
    contract: &'a nir::program::MethodContract,
    source: nir::program::SourceRange,
    result_slot: usize,
    locals: BTreeMap<LocalId, Slot>,
    live: BTreeMap<(LocalId, Vec<usize>), Slot>,
}

impl<'a> Emitter<'a> {
    /// Interface entry predicates run after argument preparation but before
    /// ownership transfers to the selected implementation. Keep the interface
    /// identity here: its body address alone does not carry this obligation.
    pub(super) fn method_entry(
        &mut self,
        callee: &Expr,
        values: &[Option<Value>],
        source: nir::program::SourceRange,
        depth: usize,
    ) -> Result<Option<MethodFrame<'a>>, String> {
        let K::TraitMethod {
            interface,
            interface_arguments,
            subject,
            member,
            method_arguments,
        } = &callee.kind
        else {
            return Ok(None);
        };
        let contract = self
            .graph
            .interfaces
            .iter()
            .find(|entry| {
                entry.interface.declaration == *interface
                    && entry.interface.arguments == *interface_arguments
                    && entry.subject == *subject
            })
            .and_then(|entry| {
                entry.methods.iter().find(|method| {
                    method.declaration == *member && method.type_arguments == *method_arguments
                })
            })
            .ok_or("host method call lacks its concrete interface contract")?;
        if contract.parameter_bindings.len() != values.len() {
            return Err("host method contract parameter layout mismatch".into());
        }
        if contract.program.is_empty() {
            return Ok(None);
        }
        if self.checked
            && !contract.program.postconditions.is_empty()
            && self.cleanup.map_or(0, |schedule| {
                schedule
                    .sites
                    .iter()
                    .filter(|site| {
                        site.source == source
                            && site.boundary
                                == iir::cleanup::Boundary::InterfacePostconditionFailure
                    })
                    .count()
            }) != 1
        {
            return Err(
                "host interface postconditions require a unique result cleanup boundary".into(),
            );
        }
        // The interface is a distinct lexical frame, not an overlay on the
        // caller. Only prepared parameters and explicit snapshots are visible.
        let mut saved = std::mem::take(&mut self.locals);
        let mut saved_live = std::mem::take(&mut self.live);
        let result = (|| {
            for (binding, value) in contract.parameter_bindings.iter().zip(values) {
                let value = value
                    .as_ref()
                    .ok_or("host method contract argument is missing")?;
                self.bind(*binding, value)?;
            }
            for snapshot in &contract.program.entry_snapshots {
                if !contract.parameter_bindings.contains(&snapshot.parameter) {
                    return Err("host method entry snapshot target missing".into());
                }
                let slot = self
                    .locals
                    .get(&snapshot.parameter)
                    .cloned()
                    .ok_or("host method entry snapshot target missing")?;
                let value = self.load(&slot);
                self.bind(snapshot.binding, &value)?;
            }
            for predicate in &contract.program.preconditions {
                let condition = self.expr(predicate, depth + 1)?;
                if self.terminated {
                    return Err("host interface predicate did not produce a condition".into());
                }
                std::mem::swap(&mut self.locals, &mut saved);
                std::mem::swap(&mut self.live, &mut saved_live);
                let guarded = self.guard_with_diagnostic(
                    &condition,
                    Some((source, iir::cleanup::Boundary::InterfacePreconditionFailure)),
                    Some(predicate.source),
                );
                std::mem::swap(&mut self.locals, &mut saved);
                std::mem::swap(&mut self.live, &mut saved_live);
                guarded?;
            }
            Ok(Some(MethodFrame {
                contract,
                source,
                result_slot: values.len(),
                locals: self.locals.clone(),
                live: self.live.clone(),
            }))
        })();
        self.locals = saved;
        self.live = saved_live;
        result
    }
}

impl Emitter<'_> {
    pub(super) fn method_exit(
        &mut self,
        frame: Option<MethodFrame<'_>>,
        value: &Value,
        depth: usize,
    ) -> Result<(), String> {
        let Some(frame) = frame else {
            return Ok(());
        };
        if frame.contract.program.postconditions.is_empty() {
            return Ok(());
        }
        // The arguments have transferred. Reserve the slot immediately after
        // them for the produced result, until the interface accepts delivery.
        self.retain_prepared_field(frame.source, frame.result_slot, frame.source, value)?;
        let mut saved = std::mem::replace(&mut self.locals, frame.locals);
        let mut saved_live = std::mem::replace(&mut self.live, frame.live);
        let result = (|| {
            if let Some(binding) = frame.contract.program.result_binding {
                self.bind(binding, value)?;
            }
            for predicate in &frame.contract.program.postconditions {
                let condition = self.expr(predicate, depth + 1)?;
                if self.terminated {
                    return Err("host interface postcondition did not produce a condition".into());
                }
                // Predicates observe the interface frame, but failure unwinds
                // caller-owned locals. Their binding IDs can coincide (e.g.
                // recursive calls); never use the observation slots as owners.
                std::mem::swap(&mut self.locals, &mut saved);
                std::mem::swap(&mut self.live, &mut saved_live);
                let guarded = self.guard_with_diagnostic(
                    &condition,
                    Some((
                        frame.source,
                        iir::cleanup::Boundary::InterfacePostconditionFailure,
                    )),
                    Some(predicate.source),
                );
                std::mem::swap(&mut self.locals, &mut saved);
                std::mem::swap(&mut self.live, &mut saved_live);
                guarded?;
            }
            self.finish_prepared_construction(frame.source)?;
            Ok(())
        })();
        self.locals = saved;
        self.live = saved_live;
        result
    }
}
