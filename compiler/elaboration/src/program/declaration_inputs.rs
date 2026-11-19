//! Shared source-order input plans, independent of the eventual type constructor.
use super::*;
mod collections;
mod completion;
mod domain;
mod environment;
mod interface;
mod observations;
pub(super) mod preparation;
pub(super) use observations::{CallableBindings, KnownInputs, Observations};
mod progress;
pub(super) mod request;
mod routing;
mod scope;
mod slots;

/// Consumers that cannot yet suspend frontend checking require ordinary typed
/// expressions. Both consumers retain the same routing and execution plan.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InputChecking {
    Contextual,
    Typed,
}

pub(super) use completion::Completion;
pub(super) use environment::copy_term as copy_input_domain;
pub(super) use environment::{Environment, copy_domains};

type Parameters = Vec<(DeclarationId, Sort, Term)>;

/// Moved from routing to slot checking and finally to the result consumer.
/// Captured parent values precede the local declaration slots.
pub(super) struct PreparedTelescope {
    pub captured: KnownInputs,
    pub local: Parameters,
}

pub(super) struct PreparedDeclarationInputs {
    pub arguments: Vec<n::NormalFormArgument>,
    pub plan: n::NormalFormInputPlan,
    pub observations: Observations,
}

/// Typed consumers receive ordinary NIR only, with execution statements separate
/// from the declaration-order result slots and their formation evidence.
pub(super) struct CheckedDeclarationInputs {
    pub operands: n::ClauseOperands,
    pub observations: Observations,
}

impl std::fmt::Display for CheckedDeclarationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(checked-declaration-inputs {} {})",
            self.operands, self.observations
        )
    }
}

impl PreparedDeclarationInputs {
    pub(super) fn into_typed(
        self,
        fuel: &mut u64,
        span: Span,
    ) -> Result<CheckedDeclarationInputs, Error> {
        let n::NormalFormInputPlan {
            order,
            preparations,
            bindings,
        } = self.plan;
        let width = order.len().saturating_add(preparations.len());
        *fuel = fuel
            .checked_sub(width as u64)
            .ok_or_else(|| error(span, "declaration operand preparation budget exhausted"))?;
        let mut sources = self.arguments.into_iter().map(Some).collect::<Vec<_>>();
        let mut outputs = vec![None; sources.len()];
        let mut statements = Vec::new();
        let steps = n::InputStep::in_order(
            order.into_iter(),
            preparations.into_iter().map(|p| (p.before, p.expression)),
        );
        for step in steps {
            let slot = match step {
                n::InputStep::Prepare(expr) => {
                    statements.push(expr);
                    continue;
                }
                n::InputStep::Supply(slot) => slot,
            };
            let Some(n::NormalFormArgument::Value(value)) = sources[slot].take() else {
                return Err(error(span, "typed declaration input was not checked"));
            };
            let binding = bindings[slot];
            let output = if matches!(value.kind, K::TypeValue(_)) {
                value
            } else {
                let ty = copy_input_domain(&value.ty, fuel, span)?;
                let location = value.source;
                statements.push(n::Expr {
                    source: location,
                    ty: if is_never(&ty) {
                        p(P::Never)
                    } else {
                        p(P::Void)
                    },
                    kind: K::Let {
                        binding,
                        value: Box::new(value),
                    },
                });
                n::Expr {
                    source: location,
                    kind: K::Local(binding),
                    ty,
                }
            };
            outputs[slot] = Some(output);
        }
        Ok(CheckedDeclarationInputs {
            operands: n::ClauseOperands {
                observations: Default::default(),
                arguments: outputs.into_iter().map(Option::unwrap).collect(),
                statements,
            },
            observations: self.observations,
        })
    }

    pub fn diverges(&self) -> bool {
        self.arguments
            .iter()
            .any(n::NormalFormArgument::is_diverging)
            || self
                .plan
                .preparations
                .iter()
                .any(|p| is_never(&p.expression.ty))
    }
}

impl std::fmt::Display for PreparedDeclarationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(prepared-declaration-inputs (arguments")?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") {} {})", self.plan, self.observations)
    }
}

enum RoutedDeclarationInput<'a, 'h> {
    Source(&'a Expr<'h>),
    Prepared(n::Expr),
    Collected(Vec<n::NormalFormArgument>),
}

impl std::fmt::Display for RoutedDeclarationInput<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(input) => write!(f, "(source {} {})", input.hir_id, source(input.span)),
            Self::Prepared(value) => write!(f, "(prepared {value})"),
            Self::Collected(items) => {
                f.write_str("(collected")?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// A computed packet occupies one original argument position, including
/// when it exports no fields. Collectors merge these actions in source order.
struct PacketPreparation {
    source_index: usize,
    action: n::NormalFormPreparation,
}
impl std::fmt::Display for PacketPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(source-index {} (before {}) {})",
            self.source_index, self.action.before, self.action.expression
        )
    }
}

/// Source references awaiting collection checking, not another expression IR.
/// `before` records the number of fixed inputs preceding this source input.
struct PendingDeclarationCollection<'a, 'h> {
    source_index: usize,
    before: usize,
    input: &'a Expr<'h>,
    spread: bool,
}

impl std::fmt::Display for PendingDeclarationCollection<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(pending-declaration-collection (source-index {}) (before {}) (spread {}) {})",
            self.source_index,
            self.before,
            self.spread,
            source(self.input.span)
        )
    }
}

/// A caller-scoped domain can remain generic while all of this call's
/// dependencies are supplied. Pending refers to callee inputs, not rigidity.
struct DeclarationInputDomain {
    ty: Term,
    pending: BTreeSet<DeclarationId>,
}
impl std::fmt::Display for DeclarationInputDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(declaration-input-domain (pending")?;
        for input in &self.pending {
            write!(f, " {input}")?;
        }
        write!(f, ") (type {}))", self.ty)
    }
}

impl Checker<'_, '_> {
    pub(super) fn checked_input_schedule<'p>(
        &mut self,
        plan: &'p n::NormalFormInputPlan,
        count: usize,
        span: Span,
    ) -> Result<Option<n::CheckedInputSchedule<'p>>, Error> {
        n::InputSchedule::normal(Some(plan)).check(count, |n| {
            self.required_steps = self
                .required_steps
                .checked_sub(n as u64)
                .ok_or_else(|| error(span, "declaration input schedule budget exhausted"))?;
            Ok(())
        })
    }
}

impl Checker<'_, '_> {
    pub(super) fn planned_positional_inputs(
        &mut self,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        collector: Option<usize>,
    ) -> Result<PreparedDeclarationInputs, Error> {
        let preparation = preparation::Frame::positional(
            self,
            inputs,
            span,
            parameters,
            collector,
            completion::Contextual,
        )?;
        self.check_declaration_inputs_region(preparation)
    }

    /// Snapshot an interleaved collector operand at its lexical position.
    fn prepare_declaration_collection_input(
        &mut self,
        value: n::Expr,
        before: usize,
        preparations: &mut Vec<n::NormalFormPreparation>,
        known_bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let binding = self.spread_binding(span)?;
        let known = self.preserved_input_identity(
            &value,
            |id, _| Ok(known_bindings.get(&id).cloned()),
            span,
        )?;
        if let Some(known) = known {
            known_bindings.insert(binding, std::sync::Arc::new(known));
        }

        let ty = copy_input_domain(&value.ty, &mut self.required_steps, span)?;
        let location = value.source;
        let diverges = is_never(&ty);
        preparations.push(n::NormalFormPreparation {
            before,
            expression: n::Expr {
                source: location,
                ty: if diverges { p(P::Never) } else { p(P::Void) },
                kind: K::Let {
                    binding,
                    value: Box::new(value),
                },
            },
        });
        Ok(n::Expr {
            source: location,
            ty,
            kind: K::Local(binding),
        })
    }

    /// Retain declaration-checked code until the VM has admitted the actual
    /// prefix. Rebinding changes local identities, not the prefix's availability.
    fn connected_normal_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        progress: &progress::Progress,
        span: Span,
    ) -> Result<n::Expr, Error> {
        progress.require_default_prefix(slot, span)?;
        let mut value = self.checked_normal_default(owner, slot)?;
        nir::copy_budget::charge_type_rewrite(&value, &mut self.required_steps).map_err(
            |reason| {
                error(
                    span,
                    format!("default expression rewrite limit: {reason:?}"),
                )
            },
        )?;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let free = nir::free_bindings::collect(&value, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let mut connections = free
            .map_err(|message| error(span, message))?
            .into_iter()
            .map(|binding| (binding, binding))
            .collect::<BTreeMap<_, _>>();
        for (previous, (_, binding, _)) in self.normal_parameters[&owner][..slot].iter().enumerate()
        {
            connections.insert(*binding, progress.binding(previous));
        }
        cmtt::freshen(
            &mut value,
            connections,
            &mut self.next_temporary_binding,
            &mut self.expansion_sources,
            span,
        )?;
        value.rewrite_types(&mut |ty| progress.instantiate(ty, &mut self.required_steps, span))?;
        Ok(value)
    }

    /// Prepare explicit inputs in source order, then omitted inputs in their
    /// declaration telescope. All reuse goes through fresh local bindings.
    pub(super) fn planned_declaration_inputs(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<&nominal::InputEnvironment<'_>>,
    ) -> Result<PreparedDeclarationInputs, Error> {
        self.plan_declaration_inputs(
            owner,
            inputs,
            span,
            parameters,
            defaults,
            completion::Contextual,
        )
    }

    pub(super) fn planned_typed_declaration_inputs(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<&nominal::InputEnvironment<'_>>,
    ) -> Result<CheckedDeclarationInputs, Error> {
        self.plan_declaration_inputs(owner, inputs, span, parameters, defaults, completion::Typed)
    }

    fn plan_declaration_inputs<C: Completion>(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<&nominal::InputEnvironment<'_>>,
        completion: C,
    ) -> Result<C::Output, Error> {
        let defaults = defaults.map(Environment::retain);
        self.complete_declaration_inputs(owner, inputs, span, parameters, defaults, completion)
    }

    pub(super) fn complete_declaration_inputs<C: Completion>(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: Vec<(DeclarationId, Sort, Term)>,
        defaults: Option<Environment>,
        completion: C,
    ) -> Result<C::Output, Error> {
        let preparation = preparation::Frame::begin(
            self,
            owner,
            inputs,
            span,
            parameters,
            KnownInputs::default(),
            defaults,
            completion,
        )?;
        self.check_declaration_inputs_region(preparation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_dump_retains_source_slots_identities_and_escaped_values() {
        let location = n::SourceRange { start: 3, end: 9 };
        let id = DeclarationId {
            package: 1,
            local: 7,
        };
        let value = n::Expr {
            source: location,
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("quote\"\n".into())),
        };
        let inputs = PreparedDeclarationInputs {
            arguments: vec![n::NormalFormArgument::Value(value)],
            plan: n::NormalFormInputPlan {
                order: vec![0],
                bindings: vec![n::LocalId { owner: 4, index: 2 }],
                preparations: vec![],
            },
            observations: Observations::new(vec![(
                nir::DeclarationParameter {
                    declaration: id,
                    sort: Sort::ComptimeValue,
                },
                None,
            )]),
        };
        let dump = inputs.to_string();
        assert!(
            dump.starts_with("(prepared-declaration-inputs (arguments "),
            "{dump}"
        );
        assert!(dump.contains("(source 3 9)"), "{dump}");
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        assert!(dump.contains("(order 0)"), "{dump}");
        assert!(dump.contains("(bindings (local 4 2))"), "{dump}");
        assert!(dump.contains("(pending)"), "{dump}");
        assert!(dump.contains(&id.to_string()), "{dump}");
        assert!(!dump.contains('\n'), "unescaped newline: {dump}");
        let checked = inputs.into_typed(&mut 1000, Span::default()).unwrap();
        let dump = checked.to_string();
        assert!(
            dump.starts_with("(checked-declaration-inputs (clause-operands (statements "),
            "{dump}"
        );
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        assert!(dump.contains("(arguments (expr"), "{dump}");
        assert!(dump.contains("(local 4 2)"), "{dump}");
        assert!(dump.contains("(pending)"), "{dump}");
        assert!(!dump.contains('\n'), "unescaped newline: {dump}");
    }
}
